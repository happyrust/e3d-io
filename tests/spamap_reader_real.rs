//! SPAMAP_Reader on the installed Corpus (requirements 3.3, 3.5): the baseline
//! table reads to the numbers the 2026-09-22 evidence counted, and on every
//! Corpus database's newest session a lookup by refno is the table filtered.

mod common;

use std::collections::HashSet;

use common::corpus::{CORPUS, SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION};
use e3d_io::index::{read_spamap, SpamapEntry, SpamapRead, SpamapTable};
use e3d_io::session::DbView;
use e3d_io::RefNo;

#[test]
fn the_baseline_table_reads_to_the_counted_numbers() {
    if common::missing(&[SPAMAP_BASELINE]) {
        return;
    }
    let mut view = DbView::open_at(SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION).expect("session 640 is on the chain");
    let SpamapRead::Table { sesno, table } = read_spamap(&mut view).expect("read the baseline table") else {
        panic!("session {SPAMAP_BASELINE_SESSION} lists no SPAMAP");
    };
    assert_eq!(sesno, SPAMAP_BASELINE_SESSION);
    let refnos: HashSet<RefNo> = table.entries().iter().map(SpamapEntry::refno).collect();
    let with_k = |k| table.entries().iter().filter(|entry| entry.k() == k).count();
    assert_eq!(
        (table.pages.len(), table.entries().len(), refnos.len(), with_k(0), with_k(1)),
        (244, 3_746, 3_628, 3_625, 121),
        "(pages, entries, distinct refnos, k = 0, k = 1)"
    );
}

// Feature: write-path-phase-3, Property 1: 按 refno 查询等于全表过滤
#[test]
fn on_every_corpus_database_a_lookup_by_refno_is_the_table_filtered() {
    if common::missing(CORPUS) {
        return;
    }
    for path in CORPUS {
        let mut view = DbView::open(path).expect("open the newest session");
        match read_spamap(&mut view).unwrap_or_else(|error| panic!("{path}: {error}")) {
            SpamapRead::NoTable { sesno } => eprintln!("{path}: session {sesno} lists no SPAMAP"),
            SpamapRead::Table { sesno, table } => {
                let checked = lookups_match_the_filtered_table(&table);
                eprintln!(
                    "{path}: session {sesno}, {} pages, {} entries, {checked} refnos looked up",
                    table.pages.len(),
                    table.entries().len()
                );
            }
        }
    }
}

/// Every refno in the table and one that is not, each looked up and compared
/// with the table's entries for it in key order. The expectation comes from a
/// stable sort by refno, not from the lookup's own index. Returns how many
/// refnos were looked up.
fn lookups_match_the_filtered_table(table: &SpamapTable) -> usize {
    let mut by_refno: Vec<&SpamapEntry> = table.entries().iter().collect();
    by_refno.sort_by_key(|entry| (entry.key[1], entry.key[2]));
    let mut checked = 0;
    for run in by_refno.chunk_by(|a, b| a.refno() == b.refno()) {
        let looked_up: Vec<&SpamapEntry> = table.for_refno(run[0].refno()).collect();
        assert_eq!(looked_up, run, "{}", run[0].refno());
        checked += 1;
    }
    let absent = RefNo::new(u32::MAX, u32::MAX);
    assert!(by_refno.iter().all(|entry| entry.refno() != absent));
    assert_eq!(table.for_refno(absent).count(), 0);
    checked + 1
}
