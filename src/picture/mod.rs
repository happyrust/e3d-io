//! DRAFT picture files: the Wigwam (GW / GTIGS) pictures that hold each
//! sheet's view graphics.
//!
//! A DRAFT sheet's view graphics do not live in the PADD database. They are
//! written by core.dll's Wigwam kernel (`GSVPI0` → `sub_54DC674` op 6) to
//! `<proj>pic\NN\M<w0>-<w1>-<EXFI>-<PVNO>`, where `w0/w1` is the sheet's
//! refno, `EXFI` its extract database and `PVNO` its picture version. This
//! module reads that file back the way op 9 does: 128-byte records of 8-byte
//! words, a 21-word header locating the name block, attribute table and
//! picture table, and per picture a directory that replays into a tree of
//! segments carrying groups of primitives.
//!
//! The format is documented in `docs/evidence/2026-09-28-gw-picture-format.md`
//! and was proven by re-encoding every decoded item of 2 766 sample files
//! (Plant 881 + Marine 1 885, 24 873 647 primitives) back to identical bytes.
//! The reference implementation is `D:\ida_scratch\draft31\gwdump.py`; this
//! module is its port, staged per `docs/plans/2026-09-24-gw-picture-read-plan.md`.
//!
//! This is a different file type from the Dabacon databases the rest of the
//! crate reads, so it has its own error type and shares only [`crate::RefNo`]
//! and the noun hash.

pub mod directory;
pub mod file;
pub mod geom;
pub mod listing;
pub mod prim;
pub mod segment;
pub mod style;
pub mod text;
pub mod word;

pub use directory::{DirEntry, Node, Picture};
pub use file::{Header, PictureFile, PictureName, HEADER_WORDS, RECORD_BYTES};
pub use geom::Transform;
pub use prim::{kind, Payload, Primitive, Span, Text2Body, Text2Prim, TextPrim};
pub use segment::{flags, Group, Member, Role, Segment, TexpNote, UserBlock};
pub use style::{pen_name, Pen};
pub use text::{Run, TextFormat};
pub use word::{Word, Words};

use std::path::PathBuf;

/// What can go wrong reading a picture file.
#[derive(Debug, thiserror::Error)]
pub enum PictureError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The first record is not a GW header (`"GW"` + version + `" PC 06"`).
    #[error("{}: not a GW picture file ({found})", path.display())]
    NotAPicture { path: PathBuf, found: String },

    /// A record number the header or a directory names is not in the file.
    #[error("record {rec} is not in the file, which has {records} records")]
    BadRecord { rec: i32, records: usize },

    /// A word was needed past the end of the data (record-relative).
    #[error("record {rec} word {word} lies past the end of the file")]
    ShortFile { rec: u32, word: usize },

    /// A directory entry the tree replay cannot apply.
    #[error("picture {picture} directory entry {entry}: {reason}")]
    BadDirectory {
        picture: i32,
        entry: usize,
        reason: String,
    },

    /// A segment header or group that cannot be walked.
    #[error("segment at record {rec}, word {word}: {reason}")]
    BadSegment {
        rec: u32,
        word: usize,
        reason: String,
    },

    /// A primitive header that cannot be walked.
    #[error("primitive at record {rec}, word {word}: {reason}")]
    BadPrimitive {
        rec: u32,
        word: usize,
        reason: String,
    },
}
