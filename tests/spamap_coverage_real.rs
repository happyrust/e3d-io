//! `spamap.coverage` against what it was read from
//! (`docs/evidence/2026-10-03-spamap-coverage.md`): the `IMAP` and `HNGT`
//! lists in `src/spatial/rules.rs` are those of the E3D 3.1 `attlib.dat`, the
//! nouns without SPAMAP are those of the design template, and — printed for
//! the evidence (requirement 5.10) — every noun of the templates the Corpus
//! uses, with what the rule makes of it and how many elements E3D wrote of it
//! in each Corpus database, and how many of those have entries.

mod common;

use std::collections::{BTreeMap, HashSet};

use common::corpus::CORPUS;
use common::{ATTLIB, CATVIR, DESVIR};
use e3d_attlib::{db1_dehash, db1_hash};
use e3d_io::engine::ScanTier;
use e3d_io::index::{read_spamap, SpamapRead};
use e3d_io::record::template::TemplateProvider;
use e3d_io::record::template_file::TemplateFile;
use e3d_io::session::{DbView, SessionSelector};
use e3d_io::spatial::check::Authors;
use e3d_io::spatial::noun_fields::NounFields;
use e3d_io::spatial::rules::{self, imap_of};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

fn name(noun: u32) -> String {
    db1_dehash(noun).unwrap_or_else(|| format!("#{noun}"))
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    names.sort();
    names
}

fn template_nouns(path: &str) -> Vec<u32> {
    let mut file = TemplateFile::open(path).expect("open the template");
    let header = file.read_header().expect("read its header");
    file.read_children(&header).expect("read its children").iter().map(|child| child.hash).collect()
}

/// What the rule makes of a noun's elements, by noun alone.
fn class(noun: u32) -> &'static str {
    let listed = |names: &[&str]| names.iter().any(|name| db1_hash(name) == noun);
    if noun == db1_hash("BRAN") || listed(rules::HNGT) || listed(rules::NO_SPAMAP_IN_TEMPLATE) {
        return "unsaid";
    }
    match imap_of(noun) {
        1 | 5 if listed(rules::CATALOGUE_BOXED) => "yes, with SPRE",
        1 | 5 => "yes",
        7 => "as its owner",
        _ => "no",
    }
}

#[test]
fn the_imap_and_hngt_lists_are_the_dictionarys() {
    if common::missing(&[ATTLIB]) {
        return;
    }
    let dictionary = NounFields::parse_file(ATTLIB).expect("read the noun dictionary");
    let field = |noun: u32, field: &str| dictionary.field(noun, db1_hash(field)).map(|value| value.word());
    for (imap, listed) in [(1, rules::IMAP_1), (2, rules::IMAP_2), (3, rules::IMAP_3), (5, rules::IMAP_5), (7, rules::IMAP_7)] {
        let mut read: Vec<String> =
            dictionary.nouns().into_iter().filter(|&noun| field(noun, "IMAP") == Some(imap)).map(name).collect();
        read.sort();
        assert_eq!(sorted(listed), read, "IMAP {imap}");
    }
    for noun in dictionary.nouns() {
        assert_eq!(u32::from(imap_of(noun)), field(noun, "IMAP").unwrap_or(0), "{}", name(noun));
    }
    let mut hngt: Vec<String> =
        dictionary.nouns().into_iter().filter(|&noun| field(noun, "HNGT") == Some(1)).map(name).collect();
    hngt.sort();
    assert_eq!(sorted(rules::HNGT), hngt);
    eprintln!("{} nouns in the dictionary, {} of them with an IMAP other than 0", dictionary.nouns().len(), {
        dictionary.nouns().into_iter().filter(|&noun| imap_of(noun) != 0).count()
    });
}

#[test]
fn the_recorded_nouns_without_spamap_are_the_design_templates() {
    if common::missing(&[DESVIR]) {
        return;
    }
    let mut templates = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("open the design template");
    let (spamap, wbox) = (db1_hash("SPAMAP"), db1_hash("WBOX"));
    let mut without_spamap = Vec::new();
    let mut with_wbox = Vec::new();
    for noun in template_nouns(DESVIR) {
        // A table's child (`SPAMAP`, `SPRE`, …) has no element template.
        let Ok(Some(template)) = templates.template_for(noun) else { continue };
        let has = |attribute: u32| template.attrs.iter().any(|attr| attr.hash == attribute);
        if has(wbox) {
            with_wbox.push(name(noun));
        }
        if matches!(imap_of(noun), 1 | 5 | 7) && !has(spamap) {
            without_spamap.push(name(noun));
        }
    }
    without_spamap.sort();
    assert_eq!(without_spamap, sorted(rules::NO_SPAMAP_IN_TEMPLATE));
    assert!(with_wbox.is_empty(), "MPBOX2 takes WBOX where an element has it: {with_wbox:?}");
}

