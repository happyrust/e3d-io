//! The `NAME` table reader (`index::named`) against the installed samples:
//! every entry the walk returns is a name a live element stores, every stored
//! name is in the table, and a point lookup finds each of them in one descent.
//!
//! This is the acceptance reader stage 2 of the write plan needs before it
//! can maintain the table (`docs/plans/2026-09-20-dabacon-write-plan.md` §2
//! 阶段 2 用例 D).

mod common;

use std::cmp::Ordering;
use std::collections::BTreeMap;

use common::write_fixture::{AMS5100, AMS8000};
use e3d_io::engine::ScanTier;
use e3d_io::index::named::compare_keys;
use e3d_io::index::{encode_name, NamedTable};
use e3d_io::session::DbView;
use e3d_io::{ReadOnlyEngine, RefNo};

/// The stored `NAME` as the table keys it: with its leading `/`.
fn as_key(stored: &str) -> String {
    if stored.starts_with('/') {
        stored.to_owned()
    } else {
        format!("/{stored}")
    }
}

fn check(path: &str) {
    let mut engine = ReadOnlyEngine::open(path).expect("open");

    // --- the whole table, in key order ---
    let entries: Vec<(String, RefNo, Vec<u32>)> = {
        let mut table = engine.name_table().expect("a design database keeps a NAME table");
        table
            .iter()
            .map(|entry| {
                let entry = entry.expect("a readable NAME table page");
                (entry.name(), entry.refno, entry.key.words().to_vec())
            })
            .collect()
    };
    assert!(!entries.is_empty(), "{path}: the NAME table is empty");
    for pair in entries.windows(2) {
        assert_eq!(
            compare_keys(&pair[0].2, &pair[1].2),
            Ordering::Less,
            "{path}: {:?} must sort strictly before {:?}",
            pair[0].0,
            pair[1].0
        );
    }
    let by_name: BTreeMap<&str, RefNo> = entries.iter().map(|(n, r, _)| (n.as_str(), *r)).collect();
    assert_eq!(by_name.len(), entries.len(), "{path}: a name appears twice in the table");

    // --- every stored NAME, from the elements themselves ---
    let mut stored: BTreeMap<String, RefNo> = BTreeMap::new();
    let mut named_elements = 0usize;
    for element in engine.scan_elements(ScanTier::Named).expect("scan") {
        let element = element.expect("scanned element");
        if let Some(name) = element.name {
            named_elements += 1;
            stored.insert(as_key(&name), element.refno);
        }
    }
    assert_eq!(
        stored.len(),
        named_elements,
        "{path}: two live elements store the same NAME"
    );

    let only_in_table: Vec<_> = by_name
        .iter()
        .filter(|(name, _)| !stored.contains_key(**name))
        .take(5)
        .collect();
    let only_on_elements: Vec<_> = stored
        .iter()
        .filter(|(name, _)| !by_name.contains_key(name.as_str()))
        .take(5)
        .collect();
    assert!(
        only_in_table.is_empty() && only_on_elements.is_empty(),
        "{path}: table and elements disagree — only in table: {only_in_table:?}; only on elements: {only_on_elements:?}"
    );
    for (name, refno) in &stored {
        assert_eq!(
            by_name.get(name.as_str()),
            Some(refno),
            "{path}: {name} names {refno} on the element but the table says otherwise"
        );
    }
    eprintln!("{path}: {} names, table and elements agree", entries.len());

    // --- point lookups: one descent each ---
    let mut table = engine.name_table().expect("NAME table");
    let root_level = {
        let mut view = DbView::open(path).expect("view");
        let root = view.table_root(e3d_io::index::name_table_hash()).expect("root");
        let page = view.pages_mut().get(root).expect("root page").to_vec();
        u32::from_be_bytes(page[8..12].try_into().unwrap())
    };
    for (name, refno, _) in entries.iter().step_by((entries.len() / 400).max(1)) {
        let before = table.stats().pages_read;
        let found = table.lookup_name(name).expect("lookup").unwrap_or_else(|| {
            panic!("{path}: {name} is in the table but lookup found nothing")
        });
        assert_eq!(found, *refno, "{path}: {name}");
        let read = table.stats().pages_read - before;
        assert!(
            read <= u64::from(root_level) + 1,
            "{path}: looking up {name} read {read} pages, the tree is {} levels",
            root_level + 1
        );
    }
    assert_eq!(table.lookup_name("/no-such-name-e3dio").unwrap(), None);
    if let Some((name, _, _)) = entries.iter().find(|(n, _, _)| n.chars().any(|c| c.is_ascii_uppercase())) {
        assert_eq!(
            table.lookup_name(&name.to_ascii_lowercase()).unwrap(),
            None,
            "{path}: names match as stored bytes; {name} in lower case is another key"
        );
    }

    // --- the engine's one-call form ---
    let (name, refno, _) = &entries[entries.len() / 2];
    assert_eq!(engine.find_by_name(name).unwrap(), Some(*refno));
    assert_eq!(engine.find_by_name("/no-such-name-e3dio").unwrap(), None);
}

