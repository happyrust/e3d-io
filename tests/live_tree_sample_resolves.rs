//! Every element in the 2026-08-30 TTY sample resolves, in the session that
//! answers today, to itself and to the shape its row records.
//!
//! The list this gates — `evidence/2026-08-30-tty-sampling/sample.tsv` — is
//! drawn by `examples/tty_sample_live_tree.rs` from the tree the session page
//! names. The list it replaces was drawn from the leaf entries of every index
//! page in the file, which is the live tree plus every tree an earlier session
//! left behind, and 77 of its 211 rows are elements that no longer exist.
//! `tests/sampled_elements_resolve.rs` still reads the old list and still
//! measures that; this file is the gate the new one has to pass instead:
//!
//! 1. **Every row resolves to itself.** `find_element` finds it, and the record
//!    states the RefNo the row names — no exceptions, no `SUPERSEDED` table.
//!    The old list needs one because it recorded the oldest copy of four
//!    elements; a row drawn from the live tree records the copy the session
//!    answers with, so agreement is the only acceptable result.
//! 2. **Noun coverage does not fall.** Every `(noun, layout)` bucket the old
//!    list holds is either in the new one or in [`UNWITNESSED`], which is the
//!    complete list of shapes no live tree in the project holds.
//! 3. **Both storage layouts are covered**, which is the whole reason layout is
//!    an axis of the sample at all.
//!
//! Run with:
//! `cargo test --release --test live_tree_sample_resolves -- --nocapture`

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use e3d_attlib::db1_dehash;
use e3d_io::index::IndexCursor;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::read_element_record;
use e3d_io::record::template::{NounTemplate, StorageLayout};
use e3d_io::refno::RefNo;
use e3d_io::session::DbView;
use e3d_io::ReadOnlyEngine;

const SAMPLE: &str = "evidence/2026-08-30-tty-sampling/sample.tsv";
const OLD_SAMPLE: &str = "evidence/20260826-tty-sampling/sample.tsv";
const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

mod common;

const FIXTURES: &[&str] = &[AMS];

/// The libraries the sampler's first pass reads, and the ones
/// [`no_live_element_in_the_core_libraries_fills_a_bucket_called_unwitnessed`]
/// walks.
const CORE_DBS: &[&str] = &["ams7000_0001", "ams8000_0001", "ams1112_0001"];

/// Word 10 of a record header is its storage format word; see `HEADER_WORDS` in
/// `src/record/mod.rs`.
const FORMAT_WORD_INDEX: usize = 10;

/// `(noun, layout)` buckets the 2026-08-26 list holds and no live tree can fill.
///
/// Sixteen of the seventeen are the same fact seen sixteen times. The old
/// sampler read whichever copy of an element the oldest tree in the file
/// addressed, and for these nouns that copy is stored compact while the copy
/// the session answers with is stored wide — the element was rewritten from one
/// layout to the other and grew by two implicit words doing it. The four rows
/// `sampled_elements_resolve.rs` carries as `SUPERSEDED` are four of these
/// sixteen, seen through the word count instead of the layout.
///
/// The seventeenth, `SLCY`, is the one noun of the old list's 142 that the new
/// list does not carry at all: the project holds no live SLCY in any layout.
///
/// Each line is checked twice over: that the old list really has the bucket (so
/// an entry cannot outlive the row it excuses), and that the live trees really
/// do not (so an entry cannot excuse a sampler that simply stopped looking).
const UNWITNESSED: &[(&str, &str)] = &[
    ("BLTA", "compact"),
    ("BTSE", "compact"),
    ("CATA", "compact"),
    ("CATE", "compact"),
    ("CCTA", "compact"),
    ("COCO", "compact"),
    ("DTSE", "compact"),
    ("GMSE", "compact"),
    ("NBRWLD", "compact"),
    ("NOMINB", "compact"),
    ("PTSE", "compact"),
    ("SCOM", "compact"),
    ("SDTE", "compact"),
    ("SECT", "compact"),
    ("SLCY", "wide"),
    ("SMTE", "compact"),
    ("SPEC", "compact"),
];

/// The one noun of the old list the new one does not reach, and the reason it
/// is a measurement rather than a gap: no session of any library in the project
/// indexes an SLCY.
const LOST_NOUNS: &[&str] = &["SLCY"];

struct Row {
    db: String,
    noun: String,
    layout: String,
    implicit_words: i32,
    refno: RefNo,
}

