//! A transaction facade over the element write path: mint refnos, keep owner
//! member lists and the `NAME` table in step, and hand the batch to
//! [`stage_element_session_with`] — the three chores a caller of
//! [`write_element_session_with`](super::write_element_session_with) does by hand today
//! (`docs/plans/2026-09-21-write-path-next-steps.md` S6, §6).
//!
//! ```no_run
//! # use e3d_io::record::template::TemplateProvider;
//! # use e3d_io::record::descriptor::DescriptorValue;
//! # use e3d_io::write::{Transaction, SessionRequest};
//! # use e3d_io::{RefNo};
//! # fn go(path: &str, zone: RefNo, box_noun: u32, pos_hash: u32) -> Result<(), Box<dyn std::error::Error>> {
//! let mut provider = TemplateProvider::open(r"schema\e3d31\binaries\desvir.dat", Some(0x000B_0692))?;
//! let mut tx = Transaction::open(path, &mut provider)?;
//! let the_box = tx.insert(zone, box_noun, None, |draft, t| {
//!     draft.set_attribute(t, pos_hash, &DescriptorValue::RealArray(vec![1.0, 2.0, 3.0]), tx_format())?;
//!     Ok(())
//! })?;
//! let report = tx.commit(&SessionRequest { user: "e3dio", comment: "one box", clock: None })?;
//! # let _ = (the_box, report); Ok(()) }
//! # fn tx_format() -> e3d_io::record::text::TextFormat { e3d_io::record::text::TextFormat::Utf8 }
//! ```
//!
//! What it does for you, and what it does not:
//!
//! - **refno** — a new element is given `(owner.word0, next)`, where `next` is
//!   one past the highest `word1` seen in that extent at open, handed out in
//!   sequence within the transaction. `core.dll` keeps a per-extent counter on
//!   page 1 (the bucket table) that sits above the live maximum because a
//!   deleted number is never reused (`docs/evidence/2026-09-22-core-dll-refno-allocation.md`);
//!   this crate does not maintain page 1 yet (that is A11 / S6b), so it hands
//!   out `max + 1`, which never collides with a live element and, if it
//!   collides with the latest session, is refused as conflict 590 at commit.
//! - **owner membership** — an insert adds the new element to its owner's
//!   member list; a delete takes it out; [`Transaction::move_to`] takes it out
//!   of one owner's list and puts it into another's at a [`MemberPosition`]
//!   (changing the record's owner word and nothing else in it, as `INCLUDE`
//!   does); [`Transaction::reorder`] moves it within its owner's list
//!   (`REORDER`). If the owner is already in the batch the pending draft is
//!   edited; otherwise the owner is read from the base and added as an update.
//!   A reference-list owner (`GROU`) holds references, not children: an insert
//!   under it changes no list, a child cannot be moved under it, and its list
//!   is edited through [`Transaction::add_reference_member`] /
//!   [`Transaction::remove_reference_member`].
//! - **`NAME`** — [`Transaction::rename`], and the `name` an insert is given,
//!   set the record's `NAME` attribute and file the matching [`NamedEdit`], so
//!   the record and the `NAME` table never drift apart. The old name's key is
//!   withdrawn first, so a rename is one net change.
//! - **the default record** — an insert starts from what `4CEIL` starts from:
//!   the noun's chunk-1 implicit defaults and the chunk-2 explicit prefix
//!   ([`RecordDraft::from_template`]), so a new `BRAN` opens its stream with
//!   `[ISOH DELDSG SLOREF AEXCES LEXCES LOOS]` as E3D's own does; `build` sets
//!   values on top of that.
//! - **`TYPEX`, the reverse-reference tables and the claim tree** follow every
//!   write through [`stage_element_session_with`], because the transaction
//!   always carries the schema; `TYPEX` is appended after what `build` set,
//!   where E3D 3.1 puts it.
//!
//! It is not a merge: it writes on top of the newest session. Use
//! [`write_merge_session_with`](crate::write::write_merge_session_with) when the
//! database may have moved on since a long-lived reader opened it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::record::descriptor::DescriptorValue;
use crate::record::template::{MemberListKind, NounTemplate, TemplateProvider};
use crate::record::text::{TextEncodeError, TextFormat};
use crate::refno::RefNo;
use crate::ReadOnlyEngine;

