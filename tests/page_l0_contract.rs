//! The L0 page layer's contract, expressed as tests that go red if the reader
//! is written the way it used to be.
//!
//! Each test names the defect it pins down. They are not general regression
//! coverage — the point is that reverting any one of the three rewritten
//! behaviours fails exactly one of these.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use e3d_io::error::PageIoError;
use e3d_io::page::io::PageReader;
use e3d_io::page::{PageId, PageSize, PageSource};
use e3d_io::E3dReaderError;

mod common;

const DEFAULT_PROJECT: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const FIXTURES: &[&str] = &[DEFAULT_PROJECT];

/// The page size every database of the sample project states.
const SAMPLE_PAGE_BYTES: usize = 2048;

/// Descriptor field holding the page of the newest session.
const LATEST_SES_PGNO: usize = 0x28;
/// Descriptor field holding the page size, counted in 4-byte words.
const PAGE_SIZE_WORDS: usize = 0x34;
/// Session page field holding the session number.
const SESSION_SESNO: usize = 0x0C;

const SESSION_PAGE_TYPE: u32 = 3;

fn be32(page: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(page[at..at + 4].try_into().unwrap())
}

/// Write a database-shaped file whose header states `words` words per page.
///
/// Each page starts with a marker byte, so a read that lands on the wrong page
/// — or in the wrong file — shows up as a value rather than as silence.
fn synth_db(dir: &Path, name: &str, words: u32, pages: usize) -> PathBuf {
    assert!(words >= 32, "the file must be at least one 128 byte page");
    let page_bytes = words as usize * 4;
    let mut bytes = vec![0u8; page_bytes * pages];
    bytes[PAGE_SIZE_WORDS..PAGE_SIZE_WORDS + 4].copy_from_slice(&words.to_be_bytes());
    for page in 0..pages {
        bytes[page * page_bytes] = 0xE0 | page as u8;
    }
    write_file(dir.join(name), &bytes)
}

/// Overwrite the page size field without touching the file's real layout.
fn restate_page_size(path: &Path, words: u32) {
    let mut bytes = std::fs::read(path).unwrap();
    bytes[PAGE_SIZE_WORDS..PAGE_SIZE_WORDS + 4].copy_from_slice(&words.to_be_bytes());
    write_file(path.to_path_buf(), &bytes);
}

fn write_file(path: PathBuf, bytes: &[u8]) -> PathBuf {
    let mut file = File::create(&path).unwrap();
    file.write_all(bytes).unwrap();
    file.flush().unwrap();
    path
}

/// Acceptance ③ — an unusable page size field must stop the open.
///
/// The reader this replaces ended its validation with `else { DEFAULT_PAGE_SIZE }`,
/// so a file stating anything unusable was read at 4096 bytes per page and
/// every subsequent page address was wrong, silently.
#[test]
fn an_unusable_page_size_field_fails_the_open_instead_of_defaulting() {
    let dir = tempfile::tempdir().unwrap();

    // Below the minimum, not a power of two, above the maximum, and a value
    // whose byte count overflows. The file itself stays a well-formed two-page
    // file — only the field is wrong, which is the case the fallback hid.
    for words in [0u32, 1, 700, 30_000, u32::MAX] {
        let path = synth_db(dir.path(), "ams1000_0001", 32, 2);
        restate_page_size(&path, words);

        let err = match PageReader::open(&path) {
            Ok(reader) => panic!(
                "words={words} was accepted as {} — the fallback is back",
                reader.page_size()
            ),
            Err(err) => err,
        };
        assert!(
            matches!(
                err,
                E3dReaderError::Page(PageIoError::InvalidPageSize { .. })
            ),
            "words={words} rejected for the wrong reason: {err}"
        );
    }
}

/// The +0x34 field counts 4-byte words. Reading it as bytes is what left 17 of
/// 490 production files unreadable.
#[test]
fn the_page_size_field_is_read_as_words() {
    let dir = tempfile::tempdir().unwrap();
    let path = synth_db(dir.path(), "ams1000_0001", 512, 2);
    let reader = PageReader::open(&path).unwrap();
    assert_eq!(reader.page_size().words(), 512);
    assert_eq!(reader.page_size().bytes(), 2048);
}

/// Acceptance ④ — an extent that is not open must be reported, not substituted.
///
/// `PageId { ext: 2, page: N }` used to come back with extent 1's page N and an
/// `Ok`, because the extent number was carried on the struct and never reached
/// the seek.
#[test]
fn an_absent_extent_is_reported_not_silently_read_from_the_primary() {
    let dir = tempfile::tempdir().unwrap();
    let path = synth_db(dir.path(), "ams1000_0001", 32, 4);
    let reader = PageReader::open(&path).unwrap();
    let mut buf = vec![0u8; reader.page_size().bytes()];

    reader.read_into(PageId::new(1, 3), &mut buf).unwrap();
    let primary_page_3 = buf[0];
    assert_eq!(primary_page_3, 0xE3);

    let err = reader
        .read_into(PageId::new(2, 3), &mut buf)
        .expect_err("extent 2 does not exist and must not resolve to extent 1");
    match err {
        PageIoError::MissingExtent {
            ext: 2,
            page: 3,
            expected_path,
        } => assert_eq!(
            expected_path.as_deref(),
            Some(dir.path().join("ams1000_0002").as_path()),
            "the error should name the file it wanted"
        ),
        other => panic!("expected MissingExtent, got {other}"),
    }
}

