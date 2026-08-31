//! The scan contract: what enumeration names, the scan reads — all of it,
//! in the same order, confirmed against the records themselves.
//!
//! `scan_elements` is the primitive derived indexes are built from (the file
//! states no index but the RefNo tree), so what it must not do is invent or
//! lose elements: its key set is pinned to `indexed_refnos` over the whole
//! corpus, and every yielded element has been confirmed against the record's
//! own header.
//!
//! The synthetic cases run everywhere; the corpus gates say what they need
//! and skip when it is absent.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use e3d_io::{ReadOnlyEngine, RefNo, ScanTier};

mod common;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const AMS7329: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7329_0001";
const AMS5100: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001";
const SAMPLES: &[&str] = &[AMS8000, AMS7329, AMS5100];

const INDEX_NOUN: u32 = 13_387_743;
const SENTINEL: u32 = 0x8000_0001;
const HEADER_DWORDS: u32 = 7;
const ENTRY_DWORDS: u32 = 4;

// ---------------------------------------------------------------- synthetic

/// 64 words a page, the same arithmetic the index contract tests use.
const WORDS: u32 = 64;
const PAGE_BYTES: usize = WORDS as usize * 4;

struct Db {
    pages: Vec<Vec<u8>>,
}

impl Db {
    fn new(pages: usize) -> Self {
        let mut db = Self {
            pages: vec![vec![0u8; PAGE_BYTES]; pages],
        };
        db.put(0, 0x34, WORDS);
        db.put(0, 0x28, 1);
        db.put(0, 0x2C, 1);
        db.put(0, 0x30, 1);
        db
    }

