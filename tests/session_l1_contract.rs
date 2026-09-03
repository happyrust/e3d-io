//! The L1 contract: one resolver for "latest" and "at session N", and a named
//! error where the old reader answered `None`.
//!
//! The synthetic cases run everywhere; the two that need the installed E3D
//! fixtures say so and skip.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use e3d_io::error::{E3dReaderError, SessionError};
use e3d_io::meta::DbDescriptor;
use e3d_io::page::PageId;
use e3d_io::session::{DbView, SessionSelector};

mod common;

const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const FIXTURES: &[&str] = &[AMS8000];

/// Session page word holding the session number.
const SESNO: u64 = 0x0C;
/// Session page word holding the index root page.
const INDEX_ROOT_PGNO: u64 = 0x1C;
/// Descriptor word holding the newest session's page.
const LATEST_SES_PGNO: u64 = 0x28;
/// Descriptor word holding the oldest session's page.
const FIRST_SES_PGNO: u64 = 0x30;
/// Descriptor word holding the page size, in 4-byte words.
const PAGE_SIZE_WORDS: u64 = 0x34;

/// Read one big-endian word straight from the file, bypassing the library.
///
/// The point of the fixture assertions is to compare what `DbView` reports
/// against the bytes on disk, so they must not be read through `DbView`.
fn raw_word(path: &Path, offset: u64) -> u32 {
    let mut file = File::open(path).expect("open fixture");
    file.seek(SeekFrom::Start(offset)).expect("seek");
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf).expect("read");
    u32::from_be_bytes(buf)
}

fn raw_session_word(path: &Path, page_bytes: u64, page: u32, at: u64) -> u32 {
    raw_word(path, u64::from(page) * page_bytes + at)
}

/// The newest session number as the bytes state it: the `sesno` word of the
/// page the descriptor's `latest_ses_pgno` names.
///
/// The sample databases are live — E3D keeps writing sessions to them
/// (`ams8000_0001` went from 264 to 281 between two runs of this file) — so a
/// session number written into an assertion is a number that will be wrong.
fn newest_sesno_on_disk(path: &Path) -> u32 {
    let page_bytes = u64::from(raw_word(path, PAGE_SIZE_WORDS)) * 4;
    raw_session_word(path, page_bytes, raw_word(path, LATEST_SES_PGNO), SESNO)
}

/// Sessions run newest first, strictly descending, and end at the head of the
/// chain — which is what a walk has to show whether or not the file has been
/// merged (a merged file keeps fewer sessions than its newest `sesno`).
fn assert_well_formed_chain(sesnos: &[u32], newest: u32) {
    assert_eq!(
        sesnos.first().copied(),
        Some(newest),
        "chain starts at the newest session"
    );
    assert!(
        sesnos.windows(2).all(|pair| pair[0] > pair[1]),
        "sesno must strictly descend along the chain: {sesnos:?}"
    );
    assert_eq!(
        sesnos.last().copied(),
        Some(1),
        "chain reaches back to session 1"
    );
    assert!(
        sesnos.len() <= newest as usize,
        "{} sessions on a chain whose newest sesno is {newest}",
        sesnos.len()
    );
}

// ---------------------------------------------------------------- synthetic

const WORDS: u32 = 64;
const PAGE_BYTES: usize = WORDS as usize * 4;

struct Db {
    pages: Vec<Vec<u8>>,
}

impl Db {
    /// A file whose descriptor points at `latest_ses_pgno`.
    fn new(latest_ses_pgno: u32, first_ses_pgno: u32, pages: usize) -> Self {
        let mut db = Self {
            pages: vec![vec![0u8; PAGE_BYTES]; pages],
        };
        db.put(0, 0x34, WORDS);
        db.put(0, 0x28, latest_ses_pgno);
        db.put(0, 0x2C, 1);
        db.put(0, 0x30, first_ses_pgno);
        db.put(0, 0x38, 16_192);
        db
    }

