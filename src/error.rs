use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum E3dReaderError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

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

    #[error("page {page_no} out of range (total {total_pages})")]
    PageOutOfRange { page_no: u32, total_pages: u32 },

    #[error("database file too small: expected at least {expected} bytes, got {actual}")]
    FileTooSmall { expected: u64, actual: u64 },

    #[error("element record for {refno} at page {page_no}, byte offset {byte_offset} is truncated: {detail}")]
    TruncatedRecord {
        refno: crate::refno::RefNo,
        page_no: u32,
        byte_offset: u32,
        detail: String,
    },

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
