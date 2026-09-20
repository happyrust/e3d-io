//! Multi-write merge — the conflict scan of a three-way save
//! (`docs/plans/2026-09-20-dabacon-write-plan.md` §2 阶段 3, A3a).
//!
//! `core.dll`'s `DOPENW` save, when the database has moved on since the writer
//! opened it, merges three views before it writes: **base**, the session the
//! writer opened at; **latest**, the newest committed session; and **working**,
//! base plus the writer's own edits. It compares base↔latest (what everyone
//! else did) with base↔working (what I did) and refuses the whole save on a
//! conflict — 561 a name I take is already taken, 562 an element I changed was
//! deleted under me, 590 an element I deleted was changed under me.
//!
//! This module does that scan against the file's committed state and, when it
//! is clean, writes the writer's edits on top of `latest` (their `BASESN`
//! recording the base they merged from, A3a.8). It is the scan and the
//! disjoint-merge write; the per-attribute and per-member reconciliation of a
//! *non-disjoint* change — two writers editing one element — is A3b, still to
//! come, so an element both sides changed is refused here rather than
//! silently resolved one way.

use std::collections::HashSet;

use crate::index::named::name_table_hash;
use crate::index::{IndexCandidate, IndexCursor, IndexDiff, NamedTable};
use crate::refno::RefNo;
use crate::session::DbView;

use super::{write_session_with_base, CommitReport, ElementWrite, NamedEdit, NamedEditKind, SessionRequest, WriteError};

/// `core.dll`'s conflict codes, from message group 43 (`460 + code`).
pub mod code {
    /// A name the writer takes is already given to another element.
    pub const NAME_TAKEN: u32 = 561;
    /// An element the writer changed was deleted by the committed session.
    pub const CHANGED_UNDER_DELETE: u32 = 562;
    /// An element the writer deleted was changed by the committed session.
    pub const DELETED_UNDER_CHANGE: u32 = 590;
    /// Not a `core.dll` code: two writers changed one element, which needs the
    /// per-attribute merge (A3b) this crate does not do yet.
    pub const OVERLAPPING_CHANGE: u32 = 0;
}

/// One reason a merge was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeConflict {
    /// A `core.dll` code (561/562/590), or 0 for the overlap this crate cannot
    /// yet merge.
    pub code: u32,
    pub refno: Option<RefNo>,
    pub name: Option<String>,
    pub detail: String,
}

