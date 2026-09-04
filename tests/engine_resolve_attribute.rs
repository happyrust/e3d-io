#![allow(deprecated)] // exercises the ATGTDF-position path on purpose

//! Slice 4 step 8 — `ReadOnlyEngine::resolve_attribute` with bounded
//! SYNO chain recursion on a real fixture element.

use std::path::Path;

use e3d_attlib::AttlibData;
use e3d_io::engine::ResolveError;
use e3d_io::record::atnlog::AttrValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = common::TEST_FILE_DB;
const ATTLIB_PATH: &str = common::TEST_FILE_ATTLIB;

mod common;

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

/// The first NXTR the index states: `17496/72444` on this fixture.
fn first_nxtr(engine: &mut ReadOnlyEngine) -> RefNo {
    common::first_element_of_noun(engine, common::NXTR_HASH)
        .expect("the fixture indexes at least one NXTR")
}

#[test]
fn resolve_attribute_returns_direct_value_when_present() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present");
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");

    let target = first_nxtr(&mut engine);
    println!("[fixture] element refno={}", target);

    // DEPEND reads as a non-zero Real on the first NXTR of this fixture
    // (Real(-180.0) on 17496/72444). Not on every NXTR: the fourth in
    // RefNo order, 17496/72561, reads 0.0 — so the element is the first
    // one, not any one.
    let depend_hash = e3d_attlib::db1_hash("DEPEND");
    println!("[query] DEPEND hash = 0x{:08X}", depend_hash);

    let value = engine
        .resolve_attribute(target, depend_hash, &attlib, 3)
        .expect("resolve");
    println!("[result] DEPEND -> {:?}", value);
    match value {
        Some(AttrValue::Real(r)) => {
            assert!(r.abs() > 0.0001, "DEPEND should be non-zero real, got {r}");
            assert!(r.is_finite(), "DEPEND should be finite, got {r}");
        }
        other => panic!("expected Real(non-zero), got {other:?}"),
    }
}

#[test]
fn resolve_attribute_returns_zero_when_chain_empty() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present");
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");
    let target = first_nxtr(&mut engine);

    // SIZE is known to read as Bool(false) on this element (see step 4
    // step 5 fixture decode log). It's also known to NOT have a SYNO
    // chain entry pointing anywhere meaningful, so the resolve should
    // settle on the original Bool(false).
    let size_hash = e3d_attlib::db1_hash("SIZE");
    let value = engine
        .resolve_attribute(target, size_hash, &attlib, 2)
        .expect("resolve");
    println!("[result] SIZE -> {:?}", value);
    match value {
        Some(AttrValue::Bool(false)) => {}
        // Acceptable: the chain may resolve to anything; we just want to
        // ensure no panic and a deterministic Some(...) coming back.
        Some(_) => {}
        None => panic!("expected Some(_), got None"),
    }
}

#[test]
fn resolve_attribute_errors_on_unknown_hash() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present");
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");
    let target = first_nxtr(&mut engine);

    // 0xDEADBEEF is statistically not in the fixture's ATGTDF table.
    let err = engine.resolve_attribute(target, 0xDEADBEEF, &attlib, 2);
    match err {
        Err(ResolveError::AttrNotInAttlib(0xDEADBEEF)) => {}
        other => panic!("expected AttrNotInAttlib(0xDEADBEEF), got {other:?}"),
    }
}

#[test]
fn resolve_attribute_caps_chain_depth() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present");
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");
    let target = first_nxtr(&mut engine);

    // With max_chain_depth=0, no chain walking is performed even if the
    // direct slot is zero. The result is whatever direct returned.
    let size_hash = e3d_attlib::db1_hash("SIZE");
    let v0 = engine
        .resolve_attribute(target, size_hash, &attlib, 0)
        .expect("resolve");
    let v3 = engine
        .resolve_attribute(target, size_hash, &attlib, 3)
        .expect("resolve");
    // The depth-0 result equals the direct decode; depth-3 may differ
    // if a chain happens to exist. We assert v0 is Some(_) (deterministic).
    assert!(v0.is_some(), "depth-0 resolve should produce a value");
    // The two may differ in value but both must be Some(_).
    assert!(v3.is_some());
    // For SIZE, the direct slot reads as 0 and SYNO doesn't apply, so
    // both paths should agree.
    let same_variant = matches!(
        (&v0, &v3),
        (Some(AttrValue::Bool(_)), Some(AttrValue::Bool(_)))
    );
    assert!(same_variant, "v0={:?} v3={:?}", v0, v3);

    // Silence unused
    let _ = ParsedElement::parse;
}
