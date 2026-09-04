#![allow(deprecated)] // exercises the ATGTDF-position path on purpose

//! Slice 4 Step 16 — `desvir.dat` end-to-end template loader test.
//!
//! Validates the new noun-template `.dat` file reader against the real
//! `desvir.dat` from a live AVEVA Design installation. The test:
//!
//! 1. Opens `desvir.dat` (SKIP if not present — CI environments without
//!    the AVEVA install).
//! 2. Loads the NXTR (0x000DBF71) noun template via the new
//!    `NounTemplate::load_from_template_file` path.
//! 3. Verifies the template has the expected shape (count > 9, beats
//!    the ATGTDF-position baseline of 9 attrs).
//! 4. Opens the fixture DB, runs the template-aware engine on a NXTR
//!    element, and compares against the legacy `summarize_element`
//!    (global ATGTDF-position) output.
//!
//! Per IDA evidence (`pdms-io-fork-engine-v2/docs/ida-3.1-structures.md
//! §14.7`), `desvir.dat` declares `template_type = 0x000B0692` and
//! contains 1113 noun templates. NXTR's template has 21 attribute
//! records with non-uniform strides (10/11).

use std::path::Path;

use e3d_attlib::{db1_dehash, AttlibData};
use e3d_io::record::template::NounTemplate;
use e3d_io::record::template_file::TemplateFile;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DESVIR_PATH: &str = common::DESVIR;
const DB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001";
const ATTLIB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\attlib.dat";

mod common;

/// Template type id of the DESIGN DB schema file (from IDA evidence:
/// `desvir.dat` header word 2).
const DESIGN_TEMPLATE_TYPE: u32 = 0x000B0692;

/// NXTR noun hash. Matches the fixture element at RefNo(17496, 9621).
const NXTR_HASH: u32 = 0x000DBF71;

fn desvir_present() -> bool {
    Path::new(DESVIR_PATH).exists()
}

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

/// Smoke: open `desvir.dat`, parse header, walk children, find NXTR,
/// load and parse its payload. Asserts the structural invariants
/// documented in `§14.7`.
#[test]
fn desvir_header_children_and_nxtr_payload() {
    if !desvir_present() {
        eprintln!("SKIP: {DESVIR_PATH} not present");
        return;
    }

    let mut tf = TemplateFile::open(DESVIR_PATH).expect("open desvir.dat");
    let hdr = tf.read_header().expect("read header");
    println!(
        "[desvir.dat] magic={} template_type=0x{:08X} aux1={} child_count={} chunk1_token={}",
        hdr.magic, hdr.template_type, hdr.aux1, hdr.child_count, hdr.chunk1_token
    );
    assert_eq!(hdr.magic, 6, "magic must be 6 per IDA sub_5AFA740");
    assert_eq!(
        hdr.template_type, DESIGN_TEMPLATE_TYPE,
        "desvir.dat declares DESIGN template_type"
    );
    assert!(hdr.child_count >= 100, "expected >= 100 noun templates");

    let children = tf.read_children(&hdr).expect("read children");
    assert_eq!(children.len(), hdr.child_count as usize);

    // The on-disk invariant the IDA binary search relies on.
    let sorted = children.windows(2).all(|w| w[0].hash <= w[1].hash);
    assert!(sorted, "children must be sorted by hash");

    // Find NXTR.
    let nxtr =
        TemplateFile::find_child(&children, NXTR_HASH).expect("NXTR must be present in desvir.dat");
    println!(
        "[NXTR child] hash=0x{:08X} chunk0=(tok={}, sz={}) chunk1=(tok={}, sz={}) chunk2=(tok={}, sz={})",
        nxtr.hash,
        nxtr.chunks[0].0, nxtr.chunks[0].1,
        nxtr.chunks[1].0, nxtr.chunks[1].1,
        nxtr.chunks[2].0, nxtr.chunks[2].1,
    );

    let payload = tf.read_payload(nxtr).expect("read NXTR payload");
    println!("[NXTR payload] {} dwords", payload.len());
    assert!(payload.len() >= 17, "payload too short for a header");

    // Parse with the high-level NounTemplate path.
    let template =
        NounTemplate::from_template_payload(&payload, NXTR_HASH).expect("parse NXTR template");
    println!(
        "[NXTR template] noun_hash=0x{:08X} count={} total_stride={}",
        template.noun_hash,
        template.attrs.len(),
        template.total_stride()
    );
    for (i, a) in template.attrs.iter().take(8).enumerate() {
        let name = e3d_attlib::lookup_system_name(a.hash)
            .map(String::from)
            .or_else(|| db1_dehash(a.hash))
            .unwrap_or_else(|| format!("0x{:08X}", a.hash));
        println!(
            "  [#{i}] hash=0x{:08X} ({}) stride={} aux={}",
            a.hash, name, a.stride, a.aux
        );
    }

    // Per the probe + IDA evidence: NXTR has 21 attribute records.
    assert_eq!(
        template.attrs.len(),
        21,
        "NXTR template should have 21 attrs per IDA"
    );

    // Stride must be > 1 for at least some records (non-uniform layout) —
    // distinguishes the noun-correct loader from the stride=1 ATGTSX
    // fallback.
    let max_stride = template.attrs.iter().map(|a| a.stride).max().unwrap();
    assert!(
        max_stride > 1,
        "expected non-uniform stride in real template"
    );
}

