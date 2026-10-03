//! SPAMAP_Reader on trees built in memory (`tests/common/spamap_pages.rs`): the
//! entries come back as built, a lookup by refno is the table filtered, and
//! each kind of damage requirement 3.2 lists is an error naming the page.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::spamap_pages::{
    leaf_capacity, node_capacity, Shape, SpamapPages, HEADER_DWORDS, LEAF_STRIDE, MIN_PAGE_WORDS, NODE_STRIDE, SENTINEL,
};
use e3d_io::index::{read_spamap_tree, SpamapEntry, SpamapReadError, SPAMAP_NAME_HASH};
use e3d_io::page::PageId;
use e3d_io::RefNo;
use proptest::prelude::*;
use proptest::sample::Index;

/// Entries in key order. Refnos come from a small pool, so most of them own
/// several entries and a lookup has runs to get right.
fn entries(len: std::ops::RangeInclusive<usize>) -> impl Strategy<Value = Vec<SpamapEntry>> {
    prop::collection::vec(((any::<u32>(), 0u32..3, 0u32..40, 0u32..3), prop::array::uniform7(any::<u32>())), len)
        .prop_map(|raw| {
            let mut by_key = BTreeMap::new();
            for ((key0, extent, word1, k), value) in raw {
                let key = [key0, 24_384 + extent, word1, k];
                if key != SENTINEL {
                    by_key.insert(key, value);
                }
            }
            by_key.into_iter().map(|(key, value)| SpamapEntry { key, value }).collect()
        })
}

/// Page lengths from the shortest that holds two internal slots up to a
/// 2048-byte page, with short ones drawn as often as long ones so trees of
/// several levels are common.
fn shapes() -> impl Strategy<Value = Shape> {
    (prop_oneof![MIN_PAGE_WORDS..=64usize, 65..=512usize], any::<bool>(), 1u32..60_000).prop_flat_map(
        |(page_words, leaf_sentinel, first_page)| {
            (1..=leaf_capacity(page_words), 2..=node_capacity(page_words)).prop_map(move |(leaf_fill, node_fill)| {
                Shape { page_words, leaf_fill, node_fill, leaf_sentinel, first_page }
            })
        },
    )
}

proptest! {
    // Feature: write-path-phase-3, Property 1: 按 refno 查询等于全表过滤
    #[test]
    fn a_lookup_by_refno_is_the_table_filtered(entries in entries(0..=300), shape in shapes(), absent in 0u32..40) {
        let tree = SpamapPages::build(&entries, shape);
        let table = read_spamap_tree(tree.root, |page| tree.read(page)).expect("a built tree reads");
        prop_assert_eq!(table.entries(), &entries[..]);
        prop_assert_eq!(table.pages.iter().copied().collect::<BTreeSet<u32>>(), tree.pages.keys().copied().collect());

        let mut refnos: BTreeSet<(u32, u32)> = entries.iter().map(|entry| (entry.key[1], entry.key[2])).collect();
        // An extent the strategy never draws, so this refno is in no table.
        refnos.insert((24_390, absent));
        for (word0, word1) in refnos {
            let refno = RefNo::new(word0, word1);
            let looked_up: Vec<&SpamapEntry> = table.for_refno(refno).collect();
            let filtered: Vec<&SpamapEntry> = table.entries().iter().filter(|entry| entry.refno() == refno).collect();
            prop_assert_eq!(looked_up, filtered);
        }
    }
}

/// One kind of damage from requirement 3.2.
#[derive(Debug, Clone, Copy)]
enum Damage {
    Unreadable,
    PageType,
    NameHash,
    KeyWidth,
    LeafValueWidth,
    FreeBeyondPage,
    FreeOffStride,
    LevelNotOneBelow,
    NoChildren,
    PageReachedTwice,
    KeyNotAscending,
}

fn damages() -> impl Strategy<Value = Damage> {
    prop::sample::select(vec![
        Damage::Unreadable,
        Damage::PageType,
        Damage::NameHash,
        Damage::KeyWidth,
        Damage::LeafValueWidth,
        Damage::FreeBeyondPage,
        Damage::FreeOffStride,
        Damage::LevelNotOneBelow,
        Damage::NoChildren,
        Damage::PageReachedTwice,
        Damage::KeyNotAscending,
    ])
}

/// A shape and enough entries for at least two leaves, so the root is an
/// internal page with two children or more and every damage has a target.
fn damageable() -> impl Strategy<Value = (Shape, Vec<SpamapEntry>)> {
    shapes().prop_flat_map(|shape| {
        let least = 2 * shape.leaf_fill + 1;
        (Just(shape), entries(least..=least + 200)).prop_filter("two leaves at least", move |(_, entries)| {
            entries.len() >= least
        })
    })
}

