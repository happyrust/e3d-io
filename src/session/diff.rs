//! What changed between two sessions, and the records behind each change.
//!
//! This is glue, deliberately. Every piece already existed on 2026-09-03 —
//! [`crate::session::resolve_pair`] turns two selectors into two session pages,
//! [`crate::index::IndexDiff`] merges the two roots those pages carry, and
//! [`crate::record::read_element_record`] reads a record from the address a
//! leaf entry names — and a caller wanting "the increment from session A to B"
//! had to string them together by hand, holding a `DbView` for the diff and a
//! `ReadOnlyEngine` for the records. This module strings them once.
//!
//! Three facts measured on the AMS sample corpus shape the API
//! (`docs/plans/2026-09-03-sesno-range-increment.md` §1):
//!
//! * **A range diff is the net difference of its two ends.** Over four sample
//!   databases the keys `diff_between(a, b)` names are always a subset of the
//!   union over the retained steps between `a` and `b`, and never a superset:
//!   an element created and deleted inside the range is in the steps and not
//!   in the range. So there are two functions, [`diff_between`] and
//!   [`diff_steps`], with the distinction in their names.
//! * **Modified means the record was rewritten, not that a value changed.**
//!   The index compares record addresses. `FTUB 24384/23262` of `ams8000_0001`
//!   was moved up 500 mm and back seven times between sessions 250 and 264 and
//!   decodes to the same 64 attribute rows at both ends; one Modified key of
//!   `ams5052_0001` 163→189 is byte-for-byte identical at both addresses. So
//!   [`ChangedElement`] carries both records and says when they are the same
//!   bytes ([`ChangeKind::Modified`]'s `relocated_only`), and anything finer is
//!   [`crate::session::delta`]'s job.
//! * **A deleted element's record is still there.** The index is copy-on-write
//!   and a retained session's pages are never reused, so the base address of a
//!   Deleted key reads back the whole record: `BOX 24384/26247` of
//!   `ams8000_0001`, deleted in session 254, reads 204 bytes at session 253's
//!   address. Deletions therefore come with their content, not just their key.
//!
//! Records are read one change at a time through [`ChangedElements`], because
//! one range of `ams7000_0001` (237→263) rewrites 68 971 records and holding
//! every one of them would be 68 971 `Vec<u8>`s for nothing.

use std::path::Path;

use crate::error::{E3dReaderError, SessionError};
use crate::index::{
    CompletedIndexDiff, DiffTally, IndexCandidate, IndexDiff, IndexEntry, RecordPosition,
};
use crate::meta::DbDescriptor;
use crate::page::cache::PageCache;
use crate::record::{self, ElementRecordView};
use crate::refno::RefNo;
use crate::session::{resolve_pair, walk_chain, SessionPage, SessionSelector};

/// The index difference between two sessions of one file.
///
/// `from` and `to` are the sessions actually resolved — with
/// [`SessionSelector::AtOrBefore`] that can be an older session than the one
/// named, and a caller that needs to know how far back it went reads it here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDiff {
    pub from: SessionPage,
    pub to: SessionPage,
    pub index: CompletedIndexDiff,
}

impl SessionDiff {
    pub fn tally(&self) -> DiffTally {
        self.index.tally
    }

    /// The changed keys in key order, with their record addresses.
    pub fn changes(&self) -> &[IndexCandidate] {
        &self.index.changes
    }

    pub fn is_empty(&self) -> bool {
        self.index.changes.is_empty()
    }

    /// Every changed key, in key order, with the records behind it.
    ///
    /// Reads through `pages`, which is the same cache the diff was made with
    /// when the caller goes through [`crate::session::DbView::changed_elements`]
    /// or `ReadOnlyEngine::changed_elements`. Nothing is read until the
    /// iterator is driven.
    pub fn elements<'a>(&'a self, pages: &'a mut PageCache) -> ChangedElements<'a> {
        ChangedElements {
            pages,
            changes: self.index.changes.iter(),
        }
    }
}