    fn put(&mut self, page: u32, at: usize, value: u32) {
        self.pages[page as usize][at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    /// Write a session page: `previous` of `None` ends the chain.
    fn session(&mut self, page: u32, sesno: u32, previous: Option<u32>, index_root: u32) {
        self.put(page, 0x00, 3);
        self.put(page, 0x04, previous.unwrap_or(u32::MAX));
        self.put(page, 0x08, 1);
        self.put(page, 0x0C, sesno);
        self.put(page, 0x1C, index_root);
        self.put(page, 0x20, 1);
    }

    fn write(&self, dir: &Path) -> PathBuf {
        let path = dir.join("ams1000_0001");
        let mut file = File::create(&path).unwrap();
        for page in &self.pages {
            file.write_all(page).unwrap();
        }
        file.flush().unwrap();
        path
    }
}

/// Three sessions, newest last: 3 -> 2 -> 1, each with its own index root.
fn three_session_db(dir: &Path) -> PathBuf {
    let mut db = Db::new(3, 1, 8);
    db.session(1, 1, None, 10);
    db.session(2, 2, Some(1), 20);
    db.session(3, 3, Some(2), 30);
    db.write(dir)
}

#[test]
fn open_takes_the_newest_session_and_open_at_takes_the_one_named() {
    let dir = tempfile::tempdir().unwrap();
    let path = three_session_db(dir.path());

    let latest = DbView::open(&path).unwrap();
    assert_eq!(latest.selected_session(), 3);
    assert_eq!(latest.index_root(), PageId::new(1, 30));

    for (sesno, root) in [(1u32, 10u32), (2, 20), (3, 30)] {
        let view = DbView::open_at(&path, sesno).unwrap();
        assert_eq!(view.selected_session(), sesno);
        assert_eq!(
            view.index_root(),
            PageId::new(1, root),
            "session {sesno} must descend its own root"
        );
    }

    // Latest is reachable through the selector too, and gives the same answer
    // as `open` — there is only the one resolver.
    let explicit = DbView::open_at(&path, SessionSelector::Latest).unwrap();
    assert_eq!(explicit.selected_session(), latest.selected_session());
    assert_eq!(explicit.index_root(), latest.index_root());
}

/// **Acceptance ④.** A session that is not there is an error naming it.
#[test]
fn an_absent_session_is_an_error_naming_it_not_a_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = three_session_db(dir.path());
    let newest = DbView::open(&path).unwrap().index_root();

    // Newer than the newest: rejected without walking.
    let err = DbView::open_at(&path, 99).unwrap_err();
    assert!(
        matches!(
            err,
            E3dReaderError::Session(SessionError::NoSuchSession {
                requested: 99,
                newest: 3,
                ..
            })
        ),
        "{err}"
    );
    assert!(err.to_string().contains("99"), "error must name it: {err}");

    // Inside the range but absent: walks to the head and still refuses.
    let err = DbView::open_at(&path, 0).unwrap_err();
    match err {
        E3dReaderError::Session(SessionError::NoSuchSession {
            requested: 0,
            newest: 3,
            oldest,
            ..
        }) => assert_eq!(oldest, Some(1), "the whole chain should have been walked"),
        other => panic!("expected NoSuchSession, got {other}"),
    }

    // And neither call quietly handed back the newest session instead.
    assert_eq!(newest, PageId::new(1, 30));
    assert!(DbView::open_at(&path, 99).is_err());
    assert!(DbView::open_at(&path, 0).is_err());
}

/// A chain that points back into itself must report, not spin.
#[test]
fn a_looping_session_chain_is_reported_rather_than_followed() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::new(3, 1, 8);
    db.session(1, 1, None, 10);
    db.session(2, 2, Some(3), 20); // back-edge: 2 -> 3 -> 2 -> ...
    db.session(3, 3, Some(2), 30);
    let path = db.write(dir.path());

    // The newest session still opens: the loop is behind it.
    assert_eq!(DbView::open(&path).unwrap().selected_session(), 3);

    let err = DbView::open_at(&path, 1).unwrap_err();
    assert!(
        matches!(
            err,
            E3dReaderError::Session(SessionError::ChainCycle { .. })
        ),
        "{err}"
    );
}

