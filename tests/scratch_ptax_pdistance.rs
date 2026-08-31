//! Where `PTAX.Pdistance` keeps the formula E3D prints for it.
//!
//! The implicit slot holds the all-zero tuple and E3D prints
//! `( ATTRIB PARA[3 ] * TAN ( 27 ) )`, which is the shape
//! [`e3d_io::record::catalogue_pml`] was written for: the body sits in the
//! explicit stream under the two's-complement negation of the attribute's own
//! hash. The row is still reported as `decoded` rather than `decoded_explicit`,
//! so either the entry is not there or the renderer declines it.
//!
//! This dumps the explicit stream of the sampled elements and says which.
//!
//! Run with:
//! `cargo test --test scratch_ptax_pdistance -- --ignored --nocapture`

use e3d_attlib::{db1_dehash, db1_hash, system_names};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

/// `(database, refno, what E3D prints for Pdistance)`.
const CASES: &[(&str, (u32, u32), &str)] = &[
    (
        "ams5053_0001",
        (13245, 515477),
        "( ATTRIB PARA[3 ] * TAN ( 27 ) )",
    ),
    (
        "ams5052_0001",
        (13244, 95067),
        "( ATTRIB RADI * TAN ( ATTRIB ANGL / 2 ) )",
    ),
    (
        "ams5052_0001",
        (13244, 88917),
        "( ATTRIB RPRO RADI * TAN ( 45 / 2 ) + ATTRIB PARA[3 ] )",
    ),
];

fn name_of(hash: u32) -> String {
    system_names::lookup_by_hash(hash)
        .map(String::from)
        .or_else(|| db1_dehash(hash))
        .unwrap_or_else(|| "?".to_string())
}

fn annotate(words: &[i32]) -> String {
    words
        .iter()
        .map(|&word| {
            let word = word as u32;
            match system_names::lookup_by_hash(word) {
                Some(name) => format!("{}={name}", word as i32),
                None => match db1_dehash(word) {
                    Some(name) if name.len() >= 3 => format!("{}~{name}", word as i32),
                    _ => format!("{}", word as i32),
                },
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
#[ignore = "requires the installed E3D 3.1 catalogue project"]
fn report_where_the_pdistance_formula_lives() {
    let wanted = db1_hash("PDIS");
    let key = wanted.wrapping_neg();
    println!("PDIS = {wanted} ({wanted:#x}), formula key = {key} ({key:#x})\n");

    for (db, (word0, word1), printed) in CASES {
        let path = format!("{AMS}\\{db}");
        if !std::path::Path::new(&path).exists() {
            eprintln!("SKIP: {path} not present");
            return;
        }
        let refno = RefNo::new(*word0, *word1);
        let mut engine = ReadOnlyEngine::open(&path).expect("open database");
        let Ok(Some(view)) = engine.find_element(refno) else {
            println!("{db} {refno}: not in the index");
            continue;
        };
        let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse element");
        println!(
            "=== {db} {refno} noun {} ({}) — E3D prints {printed}",
            parsed.noun_hash,
            name_of(parsed.noun_hash)
        );
        println!("  {} explicit entries:", parsed.explicit_attributes.len());
        for attribute in &parsed.explicit_attributes {
            let hash = attribute.hash;
            let negated = hash.wrapping_neg();
            let label = match hash == key {
                true => "  <-- PDIS formula".to_string(),
                false => format!(
                    " (as itself {}, negated {})",
                    name_of(hash),
                    name_of(negated)
                ),
            };
            let body = match &attribute.value {
                ExplicitValue::IntArray(words) => annotate(words),
                other => format!("{other:?}"),
            };
            println!("    {hash:>12} {hash:#010x}{label}");
            println!("        {body}");
            if hash == key {
                if let ExplicitValue::IntArray(words) = &attribute.value {
                    println!(
                        "        catalogue_pml says: {:?}",
                        e3d_io::record::catalogue_pml::render(words)
                    );
                }
            }
        }
        println!();
    }
}
