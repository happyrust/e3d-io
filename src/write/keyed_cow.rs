//! Copy-on-write edits to a **keyed table** — one of the valueless, 4-dword-key
//! B+trees the session page's table directory names after an attribute: the
//! reverse-reference tables (`SPRE`, `CATR`, `LSTU`, `HSTU`, `ISPE`, …, read by
//! [`crate::index::reference`]) and `TYPEX`.
//!
//! Both kinds are what `core.dll`'s table-attribute writer `sub_5AB91B0`
//! maintains as a side effect of setting an attribute (`docs/plans/
//! 2026-09-20-dabacon-write-plan.md` §7, 2026-09-20 反向表):
//!
//! * a **type-16** attribute (a single reference: `SPRE`, `CATR`, `ISPE`, …)
//!   files the key `[target.0, target.1, self.0, self.1]` in the table named
//!   by the attribute's hash — `3CTE(hash, key, 4, …, op 1)` — after removing
//!   the key the old value made, `3CTE(…, op 0)`; a null reference removes and
//!   files nothing;
//! * a **type-14** keyed attribute files `[value words…, self.0, self.1, k]`
//!   for its `k`-th element. `TYPEX` is the one-word case: every noun whose
//!   template lists `TYPEX` carries `[1, noun]` in its explicit stream and the
//!   key `[noun, self.0, self.1, 0]` in the `TYPEX` table.
//!
//! On both, an insert of a key already present (529) and a delete of a key
//! that is not there (534) are tolerated, and a table the directory does not
//! yet list is created on the spot (523 → `sub_5AFF6D0`: a fresh leaf whose
//! header states key width 4 and value width 0). This module keeps those
//! rules: [`KeyedEditKind::Insert`] and [`KeyedEditKind::Delete`] are
//! idempotent, and [`rewrite_keyed_table`] with no root builds the table.
//!
//! The page shape is the one [`crate::index::reference`] decodes: page type 5,
//! the name hash in word 1, level in word 2, key width 4, leaf value width 0,
//! `free_dwords` in word 6; a leaf entry is the four key words, an internal
//! entry the four key words then `(child_pgno, child_extno)`. Keys order
//! unsigned word by word, the internal separator is the least key of the
//! subtree behind it, and a key routes to the greatest separator at or below
//! it — the same descent the reader's `route` takes. The copy-on-write itself
//! is [`super::index_cow`]'s: copy the root-to-leaf path, give the copies
//! fresh page numbers, leave every other subtree where it is, split an
//! overflowing page at the median and grow a root above a split root.

use crate::page::PageId;

/// Words of a keyed-table key: target then referrer, or value then referrer
/// then element index.
pub const KEYED_KEY_DWORDS: usize = 4;

const HEADER_DWORDS: usize = 7;
const TABLE_PAGE_TYPE: u32 = 5;
const SENTINEL_WORD: u32 = 0x8000_0001;
const LEAF_STRIDE: usize = KEYED_KEY_DWORDS;
const CHILD_DWORDS: usize = 2;
const INTERNAL_STRIDE: usize = KEYED_KEY_DWORDS + CHILD_DWORDS;

/// One key of a keyed table.
pub type KeyedKey = [u32; KEYED_KEY_DWORDS];

/// What a [`KeyedEdit`] does to its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyedEditKind {
    /// File the key; a key already present stays as it is (core's 529).
    Insert,
    /// Remove the key; a key not present is nothing to do (core's 534).
    Delete,
}

/// One change to a keyed table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyedEdit {
    pub key: KeyedKey,
    pub kind: KeyedEditKind,
}

/// The new table: its root and every page the rewrite produced.
#[derive(Debug)]
pub struct KeyedRewrite {
    pub root: PageId,
    /// `(page_no, bytes)` for each new page, in no particular order.
    pub pages: Vec<(u32, Vec<u8>)>,
}

