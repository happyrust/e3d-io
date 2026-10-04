//! A transaction facade over the element write path: mint refnos, keep owner
//! member lists and the `NAME` table in step, and hand the batch to
//! [`super::stage_element_session_with`] — the three chores a caller of
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
//!   out `max + 1`, which never collides with a live element of the base. If
//!   the database moved on before the commit and the committed session took
//!   that number, the insert is **re-minted** past the committed maximum at
//!   commit — every owner word, member list, name key and reference in the
//!   batch that named it following — and the mapping is reported
//!   ([`TransactionCommit::reminted`]).
//! - **the base and the merge** — [`Transaction::open`] opens at the newest
//!   session, [`Transaction::open_at`] at any retained one. At commit the file's
//!   newest session is compared with the base the transaction was opened at:
//!   equal, the batch is written on top of it as before; moved on, the batch is
//!   **merged** the way `core.dll`'s `SAVEWORK` merges — the three-view conflict
//!   scan ([`TransactionError::Conflicts`]: 561 / 562 / 590, nothing written),
//!   then each update rebased attribute by attribute onto the committed record
//!   ([`write_merge_session_with`](super::write_merge_session_with)) and written
//!   on top of latest with `BASESN` = the base. Where both sides changed the
//!   *same* attribute, owner or reference list, the facade **refuses** by
//!   default ([`OnClash::Refuse`], [`TransactionError::Clashes`] naming each
//!   clash) rather than let one side win silently; [`Transaction::with_on_clash`]
//!   `(OnClash::WriterWins)` is plain `SAVEWORK`. If yet another writer commits
//!   between the staging and the page-0 write ([`WriteError::Moved`]), the
//!   facade stages once more on the new latest before it gives up.
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
//!   write through [`super::stage_element_session_with`], because the transaction
//!   always carries the schema; `TYPEX` is appended after what `build` set,
//!   where E3D 3.1 puts it.
//!
//! A caller who wants the merge's tie-breaking without the facade uses
//! [`write_merge_session_with`](crate::write::write_merge_session_with) directly.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::record::descriptor::DescriptorValue;
use crate::record::template::{MemberListKind, NounTemplate, TemplateProvider};
use crate::record::text::{TextEncodeError, TextFormat};
use crate::refno::RefNo;
use crate::session::SessionSelector;
use crate::ReadOnlyEngine;

use super::attr::AttributeWriteError;
use super::dictionary::{Dictionary, DictionaryError};
use super::merge::{MergeClash, MergeConflict, MergeOptions, OnClash};
use super::named_cow::{NamedEdit, NamedEditKind};
use super::record::{RecordDraft, RecordEncodeError};
use super::value::{parse_value, ValueParseError};
use super::{CommitReport, ElementWrite, SessionRequest, StagedSession, WriteError};
use crate::spatial::{
    SpatialError, SpatialPolicy, SpatialReport, WorldAabb,
};

