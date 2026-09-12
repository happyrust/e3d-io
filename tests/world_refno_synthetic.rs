//! WORLD location on synthetic databases (RefNo-native routing, stage 3C).
//!
//! `DbDescriptor::world_refno()` names `primary_extent_ref / 0`;
//! `ReadOnlyEngine::world()` confirms it with one point lookup. This binary
//! holds the only tests that assert exact `read_stats` deltas, because the
//! counters are process-global and every other test in a binary would add to
//! them; the real-corpus checks live in `world_refno_corpus.rs`.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use e3d_attlib::db1_hash;
use e3d_io::{read_stats, DbDescriptor, E3dReaderError, ReadOnlyEngine, RefNo};

fn word0(dbno: u32, extent: u32) -> u32 {
    (dbno & 0x1FFF) | ((extent & 0x1FFF) << 13) | ((dbno & 0x3E000) << 13)
}

// ---- descriptor -------------------------------------------------------------

fn page0(words: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for word in words {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    bytes.resize(2048, 0);
    bytes
}

/// Page 0 of `ams8000_0001`, word for word (see `meta::tests::ams8000_page0`),
/// with the primary extent reference swapped in.
fn descriptor_with_primary_ref(primary_extent_ref: u32) -> DbDescriptor {
    DbDescriptor::parse(&page0(&[
        0,
        2,
        8000,
        1,
        1,
        0,
        u32::MAX,
        0,
        722_578,
        u32::MAX,
        8897,
        1,
        4,
        512,
        primary_extent_ref,
    ]))
    .expect("ams8000 descriptor words parse")
}

#[test]
fn world_refno_is_element_zero_of_the_primary_extent() {
    let ams8000 = descriptor_with_primary_ref(16_192);
    let world = ams8000.world_refno();
    assert_eq!(world, RefNo::new(16_192, 0));
    assert_eq!(world.dbno(), 8_000);
    assert_eq!(world.extent(), 1);
    assert!(world.is_valid());

    // A dbno wider than thirteen bits keeps its high bits above the extent
    // field; the WORLD address decodes back to the same database.
    let wide = descriptor_with_primary_ref(word0(251_000, 1));
    assert_eq!(wide.world_refno().dbno(), 251_000);
    assert_eq!(wide.world_refno().word1, 0);
}

// ---- synthetic database -----------------------------------------------------

const WORDS_PER_PAGE: u32 = 64;
const PAGE_BYTES: usize = WORDS_PER_PAGE as usize * 4;
const INDEX_NOUN: u32 = 13_387_743;
const INDEX_HEADER_DWORDS: usize = 7;

fn put(page: &mut [u8], offset: usize, value: u32) {
    page[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

/// One-element database (same layout as `refno_native_stage0.rs`): the single
/// indexed element is `primary/element_no` with the given noun and owner.
///
/// Each fixture needs its own `<3 letters>1000_0001` name: the reader treats
/// `_0002`, `_0003`, … as further *extents* of the `_0001` file next to them,
/// so two fixtures differing only in that suffix would read as one database.
fn write_single_element_db(
    directory: &Path,
    name: &str,
    element_no: u32,
    noun_hash: u32,
    owner: RefNo,
) -> (PathBuf, RefNo) {
    let dbno = 1_000;
    let primary = word0(dbno, 1);
    let element = RefNo::new(primary, element_no);
    let mut pages = vec![vec![0_u8; PAGE_BYTES]; 4];

    put(&mut pages[0], 0x08, dbno);
    put(&mut pages[0], 0x28, 1);
    put(&mut pages[0], 0x2C, 1);
    put(&mut pages[0], 0x30, 1);
    put(&mut pages[0], 0x34, WORDS_PER_PAGE);
    put(&mut pages[0], 0x38, primary);

    put(&mut pages[1], 0x00, 3);
    put(&mut pages[1], 0x04, u32::MAX);
    put(&mut pages[1], 0x0C, 7);
    put(&mut pages[1], 0x1C, 2);
    put(&mut pages[1], 0x20, 1);

    put(&mut pages[2], 0x00, 5);
    for (index, value) in [INDEX_NOUN, 0, 2, 2, 0, 53].into_iter().enumerate() {
        put(&mut pages[2], (index + 1) * 4, value);
    }
    let entry = [element.word0, element.word1, 3, (20_u32 << 12) | 1];
    for (index, value) in entry.into_iter().enumerate() {
        put(&mut pages[2], (INDEX_HEADER_DWORDS + index) * 4, value);
    }

    put(&mut pages[3], 0x00, 7);
    for (index, value) in [
        11,
        element.word0,
        element.word1,
        noun_hash,
        owner.word0,
        owner.word1,
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

    let path = directory.join(name);
    let mut file = File::create(&path).expect("create synthetic E3D database");
    for page in pages {
        file.write_all(&page).expect("write synthetic page");
    }
    file.flush().expect("flush synthetic database");
    (path, element)
}

#[test]
fn world_is_one_point_lookup_and_refuses_impostors() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    let worl = db1_hash("WORL");
    let null = RefNo::new(0, 0);

    // The genuine case: element 0 is an owner-less WORL. One open, one
    // descent, no scan.
    let (path, expected) = write_single_element_db(directory.path(), "tst1000_0001", 0, worl, null);
    read_stats::reset();
    let mut engine = ReadOnlyEngine::open(&path).expect("open synthetic database");
    let world = engine
        .world()
        .expect("world lookup succeeds")
        .expect("element 0 is present");
    assert_eq!(world.refno, expected);
    assert_eq!(world.refno, engine.descriptor().world_refno());
    let header = world.header();
    assert_eq!(header.noun_hash, worl);
    assert!(!header.owner.is_valid());
    let stats = read_stats::snapshot();
    assert_eq!(stats.database_opens, 1);
    assert_eq!(stats.scan_calls, 0);
    assert_eq!(stats.scanned_elements, 0);
    assert_eq!(stats.index_point_lookups, 1);

    // Element 0 exists but is something else: reported, never guessed around.
    let (path, _) =
        write_single_element_db(directory.path(), "imp1000_0001", 0, db1_hash("SITE"), null);
    let mut engine = ReadOnlyEngine::open(&path).expect("open impostor database");
    match engine.world() {
        Err(E3dReaderError::NotAWorld {
            refno, noun_hash, ..
        }) => {
            assert_eq!(refno, engine.descriptor().world_refno());
            assert_eq!(noun_hash, db1_hash("SITE"));
        }
        other => panic!("expected NotAWorld, got {other:?}"),
    }

    // A WORL whose owner is set is not a database root either.
    let (path, _) = write_single_element_db(
        directory.path(),
        "own1000_0001",
        0,
        worl,
        RefNo::new(word0(1_000, 1), 7),
    );
    let mut engine = ReadOnlyEngine::open(&path).expect("open owned-world database");
    assert!(matches!(
        engine.world(),
        Err(E3dReaderError::NotAWorld { .. })
    ));

    // Nothing at element 0: the index simply has no such key.
    let (path, _) = write_single_element_db(directory.path(), "non1000_0001", 42, worl, null);
    let mut engine = ReadOnlyEngine::open(&path).expect("open database without element 0");
    assert!(engine.world().expect("lookup succeeds").is_none());
}
