#![allow(deprecated)] // exercises the ATGTDF-position path on purpose

//! Slice 4 step 14 — multi-noun coverage analysis for `summarize_element`.
//!
//! Scans the first N indexed RefNos on the fixture, groups one sample
//! element per noun_hash, runs `summarize_element` on each, and prints
//! a per-noun table of:
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
use e3d_io::index::IndexPageView;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001";
const ATTLIB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\attlib.dat";

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

    // Sample a window of RefNos to keep the test fast. We avoid the
    // full scan_all_indexed_refnos (slow on 9304-page fixtures) and
    // instead probe a small set of plausible RefNos around known
    // hot ones (17496/9621 NXTR, 17496/924 STWALL from prior steps),
    // plus a few neighbours, to surface the noun diversity that's
    // actually reachable via the engine.find_element fast path.
    use e3d_io::refno::RefNo;
    let probe_set: Vec<RefNo> = {
        let mut v = Vec::new();
        // Known hot RefNos from earlier sessions.
        v.push(RefNo::new(17496, 9621));
        v.push(RefNo::new(17496, 924));
        v.push(RefNo::new(17496, 13591));
        // Probe nearby ranges to catch a wider noun spread.
        for w in [1, 100, 1000, 5000, 10000, 12000, 15000, 17000] {
            v.push(RefNo::new(17496, w));
        }
        v
    };

    let mut samples_per_noun: HashMap<u32, RefNo> = HashMap::new();
    let mut refno_count_per_noun: HashMap<u32, usize> = HashMap::new();
    let mut probed = 0usize;
    for refno in &probe_set {
        probed += 1;
        let view = match engine.find_element(*refno) {
            Ok(Some(v)) => v,
            _ => continue,
        };
        let parsed = match e3d_io::record::element::ParsedElement::parse(&view.raw_bytes) {
            Ok(p) => p,
            Err(_) => continue,
        };
        samples_per_noun.entry(parsed.noun_hash).or_insert(*refno);
        *refno_count_per_noun.entry(parsed.noun_hash).or_insert(0) += 1;
    }
    let all_refnos_count = probed;
    // Keep IndexPageView import alive for future expansion.
    let _ = IndexPageView::parse;

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
