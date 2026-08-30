//! L2: walking the tree. One descent, shared by lookup and by enumeration.
//!
//! The root comes from the session, never from a scan. [`crate::session::DbView`]
//! reads the index root off the session page it was pinned to, so a cursor
//! answers as of that session and nothing else. The reader this replaces found
//! its roots by reading every page in the file and keeping the index pages no
//! other index page named — a lookup that cost the whole file before it had
//! looked at anything, and that answered from whichever tree happened to sort
//! first.
//!
//! Two properties are held here rather than hoped for:
//!
//! * **Lookup and enumeration agree on what exists.** They share
//!   [`read_node`] and [`crate::index::choose_child`], so a key the walk emits
//!   is a key the descent can reach. `tests/index_cursor_real.rs` asserts the
//!   two sets are equal on real databases; that is the gate this module exists
//!   to make passable.
//! * **A lookup costs the height of the tree, and nothing hides in the
//!   margin.** Every page the descent needs is a demand read, prefetch is
//!   charged separately by the page layer, and no path here falls back to
//!   reading the file end to end.
//!
//! A page that cannot be read, cannot be decoded, or sits at the wrong level is
//! an error naming it. It is never the empty answer: "no such element" and "I
//! could not tell" are different facts, and the reader this replaces returned
//! `None` for both.
//!
//! [`read_node`] and [`located`] are `pub(super)` so that
//! [`crate::index::diff`] merges the same nodes these two paths walk. Widening
//! them further would let a caller outside the index module build its own
//! descent, which is the thing this module exists to prevent.

use std::collections::HashSet;

use crate::index::{choose_child, IndexEntry, IndexNode, IndexPageError};
use crate::page::cache::PageCache;
use crate::page::{IoStats, PageId};
use crate::refno::RefNo;
use crate::session::DbView;

/// Where a record lives, as the index states it.
///
/// `offset_words` counts **2-byte** words from the top of `page`, which is the
/// unit the leaf value uses; [`RecordLoc::byte_offset`] is the only conversion.
///
/// The leaf value is two dwords, `page_no` and `packed`, and carries no extent
/// of its own, so `page.ext` is inherited from the index page that named it.
/// Every sample database is single-extent, so nothing in the corpus can tell
/// that apart from taking the extent out of the RefNo — which is why this is
/// stated rather than decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordLoc {
    pub page: PageId,
    pub offset_words: u32,
    /// Low 12 bits of `packed`. Reads 1 on all 940 694 leaf entries of the
    /// corpus. It takes no part in routing and none in deciding whether a key
    /// exists; observing it is all this crate does with it.
    pub flag: u16,
}

impl RecordLoc {
    pub const fn byte_offset(&self) -> u64 {
        self.offset_words as u64 * 2
    }
}

/// One key and the record it addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedRecord {
    pub refno: RefNo,
    pub loc: RecordLoc,
    /// Leaf the entry was read from.
    pub leaf: PageId,
}

impl IndexedRecord {
    /// The leaf entry this was read from, for callers still working in entries.
    pub fn entry(&self) -> IndexEntry {
        IndexEntry {
            refno: self.refno,
            page_no: self.loc.page.page,
            offset_words: self.loc.offset_words,
            flag: self.loc.flag,
        }
    }
}

/// What one lookup found, and what it cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lookup {
    /// The entry, or `None` when the leaf that would hold the key does not.
    /// A failure never arrives here; it arrives as [`IndexTreeError`].
    pub found: Option<IndexedRecord>,
    /// Pages visited from the root to the leaf, inclusive — the height of the
    /// tree at this key. On a cold cache this is exactly how many pages the
    /// lookup read.
    pub depth: u32,
}

/// Why a walk of the tree could not go on.
#[derive(Debug, thiserror::Error)]
pub enum IndexTreeError {
    #[error("{page} could not be read: {detail}")]
    Unreadable { page: PageId, detail: String },

    #[error("{page} is not a usable index page: {source}")]
    Malformed {
        page: PageId,
        #[source]
        source: IndexPageError,
    },

    /// core.dll checks the same thing and reports "Expected page of level %d,
    /// but got level %d". A descent drops exactly one level per step; anything
    /// else means the page reached is not the child that was named.
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

    /// Slot order is key order — `tests/index_cursor_real.rs` holds every
    /// database in the corpus to it. [`crate::index::diff`] merges two trees
    /// on that promise and reports it broken rather than merging on regardless
    /// and quietly calling a key deleted that is simply out of place.
    #[error("{page} emits {found} after {previous}, so slot order is not key order")]
    KeyOutOfOrder {
        page: PageId,
        previous: RefNo,
        found: RefNo,
    },
}

/// A cursor over the index of one session-pinned view.
///
/// Borrows the view's page cache, so the cache's counters measure exactly what
/// the cursor did.
pub struct IndexCursor<'a> {
    pages: &'a mut PageCache,
    root: PageId,
    page_limit: u32,
}

impl<'a> IndexCursor<'a> {
    /// A cursor on the index as of the session the view was opened at.
    pub fn new(view: &'a mut DbView) -> Self {
        let root = view.index_root();
        Self::at_root(view.pages_mut(), root)
    }

    /// A cursor on a root the caller names — for two-root differencing, where
    /// the second root comes from another session's page.
    pub fn at_root(pages: &'a mut PageCache, root: PageId) -> Self {
        let page_limit = pages.physical_page_count().max(1);
        Self {
            pages,
            root,
            page_limit,
        }
    }

    pub fn root(&self) -> PageId {
        self.root
    }

    pub fn stats(&self) -> IoStats {
        self.pages.stats()
    }

