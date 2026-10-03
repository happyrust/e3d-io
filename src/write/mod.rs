//! Writing to a dabacon file: append a session, commit it through page 0.
//!
//! The shape is `core.dll`'s (`docs/evidence/2026-09-20-core-dll-dabacon-write-path-algorithms.md`,
//! A4–A7): pages already on disk are never modified, new pages take the page
//! numbers after the last one the newest session wrote and are appended in
//! order, and the only word that turns them into "the database" is page 0's
//! `LSPGNO` (+0x28/+0x2C), written last. A crash anywhere before that write
//! leaves a file every reader still sees as the previous session.
//!
//! Stage 1 (`docs/plans/2026-09-20-dabacon-write-plan.md` §2) is the smallest
//! closed loop over that shape: a session that changes nothing — same index
//! root, same claim root, same table directory — with its own number, chain
//! link, clocks, user, comment and `BASESN`. [`stage_empty_session`] writes
//! the new page and returns a [`StagedSession`] whose [`StagedSession::commit`]
//! writes page 0; the two steps are separate so that a test can stop between
//! them and read the file as a crash would have left it. Stages 2 and 3 —
//! element writes ([`stage_element_session`]) and merges — use the same
//! two-phase [`StagedSession`].
//!
//! [`Transaction`] wraps all of this for a caller who would rather not mint
//! refnos, thread `NamedEdit`s and edit owner member lists by hand: it opens
//! on the newest session, `insert` / `update` / `rename` / `delete` build the
//! batch, and `commit` hands it here.
//!
//! Entering write mode takes `core.dll`'s sentinel lock — an exclusive
//! `LockFileEx` on one byte at `0x7FFFFFF6FFFFFFFF` (A9),
//! [`PageFile::lock_for_write`] — so a file a real E3D (or another instance of
//! this crate) is writing is refused at once with [`WriteError::Locked`],
//! nothing written; the lock is held until the [`StagedSession`] commits or
//! rolls back. [`StagedSession::commit`] also re-reads page 0 and refuses when
//! `LSPGNO` moved since staging ([`WriteError::Moved`]), which is the whole
//! guard on a platform without `LockFileEx`, where the lock is a no-op.

pub mod attr;
// The copy-on-write mechanics of the tree tables and the claim plan are the
// write path's internals — a caller drives the three steps (open a transaction
// or session, change values, commit), not the per-leaf edits. They are
// `pub(crate)`; only their error types (reachable from [`WriteError`]) and the
// user-facing `NamedEdit` are re-exported below.
pub(crate) mod claim;
pub(crate) mod claim_cow;
pub mod dictionary;
pub(crate) mod index_cow;
pub(crate) mod keyed_cow;
pub mod merge;
pub(crate) mod named_cow;
pub mod pages;
pub mod record;
pub mod reverse;
pub mod session_page;
pub mod transaction;
pub mod value;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::SessionError;
use crate::meta::descriptor::at as page0;
use crate::meta::DbDescriptor;
use crate::page::{PageId, PageSize, PageType};
use crate::record::hunk;
use crate::record::template::TemplateProvider;
use crate::record::text::TextFormat;
use crate::refno::RefNo;
use crate::session::SessionPage;

pub use attr::{set_attribute, AttributeWriteError};
// Error types are part of [`WriteError`], so they stay public; the edit and
// key types are the mechanics `plan_side_tables` and `place_record` use
// internally.
pub use claim_cow::ClaimCowError;
pub use index_cow::IndexCowError;
pub use keyed_cow::KeyedCowError;
#[doc(hidden)]
pub use keyed_cow::{build_table, rewrite_table, KeyedKey, KeyedRewrite, TableEdit, TableEditKind};
use index_cow::{EditKind, LeafEdit};
use keyed_cow::KeyedEditKind;
pub use merge::{
    stage_merge_session_with, write_merge_session, write_merge_session_with, MergeClash, MergeConflict, MergeOptions,
    OnClash,
};
pub use named_cow::{NamedCowError, NamedEdit, NamedEditKind};
pub use pages::PageFile;
pub use record::{
    ClashSite, HunkDraft, MemberMerge, MemberStanding, MergeMode, PlacedRecord, RecordDraft, RecordEncodeError,
    FORMAT_COUNT_MAX,
};
pub use reverse::{
    ensure_typex, reference_array, reference_value, set_reference, set_reference_array, table_keys, typex_table_hash,
};
pub use dictionary::{Dictionary, DictionaryEntry, DictionaryError, DictionaryLoadError};
pub use session_page::{build_session_page, NewSession, SessionClock, E3DIO_VERSION_TAG};
pub use transaction::{MemberPosition, Transaction, TransactionCommit, TransactionError};
pub use value::{parse_value, ValueParseError};

