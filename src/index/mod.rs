//! B+tree index page decoding: bytes in, structure out. No I/O lives here.
//! The tree walk that puts pages in order lives in [`cursor`].
//!
//! Four shipped implementations read these same bytes three incompatible ways,
//! and no test could separate them, so the readings below were settled against
//! 429 real databases in
//! `docs/evidence/2026-08-30-dabacon-index-page-layout-adjudication.md`. One of
//! the eight candidate combinations left all seven structural counters at zero
//! and the other seven did not, so this module implements that one and offers
//! no alternative, not even as a fallback:
//!
//! * An entry is a struct in an array — `[refno.hi, refno.lo, page_no, packed]`
//!   repeated — not a run of keys followed by a run of values. SoA decoding was
//!   clean on none of the 429 files; its key sequence comes out shuffled.
//! * The entry count comes from `free_dwords`, the seventh header dword.
//!   core.dll does not clear a slot when it leaves the live region, so stopping
//!   at the first all-zero slot reads residue as data: on `ams8000_0001` that
//!   invented 7025 leaf entries and 6790 duplicate RefNos.
//! * An internal entry's key is the *least* key of the subtree it points at, so
//!   that child covers `[K_i, K_{i+1})` and routing takes the last key
//!   `<= target`. Every comment in the old code said "greatest"; only the code
//!   was right.
//!
//! `packed` is read here as `offset_words << 12 | flag`, with `offset_words`
//! in 2-byte words. That is a re-scaling of what core.dll writes, not its own
//! layout. `sub_5AE9DF0` packs a leaf value as `ext | (word_offset << 13)` and
//! `sub_5AE9960` unpacks it as `ext = packed & 0x1FFF` — 13 bits, the extent
//! the record's page lives in (1 = the primary file, 0 = a not-yet-saved temp
//! page) — and `record = page + 4 * ((packed >> 13) & 0xFFF)` — 12 bits of
//! **4-byte** words. `2 * (packed >> 12)` and `4 * (packed >> 13)` are the
//! same byte offset whenever bit 12, the top bit of `ext`, is clear, i.e.
//! `ext < 4096`; `flag` is then the low 12 bits of `ext`. The corpus cannot
//! tell the two readings apart: `flag` is 1 on all 940 694 leaf entries
//! because every sample is single-extent, and the split measurement that
//! refuted bit 13 assumed 2-byte units — in core.dll's 4-byte units bit 13
//! is exactly the split. Bits 8 to 11 stay refuted in either unit: between
//! 426 924 and 849 009 of the entries would start past the end of their page.
//! A multi-extent database would carry `ext != 1` here; this module keeps
//! `flag` observable for that day and otherwise does not act on it.

pub mod cursor;
pub mod diff;
pub mod named;
pub mod reference;

use std::cmp::Ordering;

use crate::error::E3dReaderError;
use crate::meta::constants::DATA_PAGE_INDEX;
use crate::refno::{PageAddress, RefNo};

pub use cursor::{
    IndexCursor, IndexTreeError, IndexWalk, IndexedRecord, Lookup, RecordLoc, RecordPosition,
};
pub use diff::{
    CompletedIndexDiff, DiffDiagnostics, DiffTally, FlagOnlyDrift, IndexCandidate, IndexDiff,
    KeyChange, Pruning,
};
pub use named::{
    encode_name, name_table_hash, NamedEntry, NamedKey, NamedTable, NamedTableError, NamedWalk,
};
pub use reference::{ReferenceEntry, ReferenceTable, ReferenceTableError, ReferenceWalk};

/// `page_type` of an index page. core.dll checks it in `sub_5B026C0` and fails
/// with "Page is not a table page".
pub const INDEX_PAGE_TYPE: u32 = 5;

const DWORD: usize = 4;
const HEADER_DWORDS: usize = 7;

