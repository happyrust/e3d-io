//! L2: what changed between two index roots.
//!
//! A session does not edit the tree it inherited. The index is copy-on-write,
//! so a session writes fresh pages for the paths it touched and leaves every
//! other page where it was — which is why [`crate::session::SessionPage`]
//! carries a root of its own and why two of those roots can be compared at
//! all. Two consequences run this module:
//!
//! * **A page both roots name holds the same keys in both trees.** Written
//!   once, never modified: the same page number is the same bytes. So a
//!   subtree both roots name is never read, and neither is anything under it.
//!   The budget that falls out is exact and is gated as such in
//!   `tests/index_diff_real.rs`: a diff reads the pages the two trees do not
//!   share, and not one page they do. How much that saves is the file's
//!   business rather than this module's — the newest two sessions of
//!   `ams8000_0001` share all but 6 of its 111 index pages, while
//!   `ams251309_0001` rewrote 13 of its 25 leaves in a session that changed
//!   two keys, and there is nothing there to skip.
//! * **Deletion is a key-set difference.** Not a tombstone bit and not
//!   `owner.children` containment: the callee set of core.dll's
//!   `elementsDeletedBetween` holds no owner, children or primary-list call,
//!   and the leaf `flag` reads 1 on all 940 694 entries of the corpus, so
//!   there is nothing there to read a deletion out of either.
//!
//! The differ merges two enumerations in key order. It has **no descent of its
//! own**: pages become nodes through [`read_node`], leaf values become
//! addresses through [`located`], and keys order through
//! [`crate::index::cmp_refno`] — the same three that lookup and enumeration
//! use. Routing by key stays in [`crate::index`], where a point lookup needs
//! it and a merge does not, so there is nothing here for a second tree order
//! to hide in.
//!
//! **"Unchanged" is the fourth state and is deliberately not emitted.**
//! Listing it would mean reading every key under every shared subtree, which
//! is exactly the work the prune exists to avoid; the keys the differ did
//! compare and find identical are counted in
//! [`DiffTally::unchanged_compared`], and the subtrees it declined to open are
//! counted in [`DiffTally::shared_subtrees_pruned`]. A caller that wants the
//! unchanged keys enumerated walks either tree with [`crate::index::IndexWalk`].

use std::cmp::Ordering;
use std::collections::HashSet;

use crate::index::cursor::{located, read_node};
use crate::index::{cmp_refno, IndexTreeError, IndexedRecord, RecordPosition};
use crate::page::cache::PageCache;
use crate::page::{IoStats, PageId};
use crate::refno::RefNo;

/// What the two roots say about one key, for the three states that are a
/// change. The fourth, unchanged, is the complement — see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyChange {
    /// The base root indexes this key; the target root does not.
    Deleted(IndexedRecord),
    /// The target root indexes this key; the base root does not.
    Inserted(IndexedRecord),
    /// Both roots index the key and they address different records.
    ///
    /// Compared on [`crate::index::RecordPosition`], not on the raw flag or
    /// the leaf the entry came from: rewriting a leaf moves every key on it,
    /// and a key that still addresses the same record has not changed just
    /// because its leaf was copied.
    Modified {
        base: IndexedRecord,
        target: IndexedRecord,
    },
}

impl KeyChange {
    pub fn refno(&self) -> RefNo {
        match self {
            Self::Deleted(record) | Self::Inserted(record) => record.refno,
            Self::Modified { target, .. } => target.refno,
        }
    }

    /// Project the forensic/raw locations onto the tuple core.dll exposes.
    pub const fn candidate(self) -> IndexCandidate {
        match self {
            Self::Deleted(record) => IndexCandidate::Deleted {
                refno: record.refno,
                base: record.position(),
            },
            Self::Inserted(record) => IndexCandidate::Inserted {
                refno: record.refno,
                target: record.position(),
            },
            Self::Modified { base, target } => IndexCandidate::Modified {
                refno: target.refno,
                base: base.position(),
                target: target.position(),
            },
        }
    }
}

/// The three change states exposed by core.dll's index-table comparator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexCandidate {
    Deleted {
        refno: RefNo,
        base: RecordPosition,
    },
    Inserted {
        refno: RefNo,
        target: RecordPosition,
    },
    Modified {
        refno: RefNo,
        base: RecordPosition,
        target: RecordPosition,
    },
}

