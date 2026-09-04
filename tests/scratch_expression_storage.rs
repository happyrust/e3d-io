//! Where the catalogue keeps the value E3D prints as `( 0.75 * ATTRIB PARA[5 ] )`.
//!
//! Twenty-odd sampled rows read as an all-zero tuple, a zero real or nothing at
//! all while E3D prints a PML expression. The expression is in the record: it
//! sits in the explicit stream under a hash above `0xF000_0000`, which the
//! reader currently writes off as a UDA it has no definition for.
//!
//! Emits TSV so the blobs can be joined against E3D's own rendering:
//! `refno<TAB>noun<TAB>ordinal<TAB>hash<TAB>words`, plus an `ATTR` line per
//! template attribute giving its hash and slot so the two can be matched up.
//!
//! Run with:
//! `cargo test --test scratch_expression_storage -- --ignored --nocapture`

use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::attlib_metadata::AttributeMetadataCatalog;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::record::template::TemplateProvider;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

mod common;

const SHADOW: &str = common::SCHEMA_BINARIES;
const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

/// Every sampled element whose TTY output holds a PML expression.
const TARGETS: &[(&str, u32, u32)] = &[
    ("BLTP", 15192, 85075),
    ("BLTP", 15192, 254839),
    ("BTSE", 15192, 85074),
    ("BTSE", 15192, 254838),
    ("LPYR", 15192, 255858),
    ("NSCY", 15192, 255285),
    ("SRTO", 15192, 258935),
    ("SRTO", 15192, 210472),
    ("SEXT", 15192, 222422),
    ("SEXT", 15192, 279891),
    ("DATA", 15192, 254649),
    ("DATA", 15192, 166914),
    ("PTMI", 15192, 258928),
    ("SVER", 15192, 279893),
    ("TABHQU", 15192, 226176),
    ("TABQUE", 15192, 226166),
];

/// The hash above which the explicit stream stops being a real attribute.
const UDA_HASH_FLOOR: u32 = 0xF000_0000;

#[test]
#[ignore = "requires the installed E3D 3.1 sample project"]
fn dump_expression_blobs_and_attribute_slots() {
    let attlib_path = format!("{SHADOW}\\attlib.dat");
    let attlib = AttlibData::parse_file(&attlib_path).expect("parse attlib");
    let metadata =
        AttributeMetadataCatalog::parse_file(&attlib_path, &attlib).expect("attlib metadata");
    let mut provider =
        TemplateProvider::open(format!("{SHADOW}\\catvir.dat"), Some(db1_hash("CATA")))
            .expect("open catalogue template");
    let mut engine = ReadOnlyEngine::open(format!("{AMS}\\ams7000_0001")).expect("open database");

    for (noun, word0, word1) in TARGETS {
        let refno = RefNo::new(*word0, *word1);
        let Some(view) = engine.find_element(refno).expect("find element") else {
            eprintln!("SKIP {refno}: not in ams7000_0001");
            continue;
        };
        let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse element");

        if let Ok(Some(template)) = provider.template_for(parsed.noun_hash) {
            for attr in &template.attrs {
                if !attr.authoritative {
                    continue;
                }
                let Some(report) = metadata.report_name(attr.hash) else {
                    continue;
                };
                println!(
                    "ATTR\t{refno}\t{noun}\t{report}\t0x{:08X}\t{}",
                    attr.hash,
                    attr.wide.location.word_offset()
                );
            }
        }

        let mut ordinal = 0;
        for attribute in &parsed.explicit_attributes {
            if attribute.hash < UDA_HASH_FLOOR {
                continue;
            }
            let ExplicitValue::IntArray(words) = &attribute.value else {
                continue;
            };
            println!(
                "BLOB\t{refno}\t{noun}\t{ordinal}\t0x{:08X}\t{}",
                attribute.hash,
                words
                    .iter()
                    .map(|word| word.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            ordinal += 1;
        }
    }
}
