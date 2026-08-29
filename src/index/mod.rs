use std::cmp::Ordering;

use nom::number::complete::be_u32;
use nom::IResult;

use crate::error::E3dReaderError;
use crate::meta::constants::DATA_PAGE_INDEX;
use crate::refno::{PageAddress, RefNo};

const ENTRY_SIZE: usize = 16;
const START_MARKER: u32 = 0x8000_0001;

#[derive(Debug, Clone, Copy)]
pub struct IndexEntry {
    pub refno: RefNo,
    pub page_no: u32,
    pub offset_words: u32,
    pub flag: u16,
}

impl IndexEntry {
    pub fn is_start_marker(&self) -> bool {
        self.refno.word0 == START_MARKER && self.refno.word1 == START_MARKER
    }

    pub fn to_page_address(&self) -> PageAddress {
        PageAddress {
            page_number: self.page_no,
            slot_offset: (self.offset_words & 0x1FFF) as u16,
            slot_index: ((self.offset_words >> 13) & 0xFFF) as u16,
        }
    }
}

/// Parsed view of a B+tree index page.
///
/// Layout from pdmsdb_engine_v2/db3/index.rs:
/// - Header: 28 bytes (page_type, noun=0xCC47DF, level, 3 unknowns, pfno)
/// - Entries: 16 bytes each (refno.hi, refno.lo, page_no, packed)
#[derive(Debug, Clone)]
pub struct IndexPageView {
    pub page_type: u32,
    pub noun: u32,
    pub level: u32,
    pub pfno: u32,
    pub entries: Vec<IndexEntry>,
}

impl IndexPageView {
    pub fn parse(page: &[u8]) -> Result<Self, E3dReaderError> {
        parse_index_page(page)
            .map(|(_, view)| view)
            .map_err(|_| E3dReaderError::Other("failed to parse index page".into()))
    }

    pub fn is_leaf(&self) -> bool {
        self.level == 0
    }

    pub fn find_in_leaf(&self, target: RefNo) -> Option<&IndexEntry> {
        self.entries
            .iter()
            .find(|e| !e.is_start_marker() && e.refno == target)
    }

    pub fn select_child(&self, target: RefNo) -> Option<u32> {
        let normals: Vec<&IndexEntry> = self
            .entries
            .iter()
            .filter(|e| !e.is_start_marker())
            .collect();

        if normals.is_empty() {
            return None;
        }

        if cmp_refno(target, normals[0].refno).is_lt() {
            let start = self.entries.iter().find(|e| e.is_start_marker());
            return Some(start.unwrap_or(normals[0]).page_no);
        }

        let mut prev = normals[0];
        for entry in &normals {
            match cmp_refno(target, entry.refno) {
                Ordering::Less => return Some(prev.page_no),
                Ordering::Equal => return Some(entry.page_no),
                Ordering::Greater => prev = entry,
            }
        }

        Some(prev.page_no)
    }
}

fn cmp_refno(a: RefNo, b: RefNo) -> Ordering {
    (a.word0, a.word1).cmp(&(b.word0, b.word1))
}

fn parse_index_page(input: &[u8]) -> IResult<&[u8], IndexPageView> {
    use nom::Parser;
    let (rest, page_type) = be_u32.parse(input)?;
    let (rest, noun) = be_u32.parse(rest)?;
    if noun != DATA_PAGE_INDEX {
        return Err(nom::Err::Failure(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        )));
    }
    let (rest, level) = be_u32.parse(rest)?;
    let (rest, _unk0) = be_u32.parse(rest)?;
    let (rest, _unk1) = be_u32.parse(rest)?;
    let (rest, _unk2) = be_u32.parse(rest)?;
    let (rest, pfno) = be_u32.parse(rest)?;

    let mut entries = Vec::new();
    let mut remaining = rest;
    while remaining.len() >= ENTRY_SIZE {
        if remaining[..ENTRY_SIZE].iter().all(|&b| b == 0) {
            break;
        }
        let (r, hi) = be_u32.parse(remaining)?;
        let (r, lo) = be_u32.parse(r)?;
        let (r, page_no) = be_u32.parse(r)?;
        let (r, packed) = be_u32.parse(r)?;
        entries.push(IndexEntry {
            refno: RefNo::new(hi, lo),
            page_no,
            offset_words: packed >> 12,
            flag: (packed & 0x0FFF) as u16,
        });
        remaining = r;
    }

    Ok((
        remaining,
        IndexPageView {
            page_type,
            noun,
            level,
            pfno,
            entries,
        },
    ))
}

/// Result of a B+tree index search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entry: IndexEntry,
    pub leaf_page_no: u32,
}
