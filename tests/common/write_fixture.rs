//! Stage-0 scaffolding for the write tests: a throw-away copy of a sample
//! database, and byte-level assertions about what a write may and may not
//! touch (`docs/plans/2026-09-20-dabacon-write-plan.md` §2 阶段 0, §3).
//!
//! The copy lives in a `tempfile::TempDir`, so it goes with the test whether
//! the test passes or not. The installed samples are never written to.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// The main write fixture: the E3D 3.1 Marine sample design database. Its
/// session number and page count are read at test time, never hard-coded —
/// the sample is live and E3D keeps writing to it.
pub const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";

/// A small design database from the same sample, for the quick path.
pub const AMS5100: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001";

/// A cable-tray design database of the Marine sample (305 KB), one of the 24
/// installed databases with type-17 reference arrays set (`HCREF` / `TCREF`
/// on `CWBRAN`, `CTWALL`, …) and the `HCREF` / `TCREF` / `CONNRE` tables that
/// file them, one edge per array entry.
pub const AMS251142: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001";

/// A private copy of `source` to write on.
pub struct DbCopy {
    _dir: TempDir,
    pub path: PathBuf,
    /// The bytes as they were before any write, for the assertions below.
    pub original: Vec<u8>,
}

impl DbCopy {
    pub fn of(source: &str) -> DbCopy {
        let dir = tempfile::Builder::new()
            .prefix("e3d-io-write-")
            .tempdir()
            .expect("temp dir");
        let name = Path::new(source)
            .file_name()
            .expect("sample has a file name")
            .to_owned();
        let path = dir.path().join(name);
        fs::copy(source, &path).expect("copy the sample database");
        let original = fs::read(&path).expect("read the copy back");
        DbCopy {
            _dir: dir,
            path,
            original,
        }
    }

    pub fn current_bytes(&self) -> Vec<u8> {
        fs::read(&self.path).expect("read the written copy")
    }
}

/// The word at `word` (0-based, 4-byte, big-endian) of page `page_no`.
pub fn page_word(bytes: &[u8], page_bytes: usize, page_no: usize, word: usize) -> u32 {
    let at = page_no * page_bytes + word * 4;
    u32::from_be_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

/// Every page of `before` up to and including `until_page` is unchanged in
/// `after`, except page 0, where only the listed words may differ. Reports
/// the first offending page and word.
pub fn assert_prefix_unchanged(
    before: &[u8],
    after: &[u8],
    page_bytes: usize,
    until_page: usize,
    page0_words_allowed: &[usize],
) {
    assert!(
        after.len() >= (until_page + 1) * page_bytes,
        "written file is shorter ({} bytes) than the {} pages it must preserve",
        after.len(),
        until_page + 1
    );
    for page in 0..=until_page {
        let range = page * page_bytes..(page + 1) * page_bytes;
        let (b, a) = (&before[range.clone()], &after[range]);
        if b == a {
            continue;
        }
        let words = page_bytes / 4;
        for word in 0..words {
            let (bw, aw) = (&b[word * 4..word * 4 + 4], &a[word * 4..word * 4 + 4]);
            if bw == aw {
                continue;
            }
            assert!(
                page == 0 && page0_words_allowed.contains(&word),
                "page {page} word {word} changed ({:02x?} -> {:02x?}); only page 0 words {page0_words_allowed:?} may",
                bw,
                aw
            );
        }
    }
}

/// The pages of `after` past `old_end` are exactly `old_end + 1 ..= new_end`,
/// the file ends right after `new_end`, and each new page carries one of the
/// `allowed_types` in its first word.
pub fn assert_appended_contiguous(
    after: &[u8],
    page_bytes: usize,
    old_end: usize,
    new_end: usize,
    allowed_types: &[u32],
) {
    assert_eq!(
        after.len(),
        (new_end + 1) * page_bytes,
        "file length must be exactly {} pages (new end {new_end})",
        new_end + 1
    );
    for page in old_end + 1..=new_end {
        let page_type = page_word(after, page_bytes, page, 0);
        assert!(
            allowed_types.contains(&page_type),
            "new page {page} has type {page_type}, allowed {allowed_types:?}"
        );
    }
}