/// A malformed session page and an absent session are different facts.
#[test]
fn a_broken_session_page_does_not_read_as_a_missing_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::new(3, 1, 8);
    db.session(1, 1, None, 10);
    db.session(2, 2, Some(1), 20);
    db.session(3, 3, Some(2), 30);
    db.put(2, 0x00, 5); // page 2 is no longer a session page
    let path = db.write(dir.path());

    let err = DbView::open_at(&path, 1).unwrap_err();
    assert!(
        matches!(
            err,
            E3dReaderError::Session(SessionError::NotASessionPage { page_type: 5, .. })
        ),
        "a corrupt page must not be reported as a missing session: {err}"
    );
}

/// Walking back N sessions reads N pages, and says so.
#[test]
fn the_cost_of_reaching_back_is_visible() {
    let dir = tempfile::tempdir().unwrap();
    let path = three_session_db(dir.path());

    assert_eq!(DbView::open(&path).unwrap().sessions_read(), 1);

    let view = DbView::open_at(&path, 1).unwrap();
    assert_eq!(view.sessions_read(), 3);
    // Page 0 plus the three session pages, none of them read twice.
    assert_eq!(view.stats().pages_read, 4);
}

/// **Acceptance ⑤.** `open` must be `open_at`, not a second implementation.
#[test]
fn open_is_implemented_by_calling_open_at() {
    let source =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/session/mod.rs"))
            .unwrap();

    let body = fn_body(&source, "pub fn open(").expect("DbView::open should exist");
    assert!(
        body.contains("open_at("),
        "open must delegate to open_at, its body is:\n{body}"
    );
}

/// Source text of the block that follows `signature`.
fn fn_body<'a>(source: &'a str, signature: &str) -> Option<&'a str> {
    let start = source.find(signature)?;
    let open = start + source[start..].find('{')?;
    let mut depth = 0usize;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open..=open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

// ----------------------------------------------------------------- fixtures

