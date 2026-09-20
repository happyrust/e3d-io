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
//! recording the base they merged from, A3a.8). Two writers editing one
//! element are reconciled attribute by attribute (A3b,
//! [`three_way`](super::record::three_way)): the
//! writer's change is rebased onto the committed record, so edits to different
//! attributes compose and an edit to the same attribute is the writer's.
//! Member lists reconcile the way A3b.4 does (`sub_5B1FF60`/`sub_5B20CA0`):
//! the writer's adds, removes and moves are replayed onto the committed list,
//! each anchored after its preceding shared sibling — except on a noun whose
//! template says its members are references (`GROU`; word 3 of the template
//! header is 2), where the list is one attribute and the writer's whole list
//! replaces the committed one (`sub_5B1F660`). Knowing the noun takes the
//! schema's `*vir.dat`, so [`write_merge_session_with`] accepts a
//! [`TemplateProvider`] through [`MergeOptions`]; without one every noun
//! merges member by member.

use std::collections::HashSet;

use crate::index::named::name_table_hash;
use crate::index::{IndexCandidate, IndexCursor, IndexDiff, NamedTable};
use crate::record::template::{MemberListKind, TemplateProvider};
use crate::refno::RefNo;
use crate::session::DbView;

use super::record::{three_way_with_mode, MemberMerge, MergeMode};
use super::{stage_session_with_base, CommitReport, ElementWrite, NamedEdit, NamedEditKind, RecordDraft, SessionRequest, WriteError};
use crate::ReadOnlyEngine;

/// What a merge may consult beyond the file itself.
#[derive(Default)]
pub struct MergeOptions<'a> {
    /// The schema templates for the database's type (`desvir.dat` for a
    /// DESIGN database), used to tell a noun whose member list is a reference
    /// list ([`MemberListKind::Referenced`]) — merged whole — from one whose
    /// members are its children — merged member by member, and to keep the
    /// keyed tables (`TYPEX`, the reverse-reference tables) in step with what
    /// is written ([`super::WriteOptions`]). `None` merges every noun member
    /// by member and carries the tables over untouched.
    pub templates: Option<&'a mut TemplateProvider>,
    /// Break a same-attribute clash the extract-flush way — the committed
    /// (latest) side wins — rather than the plain `SAVEWORK` way where the
    /// writer wins ([`MergeMode`]). This is `core.dll`'s fourth claims-manager
    /// slot (`DXSWK2`): flushing an extract back to the master keeps what the
    /// master gained since the extract was taken. `false` is plain `SAVEWORK`.
    pub extract_flush: bool,
}

impl MergeOptions<'_> {
    /// The member-list arm for an element of `noun_hash`: whole-list only when
    /// the templates are at hand and say the noun's members are references;
    /// a noun the templates do not know merges member by member.
    fn member_merge_for(&mut self, noun_hash: u32) -> Result<MemberMerge, WriteError> {
        let Some(templates) = self.templates.as_deref_mut() else {
            return Ok(MemberMerge::MemberByMember);
        };
        let kind = templates
            .template_for(noun_hash)?
            .map_or(MemberListKind::Unknown, |template| template.member_list_kind);
        Ok(if kind == MemberListKind::Referenced {
            MemberMerge::WholeList
        } else {
            MemberMerge::MemberByMember
        })
    }
}

/// `core.dll`'s conflict codes, from message group 43 (`460 + code`).
pub mod code {
    /// A name the writer takes is already given to another element.
    pub const NAME_TAKEN: u32 = 561;
    /// An element the writer changed was deleted by the committed session.
    pub const CHANGED_UNDER_DELETE: u32 = 562;
    /// An element the writer deleted was changed by the committed session.
    pub const DELETED_UNDER_CHANGE: u32 = 590;
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
/// This is [`write_merge_session_with`] at [`MergeOptions::default`]: every
/// noun's member list merges member by member.
///
/// [`write_element_session`]: super::write_element_session
pub fn write_merge_session(
    path: impl AsRef<std::path::Path>,
    request: &SessionRequest<'_>,
    base_sesno: u32,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
) -> Result<CommitReport, WriteError> {
    write_merge_session_with(path, request, base_sesno, elements, name_edits, MergeOptions::default())
}

/// [`write_merge_session`] with [`MergeOptions`]: hand it the schema's
/// templates and a noun whose members are references (`GROU`) has its list
/// merged whole, the way `core.dll` does.
pub fn write_merge_session_with(
    path: impl AsRef<std::path::Path>,
    request: &SessionRequest<'_>,
    base_sesno: u32,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
    mut options: MergeOptions<'_>,
) -> Result<CommitReport, WriteError> {
    let path = path.as_ref();
    let conflicts = scan_conflicts(path, base_sesno, elements, name_edits)?;
    if !conflicts.is_empty() {
        return Err(WriteError::MergeConflicts(conflicts));
    }
    let rebased = rebase_updates(path, base_sesno, elements, &mut options)?;
    // The same templates keep the keyed tables (`TYPEX`, the reverse-reference
    // tables) in step with the records the merge writes on top of latest.
    stage_session_with_base(path, request, &rebased, name_edits, Some(base_sesno), options.templates)?.commit()
}

/// Rebase each of the writer's element updates onto the committed record, so a
/// change composes with what was committed since the base rather than
/// overwriting it. Inserts and deletes pass through unchanged.
fn rebase_updates(
    path: &std::path::Path,
    base_sesno: u32,
    elements: &[ElementWrite],
    options: &mut MergeOptions<'_>,
) -> Result<Vec<ElementWrite>, WriteError> {
    let mode = if options.extract_flush {
        MergeMode::ExtractFlush
    } else {
        MergeMode::PlainSaveWork
    };
    let mut base = ReadOnlyEngine::open_at(path, base_sesno)?;
    let mut latest = ReadOnlyEngine::open(path)?;
    let mut out = Vec::with_capacity(elements.len());
    for element in elements {
        match element {
            ElementWrite::Update(working) => {
                let base_view = base.find_element(working.refno)?;
                let latest_view = latest.find_element(working.refno)?;
                match (base_view, latest_view) {
                    (Some(base_view), Some(latest_view)) => {
                        let base_draft = RecordDraft::from_record_bytes(&base_view.raw_bytes)?;
                        let latest_draft = RecordDraft::from_record_bytes(&latest_view.raw_bytes)?;
                        let member_merge = options.member_merge_for(latest_draft.noun_hash)?;
                        let merged = three_way_with_mode(&base_draft, &latest_draft, working, member_merge, mode)?;
                        // INDEX de-duplication (A3a, sub_5B07080): a merged
                        // record equal to the committed one is not written —
                        // its leaf keeps pointing at the record already there.
                        if !merged.content_eq(&latest_draft) {
                            out.push(ElementWrite::Update(merged));
                        }
                    }
                    // Not indexed by both ends: apply the writer's record as is.
                    _ => out.push(ElementWrite::Update(working.clone())),
                }
            }
            other => out.push(other.clone()),
        }
    }
    Ok(out)
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
    // A change on both sides is not a conflict: `three_way` reconciles it
    // attribute by attribute at write time.

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
