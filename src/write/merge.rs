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
//! each anchored after its preceding shared sibling — an addition first
//! passing `sub_5B20CA0`'s gate on the member element itself, looked up in the
//! base and latest sessions: not added when the committed side deleted it,
//! nor, under extract flush, when the committed side changed its owner
//! ([`MemberStanding`]) — except on a noun whose
//! template says its members are references (`GROU`; word 3 of the template
//! header is 2), where the list is one attribute and the writer's whole list
//! replaces the committed one (`sub_5B1F660`). Knowing the noun takes the
//! schema's `*vir.dat`, so [`write_merge_session_with`] accepts a
//! [`TemplateProvider`] through [`MergeOptions`]; without one every noun
//! merges member by member. An element the merge re-parents is also taken
//! out of the member list of the owner the committed record named, the way
//! `sub_5B211C0` does it, whether or not the writer handed that owner in.

use std::collections::{HashMap, HashSet};

use crate::index::named::name_table_hash;
use crate::index::{IndexCandidate, IndexCursor, IndexDiff, NamedTable};
use crate::record::template::{MemberListKind, TemplateProvider};
use crate::refno::RefNo;
use crate::session::DbView;

use super::record::{
    member_list_diff, three_way_clashes, three_way_with_standing, ClashSite, MemberEdit,
    MemberMerge, MemberStanding, MergeMode,
};
use super::{
    CommitReport, ElementWrite, NamedEdit, NamedEditKind, RecordDraft,
    SessionRequest, StagedSession, WriteError,
};
use crate::ReadOnlyEngine;

/// What a merge does where the writer and the committed session changed the
/// **same** attribute, owner or reference list of one element to different
/// ends (a change to different attributes, or to a member-by-member list,
/// composes and is no clash).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnClash {
    /// Break the tie by [`MergeMode`]: the writer's value under plain
    /// `SAVEWORK`, the committed one under extract flush — what `core.dll`
    /// does, silently. The default at this level.
    #[default]
    WriterWins,
    /// Refuse the whole merge with the list of clashes
    /// ([`WriteError::MergeClashes`]) and write nothing, so the caller can
    /// look at what the other side did and decide. The
    /// [`Transaction`](super::Transaction) facade's default.
    Refuse,
}

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
    /// Whether a same-attribute clash is broken ([`OnClash::WriterWins`], by
    /// `extract_flush`) or refused ([`OnClash::Refuse`]).
    pub on_clash: OnClash,
}

/// One thing both the writer and the committed session changed on one
/// element, to different ends — what [`OnClash::Refuse`] refuses a merge over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeClash {
    pub refno: RefNo,
    pub site: ClashSite,
    /// The attribute's hash: the explicit field's, or — with the templates at
    /// hand — the attribute whose implicit slot holds the clashing word.
    pub attribute: Option<u32>,
    pub detail: String,
}