impl IndexCandidate {
    pub const fn refno(self) -> RefNo {
        match self {
            Self::Deleted { refno, .. }
            | Self::Inserted { refno, .. }
            | Self::Modified { refno, .. } => refno,
        }
    }
}

/// A raw leaf-flag change that core.dll's index comparator does not classify
/// as an element modification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagOnlyDrift {
    pub refno: RefNo,
    pub position: RecordPosition,
    pub base_flag: u16,
    pub target_flag: u16,
}

/// Non-semantic observations made while comparing the two roots.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffDiagnostics {
    pub flag_only_drifts: u64,
    pub first_flag_only_drift: Option<FlagOnlyDrift>,
}

/// A completely traversed diff, safe for a caller to apply atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedIndexDiff {
    pub changes: Vec<IndexCandidate>,
    pub tally: DiffTally,
    pub diagnostics: DiffDiagnostics,
    pub io: IoStats,
}

/// What a run classified, and how much of the two trees it never opened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffTally {
    pub deleted: u64,
    pub inserted: u64,
    pub modified: u64,
    /// Keys read from both trees and found to address the same record.
    ///
    /// Keys under a pruned subtree are unchanged too and are **not** counted
    /// here — counting them would mean reading them. With
    /// [`Pruning::Disabled`] this is every unchanged key; with
    /// [`Pruning::Enabled`] it is only those that happened to sit beside a
    /// change.
    pub unchanged_compared: u64,
    /// Subtrees both roots name, skipped whole and never read.
    pub shared_subtrees_pruned: u64,
}

/// Whether the differ may skip what it can show it does not need to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pruning {
    /// Skip a subtree both roots name, and skip past a subtree whose least key
    /// already sits above the key the other tree is offering.
    Enabled,
    /// Open both trees end to end and merge every key.
    ///
    /// The same answer for more money. It exists so that a test can hold the
    /// two against each other on real databases — a prune that is only ever
    /// exercised in the fast path is a prune nobody has checked.
    Disabled,
}

/// A merge of two index roots, yielding one [`KeyChange`] per changed key in
/// key order.
///
/// Borrows the page cache both trees are read through, so `IoStats` measures
/// exactly what the diff cost. Nothing is read until the first `next`.
pub struct IndexDiff<'a> {
    pages: &'a mut PageCache,
    base: Enumerator,
    target: Enumerator,
    pruning: Pruning,
    tally: DiffTally,
    diagnostics: DiffDiagnostics,
    failed: bool,
}

impl<'a> IndexDiff<'a> {
    /// Diff `base` against `target`, skipping what copy-on-write says cannot
    /// differ.
    pub fn new(pages: &'a mut PageCache, base: PageId, target: PageId) -> Self {
        Self::with_pruning(pages, base, target, Pruning::Enabled)
    }

    pub fn with_pruning(
        pages: &'a mut PageCache,
        base: PageId,
        target: PageId,
        pruning: Pruning,
    ) -> Self {
        let page_limit = pages.physical_page_count().max(1);
        Self {
            pages,
            base: Enumerator::new(base, page_limit),
            target: Enumerator::new(target, page_limit),
            pruning,
            tally: DiffTally::default(),
            diagnostics: DiffDiagnostics::default(),
            failed: false,
        }
    }

    /// Counts so far. Complete once the iterator has run out.
    pub fn tally(&self) -> DiffTally {
        self.tally
    }

    /// Raw observations that deliberately do not create a [`KeyChange`].
    pub fn diagnostics(&self) -> DiffDiagnostics {
        self.diagnostics
    }

    pub fn stats(&self) -> IoStats {
        self.pages.stats()
    }

    /// Traverse both roots completely before returning any applyable result.
    pub fn finish(mut self) -> Result<CompletedIndexDiff, IndexTreeError> {
        let mut changes = Vec::new();
        while let Some(change) = self.next() {
            changes.push(change?.candidate());
        }
        Ok(CompletedIndexDiff {
            changes,
            tally: self.tally,
            diagnostics: self.diagnostics,
            io: self.pages.stats(),
        })
    }

