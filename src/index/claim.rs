//! The `(5, 7618377)` tree — the one the session page roots at `+0x24` and
//! this crate names `claim`.
//!
//! `ENHAS2` decodes `7618377` as `CLAIM`, and `core.dll`'s extract/multiwriter
//! model does keep a claim table under that name. But in the single-writer,
//! non-extract databases the corpus is made of, the tree that is actually
//! there is the **primary-element session index**: a refno-keyed B+tree that
//! maps every *primary* element — the name-bearing, navigable hierarchy nouns
//! (`SITE`, `ZONE`, `EQUI`, `PIPE`, `BRAN`, `STRU`, `SUBS`, `SCTN`, …), and
//! none of the components or primitives beneath them — to `(owner, session)`,
//! where `owner` is the claiming user/bucket (0 with no extract) and `session`
//! is the number of the session that last changed the element or anything
//! under it (`docs/evidence/2026-08-30-e3d-io-data-table-tree-7618377.md`, R1;
//! `core.dll` maintains it in `sub_5B0C760`, A3a.6 of the write-path note).
//! On `ams8000_0001` it holds 3 129 entries, every `owner` 0 and every
//! `session` in `1..=640`, partitioning cleanly by noun.
//!
//! The page shape is the refno index's: page type 5, the name hash in word 1,
//! a 2-dword key (the refno) and a 2-dword value, `free_dwords` in word 6, the
//! sentinel opening an internal page by its first key word being
//! [`SENTINEL_WORD`]. The value is the only thing that differs from the refno
//! index — `(owner, session)` here, a record address there — so this reader
//! decodes the pages itself rather than borrowing the index cursor, whose
//! value words mean something else.

use crate::meta::constants::DATA_PAGE_MAIN;
use crate::page::cache::PageCache;
use crate::page::{IoStats, PageId};
use crate::refno::RefNo;
use crate::session::DbView;
use std::collections::HashSet;

/// `page_type` of every B+tree page.
const TABLE_PAGE_TYPE: u32 = 5;
const KEY_DWORDS: u32 = 2;
const VALUE_DWORDS: u32 = 2;
const CHILD_DWORDS: u32 = 2;
const HEADER_DWORDS: usize = 7;
const DWORD: usize = 4;

/// Both words of a sentinel key, the page-local constant the refno index uses
/// too; `core.dll` matches it as the signed `-2147483647`.
pub const SENTINEL_WORD: u32 = 0x8000_0001;

/// One claim: the primary element, the user/bucket that holds it (0 with no
/// extract), and the session that last changed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimEntry {
    pub refno: RefNo,
    pub owner: u32,
    pub session: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ClaimTableError {
    #[error("{page} could not be read: {detail}")]
    Unreadable { page: PageId, detail: String },
    #[error("{page} is {len} bytes, too small for the {needed}-byte table page header")]
    PageTooSmall { page: PageId, len: usize, needed: usize },
    #[error("{page} length {len} is not a whole number of 4-byte words")]
    PageNotDwordAligned { page: PageId, len: usize },
    #[error("{page} is page type {found}, expected {expected}")]
    UnexpectedPageType { page: PageId, found: u32, expected: u32 },
    #[error("{page} belongs to table {found}, expected {expected} for the claim tree")]
    WrongTable { page: PageId, found: u32, expected: u32 },
    #[error("{page} states a {key_dwords}-dword key and a {value_dwords}-dword value; the claim tree states {KEY_DWORDS} and {VALUE_DWORDS}")]
    NotAClaimTable { page: PageId, key_dwords: u32, value_dwords: u32 },
    #[error("free_dwords is {free_dwords} on {page} but only {usable_dwords} dwords follow the header")]
    FreeSpaceOutOfRange { page: PageId, free_dwords: u32, usable_dwords: u32 },
    #[error("{used_dwords} dwords are in use on {page}, not a whole number of {stride_dwords}-dword entries")]
    FreeSpaceNotEntryAligned { page: PageId, used_dwords: u64, stride_dwords: u64 },
    #[error("{parent} is at level {parent_level}, so its child {child} must be at level {expected}, but it is at {found}")]
    LevelNotDecreasing { parent: PageId, parent_level: u32, child: PageId, expected: u32, found: u32 },
    #[error("the tree revisits {revisited} after {after} pages")]
    Cycle { revisited: PageId, after: u32 },
    #[error("{page} is an internal page at level {level} with no entries")]
    NoChildren { page: PageId, level: u32 },
    #[error("the walk has visited {visited} pages, more than the {limit} the file holds")]
    TooManyPages { visited: u32, limit: u32 },
    #[error("{page} emits {found:?} after {previous:?}, so slot order is not key order")]
    KeyOutOfOrder { page: PageId, previous: (u32, u32), found: (u32, u32) },
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    /// `None` on the sentinel.
    key: Option<ClaimEntry>,
    child: Option<u32>,
}

