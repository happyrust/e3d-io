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
//! SPAMAP is the same tree with a seven-dword leaf value
//! ([`crate::index::spamap`]); [`rewrite_table`] takes the leaf value width
//! as `V` and carries each key's value with it, and [`rewrite_keyed_table`]
//! is its `V = 0` case.
//!
//! The page shape is the one [`crate::index::keyed`] decodes: page type 5,
//! the name hash in word 1, level in word 2, key width 4, leaf value width
//! `V` (stated on every page), header word 5, `free_dwords` in word 6; a leaf
//! entry is the four key words then the value, an internal entry the four key
//! words then `(child_pgno, child_extno)`. Keys order
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

/// What a [`TableEdit`] does to its key, in a table whose leaf value is `V`
/// dwords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableEditKind<const V: usize> {
    /// File the key with this value; a key already present keeps its own
    /// value (core's 529).
    Insert([u32; V]),
    /// Remove the key and its value; a key not present is nothing to do
    /// (core's 534).
    Delete,
}

/// One change to a table whose leaf value is `V` dwords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableEdit<const V: usize> {
    pub key: KeyedKey,
    pub kind: TableEditKind<V>,
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
         leaf value; this table has {KEYED_KEY_DWORDS} and {expected_value_dwords}"
    )]
    NotAKeyedTable {
        name_hash: u32,
        page_no: u32,
        key_dwords: u32,
        value_dwords: u32,
        expected_value_dwords: u32,
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

/// One slot of a decoded page: the key (`None` on the sentinel), on a leaf
/// its value, and on an internal page the child behind it.
#[derive(Debug, Clone, Copy)]
struct Slot<const V: usize> {
    key: Option<KeyedKey>,
    value: [u32; V],
    child: Option<u32>,
}

/// A decoded page.
struct Node<const V: usize> {
    level: u32,
    /// Header word 5, carried over verbatim (0 on every reference table and on
    /// `TYPEX`, 1 on SPAMAP; a page made from nothing states the table's own).
    flags: u32,
    slots: Vec<Slot<V>>,
}

const fn leaf_stride<const V: usize>() -> usize {
    KEYED_KEY_DWORDS + V
}

fn decode_table_page<const V: usize>(name_hash: u32, page_no: u32, data: &[u8]) -> Result<Node<V>, KeyedCowError> {
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
    if key_dwords != KEYED_KEY_DWORDS as u32 || (level == 0 && value_dwords != V as u32) {
        return Err(KeyedCowError::NotAKeyedTable {
            name_hash,
            page_no,
            key_dwords,
            value_dwords,
            expected_value_dwords: V as u32,
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
    let stride = if level == 0 { leaf_stride::<V>() } else { INTERNAL_STRIDE };
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
        let mut value = [0u32; V];
        if level == 0 {
            for (index, slot) in value.iter_mut().enumerate() {
                *slot = word(at + KEYED_KEY_DWORDS + index).unwrap_or(0);
            }
        }
        slots.push(Slot {
            key: (!sentinel).then_some(words),
            value,
            child,
        });
    }
    Ok(Node { level, flags, slots })
}

/// The slot to descend through for `key`: the greatest separator at or below
/// it; below every separator, the sentinel's (or the first) child.
fn route_slot<const V: usize>(slots: &[Slot<V>], key: KeyedKey) -> Option<usize> {
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

/// Rewrite the valueless table at `root` with `edits`, or build it when there
/// is no root yet: [`rewrite_table`] with `V = 0`, a page made from nothing
/// stating header word 5 as 0. Old pages are read through `read_page`, new
/// page numbers drawn from `alloc`.
pub fn rewrite_keyed_table(
    root: Option<PageId>,
    name_hash: u32,
    page_words: usize,
    edits: &[KeyedEdit],
    read_page: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    alloc: &mut dyn FnMut() -> u32,
    page_limit: u32,
) -> Result<KeyedRewrite, KeyedCowError> {
    let edits: Vec<TableEdit<0>> = edits
        .iter()
        .map(|edit| TableEdit {
            key: edit.key,
            kind: match edit.kind {
                KeyedEditKind::Insert => TableEditKind::Insert([]),
                KeyedEditKind::Delete => TableEditKind::Delete,
            },
        })
        .collect();
    rewrite_table::<0>(root, name_hash, 0, page_words, &edits, read_page, alloc, page_limit)
}

/// Rewrite the table at `root`, whose leaves carry a `V`-dword value, with
/// `edits`, or build it when there is no root yet. A page copied keeps its
/// header word 5; a page made from nothing — the first leaf of a new table, a
/// root grown above a split — states `new_page_flags`. Old pages are read
/// through `read_page`, new page numbers drawn from `alloc`.
#[allow(clippy::too_many_arguments)]
pub fn rewrite_table<const V: usize>(
    root: Option<PageId>,
    name_hash: u32,
    new_page_flags: u32,
    page_words: usize,
    edits: &[TableEdit<V>],
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
                flags: new_page_flags,
                slots: Vec::new(),
            };
            cow.rebuild(node, edits)?
        }
    };
    // A split root grows a root above its pieces, and that root splits in
    // turn while the pieces do not fit one internal page.
    let Rewritten { mut first_page, mut level, mut extra } = rewritten;
    while !extra.is_empty() {
        let mut slots = vec![Slot {
            key: None,
            value: [0; V],
            child: Some(first_page),
        }];
        slots.extend(extra.iter().map(|&(key, page)| Slot {
            key: Some(key),
            value: [0; V],
            child: Some(page),
        }));
        level += 1;
        let mut pieces = split_slots(slots, (page_words - HEADER_DWORDS) / INTERNAL_STRIDE).into_iter().map(|chunk| {
            let page = (cow.alloc)();
            let min_key = chunk.first().and_then(|slot| slot.key);
            let bytes = write_table_page(name_hash, level, new_page_flags, &chunk, page_words, cow.ext);
            cow.pages.push((page, bytes));
            (min_key, page)
        });
        first_page = pieces.next().expect("a root always emits at least one page").1;
        extra = pieces.map(|(min_key, page)| (min_key.unwrap_or([0; KEYED_KEY_DWORDS]), page)).collect();
    }
    Ok(KeyedRewrite {
        root: PageId::new(cow.ext, first_page),
        pages: cow.pages,
    })
}

/// A new table holding `entries`, its pages numbered from 1 in the primary
/// extent: [`rewrite_table`] with no root and no rules about what the
/// entries are, for the page-tree property tests.
#[doc(hidden)]
pub fn build_table<const V: usize>(
    name_hash: u32,
    new_page_flags: u32,
    page_words: usize,
    entries: &[(KeyedKey, [u32; V])],
) -> Result<KeyedRewrite, KeyedCowError> {
    let edits: Vec<TableEdit<V>> =
        entries.iter().map(|&(key, value)| TableEdit { key, kind: TableEditKind::Insert(value) }).collect();
    let mut next = 1;
    let mut alloc = || {
        let page = next;
        next += 1;
        page
    };
    let mut read_page = |page_no: u32| Err(format!("a new table reads no page, asked for {page_no}"));
    rewrite_table::<V>(None, name_hash, new_page_flags, page_words, &edits, &mut read_page, &mut alloc, u32::MAX)
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
    fn rewrite_node<const V: usize>(&mut self, page_no: u32, edits: &[TableEdit<V>]) -> Result<Rewritten, KeyedCowError> {
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
        let node = decode_table_page::<V>(self.name_hash, page_no, &data)?;
        let rewritten = self.rebuild(node, edits)?;
        self.depth -= 1;
        Ok(rewritten)
    }

    /// Apply `edits` to a decoded node and emit it as one page, or several
    /// when it overflows.
    fn rebuild<const V: usize>(&mut self, node: Node<V>, edits: &[TableEdit<V>]) -> Result<Rewritten, KeyedCowError> {
        let level = node.level;
        let flags = node.flags;
        let slots = if node.level == 0 {
            rewrite_leaf(node.slots, edits)
        } else {
            self.rewrite_internal(node.slots, edits)?
        };
        let max_slots = (self.page_words - HEADER_DWORDS) / if level == 0 { leaf_stride::<V>() } else { INTERNAL_STRIDE };
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

    fn rewrite_internal<const V: usize>(
        &mut self,
        slots: Vec<Slot<V>>,
        edits: &[TableEdit<V>],
    ) -> Result<Vec<Slot<V>>, KeyedCowError> {
        let mut by_slot: std::collections::BTreeMap<usize, Vec<TableEdit<V>>> = std::collections::BTreeMap::new();
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
                value: [0; V],
                child: Some(rewritten.first_page),
            });
            for (key, page) in rewritten.extra {
                rebuilt.push(Slot {
                    key: Some(key),
                    value: [0; V],
                    child: Some(page),
                });
            }
        }
        Ok(rebuilt)
    }
}