/// Why a transaction operation could not be recorded. Nothing is written until
/// [`Transaction::stage`] / [`Transaction::commit`]; these are raised while the
/// batch is being built, and a commit raises [`WriteError`] as before.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum TransactionError {
    #[error(transparent)]
    Spatial(#[from] SpatialError),
    #[error(transparent)]
    Write(WriteError),
    #[error(transparent)]
    Record(#[from] RecordEncodeError),
    #[error(transparent)]
    Attribute(#[from] AttributeWriteError),
    #[error("name {name:?}: {source}")]
    Name {
        name: String,
        source: TextEncodeError,
    },
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
    #[error("noun {noun:#x} has no attribute named {name}")]
    UnknownAttribute { noun: u32, name: String },
    #[error("{name} = {input:?}: {source}")]
    Value {
        name: String,
        input: String,
        source: ValueParseError,
    },
    #[error(transparent)]
    Dictionary(#[from] DictionaryError),
    /// [`Transaction::set_hunk`] was asked to store out of the record what
    /// only lives inline: a value that is not a text, a real array, an integer
    /// array or a string list; `NAME`; an attribute with an implicit slot.
    #[error("{name} cannot be stored as a hunk: {reason}")]
    NotAHunk { name: String, reason: &'static str },
    /// The database moved on since the base and the committed sessions conflict
    /// with this batch the way `core.dll` refuses a `SAVEWORK`: 561 a name
    /// taken, 562 an element changed here but deleted there, 590 deleted here
    /// but changed there (or a refno inserted here that is already there and
    /// could not be re-minted). Nothing was written.
    #[error("merge refused by {} conflict(s): {}", .0.len(), MergeConflict::summary(.0))]
    Conflicts(Vec<MergeConflict>),
    /// The database moved on since the base and both sides changed the same
    /// attribute, owner or reference list of an element, and the transaction
    /// was to refuse rather than break the tie ([`OnClash::Refuse`], the
    /// default). Nothing was written; [`Transaction::with_on_clash`] takes the
    /// writer's side instead.
    #[error("merge refused by {} clash(es): {}", .0.len(), MergeClash::summary(.0))]
    Clashes(Vec<MergeClash>),
}

impl From<WriteError> for TransactionError {
    fn from(error: WriteError) -> Self {
        match error {
            WriteError::Spatial(error) => Self::Spatial(error),
            error => Self::Write(error),
        }
    }
}

/// What [`Transaction::commit_reporting`] says about a commit beyond the
/// [`CommitReport`] the write path returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionCommit {
    pub report: CommitReport,
    /// The session the transaction was opened at.
    pub base_sesno: u32,
    /// The newest session the batch was merged onto, when the database had
    /// moved on since the base; `None` when it was written straight on top of
    /// the base.
    pub merged_onto: Option<u32>,
    /// `(the refno the insert was given, the refno it was written as)` for
    /// every insert whose number the committed session had taken meanwhile.
    pub reminted: Vec<(RefNo, RefNo)>,
    /// Whether the page-0 write found yet another session committed after the
    /// staging and the batch was staged a second time on the new latest.
    pub retried_after_move: bool,
    pub spatial: SpatialReport,
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
    Add {
        owner: RefNo,
        member: RefNo,
        at: MemberPosition,
        reference: bool,
    },
    /// Take `member` out of `owner`'s list.
    Remove {
        owner: RefNo,
        member: RefNo,
        reference: bool,
    },
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
    /// E3D's dictionary limits, applied by [`Transaction::set`] when given.
    dictionary: Option<&'a Dictionary>,
    /// The session the batch is relative to: the one `engine` reads.
    base_sesno: u32,
    /// What a merge does where both sides changed the same thing.
    on_clash: OnClash,
    spatial_policy: SpatialPolicy,
    spatial_given: std::collections::HashMap<RefNo, WorldAabb>,
}

impl<'a> Transaction<'a> {
    /// Open a transaction on `path`'s newest session with the schema at hand —
    /// [`Transaction::open_at`] with [`SessionSelector::Latest`].
    pub fn open(
        path: impl AsRef<Path>,
        templates: &'a mut TemplateProvider,
    ) -> Result<Self, TransactionError> {
        Self::open_at(path, SessionSelector::Latest, templates)
    }

    /// Open a transaction on the session `at` selects — the **base** every
    /// edit is relative to and every read (`update`, `set`, `delete`, an owner
    /// read for its member list) comes from. A commit onto a database whose
    /// newest session is no longer the base is a merge (module docs).
    ///
    /// Scans the base session's index once for the highest `word1` in each
    /// extent, so a new element can be given the next one.
    pub fn open_at(
        path: impl AsRef<Path>,
        at: impl Into<SessionSelector>,
        templates: &'a mut TemplateProvider,
    ) -> Result<Self, TransactionError> {
        let path = path.as_ref().to_path_buf();
        let mut engine = ReadOnlyEngine::open_at(&path, at)?;
        let base_sesno = engine.session().sesno;
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
            dictionary: None,
            base_sesno,
            on_clash: OnClash::Refuse,
            spatial_policy: SpatialPolicy::Require,
            spatial_given: std::collections::HashMap::new(),
        })
    }

    /// The session this transaction was opened at — what its edits are
    /// relative to, and the `BASESN` a merged commit records.
    pub fn base_sesno(&self) -> u32 {
        self.base_sesno
    }

    /// What a merged commit does where this batch and the committed sessions
    /// changed the same attribute, owner or reference list of an element:
    /// [`OnClash::Refuse`] (the default) refuses the commit with the list of
    /// clashes and writes nothing; [`OnClash::WriterWins`] takes this batch's
    /// value, as `core.dll`'s plain `SAVEWORK` does.
    pub fn with_on_clash(mut self, on_clash: OnClash) -> Self {
        self.on_clash = on_clash;
        self
    }

    /// Select how missing World_AABB values are handled when staging.
    pub fn with_spatial(mut self, policy: SpatialPolicy) -> Self {
        self.spatial_policy = policy;
        self
    }

    /// Provide or replace the World_AABB used for `refno` during spatial planning.
    pub fn set_aabb(&mut self, refno: RefNo, aabb_mm: [f64; 6]) -> Result<(), TransactionError> {
        let aabb = WorldAabb::from_mm(aabb_mm).map_err(|error| {
            let axis = match error {
                crate::spatial::InvalidAabb::NotFinite { axis }
                | crate::spatial::InvalidAabb::MinAboveMax { axis, .. } => axis,
            };
            SpatialError::InvalidAabb { refno, axis }
        })?;
        let noun = self.draft_noun(refno, "set AABB")?;
        if !matches!(crate::spatial::rules::imap_of(noun), 1 | 5 | 7) {
            return Err(SpatialError::NotGeometric { refno, noun }.into());
        }
        self.spatial_given.insert(refno, aabb);
        Ok(())
    }

    /// Apply E3D's dictionary to every [`Transaction::set`] / [`Transaction::set_hunk`]: an attribute the
    /// dictionary does not know is refused as E3D refuses it, a text is held
    /// to the dictionary's character count as well as the template's byte
    /// capacity, an array to its element count, a set-once attribute to one
    /// setting, a pseudo attribute to none (see [`super::dictionary`]).
    /// Without one, `set` checks only what the template says.
    pub fn with_dictionary(mut self, dictionary: &'a Dictionary) -> Self {
        self.dictionary = Some(dictionary);
        self
    }

    /// The header `FORMAT` reading text on this database is stored under, for a
    /// caller placing a text value.
    pub fn text_format(&self) -> TextFormat {
        self.text_format
    }

    /// Set attribute `name` of `refno` from its textual form — `"DESC"` to
    /// `"反应堆厂房"`, `"POS"` to `"1000 2000 3000"`, `"LOCK"` to `"true"`,
    /// `"SPRE"` to `"/A3B-SPEC"` or `"=13244/108564"`, `"ROKEYS"` to `"K1|K2"`,
    /// anything to `"unset"` ([`super::value`] has the table). The element may
    /// be one inserted in this transaction. The name is resolved on the noun's
    /// template (`UnknownAttribute` when it is not there), the text read into
    /// the shape the descriptor stores (`Value` when it cannot be), the value
    /// checked against the dictionary when one was given
    /// ([`Transaction::with_dictionary`]), then placed as
    /// [`RecordDraft::set_attribute`] places it — so the template's own
    /// capacity check applies as well. `NAME` goes through
    /// [`Transaction::rename`], keeping the `NAME` table in step.
    pub fn set(&mut self, refno: RefNo, name: &str, value: &str) -> Result<(), TransactionError> {
        self.set_stored(refno, name, value, false)
    }

    /// [`Transaction::set`], with the value stored **out of the record** as a
    /// hunk (storage types 20–23): a text as type 22, a real array as 21, an
    /// integer array as 20, a string list as 23 — what `core.dll` does on its
    /// own for a value past `HUNK_THRESHOLD_WORDS`, and what the E3D gate asks
    /// E3D to read back (`tools/e3d/write_gate.ps1` Q3 / Q4b). The name, the
    /// text and the dictionary are handled exactly as `set` handles them; the
    /// placing goes through [`RecordDraft::set_text_hunk`] and its siblings
    /// instead of [`RecordDraft::set_attribute`]. Refused as
    /// [`TransactionError::NotAHunk`]: a value of any other shape (a scalar, a
    /// reference, `unset` — use `set`), `NAME`, and an attribute with an
    /// implicit slot, which E3D reads instead of the explicit stream the
    /// hunk's directory entry sits in. An array past the descriptor's count
    /// is refused as an inline one is.
    pub fn set_hunk(
        &mut self,
        refno: RefNo,
        name: &str,
        value: &str,
    ) -> Result<(), TransactionError> {
        self.set_stored(refno, name, value, true)
    }

    fn set_stored(
        &mut self,
        refno: RefNo,
        name: &str,
        value: &str,
        hunk: bool,
    ) -> Result<(), TransactionError> {
        let noun = self.draft_noun(refno, "set")?;
        let template = self
            .templates
            .template_for(noun)?
            .ok_or(TransactionError::NoTemplate { noun })?
            .clone();
        let hash = e3d_attlib::db1_hash(name);
        let attr = template
            .attrs
            .iter()
            .find(|attr| attr.hash == hash)
            .ok_or_else(|| TransactionError::UnknownAttribute {
                noun,
                name: name.to_string(),
            })?
            .clone();
        let parsed = {
            let engine = &mut self.engine;
            parse_value(&attr, value, &mut |named: &str| {
                engine.find_by_name(named).map_err(|e| e.to_string())
            })
        }
        .map_err(|source| TransactionError::Value {
            name: name.to_string(),
            input: value.to_string(),
            source,
        })?;
        if let Some(dictionary) = self.dictionary {
            let already_set = self.attribute_is_set(refno, &attr)?;
            dictionary.check(hash, name, &parsed, already_set)?;
        }
        if hunk {
            return self.set_as_hunk(refno, name, &attr, &parsed);
        }
        if hash == e3d_attlib::db1_hash("NAME") {
            return match &parsed {
                DescriptorValue::Text(new_name) => {
                    self.set_name_to(refno, &template, Some(new_name))
                }
                DescriptorValue::Unset => self.set_name_to(refno, &template, None),
                other => Err(TransactionError::Value {
                    name: name.to_string(),
                    input: value.to_string(),
                    source: ValueParseError::Token {
                        token: format!("{other:?}"),
                        expected: "a name",
                    },
                }),
            };
        }
        let format = self.text_format;
        let draft = self.draft_mut(refno, "set")?;
        super::attr::set_attribute(draft, &template, hash, &parsed, format)?;
        Ok(())
    }

    /// Place a parsed value as a hunk ([`Transaction::set_hunk`]): the shape
    /// picks the hunk type, the descriptor's count bounds an array as
    /// [`super::attr::set_attribute`] bounds an inline one.
    fn set_as_hunk(
        &mut self,
        refno: RefNo,
        name: &str,
        attr: &crate::record::template::TemplateAttr,
        value: &DescriptorValue,
    ) -> Result<(), TransactionError> {
        let refuse = |reason: &'static str| TransactionError::NotAHunk {
            name: name.to_string(),
            reason,
        };
        if attr.hash == e3d_attlib::db1_hash("NAME") {
            return Err(refuse(
                "NAME is the record's name and the NAME table's key; it is set inline",
            ));
        }
        if attr.wide.location.word_offset() != 0 {
            return Err(refuse("the attribute has an implicit slot, which E3D reads instead of the explicit stream"));
        }
        let count = match value {
            DescriptorValue::Text(text) => text.len(),
            DescriptorValue::RealArray(reals) => reals.len(),
            DescriptorValue::IntArray(ints) => ints.len(),
            DescriptorValue::TextList(items) => items.len(),
            _ => return Err(refuse(
                "only a text, a real array, an integer array or a string list is stored as a hunk",
            )),
        };
        if attr.logical_count != 0
            && count > attr.logical_count as usize
            && !matches!(value, DescriptorValue::TextList(_))
        {
            return Err(AttributeWriteError::TooManyElements {
                hash: attr.hash,
                count,
                max: attr.logical_count,
            }
            .into());
        }
        let format = self.text_format;
        let draft = self.draft_mut(refno, "set")?;
        match value {
            DescriptorValue::Text(text) => draft.set_text_hunk(attr.hash, text, format)?,
            DescriptorValue::RealArray(reals) => draft.set_real_array_hunk(attr.hash, reals)?,
            DescriptorValue::IntArray(ints) => draft.set_int_array_hunk(attr.hash, ints)?,
            DescriptorValue::TextList(items) => {
                draft.set_text_list_hunk(attr.hash, items, format)?
            }
            _ => unreachable!("the shape was checked above"),
        }
        Ok(())
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
    /// the refno it was given. An owner deleted in this transaction is refused
    /// at once (`OwnerDeleted`); one that is not in the base is found out at
    /// staging, when its member list is read (`NotFound`).
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
        if matches!(self.writes.get(&owner), Some(ElementWrite::Delete(_))) {
            return Err(TransactionError::OwnerDeleted { owner });
        }
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
        self.member_ops.push(MemberOp::Add {
            owner,
            member: refno,
            at: MemberPosition::Last,
            reference: false,
        });
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
    pub fn move_to(
        &mut self,
        refno: RefNo,
        new_owner: RefNo,
        at: MemberPosition,
    ) -> Result<(), TransactionError> {
        self.draft_noun(refno, "moved")?; // exists, not deleted
        let old_owner = self
            .owner_of(refno)?
            .ok_or(TransactionError::NotFound { refno })?;
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
                return Err(TransactionError::WouldOwnItself {
                    refno,
                    owner: new_owner,
                });
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
            _ => self
                .engine
                .find_element(refno)?
                .map(|view| view.header().owner),
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
            self.member_ops.push(MemberOp::Remove {
                owner: old_owner,
                member: refno,
                reference: false,
            });
        }
        self.member_ops.push(MemberOp::Add {
            owner: new_owner,
            member: refno,
            at,
            reference: false,
        });
        Ok(())
    }

    /// Move an element within its owner's member list — E3D's `REORDER`. Only
    /// the owner is rewritten; the element's record is untouched.
    pub fn reorder(&mut self, refno: RefNo, at: MemberPosition) -> Result<(), TransactionError> {
        self.draft_noun(refno, "reordered")?;
        let owner = self
            .owner_of(refno)?
            .ok_or(TransactionError::NotFound { refno })?;
        if !owner.is_valid() {
            return Err(TransactionError::NotAMember {
                owner,
                member: refno,
            });
        }
        let owner_noun = self.draft_noun(owner, "an owner")?;
        if self.member_list_is_referenced(owner_noun)? {
            return Err(TransactionError::OwnerIsReferenceList { owner });
        }
        self.check_anchor(owner, refno, at)?;
        self.member_ops.push(MemberOp::Add {
            owner,
            member: refno,
            at,
            reference: false,
        });
        Ok(())
    }

    /// Put `member` into the reference list of `group` (a `GROU`-like noun whose
    /// members are references, not children) at `at`, or move it there if the
    /// list already holds it. The member's own owner is untouched.
    pub fn add_reference_member(
        &mut self,
        group: RefNo,
        member: RefNo,
        at: MemberPosition,
    ) -> Result<(), TransactionError> {
        let group_noun = self.draft_noun(group, "a reference list")?;
        if !self.member_list_is_referenced(group_noun)? {
            return Err(TransactionError::NotAReferenceList { owner: group });
        }
        self.draft_noun(member, "referenced")?; // exists, not deleted
        self.check_anchor(group, member, at)?;
        self.member_ops.push(MemberOp::Add {
            owner: group,
            member,
            at,
            reference: true,
        });
        Ok(())
    }

    /// Take `member` out of `group`'s reference list. Refused when the list does
    /// not hold it.
    pub fn remove_reference_member(
        &mut self,
        group: RefNo,
        member: RefNo,
    ) -> Result<(), TransactionError> {
        let group_noun = self.draft_noun(group, "a reference list")?;
        if !self.member_list_is_referenced(group_noun)? {
            return Err(TransactionError::NotAReferenceList { owner: group });
        }
        if !self.members_of(group)?.contains(&member) && !self.pending_add(group, member) {
            return Err(TransactionError::NotAMember {
                owner: group,
                member,
            });
        }
        self.member_ops.push(MemberOp::Remove {
            owner: group,
            member,
            reference: true,
        });
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
        self.set_name_to(refno, &template, Some(name))
    }

    /// Give `refno` the name `new` (or none): the record's `NAME` attribute and
    /// the `NAME` table together. The table edits are reckoned against the
    /// **base** — a key this transaction already queued for the element (an
    /// insert's name, an earlier rename) is withdrawn first, so renaming twice
    /// in one transaction files one net change, and renaming back to the
    /// stored name files none.
    fn set_name_to(
        &mut self,
        refno: RefNo,
        template: &NounTemplate,
        new: Option<&str>,
    ) -> Result<(), TransactionError> {
        let stored = self.stored_name(refno)?;
        let format = self.text_format;
        // Make sure the element is a pending draft carrying its base bytes.
        self.draft_mut(refno, "renamed")?;
        match self.writes.get_mut(&refno) {
            Some(ElementWrite::Insert(draft) | ElementWrite::Update(draft)) => {
                let value = match new {
                    Some(name) => DescriptorValue::Text(name.to_string()),
                    None => DescriptorValue::Unset,
                };
                super::attr::set_attribute(
                    draft,
                    template,
                    e3d_attlib::db1_hash("NAME"),
                    &value,
                    format,
                )?;
            }
            _ => {
                return Err(TransactionError::Deleted {
                    refno,
                    what: "renamed",
                })
            }
        }
        // Whatever this transaction had queued for the element's name is
        // superseded: the record now says `new`, the table must say the same.
        self.names.retain(|edit| edit.refno != refno);
        Self::file_name_edits(&mut self.names, refno, format, stored.as_deref(), new)?;
        Ok(())
    }

    /// Whether `attr` holds a value on `refno` as it stands — the pending
    /// draft's, else the base record's: an explicit field that is present, an
    /// implicit slot whose first word is neither an unset sentinel nor zero
    /// (an empty array or a zero scalar counts as unset here, which is the
    /// lenient reading a set-once rule wants). A packed bit is never "set".
    fn attribute_is_set(
        &mut self,
        refno: RefNo,
        attr: &crate::record::template::TemplateAttr,
    ) -> Result<bool, TransactionError> {
        let draft = match self.writes.get(&refno) {
            Some(ElementWrite::Insert(draft) | ElementWrite::Update(draft)) => draft.clone(),
            Some(ElementWrite::Delete(_)) => {
                return Err(TransactionError::Deleted { refno, what: "set" })
            }
            None => {
                let view = self
                    .engine
                    .find_element(refno)?
                    .ok_or(TransactionError::NotFound { refno })?;
                RecordDraft::from_record_bytes(&view.raw_bytes)?
            }
        };
        let offset = attr.wide.location.word_offset() as usize;
        if offset == 0 {
            return Ok(draft.explicit_words(attr.hash).is_some());
        }
        if attr.storage_type_code == 5 {
            return Ok(false);
        }
        Ok(draft.implicit().get(offset).is_some_and(|word| {
            *word != 0 && !crate::record::descriptor::UNSET_SENTINELS.contains(word)
        }))
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
        let parsed =
            crate::record::element::ParsedElement::parse_as(&view.raw_bytes, self.text_format)
                .map_err(|detail| TransactionError::Parse { refno, detail })?;
        let old = crate::element_name::stored_name(&parsed);
        self.register(ElementWrite::Delete(refno));
        // A queued placement of the element in some owner's child list (a
        // move or reorder earlier in this transaction) is withdrawn; the list
        // that holds it on disk loses it. A reference list keeps its entry —
        // that is the caller's list to edit.
        self.member_ops.retain(
            |op| !matches!(op, MemberOp::Add { member, reference: false, .. } if *member == refno),
        );
        self.member_ops.push(MemberOp::Remove {
            owner,
            member: refno,
            reference: false,
        });
        let format = self.text_format;
        // A name this transaction queued for the element (a rename) goes with it.
        self.names.retain(|edit| edit.refno != refno);
        Self::file_name_edits(&mut self.names, refno, format, old.as_deref(), None)?;
        Ok(())
    }

    /// Stage the batch: everything but page 0 is written and synced, and a
    /// [`StagedSession`] is returned for the commit. Owner member-list updates
    /// are resolved here and go out after the child inserts that name them — the
    /// order the primitive path writes in. On a database that moved on since
    /// the base this is the merge staging (module docs); a refno the committed
    /// session took is re-minted first, silently — [`Transaction::commit_reporting`]
    /// is the form that reports it.
    pub fn stage(
        mut self,
        request: &SessionRequest<'_>,
    ) -> Result<StagedSession, TransactionError> {
        self.resolve_members()?;
        let Transaction {
            path,
            templates,
            names,
            order,
            mut writes,
            base_sesno,
            on_clash,
            spatial_policy,
            spatial_given,
            ..
        } = self;
        let mut elements: Vec<ElementWrite> = order
            .iter()
            .map(|refno| {
                writes
                    .remove(refno)
                    .expect("every ordered refno has a write")
            })
            .collect();
        let mut names = names;
        let latest = ReadOnlyEngine::open(&path)?.session().sesno;
        let mut reminted = Vec::new();
        Self::stage_batch(
            &path,
            base_sesno,
            latest,
            on_clash,
            spatial_policy,
            &spatial_given,
            templates,
            request,
            &mut elements,
            &mut names,
            &mut reminted,
        )
    }

    /// Stage and commit in one step — [`Transaction::commit_reporting`] without
    /// the report of what the commit did beyond writing.
    pub fn commit(self, request: &SessionRequest<'_>) -> Result<CommitReport, TransactionError> {
        self.commit_reporting(request).map(|commit| commit.report)
    }

    /// Stage and commit, and say what happened: whether the batch was written
    /// straight on top of the base or merged onto a newer session, which
    /// inserted refnos were re-minted because the committed sessions had taken
    /// them, and whether the staging had to be redone because another writer
    /// committed between it and the page-0 write ([`WriteError::Moved`] —
    /// retried once; a second `Moved` is returned).
    pub fn commit_reporting(
        self,
        request: &SessionRequest<'_>,
    ) -> Result<TransactionCommit, TransactionError> {
        self.commit_observed(request, |_| {})
    }

    /// [`Transaction::commit_reporting`] with a look at each staging before
    /// its page 0 is written — for a caller who wants to see the mid-state
    /// (`StagedSession::staged` has the session number and pages), or, in a
    /// test, to act as the other writer and provoke the `Moved` retry.
    pub fn commit_observed(
        mut self,
        request: &SessionRequest<'_>,
        mut observe: impl FnMut(&StagedSession),
    ) -> Result<TransactionCommit, TransactionError> {
        self.resolve_members()?;
        let Transaction {
            path,
            templates,
            names,
            order,
            mut writes,
            base_sesno,
            on_clash,
            spatial_policy,
            spatial_given,
            ..
        } = self;
        let mut elements: Vec<ElementWrite> = order
            .iter()
            .map(|refno| {
                writes
                    .remove(refno)
                    .expect("every ordered refno has a write")
            })
            .collect();
        let mut names = names;
        let mut reminted: Vec<(RefNo, RefNo)> = Vec::new();
        let original_elements = elements.clone();
        let original_names = names.clone();
        let mut retried_after_move = false;
        loop {
            elements.clone_from(&original_elements);
            names.clone_from(&original_names);
            reminted.clear();
            let latest = ReadOnlyEngine::open(&path)?.session().sesno;
            let staged = Self::stage_batch(
                &path,
                base_sesno,
                latest,
                on_clash,
                spatial_policy,
                &spatial_given,
                &mut *templates,
                request,
                &mut elements,
                &mut names,
                &mut reminted,
            )?;
            let latest = staged.previous().sesno;
            observe(&staged);
            let spatial = staged.spatial().clone();
            match staged.commit() {
                Ok(report) => {
                    return Ok(TransactionCommit {
                        report,
                        base_sesno,
                        merged_onto: (latest != base_sesno).then_some(latest),
                        reminted,
                        retried_after_move,
                        spatial,
                    })
                }
                // Another writer got its page 0 in between our staging and ours:
                // our staged pages are unreachable (and partly overwritten by
                // theirs — both stagings start at the same page), so stage
                // again on what is now the newest session. Once.
                Err(WriteError::Moved { .. }) if !retried_after_move => retried_after_move = true,
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// One staging of the batch: straight on top of the base when the file's
    /// newest session is the base, else the merge — re-minting first any
    /// inserted refno the committed sessions took (`elements` and `names` are
    /// rewritten in place, `reminted` extended), then
    /// [`stage_merge_session_with`] with the facade's [`OnClash`].
    #[allow(clippy::too_many_arguments)]
    fn stage_batch(
        path: &Path,
        base_sesno: u32,
        _latest: u32,
        on_clash: OnClash,
        spatial_policy: SpatialPolicy,
        spatial_given: &std::collections::HashMap<RefNo, WorldAabb>,
        templates: &mut TemplateProvider,
        request: &SessionRequest<'_>,
        elements: &mut Vec<ElementWrite>,
        names: &mut Vec<NamedEdit>,
        reminted: &mut Vec<(RefNo, RefNo)>,
    ) -> Result<StagedSession, TransactionError> {
        let mut file = super::PageFile::open(path)?;
        file.lock_for_write()?;
        let (_, _, _, pinned) = super::read_base(&mut file)?;
        let latest = pinned.sesno;
        if latest != base_sesno {
            Self::remint_taken_refnos(path, latest, templates, elements.as_mut_slice(), names.as_mut_slice(), reminted)?;
        }
        let rebased = if latest != base_sesno {
            let mut options = MergeOptions { templates: Some(&mut *templates), extract_flush: false, on_clash };
            super::merge::prepare_merge(path, base_sesno, latest, elements, names, &mut options)
                .map_err(|error| match error {
                    WriteError::MergeConflicts(c) => TransactionError::Conflicts(c),
                    WriteError::MergeClashes(c) => TransactionError::Clashes(c),
                    error => error.into(),
                })?
        } else { elements.clone() };
        let spatial_stage = super::spamap::SpatialStage {
            policy: spatial_policy, given: spatial_given.clone(), reminted: reminted.clone(), base_sesno,
        };
        Ok(super::stage_session_locked(path, request, &rebased, names,
            (latest != base_sesno).then_some(base_sesno), Some(templates),
            Some(&spatial_stage), file, Some(pinned.page.page))?)
    }

    /// Give every insert whose refno the newest session already indexes a
    /// fresh one past that session's highest in the extent — what `core.dll`
    /// gets for free by minting at save time from page 1 — and rewrite the
    /// batch to match: the record's own refno, every owner word and member
    /// list that named it, every `NAME` key filed for it, and every reference
    /// attribute in the batch (explicit type 4 / 8 / 16 / 17 fields, and the
    /// implicit slots the templates place) that pointed at it.
    fn remint_taken_refnos(
        path: &Path,
        latest_sesno: u32,
        templates: &mut TemplateProvider,
        elements: &mut [ElementWrite],
        names: &mut [NamedEdit],
        reminted: &mut Vec<(RefNo, RefNo)>,
    ) -> Result<(), TransactionError> {
        let inserted: Vec<RefNo> = elements
            .iter()
            .filter_map(|element| match element {
                ElementWrite::Insert(draft) => Some(draft.refno),
                _ => None,
            })
            .collect();
        if inserted.is_empty() {
            return Ok(());
        }
        let mut latest = ReadOnlyEngine::open_at(path, latest_sesno)?;
        let mut taken = Vec::new();
        for refno in &inserted {
            if latest.find_element(*refno)?.is_some() {
                taken.push(*refno);
            }
        }
        if taken.is_empty() {
            return Ok(());
        }
        // The newest session's highest `word1` per extent, past which the fresh
        // numbers go — skipping any the batch already uses.
        let mut next: HashMap<u32, u32> = HashMap::new();
        for element in latest.scan_elements(crate::engine::ScanTier::Header)? {
            let element = element?;
            let entry = next.entry(element.refno.word0).or_insert(0);
            *entry = (*entry).max(element.refno.word1);
        }
        let mut used: HashSet<RefNo> = inserted.iter().copied().collect();
        for old in taken {
            let counter = next.entry(old.word0).or_insert(0);
            let new = loop {
                *counter += 1;
                if (*counter as i32) < 0 {
                    return Err(TransactionError::RefnoExhausted { word0: old.word0 });
                }
                let candidate = RefNo::new(old.word0, *counter);
                if !used.contains(&candidate) {
                    break candidate;
                }
            };
            used.insert(new);
            Self::rename_refno(templates, elements, names, old, new)?;
            match reminted.iter_mut().find(|(_, current)| *current == old) {
                Some(pair) => pair.1 = new,
                None => reminted.push((old, new)),
            }
        }
        Ok(())
    }

    /// Every mention of `old` in the batch becomes `new`.
    fn rename_refno(
        templates: &mut TemplateProvider,
        elements: &mut [ElementWrite],
        names: &mut [NamedEdit],
        old: RefNo,
        new: RefNo,
    ) -> Result<(), TransactionError> {
        let is_reference_type = |storage_type: u32| matches!(storage_type, 4 | 8 | 16 | 17);
        for element in elements.iter_mut() {
            let draft = match element {
                ElementWrite::Insert(draft) | ElementWrite::Update(draft) => draft,
                ElementWrite::Delete(_) => continue,
            };
            if draft.refno == old {
                draft.refno = new;
            }
            if draft.owner == old {
                draft.owner = new;
            }
            draft.replace_member(old, new);
            // Explicit reference fields: the stream's type code says which.
            for (hash, type_code, words) in draft.parse_explicit_fields() {
                if !is_reference_type(u32::from(type_code >> 10)) {
                    continue;
                }
                let mut changed = false;
                let mut words = words;
                for pair in words.as_chunks_mut::<2>().0 {
                    if *pair == [old.word0, old.word1] {
                        *pair = [new.word0, new.word1];
                        changed = true;
                    }
                }
                if changed {
                    draft.set_explicit_words(hash, type_code, words);
                }
            }
            // Implicit reference slots: the template says where they are.
            let Some(template) = templates.template_for(draft.noun_hash)? else {
                continue;
            };
            let slots: Vec<(usize, usize)> = template
                .attrs
                .iter()
                .filter(|attr| {
                    is_reference_type(attr.storage_type_code)
                        && attr.wide.location.word_offset() != 0
                })
                .map(|attr| {
                    (
                        attr.wide.location.word_offset() as usize,
                        attr.wide.storage_words as usize,
                    )
                })
                .collect();
            // A scalar slot is the pair itself, an array slot `[n][pairs…]`:
            // every adjacent pair in the slot is looked at, whatever the
            // alignment (a stray match of an extent number and a refno word
            // in one attribute's data is not a thing).
            for (offset, words) in slots {
                for at in offset..offset + words.saturating_sub(1) {
                    let hit = draft.implicit().get(at) == Some(&old.word0)
                        && draft.implicit().get(at + 1) == Some(&old.word1);
                    if hit {
                        *draft.implicit_word_mut(at).expect("in range") = new.word0;
                        *draft.implicit_word_mut(at + 1).expect("in range") = new.word1;
                    }
                }
            }
        }
        for edit in names.iter_mut() {
            if edit.refno == old {
                edit.refno = new;
            }
        }
        Ok(())
    }

    /// Apply the queued owner member-list changes. Each owner is read (from the
    /// pending batch or the base) and updated in place; an owner touched only
    /// for membership becomes an update appended after the explicit writes.
    fn resolve_members(&mut self) -> Result<(), TransactionError> {
        let ops = std::mem::take(&mut self.member_ops);
        for op in ops {
            match op {
                MemberOp::Add {
                    owner,
                    member,
                    at,
                    reference,
                } => self.add_to_owner(owner, member, at, reference)?,
                MemberOp::Remove {
                    owner,
                    member,
                    reference,
                } => self.remove_from_owner(owner, member, reference)?,
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
                let view = self
                    .engine
                    .find_element(refno)?
                    .ok_or(TransactionError::NotFound { refno })?;
                Ok(view.header().noun_hash)
            }
        }
    }

    /// A mutable pending draft for `refno`: the one already in the batch, or the
    /// base record read into a new update.
    fn draft_mut(
        &mut self,
        refno: RefNo,
        what: &'static str,
    ) -> Result<&mut RecordDraft, TransactionError> {
        if matches!(self.writes.get(&refno), Some(ElementWrite::Delete(_))) {
            return Err(TransactionError::Deleted { refno, what });
        }
        if !self.writes.contains_key(&refno) {
            let view = self
                .engine
                .find_element(refno)?
                .ok_or(TransactionError::NotFound { refno })?;
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
    fn add_to_owner(
        &mut self,
        owner: RefNo,
        member: RefNo,
        at: MemberPosition,
        reference: bool,
    ) -> Result<(), TransactionError> {
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
    fn remove_from_owner(
        &mut self,
        owner: RefNo,
        member: RefNo,
        reference: bool,
    ) -> Result<(), TransactionError> {
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
    fn place_member(
        draft: &mut RecordDraft,
        owner: RefNo,
        member: RefNo,
        at: MemberPosition,
    ) -> Result<(), TransactionError> {
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
                members[..index]
                    .iter()
                    .rev()
                    .find(|m| **m != member)
                    .copied()
            }
        };
        draft.move_member_after(member, anchor);
        Ok(())
    }

    /// Refuse an anchor that is neither in the owner's list as stored nor
    /// queued to be added to it in this transaction. The list itself is
    /// checked again at stage time, when the queued additions have landed.
    fn check_anchor(
        &mut self,
        owner: RefNo,
        member: RefNo,
        at: MemberPosition,
    ) -> Result<(), TransactionError> {
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
            Some(ElementWrite::Insert(draft) | ElementWrite::Update(draft)) => {
                Ok(Some(draft.owner))
            }
            Some(ElementWrite::Delete(_)) => Ok(None),
            None => Ok(self
                .engine
                .find_element(refno)?
                .map(|view| view.header().owner)),
        }
    }

    /// The member list an element has right now: the pending draft's, else the
    /// base record's; empty when the element is nowhere.
    fn members_of(&mut self, refno: RefNo) -> Result<Vec<RefNo>, TransactionError> {
        match self.writes.get(&refno) {
            Some(ElementWrite::Insert(draft) | ElementWrite::Update(draft)) => {
                Ok(draft.members().to_vec())
            }
            Some(ElementWrite::Delete(_)) => Ok(Vec::new()),
            None => {
                let Some(view) = self.engine.find_element(refno)? else {
                    return Ok(Vec::new());
                };
                let parsed = crate::record::element::ParsedElement::parse_as(
                    &view.raw_bytes,
                    self.text_format,
                )
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
        let parsed =
            crate::record::element::ParsedElement::parse_as(&view.raw_bytes, self.text_format)
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
        super::attr::set_attribute(
            draft,
            template,
            e3d_attlib::db1_hash("NAME"),
            &DescriptorValue::Text(name.to_string()),
            format,
        )?;
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
            let key = crate::index::named::encode_name_as(old, format).map_err(|source| {
                TransactionError::Name {
                    name: old.to_string(),
                    source,
                }
            })?;
            names.push(NamedEdit {
                key,
                refno,
                kind: NamedEditKind::Delete,
            });
        }
        if let Some(new) = new {
            let key = crate::index::named::encode_name_as(new, format).map_err(|source| {
                TransactionError::Name {
                    name: new.to_string(),
                    source,
                }
            })?;
            names.push(NamedEdit {
                key,
                refno,
                kind: NamedEditKind::Insert,
            });
        }
        Ok(())
    }
}
