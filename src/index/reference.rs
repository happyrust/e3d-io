//! The reverse-reference tables: "who points at this element, through which
//! attribute".
//!
//! A dabacon file does not only hold the refno index. Its session page also
//! carries a directory of named B+trees ([`crate::session::TableRoot`]), and
//! most of those are named after a **reference attribute** — `SPRE`, `CATR`,
//! `PRTREF`, `GOBREF`, `HDBREF`, … One table per attribute, and each entry is
//! a bare 4-dword key:
//!
//! ```text
//! [ target.word0, target.word1, referrer.word0, referrer.word1 ]
//! ```
//!
//! No value follows it — the leaf pages declare `value_dwords == 0`, because
//! the key already says everything. Keys sort ascending, so every referrer of
//! one target is a contiguous run and the answer to "who references this
//! catalogue component" is a range scan.
//!
//! This is the table core.dll searches. `DB_RefTableIterator::startSearch`
//! (`0x5a1d090`) takes `(dbno, attId, targetRef, handle)`: `attId` picks the
//! table, `targetRef` is the key, and `increment` (`0x5a1b710`) walks the run.
//! `DB_MDB::findElements(dbType, refAttr, el)` is the same search repeated over
//! every database of a type, and `BREF` / `SPBREF` and the rest of that
//! pseudo-attribute family read out of it. ADR-003 recorded these back-refs as
//! "offline unavailable" and had gen-model build its own reverse index instead;
//! that judgement predates the directory being identified and the tables being
//! read.
//!
//! **A table lives in the referring database, keyed by the target.** Across the
//! corpus below, 0 of 3 699 963 referrers were outside the file holding the
//! table, while 654 707 of the targets were — a design element in `ams8000`
//! records its `SPRE` into `ams8000`'s own `SPRE` table under the catalogue
//! component's refno. So a *complete* answer to "who references this component"
//! is the union over the databases in scope, exactly as `findElements` iterates
//! over the MDB. One file answers only for itself, and says so by construction
//! rather than by returning a short list.
//!
//! Corpus gate (543 files: `AvevaMarineSample`, `AvevaCatalogue`, `ZDJ`;
//! `docs/evidence/2026-09-03-dabacon-named-tables-and-reference-tables.md`):
//! 906 reference tables, 3 699 963 entries, 0 pages out of key order, 0
//! referrers absent from their own file's refno index.
//!
//! Not every directory table is a reference table. `TYPEX` shares the 4-dword
//! key shape and is *not* one — 2 610 190 of its keys' second halves are not
//! refnos of the file at all. Membership is decided by the dictionary, not by
//! the shape: a table is a reference table when its name is an `ELEMENT`-typed
//! attribute. This module refuses to guess, so [`ReferenceTable::open`] takes
//! the name hash the caller has already decided on.

use std::collections::HashSet;

use crate::page::cache::PageCache;
use crate::page::{IoStats, PageId};
use crate::refno::RefNo;
use crate::session::DbView;

/// `page_type` of every B+tree page, reference tables included.
pub const TABLE_PAGE_TYPE: u32 = 5;

/// A reference table's key is the target refno followed by the referrer refno.
pub const REFERENCE_KEY_DWORDS: u32 = 4;

/// A reference table's leaf carries no value; the key is the whole entry.
pub const REFERENCE_VALUE_DWORDS: u32 = 0;

/// An internal entry is followed by its child's `(pgno, extno)`.
const CHILD_DWORDS: u32 = 2;

const DWORD: usize = 4;
const HEADER_DWORDS: usize = 7;

/// Both words of a sentinel key, the same page-local constant the refno index
/// uses. core.dll matches it as the signed `-2147483647`.
const SENTINEL_WORD: u32 = 0x8000_0001;

/// One edge: `referrer` points at `target` through the attribute this table is
/// named after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReferenceEntry {
    pub target: RefNo,
    pub referrer: RefNo,
}

impl ReferenceEntry {
    /// Sort key. Words compare unsigned, which is what puts the corpus's
    /// 3 699 963 entries in order; reading `word1` as signed does not.
    fn order(&self) -> (u32, u32, u32, u32) {
        (
            self.target.word0,
            self.target.word1,
            self.referrer.word0,
            self.referrer.word1,
        )
    }