    fn step(&mut self) -> Result<Option<KeyChange>, IndexTreeError> {
        loop {
            let base_head = self.base.head();
            let target_head = self.target.head();

            match (base_head, target_head) {
                (Head::End, Head::End) => return Ok(None),

                // The one prune that copy-on-write pays for directly.
                (
                    Head::Subtree {
                        page: base,
                        least_key: base_least,
                    },
                    Head::Subtree {
                        page: target,
                        least_key: target_least,
                    },
                ) => {
                    if base == target && self.pruning == Pruning::Enabled {
                        self.base.skip_head();
                        self.target.skip_head();
                        self.tally.shared_subtrees_pruned += 1;
                    } else {
                        // Open the one that starts lower, so neither walk runs
                        // ahead of the other. When a session inserts a child,
                        // the sibling after it is the same page in both trees
                        // and will pair up on the next turn — opening both here
                        // would read that shared page for nothing and leave the
                        // two walks out of step for the rest of the node.
                        match order_by_least_key(base_least, target_least) {
                            Ordering::Less => self.base.expand(self.pages)?,
                            Ordering::Greater => self.target.expand(self.pages)?,
                            Ordering::Equal => {
                                self.base.expand(self.pages)?;
                                self.target.expand(self.pages)?;
                            }
                        }
                    }
                }

                (Head::Entry(record), Head::Subtree { least_key, .. }) => {
                    if self.pruning == Pruning::Enabled && below(record.refno, least_key) {
                        self.base.take(record)?;
                        self.tally.deleted += 1;
                        return Ok(Some(KeyChange::Deleted(record)));
                    }
                    self.target.expand(self.pages)?;
                }

                (Head::Subtree { least_key, .. }, Head::Entry(record)) => {
                    if self.pruning == Pruning::Enabled && below(record.refno, least_key) {
                        self.target.take(record)?;
                        self.tally.inserted += 1;
                        return Ok(Some(KeyChange::Inserted(record)));
                    }
                    self.base.expand(self.pages)?;
                }

                // One tree has run out; the other's remaining keys are all
                // changes, so they have to be listed.
                (Head::Subtree { .. }, Head::End) => self.base.expand(self.pages)?,
                (Head::End, Head::Subtree { .. }) => self.target.expand(self.pages)?,

                (Head::Entry(base), Head::Entry(target)) => {
                    match cmp_refno(base.refno, target.refno) {
                        Ordering::Less => {
                            self.base.take(base)?;
                            self.tally.deleted += 1;
                            return Ok(Some(KeyChange::Deleted(base)));
                        }
                        Ordering::Greater => {
                            self.target.take(target)?;
                            self.tally.inserted += 1;
                            return Ok(Some(KeyChange::Inserted(target)));
                        }
                        Ordering::Equal => {
                            self.base.take(base)?;
                            self.target.take(target)?;
                            if base.position() == target.position() {
                                self.tally.unchanged_compared += 1;
                                if base.loc.raw_flag() != target.loc.raw_flag() {
                                    let drift = FlagOnlyDrift {
                                        refno: base.refno,
                                        position: base.position(),
                                        base_flag: base.loc.raw_flag(),
                                        target_flag: target.loc.raw_flag(),
                                    };
                                    self.diagnostics.flag_only_drifts += 1;
                                    self.diagnostics.first_flag_only_drift.get_or_insert(drift);
                                }
                            } else {
                                self.tally.modified += 1;
                                return Ok(Some(KeyChange::Modified { base, target }));
                            }
                        }
                    }
                }

                (Head::Entry(record), Head::End) => {
                    self.base.take(record)?;
                    self.tally.deleted += 1;
                    return Ok(Some(KeyChange::Deleted(record)));
                }
                (Head::End, Head::Entry(record)) => {
                    self.target.take(record)?;
                    self.tally.inserted += 1;
                    return Ok(Some(KeyChange::Inserted(record)));
                }
            }
        }
    }
}

impl Iterator for IndexDiff<'_> {
    type Item = Result<KeyChange, IndexTreeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.step() {
            Ok(change) => change.map(Ok),
            Err(error) => {
                self.failed = true;
                Some(Err(error))
            }
        }
    }
}

/// Every key of one tree still ahead of the merge sits at or after `least`, so
/// a key below it is a key this tree does not hold.
///
/// An internal entry's key is the least key of the subtree behind it — the
/// same reading [`crate::index::choose_child`] routes on — and the sentinel
/// opens the left edge, which bounds nothing.
fn below(refno: RefNo, least: Option<RefNo>) -> bool {
    least.is_some_and(|least| cmp_refno(refno, least) == Ordering::Less)
}

/// Which of two subtrees starts lower. The sentinel opens the left edge, so it
/// is below every key rather than at the top of the unsigned range its own
/// bytes would put it.
fn order_by_least_key(base: Option<RefNo>, target: Option<RefNo>) -> Ordering {
    match (base, target) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(base), Some(target)) => cmp_refno(base, target),
    }
}

