//! The page-1 bucket table on the installed databases
//! (`docs/evidence/2026-09-22-core-dll-refno-allocation.md`, S6b read side):
//! the reader's counters match what is on disk, and each counter sits above the
//! highest live `word1` in its extent — the invariant that makes a deleted
//! number never come back.

mod common;

use std::collections::BTreeMap;

use common::write_fixture::{AMS251142, AMS5100, AMS8000};
use e3d_io::engine::ScanTier;
use e3d_io::meta::bucket::BucketTable;
use e3d_io::ReadOnlyEngine;

/// A `TES000` design database, `FORMAT 0`, whose secondary extent 17419 has a
/// counter (331) above its live max (315).
const TES023: &str = r"D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023";

/// Physical page 1 of `path` (2048-byte pages on these samples).
fn page_one(path: &str) -> Vec<u8> {
    let bytes = std::fs::read(path).expect("read the database");
    bytes[2048..4096].to_vec()
}

/// The highest `word1` in each extent (`word0`) of the newest session.
fn max_word1_by_extent(path: &str) -> BTreeMap<u32, u32> {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let mut max = BTreeMap::new();
    for element in engine.scan_elements(ScanTier::Header).expect("scan") {
        let refno = element.expect("header").refno;
        let entry = max.entry(refno.word0).or_insert(0);
        *entry = (*entry).max(refno.word1);
    }
    max
}

/// On each design database, every bucket entry names a secondary extent and its
/// counter is above the live maximum there; the dictionary has no entry.
#[test]
fn the_bucket_table_counters_sit_above_the_live_maximum() {
    if common::missing(&[AMS8000, AMS251142, AMS5100, TES023]) {
        return;
    }

    for path in [AMS8000, AMS251142, TES023] {
        let table = BucketTable::parse(&page_one(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(!table.entries.is_empty(), "{path}: a grown design database has bucket entries");
        let max = max_word1_by_extent(path);
        for entry in &table.entries {
            let live_max = max.get(&entry.extent_word0).copied();
            assert!(
                live_max.is_some(),
                "{path}: bucket names extent {} but no element lives there",
                entry.extent_word0
            );
            assert!(
                entry.next_word1 > live_max.unwrap(),
                "{path}: extent {} counter {} must be above the live max {}",
                entry.extent_word0,
                entry.next_word1,
                live_max.unwrap()
            );
            assert_eq!(entry.extra, 0, "{path}: the third word is zero on every installed database");
        }
        eprintln!("{path}: {} bucket entries, all above the live max: {:?}", table.entries.len(), table.entries);
    }

    // A dictionary whose elements are all in the primary extent never grew, so it
    // has no bucket entry at all.
    let dict = BucketTable::parse(&page_one(AMS5100)).expect("ams5100 page 1");
    assert!(dict.entries.is_empty(), "the dictionary has no secondary extent, so no bucket entry");
}

/// The exact counters observed on `ams8000_0001` when the evidence was taken —
/// pinned so a change to the reader shows here (the file is live, so these can
/// move; the assertion is on the shape and the relation, not the numbers).
#[test]
fn ams8000_names_its_two_secondary_extents() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let table = BucketTable::parse(&page_one(AMS8000)).expect("ams8000 page 1");
    let extents: Vec<u32> = table.entries.iter().map(|e| e.extent_word0).collect();
    assert_eq!(extents, vec![24_384, 32_576], "the two secondary extents, in order");
    assert!(table.next_word1(16_192).is_none(), "the primary extent 16192 has no counter");
}
