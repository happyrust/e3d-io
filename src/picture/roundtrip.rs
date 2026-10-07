//! The acceptance gate: encode every decoded item back and compare it with
//! the bytes it came from.
//!
//! A reading of the format is only as good as its inverse. `gwdump.py
//! --roundtrip` proved the reference reading on 2 766 files (Plant 881 +
//! Marine 1 885, 24 873 647 primitives) with zero differing bytes; this
//! module runs the same check on the Rust decoding — header list, name
//! block, picture table entries, directories, segment headers, group headers
//! and primitives — and over a whole `pic\` tree
//! (`docs/plans/2026-09-24-gw-picture-read-plan.md` §2.1 D, §4 S3).
//!
//! This is not a picture writer: nothing here lays out records or allocates
//! space. It answers one question per item — do these words re-encode to the
//! bytes on disk?

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::file::{PictureFile, HEADER_WORDS};
use super::prim::kind_name;
use super::word::{to_bytes, Word};
use super::PictureError;

/// One item that did not re-encode to its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// The 21-word header list at record 1.
    Header,
    /// The name block.
    Name,
    /// Picture table entry `index` (0-based) of picture `number`.
    PictureTable { index: usize, number: i32 },
    /// The directory of picture `number`.
    Directory { number: i32 },
    /// The header of the segment at record `rec`.
    SegmentHeader { rec: i32 },
    /// A group header at word `word` of the segment at record `rec`.
    GroupHeader { rec: i32, word: usize },
    /// A primitive of `kind` at word `word` of the segment at record `rec`.
    Primitive { rec: i32, word: usize, kind: u8 },
}

/// What re-encoding one file found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoundtripReport {
    pub path: PathBuf,
    /// The header's 15-character version string.
    pub version: String,
    /// Pictures in the file.
    pub pictures: usize,
    pub header_ok: bool,
    pub name_ok: bool,
    /// Picture table entries that re-encoded exactly.
    pub pictab_ok: usize,
    /// Directories that re-encoded exactly.
    pub dir_ok: usize,
    pub seg_hdr_ok: usize,
    pub seg_hdr_bad: usize,
    pub grp_hdr_ok: usize,
    pub grp_hdr_bad: usize,
    pub prim_ok: usize,
    pub prim_bad: usize,
    /// Primitives seen, by kind.
    pub kinds: BTreeMap<u8, usize>,
    /// The first [`RoundtripReport::MAX_MISMATCHES`] mismatches, in file order.
    pub mismatches: Vec<Mismatch>,
}

impl RoundtripReport {
    /// Mismatches kept per file; the counters go on counting.
    pub const MAX_MISMATCHES: usize = 20;

    /// True when every item re-encoded to its bytes.
    pub fn is_exact(&self) -> bool {
        self.header_ok
            && self.name_ok
            && self.pictab_ok == self.pictures
            && self.dir_ok == self.pictures
            && self.seg_hdr_bad == 0
            && self.grp_hdr_bad == 0
            && self.prim_bad == 0
    }

    fn note(&mut self, mismatch: Mismatch) {
        if self.mismatches.len() < Self::MAX_MISMATCHES {
            self.mismatches.push(mismatch);
        }
    }

    /// `gwdump.py`'s `stats` dict, spelled the way Python prints it.
    pub fn stats_line(&self) -> String {
        format!(
            "{{'seg_hdr_ok': {}, 'seg_hdr_bad': {}, 'grp_hdr_ok': {}, 'grp_hdr_bad': {}, 'prim_ok': {}, 'prim_bad': {}, 'hdr_ok': {}, 'pictab_ok': {}, 'dir_ok': {}, 'name_ok': {}}}",
            self.seg_hdr_ok,
            self.seg_hdr_bad,
            self.grp_hdr_ok,
            self.grp_hdr_bad,
            self.prim_ok,
            self.prim_bad,
            self.header_ok as u8,
            self.pictab_ok,
            self.dir_ok,
            self.name_ok as u8
        )
    }
}

