//! L3: an element record, and exactly the bytes that belong to it.
//!
//! A record states its own length. The first word of its header counts the
//! implicit region in 4-byte words; each tail block that follows states its
//! own length, which element it belongs to, and — when it runs to the end of
//! its page — the address the rest of it lives at. Nothing has to be searched
//! for, and [`read_record`] searches for nothing.
//!
//! A record states where its own parts are. Words 6/7 of the header address
//! its explicit attribute stream and words 8/9 its member list, both as
//! `(page_no, packed)` pairs in the encoding an index leaf value uses, and
//! each block states the same way where the rest of it lives. Across 438
//! sample databases and 2 923 428 records, all 3 195 015 blocks those
//! addresses reach hold a block of the right element, and every one is of the
//! kind its slot implies.
//!
//! The reader this replaces searched instead. It appended whole pages until it
//! found an aligned `0x00000007` past everything it recognised and called that
//! the end, and it found the blocks by expecting them to sit one after another
//! behind the implicit region. Against the 429 databases the index layer was
//! settled on:
//!
//! * that word sits at the true end of 29 027 of 789 831 records — 3.7% — so
//!   on the rest the reader stopped at the next record's header word or at a
//!   value inside it, and handed over a buffer longer than the record;
//! * on 1 033 records it found no such word inside its 64 KiB safety limit and
//!   reported an intact record as truncated;
//! * the padding-skip it used to bridge a gap between blocks fired on 36 106
//!   records, every single one of which then ran past the page it started on:
//!   it was splicing a later copy of the same element onto the record;
//! * and adjacency misses blocks outright. 46 566 blocks on 40 263 records are
//!   not adjacent to anything — the ROOM_NO definition 13292/122 of
//!   `ams5100_0001` ends 24 bytes from the end of its page, too few for a
//!   block, so its whole attribute stream sits on the next page and its
//!   record's bytes point nowhere. Read by adjacency it has no attributes, no
//!   name, and no list of the nouns it applies to.
//!
//! Adjacency never finds a block the stated addresses do not.
//!
//! See [`block`] for a block's own layout and the counts behind it.

pub mod atnlog;
pub mod attrs;
pub mod axis_spec;
pub mod block;
pub mod catalogue_expr;
pub mod catalogue_pml;
pub mod descriptor;
pub mod direction_spec;
pub mod element;
pub mod explicit;
pub mod orientation;
pub mod point_list;
pub mod template;
pub mod template_file;
pub mod text;

#[cfg(test)]
mod descriptor_export_tests {
    #[test]
    fn descriptor_module_exports_extraction_status() {
        let status = super::descriptor::AttributeExtractionStatus::Decoded;
        assert_eq!(
            status,
            super::descriptor::AttributeExtractionStatus::Decoded
        );
    }
}

use std::collections::HashSet;

use crate::error::E3dReaderError;
use crate::index::IndexEntry;
use crate::page::cache::PageCache;
use crate::page::PageId;
use crate::record::block::{block_at, Address, BlockKind};
use crate::refno::{PageAddress, RefNo};

/// Words of header every record starts with, which the implicit length counts:
///
/// ```text
/// 0      implicit length, in 4-byte words
/// 1..2   RefNo
/// 3      noun hash
/// 4..5   owner's RefNo
/// 6..7   where the explicit attribute stream starts
/// 8..9   where the member list starts
/// 10     storage format word
/// ```
///
/// Attribute storage begins at word 11, which is where the noun descriptors
/// place the first one. No record in the sample corpus declares fewer.
const HEADER_WORDS: u32 = 11;

/// Byte offset of the header slot addressing the explicit attribute stream.
const EXPLICIT_SLOT: usize = 24;
/// Byte offset of the header slot addressing the member list.
const MEMBERS_SLOT: usize = 32;

/// The first word of every page an element record lives on.
///
/// Over the 429-file corpus, the 106 807 pages holding any part of any live
/// record all state 7, and no index leaf value or block continuation ever
/// lands on a page that does not
/// (`docs/evidence/2026-08-30-e3d-io-data-page-header.md`). So a page stating
/// anything else is refused before a byte of it is believed — a wrong page
/// number used to be discovered only if the record it happened to describe
/// failed to parse, or not at all.
const ELEMENT_PAGE: u32 = 7;

