//! Copy-on-write edits to the `(5, 7618377)` claim tree — the primary-element
//! session index ([`crate::index::claim`], `docs/plans/2026-09-20-dabacon-write-plan.md`
//! §7 CLAIM, A3a.6 `sub_5B0C760`).
//!
//! The tree is the refno index's page shape with a different value: a 2-dword
//! refno key and a 2-dword `(owner, session)` value, page type 5, the name
//! hash `7618377` in word 1. So this is [`super::index_cow`]'s copy-on-write —
//! copy the root-to-leaf path, append fresh page numbers, leave every other
//! subtree alone, split an overflowing page at the median and grow a root above
//! a split root — but the leaf value is the two claim words rather than a
//! packed record address, and a key may be updated in place (a primary's
//! session is bumped) as well as inserted or removed.
//!
//! An insert of a key already present updates it, and a delete of a key that is
//! not there is nothing to do — the tolerances `sub_5B0C760` keeps through the
//! claims manager's 529/534 (`docs/evidence/2026-09-20-core-dll-dabacon-write-path-algorithms.md`
//! A3a.6).

use crate::index::cmp_refno;
use crate::meta::constants::DATA_PAGE_MAIN;
use crate::page::PageId;
use crate::refno::RefNo;

const HEADER_DWORDS: usize = 7;
const ENTRY_DWORDS: usize = 4;
const CHILD_ENTRY_DWORDS: usize = 4;
const TABLE_PAGE_TYPE: u32 = 5;
const SENTINEL_WORD: u32 = 0x8000_0001;

/// What a [`ClaimEdit`] does to its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimEditKind {
    /// File `(owner, session)` for the primary, or update it if already there
    /// — a newly created primary.
    Set { owner: u32, session: u32 },
    /// Set an *existing* primary's session, keeping its owner; a key that is
    /// not there is nothing to do (the claims manager's 534). This is the bump
    /// a touched primary gets, which must not conjure a claim for a primary the
    /// tree never held.
    Refresh { session: u32 },
    /// Remove the primary's claim; a key that is not there is nothing to do.
    Remove,
}

/// One change to the claim tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimEdit {
    pub refno: RefNo,
    pub kind: ClaimEditKind,
}

/// The new tree: its root and every page the rewrite produced.
#[derive(Debug)]
pub struct ClaimRewrite {
    pub root: PageId,
    pub pages: Vec<(u32, Vec<u8>)>,
}

#[derive(Debug, thiserror::Error)]
pub enum ClaimCowError {
    #[error("claim page {page_no} could not be read: {detail}")]
    Unreadable { page_no: u32, detail: String },
    #[error("claim page {page_no} is page type {found}, expected {TABLE_PAGE_TYPE}")]
    NotATablePage { page_no: u32, found: u32 },
    #[error("page {page_no} belongs to table {found}, expected {expected} for the claim tree")]
    WrongTable { page_no: u32, found: u32, expected: u32 },
    #[error("claim page {page_no} states a {key_dwords}-dword key and a {value_dwords}-dword value; the claim tree states 2 and 2")]
    NotAClaimTable { page_no: u32, key_dwords: u32, value_dwords: u32 },
    #[error("claim page {page_no}: free_dwords {free_dwords} exceeds the {usable_dwords} usable")]
    FreeSpaceOutOfRange { page_no: u32, free_dwords: u32, usable_dwords: u32 },
    #[error("claim page {page_no}: {used_dwords} dwords in use is not a whole number of {stride}-dword entries")]
    NotEntryAligned { page_no: u32, used_dwords: usize, stride: usize },
    #[error("an internal claim page routes {refno} to a child it does not list")]
    ChildNotFound { refno: RefNo },
    #[error("the claim tree is deeper than the {limit} pages the file holds")]
    TooDeep { limit: u32 },
}

/// One decoded slot: a key and its `(owner, session)`, or the sentinel; on an
/// internal page, the child behind it.
#[derive(Debug, Clone, Copy)]
struct Slot {
    key: Option<RefNo>,
    value: [u32; 2],
    child: Option<u32>,
}

struct Node {
    level: u32,
    slots: Vec<Slot>,
}

