//! The L3 contract: a record is exactly what it says it is, and it says where.
//!
//! One gate, and it is the strongest thing this crate can check without a
//! second reader to compare against: **every key the index names resolves to a
//! record that says the same RefNo back**. Nothing about that is structural —
//! the index and the record are written separately, so an addressing mistake
//! anywhere between the session root and the record header shows up here as an
//! element answering to somebody else's name.
//!
//! The reader this replaces could not pass it. Searching for an aligned
//! `0x00000007` to find the end of a record failed outright on 1 033 of the
//! 789 831 keys, and expecting the blocks to sit behind the implicit region
//! missed 46 566 blocks that are addressed from elsewhere.
//!
//! The second gate is quieter and holds the two layers together: the bytes the
//! reader hands over are exactly the bytes the parser consumes. When they
//! differ, the surplus is somebody else's record, and everything that walks
//! off the end of one element and into the next starts there.

use std::collections::BTreeMap;

use e3d_io::index::{IndexCursor, IndexEntry};
use e3d_io::page::PageId;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::{read_element_record, read_record};
use e3d_io::session::DbView;

mod common;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const AMS7329: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7329_0001";
const AMS5100: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001";
const SAMPLES: &[&str] = &[AMS8000, AMS7329, AMS5100];

#[derive(Default, Debug)]
struct Census {
    keys: usize,
    blocks: u32,
    /// Records whose parts are not all on one page.
    scattered: usize,
}

fn audit(path: &str) -> Census {
    let mut view = DbView::open(path).expect("sample database opens");
    let sesno = view.selected_session();
    let ext = view.index_root().ext;

    let entries: Vec<IndexEntry> = IndexCursor::new(&mut view)
        .walk()
        .map(|record| record.expect("the tree walks").entry())
        .collect();
    assert!(!entries.is_empty(), "{path} indexes nothing");

    let mut census = Census::default();
    let pages = view.pages_mut();
    for entry in &entries {
        census.keys += 1;
        let record = read_record(pages, PageId::new(ext, entry.page_no), entry.offset_words)
            .unwrap_or_else(|error| {
                panic!(
                    "{path} session {sesno}: {} is indexed but {error}",
                    entry.refno
                )
            });
        assert_eq!(
            record.refno,
            entry.refno,
            "{path}: the index says {} lives at page {} byte {}, and the record there is {}",
            entry.refno,
            entry.page_no,
            entry.offset_words * 2,
            record.refno
        );
        census.blocks += record.blocks;
        if record.pages > 1 {
            census.scattered += 1;
        }
    }
    census
}

/// **The gate.**
#[test]
fn every_indexed_key_reads_a_record_that_confirms_it() {
    if common::missing(SAMPLES) {
        return;
    }
    for path in SAMPLES {
        let census = audit(path);
        println!(
            "{path}: keys={} blocks={} scattered={}",
            census.keys, census.blocks, census.scattered
        );
    }
}

/// The same gate over the corpus the page layout and the index were settled on.
#[test]
#[ignore = "reads every record of 429 databases; run with --ignored"]
fn every_indexed_key_confirms_across_the_whole_corpus() {
    if common::missing(&[AMS_DIR]) {
        return;
    }
    let mut files = 0;
    let mut keys = 0;
    let mut scattered = 0;
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
        let census = audit(&entry.path().to_string_lossy());
        files += 1;
        keys += census.keys;
        scattered += census.scattered;
    }
    assert_eq!(files, 429, "the adjudication used 429 files");
    println!("files={files} keys={keys} records_on_more_than_one_page={scattered}");
}

/// **Reverting to a reader that stops at a marker turns this red.** It handed
/// over whatever lay between the record and the next `0x00000007`, which on
/// 96% of the corpus is part of the following record.
#[test]
fn the_reader_hands_over_exactly_what_the_parser_consumes() {
    if common::missing(SAMPLES) {
        return;
    }
    for path in SAMPLES {
        let mut view = DbView::open(path).expect("sample database opens");
        let entries: Vec<IndexEntry> = IndexCursor::new(&mut view)
            .walk()
            .map(|record| record.expect("the tree walks").entry())
            .take(2_000)
            .collect();

        let mut shapes: BTreeMap<usize, usize> = BTreeMap::new();
        let pages = view.pages_mut();
        for entry in &entries {
            let view = read_element_record(pages, entry).expect("the record reads");
            let parsed = ParsedElement::parse(&view.raw_bytes).expect("the record parses");
            assert_eq!(
                parsed.prefix_offset, 0,
                "{path}: {} starts at its own header, not at padding",
                entry.refno
            );
            assert_eq!(
                parsed.record_end,
                view.raw_bytes.len(),
                "{path}: {} was handed {} bytes and the parser stopped at {}",
                entry.refno,
                view.raw_bytes.len(),
                parsed.record_end
            );
            *shapes
                .entry(
                    parsed.members.len().min(1) * 2
                        + usize::from(!parsed.explicit_attributes.is_empty()),
                )
                .or_default() += 1;
        }
        println!("{path}: {} records, shapes {shapes:?}", entries.len());
    }
}

/// A location that is not a record is an error naming it, never an empty one.
#[test]
fn a_location_that_is_not_a_record_is_refused() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let mut view = DbView::open(AMS8000).expect("sample database opens");
    // Page 0 is the descriptor; its first word is the page type, which is 0.
    let error = read_record(view.pages_mut(), PageId::primary(0), 0).unwrap_err();
    assert!(error.to_string().contains("no element record"), "{error}");
}
