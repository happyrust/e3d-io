//! Slice 4 step 11 — default-value pool path validated end-to-end on the
//! fixture: locate an element slot that holds `RAW_USE_DEFAULT`, inject a
//! default for the corresponding attribute hash via
//! `AttlibData::set_attr_default`, and assert that `summarize_element`
//! surfaces the injected default instead of the sentinel.
//!
//! See `goals/e3d31-attribute-parsing/progress.jsonl` entry #20 for the
//! IDA evidence behind the default-pool path, and entry #32+ for this
//! end-to-end demonstration.

use std::path::Path;

use e3d_attlib::{AttlibData, AttrDefault};
use e3d_io::record::atnlog::{
    decode_attribute_by_hash, AttrValue, DEFAULT_HEADER_WORDS, RAW_USE_DEFAULT,
};
use e3d_io::record::element::ParsedElement;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = common::TEST_FILE_DB;
const ATTLIB_PATH: &str = common::TEST_FILE_ATTLIB;

mod common;

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

/// Scan a `ParsedElement`'s slot area for the first slot that reads
/// `RAW_USE_DEFAULT` (`0xFFFFFFFF`) AND whose 1-based position maps to
/// a unique ATGTDF entry in `attlib`. Returns the `(attlib_hash,
/// atgtdf_position)` pair so the caller can inject a default and then
/// verify the decoder substitutes it.
fn find_default_sentinel_slot(parsed: &ParsedElement, attlib: &AttlibData) -> Option<(u32, usize)> {
    // attr_defs_unique mirrors what summarize_element iterates, so we
    // only need to consider positions that the summary loop would visit.
    for (idx, def) in attlib.attr_defs_unique().enumerate() {
        let position = idx + 1; // 1-based
        let word_idx = DEFAULT_HEADER_WORDS + (position - 1);
        let byte_off = word_idx * 4;
        if byte_off + 4 > parsed.implicit_data.len() {
            break;
        }
        let bytes = &parsed.implicit_data[byte_off..byte_off + 4];
        let raw = u32::from_be_bytes(bytes.try_into().unwrap());
        if raw == RAW_USE_DEFAULT {
            return Some((def.hash, position));
        }
    }
    None
}

#[test]
fn default_pool_substitutes_for_raw_use_default_slot() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present at {DB_PATH} / {ATTLIB_PATH}");
        return;
    }

    let mut attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");

    let indexed = engine.indexed_refnos().expect("read the index");

    // Find the first indexed element that has at least one slot which
    // reads RAW_USE_DEFAULT AND maps to an ATGTDF entry. Walk
    // multiple elements so the test isn't brittle to one element's
    // schema not happening to default anything.
    let mut found = None;
    for &(refno, _, _) in &indexed {
        let view = match engine.find_element(refno) {
            Ok(Some(v)) => v,
            _ => continue,
        };
        let parsed = match ParsedElement::parse(&view.raw_bytes) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if let Some((hash, pos)) = find_default_sentinel_slot(&parsed, &attlib) {
            found = Some((refno, hash, pos, parsed));
            break;
        }
    }

    let (refno, attr_hash, position, parsed) = match found {
        Some(t) => t,
        None => {
            eprintln!(
                "SKIP: no RAW_USE_DEFAULT slot mapped to an ATGTDF entry \
                 was found in the first {} leaf entries — fixture-specific",
                indexed.len()
            );
            return;
        }
    };

    println!(
        "[default-pool] using refno={refno} attr_hash=0x{attr_hash:08X} \
         atgtdf_position={position}"
    );

    // Sanity: without injection, the decoder should return either
    // Raw(RAW_USE_DEFAULT), Int(-1), Real(NaN/abnormal), or similar
    // — whichever the generic path produces for the attr's type.
    let before = decode_attribute_by_hash(
        &parsed,
        &attlib,
        attr_hash,
        /* follow_syno = */ false,
        DEFAULT_HEADER_WORDS,
    )
    .expect("decode without default")
    .expect("some value");
    println!("[default-pool] before injection: {before:?}");

    // Inject a distinctive default and re-decode. The decoder must
    // return our injected value, not the sentinel decode.
    let injected = AttrDefault::Int(0x42_42_42);
    assert!(
        attlib.set_attr_default(attr_hash, injected),
        "set_attr_default must succeed for a hash that came from attr_defs_unique"
    );

    let after = decode_attribute_by_hash(&parsed, &attlib, attr_hash, false, DEFAULT_HEADER_WORDS)
        .expect("decode with default")
        .expect("some value");
    println!("[default-pool] after  injection: {after:?}");

    match after {
        AttrValue::Int(0x42_42_42) => {}
        other => panic!("default pool not applied: expected Int(0x424242), got {other:?}"),
    }
}

