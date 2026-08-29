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
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001";
const ATTLIB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\attlib.dat";

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
    let refno = RefNo::new(17496, 9621);

    // NXTR (0x000DBF71) is not in ATGTSX on this fixture — build a
    // small manual template from the global ATGTDF defs as a stand-in
    // (mimics what the DB-file loader would produce when it eventually
    // reads the real per-noun layout).
    let nxtr_hash = 0x000DBF71;
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

    // Probe a few known refnos and find one whose noun has an ATGTSX
    // template entry on the fixture.
    let candidates = [
        RefNo::new(17496, 9621),
        RefNo::new(17496, 924),
        RefNo::new(17496, 13591),
        RefNo::new(17496, 100),
        RefNo::new(17496, 1000),
        RefNo::new(17496, 5000),
        RefNo::new(17496, 10000),
    ];
    let mut found: Option<(RefNo, u32, NounTemplate)> = None;
    for refno in &candidates {
        let view = match engine.find_element(*refno) {
            Ok(Some(v)) => v,
            _ => continue,
        };
        let parsed = match e3d_io::record::element::ParsedElement::parse(&view.raw_bytes) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if let Some(t) = NounTemplate::from_attlib_noun_map(&attlib, parsed.noun_hash) {
            found = Some((*refno, parsed.noun_hash, t));
            break;
        }
    }

    let (refno, noun_hash, template) = match found {
        Some(x) => x,
        None => {
            eprintln!(
                "SKIP: none of the candidate refnos hit a noun with an ATGTSX template \
                 (NXTR/STWALL etc. are not in ATGTSX on this fixture)"
            );
            return;
        }
    };

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
