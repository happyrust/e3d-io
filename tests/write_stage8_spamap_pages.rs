//! The SPAMAP page tree the writer builds (task 8.2): entries encoded by the
//! keyed-table copy-on-write with a seven-dword leaf value
//! (`e3d_io::write::build_table` / `rewrite_table`) and read back by the
//! SPAMAP_Reader come back as they went in, on any legal page length and on
//! every Corpus table; an edit set applied once or twice gives the same
//! table, the one a `BTreeMap` with core's 534 / 529 rules gives.

mod common;

use std::collections::{BTreeMap, HashMap};

use common::corpus::CORPUS;
use common::spamap_pages::MIN_PAGE_WORDS;
use e3d_io::index::{read_spamap, read_spamap_tree, SpamapRead, SPAMAP_NAME_HASH};
use e3d_io::page::PageId;
use e3d_io::session::{DbView, SessionSelector};
use e3d_io::write::{build_table, rewrite_table, KeyedKey, KeyedRewrite, TableEdit, TableEditKind};
use proptest::prelude::*;

type Entry = (KeyedKey, [u32; 7]);

/// Header word 5 of every SPAMAP page (`spamap.table_header_rest`).
const SPAMAP_FLAGS: u32 = 1;
const SENTINEL: KeyedKey = [0x8000_0001; 4];

/// Entries in key order, keys from a small pool so edits hit them.
fn entries(len: std::ops::RangeInclusive<usize>) -> impl Strategy<Value = Vec<Entry>> {
    prop::collection::vec(((any::<u32>(), 0u32..3, 0u32..60, 0u32..2), prop::array::uniform7(any::<u32>())), len).prop_map(
        |raw| {
            let mut by_key = BTreeMap::new();
            for ((key0, extent, word1, k), value) in raw {
                let key = [key0 % 64, 24_384 + extent, word1, k];
                if key != SENTINEL {
                    by_key.insert(key, value);
                }
            }
            by_key.into_iter().collect()
        },
    )
}

/// Page lengths from the shortest a tree can grow in up to a 2048-byte page,
/// short ones as likely as long ones so trees of several levels are common.
fn page_words() -> impl Strategy<Value = usize> {
    prop_oneof![MIN_PAGE_WORDS..=64usize, 65..=512usize]
}

/// The entries under `root` in `pages`, through the SPAMAP_Reader.
fn read_back(pages: &HashMap<u32, Vec<u8>>, root: PageId) -> Vec<Entry> {
    let table = read_spamap_tree(root, |page| pages.get(&page.page).cloned().ok_or_else(|| format!("no page {page:?}")))
        .expect("the SPAMAP_Reader walks what the writer wrote");
    table.entries().iter().map(|entry| (entry.key, entry.value)).collect()
}

/// `edits` applied to the table at `root` in `pages`, new pages added to them.
fn apply(pages: &mut HashMap<u32, Vec<u8>>, root: PageId, page_words: usize, edits: &[TableEdit<7>]) -> PageId {
    let mut next = pages.keys().max().copied().unwrap_or(0) + 1;
    let rewrite: KeyedRewrite = {
        let snapshot = &*pages;
        let mut read = |page_no: u32| snapshot.get(&page_no).cloned().ok_or_else(|| format!("no page {page_no}"));
        let mut alloc = || {
            let page = next;
            next += 1;
            page
        };
        rewrite_table::<7>(Some(root), SPAMAP_NAME_HASH, SPAMAP_FLAGS, page_words, edits, &mut read, &mut alloc, u32::MAX)
            .expect("the rewrite")
    };
    pages.extend(rewrite.pages);
    rewrite.root
}

fn built(page_words: usize, entries: &[Entry]) -> (HashMap<u32, Vec<u8>>, PageId) {
    let rewrite = build_table::<7>(SPAMAP_NAME_HASH, SPAMAP_FLAGS, page_words, entries).expect("build the table");
    (rewrite.pages.into_iter().collect(), rewrite.root)
}

proptest! {
    // Feature: write-path-phase-3, Property 2: 条目序列往返
    #[test]
    fn entries_built_into_a_tree_read_back_as_they_went_in(page_words in page_words(), entries in entries(0..=120)) {
        let (pages, root) = built(page_words, &entries);
        prop_assert_eq!(read_back(&pages, root), entries);
    }

    // Feature: write-path-phase-3, Property 10: 编辑集合幂等
    #[test]
    fn an_edit_set_applied_twice_is_the_edit_set_applied_once(
        page_words in page_words(),
        before in entries(0..=80),
        deletes in prop::collection::vec((0usize..200, any::<bool>()), 0..20),
        inserts in entries(0..=30),
    ) {
        // Deletes of present and absent keys, then inserts of new and present
        // keys: core applies a set's deletes first, then its inserts.
        let mut edits: Vec<TableEdit<7>> = deletes
            .iter()
            .map(|&(at, present)| {
                let key = if present && !before.is_empty() { before[at % before.len()].0 } else { [65, 24_384, 99, at as u32] };
                TableEdit { key, kind: TableEditKind::Delete }
            })
            .collect();
        edits.extend(inserts.iter().map(|&(key, value)| TableEdit { key, kind: TableEditKind::Insert(value) }));

        let mut model: BTreeMap<KeyedKey, [u32; 7]> = before.iter().copied().collect();
        for edit in &edits {
            match edit.kind {
                TableEditKind::Delete => {
                    model.remove(&edit.key);
                }
                TableEditKind::Insert(value) => {
                    model.entry(edit.key).or_insert(value);
                }
            }
        }

        let (mut pages, root) = built(page_words, &before);
        let once = apply(&mut pages, root, page_words, &edits);
        let twice = apply(&mut pages, once, page_words, &edits);
        let expected: Vec<Entry> = model.into_iter().collect();
        prop_assert_eq!(read_back(&pages, once), expected.clone());
        prop_assert_eq!(read_back(&pages, twice), expected);
    }
}

/// Every Corpus table read, encoded again and read back is itself.
#[test]
fn every_corpus_table_is_encoded_back_to_the_same_entries() {
    if common::missing(CORPUS) {
        return;
    }
    for path in CORPUS {
        let mut view = DbView::open_at(path, SessionSelector::Latest).expect("open the database");
        let table = match read_spamap(&mut view).unwrap_or_else(|error| panic!("{path}: {error}")) {
            SpamapRead::NoTable { .. } => continue,
            SpamapRead::Table { table, .. } => table,
        };
        let entries: Vec<Entry> = table.entries().iter().map(|entry| (entry.key, entry.value)).collect();
        let (pages, root) = built(512, &entries);
        assert_eq!(read_back(&pages, root), entries, "{path}");
        eprintln!("{path}: {} entries on {} pages", entries.len(), pages.len());
    }
}