/// Why a write did not happen. Nothing in this enum is returned after page 0
/// has been written: past that point the session is committed.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: page {page_no}: {source}")]
    IoPage {
        path: PathBuf,
        page_no: u32,
        source: std::io::Error,
    },
    #[error("{path}: page 0 states a page size of {words} words: {detail}")]
    PageSize {
        path: PathBuf,
        words: u32,
        detail: String,
    },
    #[error("page {page_no}: buffer is {len} bytes, a page is {want}")]
    PageLength { page_no: u32, len: usize, want: usize },
    #[error("{0}")]
    Descriptor(#[from] crate::error::E3dReaderError),
    #[error("{0}")]
    Session(#[from] SessionError),
    #[error("{page}: previous session page is {len} bytes, need {need} to carry it over")]
    PreviousSessionPageTooShort { page: PageId, len: usize, need: usize },
    #[error("{page}: page type {page_type} is not a session page")]
    NotASessionPage { page: PageId, page_type: u32 },
    #[error("{field}: {source}")]
    Text {
        field: &'static str,
        source: crate::record::text::TextEncodeError,
    },
    #[error("{field}: {text:?} holds a control character; the session page is printed as it is")]
    TextControl { field: &'static str, text: String },
    #[error("{field}: {len} bytes, the field holds at most {max}")]
    TextTooLong {
        field: &'static str,
        len: usize,
        max: usize,
    },
    #[error("{path}: extent {ext} — only single-extent files (extent 1) are written")]
    MultiExtent { path: PathBuf, ext: u32 },
    #[error("{path}: the file is in write mode — another writer holds the sentinel lock; nothing was changed")]
    Locked { path: PathBuf },
    #[error(
        "{path}: page 0 moved while staging — LSPGNO was {expected}, is {found}; \
         another writer committed, nothing was changed"
    )]
    Moved {
        path: PathBuf,
        expected: u32,
        found: u32,
    },
    #[error("{path}: file holds {physical} pages but the newest session ends at page {end}")]
    FileShorterThanSession {
        path: PathBuf,
        physical: u32,
        end: u32,
    },
    #[error("a name was written but the session has no NAME table (hash {name_hash:#x})")]
    NoNameTable { name_hash: u32 },
    #[error("merge refused by {} conflict(s): {}", .0.len(), MergeConflict::summary(.0))]
    MergeConflicts(Vec<MergeConflict>),
    /// Both the writer and the committed session changed the same attribute,
    /// owner or reference list of an element, and the merge was asked to
    /// refuse rather than break the tie ([`merge::OnClash::Refuse`]). Nothing
    /// was written.
    #[error("merge refused by {} clash(es): {}", .0.len(), MergeClash::summary(.0))]
    MergeClashes(Vec<MergeClash>),
    #[error("{0}")]
    Record(#[from] RecordEncodeError),
    #[error("{0}")]
    IndexCow(#[from] IndexCowError),
    #[error("{0}")]
    NameTable(#[from] NamedCowError),
    #[error("{0}")]
    KeyedTable(#[from] KeyedCowError),
    #[error("{0}")]
    ClaimTable(#[from] ClaimCowError),
    #[error(
        "the session page's table directory holds {count} tables and has room for {room}; \
         a new reverse-reference table cannot be listed"
    )]
    TableDirectoryFull { count: u32, room: u32 },
    #[error(
        "a record's header and implicit region are {words} words, more than a {page_words}-word \
         page can hold; the implicit region cannot span pages (only the tail blocks do)"
    )]
    RecordTooLong { words: usize, page_words: usize },
    /// A tail block of `0x3FFF` words or more ends in a zero word. Its count in
    /// the record's format word saturates at `0x3FFF`
    /// ([`record::FORMAT_COUNT_MAX`]), and a block whose count reads `0x3FFF`
    /// is measured by `core.dll` along its chain with the zero words at the end
    /// of the last segment dropped (`sub_5AEA500`), so E3D would read this
    /// block short. A member list never ends in zero (a member's `word0` is its
    /// database number); an explicit stream does when its last field's data
    /// does — store such a value as a hunk, or place it earlier in the stream.
    #[error(
        "{refno}: the {block} block is {words} words, at or past the format word's 0x3FFF, and ends \
         in a zero word that core.dll's chain-length reading would drop"
    )]
    SaturatedBlockEndsInZero { refno: RefNo, block: &'static str, words: usize },
    /// The schema templates a write was given could not be read.
    #[error("{0}")]
    Template(#[from] crate::record::template::TemplateLoadError),
}

/// What an element write may consult beyond the file itself.
///
/// With the schema's templates at hand the write keeps the keyed tables the
/// way `core.dll` does ([`reverse`]): a new element of a noun whose template
/// lists `TYPEX` gets its `TYPEX` field and key, and every type-16 reference
/// attribute set, changed, cleared or deleted with its element files or
/// withdraws its edge in the table named after the attribute — creating the
/// table when the directory has none. Without templates the tables are
/// carried over untouched, as before: a new element is then not found by
/// type, and its references have no reverse entry.
#[derive(Default)]
pub struct WriteOptions<'a> {
    /// `desvir.dat`'s templates for a DESIGN database, `catvir.dat`'s for a
    /// CATALOGUE one.
    pub templates: Option<&'a mut TemplateProvider>,
}

impl WriteError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    fn io_page(path: &Path, page_no: u32, source: std::io::Error) -> Self {
        Self::IoPage {
            path: path.to_path_buf(),
            page_no,
            source,
        }
    }
}

/// What a session to be written says about itself.
#[derive(Debug, Clone)]
pub struct SessionRequest<'a> {
    /// `USER`; ASCII, at most 36 bytes.
    pub user: &'a str,
    /// `DESC`; ASCII, at most 632 bytes.
    pub comment: &'a str,
    /// `DATE` / `UDATE`. `None` takes the current time, with the local clock
    /// offset the previous session page states (0 when it states none).
    pub clock: Option<SessionClock>,
}