fn decode(page_no: u32, data: &[u8]) -> Result<Node, ClaimCowError> {
    let word = |index: usize| u32::from_be_bytes(data[index * 4..index * 4 + 4].try_into().expect("four bytes"));
    let page_type = word(0);
    if page_type != TABLE_PAGE_TYPE {
        return Err(ClaimCowError::NotATablePage { page_no, found: page_type });
    }
    let found = word(1);
    if found != DATA_PAGE_MAIN {
        return Err(ClaimCowError::WrongTable { page_no, found, expected: DATA_PAGE_MAIN });
    }
    let level = word(2);
    let key_dwords = word(3);
    let value_dwords = word(4);
    if key_dwords != 2 || (level == 0 && value_dwords != 2) {
        return Err(ClaimCowError::NotAClaimTable { page_no, key_dwords, value_dwords });
    }
    let free_dwords = word(6) as usize;
    let usable = data.len() / 4 - HEADER_DWORDS;
    if free_dwords > usable {
        return Err(ClaimCowError::FreeSpaceOutOfRange { page_no, free_dwords: free_dwords as u32, usable_dwords: usable as u32 });
    }
    let stride = if level == 0 { ENTRY_DWORDS } else { CHILD_ENTRY_DWORDS };
    let used = usable - free_dwords;
    if !used.is_multiple_of(stride) {
        return Err(ClaimCowError::NotEntryAligned { page_no, used_dwords: used, stride });
    }
    let mut slots = Vec::with_capacity(used / stride);
    for slot in 0..used / stride {
        let at = HEADER_DWORDS + slot * stride;
        let (w0, w1) = (word(at), word(at + 1));
        let sentinel = w0 == SENTINEL_WORD && w1 == SENTINEL_WORD;
        let key = (!sentinel).then(|| RefNo::new(w0, w1));
        let (value, child) = if level == 0 {
            ([word(at + 2), word(at + 3)], None)
        } else {
            ([0, 0], Some(word(at + 2)))
        };
        slots.push(Slot { key, value, child });
    }
    Ok(Node { level, slots })
}

/// The slot to descend through for `refno`: the greatest separator at or below
/// it, the sentinel's (or first) child below every separator.
fn route(slots: &[Slot], refno: RefNo) -> Option<usize> {
    let mut best: Option<(usize, RefNo)> = None;
    let mut leftmost: Option<usize> = None;
    for (index, slot) in slots.iter().enumerate() {
        if slot.child.is_none() {
            continue;
        }
        if leftmost.is_none() {
            leftmost = Some(index);
        }
        let Some(separator) = slot.key else {
            continue;
        };
        if cmp_refno(separator, refno) == std::cmp::Ordering::Greater {
            continue;
        }
        if best.is_none_or(|(_, best_key)| cmp_refno(separator, best_key) == std::cmp::Ordering::Greater) {
            best = Some((index, separator));
        }
    }
    best.map(|(index, _)| index).or(leftmost)
}

struct Rewritten {
    first_page: u32,
    level: u32,
    extra: Vec<(RefNo, u32)>,
}

/// Rewrite the claim tree at `root` with `edits`.
pub fn rewrite_claim_table(
    root: PageId,
    page_words: usize,
    edits: &[ClaimEdit],
    read_page: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    alloc: &mut dyn FnMut() -> u32,
    page_limit: u32,
) -> Result<ClaimRewrite, ClaimCowError> {
    let mut cow = Cow {
        ext: root.ext,
        page_words,
        read_page,
        alloc,
        pages: Vec::new(),
        depth: 0,
        page_limit,
    };
    let rewritten = cow.rewrite_node(root.page, edits)?;
    let root = if rewritten.extra.is_empty() {
        PageId::new(cow.ext, rewritten.first_page)
    } else {
        let mut slots = vec![Slot {
            key: None,
            value: [0, 0],
            child: Some(rewritten.first_page),
        }];
        for (key, page) in &rewritten.extra {
            slots.push(Slot {
                key: Some(*key),
                value: [0, 0],
                child: Some(*page),
            });
        }
        let new_root = (cow.alloc)();
        let bytes = write_claim_page(rewritten.level + 1, &slots, page_words, cow.ext);
        cow.pages.push((new_root, bytes));
        PageId::new(cow.ext, new_root)
    };
    Ok(ClaimRewrite { root, pages: cow.pages })
}

struct Cow<'a> {
    ext: u16,
    page_words: usize,
    read_page: &'a mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    alloc: &'a mut dyn FnMut() -> u32,
    pages: Vec<(u32, Vec<u8>)>,
    depth: u32,
    page_limit: u32,
}