/// Key and value widths a page may leave at zero, meaning "the usual pair".
/// No page in the 429-file corpus does, but the widths are declared per page.
const DEFAULT_KEY_DWORDS: u32 = 2;
const DEFAULT_VALUE_DWORDS: u32 = 2;

/// A key must hold both RefNo words; a value must hold `page_no` and `packed`.
const REFNO_DWORDS: u32 = 2;
const VALUE_MIN_DWORDS: u32 = 2;

/// Both key words of the sentinel entry. core.dll matches it as the signed
/// `-2147483647`.
const SENTINEL_WORD: u32 = 0x8000_0001;

const OFFSET_SHIFT: u32 = 12;
const FLAG_MASK: u32 = (1 << OFFSET_SHIFT) - 1;

/// Why a page could not be read as an index page.
///
/// Every rejection is one of these. A malformed page never decodes to an empty
/// entry list, because a caller cannot tell that apart from a page that is
/// genuinely empty.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IndexPageError {
    #[error("page is {len} bytes, too small for the {needed}-byte index page header")]
    PageTooSmall { len: usize, needed: usize },

    #[error("page length {len} is not a whole number of 4-byte words")]
    PageNotDwordAligned { len: usize },

    #[error("page_type is {found}, expected {expected} for an index page")]
    UnexpectedPageType { found: u32, expected: u32 },

    #[error("noun is {found:#x}, expected {expected:#x} for the RefNo index table")]
    UnexpectedNoun { found: u32, expected: u32 },

    #[error("key width of {key_dwords} dwords cannot hold a RefNo, which needs {needed}")]
    KeyTooNarrow { key_dwords: u32, needed: u32 },

    #[error("value width of {value_dwords} dwords cannot hold page_no and the packed word, which need {needed}")]
    ValueTooNarrow { value_dwords: u32, needed: u32 },

    #[error("free_dwords is {free_dwords} but only {usable_dwords} dwords follow the header")]
    FreeSpaceOutOfRange {
        free_dwords: u32,
        usable_dwords: u32,
    },

    #[error("{used_dwords} dwords are in use, which is not a whole number of {stride_dwords}-dword entries")]
    FreeSpaceNotEntryAligned {
        used_dwords: u64,
        stride_dwords: u64,
    },
}

impl From<IndexPageError> for E3dReaderError {
    fn from(error: IndexPageError) -> Self {
        match error {
            IndexPageError::UnexpectedPageType { found, .. } => {
                E3dReaderError::UnsupportedPageType(found)
            }
            other => E3dReaderError::Other(other.to_string()),
        }
    }
}

/// One index entry: a key and what it addresses.
///
/// In a leaf the address is a record — `page_no` plus `offset_words`, which
/// counts 2-byte words from the start of that page (twice core.dll's 4-byte
/// word offset; see the module doc), and `flag`, the low 12 bits of the extent
/// number of that page — 1 on every sample. In an internal page it is the
/// child page holding every key from `refno` up to the next entry's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    pub refno: RefNo,
    pub page_no: u32,
    pub offset_words: u32,
    pub flag: u16,
}

impl IndexEntry {
    /// The sentinel entry, whose key is a page-local constant rather than a
    /// RefNo. In an internal page it points at the left-most child, the one
    /// holding everything below the first real key; in a leaf it is not data.
    pub fn is_start_marker(&self) -> bool {
        self.refno.word0 == SENTINEL_WORD && self.refno.word1 == SENTINEL_WORD
    }

