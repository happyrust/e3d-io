use std::env;

use e3d_attlib::AttlibData;
use e3d_io::record::descriptor::{
    AttributeExtractionStatus, DescriptorValue, ElementExtraction, ExtractedAttribute,
};
use e3d_io::record::template::{StorageLayout, TemplateProvider};
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DEFAULT_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const DEFAULT_DESVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\desvir.dat";
const DEFAULT_ATTLIB: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat";
const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

mod common;

const FIXTURES: &[&str] = &[DEFAULT_DB, DEFAULT_DESVIR, DEFAULT_ATTLIB];

#[test]
fn bran_and_ftub_match_tty_authority_without_noun_map_json() {
    if common::missing(FIXTURES) {
        return;
    }
    let db = env::var("E3D_DESCRIPTOR_DB").unwrap_or_else(|_| DEFAULT_DB.into());
    let desvir = env::var("E3D_DESCRIPTOR_DESVIR").unwrap_or_else(|_| DEFAULT_DESVIR.into());
    let attlib_path = env::var("E3D_DESCRIPTOR_ATTLIB").unwrap_or_else(|_| DEFAULT_ATTLIB.into());
    let attlib = AttlibData::parse_file(&attlib_path).expect("parse attlib authority");
    let mut provider =
        TemplateProvider::open(&desvir, Some(DESIGN_TEMPLATE_TYPE)).expect("open desvir authority");
    let mut engine = ReadOnlyEngine::open(&db).expect("open AMS 8000 database");

    let bran = engine
        .extract_element_with_descriptors(RefNo::new(24384, 23257), &attlib, &mut provider)
        .expect("extract BRAN 24384/23257");
    assert_eq!(bran.noun_name.as_deref(), Some("BRAN"));
    assert_eq!(bran.layout, StorageLayout::Wide);
    assert_vec_near(value(&bran, "HPOS"), &[8551.0, 14060.0, 2900.0], 0.6);
    assert_vec_near(value(&bran, "TPOS"), &[9093.0, 9972.95, 2900.0], 1e-8);
    assert_vec_near(
        value(&bran, "HDIR"),
        &[0.8290375725550417, -0.5591929034707468, 0.0],
        1e-12,
    );
    assert_real(value(&bran, "HBOR"), 50.0);
    assert_word(value(&bran, "HCON"), 726491, Some("BOXI"));

    let ftub = engine
        .extract_element_with_descriptors(RefNo::new(24384, 23262), &attlib, &mut provider)
        .expect("extract FTUB 24384/23262");
    assert_eq!(ftub.noun_name.as_deref(), Some("FTUB"));
    assert_vec_near(value(&ftub, "POS"), &[10887.0, 12332.0, 2900.0], 0.3);
    assert_vec_near(value(&ftub, "ORI"), &[0.0, 0.0, -44.0], 1e-12);
    assert_real(value(&ftub, "HEIG"), 480.0);
    assert_word(value(&ftub, "ARRI"), 1, None);
    assert_word(value(&ftub, "LEAV"), 2, None);
}

fn attribute<'a>(extraction: &'a ElementExtraction, name: &str) -> &'a ExtractedAttribute {
    let attribute = extraction
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .unwrap_or_else(|| panic!("missing {name}"));
    assert_eq!(attribute.status, AttributeExtractionStatus::Decoded);
    attribute
}

fn value<'a>(extraction: &'a ElementExtraction, name: &str) -> &'a DescriptorValue {
    attribute(extraction, name)
        .value
        .as_ref()
        .unwrap_or_else(|| panic!("missing value for {name}"))
}

fn assert_vec_near(actual: &DescriptorValue, expected: &[f64], tolerance: f64) {
    let DescriptorValue::RealArray(actual) = actual else {
        panic!("expected real array, got {actual:?}");
    };
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} != {expected}"
        );
    }
}

fn assert_real(actual: &DescriptorValue, expected: f64) {
    let DescriptorValue::Real(actual) = actual else {
        panic!("expected real, got {actual:?}");
    };
    assert!((actual - expected).abs() <= 1e-12);
}

fn assert_word(actual: &DescriptorValue, expected_raw: u32, expected_text: Option<&str>) {
    let DescriptorValue::Word { raw, text } = actual else {
        panic!("expected word, got {actual:?}");
    };
    assert_eq!(*raw, expected_raw);
    assert_eq!(text.as_deref(), expected_text);
}
