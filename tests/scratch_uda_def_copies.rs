//! Scratch: does another copy of the :3D_SJJD definition carry the DFLT the
//! one we read is missing?
//!
//! `cargo test --test scratch_uda_def_copies -- --ignored --nocapture`

use std::path::Path;

use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

mod common;

const FIXTURES: &[&str] = &[AMS];

#[test]
fn dump_definition_copies() {
    if common::missing(FIXTURES) {
        return;
    }
    let path = Path::new(AMS).join("ams5100_0001");
    let mut engine = ReadOnlyEngine::open(&path).expect("open");
    let entries = engine.indexed_refnos().expect("scan");

    for refno in [
        RefNo::new(13292, 147), // :3D_SJJD  -> E3D reports C
        RefNo::new(13292, 149), // :3D_SJZT  -> E3D reports Designing (works)
        RefNo::new(13292, 75),  // :COMPTYPE -> E3D reports ANY
    ] {
        let locations: Vec<(u32, u32)> = entries
            .iter()
            .filter(|(candidate, _, _)| *candidate == refno)
            .map(|(_, page, offset)| (*page, *offset))
            .collect();
        println!("=== {refno}: {} index entries ===", locations.len());
        let mut shown = Vec::new();
        for (page, offset) in locations {
            let Ok(view) = engine.read_element_direct(refno, page, offset) else {
                continue;
            };
            let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
                continue;
            };
            if parsed.refno != refno {
                continue;
            }
            let shape: Vec<String> = parsed
                .explicit_attributes
                .iter()
                .map(|attribute| format!("{}={:?}", attribute.hash, attribute.value))
                .collect();
            if shown.contains(&shape) {
                continue;
            }
            println!(
                "  page {page:<7} offset {offset:<6} implicit_words={}",
                parsed.impl_len_words
            );
            for line in &shape {
                println!("      {line}");
            }
            shown.push(shape);
        }
        println!("  distinct shapes: {}", shown.len());
    }
}