#[derive(Debug, thiserror::Error)]
pub enum KeyedCowError {
    #[error("table {name_hash} page {page_no} could not be read: {detail}")]
    Unreadable { name_hash: u32, page_no: u32, detail: String },
    #[error("table {name_hash} page {page_no} is page type {found}, expected {TABLE_PAGE_TYPE}")]
    NotATablePage { name_hash: u32, page_no: u32, found: u32 },
    #[error("page {page_no} belongs to table {found}, but was reached as a page of table {expected}")]
    WrongTable { page_no: u32, found: u32, expected: u32 },
    #[error(
        "table {name_hash} page {page_no} states a {key_dwords}-dword key and a {value_dwords}-dword \
         leaf value; a keyed table states {KEYED_KEY_DWORDS} and 0"
    )]
    NotAKeyedTable {
        name_hash: u32,
        page_no: u32,
        key_dwords: u32,
        value_dwords: u32,
    },
    #[error("table {name_hash} page {page_no}: free_dwords {free_dwords} exceeds the {usable_dwords} usable")]
    FreeSpaceOutOfRange {
        name_hash: u32,
        page_no: u32,
        free_dwords: u32,
        usable_dwords: u32,
    },
    #[error("table {name_hash} page {page_no}: {used_dwords} dwords in use is not a whole number of {stride}-dword entries")]
    NotEntryAligned {
        name_hash: u32,
        page_no: u32,
        used_dwords: usize,
        stride: usize,
    },
    #[error("table {name_hash}: an internal page has no child to route {key:?} to")]
    ChildNotFound { name_hash: u32, key: KeyedKey },
    #[error("table {name_hash} is deeper than the {limit} pages the file holds")]
    TooDeep { name_hash: u32, limit: u32 },
}

/// One slot of a decoded page: the key (`None` on the sentinel) and, on an
/// internal page, the child behind it.
#[derive(Debug, Clone, Copy)]
struct Slot {
    key: Option<KeyedKey>,
    child: Option<u32>,
}

/// A decoded page.
struct Node {
    level: u32,
    /// Header word 5, carried over verbatim (0 on every reference table and on
    /// `TYPEX`; a new table states 0).
    flags: u32,
    slots: Vec<Slot>,
}

fn decode_table_page(name_hash: u32, page_no: u32, data: &[u8]) -> Result<Node, KeyedCowError> {
    let word = |index: usize| -> Option<u32> {
        data.get(index * 4..index * 4 + 4)
            .map(|bytes| u32::from_be_bytes(bytes.try_into().expect("four bytes")))
    };
    let header = |index: usize| {
        word(index).ok_or_else(|| KeyedCowError::Unreadable {
            name_hash,
            page_no,
            detail: format!("page is {} bytes, shorter than its header", data.len()),
        })
    };
    let page_type = header(0)?;
    if page_type != TABLE_PAGE_TYPE {
        return Err(KeyedCowError::NotATablePage {
            name_hash,
            page_no,
            found: page_type,
        });
    }
    let found = header(1)?;
    if found != name_hash {
        return Err(KeyedCowError::WrongTable {
            page_no,
            found,
            expected: name_hash,
        });
    }
    let level = header(2)?;
    let key_dwords = header(3)?;
    let value_dwords = header(4)?;
    if key_dwords != KEYED_KEY_DWORDS as u32 || (level == 0 && value_dwords != 0) {
        return Err(KeyedCowError::NotAKeyedTable {
            name_hash,
            page_no,
            key_dwords,
            value_dwords,
        });
    }
    let flags = header(5)?;
    let free_dwords = header(6)?;
    let usable_dwords = data.len() / 4 - HEADER_DWORDS;
    if free_dwords as usize > usable_dwords {
        return Err(KeyedCowError::FreeSpaceOutOfRange {
            name_hash,
            page_no,
            free_dwords,
            usable_dwords: usable_dwords as u32,
        });
    }
    let stride = if level == 0 { LEAF_STRIDE } else { INTERNAL_STRIDE };
    let used_dwords = usable_dwords - free_dwords as usize;
    if !used_dwords.is_multiple_of(stride) {
        return Err(KeyedCowError::NotEntryAligned {
            name_hash,
            page_no,
            used_dwords,
            stride,
        });
    }
    let mut slots = Vec::with_capacity(used_dwords / stride);
    for slot in 0..used_dwords / stride {
        let at = HEADER_DWORDS + slot * stride;
        let words: KeyedKey = [
            word(at).unwrap_or(0),
            word(at + 1).unwrap_or(0),
            word(at + 2).unwrap_or(0),
            word(at + 3).unwrap_or(0),
        ];
        let sentinel = words.iter().all(|&w| w == SENTINEL_WORD);
        let child = (level != 0).then(|| word(at + KEYED_KEY_DWORDS).unwrap_or(0));
        slots.push(Slot {
            key: (!sentinel).then_some(words),
            child,
        });
    }
    Ok(Node { level, flags, slots })
}