/// Which of the three changes a key underwent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Indexed by `to` and not by `from`.
    Inserted,
    /// Indexed by `from` and not by `to`.
    Deleted,
    /// Indexed by both, at different addresses.
    Modified {
        /// The two addresses hold the same bytes: the record was copied, not
        /// changed. One of the 54 Modified keys of `ams5052_0001` 163→189 is
        /// one of these, and a consumer that treats Modified as "values
        /// changed" would re-process it for nothing.
        relocated_only: bool,
    },
}

impl ChangeKind {
    pub const fn is_inserted(self) -> bool {
        matches!(self, Self::Inserted)
    }

    pub const fn is_deleted(self) -> bool {
        matches!(self, Self::Deleted)
    }

    pub const fn is_modified(self) -> bool {
        matches!(self, Self::Modified { .. })
    }

    /// The name the CLI and the JSON output use.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Inserted => "inserted",
            Self::Deleted => "deleted",
            Self::Modified { .. } => "modified",
        }
    }
}

/// One changed key with the record at each end of the range.
///
/// `before` is read at the `from` session's address and is present for
/// Deleted and Modified; `after` is read at the `to` session's address and is
/// present for Inserted and Modified. Both carry the whole record — header,
/// implicit region and every tail block — so `ParsedElement::parse` runs on
/// either without another read.
#[derive(Debug, Clone)]
pub struct ChangedElement {
    pub refno: RefNo,
    pub kind: ChangeKind,
    pub before: Option<ElementRecordView>,
    pub after: Option<ElementRecordView>,
}

impl ChangedElement {
    /// The record as it stands at the `to` session, or, for a deletion, as it
    /// stood at the `from` session: whichever side exists, preferring the newer.
    pub fn record(&self) -> &ElementRecordView {
        self.after
            .as_ref()
            .or(self.before.as_ref())
            .expect("a change has a record on at least one side")
    }
}

/// [`SessionDiff::elements`]: one [`ChangedElement`] per changed key, in key
/// order, each read as the iterator reaches it.
pub struct ChangedElements<'a> {
    pages: &'a mut PageCache,
    changes: std::slice::Iter<'a, IndexCandidate>,
}

impl ChangedElements<'_> {
    /// Changes not yet read.
    pub fn remaining(&self) -> usize {
        self.changes.len()
    }
}

impl Iterator for ChangedElements<'_> {
    type Item = Result<ChangedElement, E3dReaderError>;

    fn next(&mut self) -> Option<Self::Item> {
        let change = *self.changes.next()?;
        Some(read_change(self.pages, change))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.changes.size_hint()
    }
}

/// Diff two sessions named by selector.
///
/// Both selectors are resolved on one walk of the chain. `from` must not be
/// newer than `to`: a reversed range is [`SessionError::ReversedRange`], not a
/// silently swapped pair, because a caller that got the order wrong has every
/// insertion and deletion backwards and should hear about it. The same
/// session at both ends is an empty diff that reads nothing.
pub fn diff_between(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
    path: &Path,
    from: SessionSelector,
    to: SessionSelector,
) -> Result<SessionDiff, E3dReaderError> {
    let ([from, to], _read) = resolve_pair(pages, descriptor, from, to, path)?;
    diff_pages(pages, path, from, to)
}