    /// The lowest key that can belong to `target`, for a range scan's start.
    fn lower_bound(target: RefNo) -> Self {
        Self {
            target,
            referrer: RefNo::new(0, 0),
        }
    }
}

impl std::fmt::Display for ReferenceEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} <- {}", self.target, self.referrer)
    }
}

/// Why a reference table could not be read.
///
/// Every rejection names the page. A malformed table never decodes to an empty
/// run, because "nothing references this element" and "I could not tell" are
/// the two answers a reverse-cascade must never confuse.
#[derive(Debug, thiserror::Error)]
pub enum ReferenceTableError {
    #[error("this session declares no table named {name_hash}")]
    NoSuchTable { name_hash: u32 },

    #[error("{page} could not be read: {detail}")]
    Unreadable { page: PageId, detail: String },

    #[error("{page} is {len} bytes, too small for the {needed}-byte table page header")]
    PageTooSmall {
        page: PageId,
        len: usize,
        needed: usize,
    },

    #[error("{page} length {len} is not a whole number of 4-byte words")]
    PageNotDwordAligned { page: PageId, len: usize },

    #[error("{page} is page type {found}, expected {expected} for a table page")]
    UnexpectedPageType {
        page: PageId,
        found: u32,
        expected: u32,
    },

    #[error("{page} belongs to table {found}, but it was reached as a page of table {expected}")]
    WrongTable {
        page: PageId,
        found: u32,
        expected: u32,
    },

    #[error(
        "table {name_hash} states a {key_dwords}-dword key and a {value_dwords}-dword leaf value \
         on {page}; a reference table states {REFERENCE_KEY_DWORDS} and {REFERENCE_VALUE_DWORDS}"
    )]
    NotAReferenceTable {
        page: PageId,
        name_hash: u32,
        key_dwords: u32,
        value_dwords: u32,
    },

    #[error("free_dwords is {free_dwords} on {page} but only {usable_dwords} dwords follow the header")]
    FreeSpaceOutOfRange {
        page: PageId,
        free_dwords: u32,
        usable_dwords: u32,
    },

    #[error("{used_dwords} dwords are in use on {page}, which is not a whole number of {stride_dwords}-dword entries")]
    FreeSpaceNotEntryAligned {
        page: PageId,
        used_dwords: u64,
        stride_dwords: u64,
    },

    #[error("{parent} is at level {parent_level}, so its child {child} must be at level {expected}, but it is at {found}")]
    LevelNotDecreasing {
        parent: PageId,
        parent_level: u32,
        child: PageId,
        expected: u32,
        found: u32,
    },

    #[error("the tree revisits {revisited} after {after} pages")]
    Cycle { revisited: PageId, after: u32 },

    #[error("{page} is an internal page at level {level} with no entries, so the descent has nowhere to go")]
    NoChildren { page: PageId, level: u32 },

    #[error("the walk has visited {visited} pages, more than the {limit} the file holds")]
    TooManyPages { visited: u32, limit: u32 },

    #[error("{page} emits {found} after {previous}, so slot order is not key order")]
    KeyOutOfOrder {
        page: PageId,
        previous: ReferenceEntry,
        found: ReferenceEntry,
    },
}

/// One slot of a decoded page: a key, and on an internal page the child that
/// holds every key from this one up to the next slot's.
#[derive(Debug, Clone, Copy)]
struct Slot {
    /// `None` on the sentinel, whose key words are a page-local constant
    /// rather than refnos.
    key: Option<ReferenceEntry>,
    /// Child page number. `None` on a leaf.
    child: Option<u32>,
}

/// A decoded reference-table page.
///
/// Owned rather than borrowed: a page holds at most 84 slots at the 2048-byte
/// page size the corpus uses, and copying them out is what lets the walk hold
/// a frame per level while the page cache stays free to evict.
#[derive(Debug)]
struct Node {
    level: u32,
    slots: Vec<Slot>,
}

impl Node {
    fn is_leaf(&self) -> bool {
        self.level == 0
    }