/// The slot to descend through for `key`: the greatest separator at or below
/// it; below every separator, the sentinel's (or the first) child.
fn route_slot(slots: &[Slot], key: KeyedKey) -> Option<usize> {
    let mut best: Option<(usize, KeyedKey)> = None;
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
        if separator > key {
            continue;
        }
        if best.is_none_or(|(_, best_key)| separator > best_key) {
            best = Some((index, separator));
        }
    }
    best.map(|(index, _)| index).or(leftmost)
}

struct Rewritten {
    first_page: u32,
    level: u32,
    /// Further pieces a split made, each with the least key of its subtree.
    extra: Vec<(KeyedKey, u32)>,
}

/// Rewrite the table at `root` with `edits`, or build it when there is no
/// root yet. Old pages are read through `read_page`, new page numbers drawn
/// from `alloc`.
pub fn rewrite_keyed_table(
    root: Option<PageId>,
    name_hash: u32,
    page_words: usize,
    edits: &[KeyedEdit],
    read_page: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    alloc: &mut dyn FnMut() -> u32,
    page_limit: u32,
) -> Result<KeyedRewrite, KeyedCowError> {
    let ext = root.map_or(PageId::PRIMARY, |root| root.ext);
    let mut cow = Cow {
        name_hash,
        ext,
        page_words,
        read_page,
        alloc,
        pages: Vec::new(),
        depth: 0,
        page_limit,
    };
    let rewritten = match root {
        Some(root) => cow.rewrite_node(root.page, edits)?,
        None => {
            // A table that does not exist yet: an empty leaf, then the edits.
            let node = Node {
                level: 0,
                flags: 0,
                slots: Vec::new(),
            };
            cow.rebuild(node, edits)?
        }
    };
    let root = if rewritten.extra.is_empty() {
        PageId::new(cow.ext, rewritten.first_page)
    } else {
        let mut slots = vec![Slot {
            key: None,
            child: Some(rewritten.first_page),
        }];
        for (key, page) in &rewritten.extra {
            slots.push(Slot {
                key: Some(*key),
                child: Some(*page),
            });
        }
        let new_root = (cow.alloc)();
        let bytes = write_table_page(name_hash, rewritten.level + 1, 0, &slots, page_words, cow.ext);
        cow.pages.push((new_root, bytes));
        PageId::new(cow.ext, new_root)
    };
    Ok(KeyedRewrite {
        root,
        pages: cow.pages,
    })
}

struct Cow<'a> {
    name_hash: u32,
    ext: u16,
    page_words: usize,
    read_page: &'a mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    alloc: &'a mut dyn FnMut() -> u32,
    pages: Vec<(u32, Vec<u8>)>,
    depth: u32,
    page_limit: u32,
}

