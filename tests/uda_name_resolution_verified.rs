#![allow(deprecated)] // exercises the ATGTDF-position path on purpose

//! Slice 4 step 12 — verify that UDA name resolution in
//! `engine.summarize_element` already produces human-readable names for
//! every UDA-source attribute that the current fixture exposes, **without
//! any new code**.
//!
//! Rationale:
//! * progress.jsonl entry #13/#14/#16 stated the original intent of
//!   "follow UDA-ATGTIX page-offset to read a length-prefixed UDA name
//!   string" for attributes with `hash > 0x171FAD39`.
//! * In the current fixture, **zero** ATGTIX entries have a hash above
//!   `0x171FAD39`; all 6644 entries are within base-27 dehash range and
//!   `db1_dehash` already produces readable short names.
//! * `parser::resolve_attr_name` and `engine::summarize_element` both
//!   already chain `system_names → db1_dehash → hex` fallback. The
//!   acceptance criterion "at least one UDA slot output has a readable
//!   name (not `0x????`)" is therefore already met for every UDA on this
//!   fixture; this test pins that property so future regressions are
//!   caught immediately.
//!
//! See `goals/e3d31-attribute-parsing/NEXT_SESSION_PLAN.md` for the
//! restated scope of the original Prompt #2 and the carved-out
//! "ATGTIX-pointed payload decoding" follow-on (#2b).

use std::path::Path;

use e3d_attlib::parser::AttrSource;
use e3d_attlib::AttlibData;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001";
const ATTLIB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\attlib.dat";

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

/// Sanity property: every ATGTIX entry from the attlib has a hash that
/// `db1_dehash` can resolve, so no hash falls to the `0x????` hex
/// fallback. This is the root reason Prompt #2 is implicitly satisfied
/// on the current fixture.
#[test]
fn every_atgtix_hash_resolves_to_readable_name() {
    if !fixtures_present() {
        eprintln!("SKIP: no attlib fixture at {ATTLIB_PATH}");
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB_PATH).unwrap();

    let total = attlib.atgtix_entries.len();
    let mut by_system_table = 0usize;
    let mut by_dehash_only = 0usize;
    let mut hex_fallback = 0usize;
    let mut uda_source_resolved = 0usize;
    let mut uda_source_total = 0usize;

    for attr in &attlib.attributes {
        if matches!(attr.source, AttrSource::Uda) {
            uda_source_total += 1;
            if !attr.name.is_empty() && !attr.name.starts_with("0x") {
                uda_source_resolved += 1;
            }
        }
        let resolved_by_system = e3d_attlib::lookup_system_name(attr.id).is_some();
        let resolved_by_dehash = !resolved_by_system && e3d_attlib::db1_dehash(attr.id).is_some();
        let hex_only = !resolved_by_system && !resolved_by_dehash;
        if resolved_by_system {
            by_system_table += 1;
        } else if resolved_by_dehash {
            by_dehash_only += 1;
        }
        if hex_only {
            hex_fallback += 1;
        }
    }

    println!("\n=== ATGTIX name-resolution coverage ===");
    println!("total entries: {total}");
    println!("system_names hit: {by_system_table}");
    println!("db1_dehash-only hit: {by_dehash_only}");
    println!("hex fallback (no name): {hex_fallback}");
    println!("UDA-source records: {uda_source_total} resolved={uda_source_resolved}");

    assert_eq!(
        hex_fallback, 0,
        "every ATGTIX hash must resolve to a name via system_names or db1_dehash; \
         got {hex_fallback} that fell to 0x???? hex fallback"
    );
    assert_eq!(
        uda_source_resolved, uda_source_total,
        "every UDA-source AttributeRecord must have a non-empty / non-hex name; \
         got {uda_source_resolved}/{uda_source_total} resolved"
    );
}

/// End-to-end: `summarize_element` on the NXTR fixture element returns
/// attribute names that are all human-readable (no `0x????` fallback).
#[test]
fn summarize_element_emits_no_hex_fallback_names() {
    if !fixtures_present() {
        eprintln!("SKIP: no fixture");
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB_PATH).unwrap();
    let mut engine = ReadOnlyEngine::open(DB_PATH).unwrap();

    let target = engine.indexed_refnos().unwrap()[0].0;

    let summary = engine.summarize_element(target, &attlib, 3).unwrap();

    let mut hex_names = 0usize;
    for (name, _) in &summary.attributes {
        if name.starts_with("0x") {
            hex_names += 1;
            eprintln!("[regression] name fell through to hex fallback: {name}");
        }
    }

    println!(
        "\n=== summarize_element({}) ===\n{} non-zero attrs decoded; {} hex-fallback names",
        target,
        summary.attributes.len(),
        hex_names
    );
    assert_eq!(
        hex_names, 0,
        "summarize_element output must not contain any 0x???? hex-fallback name"
    );
}