/// A staged session: every new page (data, index, name-table, session) is on
/// disk and synced, but page 0 still names the old session, so a reader sees
/// the previous session until [`StagedSession::commit`] writes page 0.
///
/// Dropping this without committing leaves the file exactly as readable as
/// before: the appended pages are past the newest session's `end` and no
/// pointer reaches them, which is what a crash between the two syncs would
/// leave. [`StagedSession::rollback`] trims them off again.
#[derive(Debug)]
pub struct StagedSession {
    file: PageFile,
    previous: SessionSummary,
    written: SessionSummary,
    page0: Vec<u8>,
    old_len_bytes: u64,
    appended_pages: Vec<u32>,
}

/// The session a write is based on, as page 0 and its session page state it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSummary {
    pub sesno: u32,
    pub page: PageId,
    pub end: PageId,
}

/// What [`StagedSession::commit`] leaves behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReport {
    pub path: PathBuf,
    pub previous: SessionSummary,
    pub written: SessionSummary,
    /// Pages this commit appended, in the order they were written; page 0 is
    /// rewritten in place and not listed.
    pub appended_pages: Vec<u32>,
}

/// Read page 0 and the newest session page, refusing anything stage 1 cannot
/// write on: a multi-extent file, a file shorter than its own session says.
fn read_base(file: &mut PageFile) -> Result<(Vec<u8>, DbDescriptor, Vec<u8>, SessionPage), WriteError> {
    let page0 = file.read_page(0)?;
    let descriptor = DbDescriptor::parse(&page0)?;
    if descriptor.ext_no != u32::from(PageId::PRIMARY) {
        return Err(WriteError::MultiExtent {
            path: file.path().to_path_buf(),
            ext: descriptor.ext_no,
        });
    }
    let text = TextFormat::from_header(descriptor.header_format);
    let latest = PageId::new(PageId::PRIMARY, descriptor.latest_ses_pgno);
    let session_bytes = file.read_page(latest.page)?;
    let session = SessionPage::parse_as(latest, &session_bytes, text)?;
    if session.end.ext != PageId::PRIMARY {
        return Err(WriteError::MultiExtent {
            path: file.path().to_path_buf(),
            ext: u32::from(session.end.ext),
        });
    }
    let physical = file.physical_page_count()?;
    if physical <= session.end.page {
        return Err(WriteError::FileShorterThanSession {
            path: file.path().to_path_buf(),
            physical,
            end: session.end.page,
        });
    }
    Ok((page0, descriptor, session_bytes, session))
}

/// Stage a session that changes nothing, leaving page 0 for
/// [`StagedSession::commit`].
///
/// The new page is `newest.end + 1` — the next page after the last one the
/// newest session wrote, whatever the file's physical length says (a file
/// may carry pages past `end`; `core.dll` defines the file size by `LASTSP`,
/// not by the OS length, §4.2 of the write-path note).
pub fn stage_empty_session(
    path: impl AsRef<Path>,
    request: &SessionRequest<'_>,
) -> Result<StagedSession, WriteError> {
    stage_session_with_base(path, request, &[], &[], None, None)
}

impl StagedSession {
    /// The session this write is based on.
    pub fn previous(&self) -> SessionSummary {
        self.previous
    }

    /// Where the staged session page lies and what number it carries.
    pub fn staged(&self) -> SessionSummary {
        self.written
    }

    pub fn page_size(&self) -> PageSize {
        self.file.page_size()
    }

    /// The pages this staging appended, sorted.
    pub fn appended_pages(&self) -> &[u32] {
        &self.appended_pages
    }

    /// Write page 0 and sync: the commit point.
    ///
    /// Page 0 is re-read first; if its `LSPGNO` no longer names the session
    /// this write was staged on, another writer got there and nothing is
    /// written ([`WriteError::Moved`]). The staged pages stay on disk,
    /// unreachable, exactly as a crashed `core.dll` would leave them.
    pub fn commit(mut self) -> Result<CommitReport, WriteError> {
        let current = self.file.read_page(0)?;
        let found = pages::be_word(&current, page0::LATEST_SES_PGNO);
        if found != self.previous.page.page {
            return Err(WriteError::Moved {
                path: self.file.path().to_path_buf(),
                expected: self.previous.page.page,
                found,
            });
        }
        debug_assert_eq!(
            PageType::from_raw(pages::be_word(&self.page0, page0::PAGE_TYPE)),
            PageType::Descriptor
        );
        self.file.write_page(0, &self.page0)?;
        self.file.sync()?;
        Ok(CommitReport {
            path: self.file.path().to_path_buf(),
            previous: self.previous,
            written: self.written,
            appended_pages: self.appended_pages,
        })
    }

    /// Give the appended bytes back: truncate the file to its length before
    /// staging. Page 0 was never touched, so the file is byte-identical to
    /// what it was.
    pub fn rollback(self) -> Result<(), WriteError> {
        let path = self.file.path().to_path_buf();
        drop(self.file);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .map_err(|source| WriteError::io(&path, source))?;
        file.set_len(self.old_len_bytes)
            .map_err(|source| WriteError::io(&path, source))
    }
}

