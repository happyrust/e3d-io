//! Every `LINE.Pts` array in the catalogue database, to see the shape of the
//! list behind `P0 P3` and `P1 T0 P2`.
//!
//! The TTY sample holds two of them, which is not enough to tell a terminator
//! from a value or to say what range of numbers `T` covers. The database holds
//! every `LINE` in the project, and while it cannot say what E3D prints, it
//! can say whether the trailing `1000` is universal and how the other items
//! are distributed.
//!
//! Run with:
//! `cargo test --test scratch_line_pts -- --ignored --nocapture`

use std::collections::BTreeMap;

use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::template::TemplateProvider;
use e3d_io::ReadOnlyEngine;

const DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7000_0001";
const SHADOW: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all";

#[test]
#[ignore = "requires the installed E3D 3.1 catalogue project"]
fn report_every_line_pts_array() {
    let attlib_path = format!("{SHADOW}\\attlib.dat");
    if !std::path::Path::new(&attlib_path).exists() {
        eprintln!("SKIP: {attlib_path} not present");
        return;
    }
    let attlib = AttlibData::parse_file(&attlib_path).expect("parse attlib");
    let mut provider = TemplateProvider::open(
        format!("{SHADOW}\\catvir.dat"),
        Some(db1_hash("CATA")),
    )
    .expect("open catvir");
    let mut engine = ReadOnlyEngine::open(DB).expect("open catalogue database");

    let line = db1_hash("LINE");
    let pts = db1_hash("PTS");
    let template = provider
        .template_for(line)
        .expect("load LINE template")
        .expect("LINE template present")
        .clone();
    let mut indexed = engine.scan_all_indexed_refnos().expect("scan index");
    indexed.sort_by_key(|(refno, _, _)| (refno.word0, refno.word1));
    indexed.dedup_by_key(|(refno, _, _)| (refno.word0, refno.word1));
    println!("indexed elements: {}", indexed.len());

    let mut shapes: BTreeMap<String, usize> = BTreeMap::new();
    let mut items: BTreeMap<u32, usize> = BTreeMap::new();
    let mut trailing = 0usize;
    let mut shown = 0usize;
    let mut found = 0usize;
    for (refno, page, offset) in &indexed {
        let Ok(view) = engine.read_element_direct(*refno, *page, *offset) else {
            continue;
        };
        let Ok(parsed) = e3d_io::record::element::ParsedElement::parse(&view.raw_bytes) else {
            continue;
        };
        if parsed.noun_hash != line {
            continue;
        }
        found += 1;
        let Ok(extraction) =
            engine.extract_parsed_element_with_descriptors(*refno, &parsed, &attlib, &template)
        else {
            continue;
        };
        let Some(value) = extraction
            .attributes
            .iter()
            .find(|attribute| attribute.hash == pts)
            .and_then(|attribute| attribute.value.as_ref())
        else {
            continue;
        };
        let words: Vec<u32> = match value {
            DescriptorValue::IntArray(values) => values.iter().map(|item| *item as u32).collect(),
            DescriptorValue::WordArray(values) | DescriptorValue::RawWords(values) => values.clone(),
            _ => {
                *shapes
                    .entry(format!("<{value:?} is not a list>"))
                    .or_default() += 1;
                continue;
            }
        };
        if words.last() == Some(&1000) {
            trailing += 1;
        }
        for item in &words {
            *items.entry(*item).or_default() += 1;
        }
        *shapes.entry(format!("{} items", words.len())).or_default() += 1;
        if shown < 25 {
            println!("  {refno}  {words:?}");
            shown += 1;
        }
    }

    println!("\nLINE elements read: {found}");
    println!("lists ending in 1000: {trailing}");
    println!("lengths:");
    for (shape, count) in &shapes {
        println!("  {shape:<24} {count}");
    }
    println!("items by value:");
    for (item, count) in &items {
        println!("  {item:<8} x{count}");
    }
}
