use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::engine::ResolveError;
use e3d_io::record::descriptor::{AttributeExtractionStatus, DescriptorValue};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::{ExplicitAttribute, ExplicitValue};
use e3d_io::record::template::TemplateProvider;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const DESVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\desvir.dat";
const ATTLIB: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat";
const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

mod common;

const FIXTURES: &[&str] = &[DB, DESVIR, ATTLIB];

#[test]
fn dbmdat_descriptor_decodes_its_two_internal_explicit_attributes() {
    if common::missing(FIXTURES) {
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB).expect("parse attlib authority");
    let mut provider =
        TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("open desvir");
    let template = provider
        .template_for(db1_hash("DBMDAT"))
        .expect("read DBMDAT descriptor")
        .expect("DBMDAT descriptor exists");
    assert!(template.is_authoritative());
    assert_eq!(template.attrs.len(), 2);
    assert_eq!(template.attrs[0].hash, db1_hash("DBMVAL"));
    assert_eq!(template.attrs[0].storage_type_code, 7);
    assert_eq!(template.attrs[1].hash, db1_hash("DBMEXT"));
    assert_eq!(template.attrs[1].storage_type_code, 3);

    // DBMDAT is constructed internally by DB_DB metadata operations rather
    // than stored as a normal AMS design element. Exercise the same explicit
    // payload shape without inventing an indexed RefNo fixture.
    let refno = RefNo::new(1, 1);
    let parsed = ParsedElement {
        impl_len_words: 0,
        refno,
        noun_hash: db1_hash("DBMDAT"),
        owner: RefNo::new(1, 0),
        prefix_offset: 24,
        implicit_data: Vec::new(),
        members: Vec::new(),
        explicit_attributes: vec![
            ExplicitAttribute {
                hash: db1_hash("DBMVAL"),
                type_code: 0x1C00,
                data_words: 3,
                is_uda: false,
                value: ExplicitValue::IntArray(vec![101, 202]),
            },
            ExplicitAttribute {
                hash: db1_hash("DBMEXT"),
                type_code: 0x0C00,
                data_words: 1,
                is_uda: false,
                value: ExplicitValue::Integer(7),
            },
        ],
        record_end: 24,
        total_size: 24,
    };
    let engine = ReadOnlyEngine::open(DB).expect("open engine context");
    let extraction = engine
        .extract_parsed_element_with_descriptors(refno, &parsed, &attlib, &template)
        .expect("extract DBMDAT");

    assert_eq!(extraction.attributes.len(), 2);
    assert!(extraction
        .attributes
        .iter()
        .all(|attribute| { attribute.status == AttributeExtractionStatus::DecodedExplicit }));
    // Neither attribute has an attlib definition, and the descriptor states
    // the storage shape but not which of integer and word the words hold —
    // `sub_5AB5BC0` accepts whichever type the caller asks for. `DBMVAL` is
    // storage type 7, one word per element, so it stays undetermined;
    // `DBMEXT` is type 3, a single word, which is how a WORD is stored.
    // This test asserted `IntArray` and `Int` before the WORD-storage reading
    // landed and was never updated, because `#[ignore]` hid it.
    assert_eq!(
        extraction.attributes[0].value,
        Some(DescriptorValue::RawWords(vec![101, 202]))
    );
    assert_eq!(
        extraction.attributes[1].value,
        Some(DescriptorValue::Word {
            raw: 7,
            text: e3d_io::record::descriptor::word_name(7),
        })
    );

    let mut implicit_template = template.clone();
    implicit_template.attrs[0].compact.location.raw = 1;
    implicit_template.attrs[0].wide.location.raw = 1;
    assert!(matches!(
        engine.extract_parsed_element_with_descriptors(
            refno,
            &parsed,
            &attlib,
            &implicit_template
        ),
        Err(ResolveError::FormatWordMissing(hash)) if hash == db1_hash("DBMDAT")
    ));
}
