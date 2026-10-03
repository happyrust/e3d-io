//! SPAMAP, the table E3D's volume search reads (`WITHIN VOLUME`, the 3D view's
//! limits): one named B+tree per session, four-dword keys and a seven-dword
//! leaf value (`docs/evidence/2026-09-22-e3d-spamap-consequences.md`).
//!
//! ```text
//! key   [Key0, refno.word0, refno.word1, k]
//! value [minx, miny, minz, maxx, maxy, maxz, flags]
//! ```
//!
//! This module reads positions, not meanings. How Key0 is formed, what `k`
//! counts and what the flags say are rules of E3D_Core, held in the rule
//! registry with their evidence; here a key is four words in unsigned order
//! and a value is seven words as stored.
//!
//! A table is read whole and checked whole before any entry is handed out:
//! every check of requirement 3.2 (`.kiro/specs/write-path-phase-3`) runs on
//! every page reachable from the root, so a broken table is an error naming
//! the page, never a short list. Pages come through one closure, so the write
//! side and tests on pages in memory read with the same code as a file.
//!
//! A lookup by refno cannot use the tree: the tree is ordered by Key0, and the
//! refno is the second and third key word. The first lookup indexes the
//! entries in memory, O(entries); each one after costs its own entries.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use super::keyed::{self, KeyedPageError, KEY_DWORDS, TABLE_PAGE_TYPE};
use crate::page::PageId;
use crate::refno::RefNo;
use crate::session::DbView;

/// SPAMAP's name in a session's table directory.
pub const SPAMAP_NAME_HASH: u32 = 230_902_453;

/// Value dwords of a SPAMAP leaf entry: six bounding-box words and the flags.
pub const SPAMAP_VALUE_DWORDS: u32 = 7;

/// One leaf entry, the eleven words as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpamapEntry {
    pub key: [u32; 4],
    pub value: [u32; 7],
}

impl SpamapEntry {
    pub fn key0(&self) -> u32 {
        self.key[0]
    }

    pub fn refno(&self) -> RefNo {
        RefNo::new(self.key[1], self.key[2])
    }

    pub fn k(&self) -> u32 {
        self.key[3]
    }

    /// `[minx, miny, minz, maxx, maxy, maxz]` as stored.
    pub fn aabb_words(&self) -> [u32; 6] {
        [self.value[0], self.value[1], self.value[2], self.value[3], self.value[4], self.value[5]]
    }

    pub fn flags(&self) -> u32 {
        self.value[6]
    }
}

/// What one session says about SPAMAP.
#[derive(Debug)]
pub enum SpamapRead {
    /// The session's table directory has no SPAMAP. Not the same answer as a
    /// table with no entries, which is [`SpamapRead::Table`].
    NoTable { sesno: u32 },
    Table { sesno: u32, table: SpamapTable },
}

/// A SPAMAP table read whole.
#[derive(Debug)]
pub struct SpamapTable {
    pub root: PageId,
    /// Every page reached from the root, in the order the walk reached them.
    pub pages: Vec<u32>,
    entries: Vec<SpamapEntry>,
    by_refno: OnceLock<HashMap<RefNo, Vec<usize>>>,
}

impl SpamapTable {
    /// The leaf entries in key order, sentinels left out.
    pub fn entries(&self) -> &[SpamapEntry] {
        &self.entries
    }

    /// The entries whose key names `refno`, in key order. None is an answer:
    /// the table was read, and no entry names it.
    pub fn for_refno(&self, refno: RefNo) -> impl Iterator<Item = &SpamapEntry> + '_ {
        let by_refno = self.by_refno.get_or_init(|| {
            let mut by_refno: HashMap<RefNo, Vec<usize>> = HashMap::new();
            for (index, entry) in self.entries.iter().enumerate() {
                by_refno.entry(entry.refno()).or_default().push(index);
            }
            by_refno
        });
        by_refno.get(&refno).into_iter().flatten().map(|&index| &self.entries[index])
    }
}

/// Why a SPAMAP table could not be read. Every case names the page, and none
/// comes with the entries read before it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpamapReadError {
    #[error("SPAMAP {page} could not be read: {detail}")]
    Unreadable { page: PageId, detail: String },

    #[error("SPAMAP {page} length {len} is not a whole number of 4-byte words")]
    PageNotDwordAligned { page: PageId, len: usize },

    #[error("SPAMAP {page} is {len} bytes, too small for the {needed}-byte table page header")]
    PageTooSmall { page: PageId, len: usize, needed: usize },

    #[error("SPAMAP {page} is page type {found}, expected {TABLE_PAGE_TYPE}")]
    UnexpectedPageType { page: PageId, found: u32 },

    #[error("SPAMAP {page} belongs to table {found}, not SPAMAP ({SPAMAP_NAME_HASH})")]
    WrongTable { page: PageId, found: u32 },

    #[error(
        "SPAMAP {page} at level {level} states a {key_dwords}-dword key and a {value_dwords}-dword value; \
         SPAMAP keys are {KEY_DWORDS} dwords and its leaf values {SPAMAP_VALUE_DWORDS}"
    )]
    WrongShape { page: PageId, level: u32, key_dwords: u32, value_dwords: u32 },

    #[error("SPAMAP {page}: free_dwords is {free_dwords} but only {usable_dwords} dwords follow the header")]
    FreeSpaceOutOfRange { page: PageId, free_dwords: u32, usable_dwords: u32 },

    #[error("SPAMAP {page}: {used_dwords} dwords in use is not a whole number of {stride_dwords}-dword entries")]
    FreeSpaceNotEntryAligned { page: PageId, used_dwords: u64, stride_dwords: u64 },

    #[error("SPAMAP {parent} is at level {parent_level}, so its child {child} must be at level {expected}, but it is at {found}")]
    LevelNotDecreasing { parent: PageId, parent_level: u32, child: PageId, expected: u32, found: u32 },

    #[error("SPAMAP {page} is an internal page at level {level} with no children")]
    NoChildren { page: PageId, level: u32 },

    #[error("SPAMAP {page} is reached a second time")]
    Revisited { page: PageId },

    #[error("SPAMAP {page} holds key {found:?} after {previous:?}, which is not strictly ascending")]
    KeyNotAscending { page: PageId, previous: [u32; 4], found: [u32; 4] },
}