/// Apply the edits to a leaf's entries, keeping them in unsigned word order
/// of their keys.
fn rewrite_leaf<const V: usize>(slots: Vec<Slot<V>>, edits: &[TableEdit<V>]) -> Vec<Slot<V>> {
    let mut entries: Vec<(KeyedKey, [u32; V])> =
        slots.into_iter().filter_map(|slot| slot.key.map(|key| (key, slot.value))).collect();
    for edit in edits {
        match (edit.kind, entries.binary_search_by(|(key, _)| key.cmp(&edit.key))) {
            (TableEditKind::Insert(value), Err(at)) => entries.insert(at, (edit.key, value)),
            (TableEditKind::Delete, Ok(at)) => {
                entries.remove(at);
            }
            // Already there / already gone: core tolerates 529 and 534 here.
            (TableEditKind::Insert(_), Ok(_)) | (TableEditKind::Delete, Err(_)) => {}
        }
    }
    entries
        .into_iter()
        .map(|(key, value)| Slot {
            key: Some(key),
            value,
            child: None,
        })
        .collect()
}

/// One chunk unless the slots overrun a page, then as few equal chunks as
/// fit (the median split of §5 Q5).
fn split_slots<const V: usize>(slots: Vec<Slot<V>>, max_slots: usize) -> Vec<Vec<Slot<V>>> {
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
fn write_table_page<const V: usize>(
    name_hash: u32,
    level: u32,
    flags: u32,
    slots: &[Slot<V>],
    page_words: usize,
    ext: u16,
) -> Vec<u8> {
    let stride = if level == 0 { leaf_stride::<V>() } else { INTERNAL_STRIDE };
    let mut bytes = vec![0u8; page_words * 4];
    let mut put = |word: usize, value: u32| {
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_be_bytes());
    };
    put(0, TABLE_PAGE_TYPE);
    put(1, name_hash);
    put(2, level);
    put(3, KEYED_KEY_DWORDS as u32);
    put(4, V as u32);
    put(5, flags);
    put(6, (page_words - HEADER_DWORDS - slots.len() * stride) as u32);
    let mut at = HEADER_DWORDS;
    for slot in slots {
        let key = slot.key.unwrap_or([SENTINEL_WORD; KEYED_KEY_DWORDS]);
        for (offset, word) in key.iter().enumerate() {
            put(at + offset, *word);
        }
        if level == 0 {
            for (offset, word) in slot.value.iter().enumerate() {
                put(at + KEYED_KEY_DWORDS + offset, *word);
            }
        } else {
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
            let slots: Vec<Slot<0>> = keys.iter().map(|k| Slot { key: Some(*k), value: [], child: None }).collect();
            let page = self.next;
            self.next += 1;
            self.pages
                .insert(page, write_table_page(SPRE, 0, 0, &slots, PAGE_WORDS, PageId::PRIMARY));
            page
        }

        fn internal(&mut self, level: u32, children: &[(Option<KeyedKey>, u32)]) -> u32 {
            let slots: Vec<Slot<0>> = children
                .iter()
                .map(|(key, child)| Slot {
                    key: *key,
                    value: [],
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
        let new_root = decode_table_page::<0>(SPRE, result.root.page, &store.pages[&result.root.page]).unwrap();
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
            (5, SPRE, 0, 4, 0, 0, (PAGE_WORDS - HEADER_DWORDS - 2 * leaf_stride::<0>()) as u32),
            "the header sub_5AFF6D0 writes for a reference table"
        );
    }

    #[test]
    fn a_leaf_that_overflows_splits_and_the_root_grows() {
        let mut store = Store::new();
        let keys: Vec<KeyedKey> = (0..6).map(|i| key(10 * i, 1)).collect();
        let leaf = store.leaf(&keys);

        let result = store.rewrite(Some(leaf), &[insert(key(5, 1))]);
        let root = decode_table_page::<0>(SPRE, result.root.page, &store.pages[&result.root.page]).unwrap();
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
        let slots: Vec<Slot<0>> = vec![Slot { key: Some(key(1, 1)), value: [], child: None }];
        let page = store.next;
        store.next += 1;
        store
            .pages
            .insert(page, write_table_page(SPRE, 0, 1, &slots, PAGE_WORDS, PageId::PRIMARY));
        let result = store.rewrite(Some(page), &[insert(key(2, 2))]);
        let copied = decode_table_page::<0>(SPRE, result.root.page, &store.pages[&result.root.page]).unwrap();
        assert_eq!(copied.flags, 1);
    }

    #[test]
    fn a_page_of_another_table_is_refused() {
        let mut store = Store::new();
        let slots: Vec<Slot<0>> = vec![Slot { key: Some(key(1, 1)), value: [], child: None }];
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

    const SPAMAP: u32 = crate::index::SPAMAP_NAME_HASH;

    /// An in-memory table with a seven-dword leaf value, as SPAMAP's pages.
    struct ValueStore {
        pages: HashMap<u32, Vec<u8>>,
        next: u32,
    }

    impl ValueStore {
        fn rewrite(&mut self, root: Option<u32>, edits: &[TableEdit<7>]) -> KeyedRewrite {
            let snapshot = self.pages.clone();
            let mut next = self.next;
            let result = {
                let mut read = |page_no: u32| snapshot.get(&page_no).cloned().ok_or_else(|| format!("no page {page_no}"));
                let mut alloc = || {
                    let page = next;
                    next += 1;
                    page
                };
                rewrite_table::<7>(root.map(PageId::primary), SPAMAP, 1, PAGE_WORDS, edits, &mut read, &mut alloc, 1000)
                    .expect("rewrite")
            };
            self.next = next;
            self.pages.extend(result.pages.iter().cloned());
            result
        }

        /// The entries under `root`, through the reader SPAMAP's Codec_Check uses.
        fn read_back(&self, root: u32) -> Vec<([u32; 4], [u32; 7])> {
            let table = crate::index::read_spamap_tree(PageId::primary(root), |page| {
                self.pages.get(&page.page).cloned().ok_or_else(|| format!("no page {page:?}"))
            })
            .expect("the SPAMAP reader walks what the writer wrote");
            table.entries().iter().map(|entry| (entry.key, entry.value)).collect()
        }
    }

    fn entry(key0: u32, word1: u32) -> ([u32; 4], [u32; 7]) {
        ([key0, 24_384, word1, 0], [key0 % 1000, 1, 2, 3, 4, 5, 2])
    }

    fn file(entry: ([u32; 4], [u32; 7])) -> TableEdit<7> {
        TableEdit { key: entry.0, kind: TableEditKind::Insert(entry.1) }
    }

    #[test]
    fn a_seven_word_table_keeps_each_keys_value_through_inserts_deletes_and_splits() {
        // (32 - 7) / 11 = 2 entries a leaf: ten entries make a three-level tree.
        let mut store = ValueStore { pages: HashMap::new(), next: 1 };
        let entries: Vec<_> = (1..=10).map(|i| entry(300_000_000 + i * 7, i)).collect();
        let built = store.rewrite(None, &entries.iter().copied().map(file).collect::<Vec<_>>());
        assert_eq!(store.read_back(built.root.page), entries);

        // A key filed again keeps its own value (529); a gone key is nothing (534).
        let mut changed = entries[3];
        changed.1[0] = 999;
        let gone = entries[5];
        let edits = [file(changed), TableEdit { key: gone.0, kind: TableEditKind::Delete }, TableEdit {
            key: [1, 1, 1, 1],
            kind: TableEditKind::Delete,
        }];
        let result = store.rewrite(Some(built.root.page), &edits);
        let mut expected = entries.clone();
        expected.remove(5);
        assert_eq!(store.read_back(result.root.page), expected);
    }

    #[test]
    fn every_page_of_a_seven_word_table_states_its_value_width_and_its_flags() {
        let mut store = ValueStore { pages: HashMap::new(), next: 1 };
        let entries: Vec<_> = (1..=10).map(|i| entry(300_000_000 + i, i)).collect();
        let built = store.rewrite(None, &entries.iter().copied().map(file).collect::<Vec<_>>());
        assert!(built.pages.len() > 1);
        for (page_no, bytes) in &built.pages {
            let word = |i: usize| u32::from_be_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
            assert_eq!((word(0), word(1), word(3), word(4), word(5)), (5, SPAMAP, 4, 7, 1), "page {page_no}");
        }
        let fresh = build_table::<7>(SPAMAP, 1, PAGE_WORDS, &[entry(5, 1)]).unwrap();
        let page = &fresh.pages[0].1;
        let word = |i: usize| u32::from_be_bytes(page[i * 4..i * 4 + 4].try_into().unwrap());
        assert_eq!(
            (word(2), word(6)),
            (0, (PAGE_WORDS - HEADER_DWORDS - leaf_stride::<7>()) as u32),
            "one leaf, one entry: the header sub_5AFF6D0 writes plus the entry"
        );
    }
}
