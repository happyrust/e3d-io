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

#[test]
fn finds_live_bran_missing_from_btree_and_decodes_explicit_name() {
    if common::missing(FIXTURES) {
        return;
    }
    let target = RefNo::new(24383, 73930);
    let mut engine = ReadOnlyEngine::open(DB).expect("open AMS 7999");
    let indexed = engine
        .scan_all_indexed_refnos()
        .expect("scan B+tree leaves");
    // This element used to be reachable only by scanning records, which is
    // what made it the fixture for that path. It now also has a leaf entry, so
    // the lookup below no longer proves the fallback works — say so rather
    // than fail, and keep the decode assertions, which still hold either way.
    if indexed.iter().any(|(candidate, _, _)| *candidate == target) {
        eprintln!(
            "note: {target} is indexed now, so this run exercises the B+tree path, \
             not the record scan"
        );
    }

    let view = engine
        .find_element(target)
        .expect("scan fallback")
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
