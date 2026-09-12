use super::DbDescriptor;
use crate::error::E3dReaderError;
use crate::page::{PageId, PageSize, PageType};

/// Byte offsets inside page 0, all big-endian words.
mod at {
    pub const PAGE_TYPE: usize = 0x00;
    pub const NOUN: usize = 0x04;
    pub const DB_NUM: usize = 0x08;
    pub const DB_TYPE_HASH: usize = 0x20;
    pub const TEMPLATE_VERSION: usize = 0x24;
    pub const LATEST_SES_PGNO: usize = 0x28;
    pub const EXT_NO: usize = 0x2C;
    pub const FIRST_SES_PGNO: usize = 0x30;
    pub const PAGE_SIZE_WORDS: usize = 0x34;
    pub const PRIMARY_EXTENT_REF: usize = 0x38;
    pub const CREATION_INFO_WORDS: usize = 0x40;
    pub const CREATION_INFO: usize = 0x44;
    pub const FORMAT: usize = 0x154;
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
    /// | +0x20 | db1 hash of the database type name | 722578 (`DESI`) |
    /// | +0x24 | signed template version | -1 |
    /// | +0x28 | `latest_ses_pgno` — page of the **newest** session | 8897 |
    /// | +0x2C | `ext_no` of this file | 1 |
    /// | +0x30 | `first_ses_pgno` — page of the **oldest** session | 4 |
    /// | +0x34 | page size, in 4-byte words | 512 (= 2048 bytes) |
    /// | +0x38 | `primary_extent_ref` | 16192 |
    /// | +0x40/+0x44 | `CRINFO` word count / packed ASCII creator banner | varies |
    /// | +0x154 | `FORMAT` text-storage format: 1 = UTF-8 as stored, else the escaped project charset (`&~ … &`, GBK here) — what `core.dll` branches on before reading any string; see [`crate::record::text`] | 0 or 1 |
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
            db_type_hash: r(at::DB_TYPE_HASH),
            template_version: r(at::TEMPLATE_VERSION) as i32,
            latest_ses_pgno: r(at::LATEST_SES_PGNO),
            ext_no,
            session_page_no: r(at::FIRST_SES_PGNO),
            primary_extent_ref: r(at::PRIMARY_EXTENT_REF),
            creation_info: creation_info(data),
            header_format: optional_u32(data, at::FORMAT),
        })
    }

    /// The database type name (`DESI`, `CATA`, …), dehashed from +0x20.
    ///
    /// `None` when the word does not decode as a db1 name. The aios reader
    /// (`old-parse-pdms-db::parse_file_basic_info`) reads the same word with
    /// the same guard, so the two stacks agree on what a file claims to be.
    pub fn db_type_name(&self) -> Option<String> {
        e3d_attlib::db1_dehash(self.db_type_hash)
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

    /// RefNo of this database's WORLD element: `primary_extent_ref / 0`.
    ///
    /// Census over every primary database of the installed E3D 2.1 and 3.1
    /// projects (2026-09-12, 1,947 files): each database holds exactly one
    /// `WORL`, its owner is null, and for all 1,905 element/catalogue/dictionary
    /// databases (DESI, CATA, DICT, PROP, PADD, ISOD, …) that element is
    /// `RefNo(primary_extent_ref, 0)` — the first extent's word0 with element
    /// number 0. This is what `DB_DB::getWorld` in `core.dll` hands back.
    ///
    /// The three project-administration types are the only exceptions and are
    /// not element databases of any MDB: `COMM` (8190) keeps its WORLD at
    /// element 1, `MISC` (8189) at element 1, `SYST` (8191) in its second
    /// extent at element 3. Callers that route through an MDB's CURD never
    /// meet them; anyone opening those files must scan for the `WORL` noun.
    ///
    /// The descriptor only states where the WORLD *should* be. Confirm it
    /// with [`crate::ReadOnlyEngine::world`], which point-looks it up and
    /// checks the record's noun, before treating the address as fact.
    pub fn world_refno(&self) -> crate::refno::RefNo {
        crate::refno::RefNo::new(self.primary_extent_ref, 0)
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

fn optional_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn creation_info(data: &[u8]) -> Option<String> {
    let words = optional_u32(data, at::CREATION_INFO_WORDS)? as usize;
    let bytes = words.checked_mul(4)?;
    let stored = data.get(at::CREATION_INFO..at::CREATION_INFO.checked_add(bytes)?)?;
    let text: String = stored
        .iter()
        .copied()
        .take_while(|byte| *byte != 0)
        .take_while(|byte| byte.is_ascii_graphic() || *byte == b' ')
        .map(char::from)
        .collect();
    (!text.is_empty()).then_some(text)
}
