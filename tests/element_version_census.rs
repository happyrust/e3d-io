//! A Dabacon file keeps every version of an element it has ever written, and
//! the index addresses all of them. `find_element` currently answers with the
//! first copy a root confirms, which is not necessarily the newest.
//!
//! `ams1112_0001` holds twelve readable copies of SITE 9304/2. The one the
//! lookup answers with carries three explicit attributes; a copy on page 2101
//! carries five, and one of the two extra is hash 435136076 — the key of the
//! UDA `:3D_SJRY`, which E3D reports as `wangyid` and which reads as unset
//! from the copy we return.
//!
//! This measures how far that reaches across the sampled elements. The count
//! it prints is the size of the problem; closing it needs the session chain
//! (page 0 `+0x28` names the latest session page) to say which copy is
//! current, which is not decoded yet.
//!
//! Run with:
//! `cargo test --test element_version_census -- --ignored --nocapture`

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const SAMPLE: &str = "evidence/20260826-tty-sampling/sample.tsv";
const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

mod common;

const FIXTURES: &[&str] = &[AMS];

fn parse_refno(text: &str) -> Option<RefNo> {
    let (w0, w1) = text.split_once('/')?;
    Some(RefNo::new(w0.trim().parse().ok()?, w1.trim().parse().ok()?))
}

/// The attribute hashes and member count of one stored copy.
struct Copy {
    hashes: Vec<u32>,
    members: usize,
}

#[test]
fn how_often_is_the_answered_copy_not_the_richest() {
    if common::missing(FIXTURES) {
        return;
    }
    let Ok(sample) = std::fs::read_to_string(SAMPLE) else {
        eprintln!("SKIP: {SAMPLE} not present");
        return;
    };

    let mut by_db: BTreeMap<String, Vec<(String, RefNo)>> = BTreeMap::new();
    for line in sample.lines().filter(|line| !line.starts_with('#')) {
        let cols: Vec<&str> = line.split('\t').collect();
        let [db, noun, _layout, _words, refno] = cols[..] else {
            continue;
        };
        let Some(refno) = parse_refno(refno) else {
            continue;
        };
        by_db
            .entry(db.to_string())
            .or_default()
            .push((noun.to_string(), refno));
    }

    let mut checked = 0usize;
    let mut multi_copy = 0usize;
    let mut answered_is_short = Vec::new();
    let mut attributes_missed = 0usize;

    for (db, rows) in &by_db {
        let path = Path::new(AMS).join(db);
        if !path.exists() {
            eprintln!("SKIP {db}: not present");
            continue;
        }
        let mut engine = ReadOnlyEngine::open(&path).expect("open database");
        let entries = engine.indexed_refnos().expect("scan index");

        for (noun, refno) in rows {
            let locations: Vec<(u32, u32)> = entries
                .iter()
                .filter(|(candidate, _, _)| candidate == refno)
                .map(|(_, page, offset)| (*page, *offset))
                .collect();
            if locations.is_empty() {
                continue;
            }
            checked += 1;

            let mut copies = Vec::new();
            for (page, offset) in locations {
                let Ok(view) = engine.read_element_direct(*refno, page, offset) else {
                    continue;
                };
                let Ok(parsed) = ParsedElement::parse(&view.raw_bytes) else {
                    continue;
                };
                if parsed.refno != *refno {
                    continue;
                }
                copies.push(Copy {
                    hashes: parsed
                        .explicit_attributes
                        .iter()
                        .map(|attribute| attribute.hash)
                        .collect(),
                    members: parsed.members.len(),
                });
            }
            if copies.len() > 1 {
                multi_copy += 1;
            }

            let Ok(Some(view)) = engine.find_element(*refno) else {
                continue;
            };
            let answered = ParsedElement::parse(&view.raw_bytes).expect("parse");
            let answered_hashes: HashSet<u32> = answered
                .explicit_attributes
                .iter()
                .map(|attribute| attribute.hash)
                .collect();

            let elsewhere: HashSet<u32> = copies
                .iter()
                .flat_map(|copy| copy.hashes.iter().copied())
                .filter(|hash| !answered_hashes.contains(hash))
                .collect();
            let most_members = copies.iter().map(|copy| copy.members).max().unwrap_or(0);
            if !elsewhere.is_empty() || most_members > answered.members.len() {
                attributes_missed += elsewhere.len();
                answered_is_short.push(format!(
                    "{db} {noun} {refno}: answered with {} attributes / {} members; \
                     other copies hold {} attribute(s) it does not{}",
                    answered_hashes.len(),
                    answered.members.len(),
                    elsewhere.len(),
                    if most_members > answered.members.len() {
                        format!(" and up to {most_members} members")
                    } else {
                        String::new()
                    }
                ));
            }
        }
    }

    println!("elements checked: {checked}");
    println!("elements stored more than once: {multi_copy}");
    println!(
        "elements whose answered copy is not the richest: {}",
        answered_is_short.len()
    );
    println!("explicit attributes only present on another copy: {attributes_missed}");
    for line in answered_is_short.iter().take(20) {
        println!("  {line}");
    }

    assert!(checked > 0, "expected to check at least one element");
}
