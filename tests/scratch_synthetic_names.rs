//! Check how E3D builds a display name for an element that has none.
//!
//! The TTY sample shows `LCYLINDER 1 of GMSET /MSIFBDD-G13-4` and the nested
//! `NSCYLINDER 1 of SCYLINDER 1 of GMSET /GRZGG00-G3-8`, which reads as: the
//! noun's long name, the element's position among its owner's members, then
//! the owner's own display name — the same rule applied again when the owner
//! is nameless too.
//!
//! This prints the ingredients for a handful of sampled elements so the
//! ordinal rule (all members, or only same-noun members?) can be pinned down.
//!
//! Run with:
//! `cargo test --test scratch_synthetic_names -- --ignored --nocapture`

use std::path::Path;

use e3d_attlib::{db1_dehash, db1_hash};
use e3d_io::noun_catalog::NounCatalog;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const DB: &str = "ams7000_0001";
const LAYOUT: &str = "catalog/e3d31/noun_layout.json";
const FIELDS: &str = "catalog/e3d31/noun_attr_fields.json";

/// `(element, what E3D printed)`.
const CASES: &[(u32, u32, &str)] = &[
    (15192, 67352, "LCYLINDER 1 of GMSET /MSIFBDD-G13-4"),
    (15192, 67340, "PTCAR 1 of PTSET /MSIDBHP-P4-26"),
    (15192, 255017, "PTCAR 2 of PTSET /ANRC900-P3-1"),
    (
        15192,
        68747,
        "NSCYLINDER 1 of SCYLINDER 1 of GMSET /GRZGG00-G3-8",
    ),
    (15192, 67337, "PTAXIS 1 of PTSET /MSIDBHP-P4-26"),
];

fn explicit_name(parsed: &ParsedElement) -> Option<String> {
    let name_hash = db1_hash("NAME");
    parsed
        .explicit_attributes
        .iter()
        .rev()
        .find(|attribute| attribute.hash == name_hash)
        .and_then(|attribute| match &attribute.value {
            ExplicitValue::Text(text) if !text.trim().is_empty() => Some(text.clone()),
            _ => None,
        })
}

#[test]
#[ignore = "requires an installed E3D 3.1 AMS database"]
fn print_display_name_ingredients() {
    let path = Path::new(AMS).join(DB);
    if !path.exists() {
        eprintln!("SKIP: {} not present", path.display());
        return;
    }
    let catalog = NounCatalog::read(LAYOUT, FIELDS).ok();
    let mut engine = ReadOnlyEngine::open(&path).expect("open database");

    for (w0, w1, expected) in CASES {
        let refno = RefNo::new(*w0, *w1);
        println!("=== {refno}  E3D: {expected}");
        let mut current = refno;
        for depth in 0..4 {
            let Ok(Some(view)) = engine.find_element(current) else {
                println!("  [{depth}] {current} not found");
                break;
            };
            let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
                break;
            };
            let short = db1_dehash(parsed.noun_hash).unwrap_or_default();
            let long = catalog
                .as_ref()
                .and_then(|catalog| catalog.noun(parsed.noun_hash))
                .map(|noun| noun.noun.clone())
                .unwrap_or_default();
            println!(
                "  [{depth}] {current} noun={short} catalog_noun={long:?} \
                 name={:?} owner={} members={}",
                explicit_name(&parsed),
                parsed.owner,
                parsed.members.len()
            );

            // Where does this element sit in its owner's member list, counting
            // all members and counting only members of the same noun?
            if parsed.owner.word0 != 0 || parsed.owner.word1 != 0 {
                if let Ok(Some(owner_view)) = engine.find_element(parsed.owner) {
                    if let Ok(owner) = ParsedElement::parse(&owner_view.raw_bytes) {
                        let among_all = owner
                            .members
                            .iter()
                            .position(|member| *member == current)
                            .map(|index| index + 1);
                        let mut same_noun = 0usize;
                        let mut among_same = None;
                        for member in &owner.members {
                            let Ok(Some(view)) = engine.find_element(*member) else {
                                continue;
                            };
                            let Ok(child) = ParsedElement::parse(&view.raw_bytes) else {
                                continue;
                            };
                            if child.noun_hash != parsed.noun_hash {
                                continue;
                            }
                            same_noun += 1;
                            if *member == current {
                                among_same = Some(same_noun);
                            }
                        }
                        println!(
                            "        owner has {} members; this one is #{:?} of all, \
                             #{:?} of the {} with the same noun",
                            owner.members.len(),
                            among_all,
                            among_same,
                            same_noun
                        );
                    }
                }
            }

            if explicit_name(&parsed).is_some() {
                break;
            }
            if parsed.owner.word0 == 0 && parsed.owner.word1 == 0 {
                break;
            }
            current = parsed.owner;
        }
    }
}