/// [`stage_empty_session`] followed by [`StagedSession::commit`].
pub fn write_empty_session(
    path: impl AsRef<Path>,
    request: &SessionRequest<'_>,
) -> Result<CommitReport, WriteError> {
    stage_empty_session(path, request)?.commit()
}

/// One element change in a session.
///
/// `Insert` adds a new element (a key the tree does not have); `Update`
/// rewrites an existing element's record and re-points its index leaf at the
/// new location (a record is never edited in place — A2, §4.5); `Delete`
/// removes a key. A deleted element's old record bytes stay on disk,
/// unreferenced, so `read_element_direct` at the old address still reads them
/// (§4.5: the page is not reused).
#[derive(Debug, Clone)]
pub enum ElementWrite {
    Insert(RecordDraft),
    Update(RecordDraft),
    Delete(RefNo),
}

/// Append a session that writes `elements`: new element records on fresh data
/// pages, the RefNo index rewritten copy-on-write to point at them, then the
/// session page and the page-0 commit (`docs/plans/2026-09-20-dabacon-write-plan.md`
/// §2 阶段 2).
///
/// Every new page — data, index and the session page — is appended after the
/// previous session's last page and written before page 0, so a crash before
/// the final page-0 write leaves the file reading as the previous session,
/// exactly as [`write_empty_session`] does. [`stage_element_session`] returns
/// the [`StagedSession`] for a caller that wants to observe that mid-state.
///
/// This is [`write_element_session_with`] at [`WriteOptions::default`]: the
/// keyed tables (`TYPEX`, the reverse-reference tables) are carried over
/// untouched.
pub fn write_element_session(
    path: impl AsRef<Path>,
    request: &SessionRequest<'_>,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
) -> Result<CommitReport, WriteError> {
    stage_element_session(path, request, elements, name_edits)?.commit()
}

/// [`write_element_session`] with [`WriteOptions`]: hand it the schema's
/// templates and the keyed tables are kept the way `core.dll` keeps them.
pub fn write_element_session_with(
    path: impl AsRef<Path>,
    request: &SessionRequest<'_>,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
    options: WriteOptions<'_>,
) -> Result<CommitReport, WriteError> {
    stage_element_session_with(path, request, elements, name_edits, options)?.commit()
}

/// Stage a session that writes `elements`, leaving page 0 for
/// [`StagedSession::commit`] — the two-phase form of [`write_element_session`].
pub fn stage_element_session(
    path: impl AsRef<Path>,
    request: &SessionRequest<'_>,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
) -> Result<StagedSession, WriteError> {
    stage_session_with_base(path, request, elements, name_edits, None, None)
}

/// [`stage_element_session`] with [`WriteOptions`].
pub fn stage_element_session_with(
    path: impl AsRef<Path>,
    request: &SessionRequest<'_>,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
    options: WriteOptions<'_>,
) -> Result<StagedSession, WriteError> {
    stage_session_with_base(path, request, elements, name_edits, None, options.templates)
}

/// The side-table work of one staging: the element list as it will be written
/// (a new element given its `TYPEX`), the per-table keyed edits the writes
/// imply ([`reverse`]), and the claim-tree edits ([`claim`]) for the primaries
/// they touch.
struct SideTableWork {
    elements: Vec<ElementWrite>,
    keyed: reverse::TableEdits,
    claims: Vec<claim_cow::ClaimEdit>,
}

/// How deep the claim owner-climb may go before giving up — more than any
/// dabacon hierarchy is deep, so it bounds a cyclic chain without cutting a
/// real one short.
const CLAIM_MAX_CLIMB: usize = 256;

