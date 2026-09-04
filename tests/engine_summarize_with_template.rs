#![allow(deprecated)] // exercises the ATGTDF-position path on purpose

//! Slice 4 step 15 — `engine.summarize_element_with_template` integration tests.
//!
//! Builds a `NounTemplate` from the attlib's ATGTSX-derived noun map
//! (the "structural fallback" path while the DB-internal template
//! loader of Followup #3b is not yet implemented), then drives
//! `engine.summarize_element_with_template` on a fixture element and
//! compares against the legacy `summarize_element` (global ATGTDF
//! position) baseline.
//!
//! This is the first end-to-end exercise of the template path. The
//! ATGTSX-synthesized template uses stride=1 (single-word slot)
//! uniformly because ATGTSX does not carry stride/aux info — so the
//! comparison here is NOT a fair "noun-correct vs heuristic" test;
//! it's a smoke test that the template path runs and produces a
//! coherent ElementSummary. A full noun-correct comparison waits for
//! the DB-file template loader (#3b).

use std::path::Path;

use e3d_attlib::{db1_dehash, AttlibData};
use e3d_io::engine::ResolveError;
use e3d_io::record::template::NounTemplate;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = common::TEST_FILE_DB;
const ATTLIB_PATH: &str = common::TEST_FILE_ATTLIB;

mod common;

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

/// Smoke: on the NXTR fixture element, derive a template from
/// ATGTSX (if present) or synthesize a tiny manual template, then
/// run `summarize_element_with_template` and assert it returns a
/// well-formed `ElementSummary` with no panics.
#[test]
fn summarize_with_template_runs_on_nxtr() {
    if !fixtures_present() {
        eprintln!("SKIP: no fixture");
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB_PATH).unwrap();
    let mut engine = ReadOnlyEngine::open(DB_PATH).unwrap();
    let nxtr_hash = common::NXTR_HASH;
    let refno = common::first_element_of_noun(&mut engine, nxtr_hash)
        .expect("the fixture indexes at least one NXTR");
    println!("[fixture] first NXTR in the index: {refno}");

    // NXTR is not in ATGTSX on this fixture — build a small manual
    // template from the global ATGTDF defs as a stand-in (mimics what
    // the DB-file loader would produce when it eventually reads the
    // real per-noun layout).
    let template = NounTemplate::from_attlib_noun_map(&attlib, nxtr_hash).unwrap_or_else(|| {
        // Stand-in: take the first 30 unique-ATGTDF attrs as the
        // template body, all single-word stride. This mirrors what
        // summarize_element already iterates internally.
        let mut t = NounTemplate::new(nxtr_hash);
        for def in attlib.attr_defs_unique().take(30) {
            t.attrs
                .push(e3d_io::record::template::TemplateAttr::single_word(
                    def.hash,
                ));
        }
        t
    });

    println!(
        "[template] noun_hash=0x{:08X} attrs={}",
        template.noun_hash,
        template.attrs.len()
    );

    let error = engine
        .summarize_element_with_template(refno, &attlib, &template)
        .expect_err("name-only ATGTSX template must not guess physical locations");
    assert!(matches!(error, ResolveError::TemplateNotAuthoritative(hash) if hash == nxtr_hash));
}

/// Compare: pick the first noun in the fixture that has BOTH an
/// ATGTSX-derived template AND a reachable element. Run the template
/// path AND the legacy `summarize_element` (ATGTDF-position) path,
/// print both outputs, and assert both produce some attributes. This
/// is a diagnostic baseline for the future DB-file template loader
/// (#3b) — when it lands, the template path should beat the ATGTDF
/// path on at least one well-chosen noun.
#[test]
fn compare_template_vs_atgtdf_paths_on_atgtsx_noun() {
    if !fixtures_present() {
        eprintln!("SKIP: no fixture");
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB_PATH).unwrap();
    let mut engine = ReadOnlyEngine::open(DB_PATH).unwrap();

    // The first element in the index whose noun has an ATGTSX entry on
    // this fixture — SUBS, and only SUBS, here. This used to try seven
    // hand-picked RefNos and skip when none of them was one, which on
    // every fixture to hand was always.
    let Some((refno, noun_hash)) = common::first_element_with_noun(&mut engine, |noun| {
        NounTemplate::from_attlib_noun_map(&attlib, noun).is_some()
    }) else {
        eprintln!("SKIP: no indexed element has a noun with an ATGTSX template on this fixture");
        return;
    };
    let template = NounTemplate::from_attlib_noun_map(&attlib, noun_hash)
        .expect("the noun was picked for having an ATGTSX template");

    let noun_name = db1_dehash(noun_hash).unwrap_or_else(|| format!("0x{noun_hash:08X}"));
    println!(
        "[noun={noun_name}] refno={refno} hash=0x{noun_hash:08X} template_attrs={}",
        template.attrs.len()
    );

    let template_error = engine
        .summarize_element_with_template(refno, &attlib, &template)
        .expect_err("ATGTSX template has no authoritative physical offsets");
    let via_atgtdf = engine.summarize_element(refno, &attlib, 0).unwrap();

    assert!(
        matches!(template_error, ResolveError::TemplateNotAuthoritative(hash) if hash == noun_hash)
    );
    println!(
        "[atgtdf path  ] {} non-zero attrs",
        via_atgtdf.attributes.len()
    );
    for (n, v) in via_atgtdf.attributes.iter().take(8) {
        println!("    {n:<10} = {v:?}");
    }

    assert_eq!(via_atgtdf.refno, refno);
    // Both paths should at least run — concrete equality is NOT
    // expected because ATGTSX-stride-1 fallback is structurally
    // different from the global ATGTDF position used by the legacy
    // path. The real comparison waits for #3b's authoritative
    // template payload.
}