/// Requirement 5.10: every noun of the design and catalogue templates, what
/// the rule makes of it, and per Corpus database the elements E3D wrote of it
/// and how many of those have entries. Printed, and checked against the
/// rule's three states: a noun the rule says yes to has entries on every
/// element (the box type 1 nouns on every element with a catalogue), one it
/// says no to on none.
#[test]
fn every_template_noun_with_its_corpus_elements() {
    if common::missing(&[ATTLIB, DESVIR, CATVIR]) || common::missing(CORPUS) {
        return;
    }
    let dictionary = NounFields::parse_file(ATTLIB).expect("read the noun dictionary");
    // (database, noun) -> (elements E3D wrote, of them with entries)
    let mut counts: BTreeMap<(usize, u32), (u64, u64)> = BTreeMap::new();
    let mut seen: HashSet<u32> = HashSet::new();
    for (index, path) in CORPUS.iter().enumerate() {
        let mut view = DbView::open_at(path, SessionSelector::Latest).expect("open the database");
        let sesno = view.selected_session();
        let authors = Authors::from_sessions(&view.walk_sessions().expect("walk the sessions"));
        let with: HashSet<RefNo> = match read_spamap(&mut view).expect("read the SPAMAP") {
            SpamapRead::Table { table, .. } => table.entries().iter().map(|entry| entry.refno()).collect(),
            SpamapRead::NoTable { .. } => HashSet::new(),
        };
        let mut engine = ReadOnlyEngine::open_at(path, sesno).expect("open the engine");
        for element in engine.scan_elements(ScanTier::Header).expect("walk the refno index") {
            let element = element.expect("read an indexed element's header");
            let e3d = authors.user_of(element.page_no).is_some_and(|user| !user.eq_ignore_ascii_case("e3dio"));
            if !e3d {
                continue;
            }
            let tally = counts.entry((index, element.noun_hash)).or_default();
            tally.0 += 1;
            tally.1 += u64::from(with.contains(&element.refno));
            seen.insert(element.noun_hash);
        }
    }

    let databases: Vec<String> =
        CORPUS.iter().map(|path| path.rsplit('\\').next().unwrap_or(path).to_owned()).collect();
    let mut by_class: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (template, path) in [("DESI", DESVIR), ("CATA", CATVIR)] {
        let nouns: Vec<u32> = template_nouns(path).into_iter().filter(|&noun| dictionary.has_noun(noun)).collect();
        println!("== {template} template: {} nouns of the dictionary", nouns.len());
        let mut nouns = nouns;
        nouns.sort_by_key(|&noun| name(noun));
        for noun in nouns {
            let class = class(noun);
            let per_database: Vec<String> = (0..CORPUS.len())
                .filter_map(|index| counts.get(&(index, noun)).map(|(e3d, with)| format!("{} {with}/{e3d}", databases[index])))
                .collect();
            let corpus = if per_database.is_empty() { "no corpus element".to_owned() } else { per_database.join(", ") };
            println!("{:<8} IMAP {}  {:<15} {corpus}", name(noun), imap_of(noun), class);
            by_class.entry(class).or_default().push(name(noun));
            for (index, database) in databases.iter().enumerate() {
                let Some(&(e3d, with)) = counts.get(&(index, noun)) else { continue };
                match class {
                    "yes" => assert_eq!(with, e3d, "{} in {database}", name(noun)),
                    "no" => assert_eq!(with, 0, "{} in {database}", name(noun)),
                    _ => {}
                }
            }
        }
    }
    let outside: Vec<String> = seen.iter().filter(|&&noun| !dictionary.has_noun(noun)).map(|&noun| name(noun)).collect();
    println!("== Corpus nouns the dictionary lacks (ATNINT reads IMAP 0): {outside:?}");
    for (class, nouns) in &by_class {
        println!("== {class}: {} nouns", nouns.len());
    }
}