/// Re-encode every item of `file` and compare it with the file's bytes.
pub fn roundtrip(file: &PictureFile) -> Result<RoundtripReport, PictureError> {
    let mut report = RoundtripReport {
        path: file.path().to_path_buf(),
        version: file.header().version.clone(),
        pictures: file.pictures().len(),
        ..RoundtripReport::default()
    };

    // Header list: 15 characters then 6 integers, exactly as op 6 wrote it.
    let head = file.words_at(1)?;
    report.header_ok = to_bytes(&file.header().encode()) == head.raw(1, HEADER_WORDS)?;
    if !report.header_ok {
        report.note(Mismatch::Header);
    }

    // Name block: the characters then blanks to the declared length.
    let name_len = usize::try_from(file.header().name_len).unwrap_or(0);
    report.name_ok = if name_len == 0 {
        true
    } else {
        let words = file.words_at(file.header().name_rec)?;
        to_bytes(&file.name().encode()) == words.raw(1, name_len)?
    };
    if !report.name_ok {
        report.note(Mismatch::Name);
    }

    for (index, picture) in file.pictures().iter().enumerate() {
        // Picture table entry: [8, number, dir_rec, int4, window × 4].
        let table = file.words_at(file.header().pic_rec)?;
        let mut entry = vec![
            Word::int(8),
            Word::int(picture.number),
            Word::int(picture.dir_rec),
            Word::int(picture.int4),
        ];
        entry.extend(picture.window.iter().map(|&v| Word::real(v)));
        if to_bytes(&entry) == table.raw(1 + index * 8, 8)? {
            report.pictab_ok += 1;
        } else {
            report.note(Mismatch::PictureTable {
                index,
                number: picture.number,
            });
        }

        // Directory: its length word then the 7-word entries.
        let directory = file.words_at(picture.dir_rec)?;
        let mut words = vec![Word::int(picture.dir_len)];
        for e in &picture.entries {
            words.extend(e.words.iter().map(|&v| Word::int(v)));
        }
        let dir_len = usize::try_from(picture.dir_len).unwrap_or(0);
        if to_bytes(&words) == directory.raw(1, dir_len)? {
            report.dir_ok += 1;
        } else {
            report.note(Mismatch::Directory {
                number: picture.number,
            });
        }

        // Segments: header words, each group header, each primitive.
        for (entry, segment) in picture.entries.iter().zip(&picture.segments) {
            let Some(segment) = segment else { continue };
            let words = file.words_at(entry.rec())?;
            if to_bytes(&segment.encode_header()) == words.raw(1, segment.header_words)? {
                report.seg_hdr_ok += 1;
            } else {
                report.seg_hdr_bad += 1;
                report.note(Mismatch::SegmentHeader { rec: entry.rec() });
            }
            for group in &segment.groups {
                if let Some(header) = group.encode_header() {
                    if header.to_be() == words.raw(group.pos, 1)? {
                        report.grp_hdr_ok += 1;
                    } else {
                        report.grp_hdr_bad += 1;
                        report.note(Mismatch::GroupHeader {
                            rec: entry.rec(),
                            word: group.pos,
                        });
                    }
                }
                for prim in &group.prims {
                    *report.kinds.entry(prim.kind).or_insert(0) += 1;
                    if to_bytes(&prim.encode()) == words.raw(prim.pos, prim.length as usize)? {
                        report.prim_ok += 1;
                    } else {
                        report.prim_bad += 1;
                        report.note(Mismatch::Primitive {
                            rec: entry.rec(),
                            word: prim.pos,
                            kind: prim.kind,
                        });
                    }
                }
            }
        }
    }
    Ok(report)
}

/// Why a file in a corpus walk did not yield a report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CorpusFailure {
    /// The file decoded but some item did not re-encode.
    Inexact(RoundtripReport),
    /// The file did not decode (a real finding on a GW file).
    Error { path: PathBuf, message: String },
}

impl CorpusFailure {
    /// The file concerned.
    pub fn path(&self) -> &Path {
        match self {
            CorpusFailure::Inexact(report) => &report.path,
            CorpusFailure::Error { path, .. } => path,
        }
    }
}

/// What a walk over a `pic\` tree found (`gwdump.py roundtrip_dir`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CorpusReport {
    pub root: PathBuf,
    /// GW files that opened.
    pub files: usize,
    /// Files that are not GW pictures (no `GW` header) or could not be read.
    pub skipped: usize,
    /// Files per version string.
    pub versions: BTreeMap<String, usize>,
    /// Counters summed over every file (`header_ok` / `name_ok` there are
    /// meaningless; see [`CorpusReport::hdr_ok`] / [`CorpusReport::name_ok`]).
    pub totals: RoundtripReport,
    /// Files whose header list re-encoded exactly.
    pub hdr_ok: usize,
    /// Files whose name block re-encoded exactly.
    pub name_ok: usize,
    /// Primitives per kind, summed.
    pub kinds: BTreeMap<u8, usize>,
    pub failures: Vec<CorpusFailure>,
}

impl CorpusReport {
    /// True when every file opened re-encoded exactly and none failed.
    pub fn is_exact(&self) -> bool {
        self.failures.is_empty()
            && self.hdr_ok == self.files
            && self.name_ok == self.files
            && self.totals.pictab_ok == self.totals.pictures
            && self.totals.dir_ok == self.totals.pictures
    }

