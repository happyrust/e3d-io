//! A transaction facade over the element write path: mint refnos, keep owner
//! member lists and the `NAME` table in step, and hand the batch to
//! [`stage_element_session_with`] — the three chores a caller of
//! [`write_element_session_with`] does by hand today
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
//!   member list; a delete takes it out. If the owner is already in the batch
//!   the pending draft is edited; otherwise the owner is read from the base and
//!   added as an update. A reference-list owner (`GROU`) is the caller's to
//!   fill — its members are not children, so an insert under it changes no list.
//! - **`NAME`** — [`Transaction::rename`], and the `name` an insert is given,
//!   set the record's `NAME` attribute and file the matching [`NamedEdit`], so
//!   the record and the `NAME` table never drift apart. The old name's key is
//!   withdrawn first, so a rename is one net change.
//! - **`TYPEX`, the reverse-reference tables and the claim tree** follow every
//!   write through [`stage_element_session_with`], because the transaction
//!   always carries the schema.
//!
//! It is not a merge: it writes on top of the newest session. Use
//! [`write_merge_session_with`](crate::write::write_merge_session_with) when the
//! database may have moved on since a long-lived reader opened it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::record::descriptor::DescriptorValue;
use crate::record::template::{MemberListKind, NounTemplate, TemplateProvider, WIDE_LAYOUT_FLAG};
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
        let mut implicit = vec![0u32; 11];
        implicit.extend_from_slice(&template.defaults.implicit);
        // A new element is written in the wide layout; word 10's format word is
        // recomputed at placement, but `set_attribute` reads its wide bit to
        // refuse a compact record, so a fresh draft must carry it.
        implicit[10] |= WIDE_LAYOUT_FLAG;
        let mut draft = RecordDraft::new(refno, noun, owner, implicit, Vec::new(), Vec::new())?;
        build(&mut draft, &template)?;
        if let Some(name) = name {
            self.set_name(&mut draft, &template, refno, None, name)?;
        }
        self.register(ElementWrite::Insert(draft));
        self.add_to_owner(owner, refno)?;
        Ok(refno)
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
        self.remove_from_owner(owner, refno)?;
        let format = self.text_format;
        Self::file_name_edits(&mut self.names, refno, format, old.as_deref(), None)?;
        Ok(())
    }

    /// Stage the batch: everything but page 0 is written and synced, and a
    /// [`StagedSession`] is returned for the commit. Element writes go out in
    /// the order they were first registered — a new element before the owner
    /// update that lists it, which is the order the primitive path is called in.
    pub fn stage(self, request: &SessionRequest<'_>) -> Result<StagedSession, WriteError> {
        let Transaction { path, templates, names, order, mut writes, .. } = self;
        let elements: Vec<ElementWrite> =
            order.iter().map(|refno| writes.remove(refno).expect("every ordered refno has a write")).collect();
        stage_element_session_with(&path, request, &elements, &names, WriteOptions { templates: Some(templates) })
    }

    /// Stage and commit in one step.
    pub fn commit(self, request: &SessionRequest<'_>) -> Result<CommitReport, WriteError> {
        self.stage(request)?.commit()
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

    /// Add `member` to `owner`'s member list — in the pending draft if the
    /// owner is in the batch, else in the base record read as an update. A
    /// reference-list owner (`GROU`) keeps the members the caller sets; a child
    /// insert does not touch its list.
    fn add_to_owner(&mut self, owner: RefNo, member: RefNo) -> Result<(), TransactionError> {
        if !owner.is_valid() {
            return Ok(()); // the WORLD's owner is null; nothing owns it.
        }
        if matches!(self.writes.get(&owner), Some(ElementWrite::Delete(_))) {
            return Err(TransactionError::OwnerDeleted { owner });
        }
        let owner_noun = self.draft_noun(owner, "an owner")?;
        if self.member_list_is_referenced(owner_noun)? {
            return Ok(());
        }
        let draft = self.draft_mut(owner, "an owner")?;
        if !draft.members().contains(&member) {
            draft.push_member(member);
        }
        Ok(())
    }

    /// Remove `member` from `owner`'s member list, if the owner is not itself
    /// gone and is a child-list noun.
    fn remove_from_owner(&mut self, owner: RefNo, member: RefNo) -> Result<(), TransactionError> {
        if !owner.is_valid() || matches!(self.writes.get(&owner), Some(ElementWrite::Delete(_))) {
            return Ok(());
        }
        if self.engine.find_element(owner)?.is_none() {
            return Ok(());
        }
        let owner_noun = self.draft_noun(owner, "an owner")?;
        if self.member_list_is_referenced(owner_noun)? {
            return Ok(());
        }
        let draft = self.draft_mut(owner, "an owner")?;
        draft.remove_member(member);
        Ok(())
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
