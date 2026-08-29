//! Print every stored copy of a few elements so the copy `find_element`
//! answers with can be compared against the alternatives.
//!
//! Run with:
//! `cargo test --test scratch_copy_shapes -- --ignored --nocapture`

use std::path::Path;

use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const DB: &str = "ams7000_0001";
const WANTED: &[(&str, u32, u32)] = &[
    ("LCYL", 15192, 67352),
    ("CATE", 15192, 521),
    ("GMSE", 15192, 67324),
    ("PTAX", 15192, 67337),
];

#[test]
#[ignore = "requires an installed E3D 3.1 AMS database"]
fn print_every_copy() {
    let path = Path::new(AMS).join(DB);
    if !path.exists() {
        eprintln!("SKIP: {} not present", path.display());
        return;
    }
    let mut engine = ReadOnlyEngine::open(&path).expect("open database");
    let entries = engine.scan_all_indexed_refnos().expect("scan index");

    for (noun, w0, w1) in WANTED {
        let refno = RefNo::new(*w0, *w1);
        let answered = engine
            .find_element(refno)
            .expect("find")
            .map(|view| view.raw_bytes.clone());
        println!("=== {noun} {refno} ===");
        let mut locations: Vec<(u32, u32)> = entries
            .iter()
            .filter(|(candidate, _, _)| *candidate == refno)
            .map(|(_, page, offset)| (*page, *offset))
            .collect();
        locations.sort_unstable();
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
            let is_answer = answered.as_deref() == Some(view.raw_bytes.as_slice());
            println!(
                "  page {:<6} offset {:<5} bytes={:<6} implicit_words={:<4} \
                 explicit={:<3} members={:<3}{}",
                page,
                offset,
                view.raw_bytes.len(),
                parsed.impl_len_words,
                parsed.explicit_attributes.len(),
                parsed.members.len(),
                if is_answer { "   <== answered" } else { "" }
            );
        }
    }
}