/// **Acceptance ②.** Every sample database opens, and reports the session the
/// bytes say it opened.
#[test]
fn every_sample_database_opens_at_the_session_its_header_points_to() {
    if common::missing(&[AMS]) {
        return;
    }
    let files = sample_databases(Path::new(AMS));
    assert!(!files.is_empty(), "no sample databases under {AMS}");

    let mut failures = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let view = match DbView::open(path) {
            Ok(view) => view,
            Err(err) => {
                failures.push(format!("{name}: {err}"));
                continue;
            }
        };

        let page_bytes = u64::from(raw_word(path, PAGE_SIZE_WORDS)) * 4;
        let latest_page = raw_word(path, LATEST_SES_PGNO);
        let sesno = raw_session_word(path, page_bytes, latest_page, SESNO);

        if view.selected_session() != sesno {
            failures.push(format!(
                "{name}: selected_session() = {}, page {latest_page} states {sesno}",
                view.selected_session()
            ));
        }
        if view.session().page != PageId::new(1, latest_page) {
            failures.push(format!(
                "{name}: opened page {} but the header points at {latest_page}",
                view.session().page
            ));
        }
        if view.sessions_read() != 1 {
            failures.push(format!(
                "{name}: opening the newest session read {} session pages",
                view.sessions_read()
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} databases disagreed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
    eprintln!(
        "{} sample databases opened at their newest session",
        files.len()
    );
}

/// **Acceptance ③.** An older session hands back that session's own index
/// root, byte for byte, and it is not the newest one.
#[test]
fn an_older_session_descends_the_root_that_session_recorded() {
    if common::missing(FIXTURES) {
        return;
    }
    let path = Path::new(AMS8000);
    let page_bytes = u64::from(raw_word(path, PAGE_SIZE_WORDS)) * 4;

    let latest = DbView::open(path).unwrap();
    let newest = newest_sesno_on_disk(path);
    assert_eq!(
        latest.selected_session(),
        newest,
        "ams8000_0001 opens at the session its header names"
    );

    let earlier_page = latest
        .session()
        .previous
        .expect("the newest session has a predecessor");
    let earlier_sesno = raw_session_word(path, page_bytes, earlier_page.page, SESNO);
    let earlier_root = raw_session_word(path, page_bytes, earlier_page.page, INDEX_ROOT_PGNO);

    let view = DbView::open_at(path, earlier_sesno).unwrap();
    assert_eq!(view.selected_session(), earlier_sesno);
    assert_eq!(view.session().page, earlier_page);
    assert_eq!(
        view.index_root(),
        PageId::new(1, earlier_root),
        "the root must be the one that session page records"
    );
    assert_ne!(
        view.index_root(),
        latest.index_root(),
        "a different session must give a different root, or the test proves nothing"
    );

    // Reaching back one session costs exactly one extra session page.
    assert_eq!(view.sessions_read(), 2);

    // And the whole chain walks without a cycle or a break.
    let mut walker = DbView::open(path).unwrap();
    let chain = walker.walk_sessions().unwrap();
    let sesnos: Vec<u32> = chain.iter().map(|page| page.sesno).collect();
    assert_well_formed_chain(&sesnos, newest);
    assert_eq!(chain.last().unwrap().previous, None);
}

/// `ReadOnlyEngine::sessions` must come through the same chain walk.
///
/// **Reverting it to "read one page from +0x30" turns this red.** That version
/// returned a one-element vector describing session 1 of 264 and called it
/// success.
#[test]
fn the_engine_reports_the_whole_chain_from_the_newest_session() {
    if common::missing(FIXTURES) {
        return;
    }
    let mut engine = e3d_io::ReadOnlyEngine::open(AMS8000).unwrap();
    let sessions = engine.sessions().unwrap();

    assert!(
        sessions.len() > 1,
        "a 264-session database must not report {} session(s)",
        sessions.len()
    );

    let view = DbView::open(Path::new(AMS8000)).unwrap();
    assert_eq!(
        sessions.first().unwrap().session_id,
        view.selected_session(),
        "the chain must start at the newest session, not the oldest"
    );
    let sesnos: Vec<u32> = sessions.iter().map(|info| info.session_id).collect();
    assert_well_formed_chain(&sesnos, newest_sesno_on_disk(Path::new(AMS8000)));
    // The head of the chain points nowhere, and +0x10 was never a start page.
    assert_eq!(sessions.last().unwrap().session_mark, u32::MAX);
    assert_eq!(sessions.first().unwrap().start_page_no, u32::MAX);
}

/// An unreadable header is its own error, not a missing session.
#[test]
fn a_file_that_is_not_a_database_fails_at_the_header() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ams1000_0001");
    std::fs::write(&path, vec![b'#'; 4096]).unwrap();

    let err = DbView::open(&path).unwrap_err();
    assert!(
        !matches!(
            err,
            E3dReaderError::Session(SessionError::NoSuchSession { .. })
        ),
        "a broken header must not be reported as a missing session: {err}"
    );
}

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

/// The descriptor is still readable through its own type.
#[test]
fn the_descriptor_reports_both_session_pointers() {
    if common::missing(FIXTURES) {
        return;
    }
    let path = Path::new(AMS8000);
    let view = DbView::open(path).unwrap();
    let desc: &DbDescriptor = view.descriptor();
    assert_eq!(desc.db_mark, 8000);
    assert_eq!(desc.page_size, 2048);
    assert_eq!(desc.page_size_words, 512);
    // Both pointers are compared with the bytes at their offsets rather than
    // with numbers: the newest session's page moves every time E3D writes a
    // session to this file (8897 when this test was written, 9277 by now).
    assert_eq!(
        desc.latest_ses_pgno,
        raw_word(path, LATEST_SES_PGNO),
        "+0x28 is the newest session"
    );
    assert_eq!(
        desc.session_page_no,
        raw_word(path, FIRST_SES_PGNO),
        "+0x30 is the oldest session"
    );
    assert_ne!(
        desc.latest_ses_pgno, desc.session_page_no,
        "the two pointers are not interchangeable"
    );
}