/// Plan the keyed-table and claim-tree edits of `elements` against the file's
/// newest session, reading the records an update or delete replaces. Every
/// record's keys are derived by [`reverse::table_keys`] under its noun's
/// template; the claim edits by [`claim::plan_claim_edits`], which climbs the
/// owner chain (through the batch and then the file) to each touched element's
/// primary. A noun the templates do not know contributes nothing on either
/// side.
fn plan_side_tables(
    path: &Path,
    elements: &[ElementWrite],
    templates: &mut TemplateProvider,
) -> Result<SideTableWork, WriteError> {
    // The session this write becomes, for the claim sessions filed below.
    let new_sesno = crate::session::DbView::open(path)?.session().sesno + 1;
    // One read-only view of the newest session, for the records an update or
    // delete replaces and for the owner-climb the claim plan needs.
    let mut engine = crate::ReadOnlyEngine::open(path)?;
    let mut old: HashMap<RefNo, RecordDraft> = HashMap::new();
    for element in elements {
        let refno = match element {
            ElementWrite::Update(draft) => draft.refno,
            ElementWrite::Delete(refno) => *refno,
            ElementWrite::Insert(_) => continue,
        };
        if let Some(view) = engine.find_element(refno)? {
            old.insert(refno, RecordDraft::from_record_bytes(&view.raw_bytes)?);
        }
    }

    fn keys_of(
        templates: &mut TemplateProvider,
        draft: &RecordDraft,
    ) -> Result<std::collections::BTreeSet<reverse::TableKey>, WriteError> {
        Ok(match templates.template_for(draft.noun_hash)? {
            Some(template) => reverse::table_keys(draft, template),
            None => std::collections::BTreeSet::new(),
        })
    }
    let none = std::collections::BTreeSet::new();
    let mut keyed = reverse::TableEdits::default();
    let mut prepared = Vec::with_capacity(elements.len());
    let mut touched: Vec<claim::Touched> = Vec::with_capacity(elements.len());
    for element in elements {
        match element {
            ElementWrite::Insert(draft) => {
                let mut draft = draft.clone();
                if let Some(template) = templates.template_for(draft.noun_hash)? {
                    reverse::ensure_typex(&mut draft, template);
                }
                let after = keys_of(templates, &draft)?;
                keyed.transition(&none, &after);
                touched.push(claim::Touched {
                    refno: draft.refno,
                    noun_hash: draft.noun_hash,
                    owner: draft.owner,
                    kind: claim::TouchKind::Insert,
                });
                prepared.push(ElementWrite::Insert(draft));
            }
            ElementWrite::Update(draft) => {
                let before = match old.get(&draft.refno) {
                    Some(previous) => keys_of(templates, previous)?,
                    None => none.clone(),
                };
                let after = keys_of(templates, draft)?;
                keyed.transition(&before, &after);
                touched.push(claim::Touched {
                    refno: draft.refno,
                    noun_hash: draft.noun_hash,
                    owner: draft.owner,
                    kind: claim::TouchKind::Update,
                });
                prepared.push(element.clone());
            }
            ElementWrite::Delete(refno) => {
                if let Some(previous) = old.get(refno) {
                    let before = keys_of(templates, previous)?;
                    keyed.transition(&before, &none);
                    touched.push(claim::Touched {
                        refno: *refno,
                        noun_hash: previous.noun_hash,
                        owner: previous.owner,
                        kind: claim::TouchKind::Delete,
                    });
                }
                prepared.push(element.clone());
            }
        }
    }

    // Claim edits: climb each touched element to its primary. `is_primary`
    // reads the noun's template word 4; the ancestor lookup reads the newest
    // session for an owner the batch does not carry. A read error there ends a
    // climb rather than failing the write — the claim tree is advisory, and a
    // genuinely unreadable file fails the record and index work instead.
    let mut climb_error: Option<WriteError> = None;
    let claims = {
        let is_primary = |noun: u32| -> bool {
            templates
                .template_for(noun)
                .map(|template| template.is_some_and(|t| t.is_primary))
                .unwrap_or(false)
        };
        let ancestor = |refno: RefNo| -> Option<(u32, RefNo)> {
            match engine.find_element(refno) {
                Ok(Some(view)) => {
                    let header = view.header();
                    Some((header.noun_hash, header.owner))
                }
                Ok(None) => None,
                Err(error) => {
                    climb_error.get_or_insert_with(|| error.into());
                    None
                }
            }
        };
        claim::plan_claim_edits(&touched, new_sesno, is_primary, ancestor, CLAIM_MAX_CLIMB)
    };
    if let Some(error) = climb_error {
        return Err(error);
    }

    Ok(SideTableWork {
        elements: prepared,
        keyed,
        claims,
    })
}

/// Words of a tail block's header (`[kind|words][owner.0][owner.1][cont.0][cont.1]`).
const BLOCK_HEADER_WORDS: usize = crate::record::block::HEADER_BYTES / 4;

/// A fresh element-data page: `(page_no, bytes, used)` with word 0 the page
/// type and `used` at 1, past it.
fn new_data_page(next: &mut u32, page_words: usize) -> (u32, Vec<u8>, usize) {
    let page_no = *next;
    *next += 1;
    let mut bytes = vec![0u8; page_words * 4];
    bytes[0..4].copy_from_slice(&PageType::ElementData.as_raw().to_be_bytes());
    (page_no, bytes, 1)
}

fn put_word(bytes: &mut [u8], word: usize, value: u32) {
    bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_be_bytes());
}