impl Cow<'_> {
    fn rewrite_node(&mut self, page_no: u32, edits: &[KeyedEdit]) -> Result<Rewritten, KeyedCowError> {
        self.depth += 1;
        if self.depth > self.page_limit {
            return Err(KeyedCowError::TooDeep {
                name_hash: self.name_hash,
                limit: self.page_limit,
            });
        }
        let data = (self.read_page)(page_no).map_err(|detail| KeyedCowError::Unreadable {
            name_hash: self.name_hash,
            page_no,
            detail,
        })?;
        let node = decode_table_page(self.name_hash, page_no, &data)?;
        let rewritten = self.rebuild(node, edits)?;
        self.depth -= 1;
        Ok(rewritten)
    }

    /// Apply `edits` to a decoded node and emit it as one page, or several
    /// when it overflows.
    fn rebuild(&mut self, node: Node, edits: &[KeyedEdit]) -> Result<Rewritten, KeyedCowError> {
        let level = node.level;
        let flags = node.flags;
        let slots = if node.level == 0 {
            rewrite_leaf(node.slots, edits)
        } else {
            self.rewrite_internal(node.slots, edits)?
        };
        let max_slots = (self.page_words - HEADER_DWORDS) / if level == 0 { LEAF_STRIDE } else { INTERNAL_STRIDE };
        let chunks = split_slots(slots, max_slots);
        let mut pages = chunks.into_iter().map(|chunk| {
            let page = (self.alloc)();
            // The least key of the chunk: its first slot's key. Only the
            // first chunk can open with the sentinel, and that chunk keeps
            // its parent's separator, so `None` never reaches `extra`.
            let min_key = chunk.first().and_then(|slot| slot.key);
            let bytes = write_table_page(self.name_hash, level, flags, &chunk, self.page_words, self.ext);
            self.pages.push((page, bytes));
            (min_key, page)
        });
        let (_, first_page) = pages.next().expect("a node always emits at least one page");
        let extra: Vec<(KeyedKey, u32)> = pages
            .map(|(min_key, page)| (min_key.unwrap_or([0; KEYED_KEY_DWORDS]), page))
            .collect();
        Ok(Rewritten {
            first_page,
            level,
            extra,
        })
    }

    fn rewrite_internal(&mut self, slots: Vec<Slot>, edits: &[KeyedEdit]) -> Result<Vec<Slot>, KeyedCowError> {
        let mut by_slot: std::collections::BTreeMap<usize, Vec<KeyedEdit>> = std::collections::BTreeMap::new();
        for edit in edits {
            let slot = route_slot(&slots, edit.key).ok_or(KeyedCowError::ChildNotFound {
                name_hash: self.name_hash,
                key: edit.key,
            })?;
            by_slot.entry(slot).or_default().push(*edit);
        }
        let mut rebuilt = Vec::with_capacity(slots.len());
        for (index, slot) in slots.iter().enumerate() {
            let (Some(child_edits), Some(child)) = (by_slot.get(&index), slot.child) else {
                rebuilt.push(*slot);
                continue;
            };
            let rewritten = self.rewrite_node(child, child_edits)?;
            // The separator is unchanged — an insert never lowers a subtree's
            // least key, and a delete leaves the separator where it was (a
            // lookup for the gone key lands in the same leaf and finds
            // nothing) — so only the page number moves.
            rebuilt.push(Slot {
                key: slot.key,
                child: Some(rewritten.first_page),
            });
            for (key, page) in rewritten.extra {
                rebuilt.push(Slot {
                    key: Some(key),
                    child: Some(page),
                });
            }
        }
        Ok(rebuilt)
    }
}

/// Apply the edits to a leaf's keys, keeping them in unsigned word order.
fn rewrite_leaf(slots: Vec<Slot>, edits: &[KeyedEdit]) -> Vec<Slot> {
    let mut keys: Vec<KeyedKey> = slots.into_iter().filter_map(|slot| slot.key).collect();
    for edit in edits {
        match (edit.kind, keys.binary_search(&edit.key)) {
            (KeyedEditKind::Insert, Err(at)) => keys.insert(at, edit.key),
            (KeyedEditKind::Delete, Ok(at)) => {
                keys.remove(at);
            }
            // Already there / already gone: core tolerates 529 and 534 here.
            (KeyedEditKind::Insert, Ok(_)) | (KeyedEditKind::Delete, Err(_)) => {}
        }
    }
    keys.into_iter()
        .map(|key| Slot {
            key: Some(key),
            child: None,
        })
        .collect()
}

/// One chunk unless the slots overrun a page, then as few equal chunks as
/// fit (the median split of §5 Q5).
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