/// The eleven-word header every record starts with, without its blocks.
///
/// Everything here is read from the header alone — one page read, no chain
/// walking — which is what a whole-database scan wants for the fields a
/// derived index is built from: the noun is word 3, the owner words 4/5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordHeader {
    /// Length of the implicit region in 4-byte words, the header included.
    pub implicit_words: u32,
    /// The RefNo the record states for itself — the caller's to confirm.
    pub refno: RefNo,
    pub noun_hash: u32,
    pub owner: RefNo,
}

/// Structured view of an element record read from the database.
#[derive(Debug, Clone)]
pub struct ElementRecordView {
    pub refno: RefNo,
    pub address: PageAddress,
    /// The record and nothing else: header, implicit region and every tail
    /// block, laid end to end. `ParsedElement::parse` consumes all of it and
    /// stops exactly at its end.
    pub raw_bytes: Vec<u8>,
    pub page_no: u32,
    pub byte_offset: u32,
}

/// One element record's bytes, gathered from every page they live on.
#[derive(Debug, Clone)]
pub struct RecordBytes {
    /// The RefNo the record states for itself, which is not necessarily the
    /// one the caller looked up: deciding whether they agree belongs to the
    /// caller, not to the reader.
    pub refno: RefNo,
    pub noun_hash: u32,
    pub owner: RefNo,
    /// Header, implicit region, then the member list and the explicit stream,
    /// laid end to end. The parts are gathered from wherever the record says
    /// they are, so the result is contiguous whether or not the record was.
    pub bytes: Vec<u8>,
    pub blocks: u32,
    /// Pages the record's parts were gathered from, the header's included.
    pub pages: u32,
}

/// Why a record could not be read.
///
/// Every one of these names the page and byte it happened at. None of them is
/// an empty record or a short one: a caller cannot tell a record that stops
/// early from a record that is genuinely that size, and the reader this
/// replaces made that indistinguishable on 1 033 records of the corpus.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RecordError {
    #[error("{page} could not be read: {detail}")]
    Unreadable { page: PageId, detail: String },

    #[error(
        "{page} is not an element data page: its first word is {word0}, and every page an \
         element record lives on states {ELEMENT_PAGE}"
    )]
    NotAnElementPage { page: PageId, word0: u32 },

    #[error("{page} holds {len} bytes, so nothing starts at byte {at}")]
    PastEndOfPage { page: PageId, at: usize, len: usize },

    #[error(
        "no element record starts at {page} byte {at}: its first word is {first_word}, and an \
         implicit length is at least the {HEADER_WORDS}-word header"
    )]
    NotARecord {
        page: PageId,
        at: usize,
        first_word: u32,
    },

    #[error(
        "{refno} at {page} byte {at} declares {words} implicit words, running {over} bytes past \
         the end of the page"
    )]
    ImplicitOverflowsPage {
        refno: RefNo,
        page: PageId,
        at: usize,
        words: u32,
        over: usize,
    },

    #[error(
        "{refno}: the tail block at {page} byte {at} declares {bytes} bytes, running {over} past \
         the end of the page"
    )]
    BlockOverflowsPage {
        refno: RefNo,
        page: PageId,
        at: usize,
        bytes: usize,
        over: usize,
    },

    #[error(
        "{refno}: {chain} is said to be at {page} byte {at}, and no block of this element is there"
    )]
    ChainMissing {
        refno: RefNo,
        chain: Chain,
        page: PageId,
        at: usize,
    },

    #[error(
        "{refno}: {chain} reaches a {found:?} block at {page} byte {at}, which belongs to the \
         other list"
    )]
    ChainKindMismatch {
        refno: RefNo,
        chain: Chain,
        page: PageId,
        at: usize,
        found: BlockKind,
    },

    #[error("{refno}: {chain} comes back to {page} byte {at}")]
    ChainCycle {
        refno: RefNo,
        chain: Chain,
        page: PageId,
        at: usize,
    },

    #[error(
        "{refno}: {chain} is still going after {blocks} blocks, more than the {limit} pages the \
         file holds"
    )]
    ChainTooLong {
        refno: RefNo,
        chain: Chain,
        blocks: u32,
        limit: u32,
    },
}

