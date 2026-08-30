//! Every element sampled for the TTY comparison resolves, and resolves to
//! itself.
//!
//! This file used to measure how often the first indexed hit was somebody
//! else's record. On `ams1112_0001`, RefNo 17496/950 was indexed from eleven
//! leaves and ten of them addressed an unrelated element, so `find_element`
//! carried three fallbacks: search every tree in the file, keep the copy
//! furthest into it, and failing that read the whole file backwards looking
//! for the RefNo as a byte pattern.
//!
//! Ten of those eleven leaves are in trees no session can reach. The index is
//! copy-on-write, so every session leaves its tree behind, and a rule that
//! collected "index pages no other index page names" collected all of them.
//! Descending from the session's own root, the count is 211 of 211 confirmed
//! and no fallback fires — measured over the whole corpus in
//! `tests/record_l3_contract.rs`, it is 789 831 of 789 831.
//!
//! What is left worth checking is the sample itself: each row records the
//! element's noun, storage layout and implicit word count as they were on
//! 2026-08-26, so it is a shape regression over 211 real elements.
//!
//! Run with:
//! `cargo test --test sampled_elements_resolve -- --nocapture`

use std::collections::BTreeMap;
use std::path::Path;

use e3d_io::index::IndexCursor;
use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::session::{DbView, SessionSelector};
use e3d_io::ReadOnlyEngine;

const SAMPLE: &str = "evidence/20260826-tty-sampling/sample.tsv";
const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

mod common;

const FIXTURES: &[&str] = &[AMS];

/// Rows whose recorded word count belongs to a copy the current session has
/// superseded.
///
/// The sample was taken with a reader that answered from the first tree in the
/// file that confirmed the RefNo, which is the oldest copy of the element.
/// `ams7000_0001` holds two copies of SPEC 15192/62134: page 25683 declares 24
/// implicit words, which is what the sample recorded, and page 48248 declares
/// 26 — and page 48248 is the one the session root names.
///
/// Left as an exception rather than edited into the sample, which is dated
/// evidence, and rather than dropped, which would stop the row checking
/// anything at all.
const SUPERSEDED: &[(&str, u32, i32, i32)] = &[
    // db, refno.word1, what the sample recorded, what the session says
    ("ams7000_0001", 62_134, 24, 26),
    ("ams7000_0001", 521, 25, 26),
    ("ams7000_0001", 522, 15, 17),
    ("ams7000_0001", 222_761, 13, 15),
];

fn parse_refno(text: &str) -> Option<RefNo> {
    let (w0, w1) = text.split_once('/')?;
    Some(RefNo::new(w0.trim().parse().ok()?, w1.trim().parse().ok()?))
}

/// The newest session that indexes `refno`, or `None` if none does.
///
/// The one thing that separates "this element was deleted" from "the descent
/// is broken": a deleted element is still indexed by the session it was
/// deleted in the wake of, because the index is copy-on-write and that tree is
/// still on disk. An element no session indexes is neither.
fn last_session_indexing(path: &Path, refno: RefNo) -> Option<u32> {
    let mut view = DbView::open(path).expect("open database");
    let sessions = view.walk_sessions().expect("the session chain walks");
    sessions.iter().find_map(|session| {
        let mut view =
            DbView::open_at(path, SessionSelector::Exact(session.sesno)).expect("open at session");
        IndexCursor::new(&mut view)
            .seek(refno)
            .ok()
            .and_then(|hit| hit.found)
            .map(|_| session.sesno)
    })
}

fn expected_words(db: &str, refno: RefNo, sampled: i32) -> i32 {
    SUPERSEDED
        .iter()
        .find(|(sample_db, word1, recorded, _)| {
            *sample_db == db && *word1 == refno.word1 && *recorded == sampled
        })
        .map_or(sampled, |(_, _, _, current)| *current)
}

#[test]
fn every_sampled_element_resolves_to_itself_in_the_shape_recorded() {
    if common::missing(FIXTURES) {
        return;
    }
    let Ok(sample) = std::fs::read_to_string(SAMPLE) else {
        eprintln!("SKIP: {SAMPLE} not present");
        return;
    };

    // db -> [(noun, expected implicit words, refno)]
    let mut by_db: BTreeMap<String, Vec<(String, i32, RefNo)>> = BTreeMap::new();
    for line in sample.lines().filter(|l| !l.starts_with('#')) {
        let cols: Vec<&str> = line.split('\t').collect();
        let [db, noun, _layout, words, refno] = cols[..] else {
            continue;
        };
        let (Ok(words), Some(refno)) = (words.parse::<i32>(), parse_refno(refno)) else {
            continue;
        };
        by_db
            .entry(db.to_string())
            .or_default()
            .push((noun.to_string(), words, refno));
    }
    if by_db.is_empty() {
        eprintln!("SKIP: no usable rows in {SAMPLE}");
        return;
    }

    let mut checked = 0usize;
    let mut superseded = 0usize;
    let mut deleted: Vec<String> = Vec::new();
    let mut unaccounted: Vec<String> = Vec::new();
    let mut wrong_shape: Vec<String> = Vec::new();

    for (db, rows) in &by_db {
        let path = Path::new(AMS).join(db);
        if !path.exists() {
            eprintln!("SKIP {db}: not present");
            continue;
        }
        let mut engine = ReadOnlyEngine::open(&path).expect("open database");
        let current = engine.session().sesno;
        println!("{db}: session {current}");

        for (noun, sampled, refno) in rows {
            checked += 1;
            let Some(view) = engine.find_element(*refno).expect("lookup") else {
                // The sample was picked from the leaf entries of every index
                // page in the file, so it caught elements that had already
                // been deleted — their trees are still on disk. Which session
                // last indexed one tells the two cases apart.
                match last_session_indexing(&path, *refno) {
                    Some(sesno) => {
                        deleted.push(format!("{db} {noun} {refno}: last in session {sesno}"))
                    }
                    None => unaccounted.push(format!("{db} {noun} {refno}")),
                }
                continue;
            };
            let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse");
            assert_eq!(
                parsed.refno, *refno,
                "{db} {noun}: the lookup answered with a different element"
            );

            let expected = expected_words(db, *refno, *sampled);
            if expected != *sampled {
                superseded += 1;
            }
            if parsed.impl_len_words != expected {
                wrong_shape.push(format!(
                    "{db} {noun} {refno}: {} implicit words, expected {expected}",
                    parsed.impl_len_words
                ));
            }
        }
    }

    println!("elements checked: {checked}");
    println!("rows whose sampled copy is superseded: {superseded}");
    println!("rows for elements since deleted: {}", deleted.len());
    for line in &deleted {
        println!("  {line}");
    }
    for line in wrong_shape.iter().take(10) {
        println!("  shape: {line}");
    }
    for line in unaccounted.iter().take(10) {
        println!("  unaccounted: {line}");
    }

    assert!(checked > 0, "expected to check at least one element");
    assert!(
        unaccounted.is_empty(),
        "a sampled element that no session indexes is a lookup that has stopped working, \
         not an element that was deleted"
    );
    assert!(
        wrong_shape.is_empty(),
        "resolved elements should match the sampled record shape"
    );
    assert_eq!(
        superseded,
        SUPERSEDED.len(),
        "a listed exception that no longer fires is a row that can go back to \
         being checked against the sample"
    );
}