/// Acceptance ⑤ — no page-returning primitive may come back.
///
/// A `-> Vec<u8>` read is one heap allocation per page, so a tree descent
/// allocates once per level. The buffer pool only helps while nothing offers
/// the allocating shortcut, and that is a property of the source text rather
/// than of any value, so it is asserted against the source text.
#[test]
fn the_page_layer_offers_no_allocating_read() {
    let page_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/page");
    let mut offenders = Vec::new();

    for entry in std::fs::read_dir(&page_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for (lineno, line) in source.lines().enumerate() {
            let is_signature = line.contains("fn ") && line.contains("pub ");
            let returns_pages = line.contains("Vec<u8>") || line.contains("Box<[u8]>");
            if is_signature && returns_pages {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    lineno + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a page-allocating public primitive is back:\n{}",
        offenders.join("\n")
    );
}

/// Prefetch must not be billed to the counter the index layer differences.
#[test]
fn prefetch_is_counted_apart_from_demand_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = synth_db(dir.path(), "ams1000_0001", 32, 8);
    let reader = PageReader::open(&path).unwrap();
    let mut buf = vec![0u8; reader.page_size().bytes()];

    let before = reader.stats().pages_read;
    for page in 0..5 {
        reader
            .prefetch_into(PageId::primary(page), &mut buf)
            .unwrap();
    }
    reader.read_into(PageId::primary(6), &mut buf).unwrap();

    let stats = reader.stats();
    assert_eq!(
        stats.pages_read - before,
        1,
        "a lookup measured as a difference of pages_read must see only its own read"
    );
    assert_eq!(stats.prefetched_pages, 5);
}

/// Acceptance ② — every database of the sample project opens with no hint.
///
/// The file names its own page size and nothing else is supplied: no default,
/// no caller-provided size, no probing. Then the page the header points at has
/// to actually be a session page carrying a session number, which is the check
/// that the byte-vs-word defect failed — `ams7329_0001` reported `sesno = 0`
/// against an authoritative 221.
#[test]
fn every_sample_database_reads_its_own_page_size_and_session_page() {
    if common::missing(FIXTURES) {
        return;
    }
    let project =
        std::env::var("E3D_CENSUS_PROJECT").unwrap_or_else(|_| DEFAULT_PROJECT.to_string());
    let files = sample_databases(Path::new(&project));
    assert!(
        !files.is_empty(),
        "no ams*_NNNN databases of 20KB..25MB under {project}"
    );

    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();

        let reader = match PageReader::open(path) {
            Ok(reader) => reader,
            Err(err) => {
                failures.push(format!("{name}: open failed: {err}"));
                continue;
            }
        };
        if reader.page_size().bytes() != SAMPLE_PAGE_BYTES {
            failures.push(format!(
                "{name}: page size {}, expected {SAMPLE_PAGE_BYTES} bytes",
                reader.page_size()
            ));
            continue;
        }

        let mut page = vec![0u8; reader.page_size().bytes()];
        if let Err(err) = reader.read_into(PageId::primary(0), &mut page) {
            failures.push(format!("{name}: descriptor unreadable: {err}"));
            continue;
        }
        let latest_ses_pgno = be32(&page, LATEST_SES_PGNO);

        if let Err(err) = reader.read_into(PageId::primary(latest_ses_pgno), &mut page) {
            failures.push(format!(
                "{name}: session page {latest_ses_pgno} unreadable: {err}"
            ));
            continue;
        }
        let page_type = be32(&page, 0);
        let sesno = be32(&page, SESSION_SESNO);
        if page_type != SESSION_PAGE_TYPE {
            failures.push(format!(
                "{name}: page {latest_ses_pgno} is type {page_type}, expected {SESSION_PAGE_TYPE}"
            ));
            continue;
        }
        if sesno == 0 {
            failures.push(format!(
                "{name}: session page {latest_ses_pgno} states sesno 0"
            ));
            continue;
        }
        checked += 1;
    }

    assert!(
        failures.is_empty(),
        "{} of {} sample databases failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
    eprintln!("{checked} sample databases opened at {SAMPLE_PAGE_BYTES} bytes per page");
}

/// The 429 databases the index layout adjudication ran on: `ams<digits>_<4 digits>`,
/// larger than 20 KB and smaller than 25 MB.
fn sample_databases(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter(|entry| {
            entry
                .metadata()
                .map(|m| m.len() > 20_480 && m.len() < 25 * 1024 * 1024)
                .unwrap_or(false)
        })
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(is_sample_database_name)
        })
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files
}

fn is_sample_database_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("ams") else {
        return false;
    };
    let Some((digits, extent)) = rest.rsplit_once('_') else {
        return false;
    };
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && extent.len() == 4
        && extent.bytes().all(|b| b.is_ascii_digit())
}

/// `PageSize` is the only place words become bytes.
#[test]
fn the_field_value_is_never_the_byte_count() {
    // Every sample database states 512, and 512 means 2048 bytes.
    assert_eq!(PageSize::from_header_words(512).unwrap().bytes(), 2048);
    // A reader that took the field for a byte count would call 2048 the answer.
    // Here it means 8192 bytes, and the type says which is which.
    let misread = PageSize::from_header_words(2048).unwrap();
    assert_eq!(misread.words(), 2048);
    assert_eq!(misread.bytes(), 8192);
}