    /// The slot to descend through for `target`, as a slot index.
    ///
    /// Same rule as the refno index ([`crate::index::choose_child`]), applied
    /// to the 4-dword key: an internal key is the **least** key of the subtree
    /// behind it, so the only child that can hold `target` is the one behind
    /// the greatest key `<= target`. Below the first key that is the sentinel's
    /// child; a page without a sentinel falls back to its first slot.
    fn route(&self, target: ReferenceEntry) -> Option<usize> {
        let mut best: Option<(usize, ReferenceEntry)> = None;
        let mut leftmost: Option<usize> = None;
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.child.is_none() {
                continue;
            }
            let Some(key) = slot.key else {
                if leftmost.is_none() {
                    leftmost = Some(index);
                }
                continue;
            };
            if leftmost.is_none() {
                leftmost = Some(index);
            }
            if key.order() > target.order() {
                continue;
            }
            if best.is_none_or(|(_, best_key)| key.order() > best_key.order()) {
                best = Some((index, key));
            }
        }
        best.map(|(index, _)| index).or(leftmost)
    }

    fn children(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.iter().filter_map(|slot| slot.child)
    }

    fn keys(&self) -> impl Iterator<Item = ReferenceEntry> + '_ {
        self.slots.iter().filter_map(|slot| slot.key)
    }
}

/// Decode one page of the named table.
///
/// **The only page-to-node step for reference tables**, so the range scan and
/// the full walk cannot disagree about what a node is.
fn decode(page: PageId, data: &[u8], name_hash: u32) -> Result<Node, ReferenceTableError> {
    if data.len() % DWORD != 0 {
        return Err(ReferenceTableError::PageNotDwordAligned {
            page,
            len: data.len(),
        });
    }
    if data.len() < HEADER_DWORDS * DWORD {
        return Err(ReferenceTableError::PageTooSmall {
            page,
            len: data.len(),
            needed: HEADER_DWORDS * DWORD,
        });
    }
    let word = |index: usize| {
        u32::from_be_bytes(
            data[index * DWORD..index * DWORD + DWORD]
                .try_into()
                .expect("slice is exactly four bytes"),
        )
    };

    let page_type = word(0);
    if page_type != TABLE_PAGE_TYPE {
        return Err(ReferenceTableError::UnexpectedPageType {
            page,
            found: page_type,
            expected: TABLE_PAGE_TYPE,
        });
    }
    let found = word(1);
    if found != name_hash {
        return Err(ReferenceTableError::WrongTable {
            page,
            found,
            expected: name_hash,
        });
    }

    let level = word(2);
    let key_dwords = word(3);
    let value_dwords = word(4);
    // The declared value width describes the leaf. An internal entry always
    // carries `(pgno, extno)` after its key, whatever the leaf holds — the same
    // asymmetry the refno index has, where the leaf value is a record address
    // and the internal value is a child address.
    if key_dwords != REFERENCE_KEY_DWORDS
        || (level == 0 && value_dwords != REFERENCE_VALUE_DWORDS)
    {
        return Err(ReferenceTableError::NotAReferenceTable {
            page,
            name_hash,
            key_dwords,
            value_dwords,
        });
    }
    let free_dwords = word(6);

    let usable_dwords = data.len() / DWORD - HEADER_DWORDS;
    if free_dwords as usize > usable_dwords {
        return Err(ReferenceTableError::FreeSpaceOutOfRange {
            page,
            free_dwords,
            usable_dwords: usable_dwords as u32,
        });
    }
    let stride_dwords = u64::from(key_dwords)
        + if level == 0 {
            u64::from(value_dwords)
        } else {
            u64::from(CHILD_DWORDS)
        };
    let used_dwords = usable_dwords as u64 - u64::from(free_dwords);
    if used_dwords % stride_dwords != 0 {
        return Err(ReferenceTableError::FreeSpaceNotEntryAligned {
            page,
            used_dwords,
            stride_dwords,
        });
    }

    let count = (used_dwords / stride_dwords) as usize;
    let mut slots = Vec::with_capacity(count);
    let mut previous: Option<ReferenceEntry> = None;
    for slot in 0..count {
        let at = HEADER_DWORDS + slot * stride_dwords as usize;
        let words = [word(at), word(at + 1), word(at + 2), word(at + 3)];
        let sentinel = words.iter().all(|&w| w == SENTINEL_WORD);
        let key = (!sentinel).then(|| ReferenceEntry {
            target: RefNo::new(words[0], words[1]),
            referrer: RefNo::new(words[2], words[3]),
        });
        if let (Some(previous), Some(key)) = (previous, key) {
            if key.order() < previous.order() {
                return Err(ReferenceTableError::KeyOutOfOrder {
                    page,
                    previous,
                    found: key,
                });
            }
        }
        if key.is_some() {
            previous = key;
        }
        let child = (level != 0).then(|| word(at + key_dwords as usize));
        slots.push(Slot { key, child });
    }
    Ok(Node { level, slots })
}

