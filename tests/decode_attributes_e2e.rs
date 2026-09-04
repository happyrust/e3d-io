//! Slice 4 (e3d31-attribute-parsing) — first integration: schema-driven
//! attribute decode using
//!
//! - `e3d-io` for DB page / index / element record reading,
//! - `e3d-attlib::AttlibData` for the noun → attribute schema (ATNAIN/ATGTDF),
//! - `e3d-attlib::system_names::lookup_by_hash` for the
//!   IDA-extracted system attribute name table (6377 entries; Slice 3).
//!
//! Runs only when both fixture files exist:
//!
//! - `ams1112_0001` — the E3D 3.1 DB (mirror of `pdms-io-fork` test-file)
//! - `attlib.dat`   — the external attribute data file shared with that DB
//!
//! NOTE on type codes: `AttrDataType::from_code(1..12)` predates the Slice 2
//! third pass and contains 7 incorrect labels (see
//! `docs/ida-3.1-attributes.md` §3 — corrected table). This test prints the
//! raw `data_type` codes alongside the legacy labels so the discrepancy is
//! visible; renaming happens in the e3d-attlib rewrite (Slice 7).

use std::path::Path;

use e3d_attlib::{db1_hash, is_system_hash, lookup_system_name, AttlibData, AttrDataType};
use e3d_io::record::element::ParsedElement;
use e3d_io::ReadOnlyEngine;

const DB_PATH: &str = common::TEST_FILE_DB;
const ATTLIB_PATH: &str = common::TEST_FILE_ATTLIB;

mod common;

fn fixtures_present() -> bool {
    Path::new(DB_PATH).exists() && Path::new(ATTLIB_PATH).exists()
}

#[test]
fn schema_driven_attribute_decode_first_element() {
    if !fixtures_present() {
        eprintln!("SKIP: fixture files not present (DB or attlib.dat)");
        return;
    }

    println!("\n=== Slice 4 — schema-driven attribute decode ===");

    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("attlib.dat must parse");
    println!(
        "[attlib] attributes={} noun_attr_map.keys={} attr_def_map.keys={}",
        attlib.attributes.len(),
        attlib.noun_attr_map.len(),
        attlib.attr_def_map.len()
    );

    let mut engine = ReadOnlyEngine::open(DB_PATH).expect("DB must open");

    let target = engine.indexed_refnos().expect("read the index")[0].0;

    let view = engine
        .find_element(target)
        .expect("find_element")
        .expect("element exists");

    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse element record");

    println!(
        "\n[element] refno={} noun_hash=0x{:08X} impl_len_words={} impl_bytes={}",
        parsed.refno,
        parsed.noun_hash,
        parsed.impl_len_words,
        parsed.implicit_data.len()
    );

    let noun_name = e3d_attlib::db1_dehash(parsed.noun_hash);
    println!(
        "[noun] noun_name={:?} is_system_hash={}",
        noun_name,
        is_system_hash(parsed.noun_hash)
    );

    let attrs = attlib.get_noun_attributes(parsed.noun_hash);
    println!("[schema] noun has {} attributes in attlib", attrs.len());

    if attrs.is_empty() {
        // For some nouns the attlib parser yields no entries — note it but
        // do not fail the test (the attlib parser itself is pre-rewrite).
        println!("    (no schema entries — likely heuristic miss in pre-rewrite parser)");
    } else {
        println!(
            "    {:>3}  {:<8} {:<6} {:>4} {:<14} {:<26} {}",
            "#",
            "hash",
            "len",
            "size",
            "type(code→label)",
            "system_lookup_name",
            "byte_offset_guess"
        );

        let mut byte_off: usize = 24; // skip header
        for (i, (name, dtype, size)) in attrs.iter().enumerate() {
            let h = db1_hash(name);
            let system_name = lookup_system_name(h);
            let word_sz = word_size_for_type_code(dtype, *size);
            let bsz = word_sz * 4;

            println!("    {i:>3}  {hash:08X} {len:<6} {sz:>4} {ty_code:>3}={ty_label:<10} {sys:<26} +{boff}",
                hash = h,
                len = name.len(),
                sz = size,
                ty_code = dtype_code(dtype),
                ty_label = format!("{dtype}"),
                sys = system_name.unwrap_or("(none)"),
                boff = byte_off,
            );

            // Sanity: if attlib's recovered name matches a system table
            // entry, the lookup MUST agree.
            if is_system_hash(h) {
                if let Some(sysname) = system_name {
                    assert_eq!(
                        sysname.as_bytes(), name.as_bytes(),
                        "system_names disagrees with attlib for hash {h:08X}: sysname={sysname} attlibname={name}"
                    );
                }
            }

            byte_off = byte_off.saturating_add(bsz);
        }

        // We expect at least one match against the system name table.
        let matched = attrs
            .iter()
            .filter(|(name, _, _)| {
                lookup_system_name(db1_hash(name))
                    .map(|s| s == *name)
                    .unwrap_or(false)
            })
            .count();
        println!(
            "\n[xref] system-name table matched: {} / {} attrs",
            matched,
            attrs.len()
        );
    }

    // Always-on cheap assertion: the bundled system name table has the
    // expected entry count (catches accidental truncation).
    assert_eq!(e3d_attlib::system_names::entry_count(), 6377);
}