fn read_list(path: &str) -> Option<Vec<Row>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => {
            eprintln!("SKIP: {path} not present");
            return None;
        }
    };
    let rows: Vec<Row> = text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| {
            let [db, noun, layout, words, refno] = line.split('\t').collect::<Vec<_>>()[..] else {
                return None;
            };
            Some(Row {
                db: db.to_string(),
                noun: noun.to_string(),
                layout: layout.to_string(),
                implicit_words: words.parse().ok()?,
                refno: parse_refno(refno)?,
            })
        })
        .collect();
    (!rows.is_empty()).then_some(rows)
}

fn parse_refno(text: &str) -> Option<RefNo> {
    let (word0, word1) = text.split_once('/')?;
    Some(RefNo::new(
        word0.trim().parse().ok()?,
        word1.trim().parse::<i32>().ok()? as u32,
    ))
}

fn buckets(rows: &[Row]) -> BTreeSet<(String, String)> {
    rows.iter()
        .map(|row| (row.noun.clone(), row.layout.clone()))
        .collect()
}

fn nouns(rows: &[Row]) -> BTreeSet<String> {
    rows.iter().map(|row| row.noun.clone()).collect()
}

fn excused() -> BTreeSet<(String, String)> {
    UNWITNESSED
        .iter()
        .map(|(noun, layout)| (noun.to_string(), layout.to_string()))
        .collect()
}

/// The layout a record declares, or `None` when it is too short to declare one.
fn layout_of(parsed: &ParsedElement) -> Option<&'static str> {
    let at = FORMAT_WORD_INDEX * 4;
    let word = u32::from_be_bytes(parsed.implicit_data.get(at..at + 4)?.try_into().ok()?);
    Some(match NounTemplate::layout_from_format_word(word) {
        StorageLayout::Compact => "compact",
        StorageLayout::Wide => "wide",
    })
}

/// Criterion 1: the list describes elements this session can look up.
#[test]
fn every_sampled_element_resolves_to_itself_in_the_shape_recorded() {
    if common::missing(FIXTURES) {
        return;
    }
    let Some(rows) = read_list(SAMPLE) else {
        return;
    };

    let mut by_db: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
    for row in &rows {
        by_db.entry(&row.db).or_default().push(row);
    }

    let mut checked = 0usize;
    let mut problems: Vec<String> = Vec::new();

    for (db, rows) in &by_db {
        let path = Path::new(AMS).join(db);
        // Not a skip: a database the sample names and the machine does not have
        // is a row this gate would pass without checking anything.
        assert!(
            path.exists(),
            "{db} is named by {SAMPLE} but not installed at {}",
            path.display()
        );
        let mut engine = ReadOnlyEngine::open(&path).expect("open database");
        println!(
            "{db}: session {}, {} rows",
            engine.session().sesno,
            rows.len()
        );

        for row in rows {
            checked += 1;
            let found = match engine.find_element(row.refno) {
                Ok(found) => found,
                Err(error) => {
                    problems.push(format!("{db} {} {}: {error}", row.noun, row.refno));
                    continue;
                }
            };
            let Some(view) = found else {
                problems.push(format!(
                    "{db} {} {}: the session's own index does not name it",
                    row.noun, row.refno
                ));
                continue;
            };
            let parsed = match ParsedElement::parse(&view.raw_bytes) {
                Ok(parsed) => parsed,
                Err(error) => {
                    problems.push(format!(
                        "{db} {} {}: the record does not parse: {error}",
                        row.noun, row.refno
                    ));
                    continue;
                }
            };
            if parsed.refno != row.refno {
                problems.push(format!(
                    "{db} {} {}: the lookup answered with {}",
                    row.noun, row.refno, parsed.refno
                ));
                continue;
            }
            let noun = db1_dehash(parsed.noun_hash).unwrap_or_else(|| {
                format!("hash {:#010X} outside the DB1 alphabet", parsed.noun_hash)
            });
            if noun != row.noun {
                problems.push(format!(
                    "{db} {} {}: the record is a {noun}",
                    row.noun, row.refno
                ));
            }
            match layout_of(&parsed) {
                Some(layout) if layout == row.layout => {}
                Some(layout) => problems.push(format!(
                    "{db} {} {}: recorded {}, stored {layout}",
                    row.noun, row.refno, row.layout
                )),
                None => problems.push(format!(
                    "{db} {} {}: {} implicit words, too few to carry a format word",
                    row.noun, row.refno, parsed.impl_len_words
                )),
            }
            if parsed.impl_len_words != row.implicit_words {
                problems.push(format!(
                    "{db} {} {}: {} implicit words, recorded {}",
                    row.noun, row.refno, parsed.impl_len_words, row.implicit_words
                ));
            }
        }
    }

    println!("elements checked: {checked}");
    assert_eq!(checked, rows.len(), "every row must be checked");
    assert!(
        problems.is_empty(),
        "a row drawn from the live tree records the copy the session answers with, so there is \
         no such thing as an expected disagreement here:\n  {}",
        problems.join("\n  ")
    );
}