    /// `gwdump.py`'s corpus `total` dict: the per-file booleans as file counts.
    pub fn stats_line(&self) -> String {
        let t = &self.totals;
        format!(
            "{{'seg_hdr_ok': {}, 'seg_hdr_bad': {}, 'grp_hdr_ok': {}, 'grp_hdr_bad': {}, 'prim_ok': {}, 'prim_bad': {}, 'hdr_ok': {}, 'pictab_ok': {}, 'dir_ok': {}, 'name_ok': {}}}",
            t.seg_hdr_ok,
            t.seg_hdr_bad,
            t.grp_hdr_ok,
            t.grp_hdr_bad,
            t.prim_ok,
            t.prim_bad,
            self.hdr_ok,
            t.pictab_ok,
            t.dir_ok,
            self.name_ok
        )
    }
    /// `MARKER(2)=7005637, LINE(3)=1586003, …`.
    pub fn kinds_line(&self) -> String {
        let parts: Vec<String> = self
            .kinds
            .iter()
            .map(|(&k, n)| format!("{}({k})={n}", kind_name(k)))
            .collect();
        parts.join(", ")
    }

    /// `'GW6A04    PC 06'=452, 'GW9B2B    PC 06'=429`.
    pub fn versions_line(&self) -> String {
        let parts: Vec<String> = self
            .versions
            .iter()
            .map(|(v, n)| format!("'{v}'={n}"))
            .collect();
        parts.join(", ")
    }

    /// The lines `gwdump.py roundtrip_dir` prints (failures listed after).
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!("corpus     : {}", self.root.display()),
            format!(
                "files      : {} GW pictures, {} skipped (not GW)",
                self.files, self.skipped
            ),
            format!("versions   : {}", self.versions_line()),
            format!("totals     : {}", self.stats_line()),
            format!("prim kinds : {}", self.kinds_line()),
            format!("failures   : {}", self.failures.len()),
        ];
        for failure in self.failures.iter().take(40) {
            match failure {
                CorpusFailure::Inexact(report) => out.push(format!(
                    "   {} {} {:?}",
                    report.path.display(),
                    report.stats_line(),
                    &report.mismatches[..report.mismatches.len().min(5)]
                )),
                CorpusFailure::Error { path, message } => {
                    out.push(format!("   {} EXC {message}", path.display()))
                }
            }
        }
        out
    }
}