/// Which of a record's two lists a failure is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chain {
    Members,
    Explicit,
}

impl Chain {
    const fn kind(self) -> BlockKind {
        match self {
            Self::Members => BlockKind::Members,
            Self::Explicit => BlockKind::Explicit,
        }
    }
}

impl std::fmt::Display for Chain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Members => "the member list",
            Self::Explicit => "the explicit attribute stream",
        })
    }
}

impl From<RecordError> for E3dReaderError {
    fn from(error: RecordError) -> Self {
        E3dReaderError::Record(Box::new(error))
    }
}

/// Read the record at one address, gathering its parts from wherever it says
/// they are.
///
/// Costs one page read per page the record's parts live on — one page for
/// 2 791 030 of the 2 923 428 records in the sample corpus. Nothing here scans
/// a page, and nothing here reads a byte the record did not claim.
pub fn read_record(
    pages: &mut PageCache,
    start: PageId,
    offset_words: u32,
) -> Result<RecordBytes, RecordError> {
    let block_limit = pages.physical_page_count().max(1);
    let at = offset_words as usize * 2;
    let page = page_of(pages, start)?;

    let (header, explicit, members) = header_at(page, start, at)?;
    let RecordHeader {
        implicit_words,
        refno,
        noun_hash,
        owner,
    } = header;

    let implicit_bytes = implicit_words as usize * 4;
    if at + implicit_bytes > page.len() {
        return Err(RecordError::ImplicitOverflowsPage {
            refno,
            page: start,
            at,
            words: implicit_words,
            over: at + implicit_bytes - page.len(),
        });
    }

    let mut record = RecordBytes {
        refno,
        noun_hash,
        owner,
        bytes: page[at..at + implicit_bytes].to_vec(),
        blocks: 0,
        pages: 0,
    };

    // Both lists are laid out behind the implicit region so the tail parser
    // sees one contiguous record. Each block declares its own kind, so the
    // order they arrive in carries no meaning.
    let mut visited = HashSet::from([(start, at)]);
    for (chain, from) in [(Chain::Members, members), (Chain::Explicit, explicit)] {
        let Some(from) = from else { continue };
        gather(
            pages,
            &mut record,
            start.ext,
            chain,
            from,
            &mut visited,
            block_limit,
        )?;
    }
    record.pages = visited
        .iter()
        .map(|(page, _)| *page)
        .collect::<HashSet<_>>()
        .len() as u32;
    Ok(record)
}

/// Append one list, block by block, from where the record says it starts.
fn gather(
    pages: &mut PageCache,
    record: &mut RecordBytes,
    ext: u16,
    chain: Chain,
    from: Address,
    visited: &mut HashSet<(PageId, usize)>,
    block_limit: u32,
) -> Result<(), RecordError> {
    let refno = record.refno;
    let mut page_id = PageId::new(ext, from.page_no);
    let mut at = from.byte_offset();
    let mut walked = 0u32;

    loop {
        if !visited.insert((page_id, at)) {
            return Err(RecordError::ChainCycle {
                refno,
                chain,
                page: page_id,
                at,
            });
        }
        walked += 1;
        if walked > block_limit {
            return Err(RecordError::ChainTooLong {
                refno,
                chain,
                blocks: walked,
                limit: block_limit,
            });
        }

        let page = page_of(pages, page_id)?;
        let Some(block) = block_at(page, at, refno) else {
            return Err(RecordError::ChainMissing {
                refno,
                chain,
                page: page_id,
                at,
            });
        };
        if block.kind != chain.kind() {
            return Err(RecordError::ChainKindMismatch {
                refno,
                chain,
                page: page_id,
                at,
                found: block.kind,
            });
        }
        let end = at + block.total_bytes;
        if end > page.len() {
            return Err(RecordError::BlockOverflowsPage {
                refno,
                page: page_id,
                at,
                bytes: block.total_bytes,
                over: end - page.len(),
            });
        }
        record.bytes.extend_from_slice(&page[at..end]);
        record.blocks += 1;

        let Some(next) = block.continues_at else {
            return Ok(());
        };
        page_id = PageId::new(ext, next.page_no);
        at = next.byte_offset();
    }
}