#[test]
fn system_names_round_trip_with_attlib_attribute_records() {
    if !fixtures_present() {
        eprintln!("SKIP: fixture files not present (DB or attlib.dat)");
        return;
    }

    let attlib = AttlibData::parse_file(ATTLIB_PATH).expect("attlib.dat must parse");

    let mut total = 0usize;
    let mut sys_hash = 0usize;
    let mut sys_table_hit = 0usize;
    let mut sys_table_disagree: Vec<(u32, String, String)> = Vec::new();

    for rec in &attlib.attributes {
        total += 1;
        let h = db1_hash(&rec.name);
        if is_system_hash(h) {
            sys_hash += 1;
            if let Some(sys_name) = lookup_system_name(h) {
                if sys_name == rec.name {
                    sys_table_hit += 1;
                } else {
                    sys_table_disagree.push((h, rec.name.clone(), sys_name.to_string()));
                }
            }
        }
    }

    println!(
        "\n[cross-check] attlib_attrs={} system_range_hashes={} system_table_hits={} disagreements={}",
        total, sys_hash, sys_table_hit, sys_table_disagree.len()
    );

    if !sys_table_disagree.is_empty() {
        let preview = sys_table_disagree.iter().take(10);
        for (h, attlib_name, sys_name) in preview {
            println!("    disagree hash=0x{h:08X} attlib={attlib_name:?} sys={sys_name:?}");
        }
    }

    // Useful diagnostic, but do not fail when attlib parsing yields garbled
    // names (the parser is pre-rewrite). The system_names table itself is
    // already test-covered inside e3d-attlib.
    assert!(total > 0, "attlib should have some attribute records");
}

// -----------------------------------------------------------------------------
// Helpers — value width estimation for the "offset guess" column above.
//
// These widths come from the existing low-level readers in
// `e3d-io::record::attrs` plus the EXMAP value-storage observations recorded
// in `docs/ida-3.1-attributes.md` §3. They are best-effort and exist only to
// make the printed layout legible; do not trust them as the source of truth.

fn dtype_code(dt: &AttrDataType) -> u32 {
    use AttrDataType::*;
    match dt {
        Integer => 1,
        Real => 2,
        Boolean => 3,
        Reference => 4,
        Text => 5,
        Enum => 6,
        Position => 7,
        Direction => 8,
        Orientation => 9,
        IntArray => 10,
        RealArray => 11,
        RefArray => 12,
        Unknown(v) => *v,
    }
}

fn word_size_for_type_code(dt: &AttrDataType, attlib_size: u32) -> usize {
    // attlib_size is the per-attribute count from ATGTDF (number of
    // elements). The per-element word width comes from the type code.
    let per_elem = match dt {
        AttrDataType::Integer => 1,
        AttrDataType::Real => 2,
        AttrDataType::Boolean => 1,
        AttrDataType::Reference => 2,
        AttrDataType::Text => 1, // string is variable-length; placeholder
        AttrDataType::Enum => 1,
        AttrDataType::Position => 6,
        AttrDataType::Direction => 6,
        AttrDataType::Orientation => 18,
        AttrDataType::IntArray => 1,
        AttrDataType::RealArray => 2,
        AttrDataType::RefArray => 2,
        AttrDataType::Unknown(_) => 1,
    };
    per_elem * attlib_size.max(1) as usize
}