impl Cow<'_> {
    fn rewrite_node(&mut self, page_no: u32, edits: &[ClaimEdit]) -> Result<Rewritten, ClaimCowError> {
        self.depth += 1;
        if self.depth > self.page_limit {
            return Err(ClaimCowError::TooDeep { limit: self.page_limit });
        }
        let data = (self.read_page)(page_no).map_err(|detail| ClaimCowError::Unreadable { page_no, detail })?;
        let node = decode(page_no, &data)?;
        let level = node.level;
        let slots = if node.level == 0 {
            rewrite_leaf(node.slots, edits)
        } else {
            self.rewrite_internal(node.slots, edits)?
        };
        self.depth -= 1;

        let max_slots = (self.page_words - HEADER_DWORDS) / if level == 0 { ENTRY_DWORDS } else { CHILD_ENTRY_DWORDS };
        let chunks = split_slots(slots, max_slots);
        let mut pages = chunks.into_iter().map(|chunk| {
            let page = (self.alloc)();
            let min_key = chunk.first().and_then(|slot| slot.key);
            let bytes = write_claim_page(level, &chunk, self.page_words, self.ext);
            self.pages.push((page, bytes));
            (min_key, page)
        });
        let (_, first_page) = pages.next().expect("a node always emits at least one page");
        let extra: Vec<(RefNo, u32)> = pages
            .map(|(min_key, page)| (min_key.unwrap_or(RefNo::new(0, 0)), page))
            .collect();
        Ok(Rewritten { first_page, level, extra })
    }

    fn rewrite_internal(&mut self, slots: Vec<Slot>, edits: &[ClaimEdit]) -> Result<Vec<Slot>, ClaimCowError> {
        let mut by_slot: std::collections::BTreeMap<usize, Vec<ClaimEdit>> = std::collections::BTreeMap::new();
        for edit in edits {
            let slot = route(&slots, edit.refno).ok_or(ClaimCowError::ChildNotFound { refno: edit.refno })?;
            by_slot.entry(slot).or_default().push(*edit);
        }
        let mut rebuilt = Vec::with_capacity(slots.len());
        for (index, slot) in slots.iter().enumerate() {
            let (Some(child_edits), Some(child)) = (by_slot.get(&index), slot.child) else {
                rebuilt.push(*slot);
                continue;
            };
            let rewritten = self.rewrite_node(child, child_edits)?;
            rebuilt.push(Slot {
                key: slot.key,
                value: [0, 0],
                child: Some(rewritten.first_page),
            });
            for (key, page) in rewritten.extra {
                rebuilt.push(Slot { key: Some(key), value: [0, 0], child: Some(page) });
            }
        }
        Ok(rebuilt)
    }
}

/// Apply the edits to a leaf's keys, keeping them in refno order. A `Set` of a
/// key already present updates its value; a `Remove` of a key that is not there
/// does nothing.
fn rewrite_leaf(slots: Vec<Slot>, edits: &[ClaimEdit]) -> Vec<Slot> {
    let mut entries: Vec<(RefNo, [u32; 2])> = slots.into_iter().filter_map(|slot| slot.key.map(|key| (key, slot.value))).collect();
    for edit in edits {
        let found = entries.binary_search_by(|(key, _)| cmp_refno(*key, edit.refno));
        match (edit.kind, found) {
            (ClaimEditKind::Set { owner, session }, Ok(at)) => entries[at].1 = [owner, session],
            (ClaimEditKind::Set { owner, session }, Err(at)) => entries.insert(at, (edit.refno, [owner, session])),
            // Bump an existing claim's session, keep its owner; absent → skip.
            (ClaimEditKind::Refresh { session }, Ok(at)) => entries[at].1[1] = session,
            (ClaimEditKind::Refresh { .. }, Err(_)) => {}
            (ClaimEditKind::Remove, Ok(at)) => {
                entries.remove(at);
            }
            (ClaimEditKind::Remove, Err(_)) => {}
        }
    }
    entries
        .into_iter()
        .map(|(key, value)| Slot { key: Some(key), value, child: None })
        .collect()
}

fn split_slots(slots: Vec<Slot>, max_slots: usize) -> Vec<Vec<Slot>> {
    if slots.len() <= max_slots {
        return vec![slots];
    }
    let parts = slots.len().div_ceil(max_slots);
    let base = slots.len() / parts;
    let remainder = slots.len() % parts;
    let mut chunks = Vec::with_capacity(parts);
    let mut rest = slots.as_slice();
    for part in 0..parts {
        let take = base + usize::from(part < remainder);
        let (chunk, tail) = rest.split_at(take);
        chunks.push(chunk.to_vec());
        rest = tail;
    }
    chunks
}