struct Node {
    level: u32,
    slots: Vec<Slot>,
}

impl Node {
    fn is_leaf(&self) -> bool {
        self.level == 0
    }

    fn children(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.iter().filter_map(|slot| slot.child)
    }

    fn keys(&self) -> impl Iterator<Item = ClaimEntry> + '_ {
        self.slots.iter().filter_map(|slot| slot.key)
    }
}

fn order(refno: RefNo) -> (u32, u32) {
    (refno.word0, refno.word1)
}

fn decode(page: PageId, data: &[u8]) -> Result<Node, ClaimTableError> {
    if !data.len().is_multiple_of(DWORD) {
        return Err(ClaimTableError::PageNotDwordAligned { page, len: data.len() });
    }
    if data.len() < HEADER_DWORDS * DWORD {
        return Err(ClaimTableError::PageTooSmall { page, len: data.len(), needed: HEADER_DWORDS * DWORD });
    }
    let word = |index: usize| {
        u32::from_be_bytes(data[index * DWORD..index * DWORD + DWORD].try_into().expect("four bytes"))
    };
    let page_type = word(0);
    if page_type != TABLE_PAGE_TYPE {
        return Err(ClaimTableError::UnexpectedPageType { page, found: page_type, expected: TABLE_PAGE_TYPE });
    }
    let found = word(1);
    if found != DATA_PAGE_MAIN {
        return Err(ClaimTableError::WrongTable { page, found, expected: DATA_PAGE_MAIN });
    }
    let level = word(2);
    let key_dwords = word(3);
    let value_dwords = word(4);
    if key_dwords != KEY_DWORDS || (level == 0 && value_dwords != VALUE_DWORDS) {
        return Err(ClaimTableError::NotAClaimTable { page, key_dwords, value_dwords });
    }
    let free_dwords = word(6);
    let usable_dwords = data.len() / DWORD - HEADER_DWORDS;
    if free_dwords as usize > usable_dwords {
        return Err(ClaimTableError::FreeSpaceOutOfRange { page, free_dwords, usable_dwords: usable_dwords as u32 });
    }
    let stride_dwords = u64::from(key_dwords) + if level == 0 { u64::from(VALUE_DWORDS) } else { u64::from(CHILD_DWORDS) };
    let used_dwords = usable_dwords as u64 - u64::from(free_dwords);
    if !used_dwords.is_multiple_of(stride_dwords) {
        return Err(ClaimTableError::FreeSpaceNotEntryAligned { page, used_dwords, stride_dwords });
    }
    let count = (used_dwords / stride_dwords) as usize;
    let mut slots = Vec::with_capacity(count);
    let mut previous: Option<(u32, u32)> = None;
    for slot in 0..count {
        let at = HEADER_DWORDS + slot * stride_dwords as usize;
        let key_words = (word(at), word(at + 1));
        let sentinel = key_words.0 == SENTINEL_WORD && key_words.1 == SENTINEL_WORD;
        let key = (!sentinel).then(|| ClaimEntry {
            refno: RefNo::new(key_words.0, key_words.1),
            owner: if level == 0 { word(at + 2) } else { 0 },
            session: if level == 0 { word(at + 3) } else { 0 },
        });
        if let (Some(previous), Some(key)) = (previous, key) {
            if order(key.refno) < previous {
                return Err(ClaimTableError::KeyOutOfOrder { page, previous, found: order(key.refno) });
            }
        }
        if let Some(key) = key {
            previous = Some(order(key.refno));
        }
        let child = (level != 0).then(|| word(at + KEY_DWORDS as usize));
        slots.push(Slot { key, child });
    }
    Ok(Node { level, slots })
}

/// The claim tree of one session-pinned view.
pub struct ClaimTable<'a> {
    pages: &'a mut PageCache,
    root: PageId,
    page_limit: u32,
}

impl<'a> ClaimTable<'a> {
    /// Open the claim tree the given view's session roots at.
    pub fn open(view: &'a mut DbView) -> Self {
        let root = view.session().claim;
        Self::at_root(view.pages_mut(), root)
    }