/// Place one record's bytes onto the bump-packed data pages, splitting its
/// member and explicit blocks across pages when they do not fit
/// (`docs/plans/2026-09-20-dabacon-write-plan.md` §2 阶段 2b; the reader's
/// `record::block` continuation, `record::read_record`).
///
/// The record's pending hunks ([`RecordDraft::pending_hunks`]) go down first,
/// each a kind-3 chain of the same segment shape under a seven-word header,
/// and their directory entries in the explicit stream are pointed at them.
/// The eleven-word header and the implicit region then go down contiguously —
/// the index leaf points at the header, and the reader reads the implicit
/// region off the one page, so it cannot span (a record whose implicit region
/// alone overruns a page is [`WriteError::RecordTooLong`]). Each tail block is
/// then laid down as one or more segments: a segment fills to the page boundary
/// and, when payload is left, states in its header where the rest continues,
/// exactly the `(page, packed)` link `record::gather` follows. The header's two
/// slots are back-patched to where each block's first segment landed, so the
/// record reads back whole however many pages it took.
#[allow(clippy::too_many_arguments)]
fn place_record(
    draft: &RecordDraft,
    kind: EditKind,
    page_words: usize,
    ext: u16,
    current: &mut Option<(u32, Vec<u8>, usize)>,
    appended: &mut Vec<(u32, Vec<u8>)>,
    next: &mut u32,
    edits: &mut Vec<LeafEdit>,
) -> Result<(), WriteError> {
    let implicit = draft.implicit();
    let header_words = implicit.len();
    // The header and implicit region are one contiguous run and cannot span.
    if header_words > page_words - 1 {
        return Err(WriteError::RecordTooLong { words: header_words, page_words });
    }

    let member_payload: Vec<u32> = draft
        .members()
        .iter()
        .flat_map(|member| [member.word0, member.word1])
        .collect();

    // The open bump page, or a fresh one.
    let mut working = current.take().unwrap_or_else(|| new_data_page(next, page_words));
    let mut done: Vec<(u32, Vec<u8>)> = Vec::new();

    // Hunks first. `4SCEA` writes an out-of-record value into the current
    // output page the moment it is set (`sub_5AEC110`, a3 == 3), before the
    // record itself is laid down at save, so a hunk precedes its record. Each
    // is a kind-3 chain whose seven-word header names the element, the
    // attribute and the hunk type; its directory entry in the explicit stream
    // then gets the address of its first segment (`crate::record::hunk`).
    let mut hunk_addresses: Vec<(u32, u32, u32)> = Vec::with_capacity(draft.pending_hunks().len());
    for pending in draft.pending_hunks() {
        let address = place_block(
            u32::from(hunk::HUNK_KIND),
            &[pending.hash, u32::from(pending.hunk_type)],
            hunk::HUNK_MIN_WORDS - hunk::HUNK_HEADER_WORDS,
            &pending.payload,
            draft.refno,
            page_words,
            ext,
            &mut working,
            &mut done,
            next,
        );
        hunk_addresses.push((pending.hash, address.0, address.1));
    }
    let explicit_payload: Vec<u32> = draft.explicit_stream_words_with_hunks(&hunk_addresses);

    // A block whose count saturates the format word is read by core.dll along
    // its chain, minus the zero words at the end of its last segment; refuse
    // the one shape that reading would truncate rather than write it.
    for (block, payload) in [("member", &member_payload), ("explicit", &explicit_payload)] {
        if payload.len() >= record::FORMAT_COUNT_MAX && payload.last() == Some(&0) {
            return Err(WriteError::SaturatedBlockEndsInZero { refno: draft.refno, block, words: payload.len() });
        }
    }

    // The header page: the working page when the header and implicit region
    // still fit, else set it aside and start fresh.
    if working.2 + header_words > page_words {
        let full = std::mem::replace(&mut working, new_data_page(next, page_words));
        done.push((full.0, full.1));
    }
    let header_page = working.0;
    let header_off = working.2;

    // Header (slots 6..9 back-patched once the blocks land) then the implicit
    // region past the header verbatim.
    let member_words = draft.members().len() * 2;
    let explicit_words = explicit_payload.len();
    let format = record::wide_format_word(member_words, explicit_words);
    for (index, word) in implicit.iter().enumerate() {
        put_word(&mut working.1, header_off + index, *word);
    }
    put_word(&mut working.1, header_off, header_words as u32);
    put_word(&mut working.1, header_off + 1, draft.refno.word0);
    put_word(&mut working.1, header_off + 2, draft.refno.word1);
    put_word(&mut working.1, header_off + 3, draft.noun_hash);
    put_word(&mut working.1, header_off + 4, draft.owner.word0);
    put_word(&mut working.1, header_off + 5, draft.owner.word1);
    for slot in 6..10 {
        put_word(&mut working.1, header_off + slot, 0);
    }
    put_word(&mut working.1, header_off + 10, format);
    working.2 += header_words;

    edits.push(LeafEdit {
        refno: draft.refno,
        page_no: header_page,
        packed: record::pack_offset(header_off as u32, ext),
        kind,
    });

    // The two tail blocks, members then explicit — the order `encode` uses.
    let member_addr = place_block(2, &[], 1, &member_payload, draft.refno, page_words, ext, &mut working, &mut done, next);
    let explicit_addr = place_block(1, &[], 1, &explicit_payload, draft.refno, page_words, ext, &mut working, &mut done, next);

    // Back-patch the header slots into whichever page still holds the header.
    let header_buf = if working.0 == header_page {
        &mut working.1
    } else {
        &mut done
            .iter_mut()
            .find(|(page_no, _)| *page_no == header_page)
            .expect("the header page is either the working page or one already set aside")
            .1
    };
    put_word(header_buf, header_off + 6, explicit_addr.0);
    put_word(header_buf, header_off + 7, explicit_addr.1);
    put_word(header_buf, header_off + 8, member_addr.0);
    put_word(header_buf, header_off + 9, member_addr.1);

    for page in done {
        appended.push(page);
    }
    *current = Some(working);
    Ok(())
}