/// Criterion 2: the sample gives up a bucket only where none exists.
#[test]
fn the_sample_covers_every_bucket_the_old_list_did_that_a_live_tree_can_witness() {
    let (Some(new), Some(old)) = (read_list(SAMPLE), read_list(OLD_SAMPLE)) else {
        return;
    };
    let (new_buckets, old_buckets) = (buckets(&new), buckets(&old));
    let (new_nouns, old_nouns) = (nouns(&new), nouns(&old));
    let excused = excused();

    println!(
        "old: {} rows, {} nouns, {} buckets",
        old.len(),
        old_nouns.len(),
        old_buckets.len()
    );
    println!(
        "new: {} rows, {} nouns, {} buckets",
        new.len(),
        new_nouns.len(),
        new_buckets.len()
    );

    let stale: Vec<&(String, String)> = excused.difference(&old_buckets).collect();
    assert!(
        stale.is_empty(),
        "an excused bucket the old list does not have excuses nothing: {stale:?}"
    );

    let missing: BTreeSet<(String, String)> =
        old_buckets.difference(&new_buckets).cloned().collect();
    assert_eq!(
        missing, excused,
        "every bucket the new list drops has to be one no live tree holds, and every bucket \
         listed as unwitnessed has to actually be missing"
    );

    let lost: BTreeSet<String> = old_nouns.difference(&new_nouns).cloned().collect();
    let expected: BTreeSet<String> = LOST_NOUNS.iter().map(|noun| noun.to_string()).collect();
    assert_eq!(
        lost, expected,
        "the new list carries every noun the old one did, bar the ones with no live element"
    );
}

/// Criterion 3: the compact column is populated.
///
/// `evidence/20260826-record-layout-census` measured `ams7000_0001` at 41.5%
/// compact records, and every element verified against TTY so far has been
/// wide. A sample that lost the compact column would still look healthy and
/// would test half of what it claims to.
#[test]
fn both_storage_layouts_are_covered() {
    let Some(rows) = read_list(SAMPLE) else {
        return;
    };
    let compact = rows.iter().filter(|row| row.layout == "compact").count();
    let wide = rows.iter().filter(|row| row.layout == "wide").count();
    println!("{compact} compact rows, {wide} wide rows");
    assert_eq!(
        compact + wide,
        rows.len(),
        "every row's layout reads as compact or wide"
    );
    assert!(compact > 0, "the sample has lost its compact column");
    assert!(wide > 0, "the sample has lost its wide column");
}

/// Criterion 2, the half a file comparison cannot check: that [`UNWITNESSED`]
/// names buckets the live trees really are empty of.
///
/// Walks the three libraries the first pass draws from and reads every record
/// they index — 312 089 of them, a few seconds in release and rather longer
/// without it. The sampler makes the wider claim, that no library in the
/// project holds one either, by doing this over all 443.
#[test]
fn no_live_element_in_the_core_libraries_fills_a_bucket_called_unwitnessed() {
    if common::missing(FIXTURES) {
        return;
    }
    let excused = excused();
    let mut seen = 0usize;
    let mut filled: Vec<String> = Vec::new();

    for db in CORE_DBS {
        let path = Path::new(AMS).join(db);
        assert!(path.exists(), "{db} is not installed at {}", path.display());
        let mut view = DbView::open(&path).expect("open database");
        let entries: Vec<_> = IndexCursor::new(&mut view)
            .walk()
            .map(|found| found.map(|found| found.entry()))
            .collect::<Result<Vec<_>, _>>()
            .expect("the session's tree walks");
        let root_ext = view.index_root().ext;
        let pages = view.pages_mut();

        for entry in &entries {
            let record = read_element_record(pages, root_ext, entry).expect("read the record");
            let parsed = ParsedElement::parse(&record.raw_bytes).expect("parse the record");
            let (Some(noun), Some(layout)) = (db1_dehash(parsed.noun_hash), layout_of(&parsed))
            else {
                continue;
            };
            seen += 1;
            let bucket = (noun, layout.to_string());
            if excused.contains(&bucket) {
                filled.push(format!("{db} {} {} {}", bucket.0, bucket.1, parsed.refno));
            }
        }
    }

    println!("live records examined: {seen}");
    // A walk that returned nothing would leave `filled` empty and pass, which
    // is the one way this gate could lie about what it checked.
    assert!(
        seen > 300_000,
        "expected the three libraries to hold more than 300 000 live records, saw {seen}"
    );
    assert!(
        filled.is_empty(),
        "a bucket listed as unwitnessed has a live element after all, so it belongs in the \
         sample rather than in the exception list:\n  {}",
        filled
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
