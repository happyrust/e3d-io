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

use e3d_attlib::{db1_dehash, db1_hash};
use e3d_io::index::cursor::IndexCursor;
use e3d_io::index::reference::{ReferenceEntry, ReferenceTable, ReferenceTableError};
use e3d_io::refno::RefNo;
use e3d_io::session::{DbView, TableRoot};

mod common;

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
/// The largest catalogue database installed: 1.2 GB, 3.3 M index keys — and,
/// as [`a_database_can_state_no_reference_table_at_all`] pins, not one
/// reference table.
const AMS7351: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7351_0001";
const ACP_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000";
const ZDJ_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\ZDJ\ZDJ000";
/// The largest reference table in the installed corpus: `CATR` of the AVEVA
/// catalogue's main database, 744 925 edges (783-file scan, 2026-09-06).
const ACP7320: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7320_0001";

/// Directory tables that decode as reference tables — 4-dword key, no leaf
/// value — and are not: their key's second half is not a refno of the file.
/// `TYPEX` indexes elements by type; the Draft tables index symbols and
/// annotations. Anything else of that shape whose referrers are not all local
/// is a table this reader has not met, and the corpus gate says so by name.
const KNOWN_LOOKALIKES: &[&str] = &[
    "TYPEX", "OCONET", "FSTYNO", "GLYNO", "HPATNO", "LSTYNO", "MARPTT", "SYLBTA",
];

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
        (
            pair[0].target.word0,
            pair[0].target.word1,
            pair[0].referrer.word0,
            pair[0].referrer.word1,
        ) <= (
            pair[1].target.word0,
            pair[1].target.word1,
            pair[1].referrer.word0,
            pair[1].referrer.word1,
        )
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
    assert_eq!(
        foreign,
        edges.len(),
        "targets are not all in another database"
    );
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
    assert_eq!(
        dangling, 0,
        "{dangling} of {edges} edges name a missing referrer"
    );
}

/// What one directory entry turns out to be once its pages are read.
#[derive(Debug)]
enum TableKind {
    /// 4-dword key, no value, and every referrer is an element of this file.
    Reference { edges: usize },
    /// Same page shape, but the second half of the key is not a local refno —
    /// so it is not "who references whom" and must not be read as edges.
    Lookalike {
        edges: usize,
        foreign_referrers: usize,
    },
    /// A different table kind altogether (name table, `SPAMAP`, UDA tables…).
    Other(String),
}

/// The criterion that separates a reference table from a lookalike: **every
/// referrer is a refno of the database holding the table**. `db_put_reference`
/// files the edge in the referring database under the target, so the referrer
/// half of the key can never be foreign; a table whose second halves are not
/// local refnos is something else wearing the same page header.
fn classify(view: &mut DbView, table: TableRoot) -> TableKind {
    let own_dbno = view.descriptor().db_mark;
    let mut reference = match ReferenceTable::open(view, table.name_hash) {
        Ok(reference) => reference,
        Err(error) => return TableKind::Other(error.to_string()),
    };
    let mut edges = 0usize;
    let mut foreign_referrers = 0usize;
    for entry in reference.iter() {
        match entry {
            Ok(entry) => {
                edges += 1;
                if entry.referrer.dbno() != own_dbno {
                    foreign_referrers += 1;
                }
            }
            Err(ReferenceTableError::NotAReferenceTable {
                key_dwords,
                value_dwords,
                ..
            }) => return TableKind::Other(format!("key={key_dwords} value={value_dwords}")),
            Err(error) => panic!("table {}: {error}", table.name_hash),
        }
    }
    if foreign_referrers == 0 {
        TableKind::Reference { edges }
    } else {
        TableKind::Lookalike {
            edges,
            foreign_referrers,
        }
    }
}

fn table_name(table: &TableRoot) -> String {
    db1_dehash(table.name_hash).unwrap_or_else(|| format!("#{}", table.name_hash))
}

/// A reference table is told apart from `TYPEX` by its referrers all being
/// local, not by the dictionary and not by the page header. On `ams8000_0001`
/// every table named by a reference attribute passes, `TYPEX` fails by the
/// tens of thousands, and nothing of the reference shape is left unclassified.
#[test]
fn a_reference_table_is_told_apart_by_its_referrers_all_being_local() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let mut view = DbView::open(AMS8000).unwrap();
    let tables: Vec<TableRoot> = view.tables().to_vec();
    let mut references = BTreeSet::new();
    let mut lookalikes = BTreeSet::new();
    let mut others = BTreeMap::new();
    for table in tables {
        let name = table_name(&table);
        match classify(&mut view, table) {
            TableKind::Reference { edges } => {
                assert!(edges > 0, "{name} is an empty reference table");
                references.insert(name);
            }
            TableKind::Lookalike {
                edges,
                foreign_referrers,
            } => {
                assert!(
                    KNOWN_LOOKALIKES.contains(&name.as_str()),
                    "{name} has the reference shape but {foreign_referrers} of {edges} referrers \
                     are foreign — a table this reader has not met; classify it before using it"
                );
                lookalikes.insert(name);
            }
            TableKind::Other(detail) => {
                others.insert(name, detail);
            }
        }
    }
    println!(
        "ams8000_0001 reference tables: {references:?}; lookalikes: {lookalikes:?}; other kinds: {others:?}"
    );
    assert!(
        lookalikes.contains("TYPEX"),
        "TYPEX must be present and must fail the local-referrer criterion"
    );
    for name in REFERENCE_ATTRIBUTES {
        if view.table_root(db1_hash(name)).is_some() {
            assert!(
                references.contains(*name),
                "{name} is a reference attribute but its table did not classify as one"
            );
        }
    }
    assert!(
        references.contains("SPRE") && references.contains("CATR"),
        "a design database files SPRE and CATR: {references:?}"
    );
}

