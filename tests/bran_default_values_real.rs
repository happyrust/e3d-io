use std::env;

use e3d_attlib::AttlibData;
use e3d_io::noun_catalog::NounCatalog;
use e3d_io::record::descriptor::{AttributeExtractionStatus, DescriptorValue};
use e3d_io::record::template::TemplateProvider;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DEFAULT_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001";
const DEFAULT_DESVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\desvir.dat";
const DEFAULT_ATTLIB: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat";
const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

mod common;

const FIXTURES: &[&str] = &[DEFAULT_DB, DEFAULT_DESVIR, DEFAULT_ATTLIB];

fn attribute<'a>(
    extraction: &'a e3d_io::record::descriptor::ElementExtraction,
    name: &str,
) -> &'a e3d_io::record::descriptor::ExtractedAttribute {
    extraction
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .unwrap_or_else(|| panic!("missing attribute {name}"))
}

#[test]
fn print_bran_non_implicit_attlib_defaults() {
    if common::missing(FIXTURES) {
        return;
    }
    let db = env::var("E3D_DESCRIPTOR_DB").unwrap_or_else(|_| DEFAULT_DB.into());
    let desvir = env::var("E3D_DESCRIPTOR_DESVIR").unwrap_or_else(|_| DEFAULT_DESVIR.into());
    let attlib_path = env::var("E3D_DESCRIPTOR_ATTLIB").unwrap_or_else(|_| DEFAULT_ATTLIB.into());
    let attlib = AttlibData::parse_file(attlib_path).expect("parse attlib authority");
    let mut provider =
        TemplateProvider::open(desvir, Some(DESIGN_TEMPLATE_TYPE)).expect("open desvir authority");
    let mut engine = ReadOnlyEngine::open(db).expect("open AMS 7999 database");
    let extraction = engine
        .extract_element_with_descriptors(RefNo::new(24383, 73930), &attlib, &mut provider)
        .expect("extract BRAN 24383/73930");

    let mut count = 0;
    let mut with_default = 0;
    for attribute in extraction
        .attributes
        .iter()
        .filter(|attribute| attribute.status == AttributeExtractionStatus::NonImplicit)
    {
        count += 1;
        let definition = attlib.attr_def(attribute.hash);
        if definition
            .and_then(|definition| definition.default_value)
            .is_some()
        {
            with_default += 1;
        }
        println!(
            "{}|0x{:08X}|{:?}",
            attribute.name,
            attribute.hash,
            definition.and_then(|definition| definition.default_value)
        );
    }
    println!("NON_IMPLICIT={count}|WITH_ATTLIB_DEFAULT={with_default}");
    // Was 49 before the noun's own default record was read. `CACHID`,
    // `HDSHAP`, `TDSHAP` and `LCHKDA` now answer from `desvir.dat` chunk 2
    // and carry `TemplateDefault` instead.
    assert_eq!(count, 45);
    assert_eq!(
        extraction
            .attributes
            .iter()
            .filter(|attribute| attribute.status == AttributeExtractionStatus::TemplateDefault)
            .count(),
        4
    );
}

#[test]
fn bran_catalog_defaults_match_e3d_tty_default_classes() {
    if common::missing(FIXTURES) {
        return;
    }
    let attlib = AttlibData::parse_file(DEFAULT_ATTLIB).expect("parse attlib authority");
    let mut provider = TemplateProvider::open(DEFAULT_DESVIR, Some(DESIGN_TEMPLATE_TYPE))
        .expect("open desvir authority");
    let mut engine = ReadOnlyEngine::open(DEFAULT_DB).expect("open AMS 7999 database");
    let mut extraction = engine
        .extract_element_with_descriptors(RefNo::new(24383, 73930), &attlib, &mut provider)
        .expect("extract BRAN 24383/73930");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("catalog/e3d31");
    let catalog = NounCatalog::read(
        root.join("noun_layout.json"),
        root.join("noun_attr_fields.json"),
    )
    .expect("load E3D 3.1 logical catalog");

    // Was 33 before the noun's own default record was read. The catalog is the
    // fallback for what `desvir.dat` does not state, so three rows it used to
    // fill now arrive as `TemplateDefault` instead.
    assert_eq!(catalog.apply_static_defaults(&mut extraction), 30);
    assert_eq!(
        extraction
            .attributes
            .iter()
            .filter(|attribute| attribute.status == AttributeExtractionStatus::CatalogDefault)
            .count(),
        30
    );
    assert_eq!(
        attribute(&extraction, "DESC").value,
        Some(DescriptorValue::Unset)
    );
    assert_eq!(
        attribute(&extraction, "TPRESS").value,
        Some(DescriptorValue::Real(0.0))
    );
    assert_eq!(
        attribute(&extraction, "BSTA").value,
        Some(DescriptorValue::Int(0))
    );
    assert_eq!(
        attribute(&extraction, "HWRF").value,
        Some(DescriptorValue::RefNo(RefNo::new(0, 0)))
    );
    assert_eq!(
        attribute(&extraction, "TSFBR").value,
        Some(DescriptorValue::Bool(false))
    );
    assert_eq!(
        attribute(&extraction, "PTNB").value,
        Some(DescriptorValue::Unset)
    );
    assert_eq!(
        attribute(&extraction, "HEADW").status,
        AttributeExtractionStatus::NonImplicit,
        "pseudo attributes must remain for the derived evaluator"
    );
}

#[test]
fn bran_runtime_derived_defaults_match_design_pluggers() {
    if common::missing(FIXTURES) {
        return;
    }
    let attlib = AttlibData::parse_file(DEFAULT_ATTLIB).expect("parse attlib authority");
    let mut provider = TemplateProvider::open(DEFAULT_DESVIR, Some(DESIGN_TEMPLATE_TYPE))
        .expect("open desvir authority");
    let mut engine = ReadOnlyEngine::open(DEFAULT_DB).expect("open AMS 7999 database");
    let mut extraction = engine
        .extract_element_with_descriptors(RefNo::new(24383, 73930), &attlib, &mut provider)
        .expect("extract BRAN 24383/73930");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("catalog/e3d31");
    let catalog = NounCatalog::read(
        root.join("noun_layout.json"),
        root.join("noun_attr_fields.json"),
    )
    .expect("load E3D 3.1 logical catalog");

    catalog.apply_static_defaults(&mut extraction);
    assert_eq!(catalog.apply_runtime_derived(&mut extraction), 6);
    for name in ["HEADW", "HEADH", "TAILW", "TAILH"] {
        let attribute = attribute(&extraction, name);
        assert_eq!(attribute.value, Some(DescriptorValue::Real(0.0)));
        assert!(matches!(
            attribute.status,
            AttributeExtractionStatus::RuntimeDerived { ref evaluator }
                if evaluator == "bran_circular_end_dimensions"
        ));
    }
    let inco = attribute(&extraction, "INCO");
    assert_eq!(inco.value, Some(DescriptorValue::Text(String::new())));
    assert!(matches!(
        inco.status,
        AttributeExtractionStatus::RuntimeDerived { ref evaluator }
            if evaluator == "design_qrofcn_absent_internal_points"
    ));
    let rlst = attribute(&extraction, "RLST");
    assert_eq!(rlst.value, Some(DescriptorValue::Unset));
    assert!(matches!(
        rlst.status,
        AttributeExtractionStatus::RuntimeDerived { ref evaluator }
            if evaluator == "design_qrosru_absent_saved_rules"
    ));
}