fn write_claim_page(level: u32, slots: &[Slot], page_words: usize, ext: u16) -> Vec<u8> {
    let stride = if level == 0 { ENTRY_DWORDS } else { CHILD_ENTRY_DWORDS };
    let mut bytes = vec![0u8; page_words * 4];
    let mut put = |word: usize, value: u32| {
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_be_bytes());
    };
    put(0, TABLE_PAGE_TYPE);
    put(1, DATA_PAGE_MAIN);
    put(2, level);
    put(3, 2);
    put(4, 2);
    put(5, 0);
    put(6, (page_words - HEADER_DWORDS - slots.len() * stride) as u32);
    let mut at = HEADER_DWORDS;
    for slot in slots {
        match slot.key {
            Some(refno) => {
                put(at, refno.word0);
                put(at + 1, refno.word1);
            }
            None => {
                put(at, SENTINEL_WORD);
                put(at + 1, SENTINEL_WORD);
            }
        }
        if level == 0 {
            put(at + 2, slot.value[0]);
            put(at + 3, slot.value[1]);
        } else {
            put(at + 2, slot.child.unwrap_or(0));
            put(at + 3, u32::from(ext));
        }
        at += stride;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::claim::ClaimTable;
    use crate::page::cache::PageCache;
    use crate::page::io::PageReader;
    use std::collections::HashMap;
    use std::io::Write;

    /// A leaf holds (32 - 7) / 4 = 6 entries, an internal page 6 children too.
    const PAGE_WORDS: usize = 32;

    struct Store {
        pages: HashMap<u32, Vec<u8>>,
        next: u32,
    }

    impl Store {
        fn new() -> Self {
            Self { pages: HashMap::new(), next: 1 }
        }

        fn leaf(&mut self, entries: &[(RefNo, u32, u32)]) -> u32 {
            let slots: Vec<Slot> = entries
                .iter()
                .map(|(key, owner, session)| Slot { key: Some(*key), value: [*owner, *session], child: None })
                .collect();
            let page = self.next;
            self.next += 1;
            self.pages.insert(page, write_claim_page(0, &slots, PAGE_WORDS, PageId::PRIMARY));
            page
        }

        fn internal(&mut self, level: u32, children: &[(Option<RefNo>, u32)]) -> u32 {
            let slots: Vec<Slot> = children
                .iter()
                .map(|(key, child)| Slot { key: *key, value: [0, 0], child: Some(*child) })
                .collect();
            let page = self.next;
            self.next += 1;
            self.pages.insert(page, write_claim_page(level, &slots, PAGE_WORDS, PageId::PRIMARY));
            page
        }

        fn rewrite(&mut self, root: u32, edits: &[ClaimEdit]) -> ClaimRewrite {
            let snapshot = self.pages.clone();
            let mut next = self.next;
            let result = {
                let mut read = |page_no: u32| snapshot.get(&page_no).cloned().ok_or_else(|| format!("no page {page_no}"));
                let mut alloc = || {
                    let page = next;
                    next += 1;
                    page
                };
                rewrite_claim_table(PageId::primary(root), PAGE_WORDS, edits, &mut read, &mut alloc, 1000).expect("rewrite")
            };
            self.next = next;
            for (page, bytes) in &result.pages {
                self.pages.insert(*page, bytes.clone());
            }
            result
        }

        fn read_back(&self, root: u32) -> Vec<(RefNo, u32, u32)> {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("ams1000_0001");
            let mut file = std::fs::File::create(&path).unwrap();
            let mut page0 = vec![0u8; PAGE_WORDS * 4];
            page0[0x34..0x38].copy_from_slice(&(PAGE_WORDS as u32).to_be_bytes());
            file.write_all(&page0).unwrap();
            for page in 1..self.next {
                let bytes = self.pages.get(&page).cloned().unwrap_or_else(|| vec![0u8; PAGE_WORDS * 4]);
                file.write_all(&bytes).unwrap();
            }
            file.flush().unwrap();
            let mut cache = PageCache::new(PageReader::open(&path).unwrap(), 8);
            let mut table = ClaimTable::at_root(&mut cache, PageId::primary(root));
            table
                .iter()
                .map(|entry| {
                    let entry = entry.expect("the reader walks what the writer wrote");
                    (entry.refno, entry.owner, entry.session)
                })
                .collect()
        }
    }

    fn refno(word1: u32) -> RefNo {
        RefNo::new(24_384, word1)
    }

    fn set(word1: u32, session: u32) -> ClaimEdit {
        ClaimEdit { refno: refno(word1), kind: ClaimEditKind::Set { owner: 0, session } }
    }

    fn remove(word1: u32) -> ClaimEdit {
        ClaimEdit { refno: refno(word1), kind: ClaimEditKind::Remove }
    }

    #[test]
    fn a_set_inserts_in_key_order_and_updates_in_place() {
        let mut store = Store::new();
        let leaf = store.leaf(&[(refno(10), 0, 5), (refno(30), 0, 7)]);

        // Insert 20, bump 10's session to 99.
        let result = store.rewrite(leaf, &[set(20, 42), set(10, 99)]);
        assert_eq!(
            store.read_back(result.root.page),
            vec![(refno(10), 0, 99), (refno(20), 0, 42), (refno(30), 0, 7)]
        );
    }

    #[test]
    fn a_remove_takes_the_key_out_and_a_missing_remove_is_a_no_op() {
        let mut store = Store::new();
        let leaf = store.leaf(&[(refno(10), 0, 5), (refno(20), 0, 6), (refno(30), 0, 7)]);
        let result = store.rewrite(leaf, &[remove(20), remove(999)]);
        assert_eq!(store.read_back(result.root.page), vec![(refno(10), 0, 5), (refno(30), 0, 7)]);
    }

    #[test]
    fn an_untouched_subtree_keeps_its_page_and_a_split_grows_a_root() {
        let mut store = Store::new();
        let left = store.leaf(&[(refno(10), 0, 1), (refno(20), 0, 1)]);
        let right = store.leaf(&[(refno(40), 0, 1), (refno(50), 0, 1)]);
        let root = store.internal(1, &[(None, left), (Some(refno(40)), right)]);

        let result = store.rewrite(root, &[set(30, 9)]);
        let new_root = decode(result.root.page, &store.pages[&result.root.page]).unwrap();
        assert_eq!(new_root.slots[1].child, Some(right), "the right subtree had no edit and keeps its page");
        assert_eq!(
            store.read_back(result.root.page),
            vec![(refno(10), 0, 1), (refno(20), 0, 1), (refno(30), 0, 9), (refno(40), 0, 1), (refno(50), 0, 1)]
        );

        // Overflow the left leaf so it splits and the root grows.
        let mut store = Store::new();
        let entries: Vec<(RefNo, u32, u32)> = (0..6).map(|i| (refno(i * 10), 0, 1)).collect();
        let leaf = store.leaf(&entries);
        let result = store.rewrite(leaf, &[set(5, 2)]);
        let root = decode(result.root.page, &store.pages[&result.root.page]).unwrap();
        assert_eq!(root.level, 1, "a new root above the pieces");
        assert!(root.slots[0].key.is_none(), "opening with the sentinel");
        let mut expected: Vec<(RefNo, u32, u32)> = entries;
        expected.push((refno(5), 0, 2));
        expected.sort_by(|a, b| cmp_refno(a.0, b.0));
        assert_eq!(store.read_back(result.root.page), expected);
    }

    #[test]
    fn a_page_of_the_index_tree_is_refused() {
        // A page carrying the RefNo index's name hash, not the claim tree's.
        let mut bytes = write_claim_page(0, &[Slot { key: Some(refno(1)), value: [0, 1], child: None }], PAGE_WORDS, PageId::PRIMARY);
        bytes[4..8].copy_from_slice(&crate::meta::constants::DATA_PAGE_INDEX.to_be_bytes());
        let snapshot: HashMap<u32, Vec<u8>> = HashMap::from([(1u32, bytes)]);
        let mut read = |page_no: u32| snapshot.get(&page_no).cloned().ok_or_else(|| format!("no page {page_no}"));
        let mut next = 2u32;
        let mut alloc = || {
            let page = next;
            next += 1;
            page
        };
        let error = rewrite_claim_table(PageId::primary(1), PAGE_WORDS, &[set(2, 2)], &mut read, &mut alloc, 1000).unwrap_err();
        assert!(matches!(error, ClaimCowError::WrongTable { .. }), "{error}");
    }
}