impl MergeConflict {
    pub(crate) fn summary(conflicts: &[MergeConflict]) -> String {
        conflicts
            .iter()
            .map(|c| c.detail.clone())
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Which elements the writer's edits touch, classified.
#[derive(Default)]
struct WorkingChanges {
    inserted: HashSet<RefNo>,
    modified: HashSet<RefNo>,
    deleted: HashSet<RefNo>,
}

impl WorkingChanges {
    fn of(elements: &[ElementWrite]) -> Self {
        let mut out = Self::default();
        for element in elements {
            match element {
                ElementWrite::Insert(draft) => out.inserted.insert(draft.refno),
                ElementWrite::Update(draft) => out.modified.insert(draft.refno),
                ElementWrite::Delete(refno) => out.deleted.insert(*refno),
            };
        }
        out
    }
}

/// Merge the writer's edits onto the file's latest session, refusing on a
/// conflict with what was committed since `base_sesno`.
///
/// `elements` and `name_edits` are the writer's changes relative to the base
/// session (the one they opened at). On a clean merge they are written on top
/// of the latest session, exactly as [`write_element_session`] writes a plain
/// change, with the session's `BASESN` set to `base_sesno`.
///
/// [`write_element_session`]: super::write_element_session
pub fn write_merge_session(
    path: impl AsRef<std::path::Path>,
    request: &SessionRequest<'_>,
    base_sesno: u32,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
) -> Result<CommitReport, WriteError> {
    let path = path.as_ref();
    let conflicts = scan_conflicts(path, base_sesno, elements, name_edits)?;
    if !conflicts.is_empty() {
        return Err(WriteError::MergeConflicts(conflicts));
    }
    write_session_with_base(path, request, elements, name_edits, Some(base_sesno))
}

/// The three-view conflict scan. Opens the base and latest sessions read-only
/// and compares base↔latest with the writer's edits.
fn scan_conflicts(
    path: &std::path::Path,
    base_sesno: u32,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
) -> Result<Vec<MergeConflict>, WriteError> {
    let working = WorkingChanges::of(elements);

    let base_root = DbView::open_at(path, base_sesno)?.index_root();
    let mut latest = DbView::open(path)?;
    let latest_root = latest.index_root();

    // base ↔ latest: what the committed session changed since the writer's base.
    let committed = IndexDiff::new(latest.pages_mut(), base_root, latest_root)
        .finish()
        .map_err(|e| other(e.to_string()))?;
    let mut theirs_deleted = HashSet::new();
    let mut theirs_changed = HashSet::new();
    for candidate in &committed.changes {
        match candidate {
            IndexCandidate::Deleted { refno, .. } => {
                theirs_deleted.insert(*refno);
            }
            IndexCandidate::Inserted { refno, .. } | IndexCandidate::Modified { refno, .. } => {
                theirs_changed.insert(*refno);
            }
        }
    }

    let mut conflicts = Vec::new();

    // 562: I changed an element they deleted.
    for refno in &working.modified {
        if theirs_deleted.contains(refno) {
            conflicts.push(MergeConflict {
                code: code::CHANGED_UNDER_DELETE,
                refno: Some(*refno),
                name: None,
                detail: format!("562: {refno} was changed here but deleted in the committed session"),
            });
        }
    }
    // 590: I deleted an element they changed (or re-created).
    for refno in &working.deleted {
        if theirs_changed.contains(refno) {
            conflicts.push(MergeConflict {
                code: code::DELETED_UNDER_CHANGE,
                refno: Some(*refno),
                name: None,
                detail: format!("590: {refno} was deleted here but changed in the committed session"),
            });
        }
    }
    // 590 (RefNo clash): I inserted a RefNo the latest session already indexes.
    for refno in &working.inserted {
        if latest_indexes(&mut latest, latest_root, *refno)? {
            conflicts.push(MergeConflict {
                code: code::DELETED_UNDER_CHANGE,
                refno: Some(*refno),
                name: None,
                detail: format!("590: {refno} was inserted here but the committed session already has it"),
            });
        }
    }
    // Overlap this crate cannot yet merge: both sides changed one element (a
    // modify on each). A3b would reconcile it attribute by attribute; until
    // then it is refused rather than resolved one way.
    for refno in &working.modified {
        if theirs_changed.contains(refno) {
            conflicts.push(MergeConflict {
                code: code::OVERLAPPING_CHANGE,
                refno: Some(*refno),
                name: None,
                detail: format!(
                    "{refno} was changed on both sides; per-attribute merge (A3b) is not implemented yet"
                ),
            });
        }
    }

    // 561: a name I take is already in the latest NAME table for another element.
    let name_hash = name_table_hash();
    if name_edits.iter().any(|e| e.kind == NamedEditKind::Insert) {
        if let Ok(mut table) = NamedTable::open(&mut latest, name_hash) {
            for edit in name_edits {
                if edit.kind != NamedEditKind::Insert {
                    continue;
                }
                if let Some(existing) = table.lookup(&edit.key).map_err(|e| other(e.to_string()))? {
                    if existing != edit.refno {
                        conflicts.push(MergeConflict {
                            code: code::NAME_TAKEN,
                            refno: Some(existing),
                            name: Some(edit.key.text()),
                            detail: format!(
                                "561: the name {:?} is already taken by {existing} in the committed session",
                                edit.key.text()
                            ),
                        });
                    }
                }
            }
        }
    }

    Ok(conflicts)
}

/// Whether `refno` is indexed by the latest session — one descent.
fn latest_indexes(latest: &mut DbView, root: crate::page::PageId, refno: RefNo) -> Result<bool, WriteError> {
    let found = IndexCursor::at_root(latest.pages_mut(), root)
        .seek(refno)
        .map_err(|e| other(e.to_string()))?;
    Ok(found.found.is_some())
}

/// Wrap a lower layer's error string as a crate error, then a write error.
fn other(detail: String) -> WriteError {
    WriteError::from(crate::error::E3dReaderError::Other(detail))
}