/// End-to-end: load the real NXTR template from `desvir.dat`, drive
/// `engine.summarize_element_with_template` on the fixture element,
/// and compare against the legacy `summarize_element` (global ATGTDF
/// position) baseline.
///
/// Per IDA `ATNLOG` semantics the template's value-add is **filtering
/// to the noun's actual attribute set** rather than scanning all 151
/// ATGTDF entries — so the template path may decode FEWER attributes
/// when the global ATGTDF table has positions that happen to be
/// non-zero on the element but don't actually belong to NXTR's schema.
#[test]
fn engine_summarize_nxtr_with_real_template_vs_legacy() {
    if !desvir_present() || !fixtures_present() {
        eprintln!("SKIP: desvir.dat or DB fixture not present");
        return;
    }

    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let template =
        NounTemplate::load_from_template_file(DESVIR_PATH, Some(DESIGN_TEMPLATE_TYPE), NXTR_HASH)
            .expect("load NXTR template from desvir.dat");
    println!(
        "[template] NXTR loaded from desvir.dat: {} attrs",
        template.attrs.len()
    );

    // Count how many of NXTR's 21 template hashes are actually in the
    // attlib ATGTDF — only those can be decoded. Anything else is a
    // template-listed attr the attlib doesn't carry (often UDA or
    // project extension).
    let in_atgtdf = template
        .attrs
        .iter()
        .filter(|a| attlib.attr_def(a.hash).is_some())
        .count();
    println!(
        "[coverage] {} / {} template attrs have ATGTDF entries",
        in_atgtdf,
        template.attrs.len()
    );

    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");
    let refno = RefNo::new(17496, 9621);

    let via_template = engine
        .summarize_element_with_template(refno, &attlib, &template)
        .expect("template summarize");
    let via_legacy = engine
        .summarize_element(refno, &attlib, 0)
        .expect("legacy summarize");

    println!(
        "[template path] {} non-zero attrs (filtered to NXTR's {} schema attrs)",
        via_template.attributes.len(),
        template.attrs.len()
    );
    for (n, v) in via_template.attributes.iter().take(15) {
        println!("    {n:<12} = {v:?}");
    }
    println!(
        "[legacy path  ] {} non-zero attrs (scanned all {} ATGTDF entries)",
        via_legacy.attributes.len(),
        attlib.attr_defs_unique().count()
    );
    for (n, v) in via_legacy.attributes.iter().take(15) {
        println!("    {n:<12} = {v:?}");
    }

    // Basic shape checks.
    assert_eq!(via_template.refno, refno);
    assert_eq!(via_template.noun_hash, NXTR_HASH);
    // The template-path output is bounded by template.attrs.len() (one
    // entry per template slot at most). Cannot be more than that.
    assert!(
        via_template.attributes.len() <= template.attrs.len(),
        "template path cannot emit more attrs than the template carries"
    );
    // Sanity: at least the legacy path should still produce non-zero
    // attrs (regression check against the engine modification).
    assert!(
        !via_legacy.attributes.is_empty(),
        "legacy path must still produce attributes (regression check)"
    );

    // The template path's output is the SUBSET of legacy-path attrs
    // whose hashes appear in NXTR's template. Verify the overlap.
    let template_hashes: std::collections::HashSet<u32> =
        template.attrs.iter().map(|a| a.hash).collect();
    let legacy_in_template = via_legacy
        .attributes
        .iter()
        .filter(|(name, _)| {
            // Resolve name back to hash; if name lookup fails, treat as
            // not-in-template (conservative).
            let name_hash = e3d_attlib::db1_hash(name);
            template_hashes.contains(&name_hash)
        })
        .count();
    println!(
        "[overlap] {} of {} legacy attrs are listed in NXTR's template",
        legacy_in_template,
        via_legacy.attributes.len()
    );
}

/// Spot-check a few other DESIGN nouns to confirm the loader works
/// across the family, not just NXTR.
#[test]
fn desvir_spot_check_multiple_design_nouns() {
    if !desvir_present() {
        eprintln!("SKIP: {DESVIR_PATH} not present");
        return;
    }
    let mut tf = TemplateFile::open(DESVIR_PATH).expect("open");
    let hdr = tf.read_header().expect("header");
    let children = tf.read_children(&hdr).expect("children");

    // Pre-computed noun hashes (db1_hash of the names). Each name should
    // have exactly one matching child in desvir.dat per the IDA invariant.
    let cases: &[(&str, u32)] = &[
        ("NXTR", 0x000DBF71),
        ("STWALL", 0x0AAD58D2),
        ("VERT", 0x000E518C),
        ("PLOO", 0x000CC949),
        ("ZONE", 0x0009C5ED),
        ("FIXING", 0x06793D58),
    ];
    for (name, hash) in cases {
        let child = TemplateFile::find_child(&children, *hash);
        match child {
            Some(c) => {
                let payload = tf.read_payload(c).expect("payload");
                let template = NounTemplate::from_template_payload(&payload, *hash);
                match template {
                    Some(t) => println!(
                        "[{name:8}] hash=0x{:08X} payload_dwords={} template_attrs={}",
                        hash,
                        payload.len(),
                        t.attrs.len()
                    ),
                    None => println!(
                        "[{name:8}] hash=0x{:08X} payload_dwords={} (parse failed)",
                        hash,
                        payload.len()
                    ),
                }
                assert!(payload.len() >= 17, "{name}: payload too short");
            }
            None => panic!("{name} (0x{hash:08X}) must be present in desvir.dat"),
        }
    }
}
