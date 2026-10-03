//! The pages and words of every SPAMAP table E3D wrote, on the baseline
//! session and on every Corpus database's newest session
//! (`docs/evidence/2026-10-03-spamap-encoding.md`):
//!
//! - `spamap.table_shape`, `spamap.table_header_rest`: each page reached from
//!   the root reads `[5, 230902453, level, 4, 7, 1, free]` — internal pages
//!   too, which keep the leaf's value width — the words core.dll's
//!   `sub_5AFF6D0` gives a new root, free aside.
//! - `spamap.word_sign`: every key word is below 2^31, where core.dll's signed
//!   and unsigned key comparisons agree, and every coordinate is one a datum
//!   of 0 holds.

mod common;

use common::corpus::{CORPUS, SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION};
use e3d_io::index::{read_spamap, SpamapRead, SPAMAP_NAME_HASH};
use e3d_io::page::PageId;
use e3d_io::session::DbView;
use e3d_io::spatial::rules::WITHIN_DATUM;

/// The baseline session, then each Corpus database's newest.
fn views() -> Vec<(&'static str, DbView)> {
    let mut views = vec![(
        SPAMAP_BASELINE,
        DbView::open_at(SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION).expect("session 640 is on the chain"),
    )];
    views.extend(CORPUS.iter().map(|&path| (path, DbView::open(path).expect("open the newest session"))));
    views
}

/// Pages and of them internal ones, after asserting every header.
fn check_headers(path: &str, view: &mut DbView) -> Option<(usize, usize)> {
    let SpamapRead::Table { sesno, table } = read_spamap(view).unwrap_or_else(|error| panic!("{path}: {error}")) else {
        return None;
    };
    let mut internal = 0;
    for &page in &table.pages {
        let bytes = view.pages_mut().get(PageId::new(table.root.ext, page)).unwrap_or_else(|error| panic!("{path}: {error}"));
        let word = |index: usize| u32::from_be_bytes(bytes[index * 4..index * 4 + 4].try_into().expect("four bytes"));
        let level = word(2);
        assert_eq!(
            [word(0), word(1), word(3), word(4), word(5)],
            [5, SPAMAP_NAME_HASH, 4, 7, 1],
            "{path} session {sesno} page {page} at level {level}"
        );
        if level > 0 {
            internal += 1;
        }
    }
    Some((table.pages.len(), internal))
}

#[test]
fn every_page_carries_the_header_core_dll_gives_a_new_root() {
    if common::missing(CORPUS) {
        return;
    }
    let (mut pages, mut internal) = (0, 0);
    for (path, mut view) in views() {
        match check_headers(path, &mut view) {
            Some((these, inner)) => {
                eprintln!("{path}: session {}, {these} pages, {inner} internal", view.selected_session());
                pages += these;
                internal += inner;
            }
            None => eprintln!("{path}: session {} lists no SPAMAP", view.selected_session()),
        }
    }
    eprintln!("{pages} pages, {internal} internal, every header [5, {SPAMAP_NAME_HASH}, level, 4, 7, 1, free]");
    assert!(internal > 0, "the Corpus has internal pages to hold to the rule");
}

#[test]
fn every_key_word_is_below_2_31_and_every_coordinate_within_a_datum_of_0() {
    if common::missing(CORPUS) {
        return;
    }
    let (mut entries, mut key_max, mut least, mut most) = (0usize, [0u32; 4], i64::MAX, i64::MIN);
    for (path, mut view) in views() {
        let SpamapRead::Table { table, .. } = read_spamap(&mut view).unwrap_or_else(|error| panic!("{path}: {error}")) else {
            continue;
        };
        for entry in table.entries() {
            for (max, &word) in key_max.iter_mut().zip(&entry.key) {
                assert!(word < 1 << 31, "{path}: key {:?}", entry.key);
                *max = (*max).max(word);
            }
            for word in entry.aabb_words() {
                let coordinate = i64::from(word as i32);
                assert!(coordinate.abs() <= WITHIN_DATUM, "{path}: entry {:?}", entry.key);
                (least, most) = (least.min(coordinate), most.max(coordinate));
            }
            entries += 1;
        }
    }
    eprintln!("{entries} entries; key words at most {key_max:?}; coordinates in [{least}, {most}] (0.1 mm)");
    assert!(entries > 0);
}