/// The same criterion over every installed database: whatever decodes as a
/// reference table either is one (all referrers local) or is one of the named
/// lookalikes. A new name in the second bucket is a table nobody has read yet.
#[test]
#[ignore = "walks every named table of ~550 databases; run with --ignored"]
fn every_reference_shaped_table_in_the_corpus_is_a_reference_table_or_a_known_lookalike() {
    if common::missing(&[AMS_DIR, ACP_DIR, ZDJ_DIR]) {
        return;
    }
    let mut files = 0usize;
    let mut references: BTreeMap<String, usize> = BTreeMap::new();
    let mut lookalikes: BTreeMap<String, usize> = BTreeMap::new();
    let mut unknown: BTreeSet<String> = BTreeSet::new();
    for dir in [AMS_DIR, ACP_DIR, ZDJ_DIR] {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
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
            for table in view.tables().to_vec() {
                let name = table_name(&table);
                match classify(&mut view, table) {
                    TableKind::Reference { .. } => *references.entry(name).or_default() += 1,
                    TableKind::Lookalike { .. } => {
                        if !KNOWN_LOOKALIKES.contains(&name.as_str()) {
                            unknown.insert(name.clone());
                        }
                        *lookalikes.entry(name).or_default() += 1;
                    }
                    TableKind::Other(_) => {}
                }
            }
        }
    }
    let table_count: usize = references.values().sum();
    println!(
        "{files} files, {table_count} reference tables under {} names, lookalikes {lookalikes:?}",
        references.len()
    );
    assert!(
        table_count > 100,
        "only {table_count} reference tables found"
    );
    assert!(
        unknown.is_empty(),
        "reference-shaped tables whose referrers are not local and that are not known lookalikes: \
         {unknown:?}"
    );
}

/// The cost gate: answering "who references this component" from the file is
/// a descent, not a scan. On the largest reference table installed — `CATR`
/// of `acp7320_0001`, 744 925 edges — the point query reads the height of the
/// tree plus the leaves the run spans, while the walk reads every page. Should
/// the point query ever read more than a hundredth of what the walk reads, the
/// descent has degenerated into a scan and the table is no longer worth
/// reading over a scan of the elements themselves.
#[test]
fn a_point_query_costs_the_height_of_the_tree_not_the_table() {
    if common::missing(&[ACP7320]) {
        return;
    }
    let catr = db1_hash("CATR");

    // The walk, on a cold view: every page of the table once.
    let mut view = DbView::open(ACP7320).unwrap();
    let opened = view.stats().pages_read;
    let edges = walk_table(&mut view, "CATR").expect("acp7320_0001 has a CATR table");
    let walk_pages = view.stats().pages_read - opened;
    assert!(
        edges.len() > 500_000,
        "acp7320_0001 CATR holds {} edges; the gate wants the large table",
        edges.len()
    );
    // A target from the middle of the table, so the descent has to route
    // through internal pages rather than fall off either edge.
    let target = edges[edges.len() / 2].target;
    let expected: Vec<RefNo> = edges
        .iter()
        .filter(|edge| edge.target == target)
        .map(|edge| edge.referrer)
        .collect();

    // The point query, on another cold view: the descent and the run.
    let mut view = DbView::open(ACP7320).unwrap();
    let opened = view.stats().pages_read;
    let mut table = ReferenceTable::open(&mut view, catr).unwrap();
    let found = table.referrers_of(target).unwrap();
    let point_pages = table.stats().pages_read - opened;

    assert_eq!(found, expected, "the point query disagrees with the walk");
    println!(
        "acp7320_0001 CATR: {} edges, walk {walk_pages} pages, point query {point_pages} pages for \
         {} referrers",
        edges.len(),
        found.len()
    );
    assert!(
        point_pages <= 16,
        "a point query read {point_pages} pages — more than any plausible tree height plus run"
    );
    assert!(
        point_pages * 100 <= walk_pages,
        "point query {point_pages} pages vs walk {walk_pages} pages: the descent is not paying off"
    );
}

/// An absent table is not an empty answer. `ams7351_0001` — the largest
/// catalogue installed, 3.3 M index keys — names only `NAME` and `TYPEX`; it
/// has no `CATR`, no `GMRE`, no reference table of any kind. `referrers_of`
/// on it returns nothing for every target, and a reverse cascade that took
/// that for "no design element references this component" would leave every
/// dependent stale. The caller has to ask `table_root` first and treat
/// "this database keeps no such table" as its own outcome.
#[test]
fn a_database_can_state_no_reference_table_at_all() {
    if common::missing(&[AMS7351]) {
        return;
    }
    let mut view = DbView::open(AMS7351).unwrap();
    let names: BTreeSet<String> = view.tables().iter().map(table_name).collect();
    println!("ams7351_0001 names {names:?}");
    for name in REFERENCE_ATTRIBUTES {
        assert!(
            view.table_root(db1_hash(name)).is_none(),
            "ams7351_0001 now states a {name} table — this pin, and the fallback that relies on \
             it, need revisiting"
        );
    }
    let opened = ReferenceTable::open(&mut view, db1_hash("CATR"));
    assert!(
        matches!(opened, Err(ReferenceTableError::NoSuchTable { .. })),
        "opening an absent table must say so, not answer empty: {:?}",
        opened.map(|table| table.root())
    );
}