/// Read a record's eleven-word header and nothing else.
///
/// One page read, no chain walking: the noun, owner and self-stated RefNo at
/// the cost of the page they sit on. `read_record` and this decode the header
/// through the same [`header_at`], so they cannot disagree about it.
pub fn read_record_header(
    pages: &mut PageCache,
    start: PageId,
    offset_words: u32,
) -> Result<RecordHeader, RecordError> {
    let at = offset_words as usize * 2;
    let page = page_of(pages, start)?;
    header_at(page, start, at).map(|(header, _, _)| header)
}

/// Decode the record header at `at`, with the page's bounds respected.
fn header_at(
    page: &[u8],
    page_id: PageId,
    at: usize,
) -> Result<(RecordHeader, Option<Address>, Option<Address>), RecordError> {
    let header = page
        .get(at..)
        .filter(|header| header.len() >= HEADER_WORDS as usize * 4)
        .ok_or(RecordError::PastEndOfPage {
            page: page_id,
            at,
            len: page.len(),
        })?;
    let implicit_words = be32(header, 0);
    if implicit_words < HEADER_WORDS {
        return Err(RecordError::NotARecord {
            page: page_id,
            at,
            first_word: implicit_words,
        });
    }
    Ok((
        RecordHeader {
            implicit_words,
            refno: RefNo::new(be32(header, 4), be32(header, 8)),
            noun_hash: be32(header, 12),
            owner: RefNo::new(be32(header, 16), be32(header, 20)),
        },
        Address::at(header, EXPLICIT_SLOT),
        Address::at(header, MEMBERS_SLOT),
    ))
}

/// Read the record an index entry addresses.
///
/// An [`IndexEntry`] carries a bare page number, so `ext` says which file that
/// number addresses: the extent of the index page that named the entry, which
/// is the same inheritance rule [`crate::index::RecordLoc`] states. This used
/// to be hard-coded to the primary extent, which reads every single-extent
/// database correctly and the wrong file on any other.
///
/// The entry's RefNo is carried through unchecked. Whether the record agrees
/// is L3's caller's question, and the two answers are worth keeping apart: a
/// key that resolves to somebody else's record is a different fact from a key
/// that resolves to nothing.
pub fn read_element_record(
    cache: &mut PageCache,
    ext: u16,
    entry: &IndexEntry,
) -> Result<ElementRecordView, E3dReaderError> {
    let record = read_record(cache, PageId::new(ext, entry.page_no), entry.offset_words)?;
    Ok(ElementRecordView {
        refno: entry.refno,
        address: entry.to_page_address(),
        raw_bytes: record.bytes,
        page_no: entry.page_no,
        byte_offset: entry.offset_words * 2,
    })
}

/// The page at `page`, if it is a page element records live on.
///
/// Every read this module makes comes through here, so the header check
/// covers the record's own page and every page its block chains continue
/// onto alike.
fn page_of(pages: &mut PageCache, page: PageId) -> Result<&[u8], RecordError> {
    let data = pages.get(page).map_err(|source| RecordError::Unreadable {
        page,
        detail: source.to_string(),
    })?;
    let word0 = be32(data, 0);
    if word0 != ELEMENT_PAGE {
        return Err(RecordError::NotAnElementPage { page, word0 });
    }
    Ok(data)
}