    /// Find one key.
    ///
    /// Reads one page per level and stops at the leaf. Nothing here reads the
    /// record itself: confirming the record says the same RefNo back is L3's
    /// job, and doing it inside the index would make "the key is indexed" and
    /// "the record agrees" the same answer.
    pub fn seek(&mut self, target: RefNo) -> Result<Lookup, IndexTreeError> {
        let mut visited = HashSet::new();
        let mut current = self.root;
        let mut parent: Option<(PageId, u32)> = None;

        loop {
            if !visited.insert(current) {
                return Err(IndexTreeError::Cycle {
                    revisited: current,
                    after: visited.len() as u32,
                });
            }
            if visited.len() as u32 > self.page_limit {
                return Err(IndexTreeError::TooManyPages {
                    visited: visited.len() as u32,
                    limit: self.page_limit,
                });
            }

            let node = read_node(self.pages, current, parent)?;
            let level = node.level;
            if node.is_leaf() {
                let found = node.find(target).map(|entry| located(entry, current));
                return Ok(Lookup {
                    found,
                    depth: visited.len() as u32,
                });
            }
            let child = choose_child(node.entries(), target).ok_or(IndexTreeError::NoChildren {
                page: current,
                level,
            })?;
            parent = Some((current, level));
            current = PageId::new(current.ext, child);
        }
    }

    /// Every key in the tree, in key order.
    pub fn walk(&mut self) -> IndexWalk<'_> {
        IndexWalk::new(self.pages, self.root, self.page_limit)
    }
}

/// Every indexed key of one tree, in key order.
///
/// Descends the left edge and then advances, holding one frame per internal
/// level and one leaf's entries at a time — so the walk costs the tree, not the
/// file. Children are visited in slot order, which is key order: an internal
/// page lists the sentinel first and then keys ascending, and the enumeration
/// gate would go red the moment a page did not.
///
/// A page that cannot be read ends the walk with that error. The iterator this
/// replaces cleared its queue and returned success, so a damaged tree came back
/// as a short list of perfectly good entries.
pub struct IndexWalk<'a> {
    pages: &'a mut PageCache,
    stack: Vec<Frame>,
    leaf: Vec<IndexEntry>,
    leaf_page: PageId,
    leaf_slot: usize,
    visited: HashSet<PageId>,
    page_limit: u32,
    start: Option<PageId>,
    failed: bool,
}

/// One internal page part-way through being walked.
struct Frame {
    page: PageId,
    level: u32,
    children: Vec<u32>,
    next: usize,
}

impl<'a> IndexWalk<'a> {
    fn new(pages: &'a mut PageCache, root: PageId, page_limit: u32) -> Self {
        Self {
            pages,
            stack: Vec::new(),
            leaf: Vec::new(),
            leaf_page: root,
            leaf_slot: 0,
            visited: HashSet::new(),
            page_limit,
            start: Some(root),
            failed: false,
        }
    }

    /// Load the next leaf, or report that there is none left.
    fn advance(&mut self) -> Result<bool, IndexTreeError> {
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
    ) -> Result<bool, IndexTreeError> {
        let mut next = Some((page, parent));
        while let Some((page, parent)) = next.take() {
            if !self.visited.insert(page) {
                return Err(IndexTreeError::Cycle {
                    revisited: page,
                    after: self.visited.len() as u32,
                });
            }
            if self.visited.len() as u32 > self.page_limit {
                return Err(IndexTreeError::TooManyPages {
                    visited: self.visited.len() as u32,
                    limit: self.page_limit,
                });
            }

            let node = read_node(self.pages, page, parent)?;
            if node.is_leaf() {
                self.leaf.clear();
                self.leaf.extend(node.entries());
                self.leaf_page = page;
                self.leaf_slot = 0;
                return Ok(true);
            }

            let level = node.level;
            let children: Vec<u32> = node.entries().map(|entry| entry.page_no).collect();
            if children.is_empty() {
                return Err(IndexTreeError::NoChildren { page, level });
            }
            let first = PageId::new(page.ext, children[0]);
            self.stack.push(Frame {
                page,
                level,
                children,
                next: 1,
            });
            next = Some((first, Some((page, level))));
        }
        Ok(false)
    }
}

impl Iterator for IndexWalk<'_> {
    type Item = Result<IndexedRecord, IndexTreeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            while let Some(entry) = self.leaf.get(self.leaf_slot) {
                self.leaf_slot += 1;
                if entry.is_start_marker() {
                    continue;
                }
                return Some(Ok(located(*entry, self.leaf_page)));
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

/// Read and validate one index page. **The only page-to-node step in the
/// crate**, so lookup, enumeration and two-root differencing cannot disagree
/// about what a node is or which page counts as its child.
pub(super) fn read_node<'p>(
    pages: &'p mut PageCache,
    page: PageId,
    parent: Option<(PageId, u32)>,
) -> Result<IndexNode<'p>, IndexTreeError> {
    let data = pages
        .get(page)
        .map_err(|source| IndexTreeError::Unreadable {
            page,
            detail: source.to_string(),
        })?;
    let node =
        IndexNode::decode(data).map_err(|source| IndexTreeError::Malformed { page, source })?;

    if let Some((parent_page, parent_level)) = parent {
        let expected = parent_level.saturating_sub(1);
        if node.level != expected {
            return Err(IndexTreeError::LevelNotDecreasing {
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

/// The one reading of a leaf value as a record address.
pub(super) fn located(entry: IndexEntry, leaf: PageId) -> IndexedRecord {
    IndexedRecord {
        refno: entry.refno,
        loc: RecordLoc {
            page: PageId::new(leaf.ext, entry.page_no),
            offset_words: entry.offset_words,
            flag: entry.flag,
        },
        leaf,
    }
}
