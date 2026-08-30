//! Every stored copy of the four elements whose implicit word count no longer
//! matches `evidence/20260826-tty-sampling/sample.tsv`.
//!
//! Run with:
//! `cargo test --test scratch_disputed_copies -- --ignored --nocapture`

use std::path::Path;

use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const DB: &str = "ams7000_0001";
const WANTED: &[(&str, u32, u32, i32)] = &[
    ("SPEC", 15192, 62134, 24),
    ("CATE", 15192, 521, 25),
    ("SDTE", 15192, 522, 15),
    ("NOMINB", 15192, 222761, 13),
];

/// Is the UDA key that moved (398835873 -> 642952083) a second copy of the
/// definition element rather than a re-allocation?
#[test]
#[ignore = "requires an installed E3D 3.1 AMS database"]
fn print_uda_definition_copies() {
    let path = Path::new(AMS).join("ams5100_0001");
    if !path.exists() {
        eprintln!("SKIP: {} not present", path.display());
        return;
    }
    let mut engine = ReadOnlyEngine::open(&path).expect("open dictionary");
    let entries = engine.indexed_refnos().expect("scan index");
    let refno = RefNo::new(13292, 23);
    let answered = engine
        .find_element(refno)
        .expect("find")
        .map(|view| view.raw_bytes.clone());
    let mut locations: Vec<(u32, u32)> = entries
        .iter()
        .filter(|(candidate, _, _)| *candidate == refno)
        .map(|(_, page, offset)| (*page, *offset))
        .collect();
    locations.sort_unstable();
    locations.dedup();
    println!("=== UDA definition {refno} (H-AIRQUANTITY) ===");
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
        let word = |index: usize| {
            parsed
                .implicit_data
                .get(index * 4..index * 4 + 4)
                .map(|bytes| u32::from_be_bytes(bytes.try_into().unwrap()))
        };
        println!(
            "  page {:<6} offset {:<5} implicit_words={:<4} explicit={:<3} w11={:?} w12={:?}{}",
            page,
            offset,
            parsed.impl_len_words,
            parsed.explicit_attributes.len(),
            word(11),
            word(12),
            if answered.as_deref() == Some(view.raw_bytes.as_slice()) {
                "   <== answered"
            } else {
                ""
            }
        );
        for candidate in [398_835_873u32, 642_952_083u32] {
            let bytes = candidate.to_be_bytes();
            let hits: Vec<usize> = view
                .raw_bytes
                .windows(4)
                .enumerate()
                .filter_map(|(at, window)| (window == bytes).then_some(at))
                .collect();
            println!("      0x{candidate:08X} at {hits:?}");
        }
    }
}

#[test]
#[ignore = "requires an installed E3D 3.1 AMS database"]
fn print_disputed_copies() {
    let path = Path::new(AMS).join(DB);
    if !path.exists() {
        eprintln!("SKIP: {} not present", path.display());
        return;
    }
    let mut engine = ReadOnlyEngine::open(&path).expect("open database");
    let entries = engine.indexed_refnos().expect("scan index");

    for (noun, w0, w1, sampled) in WANTED {
        let refno = RefNo::new(*w0, *w1);
        let answered = engine
            .find_element(refno)
            .expect("find")
            .map(|view| view.raw_bytes.clone());
        println!("=== {noun} {refno}  sample recorded {sampled} implicit words ===");
        let mut locations: Vec<(u32, u32)> = entries
            .iter()
            .filter(|(candidate, _, _)| *candidate == refno)
            .map(|(_, page, offset)| (*page, *offset))
            .collect();
        locations.sort_unstable();
        locations.dedup();
        for (page, offset) in locations {
            let Ok(view) = engine.read_element_direct(refno, page, offset) else {
                println!("  page {page:<6} offset {offset:<5} unreadable");
                continue;
            };
            let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
                println!("  page {page:<6} offset {offset:<5} unparsable");
                continue;
            };
            if parsed.refno != refno {
                println!(
                    "  page {page:<6} offset {offset:<5} holds {} instead",
                    parsed.refno
                );
                continue;
            }
            let is_answer = answered.as_deref() == Some(view.raw_bytes.as_slice());
            println!(
                "  page {:<6} offset {:<5} bytes={:<6} implicit_words={:<4} \
                 explicit={:<3} members={:<4} owner={}{}{}",
                page,
                offset,
                view.raw_bytes.len(),
                parsed.impl_len_words,
                parsed.explicit_attributes.len(),
                parsed.members.len(),
                parsed.owner,
                if parsed.impl_len_words == *sampled {
                    "   [sampled shape]"
                } else {
                    ""
                },
                if is_answer { "   <== answered" } else { "" }
            );
        }
    }
}
