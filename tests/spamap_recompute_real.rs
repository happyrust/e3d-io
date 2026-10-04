//! `spamap.recompute.<NOUN>` against what it was read from
//! (`docs/evidence/2026-10-03-spamap-recompute.md`): `rules::RECOMPUTING` is
//! the set of attributes of the Geometric_Noun templates
//! (`schema/e3d31/catalog/noun_layout.json`) whose `DCHC` in the E3D 3.1
//! `attlib.dat` is not 0 and that are not `WNOEVT`, and `recompute_row` agrees
//! with it on every template attribute. Printed for the evidence
//! (requirement 5.2): each Geometric_Noun's table, the attributes that trigger
//! and those that do not.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::ATTLIB;
use e3d_attlib::AttrDict;
use e3d_io::core_rules::{member_order_row, recompute_row, Trigger};
use e3d_io::spatial::rules::{self, imap_of};

const LAYOUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/catalog/noun_layout.json");

type Template = (u32, Vec<(String, u32)>);

#[test]
fn ancestor_trigger_table_matches_the_dictionary() {
    let dictionary = AttrDict::parse_file(ATTLIB).expect("required dictionary");
    let layout: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(LAYOUT).unwrap()).unwrap();
    let mut extra = BTreeSet::new();
    let mut mismatches = Vec::new();
    for noun in layout.as_array().unwrap() {
        let hash = noun["hash"].as_u64().unwrap() as u32;
        for attr in noun["attrs"].as_array().unwrap() {
            let h = attr["hash"].as_u64().unwrap() as u32;
            let trigger = dictionary.dchc(h).is_some_and(|d| d != 0) && !dictionary.wnoevt(h);
            if trigger && !rules::RECOMPUTING.iter().any(|(_, listed)| *listed == h) { extra.insert(h); }
            let expected = if trigger { Trigger::Triggers } else { Trigger::DoesNotTrigger };
            if recompute_row(hash, h) != expected { mismatches.push((noun["short"].clone(), attr["name"].clone())); }
        }
    }
    assert_eq!(extra.iter().copied().collect::<Vec<_>>(), rules::ANCESTOR_RECOMPUTING);
    assert!(mismatches.is_empty(), "{mismatches:?}");
}

/// Each Geometric_Noun (`IMAP` 1, 5 or 7) by short name, with its hash and its
/// template's attributes, as the layout E3D exported names them.
fn geometric_templates() -> BTreeMap<String, Template> {
    let layout: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(LAYOUT).expect("read the noun layout")).expect("parse it");
    let mut templates = BTreeMap::new();
    for noun in layout.as_array().expect("an array of nouns") {
        let hash = noun["hash"].as_u64().expect("a noun hash") as u32;
        if !matches!(imap_of(hash), 1 | 5 | 7) {
            continue;
        }
        let attrs = noun["attrs"]
            .as_array()
            .expect("its attributes")
            .iter()
            .map(|attr| (attr["name"].as_str().expect("a name").to_owned(), attr["hash"].as_u64().expect("a hash") as u32))
            .collect();
        templates.insert(noun["short"].as_str().expect("a short name").to_owned(), (hash, attrs));
    }
    templates
}

#[test]
fn the_recomputing_attributes_are_the_dictionarys() {
    if common::missing(&[ATTLIB, LAYOUT]) {
        return;
    }
    let dictionary = AttrDict::parse_file(ATTLIB).expect("read the attribute dictionary");
    let templates = geometric_templates();
    assert_eq!(templates.len(), rules::IMAP_1.len() + rules::IMAP_5.len() + rules::IMAP_7.len(), "every Geometric_Noun has a layout");

    let mut read = BTreeSet::new();
    for (noun, (_, attrs)) in &templates {
        for (name, hash) in attrs {
            assert!(!dictionary.wnoevt(*hash), "{noun} {name} is WNOEVT");
            let dchc = dictionary.dchc(*hash).unwrap_or_else(|| panic!("{noun} {name} has no DCHC"));
            if dchc != 0 {
                read.insert((name.clone(), *hash));
            }
        }
    }
    let listed: BTreeSet<(String, u32)> = rules::RECOMPUTING.iter().map(|&(name, hash)| (name.to_owned(), hash)).collect();
    assert_eq!(listed, read);

    for (noun, (noun_hash, attrs)) in &templates {
        let noun_hash = *noun_hash;
        assert_eq!(member_order_row(noun_hash), Trigger::Triggers, "{noun}");
        let (mut triggering, mut not) = (Vec::new(), Vec::new());
        for (name, hash) in attrs {
            let expected = if read.contains(&(name.clone(), *hash)) { Trigger::Triggers } else { Trigger::DoesNotTrigger };
            assert_eq!(recompute_row(noun_hash, *hash), expected, "{noun} {name}");
            if expected == Trigger::Triggers { triggering.push(name.as_str()) } else { not.push(name.as_str()) }
        }
        eprintln!("{noun}: triggers {} (+ member order); does not {}", triggering.join(" "), not.join(" "));
    }
    eprintln!("{} Geometric_Nouns, {} attributes that recompute", templates.len(), read.len());
}
