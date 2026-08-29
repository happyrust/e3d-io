pub mod cache;
pub mod io;

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use nom::number::complete::be_u32;
use nom::IResult;

use crate::error::PageIoError;

/// A physical page address: which extent, and which page inside it.
///
/// core.dll addresses a page as an `(extent, page)` pair and so does this
/// layer. Flattening it to a bare page number is what let `PageId { ext: 2, .. }`
/// read extent 1 and report success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PageId {
    pub ext: u16,
    pub page: u32,
}

impl PageId {
    /// Extent number of the first extent of a database.
    ///
    /// The descriptor's `primary_extent_ref` at +0x38 decodes back to `db_mark`
    /// with an extent index of 1 in all 448 databases of `AvevaMarineSample`,
    /// and the primary file is named `_0001`.
    pub const PRIMARY: u16 = 1;

    pub const fn primary(page: u32) -> Self {
        Self {
            ext: Self::PRIMARY,
            page,
        }
    }

    pub const fn new(ext: u16, page: u32) -> Self {
        Self { ext, page }
    }

    pub const fn byte_offset(self, size: PageSize) -> u64 {
        self.page as u64 * size.bytes() as u64
    }
}

impl fmt::Display for PageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "extent {} page {}", self.ext, self.page)
    }
}

/// How large a page is, stored as the word count the file actually states.
///
/// The header field at +0x34 counts 4-byte words. Reading it as a byte count
/// made 17 of 490 production files unreadable — `ams7329_0001` reported
/// `sesno = 0` against an authoritative 221. Keeping words as the stored form
/// and offering [`PageSize::words`] and [`PageSize::bytes`] as separate
/// accessors means the conversion can only happen in one direction, once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PageSize(u32);

impl PageSize {
    pub const MIN_BYTES: u32 = 128;
    pub const MAX_BYTES: u32 = 65_536;

    /// Validate the header's word count.
    ///
    /// Returns `Err` for anything outside the observed shape. There is
    /// deliberately no fallback: the reader this replaced ended its check with
    /// `else { DEFAULT_PAGE_SIZE }`, so a corrupt or misread field produced a
    /// reader that silently addressed every page in the file wrongly.
    ///
    /// The power-of-two requirement is an observed invariant, not a proven
    /// format rule — all 429 sample databases state 512 words (2048 bytes). A
    /// file that legitimately used another size would be rejected loudly here,
    /// which is the intended direction to fail in.
    pub fn from_header_words(words: u32) -> Result<Self, PageIoError> {
        let reject = |detail: &str| PageIoError::InvalidPageSize {
            path: None,
            words,
            detail: detail.to_string(),
        };

        let bytes = words
            .checked_mul(4)
            .ok_or_else(|| reject("word count overflows a byte count"))?;
        if bytes < Self::MIN_BYTES {
            return Err(reject(&format!(
                "{bytes} bytes is below the {} byte minimum",
                Self::MIN_BYTES
            )));
        }
        if bytes > Self::MAX_BYTES {
            return Err(reject(&format!(
                "{bytes} bytes is above the {} byte maximum",
                Self::MAX_BYTES
            )));
        }
        if !bytes.is_power_of_two() {
            return Err(reject(&format!("{bytes} bytes is not a power of two")));
        }
        Ok(Self(words))
    }

    pub const fn words(self) -> u32 {
        self.0
    }

    pub const fn bytes(self) -> usize {
        self.0 as usize * 4
    }
}

impl fmt::Display for PageSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} bytes ({} words)", self.bytes(), self.words())
    }
}

/// Read accounting for one page source.
///
/// `pages_read` and `bytes_read` cover demand reads only. Speculative reads are
/// counted apart in `prefetched_pages`, because the index layer measures "how
/// many pages did this point lookup cost" as a difference of two `pages_read`
/// snapshots — mixing prefetch into that counter is what made the measurement
/// untrustworthy. Total disk traffic is
/// `bytes_read + prefetched_pages * page_size.bytes()`.
///
/// `cache_hits` and `cache_misses` are always zero on an uncached source; only
/// [`cache::PageCache`] fills them in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IoStats {
    pub pages_read: u64,
    pub bytes_read: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub prefetched_pages: u64,
}

/// Atomic counters behind an `&self` page source.
#[derive(Debug, Default)]
pub(crate) struct IoCounters {
    pages_read: AtomicU64,
    bytes_read: AtomicU64,
    prefetched_pages: AtomicU64,
}