/// Lay one block's payload down as segments, returning `(page_no, packed)` of
/// its first segment — the address the record header's slot (or a hunk's
/// directory entry) names, or `(0, 0)` for an empty block. A segment that
/// cannot hold the rest of the payload states the `(page, packed)` where it
/// continues; the continuation opens a fresh page at word 1.
///
/// The header is the five tail-block words — `[kind|words][owner.0][owner.1]
/// [cont.0][cont.1]` — followed by `header_extra`: nothing for a member or
/// explicit block, `[hash][hunk type]` for a hunk (kind 3). `min_payload` is
/// the fewest payload words a segment may open with; a page with less room
/// than header plus that is set aside first (`sub_5AE9DF0`'s minimum: two
/// words for a hunk, `a6 = 9`).
#[allow(clippy::too_many_arguments)]
fn place_block(
    kind_word: u32,
    header_extra: &[u32],
    min_payload: usize,
    payload: &[u32],
    owner: RefNo,
    page_words: usize,
    ext: u16,
    working: &mut (u32, Vec<u8>, usize),
    done: &mut Vec<(u32, Vec<u8>)>,
    next: &mut u32,
) -> (u32, u32) {
    if payload.is_empty() {
        return (0, 0);
    }
    let header_words = BLOCK_HEADER_WORDS + header_extra.len();
    let mut remaining = payload;
    let mut first_addr = None;
    loop {
        // A segment needs its header and at least `min_payload` words; a page
        // with less left is flushed and the segment starts on a fresh one.
        if page_words - working.2 < header_words + min_payload {
            let full = std::mem::replace(working, new_data_page(next, page_words));
            done.push((full.0, full.1));
        }
        let seg_off = working.2;
        if first_addr.is_none() {
            first_addr = Some((working.0, record::pack_offset(seg_off as u32, ext)));
        }
        let capacity = page_words - working.2 - header_words;
        let take = remaining.len().min(capacity);
        let continues = take < remaining.len();
        // The continuation page is drawn now so the segment header can name it.
        let cont = if continues {
            let cont_page = *next;
            (cont_page, record::pack_offset(1, ext))
        } else {
            (0, 0)
        };
        put_word(&mut working.1, seg_off, (kind_word << 16) | (header_words + take) as u32);
        put_word(&mut working.1, seg_off + 1, owner.word0);
        put_word(&mut working.1, seg_off + 2, owner.word1);
        put_word(&mut working.1, seg_off + 3, cont.0);
        put_word(&mut working.1, seg_off + 4, cont.1);
        for (index, word) in header_extra.iter().enumerate() {
            put_word(&mut working.1, seg_off + BLOCK_HEADER_WORDS + index, *word);
        }
        for (index, word) in remaining[..take].iter().enumerate() {
            put_word(&mut working.1, seg_off + header_words + index, *word);
        }
        working.2 += header_words + take;
        if !continues {
            return first_addr.expect("a non-empty block set its first address");
        }
        // Move to the continuation page — the one the segment just named.
        let full = std::mem::replace(working, new_data_page(next, page_words));
        done.push((full.0, full.1));
        debug_assert_eq!(working.0, cont.0, "the continuation page is the one just drawn");
        remaining = &remaining[take..];
    }
}

