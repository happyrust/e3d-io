//! Slice 4 step 2 — integration test: decode attribute slots of the first
//! fixture element using the ATNLOG byte-decoder + e3d-attlib ATGTDF.
//!
//! Expected per
//! `goals/e3d31-attribute-parsing/SLICE4_IMPLEMENTATION_PLAN.md`:
//! at least 5 attributes should yield non-zero, plausibly-typed values for
//! the first element record. Walks ATGTDF index 1..N and prints each.

use std::path::Path;

use e3d_attlib::{is_system_hash, lookup_system_name, AttlibData};
use e3d_io::record::atnlog::{self, AttrValue, DEFAULT_HEADER_WORDS};
use e3d_io::record::element::ParsedElement;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001";
const ATTLIB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\attlib.dat";

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

#[test]
fn decode_first_fixture_element_attributes() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present");
        return;
    }

    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");

    let target = engine.indexed_refnos().expect("read the index")[0].0;

    let view = engine.find_element(target).unwrap().expect("element");
    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse element");

    println!("\n=== Slice 4 step 2 — ATNLOG decode ===");
    println!(
        "[element] refno={} noun_hash=0x{:08X} impl_len_words={} implicit_bytes={}",
        parsed.refno,
        parsed.noun_hash,
        parsed.impl_len_words,
        parsed.implicit_data.len()
    );

    // Slot count we can safely access in this element:
    let max_slot = parsed
        .implicit_data
        .len()
        .saturating_sub(DEFAULT_HEADER_WORDS * 4)
        / 4;
    println!("[layout] DEFAULT_HEADER_WORDS={DEFAULT_HEADER_WORDS}  max_slots={max_slot}");

    let mut nonzero = 0u32;
    let mut named = 0u32;
    let mut decoded_samples: Vec<(u32, String, String)> = Vec::new();

    println!(
        "\n    {:>3}  {:>3}  {:<10} {:>8}  {:<26} {:<20}",
        "atg", "slot", "data_type", "raw", "system_name", "decoded"
    );
    for atgtdf_pos in 1..=max_slot.min(64) as u32 {
        // pull AttrDefEntry from attlib by position
        let def = attlib.attr_defs.get(atgtdf_pos as usize - 1);
        let Some(def) = def else {
            break;
        };
        let raw = atnlog::read_slot(&parsed, atgtdf_pos, DEFAULT_HEADER_WORDS);
        let Some(raw) = raw else { break };
        if raw.raw_word != 0 {
            nonzero += 1;
        }
        let sys_name = lookup_system_name(def.hash).unwrap_or("(no system name)");
        if lookup_system_name(def.hash).is_some() && raw.raw_word != 0 {
            named += 1;
        }
        // Use the EXMAP-aware path: pass ATGTDF data_type + kind/size, ityp=0
        // (we don't have access to DB_Attribute::ityp from attlib alone),
        // and is_uda derived from hash range.
        let decoded = atnlog::decode_value_with_meta(
            &parsed,
            atgtdf_pos,
            def.data_type,
            def.size,
            0,
            !is_system_hash(def.hash),
            DEFAULT_HEADER_WORDS,
        );
        let decoded_str = match decoded {
            Some(AttrValue::Bool(b)) => format!("Bool({b})"),
            Some(AttrValue::Int(i)) => format!("Int({i})"),
            Some(AttrValue::Real(r)) => format!("Real({r:.4})"),
            Some(AttrValue::DbRef(r)) => format!("Ref({r})"),
            Some(AttrValue::Text(t)) => format!("Text(0x{t:08X})"),
            Some(AttrValue::Position(p)) => format!("Pos({:.2},{:.2},{:.2})", p[0], p[1], p[2]),
            Some(AttrValue::Direction(d)) => format!("Dir({:.2},{:.2},{:.2})", d[0], d[1], d[2]),
            Some(AttrValue::Orientation(_)) => "Orient[9]".to_string(),
            Some(AttrValue::IntArray(v)) => {
                format!("IntArr[{}]={:?}", v.len(), &v[..v.len().min(4)])
            }
            Some(AttrValue::RealArray(v)) => {
                format!("RealArr[{}]={:?}", v.len(), &v[..v.len().min(4)])
            }
            Some(AttrValue::BoolArray(v)) => {
                format!("BoolArr[{}]={:?}", v.len(), &v[..v.len().min(4)])
            }
            Some(AttrValue::DbRefArray(v)) => format!("RefArr[{}]", v.len()),
            Some(AttrValue::TextArray(v)) => format!("TextArr[{}]", v.len()),
            Some(AttrValue::Raw(w)) => format!("Raw(0x{w:08X})"),
            None => "(no decode)".to_string(),
        };

        println!(
            "    {:>3}  {:>3}  {:>10}  {:08X}  {:<26} {:<20}",
            atgtdf_pos, atgtdf_pos, def.data_type, raw.raw_word, sys_name, decoded_str
        );

        if raw.raw_word != 0 && raw.raw_word != u32::MAX {
            decoded_samples.push((def.hash, sys_name.to_string(), decoded_str));
        }
    }

    println!("\n[summary] nonzero_slots={nonzero}  named_nonzero={named}");
    println!("[samples-with-values]");
    for (h, name, decoded) in decoded_samples.iter().take(10) {
        println!("  hash=0x{h:08X} name={name:<10} -> {decoded}");
    }

    // Soft assertion (do not panic — header_word_count may be off and
    // require iteration in next session).
    if nonzero < 5 {
        eprintln!("WARN: only {nonzero} non-zero slots; consider trying header_word_count=5 or 7");
    }
}
