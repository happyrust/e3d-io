//! Stage-0 guards for RefNo-native routing.
//!
//! These tests deliberately stop short of changing routing. They pin the
//! recovered bit layout, the fail-closed "db is outside this set" behavior,
//! and the read-cost counters needed to prove later phases no longer scan an
//! entire MDB.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use e3d_io::db_element::{DbElementError, DbSet};
use e3d_io::{read_stats, ReadOnlyEngine, RefNo, ScanTier};

const WORDS_PER_PAGE: u32 = 64;
const PAGE_BYTES: usize = WORDS_PER_PAGE as usize * 4;
const INDEX_NOUN: u32 = 13_387_743;
const INDEX_HEADER_DWORDS: usize = 7;

fn word0(dbno: u32, extent: u32) -> u32 {
    (dbno & 0x1FFF) | ((extent & 0x1FFF) << 13) | ((dbno & 0x3E000) << 13)
}

fn put(page: &mut [u8], offset: usize, value: u32) {
    page[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn write_counter_fixture(directory: &Path) -> (PathBuf, RefNo) {
    let dbno = 1_000;
    let element = RefNo::new(word0(dbno, 1), 42);
    let mut pages = vec![vec![0_u8; PAGE_BYTES]; 4];

    // Page 0: just enough descriptor data for the ordinary engine open path.
    put(&mut pages[0], 0x08, dbno);
    put(&mut pages[0], 0x28, 1);
    put(&mut pages[0], 0x2C, 1);
    put(&mut pages[0], 0x30, 1);
    put(&mut pages[0], 0x34, WORDS_PER_PAGE);
    put(&mut pages[0], 0x38, element.word0);

    // Page 1: selected session, whose live RefNo tree starts on page 2.
    put(&mut pages[1], 0x00, 3);
    put(&mut pages[1], 0x04, u32::MAX);
    put(&mut pages[1], 0x0C, 7);
    put(&mut pages[1], 0x1C, 2);
    put(&mut pages[1], 0x20, 1);

    // Page 2: a one-entry leaf. The packed record address points at byte 40
    // of element-data page 3, using the existing half-word address encoding.
    put(&mut pages[2], 0x00, 5);
    for (index, value) in [INDEX_NOUN, 0, 2, 2, 0, 53].into_iter().enumerate() {
        put(&mut pages[2], (index + 1) * 4, value);
    }
    let entry = [element.word0, element.word1, 3, (20_u32 << 12) | 1];
    for (index, value) in entry.into_iter().enumerate() {
        put(&mut pages[2], (INDEX_HEADER_DWORDS + index) * 4, value);
    }

    // Page 3: one eleven-word element record.
    put(&mut pages[3], 0x00, 7);
    for (index, value) in [
        11,
        element.word0,
        element.word1,
        0x000A_AA01,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut pages[3], 40 + index * 4, value);
    }

    let path = directory.join("tst1000_0001");
    let mut file = File::create(&path).expect("create synthetic E3D database");
    for page in pages {
        file.write_all(&page).expect("write synthetic page");
    }
    file.flush().expect("flush synthetic database");
    (path, element)
}

#[test]
fn refno_routing_cases_and_read_cost_counters_are_pinned() {
    // E3D 2.1 JEU field capture: the same dbno appears in primary and
    // secondary extent RefNos. Extent bits must never leak into routing.
    let jeu_primary = RefNo::new(word0(8_188, 1), 115);
    let jeu_secondary = RefNo::new(24_572, 115);
    assert_eq!(jeu_primary.dbno(), 8_188);
    assert_eq!(jeu_primary.extent(), 1);
    assert_eq!(jeu_secondary.dbno(), 8_188);
    assert_eq!(jeu_secondary.extent(), 2);

    // A dbno wider than the low thirteen bits is reconstructed from both
    // halves around the extent field.
    for extent in [1, 2, 17] {
        let wide = RefNo::new(word0(251_000, extent), 1);
        assert_eq!(wide.dbno(), 251_000);
        assert_eq!(wide.extent(), extent);
    }

    assert!(!RefNo::new(0, 0).is_valid(), "the null RefNo is invalid");
    assert!(
        !RefNo::new(word0(8_188, 1), 0x8000_0000).is_valid(),
        "a negative signed word1 is invalid"
    );

    // Decoding a valid dbno is not permission to read it. MDB scope remains
    // a separate, fail-closed resolver decision.
    let set = DbSet::new(Arc::new(e3d_attlib::AttlibData::default()));
    match set.element_exists(jeu_primary).unwrap_err() {
        DbElementError::DbNotRegistered { dbno } => assert_eq!(dbno, 8_188),
        other => panic!("unexpected outside-MDB error: {other}"),
    }

    // Exercise the public read statistics through real engine operations.
    // This integration-test binary contains only this non-ignored test, so an
    // exact delta is stable even though the counters are process-global.
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    let (path, element) = write_counter_fixture(directory.path());
    read_stats::reset();

    let mut engine = ReadOnlyEngine::open(&path).expect("open synthetic database");
    let opened = read_stats::snapshot();
    assert_eq!(opened.database_opens, 1);
    assert_eq!(opened.scan_calls, 0);
    assert_eq!(opened.scanned_elements, 0);
    assert_eq!(opened.index_point_lookups, 0);

    let mut scan = engine
        .scan_elements(ScanTier::Header)
        .expect("create header scan");
    let scan_created = read_stats::snapshot();
    assert_eq!(scan_created.scan_calls, 1);
    assert_eq!(scan_created.scanned_elements, 0);
    assert_eq!(
        scan.next()
            .expect("one index entry")
            .expect("read indexed record")
            .refno,
        element
    );
    assert!(scan.next().is_none());
    drop(scan);

    assert!(engine
        .search_index(element)
        .expect("point lookup")
        .is_some());
    assert!(engine
        .find_element(RefNo::new(element.word0, 999))
        .expect("missing point lookup")
        .is_none());

    let measured = read_stats::snapshot();
    assert_eq!(measured.database_opens, 1);
    assert_eq!(measured.scan_calls, 1);
    assert_eq!(measured.scanned_elements, 1);
    assert_eq!(measured.index_point_lookups, 2);
    assert!(measured.summary().contains("db opens=1 scans=1"));
}
