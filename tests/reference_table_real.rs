//! The reverse-reference tables, read from the installed E3D 3.1 databases.
//!
//! These hold the two claims the reader is worth having only if both are true:
//!
//! * a range scan and a full walk return the same referrers, so
//!   `referrers_of` may be trusted to be complete, and
//! * every referrer a table names is a live element of the file the table
//!   lives in, so an edge never points at nothing.
//!
//! The second one is what tells a reference table apart from `TYPEX`, which
//! declares the same key width and fails it by millions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use e3d_attlib::db1_hash;
use e3d_io::index::cursor::IndexCursor;
use e3d_io::index::reference::{ReferenceEntry, ReferenceTable};
use e3d_io::refno::RefNo;
use e3d_io::session::DbView;

mod common;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";

/// Reference attributes seen in this corpus's directories. Chosen by the
/// dictionary — each is an `ELEMENT`-typed attribute — rather than by page
/// shape, which `TYPEX` also matches.
const REFERENCE_ATTRIBUTES: &[&str] = &[
    "SPRE", "CATR", "LSTU", "HSTU", "ISPE", "PTRE", "DTRE", "GMRE", "PRTREF", "GOBREF", "HDBREF",
    "POSREF", "HTPREF", "SCREF", "NGMR", "PSTR", "GSTR",
];

fn live_keys(view: &mut DbView) -> BTreeSet<(u32, u32)> {
    let mut cursor = IndexCursor::new(view);
    cursor
        .walk()
        .map(|found| {
            let record = found.expect("the refno index walks");
            (record.refno.word0, record.refno.word1)
        })
        .collect()
}

fn walk_table(view: &mut DbView, name: &str) -> Option<Vec<ReferenceEntry>> {
    let name_hash = db1_hash(name);
    view.table_root(name_hash)?;
    let mut table = ReferenceTable::open(view, name_hash).expect("the session names this table");
    Some(
        table
            .iter()
            .map(|entry| entry.expect("the table walks"))
            .collect(),
    )
}

/// The directory is the only place the other trees are named, and every root
/// it states must be a page of the table it was filed under. A root that is
/// not would let a caller walk `SPRE` and read `CATR`'s edges.
#[test]
fn every_table_the_session_names_roots_at_a_page_of_that_table() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let mut view = DbView::open(AMS8000).unwrap();
    let tables: Vec<_> = view.tables().to_vec();
    assert!(
        tables.len() >= 4,
        "ams8000_0001 states {} tables",
        tables.len()
    );

    for table in tables {
        let page = view.pages_mut().get(table.root).expect("root is readable");
        let page_type = u32::from_be_bytes(page[0..4].try_into().unwrap());
        let noun = u32::from_be_bytes(page[4..8].try_into().unwrap());
        assert_eq!(page_type, 5, "{:?} is not a tree page", table);
        assert_eq!(
            noun, table.name_hash,
            "{:?} roots at a page of table {noun}",
            table
        );
    }
}

/// `SPRE` in `ams8000_0001`: every design element that names a specification
/// component, filed under the component.
#[test]
fn the_spre_table_names_this_databases_own_elements() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let mut view = DbView::open(AMS8000).unwrap();
    let index = live_keys(&mut view);
    let edges = walk_table(&mut view, "SPRE").expect("ams8000_0001 has an SPRE table");

    assert!(!edges.is_empty());
    let ordered = edges.windows(2).all(|pair| {
        (pair[0].target.word0, pair[0].target.word1, pair[0].referrer.word0, pair[0].referrer.word1)
            <= (pair[1].target.word0, pair[1].target.word1, pair[1].referrer.word0, pair[1].referrer.word1)
    });
    assert!(ordered, "the walk did not come out in key order");

    let own_dbno = RefNo::new(edges[0].referrer.word0, 0).dbno();
    for edge in &edges {
        assert!(
            index.contains(&(edge.referrer.word0, edge.referrer.word1)),
            "{edge} names a referrer this database does not hold"
        );
        assert_eq!(
            edge.referrer.dbno(),
            own_dbno,
            "{edge} names a referrer from another database"
        );
    }

    // The targets are catalogue components, so they live elsewhere. If they
    // ever came out local, the table would be being read the wrong way round.
    let foreign = edges
        .iter()
        .filter(|edge| edge.target.dbno() != own_dbno)
        .count();
    assert_eq!(foreign, edges.len(), "targets are not all in another database");
}

/// A range scan must return the same run the full walk holds. `referrers_of`
/// stops at the first key past the target, so a descent landing one leaf late
/// would return an answer that is short rather than one that is wrong — the
/// failure mode a reverse cascade cannot see.
#[test]
fn a_range_scan_returns_exactly_what_the_walk_holds() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let mut view = DbView::open(AMS8000).unwrap();
    for name in ["SPRE", "CATR", "LSTU", "HSTU", "ISPE"] {
        let Some(edges) = walk_table(&mut view, name) else {
            continue;
        };
        let mut expected: BTreeMap<(u32, u32), Vec<(u32, u32)>> = BTreeMap::new();
        for edge in &edges {
            expected
                .entry((edge.target.word0, edge.target.word1))
                .or_default()
                .push((edge.referrer.word0, edge.referrer.word1));
        }

        let mut table = ReferenceTable::open(&mut view, db1_hash(name)).unwrap();
        for (target, referrers) in &expected {
            let found = table
                .referrers_of(RefNo::new(target.0, target.1))
                .unwrap_or_else(|error| panic!("{name} {target:?}: {error}"));
            let found: Vec<(u32, u32)> = found.iter().map(|r| (r.word0, r.word1)).collect();
            assert_eq!(&found, referrers, "{name}: run for {target:?} disagrees");
        }

        // A target nothing references is an empty answer, not a stray run.
        let absent = RefNo::new(expected.keys().next_back().unwrap().0, u32::MAX / 2);
        assert!(table.referrers_of(absent).unwrap().is_empty());
    }
}

/// The gate the reading was settled against: across every installed database,
/// no reference table names a referrer its own file does not hold.
#[test]
#[ignore = "walks every reference table of ~500 databases; run with --ignored"]
fn no_reference_table_names_a_referrer_its_file_does_not_hold() {
    if common::missing(&[AMS_DIR]) {
        return;
    }
    let mut files = 0usize;
    let mut tables = 0usize;
    let mut edges = 0usize;
    let mut dangling = 0usize;

    let mut paths: Vec<PathBuf> = std::fs::read_dir(AMS_DIR)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect();
    paths.sort();

    for path in paths {
        let Ok(mut view) = DbView::open(&path) else {
            continue;
        };
        files += 1;
        let index = live_keys(&mut view);
        for name in REFERENCE_ATTRIBUTES {
            let Some(found) = walk_table(&mut view, name) else {
                continue;
            };
            tables += 1;
            edges += found.len();
            dangling += found
                .iter()
                .filter(|edge| !index.contains(&(edge.referrer.word0, edge.referrer.word1)))
                .count();
        }
    }

    println!("{files} files, {tables} reference tables, {edges} edges");
    assert!(tables > 100, "only {tables} tables found");
    assert_eq!(dangling, 0, "{dangling} of {edges} edges name a missing referrer");
}