/// One reference table of one session-pinned view.
///
/// Borrows the view's page cache, so [`ReferenceTable::stats`] measures exactly
/// what the scan did.
pub struct ReferenceTable<'a> {
    pages: &'a mut PageCache,
    root: PageId,
    name_hash: u32,
    page_limit: u32,
}

impl<'a> ReferenceTable<'a> {
    /// Open the table this session names, or say it has none.
    ///
    /// A database with no `SPRE` table has nothing referencing through `SPRE`,
    /// but that is the *caller's* conclusion to draw: an absent table and an
    /// empty one are the same answer here only because the writer never creates
    /// a table it would leave empty, and this crate does not assume it.
    pub fn open(view: &'a mut DbView, name_hash: u32) -> Result<Self, ReferenceTableError> {
        let root = view
            .table_root(name_hash)
            .ok_or(ReferenceTableError::NoSuchTable { name_hash })?;
        Ok(Self::at_root(view.pages_mut(), root, name_hash))
    }

    /// Open a root the caller names — for reading another session's table.
    pub fn at_root(pages: &'a mut PageCache, root: PageId, name_hash: u32) -> Self {
        let page_limit = pages.physical_page_count().max(1);
        Self {
            pages,
            root,
            name_hash,
            page_limit,
        }
    }

    pub fn root(&self) -> PageId {
        self.root
    }

    pub fn name_hash(&self) -> u32 {
        self.name_hash
    }

    pub fn stats(&self) -> IoStats {
        self.pages.stats()
    }

    /// Every edge in the table, in key order.
    pub fn iter(&mut self) -> ReferenceWalk<'_> {
        ReferenceWalk::new(self.pages, self.root, self.name_hash, self.page_limit, None)
    }

    /// Every edge from `target` on, in key order.
    ///
    /// Costs the height of the tree to reach the first one, then one page per
    /// leaf crossed — not the whole table.
    pub fn scan_from(&mut self, target: RefNo) -> ReferenceWalk<'_> {
        ReferenceWalk::new(
            self.pages,
            self.root,
            self.name_hash,
            self.page_limit,
            Some(ReferenceEntry::lower_bound(target)),
        )
    }

    /// Everything in **this database** that references `target` through this
    /// table's attribute.
    ///
    /// Empty means this database holds no such referrer, never that the table
    /// could not be read: a failure arrives as `Err`. Answering across an MDB
    /// is the caller's union over its databases — see the module note.
    pub fn referrers_of(&mut self, target: RefNo) -> Result<Vec<RefNo>, ReferenceTableError> {
        let mut out = Vec::new();
        for entry in self.scan_from(target) {
            let entry = entry?;
            if entry.target != target {
                break;
            }
            out.push(entry.referrer);
        }
        Ok(out)
    }
}

/// One internal page part-way through being walked.
struct Frame {
    page: PageId,
    level: u32,
    children: Vec<u32>,
    next: usize,
}

/// Edges of one reference table, in key order, from a bound or from the start.
///
/// Descends once and then advances, holding one frame per internal level and
/// one leaf at a time — so a scan costs the run it returns plus the height of
/// the tree, not the table.
///
/// A page that cannot be read ends the walk with that error rather than with a
/// short list of perfectly good entries.
pub struct ReferenceWalk<'a> {
    pages: &'a mut PageCache,
    name_hash: u32,
    stack: Vec<Frame>,
    leaf: Vec<ReferenceEntry>,
    leaf_slot: usize,
    visited: HashSet<PageId>,
    page_limit: u32,
    start: Option<PageId>,
    /// Set until the descent reaches its first leaf; routes the descent and
    /// then trims that leaf's head.
    seeking: Option<ReferenceEntry>,
    failed: bool,
}