    /// **The 13/12 split below is applied to the wrong word.** [`PageAddress`]
    /// is core.dll's own decode of a leaf value's second word (`sub_5AE9960`):
    /// `word1 & 0x1FFF` is the extent number of the page and
    /// `(word1 >> 13) & 0xFFF` the record's offset in 4-byte words. Applied to
    /// `packed` it would give `(ext, word_offset)`; applied to `offset_words`
    /// — already shifted, and in 2-byte units — it gives nothing. On a
    /// 2048-byte page every `offset_words` fits in 13 bits, so `slot_index`
    /// comes out zero and nothing has noticed.
    ///
    /// It survives only because `ElementRecordView::address` is written and
    /// never read — the record reader addresses bytes with `page_no` and its
    /// own byte offset. Retire the field, or rebuild it from the packed word
    /// (`(offset_words << 12) | flag`) if a caller ever needs the extent.
    pub fn to_page_address(&self) -> PageAddress {
        PageAddress {
            page_number: self.page_no,
            slot_offset: (self.offset_words & 0x1FFF) as u16,
            slot_index: ((self.offset_words >> 13) & 0xFFF) as u16,
        }
    }
}

/// A validated index page still sitting in the caller's buffer.
///
/// Header is seven dwords, 28 bytes: `page_type`, `noun`, `level`,
/// `key_dwords`, `value_dwords`, `reserved`, `free_dwords`. Entries follow,
/// `key_dwords + value_dwords` dwords each, and are read out one at a time
/// rather than collected — a descent visits one node per level and wants one
/// entry from each, so materialising all 126 of them per level is 126 times
/// the work and an allocation the page layer went out of its way to avoid.
///
/// [`IndexPageView`] is the owned form, for callers that do want the whole
/// page; it is this type plus a `collect`.
#[derive(Debug, Clone, Copy)]
pub struct IndexNode<'a> {
    page: &'a [u8],
    pub page_type: u32,
    pub noun: u32,
    pub level: u32,
    pub key_dwords: u32,
    pub value_dwords: u32,
    pub reserved: u32,
    pub free_dwords: u32,
    count: usize,
    stride_bytes: usize,
    value_offset: usize,
}

impl<'a> IndexNode<'a> {
    /// Validate a page as an index node, saying why if it is not one.
    ///
    /// The only place these bytes are judged. [`IndexPageView::decode`] and the
    /// tree walk in [`cursor`] both come here, so "is this an index page" has
    /// one answer.
    pub fn decode(page: &'a [u8]) -> Result<Self, IndexPageError> {
        if page.len() % DWORD != 0 {
            return Err(IndexPageError::PageNotDwordAligned { len: page.len() });
        }
        if page.len() < HEADER_DWORDS * DWORD {
            return Err(IndexPageError::PageTooSmall {
                len: page.len(),
                needed: HEADER_DWORDS * DWORD,
            });
        }

        let page_type = be32(page, 0x00);
        if page_type != INDEX_PAGE_TYPE {
            return Err(IndexPageError::UnexpectedPageType {
                found: page_type,
                expected: INDEX_PAGE_TYPE,
            });
        }
        let noun = be32(page, 0x04);
        if noun != DATA_PAGE_INDEX {
            return Err(IndexPageError::UnexpectedNoun {
                found: noun,
                expected: DATA_PAGE_INDEX,
            });
        }

        let level = be32(page, 0x08);
        let key_dwords = or_default(be32(page, 0x0C), DEFAULT_KEY_DWORDS);
        let value_dwords = or_default(be32(page, 0x10), DEFAULT_VALUE_DWORDS);
        let reserved = be32(page, 0x14);
        let free_dwords = be32(page, 0x18);

        if key_dwords < REFNO_DWORDS {
            return Err(IndexPageError::KeyTooNarrow {
                key_dwords,
                needed: REFNO_DWORDS,
            });
        }
        if value_dwords < VALUE_MIN_DWORDS {
            return Err(IndexPageError::ValueTooNarrow {
                value_dwords,
                needed: VALUE_MIN_DWORDS,
            });
        }

        let usable_dwords = page.len() / DWORD - HEADER_DWORDS;
        if free_dwords as usize > usable_dwords {
            return Err(IndexPageError::FreeSpaceOutOfRange {
                free_dwords,
                usable_dwords: usable_dwords as u32,
            });
        }

        let stride_dwords = key_dwords as u64 + value_dwords as u64;
        let used_dwords = usable_dwords as u64 - free_dwords as u64;
        if used_dwords % stride_dwords != 0 {
            return Err(IndexPageError::FreeSpaceNotEntryAligned {
                used_dwords,
                stride_dwords,
            });
        }

        Ok(Self {
            page,
            page_type,
            noun,
            level,
            key_dwords,
            value_dwords,
            reserved,
            free_dwords,
            count: (used_dwords / stride_dwords) as usize,
            stride_bytes: stride_dwords as usize * DWORD,
            value_offset: key_dwords as usize * DWORD,
        })
    }