/// Step 13: after `e3d-attlib` started auto-populating
/// `AttrDefEntry::default_value` for every `kind == 2 && type != 4`
/// ATGTDF entry, `summarize_element` should surface the real default
/// (from the on-disk pool) automatically — no `set_attr_default`
/// injection needed.
#[test]
fn auto_populated_default_pool_substitutes_on_fixture() {
    if !fixtures_present() {
        eprintln!("SKIP: fixtures not present");
        return;
    }

    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("parse attlib");
    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("open DB");

    let indexed = engine.indexed_refnos().expect("read the index");

    // Walk indexed elements until we find one with a RAW_USE_DEFAULT
    // slot whose attr_def carries an auto-populated default. Demand
    // that the decoded value match what `attr_default_to_value(default)`
    // would yield from the attlib's default field — proving the
    // auto-population path end-to-end.
    let mut found = None;
    for &(refno, _, _) in &indexed {
        let view = match engine.find_element(refno) {
            Ok(Some(v)) => v,
            _ => continue,
        };
        let parsed = match ParsedElement::parse(&view.raw_bytes) {
            Ok(p) => p,
            Err(_) => continue,
        };
        // For each unique attr_def, check if the corresponding slot
        // reads RAW_USE_DEFAULT AND the attr_def already carries a
        // default_value populated by parse_file.
        for (idx, def) in attlib.attr_defs_unique().enumerate() {
            let position = idx + 1;
            let word_idx = DEFAULT_HEADER_WORDS + (position - 1);
            let byte_off = word_idx * 4;
            if byte_off + 4 > parsed.implicit_data.len() {
                break;
            }
            let bytes = &parsed.implicit_data[byte_off..byte_off + 4];
            let raw = u32::from_be_bytes(bytes.try_into().unwrap());
            if raw == RAW_USE_DEFAULT && def.default_value.is_some() {
                found = Some((refno, def.hash, def.default_value.unwrap(), parsed));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }

    let (refno, attr_hash, default_from_attlib, parsed) = match found {
        Some(t) => t,
        None => {
            eprintln!(
                "SKIP: no element-slot combo with RAW_USE_DEFAULT + populated default \
                 was found — fixture-specific (parse_file may have left default_value None \
                 for every relevant attr)"
            );
            return;
        }
    };

    println!(
        "[auto-default] refno={refno} attr_hash=0x{attr_hash:08X} default={default_from_attlib:?}"
    );

    let decoded =
        decode_attribute_by_hash(&parsed, &attlib, attr_hash, false, DEFAULT_HEADER_WORDS)
            .expect("decode")
            .expect("some value");

    println!("[auto-default] decoded={decoded:?}");

    // The decoded value must equal what attr_default_to_value would
    // emit for the auto-populated default — proving the auto-population
    // path connects end-to-end without any test-side injection.
    let expected = e3d_io::record::atnlog::attr_default_to_value(default_from_attlib);
    let matches = match (&decoded, &expected) {
        (AttrValue::Int(a), AttrValue::Int(b)) => a == b,
        (AttrValue::Bool(a), AttrValue::Bool(b)) => a == b,
        (AttrValue::Raw(a), AttrValue::Raw(b)) => a == b,
        (AttrValue::Real(a), AttrValue::Real(b)) => (a - b).abs() < 1e-12,
        _ => false,
    };
    assert!(
        matches,
        "decoded value {decoded:?} did not match expected default {expected:?} from attlib"
    );
}