    /// Open a root the caller names — for reading another session's tree.
    pub fn at_root(pages: &'a mut PageCache, root: PageId) -> Self {
        let page_limit = pages.physical_page_count().max(1);
        Self { pages, root, page_limit }
    }

    pub fn root(&self) -> PageId {
        self.root
    }

    pub fn stats(&self) -> IoStats {
        self.pages.stats()
    }

    /// Every claim, in refno order.
    pub fn iter(&mut self) -> ClaimWalk<'_> {
        ClaimWalk::new(self.pages, self.root, self.page_limit)
    }

    /// The claim on one element, or `None` when the tree holds none — the
    /// element is not a primary, or is a primary the session never touched.
    pub fn get(&mut self, refno: RefNo) -> Result<Option<ClaimEntry>, ClaimTableError> {
        for entry in self.iter() {
            let entry = entry?;
            match order(entry.refno).cmp(&order(refno)) {
                std::cmp::Ordering::Less => continue,
                std::cmp::Ordering::Equal => return Ok(Some(entry)),
                std::cmp::Ordering::Greater => return Ok(None),
            }
        }
        Ok(None)
    }
}

struct Frame {
    page: PageId,
    level: u32,
    children: Vec<u32>,
    next: usize,
}

/// The claims in refno order.
pub struct ClaimWalk<'a> {
    pages: &'a mut PageCache,
    stack: Vec<Frame>,
    leaf: Vec<ClaimEntry>,
    leaf_slot: usize,
    visited: HashSet<PageId>,
    page_limit: u32,
    start: Option<PageId>,
    failed: bool,
}

impl<'a> ClaimWalk<'a> {
    fn new(pages: &'a mut PageCache, root: PageId, page_limit: u32) -> Self {
        Self {
            pages,
            stack: Vec::new(),
            leaf: Vec::new(),
            leaf_slot: 0,
            visited: HashSet::new(),
            page_limit,
            start: Some(root),
            failed: false,
        }
    }

    fn read(&mut self, page: PageId, parent: Option<(PageId, u32)>) -> Result<Node, ClaimTableError> {
        if !self.visited.insert(page) {
            return Err(ClaimTableError::Cycle { revisited: page, after: self.visited.len() as u32 });
        }
        if self.visited.len() as u32 > self.page_limit {
            return Err(ClaimTableError::TooManyPages { visited: self.visited.len() as u32, limit: self.page_limit });
        }
        let data = self
            .pages
            .get(page)
            .map_err(|source| ClaimTableError::Unreadable { page, detail: source.to_string() })?;
        let node = decode(page, data)?;
        if let Some((parent_page, parent_level)) = parent {
            let expected = parent_level.saturating_sub(1);
            if node.level != expected {
                return Err(ClaimTableError::LevelNotDecreasing {
                    parent: parent_page,
                    parent_level,
                    child: page,
                    expected,
                    found: node.level,
                });
            }
        }
        Ok(node)
    }

    fn advance(&mut self) -> Result<bool, ClaimTableError> {
        if let Some(root) = self.start.take() {
            return self.open(root, None);
        }
        while let Some(frame) = self.stack.last_mut() {
            let Some(&child) = frame.children.get(frame.next) else {
                self.stack.pop();
                continue;
            };
            frame.next += 1;
            let parent = Some((frame.page, frame.level));
            let child = PageId::new(frame.page.ext, child);
            if self.open(child, parent)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn open(&mut self, page: PageId, parent: Option<(PageId, u32)>) -> Result<bool, ClaimTableError> {
        let node = self.read(page, parent)?;
        if node.is_leaf() {
            self.leaf.clear();
            self.leaf.extend(node.keys());
            self.leaf_slot = 0;
            return Ok(true);
        }
        let level = node.level;
        let children: Vec<u32> = node.children().collect();
        if children.is_empty() {
            return Err(ClaimTableError::NoChildren { page, level });
        }
        let first = PageId::new(page.ext, children[0]);
        self.stack.push(Frame { page, level, children, next: 1 });
        // Descend the left edge until the first leaf.
        self.open(first, Some((page, level)))
    }
}

impl Iterator for ClaimWalk<'_> {
    type Item = Result<ClaimEntry, ClaimTableError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(entry) = self.leaf.get(self.leaf_slot) {
                self.leaf_slot += 1;
                return Some(Ok(*entry));
            }
            match self.advance() {
                Ok(true) => continue,
                Ok(false) => return None,
                Err(error) => {
                    self.failed = true;
                    return Some(Err(error));
                }
            }
        }
    }
}