/// Serialise a table page: the seven-word header, then the slots, the rest
/// zero and `free_dwords` set to match.
fn write_table_page(name_hash: u32, level: u32, flags: u32, slots: &[Slot], page_words: usize, ext: u16) -> Vec<u8> {
    let stride = if level == 0 { LEAF_STRIDE } else { INTERNAL_STRIDE };
    let mut bytes = vec![0u8; page_words * 4];
    let mut put = |word: usize, value: u32| {
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_be_bytes());
    };
    put(0, TABLE_PAGE_TYPE);
    put(1, name_hash);
    put(2, level);
    put(3, KEYED_KEY_DWORDS as u32);
    put(4, 0);
    put(5, flags);
    put(6, (page_words - HEADER_DWORDS - slots.len() * stride) as u32);
    let mut at = HEADER_DWORDS;
    for slot in slots {
        let key = slot.key.unwrap_or([SENTINEL_WORD; KEYED_KEY_DWORDS]);
        for (offset, word) in key.iter().enumerate() {
            put(at + offset, *word);
        }
        if level != 0 {
            put(at + KEYED_KEY_DWORDS, slot.child.unwrap_or(0));
            put(at + KEYED_KEY_DWORDS + 1, u32::from(ext));
        }
        at += stride;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::reference::ReferenceTable;
    use crate::page::cache::PageCache;
    use crate::page::io::PageReader;
    use std::collections::HashMap;
    use std::io::Write;

    /// Small pages: a leaf holds (32 - 7) / 4 = 6 keys, an internal page
    /// (32 - 7) / 6 = 4 children.
    const PAGE_WORDS: usize = 32;
    const SPRE: u32 = 643_429;

    struct Store {
        pages: HashMap<u32, Vec<u8>>,
        next: u32,
    }

    impl Store {
        fn new() -> Self {
            Self {
                pages: HashMap::new(),
                // Page 0 is the descriptor when the store is written to a file.
                next: 1,
            }
        }

        fn leaf(&mut self, keys: &[KeyedKey]) -> u32 {
            let slots: Vec<Slot> = keys.iter().map(|k| Slot { key: Some(*k), child: None }).collect();
            let page = self.next;
            self.next += 1;
            self.pages
                .insert(page, write_table_page(SPRE, 0, 0, &slots, PAGE_WORDS, PageId::PRIMARY));
            page
        }

        fn internal(&mut self, level: u32, children: &[(Option<KeyedKey>, u32)]) -> u32 {
            let slots: Vec<Slot> = children
                .iter()
                .map(|(key, child)| Slot {
                    key: *key,
                    child: Some(*child),
                })
                .collect();
            let page = self.next;
            self.next += 1;
            self.pages
                .insert(page, write_table_page(SPRE, level, 0, &slots, PAGE_WORDS, PageId::PRIMARY));
            page
        }

        fn rewrite(&mut self, root: Option<u32>, edits: &[KeyedEdit]) -> KeyedRewrite {
            let snapshot = self.pages.clone();
            let mut next = self.next;
            let result = {
                let mut read = |page_no: u32| {
                    snapshot
                        .get(&page_no)
                        .cloned()
                        .ok_or_else(|| format!("no page {page_no}"))
                };
                let mut alloc = || {
                    let page = next;
                    next += 1;
                    page
                };
                rewrite_keyed_table(
                    root.map(PageId::primary),
                    SPRE,
                    PAGE_WORDS,
                    edits,
                    &mut read,
                    &mut alloc,
                    1000,
                )
                .expect("rewrite")
            };
            self.next = next;
            for (page, bytes) in &result.pages {
                self.pages.insert(*page, bytes.clone());
            }
            result
        }

        /// Write the store to a file and read the table at `root` back through
        /// the reader every reverse-reference lookup uses.
        fn read_back(&self, root: u32) -> Vec<KeyedKey> {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("ams1000_0001");
            let mut file = std::fs::File::create(&path).unwrap();
            let mut page0 = vec![0u8; PAGE_WORDS * 4];
            page0[0x34..0x38].copy_from_slice(&(PAGE_WORDS as u32).to_be_bytes());
            file.write_all(&page0).unwrap();
            for page in 1..self.next {
                let bytes = self
                    .pages
                    .get(&page)
                    .cloned()
                    .unwrap_or_else(|| vec![0u8; PAGE_WORDS * 4]);
                file.write_all(&bytes).unwrap();
            }
            file.flush().unwrap();
            let mut cache = PageCache::new(PageReader::open(&path).unwrap(), 8);
            let mut table = ReferenceTable::at_root(&mut cache, PageId::primary(root), SPRE);
            table
                .iter()
                .map(|entry| {
                    let entry = entry.expect("the reader walks what the writer wrote");
                    [entry.target.word0, entry.target.word1, entry.referrer.word0, entry.referrer.word1]
                })
                .collect()
        }
    }

    fn key(target: u32, referrer: u32) -> KeyedKey {
        [13_244, target, 24_384, referrer]
    }

    fn insert(k: KeyedKey) -> KeyedEdit {
        KeyedEdit {
            key: k,
            kind: KeyedEditKind::Insert,
        }
    }

    fn delete(k: KeyedKey) -> KeyedEdit {
        KeyedEdit {
            key: k,
            kind: KeyedEditKind::Delete,
        }
    }

    #[test]
    fn an_insert_lands_in_key_order_and_an_untouched_subtree_keeps_its_page() {
        let mut store = Store::new();
        let left = store.leaf(&[key(10, 1), key(10, 2)]);
        let right = store.leaf(&[key(40, 1), key(50, 1)]);
        let root = store.internal(1, &[(None, left), (Some(key(40, 1)), right)]);

        let result = store.rewrite(Some(root), &[insert(key(30, 1))]);
        assert_eq!(
            store.read_back(result.root.page),
            vec![key(10, 1), key(10, 2), key(30, 1), key(40, 1), key(50, 1)]
        );
        let new_root = decode_table_page(SPRE, result.root.page, &store.pages[&result.root.page]).unwrap();
        assert_eq!(new_root.slots[1].child, Some(right), "the right subtree had no edit and keeps its page");
        assert_ne!(new_root.slots[0].child, Some(left), "the left leaf was copied");
    }

    #[test]
    fn a_delete_removes_the_key_and_both_edits_are_idempotent() {
        let mut store = Store::new();
        let leaf = store.leaf(&[key(10, 1), key(20, 1), key(30, 1)]);

        let result = store.rewrite(Some(leaf), &[delete(key(20, 1))]);
        assert_eq!(store.read_back(result.root.page), vec![key(10, 1), key(30, 1)]);

        // Deleting a key that is not there and inserting one that is: core's
        // 534 and 529, tolerated — the table is unchanged in content.
        let result = store.rewrite(Some(result.root.page), &[delete(key(99, 9)), insert(key(10, 1))]);
        assert_eq!(store.read_back(result.root.page), vec![key(10, 1), key(30, 1)]);
    }

    #[test]
    fn a_missing_table_is_built_from_its_first_keys() {
        let mut store = Store::new();
        let result = store.rewrite(None, &[insert(key(5, 2)), insert(key(5, 1)), delete(key(7, 7))]);
        assert_eq!(result.pages.len(), 1, "one fresh leaf");
        assert_eq!(store.read_back(result.root.page), vec![key(5, 1), key(5, 2)]);
        let page = &store.pages[&result.root.page];
        let word = |i: usize| u32::from_be_bytes(page[i * 4..i * 4 + 4].try_into().unwrap());
        assert_eq!(
            (word(0), word(1), word(2), word(3), word(4), word(5), word(6)),
            (5, SPRE, 0, 4, 0, 0, (PAGE_WORDS - HEADER_DWORDS - 2 * LEAF_STRIDE) as u32),
            "the header sub_5AFF6D0 writes for a reference table"
        );
    }

    #[test]
    fn a_leaf_that_overflows_splits_and_the_root_grows() {
        let mut store = Store::new();
        let keys: Vec<KeyedKey> = (0..6).map(|i| key(10 * i, 1)).collect();
        let leaf = store.leaf(&keys);

        let result = store.rewrite(Some(leaf), &[insert(key(5, 1))]);
        let root = decode_table_page(SPRE, result.root.page, &store.pages[&result.root.page]).unwrap();
        assert_eq!(root.level, 1, "a new root above the two pieces");
        assert!(root.slots[0].key.is_none(), "the new root opens with the sentinel");
        let mut expected = keys.clone();
        expected.push(key(5, 1));
        expected.sort();
        assert_eq!(store.read_back(result.root.page), expected);
    }

    #[test]
    fn flags_are_carried_over_from_the_page_that_was_copied() {
        let mut store = Store::new();
        let slots = vec![Slot { key: Some(key(1, 1)), child: None }];
        let page = store.next;
        store.next += 1;
        store
            .pages
            .insert(page, write_table_page(SPRE, 0, 1, &slots, PAGE_WORDS, PageId::PRIMARY));
        let result = store.rewrite(Some(page), &[insert(key(2, 2))]);
        let copied = decode_table_page(SPRE, result.root.page, &store.pages[&result.root.page]).unwrap();
        assert_eq!(copied.flags, 1);
    }

    #[test]
    fn a_page_of_another_table_is_refused() {
        let mut store = Store::new();
        let slots = vec![Slot { key: Some(key(1, 1)), child: None }];
        let page = store.next;
        store.next += 1;
        store
            .pages
            .insert(page, write_table_page(900_345, 0, 0, &slots, PAGE_WORDS, PageId::PRIMARY));
        let snapshot = store.pages.clone();
        let mut read = |page_no: u32| snapshot.get(&page_no).cloned().ok_or_else(|| format!("no page {page_no}"));
        let mut next = store.next;
        let mut alloc = || {
            let page = next;
            next += 1;
            page
        };
        let error = rewrite_keyed_table(
            Some(PageId::primary(page)),
            SPRE,
            PAGE_WORDS,
            &[insert(key(2, 2))],
            &mut read,
            &mut alloc,
            1000,
        )
        .unwrap_err();
        assert!(matches!(error, KeyedCowError::WrongTable { found: 900_345, expected: SPRE, .. }), "{error}");
    }
}