#[test]
fn name_table_agrees_with_the_elements_on_ams8000() {
    if common::missing(&[AMS8000]) {
        return;
    }
    check(AMS8000);
}

#[test]
fn name_table_agrees_with_the_elements_on_ams5100() {
    if common::missing(&[AMS5100]) {
        return;
    }
    check(AMS5100);
}

#[test]
fn a_name_encodes_as_the_table_keys_it() {
    let key = encode_name("/-CSV-E-1-H-7802");
    assert_eq!(key.words().len(), 4);
    assert_eq!(key.text(), "/-CSV-E-1-H-7802");
}

/// Every installed E3D 3.1 database with a NAME table: the walk decodes every
/// page and the keys are in order. Whole-corpus, so ignored by default.
#[test]
#[ignore = "walks the NAME table of every installed 3.1 database; run with --ignored"]
fn every_installed_name_table_decodes() {
    let root = std::path::Path::new(r"D:\AVEVA\Projects\E3D3.1");
    if !root.exists() {
        eprintln!("skipped: {} not installed", root.display());
        return;
    }
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(read) = std::fs::read_dir(dir) else { return };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    let (mut tables, mut names) = (0usize, 0usize);
    // Adjacent pairs whose first differing word differs in its top bit: the
    // pairs a signed word comparison would order the other way round.
    let mut sign_sensitive_pairs = 0usize;
    let mut failures: Vec<String> = Vec::new();
    'files: for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = name.as_bytes();
        let looks_like_db = bytes.len() >= 9
            && bytes[bytes.len() - 5] == b'_'
            && bytes[..3].iter().all(u8::is_ascii_alphabetic)
            && bytes[3..bytes.len() - 5].iter().all(u8::is_ascii_digit)
            && bytes[bytes.len() - 4..].iter().all(u8::is_ascii_digit);
        if !looks_like_db {
            continue;
        }
        let Ok(mut engine) = ReadOnlyEngine::open(&path) else { continue };
        let Some(mut table) = engine.name_table() else { continue };
        tables += 1;
        let mut previous: Option<Vec<u32>> = None;
        for entry in table.iter() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    failures.push(format!("{}: {error}", path.display()));
                    continue 'files;
                }
            };
            if let Some(prev) = &previous {
                if compare_keys(prev, entry.key.words()) != Ordering::Less {
                    failures.push(format!(
                        "{}: out of order at {} (after {:?})",
                        path.display(),
                        entry.name(),
                        prev
                    ));
                    continue 'files;
                }
                if prev
                    .iter()
                    .zip(entry.key.words())
                    .find(|(a, b)| a != b)
                    .is_some_and(|(a, b)| (a ^ b) & 0x8000_0000 != 0)
                {
                    sign_sensitive_pairs += 1;
                }
            }
            previous = Some(entry.key.words().to_vec());
            names += 1;
        }
    }
    eprintln!(
        "{tables} NAME tables, {names} names, {sign_sensitive_pairs} adjacent pairs a signed comparison would swap; {} failures",
        failures.len()
    );
    for failure in &failures {
        eprintln!("  {failure}");
    }
    assert!(tables > 0);
    assert!(failures.is_empty(), "{} NAME tables did not read cleanly", failures.len());
}

#[allow(dead_code)]
fn _table_type_is_exported(view: &mut DbView) -> Option<NamedTable<'_>> {
    NamedTable::open(view, e3d_io::index::name_table_hash()).ok()
}