    /// Live entries on this page, as `free_dwords` reports them.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn is_leaf(&self) -> bool {
        self.level == 0
    }

    /// Entry `slot`, or `None` past the live region.
    pub fn entry(&self, slot: usize) -> Option<IndexEntry> {
        if slot >= self.count {
            return None;
        }
        let key_at = HEADER_DWORDS * DWORD + slot * self.stride_bytes;
        let value_at = key_at + self.value_offset;
        let packed = be32(self.page, value_at + 4);
        Some(IndexEntry {
            refno: RefNo::new(be32(self.page, key_at), be32(self.page, key_at + 4)),
            page_no: be32(self.page, value_at),
            offset_words: packed >> OFFSET_SHIFT,
            flag: (packed & FLAG_MASK) as u16,
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = IndexEntry> + '_ {
        (0..self.count).map(|slot| self.entry(slot).expect("slot is inside the live region"))
    }

    pub fn find(&self, target: RefNo) -> Option<IndexEntry> {
        find_key(self.entries(), target)
    }

    pub fn leftmost_child(&self) -> Option<u32> {
        leftmost_child(self.entries())
    }

    pub fn select_child(&self, target: RefNo) -> Option<u32> {
        choose_child(self.entries(), target)
    }

    /// Collect into the owned form.
    pub fn to_view(&self) -> IndexPageView {
        IndexPageView {
            page_type: self.page_type,
            noun: self.noun,
            level: self.level,
            key_dwords: self.key_dwords,
            value_dwords: self.value_dwords,
            reserved: self.reserved,
            free_dwords: self.free_dwords,
            entries: self.entries().collect(),
        }
    }
}

/// A decoded B+tree index page, entries and all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexPageView {
    pub page_type: u32,
    pub noun: u32,
    pub level: u32,
    /// Declared key width. Zero means [`DEFAULT_KEY_DWORDS`]; the field here
    /// carries the width actually used.
    pub key_dwords: u32,
    /// Declared value width, defaulted the same way as `key_dwords`.
    pub value_dwords: u32,
    pub reserved: u32,
    /// Dwords left free after the last live entry, and the only thing that says
    /// how many entries the page holds. Older code called this `pfno` and threw
    /// it away.
    pub free_dwords: u32,
    pub entries: Vec<IndexEntry>,
}

impl IndexPageView {
    /// Decode a page, saying why if it is not one.
    pub fn decode(page: &[u8]) -> Result<Self, IndexPageError> {
        IndexNode::decode(page).map(|node| node.to_view())
    }

    /// [`decode`](Self::decode) with the reason folded into the crate error.
    pub fn parse(page: &[u8]) -> Result<Self, E3dReaderError> {
        Self::decode(page).map_err(E3dReaderError::from)
    }

    pub fn is_leaf(&self) -> bool {
        self.level == 0
    }

    pub fn find_in_leaf(&self, target: RefNo) -> Option<&IndexEntry> {
        self.entries.iter().find(|entry| keys_match(entry, target))
    }

    /// The left-most child: everything below the first key lives under it.
    pub fn leftmost_child(&self) -> Option<u32> {
        leftmost_child(self.entries.iter().copied())
    }