/// Walk every file below `root` (directories in name order, files in name
/// order within each), round-trip each GW picture, and sum the results.
/// `progress` is called with each file's report as it completes.
pub fn roundtrip_dir(
    root: impl AsRef<Path>,
    mut progress: impl FnMut(&RoundtripReport),
) -> Result<CorpusReport, PictureError> {
    let root = root.as_ref();
    let mut report = CorpusReport {
        root: root.to_path_buf(),
        ..CorpusReport::default()
    };
    for path in walk_files(root)? {
        let file = match PictureFile::open(&path) {
            Ok(file) => file,
            Err(PictureError::NotAPicture { .. }) | Err(PictureError::Io { .. }) => {
                report.skipped += 1;
                continue;
            }
            Err(error) => {
                report.failures.push(CorpusFailure::Error {
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        let one = match roundtrip(&file) {
            Ok(one) => one,
            Err(error) => {
                report.failures.push(CorpusFailure::Error {
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        report.files += 1;
        *report.versions.entry(one.version.clone()).or_insert(0) += 1;
        let t = &mut report.totals;
        t.pictures += one.pictures;
        t.pictab_ok += one.pictab_ok;
        t.dir_ok += one.dir_ok;
        t.seg_hdr_ok += one.seg_hdr_ok;
        t.seg_hdr_bad += one.seg_hdr_bad;
        t.grp_hdr_ok += one.grp_hdr_ok;
        t.grp_hdr_bad += one.grp_hdr_bad;
        t.prim_ok += one.prim_ok;
        t.prim_bad += one.prim_bad;
        report.hdr_ok += one.header_ok as usize;
        report.name_ok += one.name_ok as usize;
        for (&k, &n) in &one.kinds {
            *report.kinds.entry(k).or_insert(0) += n;
        }
        progress(&one);
        if !one.is_exact() {
            report.failures.push(CorpusFailure::Inexact(one));
        }
    }
    Ok(report)
}

/// Every regular file below `root`, top-down, names sorted within each
/// directory (the order `os.walk` + `sorted` gives `gwdump.py`).
fn walk_files(root: &Path) -> Result<Vec<PathBuf>, PictureError> {
    let io = |source: std::io::Error| PictureError::Io {
        path: root.to_path_buf(),
        source,
    };
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root).map_err(io)? {
        let entry = entry.map_err(io)?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else {
            files.push(path);
        }
    }
    files.sort();
    dirs.sort();
    for dir in dirs {
        files.extend(walk_files(&dir)?);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picture::prim::kind;
    use e3d_attlib::db1_hash;

    /// A two-record file built by hand: header, name, picture table,
    /// directory and one segment holding a group with one LINE.
    fn tiny_file() -> Vec<u8> {
        const WORDS_PER_RECORD: usize = 16;
        let mut words: Vec<Word> = Vec::new();
        // Records 1–2: the 21-word header — name block at record 3 (9 words),
        // no attribute table, picture table at record 4 (8 words).
        words.extend("GW6A04    PC 06".bytes().map(|c| Word::int(c as i32)));
        words.extend([3, 9, 0, 0, 4, 8].map(Word::int));
        words.resize(2 * WORDS_PER_RECORD, Word::int(0));
        // Record 3: the name "9 8 7 0 6".
        words.extend("9 8 7 0 6".bytes().map(|c| Word::int(c as i32)));
        words.resize(3 * WORDS_PER_RECORD, Word::int(0));
        // Record 4: picture table [8, number 1, directory at record 5, 0, window].
        words.extend([8, 1, 5, 0].map(Word::int));
        words.extend([-10.0, -20.0, 10.0, 20.0].map(Word::real));
        words.resize(4 * WORDS_PER_RECORD, Word::int(0));
        // Record 5: directory [8][7, 0, 0, record 6, rel 6, type 3, 12 words].
        words.extend([8, 7, 0, 0, 6, 6, 3, 12].map(Word::int));
        words.resize(5 * WORDS_PER_RECORD, Word::int(0));
        // Record 6: one segment — header (6 words: header word, [5],
        // [SHEE, 9, 8, 0]) then a group (6 words: header, LINE of 5).
        words.push(Word::header(kind::SEGMENT, 0x7, 6, 0));
        words.push(Word::int(5));
        words.extend([db1_hash("SHEE") as i32, 9, 8, 0].map(Word::int));
        words.push(Word::header(kind::GROUP, 1, 6, 0));
        words.push(Word::header(kind::LINE, 8001, 5, 7));
        words.extend([0.0, 0.0, 1.0, 1.0].map(Word::real));
        words.resize(6 * WORDS_PER_RECORD, Word::int(0));
        to_bytes(&words)
    }

    #[test]
    fn a_hand_built_file_round_trips_exactly() {
        let bytes = tiny_file();
        let file = PictureFile::from_bytes(PathBuf::from("tiny"), bytes).unwrap();
        assert_eq!(file.name().text, "9 8 7 0 6");
        let report = roundtrip(&file).unwrap();
        assert!(report.is_exact(), "{report:?}");
        assert_eq!(
            (report.seg_hdr_ok, report.grp_hdr_ok, report.prim_ok),
            (1, 1, 1)
        );
        assert_eq!(report.kinds.get(&kind::LINE), Some(&1));
        assert_eq!(
            report.stats_line(),
            "{'seg_hdr_ok': 1, 'seg_hdr_bad': 0, 'grp_hdr_ok': 1, 'grp_hdr_bad': 0, 'prim_ok': 1, 'prim_bad': 0, 'hdr_ok': 1, 'pictab_ok': 1, 'dir_ok': 1, 'name_ok': 1}"
        );
    }

    #[test]
    fn a_byte_the_decoder_does_not_carry_shows_up_as_a_mismatch() {
        let mut bytes = tiny_file();
        // An integer word's u0 is always 0 and is not kept by the decoder:
        // set one in the name block and the name no longer round-trips.
        bytes[2 * 128] = 0x01;
        let file = PictureFile::from_bytes(PathBuf::from("tiny"), bytes).unwrap();
        let report = roundtrip(&file).unwrap();
        assert!(!report.name_ok);
        assert!(!report.is_exact());
        assert_eq!(report.mismatches, vec![Mismatch::Name]);

        // Likewise the u0 of the LINE's first coordinate word is part of the
        // double and *is* carried, so changing it still round-trips.
        let mut bytes = tiny_file();
        bytes[5 * 128 + 8 * 8] = 0x01;
        let file = PictureFile::from_bytes(PathBuf::from("tiny"), bytes).unwrap();
        assert!(roundtrip(&file).unwrap().is_exact());
    }
}