use super::attr::AttributeWriteError;
use super::named_cow::{NamedEdit, NamedEditKind};
use super::record::{RecordDraft, RecordEncodeError};
use super::{
    stage_element_session_with, CommitReport, ElementWrite, SessionRequest, StagedSession, WriteError, WriteOptions,
};

/// Why a transaction operation could not be recorded. Nothing is written until
/// [`Transaction::stage`] / [`Transaction::commit`]; these are raised while the
/// batch is being built, and a commit raises [`WriteError`] as before.
#[derive(Debug, thiserror::Error)]
pub enum TransactionError {
    #[error(transparent)]
    Write(#[from] WriteError),
    #[error(transparent)]
    Record(#[from] RecordEncodeError),
    #[error(transparent)]
    Attribute(#[from] AttributeWriteError),
    #[error("name {name:?}: {source}")]
    Name { name: String, source: TextEncodeError },
    #[error("{0}")]
    Read(#[from] crate::error::E3dReaderError),
    #[error("{0}")]
    TemplateLoad(#[from] crate::record::template::TemplateLoadError),
    #[error("record {refno} parse failed: {detail}")]
    Parse { refno: RefNo, detail: String },
    #[error("noun {noun:#x} has no template in the schema")]
    NoTemplate { noun: u32 },
    #[error("{refno} is not indexed in the base session")]
    NotFound { refno: RefNo },
    #[error("{refno} is deleted in this transaction and cannot be {what}")]
    Deleted { refno: RefNo, what: &'static str },
    #[error("owner {owner} is deleted in this transaction")]
    OwnerDeleted { owner: RefNo },
    #[error("extent {word0} has no more refnos below the sign bit")]
    RefnoExhausted { word0: u32 },
    #[error("{owner} is a reference-list noun: it has no children to move under it (use add_reference_member)")]
    OwnerIsReferenceList { owner: RefNo },
    #[error("{owner} is not a reference-list noun: its members are its children (use move_to)")]
    NotAReferenceList { owner: RefNo },
    #[error("moving {refno} under {owner} would make it its own ancestor")]
    WouldOwnItself { refno: RefNo, owner: RefNo },
    #[error("{anchor} is not a member of {owner}, so nothing can be placed next to it")]
    AnchorNotAMember { owner: RefNo, anchor: RefNo },
    #[error("{member} is not a member of {owner}")]
    NotAMember { owner: RefNo, member: RefNo },
}

/// Where a member goes in its owner's list — the positions `REORDER` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberPosition {
    /// At the end of the list (where an insert puts a new element).
    Last,
    /// At the front.
    First,
    /// Directly after this member, which must be in the list.
    After(RefNo),
    /// Directly before this member, which must be in the list.
    Before(RefNo),
}

/// One queued member-list change, applied at stage time in registration order.
#[derive(Debug, Clone, Copy)]
enum MemberOp {
    /// Put `member` into `owner`'s list at `at`. `reference` says the owner is a
    /// reference-list noun and the caller means its list (a child insert under
    /// such an owner changes nothing).
    Add { owner: RefNo, member: RefNo, at: MemberPosition, reference: bool },
    /// Take `member` out of `owner`'s list.
    Remove { owner: RefNo, member: RefNo, reference: bool },
}

/// A batch of element writes against the newest session of one database.
///
/// Held open with the schema's [`TemplateProvider`], which it needs to build a
/// new element's default record, to place values by descriptor, and to keep the
/// keyed tables. Reads the base session lazily for the records an update, a
/// rename, a delete or an owner-membership change touches.
pub struct Transaction<'a> {
    path: PathBuf,
    engine: ReadOnlyEngine,
    templates: &'a mut TemplateProvider,
    text_format: TextFormat,
    /// The element writes, in the order they were first registered.
    order: Vec<RefNo>,
    writes: HashMap<RefNo, ElementWrite>,
    names: Vec<NamedEdit>,
    /// Owner member-list changes, applied at stage time so an owner-only update
    /// lands after the child inserts that name it — the order the primitive path
    /// writes in, which keeps a new element's tail blocks (and hunks) where a
    /// hand-written batch would put them.
    member_ops: Vec<MemberOp>,
    /// Per-extent (`word0`) next `word1` to hand out; seeded from the base.
    next_word1: HashMap<u32, u32>,
}

impl<'a> Transaction<'a> {
    /// Open a transaction on `path`'s newest session with the schema at hand.
    ///
    /// Scans the base session's index once for the highest `word1` in each
    /// extent, so a new element can be given the next one.
    pub fn open(path: impl AsRef<Path>, templates: &'a mut TemplateProvider) -> Result<Self, TransactionError> {
        let path = path.as_ref().to_path_buf();
        let mut engine = ReadOnlyEngine::open(&path)?;
        let text_format = engine.text_format();
        let mut next_word1: HashMap<u32, u32> = HashMap::new();
        for element in engine.scan_elements(crate::engine::ScanTier::Header)? {
            let element = element?;
            let entry = next_word1.entry(element.refno.word0).or_insert(0);
            *entry = (*entry).max(element.refno.word1);
        }
        // Store the next number to hand out, not the max seen.
        for value in next_word1.values_mut() {
            *value += 1;
        }
        Ok(Self {
            path,
            engine,
            templates,
            text_format,
            order: Vec::new(),
            writes: HashMap::new(),
            names: Vec::new(),
            member_ops: Vec::new(),
            next_word1,
        })
    }

    /// The header `FORMAT` reading text on this database is stored under, for a
    /// caller placing a text value.
    pub fn text_format(&self) -> TextFormat {
        self.text_format
    }

    /// A fresh `word1` in `word0`'s extent, past every one seen at open and
    /// every one handed out since.
    fn allocate(&mut self, word0: u32) -> Result<RefNo, TransactionError> {
        let next = self.next_word1.entry(word0).or_insert(1);
        if (*next as i32) < 0 {
            return Err(TransactionError::RefnoExhausted { word0 });
        }
        let refno = RefNo::new(word0, *next);
        *next += 1;
        Ok(refno)
    }

    /// Insert a new element of `noun` under `owner`, built from the noun's
    /// default record and then handed to `build` for its values. The element
    /// is added to `owner`'s member list (unless `owner` is a reference-list
    /// noun, whose members `build` sets on the owner itself), and `name`, when
    /// given, is stored as its `NAME` and filed in the `NAME` table. Returns
    /// the refno it was given.
    pub fn insert(
        &mut self,
        owner: RefNo,
        noun: u32,
        name: Option<&str>,
        build: impl FnOnce(&mut RecordDraft, &NounTemplate) -> Result<(), TransactionError>,
    ) -> Result<RefNo, TransactionError> {
        let template = self
            .templates
            .template_for(noun)?
            .ok_or(TransactionError::NoTemplate { noun })?
            .clone();
        let refno = self.allocate(owner.word0)?;
        // The record `4CEIL` starts from: chunk-1 defaults behind the header,
        // the wide flag, and the chunk-2 explicit prefix seeded in order
        // (`RecordDraft::from_template`); `build` then sets values on top, and
        // `TYPEX` is appended at staging — the order E3D 3.1 itself writes.
        let mut draft = RecordDraft::from_template(refno, owner, &template)?;
        build(&mut draft, &template)?;
        if let Some(name) = name {
            self.set_name(&mut draft, &template, refno, None, name)?;
        }
        self.register(ElementWrite::Insert(draft));
        self.member_ops.push(MemberOp::Add { owner, member: refno, at: MemberPosition::Last, reference: false });
        Ok(refno)
    }

    /// Move an element under another owner — what `INCLUDE` does in E3D: the
    /// record's owner word changes, the old owner's member list loses it, the
    /// new owner's gains it at `at`, and everything else in the record (its
    /// owner-relative `POS` / `ORI` included) is left exactly as stored. Both
    /// owners are rewritten in this session, so each one's primary element is
    /// refreshed in the claim tree by the staging path as for any update.
    ///
    /// Refused: a new owner that is a reference-list noun (`GROU`; its members
    /// are references, see [`Transaction::add_reference_member`]), one deleted
    /// in this transaction, one that is the element itself or sits beneath it,
    /// and an anchor that is not a member. Moving under the current owner is a
    /// [`Transaction::reorder`].
    pub fn move_to(&mut self, refno: RefNo, new_owner: RefNo, at: MemberPosition) -> Result<(), TransactionError> {
        self.draft_noun(refno, "moved")?; // exists, not deleted
        let old_owner = self.owner_of(refno)?.ok_or(TransactionError::NotFound { refno })?;
        if old_owner == new_owner {
            return self.reorder(refno, at);
        }
        if matches!(self.writes.get(&new_owner), Some(ElementWrite::Delete(_))) {
            return Err(TransactionError::OwnerDeleted { owner: new_owner });
        }
        let owner_noun = self.draft_noun(new_owner, "an owner")?;
        if self.member_list_is_referenced(owner_noun)? {
            return Err(TransactionError::OwnerIsReferenceList { owner: new_owner });
        }
        // The new owner must not be the element or anything beneath it.
        let mut cursor = Some(new_owner);
        let mut hops = 0;
        while let Some(at_ref) = cursor {
            if at_ref == refno {
                return Err(TransactionError::WouldOwnItself { refno, owner: new_owner });
            }
            hops += 1;
            if hops > 10_000 {
                break; // a cycle already on disk is not this call's to diagnose
            }
            cursor = self.owner_of(at_ref)?.filter(|o| o.is_valid());
        }
        self.check_anchor(new_owner, refno, at)?;
        let stored_owner = match self.writes.get(&refno) {
            Some(ElementWrite::Insert(_)) => None,
            _ => self.engine.find_element(refno)?.map(|view| view.header().owner),
        };
        self.draft_mut(refno, "moved")?.owner = new_owner;
        // A queued addition to the old owner (the insert's own, or an earlier
        // move / reorder) is withdrawn rather than applied and undone, so an
        // owner the element only passed through in this transaction is not
        // rewritten for nothing. Only the owner whose stored list holds the
        // element has to lose it.
        self.member_ops.retain(|op| {
            !matches!(op, MemberOp::Add { owner, member, reference: false, .. } if *owner == old_owner && *member == refno)
        });
        if stored_owner == Some(old_owner) {
            self.member_ops.push(MemberOp::Remove { owner: old_owner, member: refno, reference: false });
        }
        self.member_ops.push(MemberOp::Add { owner: new_owner, member: refno, at, reference: false });
        Ok(())
    }

    /// Move an element within its owner's member list — E3D's `REORDER`. Only
    /// the owner is rewritten; the element's record is untouched.
    pub fn reorder(&mut self, refno: RefNo, at: MemberPosition) -> Result<(), TransactionError> {
        self.draft_noun(refno, "reordered")?;
        let owner = self.owner_of(refno)?.ok_or(TransactionError::NotFound { refno })?;
        if !owner.is_valid() {
            return Err(TransactionError::NotAMember { owner, member: refno });
        }
        let owner_noun = self.draft_noun(owner, "an owner")?;
        if self.member_list_is_referenced(owner_noun)? {
            return Err(TransactionError::OwnerIsReferenceList { owner });
        }
        self.check_anchor(owner, refno, at)?;
        self.member_ops.push(MemberOp::Add { owner, member: refno, at, reference: false });
        Ok(())
    }

    /// Put `member` into the reference list of `group` (a `GROU`-like noun whose
    /// members are references, not children) at `at`, or move it there if the
    /// list already holds it. The member's own owner is untouched.
    pub fn add_reference_member(&mut self, group: RefNo, member: RefNo, at: MemberPosition) -> Result<(), TransactionError> {
        let group_noun = self.draft_noun(group, "a reference list")?;
        if !self.member_list_is_referenced(group_noun)? {
            return Err(TransactionError::NotAReferenceList { owner: group });
        }
        self.draft_noun(member, "referenced")?; // exists, not deleted
        self.check_anchor(group, member, at)?;
        self.member_ops.push(MemberOp::Add { owner: group, member, at, reference: true });
        Ok(())
    }

    /// Take `member` out of `group`'s reference list. Refused when the list does
    /// not hold it.
    pub fn remove_reference_member(&mut self, group: RefNo, member: RefNo) -> Result<(), TransactionError> {
        let group_noun = self.draft_noun(group, "a reference list")?;
        if !self.member_list_is_referenced(group_noun)? {
            return Err(TransactionError::NotAReferenceList { owner: group });
        }
        if !self.members_of(group)?.contains(&member) && !self.pending_add(group, member) {
            return Err(TransactionError::NotAMember { owner: group, member });
        }
        self.member_ops.push(MemberOp::Remove { owner: group, member, reference: true });
        Ok(())
    }

    /// Edit an existing element. It is read from the base the first time it is
    /// named (or the pending draft is reused), handed to `edit`, and rewritten.
    pub fn update(
        &mut self,
        refno: RefNo,
        edit: impl FnOnce(&mut RecordDraft, &NounTemplate) -> Result<(), TransactionError>,
    ) -> Result<(), TransactionError> {
        let noun = self.draft_noun(refno, "updated")?;
        let template = self
            .templates
            .template_for(noun)?
            .ok_or(TransactionError::NoTemplate { noun })?
            .clone();
        let draft = self.draft_mut(refno, "updated")?;
        edit(draft, &template)
    }

    /// Rename an element: set its `NAME` attribute to `name` and file the
    /// `NAME`-table edit (withdrawing the old name's key first). A record and
    /// the table can never disagree about a name this way.
    pub fn rename(&mut self, refno: RefNo, name: &str) -> Result<(), TransactionError> {
        let noun = self.draft_noun(refno, "renamed")?;
        let template = self
            .templates
            .template_for(noun)?
            .ok_or(TransactionError::NoTemplate { noun })?
            .clone();
        let old = self.stored_name(refno)?;
        let format = self.text_format;
        // Make sure the element is a pending draft carrying its base bytes.
        self.draft_mut(refno, "renamed")?;
        match self.writes.get_mut(&refno) {
            Some(ElementWrite::Insert(draft) | ElementWrite::Update(draft)) => {
                super::attr::set_attribute(
                    draft,
                    &template,
                    e3d_attlib::db1_hash("NAME"),
                    &DescriptorValue::Text(name.to_string()),
                    format,
                )?;
            }
            _ => return Err(TransactionError::Deleted { refno, what: "renamed" }),
        }
        Self::file_name_edits(&mut self.names, refno, format, old.as_deref(), Some(name))?;
        Ok(())
    }

    /// Delete an element: drop its key, and take it out of its owner's member
    /// list and out of the `NAME` table.
    pub fn delete(&mut self, refno: RefNo) -> Result<(), TransactionError> {
        // Read the base for its owner and name before it is gone.
        let view = self
            .engine
            .find_element(refno)?
            .ok_or(TransactionError::NotFound { refno })?;
        let owner = view.header().owner;
        let parsed = crate::record::element::ParsedElement::parse_as(&view.raw_bytes, self.text_format)
            .map_err(|detail| TransactionError::Parse { refno, detail })?;
        let old = crate::element_name::stored_name(&parsed);
        self.register(ElementWrite::Delete(refno));
        // A queued placement of the element in some owner's child list (a
        // move or reorder earlier in this transaction) is withdrawn; the list
        // that holds it on disk loses it. A reference list keeps its entry —
        // that is the caller's list to edit.
        self.member_ops
            .retain(|op| !matches!(op, MemberOp::Add { member, reference: false, .. } if *member == refno));
        self.member_ops.push(MemberOp::Remove { owner, member: refno, reference: false });
        let format = self.text_format;
        Self::file_name_edits(&mut self.names, refno, format, old.as_deref(), None)?;
        Ok(())
    }

    /// Stage the batch: everything but page 0 is written and synced, and a
    /// [`StagedSession`] is returned for the commit. Owner member-list updates
    /// are resolved here and go out after the child inserts that name them — the
    /// order the primitive path writes in.
    pub fn stage(mut self, request: &SessionRequest<'_>) -> Result<StagedSession, TransactionError> {
        self.resolve_members()?;
        let Transaction { path, templates, names, order, mut writes, .. } = self;
        let elements: Vec<ElementWrite> =
            order.iter().map(|refno| writes.remove(refno).expect("every ordered refno has a write")).collect();
        Ok(stage_element_session_with(&path, request, &elements, &names, WriteOptions { templates: Some(templates) })?)
    }

    /// Stage and commit in one step.
    pub fn commit(self, request: &SessionRequest<'_>) -> Result<CommitReport, TransactionError> {
        Ok(self.stage(request)?.commit()?)
    }

    /// Apply the queued owner member-list changes. Each owner is read (from the
    /// pending batch or the base) and updated in place; an owner touched only
    /// for membership becomes an update appended after the explicit writes.
    fn resolve_members(&mut self) -> Result<(), TransactionError> {
        let ops = std::mem::take(&mut self.member_ops);
        for op in ops {
            match op {
                MemberOp::Add { owner, member, at, reference } => self.add_to_owner(owner, member, at, reference)?,
                MemberOp::Remove { owner, member, reference } => self.remove_from_owner(owner, member, reference)?,
            }
        }
        Ok(())
    }

    // --- internals ---

    /// The noun of an element, from the pending draft or the base record.
    fn draft_noun(&mut self, refno: RefNo, what: &'static str) -> Result<u32, TransactionError> {
        match self.writes.get(&refno) {
            Some(ElementWrite::Insert(draft) | ElementWrite::Update(draft)) => Ok(draft.noun_hash),
            Some(ElementWrite::Delete(_)) => Err(TransactionError::Deleted { refno, what }),
            None => {
                let view = self.engine.find_element(refno)?.ok_or(TransactionError::NotFound { refno })?;
                Ok(view.header().noun_hash)
            }
        }
    }

    /// A mutable pending draft for `refno`: the one already in the batch, or the
    /// base record read into a new update.
    fn draft_mut(&mut self, refno: RefNo, what: &'static str) -> Result<&mut RecordDraft, TransactionError> {
        if matches!(self.writes.get(&refno), Some(ElementWrite::Delete(_))) {
            return Err(TransactionError::Deleted { refno, what });
        }
        if !self.writes.contains_key(&refno) {
            let view = self.engine.find_element(refno)?.ok_or(TransactionError::NotFound { refno })?;
            let draft = RecordDraft::from_record_bytes(&view.raw_bytes)?;
            self.register(ElementWrite::Update(draft));
        }
        match self.writes.get_mut(&refno) {
            Some(ElementWrite::Insert(draft) | ElementWrite::Update(draft)) => Ok(draft),
            _ => unreachable!("just ensured a non-delete draft for {refno}"),
        }
    }

    /// Register a write, keeping first-seen order and coalescing on refno.
    fn register(&mut self, write: ElementWrite) {
        let refno = match &write {
            ElementWrite::Insert(draft) | ElementWrite::Update(draft) => draft.refno,
            ElementWrite::Delete(refno) => *refno,
        };
        if !self.writes.contains_key(&refno) {
            self.order.push(refno);
        }
        self.writes.insert(refno, write);
    }

    /// Put `member` into `owner`'s member list at `at` — in the pending draft if
    /// the owner is in the batch, else in the base record read as an update. A
    /// reference-list owner (`GROU`) keeps the members the caller sets: a child
    /// insert (`reference == false`) does not touch its list, only
    /// [`Transaction::add_reference_member`] does.
    fn add_to_owner(&mut self, owner: RefNo, member: RefNo, at: MemberPosition, reference: bool) -> Result<(), TransactionError> {
        if !owner.is_valid() {
            return Ok(()); // the WORLD's owner is null; nothing owns it.
        }
        if matches!(self.writes.get(&owner), Some(ElementWrite::Delete(_))) {
            return Err(TransactionError::OwnerDeleted { owner });
        }
        let owner_noun = self.draft_noun(owner, "an owner")?;
        if !reference && self.member_list_is_referenced(owner_noun)? {
            return Ok(());
        }
        let draft = self.draft_mut(owner, "an owner")?;
        Self::place_member(draft, owner, member, at)
    }

    /// Remove `member` from `owner`'s member list, if the owner is not itself
    /// gone. A child-list owner loses a child (`reference == false`); a
    /// reference list loses a reference only when the caller asked for that.
    fn remove_from_owner(&mut self, owner: RefNo, member: RefNo, reference: bool) -> Result<(), TransactionError> {
        if !owner.is_valid() || matches!(self.writes.get(&owner), Some(ElementWrite::Delete(_))) {
            return Ok(());
        }
        if !self.writes.contains_key(&owner) && self.engine.find_element(owner)?.is_none() {
            return Ok(());
        }
        let owner_noun = self.draft_noun(owner, "an owner")?;
        if !reference && self.member_list_is_referenced(owner_noun)? {
            return Ok(());
        }
        let draft = self.draft_mut(owner, "an owner")?;
        draft.remove_member(member);
        Ok(())
    }

    /// Put `member` at `at` in `draft`'s list: appended first when it is not
    /// there, then moved — `move_member_after` is the one list edit `REORDER`
    /// makes, so `First` / `After` / `Before` all reduce to it.
    fn place_member(draft: &mut RecordDraft, owner: RefNo, member: RefNo, at: MemberPosition) -> Result<(), TransactionError> {
        if !draft.members().contains(&member) {
            draft.push_member(member);
        }
        let anchor = match at {
            MemberPosition::Last => {
                if draft.members().last() != Some(&member) {
                    draft.remove_member(member);
                    draft.push_member(member);
                }
                return Ok(());
            }
            MemberPosition::First => None,
            MemberPosition::After(anchor) => {
                if anchor == member {
                    return Ok(());
                }
                if !draft.members().contains(&anchor) {
                    return Err(TransactionError::AnchorNotAMember { owner, anchor });
                }
                Some(anchor)
            }
            MemberPosition::Before(anchor) => {
                if anchor == member {
                    return Ok(());
                }
                let members = draft.members();
                let Some(index) = members.iter().position(|m| *m == anchor) else {
                    return Err(TransactionError::AnchorNotAMember { owner, anchor });
                };
                // The member before the anchor, skipping the member itself
                // (which is being moved out of wherever it sits).
                members[..index].iter().rev().find(|m| **m != member).copied()
            }
        };
        draft.move_member_after(member, anchor);
        Ok(())
    }

    /// Refuse an anchor that is neither in the owner's list as stored nor
    /// queued to be added to it in this transaction. The list itself is
    /// checked again at stage time, when the queued additions have landed.
    fn check_anchor(&mut self, owner: RefNo, member: RefNo, at: MemberPosition) -> Result<(), TransactionError> {
        let anchor = match at {
            MemberPosition::After(anchor) | MemberPosition::Before(anchor) => anchor,
            MemberPosition::Last | MemberPosition::First => return Ok(()),
        };
        if anchor == member {
            return Ok(());
        }
        if self.members_of(owner)?.contains(&anchor) || self.pending_add(owner, anchor) {
            return Ok(());
        }
        Err(TransactionError::AnchorNotAMember { owner, anchor })
    }

    /// Whether an addition of `member` to `owner` is already queued.
    fn pending_add(&self, owner: RefNo, member: RefNo) -> bool {
        self.member_ops
            .iter()
            .any(|op| matches!(op, MemberOp::Add { owner: o, member: m, .. } if *o == owner && *m == member))
    }

    /// The owner an element has right now: the pending draft's, else the base
    /// record's. `None` when the element is nowhere.
    fn owner_of(&mut self, refno: RefNo) -> Result<Option<RefNo>, TransactionError> {
        match self.writes.get(&refno) {
            Some(ElementWrite::Insert(draft) | ElementWrite::Update(draft)) => Ok(Some(draft.owner)),
            Some(ElementWrite::Delete(_)) => Ok(None),
            None => Ok(self.engine.find_element(refno)?.map(|view| view.header().owner)),
        }
    }

    /// The member list an element has right now: the pending draft's, else the
    /// base record's; empty when the element is nowhere.
    fn members_of(&mut self, refno: RefNo) -> Result<Vec<RefNo>, TransactionError> {
        match self.writes.get(&refno) {
            Some(ElementWrite::Insert(draft) | ElementWrite::Update(draft)) => Ok(draft.members().to_vec()),
            Some(ElementWrite::Delete(_)) => Ok(Vec::new()),
            None => {
                let Some(view) = self.engine.find_element(refno)? else {
                    return Ok(Vec::new());
                };
                let parsed = crate::record::element::ParsedElement::parse_as(&view.raw_bytes, self.text_format)
                    .map_err(|detail| TransactionError::Parse { refno, detail })?;
                Ok(parsed.members)
            }
        }
    }

    fn member_list_is_referenced(&mut self, noun: u32) -> Result<bool, TransactionError> {
        Ok(match self.templates.template_for(noun)? {
            Some(template) => template.member_list_kind == MemberListKind::Referenced,
            None => false,
        })
    }

    /// The name an element stores in the base session, if any.
    fn stored_name(&mut self, refno: RefNo) -> Result<Option<String>, TransactionError> {
        let Some(view) = self.engine.find_element(refno)? else {
            return Ok(None);
        };
        let parsed = crate::record::element::ParsedElement::parse_as(&view.raw_bytes, self.text_format)
            .map_err(|detail| TransactionError::Parse { refno, detail })?;
        Ok(crate::element_name::stored_name(&parsed))
    }

    /// Set `draft`'s `NAME` and file the table edits for an insert with a name.
    fn set_name(
        &mut self,
        draft: &mut RecordDraft,
        template: &NounTemplate,
        refno: RefNo,
        old: Option<&str>,
        name: &str,
    ) -> Result<(), TransactionError> {
        let format = self.text_format;
        super::attr::set_attribute(draft, template, e3d_attlib::db1_hash("NAME"), &DescriptorValue::Text(name.to_string()), format)?;
        Self::file_name_edits(&mut self.names, refno, format, old, Some(name))
    }

    /// Withdraw `old`'s key and file `new`'s, skipping the no-op where they are
    /// equal.
    fn file_name_edits(
        names: &mut Vec<NamedEdit>,
        refno: RefNo,
        format: TextFormat,
        old: Option<&str>,
        new: Option<&str>,
    ) -> Result<(), TransactionError> {
        if old == new {
            return Ok(());
        }
        if let Some(old) = old {
            let key = crate::index::named::encode_name_as(old, format)
                .map_err(|source| TransactionError::Name { name: old.to_string(), source })?;
            names.push(NamedEdit { key, refno, kind: NamedEditKind::Delete });
        }
        if let Some(new) = new {
            let key = crate::index::named::encode_name_as(new, format)
                .map_err(|source| TransactionError::Name { name: new.to_string(), source })?;
            names.push(NamedEdit { key, refno, kind: NamedEditKind::Insert });
        }
        Ok(())
    }
}
