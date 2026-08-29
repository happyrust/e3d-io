use std::collections::HashSet;
use std::env;

use e3d_attlib::AttlibData;
use e3d_io::attlib_metadata::AttributeMetadataCatalog;
use e3d_io::record::descriptor::AttributeExtractionStatus;
use e3d_io::record::template::TemplateProvider;
use e3d_io::refno::RefNo;
use e3d_io::tty::{parse_attlist_capture, reconcile_attlist, CompleteAttributeSource};
use e3d_io::ReadOnlyEngine;

const DEFAULT_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const DEFAULT_DESVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\desvir.dat";
const DEFAULT_ATTLIB: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat";
const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

mod common;

const FIXTURES: &[&str] = &[DEFAULT_DB, DEFAULT_DESVIR, DEFAULT_ATTLIB];

#[test]
fn bran_complete_attlist_has_no_omitted_tty_rows() {
    if common::missing(FIXTURES) {
        return;
    }
    let db = env::var("E3D_DESCRIPTOR_DB").unwrap_or_else(|_| DEFAULT_DB.into());
    let desvir = env::var("E3D_DESCRIPTOR_DESVIR").unwrap_or_else(|_| DEFAULT_DESVIR.into());
    let attlib_path = env::var("E3D_DESCRIPTOR_ATTLIB").unwrap_or_else(|_| DEFAULT_ATTLIB.into());
    let attlib = AttlibData::parse_file(&attlib_path).expect("parse attlib authority");
    let metadata =
        AttributeMetadataCatalog::parse_file(&attlib_path, &attlib).expect("parse report names");
    let mut provider =
        TemplateProvider::open(&desvir, Some(DESIGN_TEMPLATE_TYPE)).expect("open desvir authority");
    let mut engine = ReadOnlyEngine::open(&db).expect("open AMS 8000 database");
    let extraction = engine
        .extract_element_with_descriptors(RefNo::new(24384, 23257), &attlib, &mut provider)
        .expect("extract BRAN 24384/23257");

    let tty_rows = parse_attlist_capture(include_str!("fixtures/bran_attlist_tty.log"))
        .expect("parse saved E3D TTY ATTLIST authority");
    let complete = reconcile_attlist(&tty_rows, &extraction, &metadata, None);

    assert_eq!(tty_rows.len(), 121);
    assert_eq!(complete.len(), tty_rows.len(), "a TTY row was omitted");
    assert_eq!(
        complete
            .iter()
            .map(|row| row.name.to_ascii_lowercase())
            .collect::<HashSet<_>>()
            .len(),
        118
    );
    assert!(complete
        .iter()
        .any(|row| row.name == "Inco" && row.rendered_value.is_empty()));
    for source in [
        CompleteAttributeSource::BinaryImplicit,
        CompleteAttributeSource::BinaryExplicit,
        CompleteAttributeSource::BinaryUnresolved,
        CompleteAttributeSource::Structural,
        CompleteAttributeSource::TtyAuthority,
    ] {
        assert!(complete.iter().any(|row| row.source == source));
    }
    let name = complete.iter().find(|row| row.name == "Name").unwrap();
    assert_eq!(name.source, CompleteAttributeSource::BinaryExplicit);
    assert_eq!(
        name.binary_status,
        Some(AttributeExtractionStatus::DecodedExplicit)
    );
    assert_eq!(name.rendered_value, "/C-OR-1R345-C");
    assert!(complete.iter().any(|row| {
        row.name == ":PFConsChk" && row.source == CompleteAttributeSource::TtyAuthority
    }));
}