/// One tree, in key order, with the next subtree left unread until something
/// needs it.
///
/// [`crate::index::IndexWalk`] opens a child the moment it reaches it, which is
/// right for an enumeration and wrong here: the page number is the whole
/// question, and reading the page to learn what is under it throws away the
/// answer.
struct Enumerator {
    /// The root, before it has been opened.
    pending_root: Option<PageId>,
    /// One frame per internal page on the path, deepest last.
    stack: Vec<Frame>,
    leaf: Vec<IndexedRecord>,
    slot: usize,
    last_key: Option<RefNo>,
    visited: HashSet<PageId>,
    page_limit: u32,
}

struct Frame {
    page: PageId,
    level: u32,
    children: Vec<Child>,
    next: usize,
}

#[derive(Clone, Copy)]
struct Child {
    /// Least key of the subtree, or `None` for the sentinel's left edge.
    least_key: Option<RefNo>,
    page_no: u32,
}

/// The next thing in one tree, named without reading it.
enum Head {
    Subtree {
        page: PageId,
        least_key: Option<RefNo>,
    },
    Entry(IndexedRecord),
    End,
}

impl Enumerator {
    fn new(root: PageId, page_limit: u32) -> Self {
        Self {
            pending_root: Some(root),
            stack: Vec::new(),
            leaf: Vec::new(),
            slot: 0,
            last_key: None,
            visited: HashSet::new(),
            page_limit,
        }
    }

    /// What comes next, reading nothing. Drops frames that are used up, so the
    /// answer is always the next thing in key order.
    fn head(&mut self) -> Head {
        if self.slot < self.leaf.len() {
            return Head::Entry(self.leaf[self.slot]);
        }
        if let Some(page) = self.pending_root {
            return Head::Subtree {
                page,
                least_key: None,
            };
        }
        loop {
            let Some(frame) = self.stack.last() else {
                return Head::End;
            };
            let ext = frame.page.ext;
            match frame.children.get(frame.next).copied() {
                Some(child) => {
                    return Head::Subtree {
                        page: PageId::new(ext, child.page_no),
                        least_key: child.least_key,
                    }
                }
                None => {
                    self.stack.pop();
                }
            }
        }
    }

    /// Consume the entry [`Enumerator::head`] just named.
    fn take(&mut self, record: IndexedRecord) -> Result<(), IndexTreeError> {
        if let Some(previous) = self.last_key {
            if cmp_refno(record.refno, previous) != Ordering::Greater {
                return Err(IndexTreeError::KeyOutOfOrder {
                    page: record.leaf,
                    previous,
                    found: record.refno,
                });
            }
        }
        self.last_key = Some(record.refno);
        self.slot += 1;
        Ok(())
    }

    /// Drop the subtree [`Enumerator::head`] just named, without reading it.
    fn skip_head(&mut self) {
        if self.pending_root.take().is_some() {
            return;
        }
        if let Some(frame) = self.stack.last_mut() {
            frame.next += 1;
        }
    }

    /// Read the subtree [`Enumerator::head`] just named: fill the leaf buffer,
    /// or push a frame and leave its children for the next `head`.
    fn expand(&mut self, pages: &mut PageCache) -> Result<(), IndexTreeError> {
        let (page, parent) = match self.pending_root.take() {
            Some(root) => (root, None),
            None => {
                let frame = self
                    .stack
                    .last_mut()
                    .expect("head named a subtree, so a frame has a child left");
                let child = frame.children[frame.next];
                frame.next += 1;
                (
                    PageId::new(frame.page.ext, child.page_no),
                    Some((frame.page, frame.level)),
                )
            }
        };

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

        let node = read_node(pages, page, parent)?;
        if node.is_leaf() {
            self.leaf.clear();
            self.leaf.extend(
                node.entries()
                    .filter(|entry| !entry.is_start_marker())
                    .map(|entry| located(entry, page)),
            );
            self.slot = 0;
            return Ok(());
        }

        let level = node.level;
        let children: Vec<Child> = node
            .entries()
            .map(|entry| Child {
                least_key: (!entry.is_start_marker()).then_some(entry.refno),
                page_no: entry.page_no,
            })
            .collect();
        if children.is_empty() {
            return Err(IndexTreeError::NoChildren { page, level });
        }
        self.stack.push(Frame {
            page,
            level,
            children,
            next: 0,
        });
        Ok(())
    }
}
