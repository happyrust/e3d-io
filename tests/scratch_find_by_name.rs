//! Locate elements by their `Name` attribute so a pasted `q att` capture can be
//! turned into a comparison run.
//!
//! Run with:
//! `cargo test --test scratch_find_by_name -- --ignored --nocapture`

use std::collections::BTreeSet;
use std::path::Path;

use e3d_attlib::db1_hash;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::ReadOnlyEngine;

const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const DB: &str = "ams7999_0001";
const WANTED: &[&str] = &["/R332.001-BRAN-02", "/1RCV0102A"];

#[test]
#[ignore = "requires an installed E3D 3.1 AMS database"]
fn find_elements_by_name() {
    let path = Path::new(AMS).join(DB);
    if !path.exists() {
        eprintln!("SKIP: {} not present", path.display());
        return;
    }
    let mut engine = ReadOnlyEngine::open(&path).expect("open database");
    let entries = engine.indexed_refnos().expect("scan index");
    println!("{DB}: {} indexed entries", entries.len());

    let name_hash = db1_hash("NAME");
    let mut hits: BTreeSet<String> = BTreeSet::new();

    for (refno, page, offset) in &entries {
        let Ok(view) = engine.read_element_direct(*refno, *page, *offset) else {
            continue;
        };
        let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
            continue;
        };
        if parsed.refno != *refno {
            continue;
        }
        for attribute in &parsed.explicit_attributes {
            if attribute.hash != name_hash {
                continue;
            }
            let ExplicitValue::Text(text) = &attribute.value else {
                continue;
            };
            if WANTED.iter().any(|wanted| wanted.eq_ignore_ascii_case(text)) {
                hits.insert(format!(
                    "{text}  refno={refno}  noun=0x{:08X} ({})  page={page} offset={offset} \
                     explicit={} members={}",
                    parsed.noun_hash,
                    e3d_attlib::db1_dehash(parsed.noun_hash).unwrap_or_default(),
                    parsed.explicit_attributes.len(),
                    parsed.members.len()
                ));
            }
        }
    }

    println!("命中 {} 条：", hits.len());
    for hit in &hits {
        println!("  {hit}");
    }
}