/// The SPAMAP table of the session `view` was opened at.
///
/// The session is chosen when the view is opened: `DbView::open_at(path, 640)`
/// refuses a session that is not on the chain rather than reading another.
pub fn read_spamap(view: &mut DbView) -> Result<SpamapRead, SpamapReadError> {
    let sesno = view.selected_session();
    let Some(root) = view.table_root(SPAMAP_NAME_HASH) else {
        return Ok(SpamapRead::NoTable { sesno });
    };
    let pages = view.pages_mut();
    let table = read_spamap_tree(root, |page| pages.get(page).map(<[u8]>::to_vec).map_err(|error| error.to_string()))?;
    Ok(SpamapRead::Table { sesno, table })
}

/// Read and check the SPAMAP tree under `root`. Children are in the root's
/// extent; `read_page` hands over a page's bytes or says why it cannot.
pub fn read_spamap_tree(
    root: PageId,
    mut read_page: impl FnMut(PageId) -> Result<Vec<u8>, String>,
) -> Result<SpamapTable, SpamapReadError> {
    let mut entries: Vec<SpamapEntry> = Vec::new();
    let mut pages: Vec<u32> = Vec::new();
    let mut visited: HashSet<PageId> = HashSet::new();
    // Depth first in slot order: children go on in reverse so the first comes
    // off first, and leaves are met left to right.
    let mut pending: Vec<(PageId, Option<(PageId, u32)>)> = vec![(root, None)];
    while let Some((page, parent)) = pending.pop() {
        if !visited.insert(page) {
            return Err(SpamapReadError::Revisited { page });
        }
        pages.push(page.page);
        let data = read_page(page).map_err(|detail| SpamapReadError::Unreadable { page, detail })?;
        let node = keyed::decode::<{ SPAMAP_VALUE_DWORDS as usize }>(&data, SPAMAP_NAME_HASH)
            .map_err(|error| page_error(page, error))?;
        // Only internal pages have children, so `parent_level` is at least 1.
        if let Some((parent, parent_level)) = parent {
            let expected = parent_level - 1;
            if node.level != expected {
                return Err(SpamapReadError::LevelNotDecreasing { parent, parent_level, child: page, expected, found: node.level });
            }
        }
        if node.level == 0 {
            for slot in &node.slots {
                let Some(key) = slot.key else { continue };
                if let Some(previous) = entries.last() {
                    if key <= previous.key {
                        return Err(SpamapReadError::KeyNotAscending { page, previous: previous.key, found: key });
                    }
                }
                entries.push(SpamapEntry { key, value: slot.value });
            }
            continue;
        }
        let children: Vec<u32> = node.slots.iter().filter_map(|slot| slot.child).collect();
        if children.is_empty() {
            return Err(SpamapReadError::NoChildren { page, level: node.level });
        }
        for &child in children.iter().rev() {
            pending.push((PageId::new(root.ext, child), Some((page, node.level))));
        }
    }
    Ok(SpamapTable { root, pages, entries, by_refno: OnceLock::new() })
}

fn page_error(page: PageId, error: KeyedPageError) -> SpamapReadError {
    match error {
        KeyedPageError::PageNotDwordAligned { len } => SpamapReadError::PageNotDwordAligned { page, len },
        KeyedPageError::PageTooSmall { len, needed } => SpamapReadError::PageTooSmall { page, len, needed },
        KeyedPageError::UnexpectedPageType { found } => SpamapReadError::UnexpectedPageType { page, found },
        KeyedPageError::WrongTable { found } => SpamapReadError::WrongTable { page, found },
        KeyedPageError::WrongShape { level, key_dwords, value_dwords } => {
            SpamapReadError::WrongShape { page, level, key_dwords, value_dwords }
        }
        KeyedPageError::FreeSpaceOutOfRange { free_dwords, usable_dwords } => {
            SpamapReadError::FreeSpaceOutOfRange { page, free_dwords, usable_dwords }
        }
        KeyedPageError::FreeSpaceNotEntryAligned { used_dwords, stride_dwords } => {
            SpamapReadError::FreeSpaceNotEntryAligned { page, used_dwords, stride_dwords }
        }
    }
}
