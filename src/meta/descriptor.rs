use super::DbDescriptor;
use crate::error::E3dReaderError;
use crate::page::{PageId, PageSize, PageType};

/// Byte offsets inside page 0, all big-endian words.
mod at {
    pub const PAGE_TYPE: usize = 0x00;
    pub const NOUN: usize = 0x04;
    pub const DB_NUM: usize = 0x08;
    pub const LATEST_SES_PGNO: usize = 0x28;
    pub const EXT_NO: usize = 0x2C;
    pub const FIRST_SES_PGNO: usize = 0x30;
    pub const PAGE_SIZE_WORDS: usize = 0x34;
    pub const PRIMARY_EXTENT_REF: usize = 0x38;
}

impl DbDescriptor {
    pub const PAGE_INDEX: u32 = 0;

    /// Smallest page 0 this parser will read: through `primary_extent_ref`.
    const MIN_BYTES: usize = at::PRIMARY_EXTENT_REF + 4;

    /// Parse the descriptor from page 0.
    ///
    /// Layout, verified word by word against `ams8000_0001`:
    ///
    /// | offset | field | `ams8000_0001` |
    /// |---|---|---|
    /// | +0x00 | page type (0 = descriptor) | 0 |
    /// | +0x04 | noun | 2 |
    /// | +0x08 | db_num | 8000 |
    /// | +0x28 | `latest_ses_pgno` — page of the **newest** session | 8897 |
    /// | +0x2C | `ext_no` of this file | 1 |
    /// | +0x30 | `first_ses_pgno` — page of the **oldest** session | 4 |
    /// | +0x34 | page size, in 4-byte words | 512 (= 2048 bytes) |
    /// | +0x38 | `primary_extent_ref` | 16192 |
    ///
    /// The two session pointers are easy to mistake for each other and are not
    /// interchangeable: page 8897 states `sesno` 264 and page 4 states 1. A
    /// reader that starts from +0x30 expecting the current state gets the
    /// database as it was at its very first session.
    ///
    /// +0x38 was read as a stored page count until a census showed it is the
    /// `RefNo::word0` of the elements stored in the database's first extent:
    /// it decodes back to `db_num` through `RefNo::dbno` with an extent index
    /// of 1, in all 448 databases of `AvevaMarineSample\ams000`. It is a refno
    /// prefix, not a length — `ams7330_0001` states 15522 and is 644 pages
    /// long.
    ///
    /// Byte order: BIG-ENDIAN (Fortran legacy).
    pub fn parse(data: &[u8]) -> Result<Self, E3dReaderError> {
        if data.len() < Self::MIN_BYTES {
            return Err(E3dReaderError::InvalidPage {
                page_no: Self::PAGE_INDEX,
                detail: format!(
                    "descriptor page too small: {} bytes, need {}",
                    data.len(),
                    Self::MIN_BYTES
                ),
            });
        }
        let r = |off: usize| u32::from_be_bytes(data[off..off + 4].try_into().unwrap());

        // Session pages check for type 3 and index pages for type 5; page 0
        // gets the same courtesy. A file whose first word is not 0 is not a
        // database opened at the wrong session, it is not this kind of file.
        let bucket_type_id = r(at::PAGE_TYPE);
        if PageType::from_raw(bucket_type_id) != PageType::Descriptor {
            return Err(E3dReaderError::InvalidPage {
                page_no: Self::PAGE_INDEX,
                detail: format!("page type {bucket_type_id} is not a descriptor page (type 0)"),
            });
        }

        // `extent()` hands this out as a u16 page address. Refusing the
        // overflow here keeps that conversion infallible, where it used to
        // fall back to the primary extent without a word.
        let ext_no = r(at::EXT_NO);
        if u16::try_from(ext_no).is_err() {
            return Err(E3dReaderError::InvalidPage {
                page_no: Self::PAGE_INDEX,
                detail: format!("ext_no {ext_no} does not fit in a u16 extent number"),
            });
        }

        let page_size_words = r(at::PAGE_SIZE_WORDS);
        let page_size = PageSize::from_header_words(page_size_words).map_err(|source| {
            E3dReaderError::InvalidPage {
                page_no: Self::PAGE_INDEX,
                detail: source.to_string(),
            }
        })?;

        Ok(Self {
            bucket_type_id,
            noun: r(at::NOUN),
            page_size: page_size.bytes() as u32,
            page_size_words,
            db_mark: r(at::DB_NUM),
            latest_ses_pgno: r(at::LATEST_SES_PGNO),
            ext_no,
            session_page_no: r(at::FIRST_SES_PGNO),
            primary_extent_ref: r(at::PRIMARY_EXTENT_REF),
        })
    }

    /// Page size as the unit-carrying type. Prefer it over the byte field.
    pub fn page_size_typed(&self) -> PageSize {
        PageSize::from_header_words(self.page_size_words).expect("validated in DbDescriptor::parse")
    }

    /// Address of the newest session page.
    pub fn latest_session_page(&self) -> PageId {
        PageId::new(self.extent(), self.latest_ses_pgno)
    }

    /// Address of the oldest session page.
    pub fn first_session_page(&self) -> PageId {
        PageId::new(self.extent(), self.session_page_no)
    }

    /// This file's extent number, falling back to the primary when the header
    /// states 0 — which no sampled database does. A value past `u16::MAX` was
    /// refused by [`DbDescriptor::parse`], so the conversion cannot truncate.
    fn extent(&self) -> u16 {
        let ext = u16::try_from(self.ext_no).expect("validated in DbDescriptor::parse");
        if ext == 0 {
            PageId::PRIMARY
        } else {
            ext
        }
    }

    /// Page 0 must be a descriptor page.
    pub fn page_type(&self) -> PageType {
        PageType::from_raw(self.bucket_type_id)
    }
}
