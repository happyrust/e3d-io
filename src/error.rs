use std::path::{Path, PathBuf};

use crate::page::PageId;

/// Failures raised by the L0 page layer.
///
/// This layer owns three units of confusion that have each cost the project a
/// production incident, so each one gets its own variant rather than being
/// folded into a string:
///
/// * the page size field counts 4-byte words, not bytes;
/// * a page address is `(extent, page)`, and the extent selects the file;
/// * a page that cannot be read is an error, never a default.
#[derive(Debug, thiserror::Error)]
pub enum PageIoError {
    #[error(
        "{}: header word at 0x34 says {words} four-byte words, which is not a usable page size ({detail})",
        show_path(.path)
    )]
    InvalidPageSize {
        path: Option<PathBuf>,
        words: u32,
        detail: String,
    },

    #[error(
        "extent {ext} is not open, so page {page} cannot be read; expected it at {}",
        show_path(.expected_path)
    )]
    MissingExtent {
        ext: u16,
        page: u32,
        expected_path: Option<PathBuf>,
    },

    #[error(
        "extent {ext} at {} declares {words} four-byte words per page but extent {primary_ext} declares {primary_words}",
        .path.display()
    )]
    ExtentPageSizeMismatch {
        ext: u16,
        path: PathBuf,
        words: u32,
        primary_ext: u16,
        primary_words: u32,
    },

    #[error("buffer holds {got} bytes but a page is {need} bytes")]
    BufferTooSmall { need: usize, got: usize },

    #[error("{}: database file is {actual} bytes, below the {expected} byte minimum", .path.display())]
    FileTooSmall {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },

    #[error("{}: reading {} failed: {source}", .path.display(), show_page(.id))]
    Io {
        path: PathBuf,
        id: Option<PageId>,
        #[source]
        source: std::io::Error,
    },
}

impl PageIoError {
    /// Attach the file a context-free failure came from.
    ///
    /// [`crate::page::PageSize::from_header_words`] validates a bare number and
    /// has no path to report; the opener calls this to name the file.
    pub fn with_path(self, path: impl Into<PathBuf>) -> Self {
        match self {
            Self::InvalidPageSize {
                path: None,
                words,
                detail,
            } => Self::InvalidPageSize {
                path: Some(path.into()),
                words,
                detail,
            },
            other => other,
        }
    }

    pub fn io(path: impl Into<PathBuf>, id: Option<PageId>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            id,
            source,
        }
    }

    /// A read that stopped before the page was full.
    ///
    /// An `UnexpectedEof` `io::Error` rather than a variant of its own, which
    /// is what `read_exact` produced before positioned reads replaced it.
    pub fn short_read(path: &Path, id: PageId, expected: usize, got: usize) -> Self {
        Self::io(
            path,
            Some(id),
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("{id} ended after {got} of {expected} bytes"),
            ),
        )
    }
}

/// Failures raised by the L1 session chain.
///
/// Every variant here was `None` in the reader this replaces — the same `None`
/// that one level up means "no such element". Missing, malformed and
/// unreadable are three different facts and each says so.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(
        "{}: no session {requested}; newest is {newest}{}",
        .path.display(),
        show_oldest(.oldest)
    )]
    NoSuchSession {
        path: PathBuf,
        requested: u32,
        newest: u32,
        /// Oldest session actually reached, or `None` when the request was
        /// already newer than the newest and the chain was never walked.
        oldest: Option<u32>,
    },

    #[error("{page} is page type {page_type}, not a session page (type 3)")]
    NotASessionPage { page: PageId, page_type: u32 },

    #[error("{page} holds {len} bytes, too few for a session page ({need} needed)")]
    PageTooSmall {
        page: PageId,
        len: usize,
        need: usize,
    },

    #[error("{page} could not be read: {detail}")]
    Unreadable { page: PageId, detail: String },

    #[error("the session chain revisits {revisited} after {after} sessions")]
    ChainCycle { revisited: PageId, after: u32 },

    #[error("the session chain is still going after {steps} steps, past the {limit} pages the file holds")]
    ChainTooLong { steps: u32, limit: u32 },
}

fn show_oldest(oldest: &Option<u32>) -> String {
    match oldest {
        Some(sesno) => format!(" and the chain reaches back to {sesno}"),
        None => String::new(),
    }
}

fn show_path(path: &Option<PathBuf>) -> String {
    match path {
        Some(path) => path.display().to_string(),
        None => "<unknown file>".to_string(),
    }
}

fn show_page(id: &Option<PageId>) -> String {
    match id {
        Some(id) => id.to_string(),
        None => "the file header".to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum E3dReaderError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error(transparent)]
    Page(PageIoError),

    #[error(transparent)]
    Session(#[from] SessionError),

    #[error("invalid page header at page {page_no}: {detail}")]
    InvalidPage { page_no: u32, detail: String },

    #[error("unsupported page type {0}")]
    UnsupportedPageType(u32),

    #[error("invalid file signature: expected valid E3D database")]
    InvalidSignature,

    #[error("element not found: RefNo({0}/{1})")]
    ElementNotFound(u32, i32),

    #[error("index search failed at page {page_no}: {detail}")]
    IndexError { page_no: u32, detail: String },

    #[error(
        "the index says {refno} is at page {page_no} byte {byte_offset}, and the record there is \
         {found}"
    )]
    IndexRecordMismatch {
        refno: crate::refno::RefNo,
        page_no: u32,
        byte_offset: u32,
        found: crate::refno::RefNo,
    },

    #[error("page {page_no} out of range (total {total_pages})")]
    PageOutOfRange { page_no: u32, total_pages: u32 },

    #[error("database file too small: expected at least {expected} bytes, got {actual}")]
    FileTooSmall { expected: u64, actual: u64 },

    /// Boxed because it is the largest variant by some way and every other
    /// error would otherwise be carried at its size.
    #[error(transparent)]
    Record(Box<crate::record::RecordError>),

    #[error("{0}")]
    Other(String),
}

impl E3dReaderError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Widen a page-layer failure into the crate error.
///
/// `Io` and `FileTooSmall` are re-expressed as the long-standing
/// [`E3dReaderError`] variants instead of being wrapped, because callers match
/// on them.
impl From<PageIoError> for E3dReaderError {
    fn from(err: PageIoError) -> Self {
        match err {
            PageIoError::Io { path, source, .. } => Self::Io { path, source },
            PageIoError::FileTooSmall {
                expected, actual, ..
            } => Self::FileTooSmall { expected, actual },
            other => Self::Page(other),
        }
    }
}