    fn put(&mut self, page: u32, at: usize, value: u32) {
        self.pages[page as usize][at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn session(&mut self, page: u32, sesno: u32, index_root: u32) {
        self.put(page, 0x00, 3);
        self.put(page, 0x04, u32::MAX);
        self.put(page, 0x0C, sesno);
        self.put(page, 0x1C, index_root);
        self.put(page, 0x20, 1);
    }

    fn index(&mut self, page: u32, level: u32, entries: &[[u32; 4]]) {
        let free = WORDS - HEADER_DWORDS - entries.len() as u32 * ENTRY_DWORDS;
        for (slot, word) in [INDEX_NOUN, level, 2, 2, 0, free].iter().enumerate() {
            self.put(page, (slot + 1) * 4, *word);
        }
        self.put(page, 0x00, 5);
        for (slot, entry) in entries.iter().enumerate() {
            let at = (HEADER_DWORDS as usize + slot * ENTRY_DWORDS as usize) * 4;
            for (word, value) in entry.iter().enumerate() {
                self.put(page, at + word * 4, *value);
            }
        }
    }

    /// An eleven-word element record with no blocks, on an element page.
    fn element(&mut self, page: u32, byte: usize, refno: RefNo, noun: u32, owner: RefNo) {
        self.put(page, 0, 7);
        for (word, value) in [
            11,
            refno.word0,
            refno.word1,
            noun,
            owner.word0,
            owner.word1,
            0,
            0,
            0,
            0,
            0,
        ]
        .iter()
        .enumerate()
        {
            self.put(page, byte + word * 4, *value);
        }
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

fn sentinel(page_no: u32) -> [u32; 4] {
    [SENTINEL, SENTINEL, page_no, 1]
}

fn leaf_entry(key: u32, page_no: u32, offset_words: u32) -> [u32; 4] {
    [0, key, page_no, (offset_words << 12) | 1]
}

/// A root over two leaves, six keys, records on two element pages.
fn scannable_db(dir: &Path) -> PathBuf {
    let mut db = Db::new(9);
    db.session(1, 7, 2);
    db.index(2, 1, &[sentinel(3), leaf_entry(200, 4, 0)]);
    db.index(
        3,
        0,
        &[
            leaf_entry(10, 6, 20),
            leaf_entry(20, 6, 42),
            leaf_entry(100, 7, 20),
        ],
    );
    db.index(4, 0, &[leaf_entry(200, 7, 42), leaf_entry(210, 7, 64)]);
    for (key, page, byte, noun) in [
        (10u32, 6u32, 40usize, 0xAAA1),
        (20, 6, 84, 0xAAA2),
        (100, 7, 40, 0xBBB1),
        (200, 7, 84, 0xBBB2),
        (210, 7, 128, 0xBBB3),
    ] {
        db.element(page, byte, RefNo::new(0, key), noun, RefNo::new(0, 1));
    }
    db.write(dir)
}

#[test]
fn a_scan_yields_what_enumeration_names_in_key_order_with_the_header_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = scannable_db(dir.path());
    let mut engine = ReadOnlyEngine::open(&path).unwrap();

    let enumerated = engine.indexed_refnos().unwrap();
    let scanned: Vec<_> = engine
        .scan_elements(ScanTier::Header)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(
        scanned.iter().map(|e| e.refno).collect::<Vec<_>>(),
        enumerated
            .iter()
            .map(|(refno, _, _)| *refno)
            .collect::<Vec<_>>(),
        "the scan and the enumeration are the same walk"
    );
    assert_eq!(
        scanned
            .iter()
            .map(|e| (e.refno.word1, e.noun_hash, e.owner.word1))
            .collect::<Vec<_>>(),
        vec![
            (10, 0xAAA1, 1),
            (20, 0xAAA2, 1),
            (100, 0xBBB1, 1),
            (200, 0xBBB2, 1),
            (210, 0xBBB3, 1),
        ],
        "nouns and owners come off the record headers"
    );

    // The named tier parses the whole record; these store no NAME.
    let named: Vec<_> = engine
        .scan_elements(ScanTier::Named)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(named.iter().all(|e| e.name.is_none()));
    assert!(named.iter().all(|e| e.parsed.is_none()));

    let full: Vec<_> = engine
        .scan_elements(ScanTier::Full)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(full.iter().all(|e| e.parsed.is_some()));
    assert_eq!(full[0].parsed.as_ref().unwrap().refno, RefNo::new(0, 10));
}

/// **Reverting the self-confirmation turns this red.** A leaf that addresses
/// somebody else's record is one failed element, not a poisoned scan and not
/// a quietly wrong one.
#[test]
fn a_record_that_answers_to_another_name_fails_that_element_and_the_scan_carries_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::new(8);
    db.session(1, 7, 2);
    db.index(2, 0, &[leaf_entry(10, 6, 20), leaf_entry(20, 6, 42)]);
    db.element(6, 40, RefNo::new(0, 10), 0xAAA1, RefNo::new(0, 1));
    // The record at key 20's address states it is 999.
    db.element(6, 84, RefNo::new(0, 999), 0xAAA2, RefNo::new(0, 1));
    let path = db.write(dir.path());

    let mut engine = ReadOnlyEngine::open(&path).unwrap();
    let results: Vec<_> = engine.scan_elements(ScanTier::Header).unwrap().collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_ref().unwrap().refno, RefNo::new(0, 10));
    let error = results[1].as_ref().unwrap_err();
    assert!(
        error.to_string().contains("999"),
        "the mismatch names the record that answered: {error}"
    );
}

/// **Reverting the element-page check turns this red.** A leaf that addresses
/// an index page is refused by page type, before its bytes are read as a
/// record.
#[test]
fn a_leaf_that_addresses_an_index_page_is_refused_by_page_type() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::new(8);
    db.session(1, 7, 2);
    db.index(2, 0, &[leaf_entry(10, 3, 20)]);
    // Page 3 is another index page, not an element page.
    db.index(3, 0, &[]);
    let path = db.write(dir.path());

    let mut engine = ReadOnlyEngine::open(&path).unwrap();
    let results: Vec<_> = engine.scan_elements(ScanTier::Header).unwrap().collect();
    let error = results[0].as_ref().unwrap_err();
    assert!(
        error.to_string().contains("not an element data page"),
        "{error}"
    );
}

// -------------------------------------------------------------- real files