/// The four bytes at `at`, which the caller has already bounds-checked.
fn be32(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(data[at..at + 4].try_into().expect("four bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::io::PageReader;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    const TEST_PAGE_SIZE: usize = 128;
    const OWNER: RefNo = RefNo {
        word0: 24_383,
        word1: 85_432,
    };

    struct Db {
        pages: Vec<Vec<u8>>,
    }

    impl Db {
        fn new(pages: usize) -> Self {
            let mut db = Self {
                pages: vec![vec![0u8; TEST_PAGE_SIZE]; pages.max(2)],
            };
            db.put(0, 0x34, TEST_PAGE_SIZE as u32 / 4);
            db
        }

        fn put(&mut self, page: usize, at: usize, value: u32) {
            self.pages[page][at..at + 4].copy_from_slice(&value.to_be_bytes());
        }

        fn words(&mut self, page: usize, at: usize, words: &[u32]) {
            for (i, word) in words.iter().enumerate() {
                self.put(page, at + i * 4, *word);
            }
        }

        /// A record header. `explicit` and `members` are `(page_no, packed)`.
        fn record(
            &mut self,
            page: usize,
            at: usize,
            implicit_words: u32,
            refno: RefNo,
            explicit: (u32, u32),
            members: (u32, u32),
        ) {
            self.put(page, 0, ELEMENT_PAGE);
            self.words(
                page,
                at,
                &[
                    implicit_words,
                    refno.word0,
                    refno.word1,
                    0xC551C,
                    0,
                    0,
                    explicit.0,
                    explicit.1,
                    members.0,
                    members.1,
                    0,
                ],
            );
        }

        /// `[kind][words][owner][cont_pgno][cont_packed]` then `payload`.
        fn block(
            &mut self,
            page: usize,
            at: usize,
            kind: u16,
            owner: RefNo,
            cont: (u32, u32),
            payload: &[u32],
        ) {
            self.put(page, 0, ELEMENT_PAGE);
            let words = block::MIN_WORDS as u32 + payload.len() as u32;
            self.put(page, at, (u32::from(kind) << 16) | words);
            self.words(page, at + 4, &[owner.word0, owner.word1, cont.0, cont.1]);
            self.words(page, at + block::HEADER_BYTES, payload);
        }

        fn open(&self) -> (TempDir, PageCache) {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("ams1000_0001");
            let mut file = File::create(&path).unwrap();
            for page in &self.pages {
                file.write_all(page).unwrap();
            }
            file.flush().unwrap();
            let reader = PageReader::open(&path).unwrap();
            (dir, PageCache::new(reader, 4))
        }
    }

    fn read(db: &Db, page: u32, offset_words: u32) -> Result<RecordBytes, RecordError> {
        let (_dir, mut cache) = db.open();
        read_record(&mut cache, PageId::primary(page), offset_words)
    }

    /// Addresses in a header or a continuation are `(page_no, packed)` and
    /// `packed` counts 2-byte words.
    fn address(page: u32, byte: usize) -> (u32, u32) {
        (page, ((byte as u32 / 2) << 12) | 1)
    }

    #[test]
    fn a_record_is_its_header_its_implicit_region_and_the_blocks_it_names() {
        let mut db = Db::new(2);
        db.record(1, 8, 12, OWNER, address(1, 8 + 48), address(1, 8 + 48 + 28));
        db.block(1, 8 + 48, 1, OWNER, (0, 0), &[0x000A_AFCA, 0x1400_0001]);
        db.block(1, 8 + 48 + 28, 2, OWNER, (0, 0), &[OWNER.word0, 85_433]);
        // Whatever follows belongs to somebody else and must not be read.
        db.words(1, 8 + 48 + 56, &[7, 0x2A, 99, 1]);

        let record = read(&db, 1, 4).expect("the record reads");
        assert_eq!(record.refno, OWNER);
        assert_eq!(record.noun_hash, 0xC551C);
        assert_eq!(record.blocks, 2);
        assert_eq!(record.pages, 1);
        assert_eq!(record.bytes.len(), 48 + 28 + 28);
    }

    /// **Reverting to the `0x00000007` search turns this red.** A record with
    /// no such word after it is 96% of the corpus, and 1 033 of them had no
    /// such word anywhere in the 64 KiB that followed either.
    #[test]
    fn a_record_that_names_no_blocks_is_its_implicit_region() {
        let mut db = Db::new(2);
        db.record(1, 8, 12, OWNER, (0, 0), (0, 0));
        db.words(1, 8 + 48, &[7, 0, 0, 0]);

        let record = read(&db, 1, 4).expect("the record reads");
        assert_eq!(record.blocks, 0);
        assert_eq!(record.bytes.len(), 48);
    }

    /// **Reverting to "the blocks follow the implicit region" turns this red.**
    /// The shape of ROOM_NO 13292/122 in `ams5100_0001`: nothing fits in what
    /// is left of the page, so the whole stream is on the next one and nothing
    /// beside the header points at it.
    #[test]
    fn a_block_that_is_nowhere_near_the_record_is_still_found() {
        let mut db = Db::new(3);
        let implicit_words = (TEST_PAGE_SIZE as u32 - 8) / 4 - 2;
        db.record(1, 8, implicit_words, OWNER, address(2, 4), (0, 0));
        db.block(2, 4, 1, OWNER, (0, 0), &[0x000A_AFCA, 0x1400_0001]);

        let record = read(&db, 1, 4).expect("the record reads");
        assert_eq!(record.blocks, 1);
        assert_eq!(record.pages, 2);
        assert_eq!(
            record.bytes.len(),
            implicit_words as usize * 4 + 28,
            "the block arrives right behind the implicit region"
        );
        assert!(
            block::block_at(&record.bytes, implicit_words as usize * 4, OWNER).is_some(),
            "so the tail parser sees one contiguous record"
        );
    }

    /// The shape of BRAN 24383/85432 in `ams7999_0001`: a member block that
    /// ends at the page boundary and states where the rest of it is.
    #[test]
    fn a_block_that_states_where_it_continues_is_followed_there() {
        let mut db = Db::new(3);
        let first = TEST_PAGE_SIZE - 28;
        db.record(
            1,
            8,
            (first as u32 - 8) / 4,
            OWNER,
            (0, 0),
            address(1, first),
        );
        db.block(1, first, 2, OWNER, address(2, 4), &[OWNER.word0, 85_433]);
        db.block(2, 4, 2, OWNER, (0, 0), &[85_434, 0]);

        let record = read(&db, 1, 4).expect("the record reads");
        assert_eq!(record.blocks, 2);
        assert_eq!(record.pages, 2);
        assert_eq!(
            record.bytes.len(),
            (first - 8) + 28 + 28,
            "the two blocks arrive back to back, with the page boundary gone"
        );
    }

    /// **Reverting to "stop when there is no block there" turns this red.**
    /// A stated address that holds nothing is a record this reader cannot
    /// deliver, not a record with fewer attributes than it has.
    #[test]
    fn an_address_that_holds_no_block_is_an_error_not_a_short_record() {
        let mut db = Db::new(3);
        db.record(1, 8, 12, OWNER, address(2, 4), (0, 0));
        // Page 2 is an element page with nothing at byte 4 — the dangling
        // address is the failure here, not the page's type.
        db.put(2, 0, ELEMENT_PAGE);

        let error = read(&db, 1, 4).unwrap_err();
        assert!(
            matches!(
                error,
                RecordError::ChainMissing { chain: Chain::Explicit, page, at, .. }
                    if page == PageId::primary(2) && at == 4
            ),
            "{error}"
        );
    }

    #[test]
    fn a_slot_reaching_the_other_list_is_refused() {
        let mut db = Db::new(3);
        db.record(1, 8, 12, OWNER, address(1, 8 + 48), (0, 0));
        db.block(1, 8 + 48, 2, OWNER, (0, 0), &[OWNER.word0, 85_433]);

        let error = read(&db, 1, 4).unwrap_err();
        assert!(
            matches!(
                error,
                RecordError::ChainKindMismatch {
                    chain: Chain::Explicit,
                    found: block::BlockKind::Members,
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn a_list_that_points_back_at_itself_is_refused() {
        let mut db = Db::new(3);
        db.record(1, 8, 12, OWNER, (0, 0), address(1, 8 + 48));
        db.block(1, 8 + 48, 2, OWNER, address(1, 8 + 48), &[OWNER.word0]);

        let error = read(&db, 1, 4).unwrap_err();
        assert!(matches!(error, RecordError::ChainCycle { .. }), "{error}");
    }

    #[test]
    fn a_location_that_holds_no_record_says_so() {
        let mut db = Db::new(2);
        db.put(1, 0, ELEMENT_PAGE);
        db.put(1, 8, 3);

        let error = read(&db, 1, 4).unwrap_err();
        assert!(
            matches!(error, RecordError::NotARecord { first_word: 3, .. }),
            "{error}"
        );
    }

    /// **Reverting to an unchecked page read turns this red.** The record on
    /// the page is perfectly well formed; what is wrong is the page — its
    /// first word says index tree, and core.dll refuses a page whose header
    /// is not the one asked for rather than reading records off it.
    #[test]
    fn a_page_that_is_not_an_element_page_is_refused_before_its_bytes_are_believed() {
        let mut db = Db::new(2);
        db.record(1, 8, 12, OWNER, (0, 0), (0, 0));
        // The same page re-labelled as a tree page: 5, then the index tree id.
        db.put(1, 0, 5);
        db.put(1, 4, 13_387_743);

        let error = read(&db, 1, 4).unwrap_err();
        assert!(
            matches!(error, RecordError::NotAnElementPage { word0: 5, .. }),
            "{error}"
        );
        assert!(error.to_string().contains("first word is 5"), "{error}");

        // A chain that leaves the element pages is refused the same way.
        let mut db = Db::new(3);
        db.record(1, 8, 12, OWNER, address(2, 4), (0, 0));
        db.block(2, 4, 1, OWNER, (0, 0), &[0x000A_AFCA, 0x1400_0001]);
        db.put(2, 0, 5);

        let error = read(&db, 1, 4).unwrap_err();
        assert!(
            matches!(
                error,
                RecordError::NotAnElementPage { word0: 5, page }
                    if page == PageId::primary(2)
            ),
            "{error}"
        );
    }

    /// **Reverting to the hard-coded primary extent turns this red.** The
    /// caller says which file an entry's page number addresses; a stated
    /// extent that is not open is [`RecordError::Unreadable`] naming it, not a
    /// quiet read of the same page number in the primary file.
    #[test]
    fn an_entry_is_read_from_the_extent_the_caller_names() {
        use crate::index::IndexEntry;

        let mut db = Db::new(2);
        db.record(1, 8, 12, OWNER, (0, 0), (0, 0));
        let (_dir, mut cache) = db.open();
        let entry = IndexEntry {
            refno: OWNER,
            page_no: 1,
            offset_words: 4,
            flag: 1,
        };

        let view = read_element_record(&mut cache, PageId::PRIMARY, &entry)
            .expect("the primary extent reads");
        assert_eq!(view.refno, OWNER);

        let error = read_element_record(&mut cache, 2, &entry).unwrap_err();
        assert!(
            error.to_string().contains("extent 2"),
            "the missing extent is named, not redirected: {error}"
        );
    }

    #[test]
    fn a_record_longer_than_its_page_is_refused_rather_than_run_on() {
        let mut db = Db::new(3);
        db.record(1, 8, 100, OWNER, (0, 0), (0, 0));

        let error = read(&db, 1, 4).unwrap_err();
        assert!(
            matches!(error, RecordError::ImplicitOverflowsPage { words: 100, .. }),
            "{error}"
        );

        let mut db = Db::new(3);
        db.record(1, 8, 12, OWNER, (0, 0), address(1, 8 + 48));
        db.put(1, 8 + 48, (2 << 16) | 40);
        db.words(1, 8 + 52, &[OWNER.word0, OWNER.word1]);

        let error = read(&db, 1, 4).unwrap_err();
        assert!(
            matches!(error, RecordError::BlockOverflowsPage { bytes: 160, .. }),
            "{error}"
        );
    }
}