impl MergeClash {
    pub(crate) fn summary(clashes: &[MergeClash]) -> String {
        clashes
            .iter()
            .map(|c| c.detail.clone())
            .collect::<Vec<_>>()
            .join("; ")
    }
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
            .map_or(MemberListKind::Unknown, |template| {
                template.member_list_kind
            });
        Ok(if kind == MemberListKind::Referenced {
            MemberMerge::WholeList
        } else {
            MemberMerge::MemberByMember
        })
    }

    /// A [`MergeClash`] for `site` on `refno` of `noun_hash`, the attribute
    /// named where the templates can name it.
    fn describe_clash(
        &mut self,
        refno: RefNo,
        noun_hash: u32,
        site: ClashSite,
    ) -> Result<MergeClash, WriteError> {
        let attribute = match site {
            ClashSite::ExplicitAttribute(hash) => Some(hash),
            ClashSite::ImplicitWord(index) => match self.templates.as_deref_mut() {
                Some(templates) => templates.template_for(noun_hash)?.and_then(|template| {
                    template
                        .attrs
                        .iter()
                        .find(|attr| {
                            let offset = attr.wide.location.word_offset() as usize;
                            offset != 0
                                && (offset..offset + attr.wide.storage_words as usize)
                                    .contains(&index)
                        })
                        .map(|attr| attr.hash)
                }),
                None => None,
            },
            ClashSite::Owner | ClashSite::MemberList => None,
        };
        let name = attribute.map(|hash| {
            e3d_attlib::lookup_system_name(hash)
                .map(str::to_string)
                .or_else(|| e3d_attlib::db1_dehash(hash))
                .unwrap_or_else(|| format!("{hash:#x}"))
        });
        let detail = match (site, &name) {
            (ClashSite::Owner, _) => format!(
                "{refno}: moved here and moved in the committed session, to different owners"
            ),
            (ClashSite::MemberList, _) => {
                format!("{refno}: its reference list was changed here and in the committed session")
            }
            (ClashSite::ImplicitWord(index), None) => {
                format!(
                    "{refno}: implicit word {index} was changed here and in the committed session"
                )
            }
            (_, Some(name)) => {
                format!("{refno}: {name} was changed here and in the committed session")
            }
            (ClashSite::ExplicitAttribute(hash), None) => {
                format!(
                    "{refno}: attribute {hash:#x} was changed here and in the committed session"
                )
            }
        };
        Ok(MergeClash {
            refno,
            site,
            attribute,
            detail,
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
    write_merge_session_with(
        path,
        request,
        base_sesno,
        elements,
        name_edits,
        MergeOptions::default(),
    )
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
    options: MergeOptions<'_>,
) -> Result<CommitReport, WriteError> {
    stage_merge_session_with(path, request, base_sesno, elements, name_edits, options)?.commit()
}

/// The two-phase form of [`write_merge_session_with`]: the conflict scan, the
/// clash scan ([`OnClash`]), the rebase, and the staging of every page but
/// page 0 — leaving the commit to [`StagedSession::commit`], which refuses
/// ([`WriteError::Moved`]) if yet another session was committed meanwhile.
/// Nothing is written when a conflict or a refused clash comes back.
pub fn stage_merge_session_with(
    path: impl AsRef<std::path::Path>,
    request: &SessionRequest<'_>,
    base_sesno: u32,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
    options: MergeOptions<'_>,
) -> Result<StagedSession, WriteError> {
    let mut file = super::PageFile::open(path.as_ref())?;
    file.lock_for_write()?;
    let (_, _, _, latest) = super::read_base(&mut file)?;
    let mut options = options;
    let rebased = prepare_merge(path.as_ref(), base_sesno, latest.sesno, elements, name_edits, &mut options)?;
    super::stage_session_locked(path, request, &rebased, name_edits, Some(base_sesno),
        options.templates, None, file, Some(latest.page.page))
}

/// Scan and rebase against the same session fixed by the caller's write lock.
pub(crate) fn prepare_merge(
    path: &std::path::Path, base_sesno: u32, latest_sesno: u32,
    elements: &[ElementWrite], name_edits: &[NamedEdit], options: &mut MergeOptions<'_>,
) -> Result<Vec<ElementWrite>, WriteError> {
    let conflicts = scan_conflicts(path, base_sesno, latest_sesno, elements, name_edits)?;
    if !conflicts.is_empty() { return Err(WriteError::MergeConflicts(conflicts)); }
    rebase_updates(path, base_sesno, latest_sesno, elements, options)
}

/// Rebase each of the writer's element updates onto the committed record, so a
/// change composes with what was committed since the base rather than
/// overwriting it. Inserts and deletes pass through unchanged.
///
/// An element the merge re-parents — the writer moved it and the merge took
/// the writer's owner — is also taken out of the member list of the owner the
/// committed record named, `sub_5B211C0`'s fix-up ([`detach_from_committed_owner`]),
/// so the writer need not have handed that owner's record in: when the
/// committed side had moved the element elsewhere meanwhile, the writer could
/// not have known which list to take it out of.
fn rebase_updates(
    path: &std::path::Path,
    base_sesno: u32,
    latest_sesno: u32,
    elements: &[ElementWrite],
    options: &mut MergeOptions<'_>,
) -> Result<Vec<ElementWrite>, WriteError> {
    let mode = if options.extract_flush {
        MergeMode::ExtractFlush
    } else {
        MergeMode::PlainSaveWork
    };
    let mut base = ReadOnlyEngine::open_at(path, base_sesno)?;
    let mut latest = ReadOnlyEngine::open_at(path, latest_sesno)?;
    let mut out = Vec::with_capacity(elements.len());
    // `(element, the owner the committed record names)` for each element the
    // merge re-parents; fixed up after every update is rebased.
    let mut re_parented: Vec<(RefNo, RefNo)> = Vec::new();
    // What both sides changed, for a caller who refuses rather than breaks a
    // tie; collected across the whole batch so the refusal names all of it.
    let mut clashes: Vec<MergeClash> = Vec::new();
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
                        if options.on_clash == OnClash::Refuse {
                            for site in
                                three_way_clashes(&base_draft, &latest_draft, working, member_merge)
                            {
                                clashes.push(options.describe_clash(
                                    working.refno,
                                    latest_draft.noun_hash,
                                    site,
                                )?);
                            }
                        }
                        // `sub_5B20CA0`'s lookups for each member the writer
                        // added, done up front so a read failure is this
                        // function's error rather than a silent "unknown".
                        let standing = match member_merge {
                            MemberMerge::MemberByMember => member_standing(
                                &mut base,
                                &mut latest,
                                base_draft.members(),
                                working.members(),
                            )?,
                            MemberMerge::WholeList => HashMap::new(),
                        };
                        let merged = three_way_with_standing(
                            &base_draft,
                            &latest_draft,
                            working,
                            member_merge,
                            mode,
                            |member| standing.get(&member).copied(),
                        )?;
                        // `sub_5B211C0`: the writer moved the element (base ≠
                        // working owner) and the merge kept the writer's owner
                        // — plain `SAVEWORK` unless the committed side had
                        // already moved it to the same place, extract flush
                        // unless the committed side moved it at all — so the
                        // owner the committed record names must let go of it.
                        let writer_moved = working.owner != base_draft.owner;
                        if writer_moved
                            && merged.owner == working.owner
                            && merged.owner != latest_draft.owner
                        {
                            re_parented.push((working.refno, latest_draft.owner));
                        }
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
    if !clashes.is_empty() {
        return Err(WriteError::MergeClashes(clashes));
    }
    for (member, committed_owner) in re_parented {
        detach_from_committed_owner(&mut out, &mut latest, committed_owner, member)?;
    }
    Ok(out)
}

/// `sub_5B211C0`'s member-list fix-up for an element the merge re-parents: on
/// the owner the *committed* record names (`4GTE` it in the merged state),
/// locate the element in its member list (`4FA` with `FIRST`) and delete that
/// entry (`sub_5AEB0B0` + `sub_5AB3F30`); an owner no longer indexed (error
/// 10) or a list without the element (error 6) is tolerated and nothing is
/// done. In the merged state the owner is whatever the batch already carries
/// for it — its rebased update (or insert) loses the member, a delete needs
/// nothing — else the committed record, read and re-emitted without the
/// member as one more update.
fn detach_from_committed_owner(
    out: &mut Vec<ElementWrite>,
    latest: &mut ReadOnlyEngine,
    committed_owner: RefNo,
    member: RefNo,
) -> Result<(), WriteError> {
    for element in out.iter_mut() {
        match element {
            ElementWrite::Update(draft) | ElementWrite::Insert(draft)
                if draft.refno == committed_owner =>
            {
                draft.remove_member(member);
                return Ok(());
            }
            ElementWrite::Delete(refno) if *refno == committed_owner => return Ok(()),
            _ => {}
        }
    }
    let Some(view) = latest.find_element(committed_owner)? else {
        return Ok(());
    };
    let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes)?;
    if draft.remove_member(member) {
        out.push(ElementWrite::Update(draft));
    }
    Ok(())
}

/// Where each member the writer added to a list stands in the base and latest
/// sessions — `sub_5B20CA0`'s `4GTE` on the member itself in each view, and
/// its `OWNREF` where indexed — keyed by member. Only additions
/// ([`MemberEdit::Added`] in the diff of the writer's base list against its
/// working list) are looked up; removals and moves carry no gate.
fn member_standing(
    base: &mut ReadOnlyEngine,
    latest: &mut ReadOnlyEngine,
    base_members: &[RefNo],
    working_members: &[RefNo],
) -> Result<HashMap<RefNo, MemberStanding>, WriteError> {
    let mut out = HashMap::new();
    for edit in member_list_diff(base_members, working_members) {
        let MemberEdit::Added { member, .. } = edit else {
            continue;
        };
        let owner_in = |engine: &mut ReadOnlyEngine| -> Result<Option<RefNo>, WriteError> {
            Ok(engine.find_element(member)?.map(|view| view.header().owner))
        };
        let standing = MemberStanding {
            base_owner: owner_in(base)?,
            latest_owner: owner_in(latest)?,
        };
        out.insert(member, standing);
    }
    Ok(out)
}

/// The three-view conflict scan. Opens the base and latest sessions read-only
/// and compares base↔latest with the writer's edits.
fn scan_conflicts(
    path: &std::path::Path,
    base_sesno: u32,
    latest_sesno: u32,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
) -> Result<Vec<MergeConflict>, WriteError> {
    let working = WorkingChanges::of(elements);

    let base_root = DbView::open_at(path, base_sesno)?.index_root();
    let mut latest = DbView::open_at(path, latest_sesno)?;
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
                detail: format!(
                    "562: {refno} was changed here but deleted in the committed session"
                ),
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
                detail: format!(
                    "590: {refno} was deleted here but changed in the committed session"
                ),
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
                detail: format!(
                    "590: {refno} was inserted here but the committed session already has it"
                ),
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
fn latest_indexes(
    latest: &mut DbView,
    root: crate::page::PageId,
    refno: RefNo,
) -> Result<bool, WriteError> {
    let found = IndexCursor::at_root(latest.pages_mut(), root)
        .seek(refno)
        .map_err(|e| other(e.to_string()))?;
    Ok(found.found.is_some())
}

/// Wrap a lower layer's error string as a crate error, then a write error.
fn other(detail: String) -> WriteError {
    WriteError::from(crate::error::E3dReaderError::Other(detail))
}