/// **The key-set gate on the sample databases.**
#[test]
fn a_scan_names_exactly_the_indexed_keys_on_the_sample_databases() {
    if common::missing(SAMPLES) {
        return;
    }
    for path in SAMPLES {
        let mut engine = ReadOnlyEngine::open(path).expect("sample database opens");
        let enumerated: Vec<RefNo> = engine
            .indexed_refnos()
            .expect("the tree walks")
            .into_iter()
            .map(|(refno, _, _)| refno)
            .collect();
        let scanned: Vec<_> = engine
            .scan_elements(ScanTier::Named)
            .expect("the tree walks")
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|error| panic!("{path}: {error}"));

        assert_eq!(
            scanned.iter().map(|e| e.refno).collect::<Vec<_>>(),
            enumerated,
            "{path}: the scan names exactly the indexed keys, in order"
        );

        // Spot-check the names against the one-element path.
        let mut checked = 0usize;
        for element in scanned.iter().step_by(97) {
            let view = engine
                .find_element(element.refno)
                .expect("lookup")
                .expect("the scanned element resolves");
            let parsed =
                e3d_io::record::element::ParsedElement::parse(&view.raw_bytes).expect("parses");
            assert_eq!(
                element.name,
                e3d_io::element_name::stored_name(&parsed),
                "{path}: {} scanned name differs from the one-element read",
                element.refno
            );
            checked += 1;
        }
        assert!(checked > 0, "{path}: nothing was spot-checked");
        let named = scanned.iter().filter(|e| e.name.is_some()).count();
        println!("{path}: {} elements, {named} named", scanned.len());
    }
}

/// The key-set gate over the corpus the index layer was settled on, with the
/// throughput numbers the derived-index decision needs.
#[test]
#[ignore = "scans every element of 429 databases twice; run with --ignored"]
fn a_scan_names_exactly_the_indexed_keys_across_the_whole_corpus() {
    if common::missing(&[AMS_DIR]) {
        return;
    }
    let mut files = 0usize;
    let mut keys = 0usize;
    let mut named = 0usize;
    let mut header_elapsed = std::time::Duration::ZERO;
    let mut full_elapsed = std::time::Duration::ZERO;

    for entry in std::fs::read_dir(AMS_DIR).expect("sample directory reads") {
        let entry = entry.expect("directory entry");
        let metadata = entry.metadata().expect("entry metadata");
        let name = entry.file_name().to_string_lossy().into_owned();
        let sampled = name.starts_with("ams")
            && name.len() > 5
            && name.as_bytes()[name.len() - 5] == b'_'
            && name[name.len() - 4..].bytes().all(|b| b.is_ascii_digit())
            && name[3..name.len() - 5].bytes().all(|b| b.is_ascii_digit());
        if !metadata.is_file() || !sampled {
            continue;
        }
        if metadata.len() >= 25 * 1024 * 1024 || metadata.len() <= 20 * 1024 {
            continue;
        }
        let path = entry.path();
        let mut engine = ReadOnlyEngine::open(&path).expect("database opens");
        let enumerated: BTreeMap<(u32, u32), (u32, u32)> = engine
            .indexed_refnos()
            .expect("the tree walks")
            .into_iter()
            .map(|(refno, page, off)| ((refno.word0, refno.word1), (page, off)))
            .collect();

        let started = std::time::Instant::now();
        let mut header_keys = BTreeMap::new();
        for element in engine.scan_elements(ScanTier::Header).expect("scans") {
            let element = element.unwrap_or_else(|error| panic!("{name}: {error}"));
            header_keys.insert(
                (element.refno.word0, element.refno.word1),
                (element.page_no, element.offset_words),
            );
        }
        header_elapsed += started.elapsed();
        assert_eq!(
            header_keys, enumerated,
            "{name}: the header scan and the enumeration disagree"
        );

        let started = std::time::Instant::now();
        for element in engine.scan_elements(ScanTier::Full).expect("scans") {
            let element = element.unwrap_or_else(|error| panic!("{name}: {error}"));
            named += usize::from(element.name.is_some());
        }
        full_elapsed += started.elapsed();

        files += 1;
        keys += header_keys.len();
    }

    assert_eq!(files, 429, "the adjudication used 429 files");
    println!(
        "files={files} keys={keys} named={named} header_scan={header_elapsed:?} \
         full_scan={full_elapsed:?}"
    );
}