impl IoCounters {
    pub(crate) fn record_demand(&self, bytes: usize) {
        self.pages_read.fetch_add(1, Ordering::Relaxed);
        self.bytes_read.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub(crate) fn record_prefetch(&self) {
        self.prefetched_pages.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> IoStats {
        IoStats {
            pages_read: self.pages_read.load(Ordering::Relaxed),
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            cache_hits: 0,
            cache_misses: 0,
            prefetched_pages: self.prefetched_pages.load(Ordering::Relaxed),
        }
    }
}

/// Whether a read was asked for or guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadIntent {
    /// The caller needs these bytes now; charged to `pages_read`.
    Demand,
    /// Speculative fill; charged to `prefetched_pages`.
    Prefetch,
}

/// The L0 contract: hand me a page address and a buffer, get bytes or an error.
///
/// `read_into` takes `&self` because every implementation uses positioned reads
/// (`seek_read` / `read_at`), so there is no shared file cursor to protect.
///
/// There is deliberately no `read_page() -> Vec<u8>`. The reader this replaced
/// allocated one vector per page, which made a single tree descent cost one
/// heap allocation per level.
pub trait PageSource {
    fn page_size(&self) -> PageSize;

    fn read_with_intent(
        &self,
        id: PageId,
        buf: &mut [u8],
        intent: ReadIntent,
    ) -> Result<(), PageIoError>;

    fn stats(&self) -> IoStats;

    /// Read a page the caller needs now.
    fn read_into(&self, id: PageId, buf: &mut [u8]) -> Result<(), PageIoError> {
        self.read_with_intent(id, buf, ReadIntent::Demand)
    }

    /// Read a page the caller only expects to need.
    fn prefetch_into(&self, id: PageId, buf: &mut [u8]) -> Result<(), PageIoError> {
        self.read_with_intent(id, buf, ReadIntent::Prefetch)
    }
}

/// Page type enumeration.
///
/// Evidence: E3D_数据库文件架构完整分析.md + struct_layouts.json page_header
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageType {
    Descriptor,
    RefArray,
    Session,
    Data,
    Special,
    Index,
    Unknown(u32),
}

impl PageType {
    pub fn from_raw(value: u32) -> Self {
        match value {
            0 => Self::Descriptor,
            1 => Self::RefArray,
            3 => Self::Session,
            5 => Self::Data,
            7 => Self::Special,
            8 => Self::Index,
            other => Self::Unknown(other),
        }
    }

    pub fn as_raw(&self) -> u32 {
        match self {
            Self::Descriptor => 0,
            Self::RefArray => 1,
            Self::Session => 3,
            Self::Data => 5,
            Self::Special => 7,
            Self::Index => 8,
            Self::Unknown(v) => *v,
        }
    }
}

/// Common page header present at the start of every database page.
///
/// Layout from struct_layouts.json "page_header" (all fields big-endian):
/// - +0: page_type (u32)
/// - +4: type_id (u32)
/// - +8: db_handle (u32)
/// - +12: ext_no (u32)
/// - +16: page_no (u32)
/// - +20: bucket_id (u32, low 13 bits significant)
#[derive(Debug, Clone)]
pub struct PageHeader {
    pub page_type: PageType,
    pub type_id: u32,
    pub db_handle: u32,
    pub ext_no: u32,
    pub page_no: u32,
    pub bucket_id: u16,
}

impl PageHeader {
    pub const SIZE: usize = 24;

    pub fn parse(data: &[u8]) -> Option<Self> {
        parse_page_header(data).ok().map(|(_, hdr)| hdr)
    }
}

fn parse_page_header(input: &[u8]) -> IResult<&[u8], PageHeader> {
    use nom::Parser;
    let (input, page_type_raw) = be_u32.parse(input)?;
    let (input, type_id) = be_u32.parse(input)?;
    let (input, db_handle) = be_u32.parse(input)?;
    let (input, ext_no) = be_u32.parse(input)?;
    let (input, page_no) = be_u32.parse(input)?;
    let (input, bucket_raw) = be_u32.parse(input)?;

    Ok((
        input,
        PageHeader {
            page_type: PageType::from_raw(page_type_raw),
            type_id,
            db_handle,
            ext_no,
            page_no,
            bucket_id: (bucket_raw & 0x1FFF) as u16,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_size_keeps_words_and_bytes_apart() {
        let size = PageSize::from_header_words(512).unwrap();
        assert_eq!(size.words(), 512);
        assert_eq!(size.bytes(), 2048);
    }

    #[test]
    fn page_size_rejects_instead_of_defaulting() {
        // 2048 read as a byte count is the exact defect this replaces: as a
        // word count it means 8192 bytes, and as a bare field it must not
        // silently become the old DEFAULT_PAGE_SIZE.
        assert!(PageSize::from_header_words(0).is_err());
        assert!(PageSize::from_header_words(1).is_err());
        assert!(PageSize::from_header_words(500).is_err());
        assert!(PageSize::from_header_words(u32::MAX).is_err());
        assert!(PageSize::from_header_words(65_536).is_err());
    }

    #[test]
    fn byte_offset_scales_by_bytes_not_words() {
        let size = PageSize::from_header_words(512).unwrap();
        assert_eq!(PageId::primary(3).byte_offset(size), 6144);
    }
}
