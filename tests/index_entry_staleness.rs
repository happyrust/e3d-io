//! How often does a B+tree leaf entry address a record that is no longer the
//! element it claims?
//!
//! `find_element` used to return whatever the first matching leaf pointed at.
//! That is only safe if leaf entries never outlive their record. They do: in
//! `ams1112_0001`, RefNo 17496/950 is indexed from eleven leaves and ten of
//! them address a location that now holds an unrelated element, so the lookup
//! answered with someone else's data. The failure is normally silent — it
//! surfaced only because that particular location parses to noun hash 0, which
//! has no template.
//!
//! This test measures the rate over the elements sampled for TTY comparison.
//!
//! Run with:
//! `cargo test --test index_entry_staleness -- --ignored --nocapture`

use std::collections::BTreeMap;
use std::path::Path;

use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const SAMPLE: &str = "evidence/20260826-tty-sampling/sample.tsv";
const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

mod common;

const FIXTURES: &[&str] = &[AMS];

fn parse_refno(text: &str) -> Option<RefNo> {
    let (w0, w1) = text.split_once('/')?;
    Some(RefNo::new(w0.trim().parse().ok()?, w1.trim().parse().ok()?))
}

/// Outcome of believing the first leaf entry that matches, as `find_element`
/// used to. The three cases differ in how loudly they go wrong.
#[derive(Debug, PartialEq, Eq)]
enum FirstHit {
    /// The record confirms the RefNo. The answer was right.
    Confirms,
    /// The location holds a different, readable element. The old lookup
    /// returned it, and every attribute decoded from it was attributed to the
    /// RefNo that was asked for. Silent.
    OtherElement,
    /// The location cannot be read or parsed. The old lookup raised an error.
    Unreadable,
    /// No leaf entry matched at all; the record scan was the only path.
    NoEntry,
}

fn first_indexed_hit(engine: &mut ReadOnlyEngine, refno: RefNo) -> FirstHit {
    let Ok(roots) = engine.find_all_index_roots() else {
        return FirstHit::NoEntry;
    };
    for (root_page, _) in roots {
        let Ok(Some(result)) = engine.search_index(root_page, refno) else {
            continue;
        };
        let Ok(view) =
            engine.read_element_direct(refno, result.entry.page_no, result.entry.offset_words)
        else {
            return FirstHit::Unreadable;
        };
        return match ParsedElement::parse(&view.raw_bytes) {
            Ok(parsed) if parsed.refno == refno => FirstHit::Confirms,
            Ok(_) => FirstHit::OtherElement,
            Err(_) => FirstHit::Unreadable,
        };
    }
    FirstHit::NoEntry
}

#[test]
fn stale_leaf_entries_are_not_believed() {
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
    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();
    let mut recovered = 0usize;
    let mut silently_wrong: Vec<String> = Vec::new();
    let mut still_missing: Vec<String> = Vec::new();
    let mut wrong_words: Vec<String> = Vec::new();

    for (db, rows) in &by_db {
        let path = Path::new(AMS).join(db);
        if !path.exists() {
            eprintln!("SKIP {db}: not present");
            continue;
        }
        let mut engine = ReadOnlyEngine::open(&path).expect("open database");

        for (noun, expected_words, refno) in rows {
            checked += 1;

            let first_hit = first_indexed_hit(&mut engine, *refno);
            *outcomes.entry(format!("{first_hit:?}")).or_default() += 1;
            if first_hit == FirstHit::OtherElement {
                silently_wrong.push(format!("{db} {noun} {refno}"));
            }
            let was_stale = first_hit != FirstHit::Confirms;

            match engine.find_element(*refno).expect("lookup") {
                Some(view) => {
                    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse");
                    assert_eq!(
                        parsed.refno, *refno,
                        "{db} {noun}: lookup returned a different element"
                    );
                    if parsed.impl_len_words != *expected_words {
                        wrong_words.push(format!(
                            "{db} {noun} {refno}: {} words, sample recorded {expected_words}",
                            parsed.impl_len_words
                        ));
                    }
                    if was_stale {
                        recovered += 1;
                    }
                }
                None => still_missing.push(format!("{db} {noun} {refno}")),
            }
        }
    }

    let stale_first_hit = checked - outcomes.get("Confirms").copied().unwrap_or(0);
    println!("elements checked: {checked}");
    println!("first indexed hit: {outcomes:?}");
    println!("not confirmed by the record: {stale_first_hit}");
    println!(
        "  of those, answered with an unrelated element: {} (silent)",
        silently_wrong.len()
    );
    for w in silently_wrong.iter().take(10) {
        println!("    {w}");
    }
    println!("  recovered by verifying and falling back: {recovered}");
    println!("elements still not found: {}", still_missing.len());
    for m in still_missing.iter().take(10) {
        println!("  {m}");
    }
    println!("implicit word count disagreements: {}", wrong_words.len());
    for w in wrong_words.iter().take(10) {
        println!("  {w}");
    }

    assert!(checked > 0, "expected to check at least one element");
    assert!(
        still_missing.is_empty(),
        "every sampled element should resolve"
    );
    assert!(
        wrong_words.is_empty(),
        "resolved elements should match the sampled record shape"
    );
    assert_eq!(
        stale_first_hit, recovered,
        "every stale first hit should be recovered"
    );
}
