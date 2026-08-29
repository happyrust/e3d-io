#![allow(deprecated)] // exercises the ATGTDF-position path on purpose

//! Slice 4 step 9 — one-call element summary on fixture.

use std::path::Path;

use e3d_attlib::AttlibData;
use e3d_io::index::IndexPageView;
use e3d_io::record::atnlog::AttrValue;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001";
const ATTLIB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\attlib.dat";

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

#[test]
fn summarize_first_fixture_element() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present");
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");

    let root = engine.find_index_root().unwrap().expect("root");
    let target = {
        let data = engine.read_page(root).unwrap().to_vec();
        let view = IndexPageView::parse(&data).unwrap();
        view.entries
            .iter()
            .find(|e| !e.is_start_marker())
            .unwrap()
            .refno
    };

    let summary = engine
        .summarize_element(target, &attlib, 3)
        .expect("summarize");

    println!("\n=== Element Summary ===");
    println!(
        "[id] refno={} noun_hash=0x{:08X} noun_name={:?}",
        summary.refno, summary.noun_hash, summary.noun_name
    );
    println!(
        "[layout] implicit_word_count={}",
        summary.implicit_word_count
    );
    println!("[attrs] {} non-zero decoded", summary.attributes.len());
    for (i, (name, value)) in summary.attributes.iter().enumerate() {
        let short = match value {
            AttrValue::Bool(b) => format!("Bool({b})"),
            AttrValue::Int(v) => format!("Int({v})"),
            AttrValue::Real(v) => format!("Real({v:.4})"),
            AttrValue::DbRef(r) => format!("Ref({r})"),
            AttrValue::Text(t) => format!("Text(0x{t:08X})"),
            AttrValue::Position(p) => format!("Pos({:.2},{:.2},{:.2})", p[0], p[1], p[2]),
            AttrValue::Direction(d) => format!("Dir({:.2},{:.2},{:.2})", d[0], d[1], d[2]),
            AttrValue::Orientation(_) => "Orient[9]".into(),
            AttrValue::IntArray(v) => format!("IntArr[{}]", v.len()),
            AttrValue::RealArray(v) => format!("RealArr[{}]", v.len()),
            AttrValue::BoolArray(v) => format!("BoolArr[{}]", v.len()),
            AttrValue::DbRefArray(v) => format!("RefArr[{}]", v.len()),
            AttrValue::TextArray(v) => format!("TextArr[{}]", v.len()),
            AttrValue::Raw(w) => format!("Raw(0x{w:08X})"),
        };
        println!("  [{:>3}] {:<10} -> {}", i, name, short);
    }

    assert!(summary.refno == target, "refno mismatch");
    assert!(summary.noun_hash != 0, "noun_hash should be non-zero");
    assert!(
        summary.attributes.len() >= 5,
        "expected at least 5 decoded non-zero attrs, got {}",
        summary.attributes.len()
    );
}
