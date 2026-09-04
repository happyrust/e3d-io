//! Exploratory test for Slice 4 step 2 — inventory which nouns in
//! `attlib.dat` have full schema coverage in the pre-rewrite ATGTDF table.
//!
//! Output guides the choice of a fixture element with a fully-decoded schema
//! so step 2 can attempt real byte decoding.

use std::path::Path;

use e3d_attlib::{db1_hash, lookup_system_name, AttlibData};
use e3d_io::record::element::ParsedElement;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = common::TEST_FILE_DB;
const ATTLIB_PATH: &str = common::TEST_FILE_ATTLIB;

mod common;

#[test]
fn report_attlib_coverage_and_find_decodable_noun() {
    if !Path::new(DB_PATH).exists() || !Path::new(ATTLIB_PATH).exists() {
        eprintln!("SKIP: fixtures missing");
        return;
    }

    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("attlib parse");

    println!(
        "\n[attlib] attributes={} nouns={} attr_def_map={}",
        attlib.attributes.len(),
        attlib.noun_attr_map.len(),
        attlib.attr_def_map.len()
    );

    println!(
        "\n=== All {} attr_def_map entries ===",
        attlib.attr_def_map.len()
    );
    let mut defs: Vec<_> = attlib.attr_def_map.values().collect();
    defs.sort_by_key(|d| d.hash);
    for d in &defs {
        let name = lookup_system_name(d.hash).unwrap_or("(no system name)");
        println!(
            "  hash=0x{:08X} {:<8} data_type={} kind={} size={}",
            d.hash, name, d.data_type, d.kind, d.size
        );
    }

    // For each noun, count how many of its attributes have attr_def entries.
    let mut covered_nouns: Vec<(u32, String, usize, usize)> = Vec::new();
    for (noun_hash, attr_idxs) in &attlib.noun_attr_map {
        let mut total = 0;
        let mut covered = 0;
        for &ai in attr_idxs {
            if let Some(attr) = attlib.attributes.get(ai as usize) {
                total += 1;
                if attlib.attr_def_map.contains_key(&db1_hash(&attr.name)) {
                    covered += 1;
                }
            }
        }
        if covered > 0 {
            let noun_name =
                e3d_attlib::db1_dehash(*noun_hash).unwrap_or_else(|| format!("0x{noun_hash:08X}"));
            covered_nouns.push((*noun_hash, noun_name, covered, total));
        }
    }
    covered_nouns.sort_by(|a, b| b.2.cmp(&a.2));

    println!(
        "\n=== Nouns with ANY attr_def coverage ({}) — top 15 ===",
        covered_nouns.len()
    );
    for (h, name, cov, total) in covered_nouns.iter().take(15) {
        println!(
            "  noun=0x{:08X} {:<8} covered={:>3}/{:<3}",
            h, name, cov, total
        );
    }

    // Now scan the DB for elements whose noun_hash matches one of the
    // covered nouns and report the first hit per noun.
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");
    let refnos = engine.indexed_refnos().expect("scan all refnos");

    println!("\n[db] {} indexed refnos", refnos.len());

    let mut samples: std::collections::HashMap<u32, (e3d_io::refno::RefNo, u32, u32)> =
        Default::default();
    let mut scanned = 0usize;
    for (refno, page, off) in refnos.iter() {
        if scanned >= 5000 {
            break; // safety cap
        }
        scanned += 1;
        if let Ok(view) = engine.read_element_direct(*refno, *page, *off) {
            if let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) {
                if covered_nouns
                    .iter()
                    .any(|(h, _, _, _)| *h == parsed.noun_hash)
                    && !samples.contains_key(&parsed.noun_hash)
                {
                    samples.insert(parsed.noun_hash, (*refno, *page, *off));
                }
            }
        }
    }

    println!(
        "\n=== Sample fixture elements per covered noun (scanned {} of {}) ===",
        scanned,
        refnos.len()
    );
    for (h, name, cov, total) in covered_nouns.iter().take(15) {
        if let Some((refno, page, off)) = samples.get(h) {
            println!(
                "  noun=0x{:08X} {:<8} covered={:>3}/{:<3} -> sample refno={} page={} off={}",
                h, name, cov, total, refno, page, off
            );
        }
    }

    assert!(
        !attlib.attr_def_map.is_empty(),
        "attlib must yield at least some defs"
    );
}
