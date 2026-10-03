//! SPAMAP page trees built in memory for the reader's tests, without the write
//! side: the read side is checked against a builder it does not share code
//! with (requirement 2.5).
//!
//! A tree is built bottom up — leaves, then internal levels over them — in the
//! layout the reader checks: page type 5, SPAMAP's name hash, four key dwords,
//! a seven-dword leaf value, `free_dwords` in the seventh header dword. Each
//! internal page opens with the sentinel, pointing at its first child; every
//! other internal slot carries the least key under its child.

#![allow(dead_code)]

use std::collections::BTreeMap;

use e3d_io::index::{SpamapEntry, SPAMAP_NAME_HASH};
use e3d_io::page::PageId;

pub const SENTINEL: [u32; 4] = [0x8000_0001; 4];
pub const HEADER_DWORDS: usize = 7;
pub const LEAF_STRIDE: usize = 11;
pub const NODE_STRIDE: usize = 6;
/// A page short enough that two internal slots still fit, so any tree can be
/// built with it.
pub const MIN_PAGE_WORDS: usize = HEADER_DWORDS + 2 * NODE_STRIDE;

/// How the tree is cut into pages.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    pub page_words: usize,
    /// Slots per leaf, at most [`leaf_capacity`].
    pub leaf_fill: usize,
    /// Slots per internal page, at least 2 and at most [`node_capacity`].
    pub node_fill: usize,
    /// Whether the first leaf opens with a sentinel slot, as E3D's do.
    pub leaf_sentinel: bool,
    pub first_page: u32,
}

pub fn leaf_capacity(page_words: usize) -> usize {
    (page_words - HEADER_DWORDS) / LEAF_STRIDE
}

pub fn node_capacity(page_words: usize) -> usize {
    (page_words - HEADER_DWORDS) / NODE_STRIDE
}

/// A built tree: its pages as words, by page number, so a test can change one
/// word and read again.
#[derive(Debug, Clone)]
pub struct SpamapPages {
    pub root: PageId,
    pub page_words: usize,
    pub pages: BTreeMap<u32, Vec<u32>>,
    /// Page numbers level by level, leaves first.
    pub levels: Vec<Vec<u32>>,
}

impl SpamapPages {
    /// Build the tree of `entries`, which must be in strictly ascending key
    /// order and hold no sentinel key.
    pub fn build(entries: &[SpamapEntry], shape: Shape) -> SpamapPages {
        assert!(shape.page_words >= MIN_PAGE_WORDS, "page of {} words is too short", shape.page_words);
        assert!(shape.leaf_fill >= 1 && shape.leaf_fill <= leaf_capacity(shape.page_words));
        assert!(shape.node_fill >= 2 && shape.node_fill <= node_capacity(shape.page_words));
        let mut pages = BTreeMap::new();
        let mut next_page = shape.first_page;
        let mut take_page = || {
            let page = next_page;
            next_page += 1;
            page
        };

        // Leaves, each as (page, least key under it).
        let mut slots: Vec<Vec<([u32; 4], [u32; 7])>> = Vec::new();
        let mut rest = entries;
        let mut first = true;
        loop {
            let mut leaf = Vec::new();
            if first && shape.leaf_sentinel {
                leaf.push((SENTINEL, [0; 7]));
            }
            let take = (shape.leaf_fill - leaf.len()).min(rest.len());
            leaf.extend(rest[..take].iter().map(|entry| (entry.key, entry.value)));
            rest = &rest[take..];
            slots.push(leaf);
            first = false;
            if rest.is_empty() {
                break;
            }
        }
        let mut level_pages: Vec<(u32, [u32; 4])> = Vec::new();
        for leaf in slots {
            let page = take_page();
            let least = leaf.iter().map(|(key, _)| *key).find(|key| *key != SENTINEL).unwrap_or(SENTINEL);
            let mut words = Vec::new();
            for (key, value) in &leaf {
                words.extend_from_slice(key);
                words.extend_from_slice(value);
            }
            pages.insert(page, page_words(shape.page_words, 0, &words));
            level_pages.push((page, least));
        }

        let mut levels = vec![level_pages.iter().map(|(page, _)| *page).collect::<Vec<_>>()];
        let mut level = 0;
        while level_pages.len() > 1 {
            level += 1;
            let mut above = Vec::new();
            for children in level_pages.chunks(shape.node_fill) {
                let page = take_page();
                let mut words = Vec::new();
                for (index, (child, least)) in children.iter().enumerate() {
                    words.extend_from_slice(if index == 0 { &SENTINEL } else { least });
                    words.push(*child);
                    words.push(u32::from(PageId::PRIMARY));
                }
                pages.insert(page, page_words(shape.page_words, level, &words));
                above.push((page, children[0].1));
            }
            levels.push(above.iter().map(|(page, _)| *page).collect());
            level_pages = above;
        }

        SpamapPages { root: PageId::primary(level_pages[0].0), page_words: shape.page_words, pages, levels }
    }

    /// The reader's page source.
    pub fn read(&self, page: PageId) -> Result<Vec<u8>, String> {
        self.pages
            .get(&page.page)
            .map(|words| words.iter().flat_map(|word| word.to_be_bytes()).collect())
            .ok_or_else(|| format!("no page {}", page.page))
    }
}

/// One page: the header, the slot words, and zeros to the page's end.
fn page_words(page_words: usize, level: u32, slots: &[u32]) -> Vec<u32> {
    let usable = page_words - HEADER_DWORDS;
    assert!(slots.len() <= usable, "{} slot words do not fit in a {page_words}-word page", slots.len());
    let mut words = vec![5, SPAMAP_NAME_HASH, level, 4, 7, 0, (usable - slots.len()) as u32];
    words.extend_from_slice(slots);
    words.resize(page_words, 0);
    words
}
