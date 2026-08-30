//! Slice 4 step 7 — high-level `decode_attribute_by_hash` API on fixture.
//!
//! Exercises the new ergonomic entry point that combines attlib lookup
//! + EXMAP dispatch + SYNO chain fallback in one call, and reports how
//! many attributes on a real fixture element resolve directly vs via
//! the SYNO chain pointer.

use std::path::Path;

use e3d_attlib::AttlibData;
use e3d_io::record::atnlog::{
    self, decode_attribute_by_hash, AttrValue, LookupError, DEFAULT_HEADER_WORDS, SYNO_CHAIN_HASH,
};
use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001";
const ATTLIB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\attlib.dat";

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

/// The first element the current session indexes.
///
/// This used to parse the index root as though it were a leaf and take its
/// first entry, which only worked while the root discovery happened to return
/// a one-level tree.
fn first_indexed_refno(engine: &mut ReadOnlyEngine) -> RefNo {
    engine.indexed_refnos().expect("read the index")[0].0
}

#[test]
fn high_level_decode_on_fixture_root_element() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present");
        return;
    }

    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");

    let target = first_indexed_refno(&mut engine);

    let view = engine.find_element(target).unwrap().expect("element");
    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse element");

    println!("\n=== Slice 4 step 7 — high-level decode_attribute_by_hash ===");
    println!(
        "[element] refno={} noun_hash=0x{:08X} impl_len_words={}",
        parsed.refno, parsed.noun_hash, parsed.impl_len_words
    );

    // Sample: try the first 20 ATGTDF hashes through the high-level API
    // (without and with SYNO fallback), tally each path.
    let mut direct_ok = 0u32;
    let mut direct_zero = 0u32;
    let mut syno_resolved = 0u32;
    let mut lookup_err = 0u32;
    let mut not_in_attlib = 0u32;

    // Use the same attribute set the low-level test walked: first ~20
    // ATGTDF entries.
    let samples: Vec<u32> = attlib.attr_defs.iter().take(20).map(|d| d.hash).collect();

    println!(
        "\n  {:>10}  {:<22} {:<24} {:<22}",
        "hash", "direct (no SYNO)", "with SYNO fallback", "name"
    );
    for hash in &samples {
        let name = e3d_attlib::lookup_system_name(*hash)
            .map(String::from)
            .or_else(|| e3d_attlib::db1_dehash(*hash))
            .unwrap_or_else(|| String::from("(no name)"));
        let direct =
            match decode_attribute_by_hash(&parsed, &attlib, *hash, false, DEFAULT_HEADER_WORDS) {
                Ok(Some(v)) => {
                    let is_zero = match &v {
                        AttrValue::Int(0) => true,
                        AttrValue::Bool(false) => true,
                        AttrValue::Real(r) => *r == 0.0,
                        AttrValue::Text(0) => true,
                        AttrValue::Raw(0) => true,
                        _ => false,
                    };
                    if is_zero {
                        direct_zero += 1;
                        "zero".to_string()
                    } else {
                        direct_ok += 1;
                        format!("{:?}", v)
                    }
                }
                Ok(None) => {
                    direct_zero += 1;
                    "none".to_string()
                }
                Err(LookupError::NotInAttlib(_)) => {
                    not_in_attlib += 1;
                    "not-in-attlib".to_string()
                }
                Err(e) => {
                    lookup_err += 1;
                    format!("err: {:?}", e)
                }
            };
        let with_syno =
            match decode_attribute_by_hash(&parsed, &attlib, *hash, true, DEFAULT_HEADER_WORDS) {
                Ok(Some(AttrValue::DbRef(r))) => {
                    // If we got a DbRef back, it could be either the original
                    // attribute being a DbRef (system attr) or the SYNO chain
                    // pointer surfaced. We tally the latter case heuristically:
                    // if direct was "zero" then this is a chain resolve.
                    if direct == "zero" {
                        syno_resolved += 1;
                        format!("SYNO->{}", r)
                    } else {
                        format!("Ref({})", r)
                    }
                }
                Ok(Some(v)) => format!("{:?}", v),
                Ok(None) => "none".to_string(),
                Err(_) => "err".to_string(),
            };
        let direct_short: String = direct.chars().take(22).collect();
        let with_syno_short: String = with_syno.chars().take(24).collect();
        println!(
            "  0x{:08X}  {:<22} {:<24} {:<22}",
            hash, direct_short, with_syno_short, name
        );
    }

    // The SYNO attribute itself should exist in the attlib (system table).
    let syno_present = attlib.attr_def(SYNO_CHAIN_HASH).is_some();
    println!(
        "\n[summary] direct_ok={direct_ok} direct_zero={direct_zero} \
         syno_resolved={syno_resolved} not_in_attlib={not_in_attlib} lookup_err={lookup_err}"
    );
    println!(
        "[syno] SYNO_CHAIN_HASH 0x{:08X} present in attlib ATGTDF = {}",
        SYNO_CHAIN_HASH, syno_present
    );

    // Soft assertion: we want at least ONE attribute to actually decode.
    assert!(
        direct_ok + syno_resolved >= 1,
        "expected at least 1 decoded attribute, got direct_ok={direct_ok} syno_resolved={syno_resolved}"
    );
}

#[test]
fn lookup_unknown_hash_returns_not_in_attlib() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present");
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");
    let target = first_indexed_refno(&mut engine);
    let view = engine.find_element(target).unwrap().expect("element");
    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse element");
    // 0x99999999 is in the system hash range but is statistically not in
    // the fixture's attlib ATGTDF table (which has ~150 entries).
    let err = decode_attribute_by_hash(&parsed, &attlib, 0x99999999, false, DEFAULT_HEADER_WORDS);
    assert!(
        matches!(err, Err(LookupError::NotInAttlib(0x99999999))),
        "expected NotInAttlib, got {err:?}"
    );
    // Silence unused
    let _ = atnlog::DEFAULT_HEADER_WORDS;
}