impl<'a> ReferenceWalk<'a> {
    fn new(
        pages: &'a mut PageCache,
        root: PageId,
        name_hash: u32,
        page_limit: u32,
        from: Option<ReferenceEntry>,
    ) -> Self {
        Self {
            pages,
            name_hash,
            stack: Vec::new(),
            leaf: Vec::new(),
            leaf_slot: 0,
            visited: HashSet::new(),
            page_limit,
            start: Some(root),
            seeking: from,
            failed: false,
        }
    }

    fn read(&mut self, page: PageId, parent: Option<(PageId, u32)>) -> Result<Node, ReferenceTableError> {
        if !self.visited.insert(page) {
            return Err(ReferenceTableError::Cycle {
                revisited: page,
                after: self.visited.len() as u32,
            });
        }
        if self.visited.len() as u32 > self.page_limit {
            return Err(ReferenceTableError::TooManyPages {
                visited: self.visited.len() as u32,
                limit: self.page_limit,
            });
        }
        let data = self
            .pages
            .get(page)
            .map_err(|source| ReferenceTableError::Unreadable {
                page,
                detail: source.to_string(),
            })?;
        let node = decode(page, data, self.name_hash)?;
        if let Some((parent_page, parent_level)) = parent {
            let expected = parent_level.saturating_sub(1);
            if node.level != expected {
                return Err(ReferenceTableError::LevelNotDecreasing {
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

    /// Load the next leaf, or report that there is none left.
    fn advance(&mut self) -> Result<bool, ReferenceTableError> {
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

    /// Read one page: fill the leaf buffer, or push a frame and keep going.
    fn open(
        &mut self,
        page: PageId,
        parent: Option<(PageId, u32)>,
    ) -> Result<bool, ReferenceTableError> {
        let mut next = Some((page, parent));
        while let Some((page, parent)) = next.take() {
            let node = self.read(page, parent)?;
            if node.is_leaf() {
                self.leaf.clear();
                self.leaf.extend(node.keys());
                self.leaf_slot = match self.seeking.take() {
                    Some(bound) => self
                        .leaf
                        .partition_point(|entry| entry.order() < bound.order()),
                    None => 0,
                };
                return Ok(true);
            }

            let level = node.level;
            let children: Vec<u32> = node.children().collect();
            if children.is_empty() {
                return Err(ReferenceTableError::NoChildren { page, level });
            }
            // A bounded scan descends through the routed slot and resumes at
            // the one after it; an unbounded one takes the left edge. Both
            // leave the frame in the same state, so everything past the first
            // leaf is the same walk.
            let slot = match self.seeking {
                Some(bound) => node
                    .route(bound)
                    // `route` answers in slots and `children` skips slots
                    // without one, so the two index the same list only after
                    // this count. Every internal slot in the corpus has a
                    // child, which is exactly why the conversion has to be
                    // written down rather than assumed.
                    .map(|index| {
                        node.slots[..index]
                            .iter()
                            .filter(|slot| slot.child.is_some())
                            .count()
                    })
                    .unwrap_or(0),
                None => 0,
            };
            let first = PageId::new(page.ext, children[slot]);
            self.stack.push(Frame {
                page,
                level,
                children,
                next: slot + 1,
            });
            next = Some((first, Some((page, level))));
        }
        Ok(false)
    }
}

impl Iterator for ReferenceWalk<'_> {
    type Item = Result<ReferenceEntry, ReferenceTableError>;

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

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_BYTES: usize = 2048;

    fn page(level: u32, name_hash: u32, entries: &[[u32; 4]], children: &[u32]) -> Vec<u8> {
        let stride = if level == 0 { 4 } else { 6 };
        let usable = PAGE_BYTES / DWORD - HEADER_DWORDS;
        let free = usable - entries.len() * stride;
        let mut words = vec![
            TABLE_PAGE_TYPE,
            name_hash,
            level,
            REFERENCE_KEY_DWORDS,
            REFERENCE_VALUE_DWORDS,
            0,
            free as u32,
        ];
        for (index, entry) in entries.iter().enumerate() {
            words.extend_from_slice(entry);
            if level != 0 {
                words.push(children[index]);
                words.push(u32::from(PageId::PRIMARY));
            }
        }
        words.resize(PAGE_BYTES / DWORD, 0);
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    const SPRE: u32 = 643_429;

    #[test]
    fn a_leaf_decodes_target_then_referrer() {
        let bytes = page(
            0,
            SPRE,
            &[[13_244, 108_564, 24_384, 2952], [13_244, 108_564, 24_384, 2962]],
            &[],
        );
        let node = decode(PageId::new(1, 683), &bytes, SPRE).unwrap();
        let keys: Vec<_> = node.keys().collect();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].target, RefNo::new(13_244, 108_564));
        assert_eq!(keys[0].referrer, RefNo::new(24_384, 2952));
        assert_eq!(keys[1].referrer, RefNo::new(24_384, 2962));
    }

    /// The sentinel is not an edge. Emitting it would put `RefNo(0x80000001)`
    /// into a cascade as if something referenced it.
    #[test]
    fn the_sentinel_is_not_an_edge() {
        let sentinel = [SENTINEL_WORD; 4];
        let bytes = page(0, SPRE, &[sentinel, [1, 2, 3, 4]], &[]);
        let node = decode(PageId::new(1, 1), &bytes, SPRE).unwrap();
        assert_eq!(node.slots.len(), 2);
        assert_eq!(node.keys().count(), 1);
    }

    /// A page of another table reached through this one's pointers is a broken
    /// tree, not an empty answer.
    #[test]
    fn a_page_of_another_table_is_refused() {
        let bytes = page(0, 900_345, &[[1, 2, 3, 4]], &[]);
        let error = decode(PageId::new(1, 5), &bytes, SPRE).unwrap_err();
        assert!(
            matches!(
                error,
                ReferenceTableError::WrongTable {
                    found: 900_345,
                    expected: SPRE,
                    ..
                }
            ),
            "{error}"
        );
    }

    /// `TYPEX` shares the key width; the refusal that separates the shapes is
    /// the leaf value width, so a table declaring one is not read as edges.
    #[test]
    fn a_leaf_with_a_value_is_not_a_reference_table() {
        let mut bytes = page(0, SPRE, &[[1, 2, 3, 4]], &[]);
        bytes[0x10..0x14].copy_from_slice(&7u32.to_be_bytes());
        let error = decode(PageId::new(1, 5), &bytes, SPRE).unwrap_err();
        assert!(
            matches!(
                error,
                ReferenceTableError::NotAReferenceTable {
                    value_dwords: 7,
                    ..
                }
            ),
            "{error}"
        );
    }

    /// Slot order is key order, and a page that breaks it is reported rather
    /// than merged over — a range scan stops at the first key past the target,
    /// so an unsorted page silently shortens the answer.
    #[test]
    fn a_page_whose_slots_are_out_of_order_is_refused() {
        let bytes = page(0, SPRE, &[[1, 9, 0, 0], [1, 2, 0, 0]], &[]);
        let error = decode(PageId::new(1, 5), &bytes, SPRE).unwrap_err();
        assert!(
            matches!(error, ReferenceTableError::KeyOutOfOrder { .. }),
            "{error}"
        );
    }

    /// An internal key is the least key of the subtree behind it, so routing
    /// takes the last key `<= target`. Taking the first key `>=` instead misses
    /// every target that opens a subtree.
    #[test]
    fn routing_takes_the_last_key_at_or_below_the_target() {
        let sentinel = [SENTINEL_WORD; 4];
        let bytes = page(
            1,
            SPRE,
            &[sentinel, [10, 0, 0, 0], [20, 0, 0, 0]],
            &[100, 200, 300],
        );
        let node = decode(PageId::new(1, 9), &bytes, SPRE).unwrap();
        let route = |word0| {
            node.route(ReferenceEntry::lower_bound(RefNo::new(word0, 0)))
                .map(|slot| node.slots[slot].child.unwrap())
        };
        assert_eq!(route(5), Some(100), "below the first key: the sentinel's child");
        assert_eq!(route(10), Some(200));
        assert_eq!(route(15), Some(200));
        assert_eq!(route(20), Some(300));
        assert_eq!(route(99), Some(300));
    }
}