/// The staging core, shared by the plain, empty and merge paths. `base_sesno`
/// sets the session page's `BASESN`: the previous session for a plain write,
/// the merge base for [`crate::write::merge::write_merge_session`]. With
/// `templates` the keyed tables are maintained ([`reverse`]); without, they
/// are carried over. Writes every new page and syncs, then hands back a
/// [`StagedSession`] whose page 0 is prepared but not yet written.
pub(crate) fn stage_session_with_base(
    path: impl AsRef<Path>,
    request: &SessionRequest<'_>,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
    base_sesno: Option<u32>,
    templates: Option<&mut TemplateProvider>,
) -> Result<StagedSession, WriteError> {
    // The keyed-table and claim plan reads the newest session's records
    // through the read side, so it is made before the write handle is opened.
    let side = match templates {
        Some(templates) => Some(plan_side_tables(path.as_ref(), elements, templates)?),
        None => None,
    };
    let elements: &[ElementWrite] = side.as_ref().map_or(elements, |work| work.elements.as_slice());

    let mut file = PageFile::open(path)?;
    // Enter write mode: take the sentinel lock before appending anything, so a
    // file a real E3D (or another instance of this crate) is writing is refused
    // at once and untouched. Held until this handle drops at commit or rollback.
    file.lock_for_write()?;
    let (mut page0, descriptor, previous_bytes, previous) = read_base(&mut file)?;
    let text = TextFormat::from_header(descriptor.header_format);
    let page_words = file.page_size().words() as usize;
    let ext = PageId::PRIMARY;

    // Pages are drawn in order from just past the previous session's last one:
    // data pages first, then the index pages the rewrite makes, then the
    // session page last (A5/A6).
    let mut next = previous.end.page + 1;
    let mut appended: Vec<(u32, Vec<u8>)> = Vec::new();

    // --- data pages: bump-pack every written record, and collect the index
    // edits (a delete writes no record, only removes a key) ---
    let mut edits: Vec<LeafEdit> = Vec::new();
    {
        let mut current: Option<(u32, Vec<u8>, usize)> = None;
        for element in elements {
            match element {
                ElementWrite::Insert(draft) => {
                    place_record(draft, EditKind::Insert, page_words, ext, &mut current, &mut appended, &mut next, &mut edits)?
                }
                ElementWrite::Update(draft) => {
                    place_record(draft, EditKind::Update, page_words, ext, &mut current, &mut appended, &mut next, &mut edits)?
                }
                ElementWrite::Delete(refno) => edits.push(LeafEdit {
                    refno: *refno,
                    page_no: 0,
                    packed: 0,
                    kind: EditKind::Delete,
                }),
            }
        }
        if let Some((page_no, bytes, _)) = current.take() {
            appended.push((page_no, bytes));
        }
    }

    let page_limit = file.physical_page_count()?.max(1);
    // A names-only session leaves the refno index alone: no element moved, so
    // its root and every one of its pages stay.
    let index_root = if edits.is_empty() {
        previous.index_root
    } else {
        let rewrite = {
            let mut read_page = |page_no: u32| {
                file.read_page(page_no)
                    .map_err(|error| error.to_string())
            };
            let mut alloc = || {
                let page = next;
                next += 1;
                page
            };
            index_cow::rewrite_index(
                previous.index_root,
                page_words,
                &edits,
                &mut read_page,
                &mut alloc,
                page_limit,
            )?
        };
        for page in rewrite.pages {
            appended.push(page);
        }
        rewrite.root
    };

    // --- the NAME table: a second copy-on-write pass when a name was written ---
    let mut table_overrides: Vec<(u32, PageId)> = Vec::new();
    if !name_edits.is_empty() {
        let name_hash = crate::index::named::name_table_hash();
        let root = previous
            .table_root(name_hash)
            .ok_or(WriteError::NoNameTable { name_hash })?;
        let named = {
            let mut read_page = |page_no: u32| file.read_page(page_no).map_err(|error| error.to_string());
            let mut alloc = || {
                let page = next;
                next += 1;
                page
            };
            named_cow::rewrite_named_table(root, page_words, name_edits, &mut read_page, &mut alloc, page_limit)?
        };
        for page in named.pages {
            appended.push(page);
        }
        table_overrides.push((name_hash, named.root));
    }

    // --- the keyed tables: TYPEX and one reverse-reference table per type-16
    // attribute touched, each rewritten copy-on-write; a table the directory
    // has none of is built for its first key and listed behind the others ---
    let mut table_additions: Vec<(u32, PageId)> = Vec::new();
    if let Some(work) = &side {
        for (table, edits) in work.keyed.iter() {
            let root = previous.table_root(table);
            // Withdrawing keys from a table that does not exist is nothing to
            // do (core's 523 is tolerated on a delete); only an insert builds one.
            if root.is_none() && edits.iter().all(|edit| edit.kind == KeyedEditKind::Delete) {
                continue;
            }
            let rewrite = {
                let mut read_page = |page_no: u32| file.read_page(page_no).map_err(|error| error.to_string());
                let mut alloc = || {
                    let page = next;
                    next += 1;
                    page
                };
                keyed_cow::rewrite_keyed_table(root, table, page_words, edits, &mut read_page, &mut alloc, page_limit)?
            };
            for page in rewrite.pages {
                appended.push(page);
            }
            match root {
                Some(_) => table_overrides.push((table, rewrite.root)),
                None => table_additions.push((table, rewrite.root)),
            }
        }
    }

    // --- the claim tree: the primary-element session index, one CoW pass over
    // the previous session's tree; carried over when nothing primary changed ---
    let claim_root = match &side {
        Some(work) if !work.claims.is_empty() => {
            let rewrite = {
                let mut read_page = |page_no: u32| file.read_page(page_no).map_err(|error| error.to_string());
                let mut alloc = || {
                    let page = next;
                    next += 1;
                    page
                };
                claim_cow::rewrite_claim_table(previous.claim, page_words, &work.claims, &mut read_page, &mut alloc, page_limit)?
            };
            for page in rewrite.pages {
                appended.push(page);
            }
            rewrite.root
        }
        _ => previous.claim,
    };

    // --- the session page, last, naming the new index root ---
    let new_sesno = previous.sesno + 1;
    let session_page = PageId::new(ext, next);
    let clock = request
        .clock
        .unwrap_or_else(|| SessionClock::now(previous.zone_offset_seconds().unwrap_or(0)));
    let page = build_session_page(
        &previous_bytes,
        previous.page,
        &NewSession {
            page: session_page,
            sesno: new_sesno,
            end: session_page,
            index_root,
            claim_root,
            table_overrides: &table_overrides,
            table_additions: &table_additions,
            base_sesno: base_sesno.unwrap_or(previous.sesno),
            user: request.user,
            comment: request.comment,
            clock,
            text,
        },
    )?;
    appended.push((session_page.page, page));

    // --- write every new page and sync; page 0 is left for the commit ---
    // The file's length before the append, so a rollback can trim back to it.
    let old_len_bytes = u64::from(file.physical_page_count()?) * file.page_size().bytes() as u64;
    for (page_no, bytes) in &appended {
        file.write_page(*page_no, bytes)?;
    }
    file.sync()?;

    // Prepare page 0 (LSPGNO → the new session, TMPLVN negated when positive,
    // A6.4) but do not write it: that is the commit point.
    pages::put_be_word(&mut page0, page0::LATEST_SES_PGNO, session_page.page);
    pages::put_be_word(&mut page0, page0::EXT_NO, u32::from(session_page.ext));
    let template_version = pages::be_word(&page0, page0::TEMPLATE_VERSION) as i32;
    if template_version > 0 {
        pages::put_be_word(
            &mut page0,
            page0::TEMPLATE_VERSION,
            template_version.wrapping_neg() as u32,
        );
    }

    let mut appended_pages: Vec<u32> = appended.iter().map(|(page_no, _)| *page_no).collect();
    appended_pages.sort_unstable();
    Ok(StagedSession {
        file,
        previous: SessionSummary {
            sesno: previous.sesno,
            page: previous.page,
            end: previous.end,
        },
        written: SessionSummary {
            sesno: new_sesno,
            page: session_page,
            end: session_page,
        },
        page0,
        old_len_bytes,
        appended_pages,
    })
}