    /// The child to descend into for `target`. See [`choose_child`].
    pub fn select_child(&self, target: RefNo) -> Option<u32> {
        choose_child(self.entries.iter().copied(), target)
    }
}

/// The child to descend into for `target`, and **the only routing rule in the
/// crate** — a tree built one way cannot be navigated another.
///
/// Enumeration ([`cursor::IndexWalk`]) and two-root differencing
/// ([`diff::IndexDiff`]) take every child in slot order and so never route,
/// but they read an internal key the same way this does: as the least key of
/// the subtree behind it. `diff` bounds a subtree by that key to decide it can
/// be skipped, which is the same claim as the routing below and would fail
/// with it.
///
/// A key is the least key of the subtree behind it, so the only child that can
/// hold `target` is the one behind the greatest key `<= target`. Taking the
/// *smallest* key `>= target` instead — what the old comments described —
/// misses every element that opens a subtree: on `ams8000_0001` that is 6439 of
/// 6605 leaf entries.
///
/// Below the first key the answer is the left-most child, which is the sentinel
/// entry's; a page with no sentinel falls back to its first entry, which is
/// what the reader this replaced did.
pub fn choose_child(entries: impl IntoIterator<Item = IndexEntry>, target: RefNo) -> Option<u32> {
    let mut best: Option<IndexEntry> = None;
    let mut leftmost: Option<IndexEntry> = None;
    for entry in entries {
        if entry.is_start_marker() {
            if leftmost.map_or(true, |left| !left.is_start_marker()) {
                leftmost = Some(entry);
            }
            continue;
        }
        if leftmost.is_none() {
            leftmost = Some(entry);
        }
        if cmp_refno(entry.refno, target) == Ordering::Greater {
            continue;
        }
        if best.map_or(true, |b| {
            cmp_refno(entry.refno, b.refno) == Ordering::Greater
        }) {
            best = Some(entry);
        }
    }
    best.or(leftmost).map(|entry| entry.page_no)
}

/// The left-most child: everything below the first key lives under it.
pub fn leftmost_child(entries: impl IntoIterator<Item = IndexEntry>) -> Option<u32> {
    let mut first: Option<IndexEntry> = None;
    for entry in entries {
        if entry.is_start_marker() {
            return Some(entry.page_no);
        }
        if first.is_none() {
            first = Some(entry);
        }
    }
    first.map(|entry| entry.page_no)
}

/// The entry for `target`, if the page holds one.
///
/// First match wins. The corpus holds no page with a repeated key, and the
/// enumeration gate would go red if one appeared, so "first" and "only" are the
/// same answer here — but the tie-break is core.dll's, which takes the first
/// sighting of a key and does not consult `flag` to break the tie.
pub fn find_key(
    entries: impl IntoIterator<Item = IndexEntry>,
    target: RefNo,
) -> Option<IndexEntry> {
    entries.into_iter().find(|entry| keys_match(entry, target))
}

/// A real entry for this key: the sentinel's key is a page-local constant, so
/// it never answers a lookup even if a target happened to equal it.
fn keys_match(entry: &IndexEntry, target: RefNo) -> bool {
    !entry.is_start_marker() && entry.refno == target
}

/// Keys order as an unsigned pair. `word1` is signed elsewhere, but reading it
/// that way puts pages out of order.
pub fn cmp_refno(a: RefNo, b: RefNo) -> Ordering {
    (a.word0, a.word1).cmp(&(b.word0, b.word1))
}

fn or_default(declared: u32, fallback: u32) -> u32 {
    if declared == 0 {
        fallback
    } else {
        declared
    }
}

fn be32(page: &[u8], byte_offset: usize) -> u32 {
    u32::from_be_bytes(
        page[byte_offset..byte_offset + 4]
            .try_into()
            .expect("slice is exactly four bytes"),
    )
}

/// Result of a B+tree index search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entry: IndexEntry,
    pub leaf_page_no: u32,
}