/// The retained sessions from `from` to `to`, each diffed against the next.
///
/// This is the event history the range hides: `ams8000_0001` 1→281 is 6605
/// insertions as a range and 46 436 changes over 280 steps, because 19 651
/// keys were created and deleted again along the way. Each step carries its
/// key set and addresses; records are read on demand through
/// [`SessionDiff::elements`] as for any other diff.
///
/// Only the sessions still on the chain take part — a session `MERGE CHANGES`
/// folded away cannot be stepped through, and the step across the hole is
/// the diff of the two sessions either side of it.
pub fn diff_steps(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
    path: &Path,
    from: SessionSelector,
    to: SessionSelector,
) -> Result<Vec<SessionDiff>, E3dReaderError> {
    let ([from, to], _read) = resolve_pair(pages, descriptor, from, to, path)?;
    check_order(path, &from, &to)?;

    // Newest first off the chain; oldest first for stepping.
    let mut within: Vec<SessionPage> = walk_chain(pages, descriptor)?
        .into_iter()
        .filter(|session| session.sesno >= from.sesno && session.sesno <= to.sesno)
        .collect();
    within.reverse();

    let mut steps = Vec::with_capacity(within.len().saturating_sub(1));
    for pair in within.windows(2) {
        steps.push(diff_pages(pages, path, pair[0].clone(), pair[1].clone())?);
    }
    Ok(steps)
}

/// Diff two resolved session pages.
fn diff_pages(
    pages: &mut PageCache,
    path: &Path,
    from: SessionPage,
    to: SessionPage,
) -> Result<SessionDiff, E3dReaderError> {
    check_order(path, &from, &to)?;
    let index = IndexDiff::new(pages, from.index_root, to.index_root)
        .finish()
        .map_err(|source| E3dReaderError::IndexError {
            page_no: to.index_root.page,
            detail: source.to_string(),
        })?;
    Ok(SessionDiff { from, to, index })
}

fn check_order(path: &Path, from: &SessionPage, to: &SessionPage) -> Result<(), SessionError> {
    if from.sesno > to.sesno {
        return Err(SessionError::ReversedRange {
            path: path.to_path_buf(),
            from: from.sesno,
            to: to.sesno,
        });
    }
    Ok(())
}

fn read_change(
    pages: &mut PageCache,
    change: IndexCandidate,
) -> Result<ChangedElement, E3dReaderError> {
    match change {
        IndexCandidate::Inserted { refno, target } => Ok(ChangedElement {
            refno,
            kind: ChangeKind::Inserted,
            before: None,
            after: Some(read_at(pages, refno, target)?),
        }),
        IndexCandidate::Deleted { refno, base } => Ok(ChangedElement {
            refno,
            kind: ChangeKind::Deleted,
            before: Some(read_at(pages, refno, base)?),
            after: None,
        }),
        IndexCandidate::Modified {
            refno,
            base,
            target,
        } => {
            let before = read_at(pages, refno, base)?;
            let after = read_at(pages, refno, target)?;
            let relocated_only = before.raw_bytes == after.raw_bytes;
            Ok(ChangedElement {
                refno,
                kind: ChangeKind::Modified { relocated_only },
                before: Some(before),
                after: Some(after),
            })
        }
    }
}

/// The record at one index address, confirmed to be the element the index
/// says it is.
///
/// The extent comes from the address, which inherited it from the root it was
/// found under — so a record in a sibling extent is read from that file. A
/// record that answers to a different RefNo is
/// [`E3dReaderError::IndexRecordMismatch`], as it is for a lookup: the address
/// resolved to somebody else's record, which is a different fact from the
/// address resolving to nothing.
fn read_at(
    pages: &mut PageCache,
    refno: RefNo,
    at: RecordPosition,
) -> Result<ElementRecordView, E3dReaderError> {
    let entry = IndexEntry {
        refno,
        page_no: at.page.page,
        offset_words: at.offset_words,
        flag: 0,
    };
    let view = record::read_element_record(pages, at.page.ext, &entry)?;
    let stated = RefNo::new(
        u32::from_be_bytes(view.raw_bytes[4..8].try_into().expect("record header")),
        u32::from_be_bytes(view.raw_bytes[8..12].try_into().expect("record header")),
    );
    if stated != refno {
        return Err(E3dReaderError::IndexRecordMismatch {
            refno,
            page_no: at.page.page,
            byte_offset: at.offset_words * 2,
            found: stated,
        });
    }
    Ok(view)
}