/// Damage `tree` and say which page the reader must name.
fn damage(tree: &mut SpamapPages, damage: Damage, pick: Index) -> u32 {
    let usable = (tree.page_words - HEADER_DWORDS) as u32;
    let all: Vec<u32> = tree.pages.keys().copied().collect();
    let leaves = tree.levels[0].clone();
    let internal: Vec<u32> = tree.levels[1..].iter().flatten().copied().collect();
    let any_page = *pick.get(&all);
    match damage {
        Damage::Unreadable => {
            tree.pages.remove(&any_page);
            any_page
        }
        Damage::PageType => {
            tree.pages.get_mut(&any_page).unwrap()[0] = 6;
            any_page
        }
        Damage::NameHash => {
            tree.pages.get_mut(&any_page).unwrap()[1] = SPAMAP_NAME_HASH + 1;
            any_page
        }
        Damage::KeyWidth => {
            tree.pages.get_mut(&any_page).unwrap()[3] = 5;
            any_page
        }
        Damage::LeafValueWidth => {
            let leaf = *pick.get(&leaves);
            tree.pages.get_mut(&leaf).unwrap()[4] = 6;
            leaf
        }
        Damage::FreeBeyondPage => {
            tree.pages.get_mut(&any_page).unwrap()[6] = usable + 1;
            any_page
        }
        Damage::FreeOffStride => {
            // Every page holds a slot, so one dword more or less in use is off
            // the stride either way.
            let words = tree.pages.get_mut(&any_page).unwrap();
            words[6] = if words[6] > 0 { words[6] - 1 } else { 1 };
            any_page
        }
        Damage::LevelNotOneBelow => {
            let page = *pick.get(&internal);
            tree.pages.get_mut(&page).unwrap()[2] += 1;
            page
        }
        Damage::NoChildren => {
            let page = *pick.get(&internal);
            tree.pages.get_mut(&page).unwrap()[6] = usable;
            page
        }
        Damage::PageReachedTwice => {
            let parents: Vec<u32> = internal.iter().copied().filter(|page| slots(tree, *page) >= 2).collect();
            let page = *pick.get(&parents);
            let words = tree.pages.get_mut(&page).unwrap();
            let first_child = words[HEADER_DWORDS + 4];
            words[HEADER_DWORDS + NODE_STRIDE + 4] = first_child;
            first_child
        }
        Damage::KeyNotAscending => {
            // Every (page, slot) of a real entry, left to right.
            let mut at = Vec::new();
            for &leaf in &leaves {
                for slot in 0..slots(tree, leaf) {
                    let start = HEADER_DWORDS + slot * LEAF_STRIDE;
                    if tree.pages[&leaf][start..start + 4] != SENTINEL {
                        at.push((leaf, start));
                    }
                }
            }
            let pairs: Vec<usize> = (1..at.len()).collect();
            let later = *pick.get(&pairs);
            let ((before_page, before), (page, start)) = (at[later - 1], at[later]);
            let key: Vec<u32> = tree.pages[&before_page][before..before + 4].to_vec();
            tree.pages.get_mut(&page).unwrap()[start..start + 4].copy_from_slice(&key);
            page
        }
    }
}

fn slots(tree: &SpamapPages, page: u32) -> usize {
    let words = &tree.pages[&page];
    let stride = if words[2] == 0 { LEAF_STRIDE } else { NODE_STRIDE };
    (tree.page_words - HEADER_DWORDS - words[6] as usize) / stride
}

proptest! {
    // Feature: write-path-phase-3, Property 3: 畸形页必报错
    #[test]
    fn every_kind_of_damage_is_an_error_naming_the_page(
        (shape, entries) in damageable(),
        kind in damages(),
        pick in any::<Index>(),
    ) {
        let mut tree = SpamapPages::build(&entries, shape);
        let page = PageId::primary(damage(&mut tree, kind, pick));
        let error = read_spamap_tree(tree.root, |page| tree.read(page)).expect_err("a damaged tree is refused");
        let named = match (&error, kind) {
            (SpamapReadError::Unreadable { page: at, .. }, Damage::Unreadable)
            | (SpamapReadError::UnexpectedPageType { page: at, found: 6 }, Damage::PageType)
            | (SpamapReadError::WrongTable { page: at, .. }, Damage::NameHash)
            | (SpamapReadError::WrongShape { page: at, key_dwords: 5, .. }, Damage::KeyWidth)
            | (SpamapReadError::WrongShape { page: at, level: 0, value_dwords: 6, .. }, Damage::LeafValueWidth)
            | (SpamapReadError::FreeSpaceOutOfRange { page: at, .. }, Damage::FreeBeyondPage)
            | (SpamapReadError::FreeSpaceNotEntryAligned { page: at, .. }, Damage::FreeOffStride)
            | (SpamapReadError::NoChildren { page: at, .. }, Damage::NoChildren)
            | (SpamapReadError::Revisited { page: at }, Damage::PageReachedTwice)
            | (SpamapReadError::KeyNotAscending { page: at, .. }, Damage::KeyNotAscending) => *at == page,
            // Raised level: the page's parent no longer has it one level
            // below, or the page no longer has its children one below.
            (SpamapReadError::LevelNotDecreasing { parent, child, .. }, Damage::LevelNotOneBelow) => {
                *parent == page || *child == page
            }
            _ => false,
        };
        prop_assert!(named, "{kind:?} on {page} gave: {error}");
    }
}

/// A table with no entries is a table, not an absent one: one leaf, nothing in it.
#[test]
fn an_empty_tree_reads_as_no_entries() {
    for leaf_sentinel in [false, true] {
        let shape = Shape { page_words: 512, leaf_fill: 45, node_fill: 84, leaf_sentinel, first_page: 7 };
        let tree = SpamapPages::build(&[], shape);
        let table = read_spamap_tree(tree.root, |page| tree.read(page)).expect("an empty tree reads");
        assert!(table.entries().is_empty());
        assert_eq!(table.pages, vec![7]);
    }
}
