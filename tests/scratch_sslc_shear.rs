//! Every `SSLC` shear word in the catalogue database.
//!
//! The TTY sample holds one non-zero shear: `Pxbshear` reads as the word
//! 0xFFFFFA24, which is -1500 as a signed integer, where E3D prints `-15`. One
//! row cannot say whether that is a hundredth-of-a-degree fixed point or
//! something else, but the spread of values across every `SSLC` in the project
//! can say whether they are all multiples of a hundred.
//!
//! Run with:
//! `cargo test --test scratch_sslc_shear -- --ignored --nocapture`

use std::collections::BTreeMap;

use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::TemplateProvider;
use e3d_io::ReadOnlyEngine;

const DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7000_0001";
mod common;

const SHADOW: &str = common::SCHEMA_BINARIES;
const SHEARS: &[&str] = &["PXTS", "PYTS", "PXBS", "PYBS"];

#[test]
#[ignore = "requires the installed E3D 3.1 catalogue project"]
fn report_every_sslc_shear_word() {
    let attlib_path = format!("{SHADOW}\\attlib.dat");
    if !std::path::Path::new(&attlib_path).exists() {
        eprintln!("SKIP: {attlib_path} not present");
        return;
    }
    let attlib = AttlibData::parse_file(&attlib_path).expect("parse attlib");
    let mut provider =
        TemplateProvider::open(format!("{SHADOW}\\catvir.dat"), Some(db1_hash("CATA")))
            .expect("open catvir");
    let mut engine = ReadOnlyEngine::open(DB).expect("open catalogue database");

    let sslc = db1_hash("SSLC");
    let template = provider
        .template_for(sslc)
        .expect("load SSLC template")
        .expect("SSLC template present")
        .clone();
    let mut indexed = engine.indexed_refnos().expect("scan index");
    indexed.sort_by_key(|(refno, _, _)| (refno.word0, refno.word1));
    indexed.dedup_by_key(|(refno, _, _)| (refno.word0, refno.word1));

    let mut values: BTreeMap<i64, usize> = BTreeMap::new();
    let mut shapes: BTreeMap<String, usize> = BTreeMap::new();
    let mut found = 0usize;
    for (refno, page, offset) in &indexed {
        let Ok(view) = engine.read_element_direct(*refno, *page, *offset) else {
            continue;
        };
        let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
            continue;
        };
        if parsed.noun_hash != sslc {
            continue;
        }
        found += 1;
        let Ok(extraction) =
            engine.extract_parsed_element_with_descriptors(*refno, &parsed, &attlib, &template)
        else {
            continue;
        };
        for shear in SHEARS {
            let hash = db1_hash(shear);
            let Some(attribute) = extraction
                .attributes
                .iter()
                .find(|attribute| attribute.hash == hash)
            else {
                continue;
            };
            match attribute.value.as_ref() {
                Some(DescriptorValue::Word { raw, .. }) => {
                    *values.entry(i64::from(*raw as i32)).or_default() += 1;
                }
                other => {
                    *shapes.entry(format!("{other:?}")).or_default() += 1;
                }
            }
        }
    }

    println!("SSLC elements read: {found}");
    println!("shear words, as signed integers:");
    for (value, count) in &values {
        let hundredths = *value as f64 / 100.0;
        let thousandths = *value as f64 / 1000.0;
        println!("  {value:<12} x{count:<5} /100 = {hundredths:<10} /1000 = {thousandths}");
    }
    if !shapes.is_empty() {
        println!("values that are not a word:");
        for (shape, count) in &shapes {
            println!("  {shape:<44} {count}");
        }
    }
}
