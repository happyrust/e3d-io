//! Slice 4 step 5 — scan the fixture for elements with larger
//! implicit data (likely richer attribute schemas) and decode their
//! first ~32 slots via the EXMAP-aware ATNLOG path.
//!
//! Goal: surface a sample of attributes with meaningful decoded values
//! beyond the small NXTR case used elsewhere.

use std::path::Path;

use e3d_attlib::{db1_dehash, is_system_hash, lookup_system_name, AttlibData};
use e3d_io::record::atnlog::{self, AttrValue, DEFAULT_HEADER_WORDS};
use e3d_io::record::element::ParsedElement;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001";
const ATTLIB_PATH: &str = r"D:\work\plant-code\pdms-io-fork\test-file\attlib.dat";

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

#[test]
fn decode_a_larger_fixture_element() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present");
        return;
    }

    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("DB");

    let refnos = engine.scan_all_indexed_refnos().expect("scan refnos");
    println!("\n=== Slice 4 step 5 — scan for richer element ===");
    println!("[db] {} indexed refnos", refnos.len());

    // Iterate refnos, find first element with implicit_data > 256 bytes
    // (=> > 64 words; lots of attribute slots).
    let mut best: Option<(ParsedElement, e3d_io::refno::RefNo)> = None;
    let mut scanned = 0usize;
    for &(refno, page, off) in refnos.iter() {
        scanned += 1;
        if scanned > 2000 {
            break;
        }
        let view = match engine.read_element_direct(refno, page, off) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let parsed = match ParsedElement::parse(&view.raw_bytes) {
            Ok(p) if p.refno == refno => p,
            _ => continue,
        };
        if parsed.implicit_data.len() >= 256 {
            best = Some((parsed, refno));
            break;
        }
    }

    let Some((parsed, refno)) = best else {
        eprintln!("no richer element found in first {scanned} refnos");
        return;
    };

    println!(
        "[element] refno={refno} noun_hash=0x{:08X} noun_name={:?} impl_len_words={} implicit_bytes={}",
        parsed.noun_hash,
        db1_dehash(parsed.noun_hash),
        parsed.impl_len_words,
        parsed.implicit_data.len()
    );

    let max_slot = parsed
        .implicit_data
        .len()
        .saturating_sub(DEFAULT_HEADER_WORDS * 4)
        / 4;
    println!("[layout] DEFAULT_HEADER_WORDS={DEFAULT_HEADER_WORDS}  max_slots={max_slot}");

    let mut nonzero = 0u32;
    let mut named_nonzero: Vec<(u32, String, String)> = Vec::new();

    println!(
        "\n    {:>3}  {:>3}  {:<10} {:>8}  {:<26} {:<24}",
        "atg", "slot", "data_type", "raw", "system_name", "decoded"
    );
    for atgtdf_pos in 1..=max_slot.min(64) as u32 {
        let Some(def) = attlib.attr_defs.get(atgtdf_pos as usize - 1) else {
            break;
        };
        let raw = match atnlog::read_slot(&parsed, atgtdf_pos, DEFAULT_HEADER_WORDS) {
            Some(r) => r,
            None => break,
        };
        if raw.raw_word != 0 {
            nonzero += 1;
        }
        let sys_name = lookup_system_name(def.hash).unwrap_or("(no system name)");
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
            "    {:>3}  {:>3}  {:>10}  {:08X}  {:<26} {:<24}",
            atgtdf_pos, atgtdf_pos, def.data_type, raw.raw_word, sys_name, decoded_str
        );
        if raw.raw_word != 0 && raw.raw_word != u32::MAX && lookup_system_name(def.hash).is_some() {
            named_nonzero.push((def.hash, sys_name.to_string(), decoded_str));
        }
    }

    println!(
        "\n[summary] nonzero_slots={nonzero}  named_nonzero={}",
        named_nonzero.len()
    );
    for (h, name, decoded) in named_nonzero.iter().take(20) {
        println!("  hash=0x{h:08X} name={name:<10} -> {decoded}");
    }
}
