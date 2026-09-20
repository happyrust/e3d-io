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
//! the new page and returns a [`Staged`] whose [`Staged::commit`] writes page
//! 0; the two steps are separate so that a test can stop between them and read
//! the file as a crash would have left it.
//!
//! Not done here: the write-mode sentinel lock `core.dll` takes
//! (`LockFileEx` on one byte at `0x7FFFFFF6FFFFFFFF`, A9). In its place
//! [`Staged::commit`] re-reads page 0 and refuses when `LSPGNO` moved since
//! staging, which catches a concurrent writer that finished first but not one
//! that is mid-flight. Take the file out of E3D's write mode before writing.

pub mod index_cow;
pub mod merge;
pub mod named_cow;
pub mod pages;
pub mod record;
pub mod session_page;

use std::path::{Path, PathBuf};

use crate::error::SessionError;
use crate::meta::descriptor::at as page0;
use crate::meta::DbDescriptor;
use crate::page::{PageId, PageSize, PageType};
use crate::record::text::TextFormat;
use crate::refno::RefNo;
use crate::session::SessionPage;

pub use index_cow::{EditKind, IndexCowError, LeafEdit};
pub use merge::{write_merge_session, MergeConflict};
pub use named_cow::{NamedCowError, NamedEdit, NamedEditKind};
pub use pages::PageFile;
pub use record::{PlacedRecord, RecordDraft, RecordEncodeError};
pub use session_page::{build_session_page, NewSession, SessionClock, E3DIO_VERSION_TAG};

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
    #[error("{field}: stage 1 writes ASCII only, got {text:?}")]
    TextNotAscii { field: &'static str, text: String },
    #[error("{field}: {len} bytes, the field holds at most {max}")]
    TextTooLong {
        field: &'static str,
        len: usize,
        max: usize,
    },
    #[error("{path}: extent {ext} — only single-extent files (extent 1) are written")]
    MultiExtent { path: PathBuf, ext: u32 },
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
    #[error("{0}")]
    Record(#[from] RecordEncodeError),
    #[error("{0}")]
    IndexCow(#[from] IndexCowError),
    #[error("{0}")]
    NameTable(#[from] NamedCowError),
    #[error(
        "a record of {words} words does not fit on a {page_words}-word page \
         (stage 2 does not split a record across pages)"
    )]
    RecordTooLong { words: usize, page_words: usize },
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

/// The new pages are on disk; page 0 still names the old session.
///
/// Dropping this without [`Staged::commit`] leaves the file exactly as
/// readable as before: the appended page is past the newest session's `end`
/// and no pointer reaches it. [`Staged::rollback`] trims it off again.
#[derive(Debug)]
pub struct Staged {
    file: PageFile,
    old: SessionSummary,
    new_page: PageId,
    new_sesno: u32,
    page0: Vec<u8>,
    old_len_bytes: u64,
}

/// The session a write is based on, as page 0 and its session page state it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSummary {
    pub sesno: u32,
    pub page: PageId,
    pub end: PageId,
}

/// What [`Staged::commit`] leaves behind.
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

/// Append a session that changes nothing, leaving page 0 for [`Staged::commit`].
///
/// The new page is `newest.end + 1` — the next page after the last one the
/// newest session wrote, whatever the file's physical length says (a file
/// may carry pages past `end`; `core.dll` defines the file size by `LASTSP`,
/// not by the OS length, §4.2 of the write-path note).
pub fn stage_empty_session(
    path: impl AsRef<Path>,
    request: &SessionRequest<'_>,
) -> Result<Staged, WriteError> {
    let mut file = PageFile::open(path)?;
    let (mut page0, descriptor, previous_bytes, previous) = read_base(&mut file)?;
    let text = TextFormat::from_header(descriptor.header_format);

    let new_page = PageId::new(PageId::PRIMARY, previous.end.page + 1);
    let new_sesno = previous.sesno + 1;
    let clock = request
        .clock
        .unwrap_or_else(|| SessionClock::now(previous.zone_offset_seconds().unwrap_or(0)));
    let page = build_session_page(
        &previous_bytes,
        previous.page,
        &NewSession {
            page: new_page,
            sesno: new_sesno,
            end: new_page,
            index_root: previous.index_root,
            table_overrides: &[],
            base_sesno: previous.sesno,
            user: request.user,
            comment: request.comment,
            clock,
            text,
        },
    )?;

    // Page 0 after commit: LSPGNO → the new page; TMPLVN negated when it is
    // still positive (A6.4: "第 9 字 TMPLVN >0 则取负"); nothing else.
    pages::put_be_word(&mut page0, page0::LATEST_SES_PGNO, new_page.page);
    pages::put_be_word(&mut page0, page0::EXT_NO, u32::from(new_page.ext));
    let template_version = pages::be_word(&page0, page0::TEMPLATE_VERSION) as i32;
    if template_version > 0 {
        pages::put_be_word(
            &mut page0,
            page0::TEMPLATE_VERSION,
            template_version.wrapping_neg() as u32,
        );
    }

    let old_len_bytes = u64::from(file.physical_page_count()?) * file.page_size().bytes() as u64;
    file.write_page(new_page.page, &page)?;
    file.sync()?;

    Ok(Staged {
        file,
        old: SessionSummary {
            sesno: previous.sesno,
            page: previous.page,
            end: previous.end,
        },
        new_page,
        new_sesno,
        page0,
        old_len_bytes,
    })
}

impl Staged {
    /// The session this write is based on.
    pub fn previous(&self) -> SessionSummary {
        self.old
    }

    /// Where the staged session page lies and what number it carries.
    pub fn staged(&self) -> SessionSummary {
        SessionSummary {
            sesno: self.new_sesno,
            page: self.new_page,
            end: self.new_page,
        }
    }

    pub fn page_size(&self) -> PageSize {
        self.file.page_size()
    }

    /// Write page 0 and sync: the commit point.
    ///
    /// Page 0 is re-read first; if its `LSPGNO` no longer names the session
    /// this write was staged on, another writer got there and nothing is
    /// written ([`WriteError::Moved`]). The staged page stays on disk,
    /// unreachable, exactly as a crashed `core.dll` would leave one.
    pub fn commit(mut self) -> Result<CommitReport, WriteError> {
        let current = self.file.read_page(0)?;
        let found = pages::be_word(&current, page0::LATEST_SES_PGNO);
        if found != self.old.page.page {
            return Err(WriteError::Moved {
                path: self.file.path().to_path_buf(),
                expected: self.old.page.page,
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
            previous: self.old,
            written: self.staged(),
            appended_pages: vec![self.new_page.page],
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

/// [`stage_empty_session`] followed by [`Staged::commit`].
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
/// exactly as [`write_empty_session`] does. `Staged`-style two-phase staging is
/// not exposed for this path yet; the commit still re-reads page 0 and refuses
/// on [`WriteError::Moved`].
pub fn write_element_session(
    path: impl AsRef<Path>,
    request: &SessionRequest<'_>,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
) -> Result<CommitReport, WriteError> {
    write_session_with_base(path, request, elements, name_edits, None)
}

/// The write core, shared by the plain and merge paths. `base_sesno` sets the
/// session page's `BASESN`: the previous session for a plain write, the merge
/// base for [`crate::write::merge::write_merge_session`].
fn write_session_with_base(
    path: impl AsRef<Path>,
    request: &SessionRequest<'_>,
    elements: &[ElementWrite],
    name_edits: &[NamedEdit],
    base_sesno: Option<u32>,
) -> Result<CommitReport, WriteError> {
    let mut file = PageFile::open(path)?;
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
        let place = |draft: &RecordDraft,
                         kind: EditKind,
                         current: &mut Option<(u32, Vec<u8>, usize)>,
                         appended: &mut Vec<(u32, Vec<u8>)>,
                         next: &mut u32,
                         edits: &mut Vec<LeafEdit>|
         -> Result<(), WriteError> {
            let len = draft.encoded_len_words();
            if len > page_words - 1 {
                return Err(WriteError::RecordTooLong { words: len, page_words });
            }
            let need_new = match current {
                Some((_, _, used)) => *used + len > page_words,
                None => true,
            };
            if need_new {
                if let Some((page_no, bytes, _)) = current.take() {
                    appended.push((page_no, bytes));
                }
                let page_no = *next;
                *next += 1;
                let mut bytes = vec![0u8; page_words * 4];
                bytes[0..4].copy_from_slice(&PageType::ElementData.as_raw().to_be_bytes());
                *current = Some((page_no, bytes, 1));
            }
            let (page_no, bytes, used) = current.as_mut().expect("a page is open");
            let placed = draft.encode(*page_no, *used as u32, ext);
            bytes[*used * 4..*used * 4 + placed.bytes.len()].copy_from_slice(&placed.bytes);
            *used += len;
            edits.push(LeafEdit {
                refno: placed.refno,
                page_no: placed.page_no,
                packed: record::pack_offset(placed.word_offset, ext),
                kind,
            });
            Ok(())
        };

        for element in elements {
            match element {
                ElementWrite::Insert(draft) => {
                    place(draft, EditKind::Insert, &mut current, &mut appended, &mut next, &mut edits)?
                }
                ElementWrite::Update(draft) => {
                    place(draft, EditKind::Update, &mut current, &mut appended, &mut next, &mut edits)?
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
            table_overrides: &table_overrides,
            base_sesno: base_sesno.unwrap_or(previous.sesno),
            user: request.user,
            comment: request.comment,
            clock,
            text,
        },
    )?;
    appended.push((session_page.page, page));

    // --- write every new page, sync, then commit page 0 ---
    for (page_no, bytes) in &appended {
        file.write_page(*page_no, bytes)?;
    }
    file.sync()?;

    let current = file.read_page(0)?;
    let found = pages::be_word(&current, page0::LATEST_SES_PGNO);
    if found != previous.page.page {
        return Err(WriteError::Moved {
            path: file.path().to_path_buf(),
            expected: previous.page.page,
            found,
        });
    }
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
    file.write_page(0, &page0)?;
    file.sync()?;

    let mut appended_pages: Vec<u32> = appended.iter().map(|(page_no, _)| *page_no).collect();
    appended_pages.sort_unstable();
    Ok(CommitReport {
        path: file.path().to_path_buf(),
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
        appended_pages,
    })
}
