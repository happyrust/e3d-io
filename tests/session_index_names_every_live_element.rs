//! An element the record scan was written for, found by one descent.
//!
//! `find_element` used to end in a whole-file byte scan for the RefNo,
//! because on this database the roots it searched did not name this BRAN.
//! Those roots came from reading every page and keeping the index pages no
//! other index page named, which finds the current tree along with every tree
//! an earlier session left behind — and answers from whichever sorted first.
//! The session's own root names it, so the scan has no work left to cover for
//! and is gone.
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::record::descriptor::{AttributeExtractionStatus, DescriptorValue};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::TemplateProvider;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001";
const DESVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\desvir.dat";
const ATTLIB: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat";
const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

mod common;

const FIXTURES: &[&str] = &[DB, DESVIR, ATTLIB];

/// This BRAN was the fixture for the record scan: it appeared to be missing
/// from the index, so `find_element` fell back to reading the whole file
/// backwards looking for its RefNo as a byte pattern. It was never missing.
/// The reader was searching the wrong trees — every index page in the file
/// rather than the one the session names — and the scan was covering for it.
#[test]
fn the_session_index_names_the_bran_the_record_scan_was_written_for() {
    if common::missing(FIXTURES) {
        return;
    }
    let target = RefNo::new(24383, 73930);
    let mut engine = ReadOnlyEngine::open(DB).expect("open AMS 7999");
    let indexed = engine.indexed_refnos().expect("read the index");
    assert!(
        indexed.iter().any(|(candidate, _, _)| *candidate == target),
        "{target} is indexed by session {}",
        engine.session().sesno
    );

    let view = engine
        .find_element(target)
        .expect("one descent")
        .expect("BRAN record must be found");
    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse BRAN record");
    assert_eq!(parsed.refno, target);
    assert_eq!(parsed.noun_hash, db1_hash("BRAN"));

    let attlib = AttlibData::parse_file(ATTLIB).expect("parse attlib");
    let mut provider =
        TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("open desvir");
    let template = provider
        .template_for(parsed.noun_hash)
        .expect("load BRAN descriptor")
        .expect("BRAN descriptor exists");
    let extraction = engine
        .extract_parsed_element_with_descriptors(target, &parsed, &attlib, &template)
        .expect("extract BRAN");
    let name = extraction
        .attributes
        .iter()
        .find(|attribute| attribute.hash == db1_hash("NAME"))
        .expect("NAME descriptor");
    assert_eq!(name.status, AttributeExtractionStatus::DecodedExplicit);
    assert_eq!(
        name.value,
        Some(DescriptorValue::Text(
            "/1RCV0101-60.3-VMB-R90-R332]R333".into()
        ))
    );
}
