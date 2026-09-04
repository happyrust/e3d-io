#![allow(deprecated)] // exercises the ATGTDF-position path on purpose

//! Slice 4 step 14 — multi-noun coverage analysis for `summarize_element`.
//!
//! Walks the fixture's index once, takes the first element of every
//! noun as its sample, runs `summarize_element` on each, and prints a
//! per-noun table of:
//!
//! * implicit word count
//! * non-zero decoded attributes
//! * readable-name ratio
//!
//! This gives a quantitative read on **how much value the future
//! noun-template loader (Followup #3) brings**: nouns where the current
//! global-ATGTDF-position path already produces a reasonable number of
//! readable values get less marginal benefit from a template-aware
//! decoder; nouns whose summaries are sparse / mostly hex names are
//! prime candidates for #3.
//!
//! The test does **not** assert specific numbers (the goal is
//! diagnostic, not regression); it only asserts the scan runs and
//! produces a non-empty per-noun table.
//!
//! Run with `--nocapture` to see the breakdown.

use std::collections::HashMap;
use std::path::Path;

use e3d_attlib::{db1_dehash, AttlibData};
use e3d_io::engine::ScanTier;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = common::TEST_FILE_DB;
const ATTLIB_PATH: &str = common::TEST_FILE_ATTLIB;

mod common;

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

#[test]
fn report_per_noun_summarize_quality() {
    if !fixtures_present() {
        eprintln!("SKIP: no fixture");
        return;
    }

    let attlib = AttlibData::parse_file(ATTLIB_PATH).unwrap();
    let mut engine = ReadOnlyEngine::open(DB_PATH).unwrap();

    // One sample per noun: the first element of it the index states, in
    // RefNo order, with the noun's whole population counted alongside.
    // The header tier reads one page per element, so the full database
    // goes by in under a second. This used to probe eleven hand-picked
    // RefNos instead, and every one named an element no fixture to hand
    // indexes — the table came out empty.
    let mut samples_per_noun: HashMap<u32, RefNo> = HashMap::new();
    let mut refno_count_per_noun: HashMap<u32, usize> = HashMap::new();
    let mut all_refnos_count = 0usize;
    for element in engine
        .scan_elements(ScanTier::Header)
        .expect("walk the index")
    {
        let element = element.expect("read an indexed element's header");
        all_refnos_count += 1;
        samples_per_noun
            .entry(element.noun_hash)
            .or_insert(element.refno);
        *refno_count_per_noun.entry(element.noun_hash).or_insert(0) += 1;
    }

    println!(
        "\n=== Per-noun coverage analysis ({} unique nouns from {} probed refnos) ===",
        samples_per_noun.len(),
        all_refnos_count
    );

    let mut rows: Vec<(String, u32, usize, u32, u32, u32, u32)> = Vec::new();
    for (&noun_hash, &refno) in &samples_per_noun {
        let count = *refno_count_per_noun.get(&noun_hash).unwrap_or(&0);
        let summary = match engine.summarize_element(refno, &attlib, 3) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let total = summary.attributes.len() as u32;
        let hex_names = summary
            .attributes
            .iter()
            .filter(|(n, _)| n.starts_with("0x"))
            .count() as u32;
        let readable_short = summary
            .attributes
            .iter()
            .filter(|(n, _)| !n.starts_with("0x") && n.len() <= 8)
            .count() as u32;
        let noun_name = summary
            .noun_name
            .or_else(|| db1_dehash(noun_hash))
            .unwrap_or_else(|| format!("0x{noun_hash:08X}"));
        rows.push((
            noun_name,
            noun_hash,
            count,
            summary.implicit_word_count,
            total,
            readable_short,
            hex_names,
        ));
    }

    // Sort by descending element count so the most populous nouns
    // surface first.
    rows.sort_by(|a, b| b.2.cmp(&a.2));

    println!(
        "{:<10} {:<12} {:>10} {:>6} {:>6} {:>9} {:>4}",
        "noun", "hash", "elem_count", "impl_w", "attrs", "readable", "hex"
    );
    for (noun_name, hash, count, implicit_words, total, readable, hex) in &rows {
        println!(
            "{:<10} 0x{:08X}  {:>9} {:>6} {:>6} {:>9} {:>4}",
            noun_name, hash, count, implicit_words, total, readable, hex
        );
    }

    let total_elem: usize = rows.iter().map(|r| r.2).sum();
    let total_attrs: u32 = rows.iter().map(|r| r.4).sum();
    let total_readable: u32 = rows.iter().map(|r| r.5).sum();
    let total_hex: u32 = rows.iter().map(|r| r.6).sum();
    let nouns_with_any_attrs = rows.iter().filter(|r| r.4 > 0).count();
    let nouns_zero_attrs = rows.iter().filter(|r| r.4 == 0).count();
    println!(
        "\nsummary: {} nouns sampled, {} produced at least 1 attr ({} produced 0); covered {}/{} elements; total attrs={} readable={} hex_fallback={}",
        rows.len(),
        nouns_with_any_attrs,
        nouns_zero_attrs,
        total_elem,
        all_refnos_count,
        total_attrs,
        total_readable,
        total_hex
    );

    assert!(
        !rows.is_empty(),
        "expected the scan to produce at least one noun-summary row"
    );
    assert_eq!(
        total_hex, 0,
        "no decoded attribute name should fall back to 0x???? hex form"
    );
}
