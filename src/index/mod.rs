//! B+tree index page decoding: bytes in, structure out. No I/O lives here.
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
//! `packed` splits at bit 12 into `offset_words << 12 | flag`. Every other split
//! from bit 8 to bit 16 is refuted by the 940 694 leaf entries in that corpus:
//! at bit 11 and below, between 426 924 and 849 009 of them place the record
//! start past the end of the page it names; at bit 13 and above, between
//! 433 395 and 523 893 place it off a 4-byte boundary. At bit 12, neither
//! happens even once. `flag` itself is 1 on all 940 694 of them — the corpus
//! says nothing more about it, so neither does this module.

use std::cmp::Ordering;

use crate::error::E3dReaderError;
use crate::meta::constants::DATA_PAGE_INDEX;
use crate::refno::{PageAddress, RefNo};

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
/// counts 2-byte words from the start of that page. In an internal page it is
/// the child page holding every key from `refno` up to the next entry's key.
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

    /// **The 13/12 split below is not a reading of `offset_words`.** It is the
    /// bit layout of a RefNo's `word1` ([`PageAddress::from_word1`]), applied
    /// here to a field that the corpus says is a single count of 2-byte words
    /// from the top of the page. On a 2048-byte page every real offset fits in
    /// the low 13 bits, so `slot_index` comes out zero and nothing has noticed.
    ///
    /// It survives only because `ElementRecordView::address` is written and
    /// never read — the record reader addresses bytes with `page_no` and its
    /// own byte offset. Retire the field rather than invent a meaning for this
    /// split; guessing one would make it a sixth reading of a field that took
    /// 429 databases to settle.
    pub fn to_page_address(&self) -> PageAddress {
        PageAddress {
            page_number: self.page_no,
            slot_offset: (self.offset_words & 0x1FFF) as u16,
            slot_index: ((self.offset_words >> 13) & 0xFFF) as u16,
        }
    }
}

/// A decoded B+tree index page.
///
/// Header is seven dwords, 28 bytes: `page_type`, `noun`, `level`,
/// `key_dwords`, `value_dwords`, `reserved`, `free_dwords`. Entries follow,
/// `key_dwords + value_dwords` dwords each.
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
        let count = (used_dwords / stride_dwords) as usize;

        let stride_bytes = stride_dwords as usize * DWORD;
        let value_offset = key_dwords as usize * DWORD;
        let entries = (0..count)
            .map(|i| {
                let key_at = HEADER_DWORDS * DWORD + i * stride_bytes;
                let value_at = key_at + value_offset;
                let packed = be32(page, value_at + 4);
                IndexEntry {
                    refno: RefNo::new(be32(page, key_at), be32(page, key_at + 4)),
                    page_no: be32(page, value_at),
                    offset_words: packed >> OFFSET_SHIFT,
                    flag: (packed & FLAG_MASK) as u16,
                }
            })
            .collect();

        Ok(Self {
            page_type,
            noun,
            level,
            key_dwords,
            value_dwords,
            reserved,
            free_dwords,
            entries,
        })
    }

    /// [`decode`](Self::decode) with the reason folded into the crate error.
    pub fn parse(page: &[u8]) -> Result<Self, E3dReaderError> {
        Self::decode(page).map_err(E3dReaderError::from)
    }

    pub fn is_leaf(&self) -> bool {
        self.level == 0
    }

    pub fn find_in_leaf(&self, target: RefNo) -> Option<&IndexEntry> {
        self.entries
            .iter()
            .find(|entry| !entry.is_start_marker() && entry.refno == target)
    }

    /// The left-most child: everything below the first key lives under it.
    pub fn leftmost_child(&self) -> Option<u32> {
        self.entries
            .iter()
            .find(|entry| entry.is_start_marker())
            .or_else(|| self.entries.first())
            .map(|entry| entry.page_no)
    }

    /// The child to descend into for `target`.
    ///
    /// A key is the least key of the subtree behind it, so the only child that
    /// can hold `target` is the one behind the greatest key `<= target`. Taking
    /// the *smallest* key `>= target` instead — what the old comments described
    /// — misses every element that opens a subtree: on `ams8000_0001` that is
    /// 6439 of 6605 leaf entries.
    pub fn select_child(&self, target: RefNo) -> Option<u32> {
        let mut best: Option<&IndexEntry> = None;
        for entry in self.entries.iter().filter(|e| !e.is_start_marker()) {
            if cmp_refno(entry.refno, target) == Ordering::Greater {
                continue;
            }
            if best.map_or(true, |b| {
                cmp_refno(entry.refno, b.refno) == Ordering::Greater
            }) {
                best = Some(entry);
            }
        }
        best.map(|entry| entry.page_no)
            .or_else(|| self.leftmost_child())
    }
}

/// Keys order as an unsigned pair. `word1` is signed elsewhere, but reading it
/// that way puts pages out of order.
fn cmp_refno(a: RefNo, b: RefNo) -> Ordering {
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
