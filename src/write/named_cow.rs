//! Copy-on-write edits to a `NAME`-shaped table — the variable-key B+tree
//! `core.dll` keeps a type-15 attribute's uniqueness in
//! (`docs/plans/2026-09-20-dabacon-write-plan.md` §2 阶段 2 用例 D, A3a.4).
//!
//! The shape is [`crate::index::named`]'s: page type 5, `key_dwords`
//! `0xFFFFFFFF`, `value_dwords` 2, each entry `[key_len][key word × key_len]
//! [value word × 2]`, the sentinel opening an internal page by its first key
//! word being [`SENTINEL_WORD`], and keys ordered by [`compare_keys`]. The CoW
//! is the one [`super::index_cow`] does — copy the root-to-leaf path, append
//! fresh page numbers, leave every other subtree alone — but for variable-
//! length keys, so "full" is a dword budget rather than a fixed entry count.
//!
//! Only two edits: **insert** a name the table does not have, and **delete**
//! one it does. A rename is a delete of the old key and an insert of the new;
//! `core.dll`'s own uniqueness check — a second element writing a name already
//! taken reports conflict 561 — is this module refusing an insert whose key is
//! already present ([`NamedCowError::Duplicate`]). As on the refno index, an
//! insert never lowers a subtree's least key, so no separator moves; a delete
//! leaves the separator it emptied in place, which still routes every live key
//! correctly (a lookup for the gone key lands in the same leaf and finds
//! nothing).

use crate::index::named::{compare_keys, name_table_hash, NamedKey, NAMED_VALUE_DWORDS, SENTINEL_WORD, VARIABLE_KEY_DWORDS};
use crate::page::PageId;
use crate::refno::RefNo;

const HEADER_DWORDS: usize = 7;
const TABLE_PAGE_TYPE: u32 = 5;
const VALUE_DWORDS: usize = NAMED_VALUE_DWORDS as usize;

/// What a [`NamedEdit`] does to its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedEditKind {
    Insert,
    Delete,
}

/// One change to the `NAME` table: a key and, for an insert, the RefNo it
/// should name.
#[derive(Debug, Clone)]
pub struct NamedEdit {
    pub key: NamedKey,
    pub refno: RefNo,
    pub kind: NamedEditKind,
}

/// The new table: its root and every page the rewrite produced.
#[derive(Debug)]
pub struct NamedRewrite {
    pub root: PageId,
    pub pages: Vec<(u32, Vec<u8>)>,
}

#[derive(Debug, thiserror::Error)]
pub enum NamedCowError {
    #[error("NAME table page {page_no} could not be read: {detail}")]
    Unreadable { page_no: u32, detail: String },
    #[error("NAME table page {page_no} is page type {found}, expected {TABLE_PAGE_TYPE}")]
    NotATablePage { page_no: u32, found: u32 },
    #[error("NAME table page {page_no} belongs to table {found:#x}, expected {expected:#x}")]
    WrongTable { page_no: u32, found: u32, expected: u32 },
    #[error("NAME table page {page_no} states key width {key_dwords:#x}/value width {value_dwords}, not a name table")]
    NotANameTable { page_no: u32, key_dwords: u32, value_dwords: u32 },
    #[error("NAME table page {page_no} entry at word {at} runs past the {used} words in use")]
    EntryOverrun { page_no: u32, at: usize, used: usize },
    #[error("the name {name:?} is already in the table, naming {existing} — a second element cannot take it (conflict 561)")]
    Duplicate { name: String, existing: RefNo },
    #[error("the name {name:?} was to be deleted but the table has no such key")]
    DeleteMissing { name: String },
    #[error("a name key does not fit on a {page_words}-word page")]
    KeyTooLong { page_words: usize },
    #[error("the NAME table is deeper than the {limit} pages the file holds")]
    TooDeep { limit: u32 },
}

/// One slot of a decoded page: a key (`None` on the sentinel) and its two value
/// words — a RefNo on a leaf, `(page, ext)` on an internal page.
#[derive(Debug, Clone)]
struct Slot {
    key: Option<NamedKey>,
    value: [u32; 2],
}

impl Slot {
    fn dwords(&self) -> usize {
        let key_len = self.key.as_ref().map_or(1, |k| k.0.len());
        1 + key_len + VALUE_DWORDS
    }
}

struct Rewritten {
    first_page: u32,
    level: u32,
    extra: Vec<(NamedKey, u32)>,
}

/// Rewrite the `NAME` table at `root` with `edits`.
pub fn rewrite_named_table(
    root: PageId,
    page_words: usize,
    edits: &[NamedEdit],
    read_page: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    alloc: &mut dyn FnMut() -> u32,
    page_limit: u32,
) -> Result<NamedRewrite, NamedCowError> {
    let mut cow = Cow {
        name_hash: name_table_hash(),
        ext: root.ext,
        page_words,
        budget: page_words - HEADER_DWORDS,
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
            value: [rewritten.first_page, u32::from(cow.ext)],
        }];
        for (key, page) in &rewritten.extra {
            slots.push(Slot {
                key: Some(key.clone()),
                value: [*page, u32::from(cow.ext)],
            });
        }
        let new_root = (cow.alloc)();
        let bytes = write_named_page(rewritten.level + 1, &slots, cow.name_hash, cow.page_words);
        cow.pages.push((new_root, bytes));
        PageId::new(cow.ext, new_root)
    };
    Ok(NamedRewrite {
        root,
        pages: cow.pages,
    })
}

struct Cow<'a> {
    name_hash: u32,
    ext: u16,
    page_words: usize,
    budget: usize,
    read_page: &'a mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    alloc: &'a mut dyn FnMut() -> u32,
    pages: Vec<(u32, Vec<u8>)>,
    depth: u32,
    page_limit: u32,
}

impl Cow<'_> {
    fn rewrite_node(&mut self, page_no: u32, edits: &[NamedEdit]) -> Result<Rewritten, NamedCowError> {
        self.depth += 1;
        if self.depth > self.page_limit {
            return Err(NamedCowError::TooDeep { limit: self.page_limit });
        }
        let data = (self.read_page)(page_no).map_err(|detail| NamedCowError::Unreadable { page_no, detail })?;
        let (level, slots) = self.decode(page_no, &data)?;
        let rebuilt = if level == 0 {
            self.rewrite_leaf(page_no, slots, edits)?
        } else {
            self.rewrite_internal(slots, edits)?
        };
        self.depth -= 1;

        let chunks = self.split(page_no, rebuilt)?;
        let mut pages = chunks.into_iter().map(|chunk| {
            let page = (self.alloc)();
            let key = chunk[0].key.clone();
            let bytes = write_named_page(level, &chunk, self.name_hash, self.page_words);
            self.pages.push((page, bytes));
            (key, page)
        });
        let (_, first_page) = pages.next().expect("a node always has one page");
        let extra: Vec<(NamedKey, u32)> = pages
            .map(|(key, page)| (key.expect("a split-off chunk starts with a real key"), page))
            .collect();
        Ok(Rewritten { first_page, level, extra })
    }

    fn rewrite_leaf(
        &self,
        page_no: u32,
        mut slots: Vec<Slot>,
        edits: &[NamedEdit],
    ) -> Result<Vec<Slot>, NamedCowError> {
        for edit in edits {
            let position = slots.iter().position(|s| s.key.as_ref().is_some_and(|k| k.0 == edit.key.0));
            match edit.kind {
                NamedEditKind::Insert => {
                    if let Some(index) = position {
                        let existing = slots[index].value;
                        return Err(NamedCowError::Duplicate {
                            name: edit.key.text(),
                            existing: RefNo::new(existing[0], existing[1]),
                        });
                    }
                    let at = slots
                        .iter()
                        .position(|s| {
                            s.key.as_ref().is_some_and(|k| compare_keys(&k.0, &edit.key.0) == std::cmp::Ordering::Greater)
                        })
                        .unwrap_or(slots.len());
                    slots.insert(at, Slot {
                        key: Some(edit.key.clone()),
                        value: [edit.refno.word0, edit.refno.word1],
                    });
                }
                NamedEditKind::Delete => {
                    let Some(index) = position else {
                        return Err(NamedCowError::DeleteMissing { name: edit.key.text() });
                    };
                    slots.remove(index);
                }
            }
        }
        let _ = page_no;
        Ok(slots)
    }

    fn rewrite_internal(&mut self, slots: Vec<Slot>, edits: &[NamedEdit]) -> Result<Vec<Slot>, NamedCowError> {
        let mut by_child: std::collections::HashMap<u32, Vec<NamedEdit>> = std::collections::HashMap::new();
        for edit in edits {
            let child = route(&slots, &edit.key.0);
            by_child.entry(slots[child].value[0]).or_default().push(edit.clone());
        }
        let mut rebuilt = Vec::with_capacity(slots.len());
        for slot in &slots {
            let Some(child_edits) = by_child.get(&slot.value[0]) else {
                rebuilt.push(slot.clone());
                continue;
            };
            let rewritten = self.rewrite_node(slot.value[0], child_edits)?;
            let mut updated = slot.clone();
            updated.value = [rewritten.first_page, u32::from(self.ext)];
            rebuilt.push(updated);
            for (key, page) in rewritten.extra {
                rebuilt.push(Slot {
                    key: Some(key),
                    value: [page, u32::from(self.ext)],
                });
            }
        }
        Ok(rebuilt)
    }

    /// One chunk, or several when the entries overrun a page's dword budget.
    fn split(&self, page_no: u32, slots: Vec<Slot>) -> Result<Vec<Vec<Slot>>, NamedCowError> {
        let total: usize = slots.iter().map(Slot::dwords).sum();
        if total <= self.budget {
            return Ok(vec![slots]);
        }
        let mut chunks: Vec<Vec<Slot>> = Vec::new();
        let mut current: Vec<Slot> = Vec::new();
        let mut used = 0usize;
        for slot in slots {
            let need = slot.dwords();
            if need > self.budget {
                return Err(NamedCowError::KeyTooLong { page_words: self.page_words });
            }
            if used + need > self.budget && !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                used = 0;
            }
            used += need;
            current.push(slot);
        }
        if !current.is_empty() {
            chunks.push(current);
        }
        let _ = page_no;
        Ok(chunks)
    }

    fn decode(&self, page_no: u32, data: &[u8]) -> Result<(u32, Vec<Slot>), NamedCowError> {
        let word = |index: usize| u32::from_be_bytes(data[index * 4..index * 4 + 4].try_into().unwrap());
        let page_type = word(0);
        if page_type != TABLE_PAGE_TYPE {
            return Err(NamedCowError::NotATablePage { page_no, found: page_type });
        }
        let found = word(1);
        if found != self.name_hash {
            return Err(NamedCowError::WrongTable { page_no, found, expected: self.name_hash });
        }
        let level = word(2);
        let key_dwords = word(3);
        let value_dwords = word(4);
        if key_dwords != VARIABLE_KEY_DWORDS || value_dwords != NAMED_VALUE_DWORDS {
            return Err(NamedCowError::NotANameTable { page_no, key_dwords, value_dwords });
        }
        let free = word(6) as usize;
        let usable = data.len() / 4 - HEADER_DWORDS;
        let used = usable.saturating_sub(free);
        let end = HEADER_DWORDS + used;
        let mut slots = Vec::new();
        let mut at = HEADER_DWORDS;
        while at < end {
            let key_len = word(at) as usize;
            if key_len == 0 || at + 1 + key_len + VALUE_DWORDS > end {
                return Err(NamedCowError::EntryOverrun { page_no, at, used });
            }
            let words: Vec<u32> = (0..key_len).map(|i| word(at + 1 + i)).collect();
            let value = [word(at + 1 + key_len), word(at + 2 + key_len)];
            let sentinel = words[0] == SENTINEL_WORD;
            slots.push(Slot {
                key: (!sentinel).then_some(NamedKey(words)),
                value,
            });
            at += 1 + key_len + VALUE_DWORDS;
        }
        Ok((level, slots))
    }
}

/// The slot to descend through for `target`: the greatest key `<= target`,
/// else the sentinel's slot, else the first. Mirrors [`crate::index::named`]'s
/// `Node::route`.
fn route(slots: &[Slot], target: &[u32]) -> usize {
    let mut best: Option<usize> = None;
    let mut leftmost: Option<usize> = None;
    for (index, slot) in slots.iter().enumerate() {
        let Some(key) = &slot.key else {
            if leftmost.is_none() {
                leftmost = Some(index);
            }
            continue;
        };
        if leftmost.is_none() {
            leftmost = Some(index);
        }
        if compare_keys(&key.0, target) == std::cmp::Ordering::Greater {
            continue;
        }
        match best {
            Some(b) => {
                let best_key = slots[b].key.as_ref().expect("best is never the sentinel");
                if compare_keys(&key.0, &best_key.0) == std::cmp::Ordering::Greater {
                    best = Some(index);
                }
            }
            None => best = Some(index),
        }
    }
    best.or(leftmost).unwrap_or(0)
}

/// Serialise a `NAME`-shaped page: the seven-word header, then the entries,
/// the sentinel written as a one-word key whatever it was read as.
fn write_named_page(level: u32, slots: &[Slot], name_hash: u32, page_words: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; page_words * 4];
    let mut put = |word: usize, value: u32| {
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_be_bytes());
    };
    let used: usize = slots.iter().map(Slot::dwords).sum();
    let free = page_words - HEADER_DWORDS - used;
    put(0, TABLE_PAGE_TYPE);
    put(1, name_hash);
    put(2, level);
    put(3, VARIABLE_KEY_DWORDS);
    put(4, NAMED_VALUE_DWORDS);
    put(5, 0);
    put(6, free as u32);
    let mut at = HEADER_DWORDS;
    for slot in slots {
        match &slot.key {
            Some(key) => {
                put(at, key.0.len() as u32);
                for (i, word) in key.0.iter().enumerate() {
                    put(at + 1 + i, *word);
                }
                at += 1 + key.0.len();
            }
            None => {
                put(at, 1);
                put(at + 1, SENTINEL_WORD);
                at += 2;
            }
        }
        put(at, slot.value[0]);
        put(at + 1, slot.value[1]);
        at += VALUE_DWORDS;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::named::encode_name;
    use std::collections::HashMap;

    const PAGE_WORDS: usize = 64;

    struct Store {
        pages: HashMap<u32, Vec<u8>>,
        next: u32,
    }

    impl Store {
        fn leaf(&mut self, entries: &[(&str, RefNo)]) -> u32 {
            let slots: Vec<Slot> = entries
                .iter()
                .map(|(name, refno)| Slot {
                    key: Some(encode_name(name)),
                    value: [refno.word0, refno.word1],
                })
                .collect();
            let page = self.next;
            self.next += 1;
            self.pages.insert(page, write_named_page(0, &slots, name_table_hash(), PAGE_WORDS));
            page
        }
    }

    fn rewrite(store: &mut Store, root: u32, edits: &[NamedEdit]) -> Result<NamedRewrite, NamedCowError> {
        let snapshot: HashMap<u32, Vec<u8>> = store.pages.clone();
        let mut next = store.next;
        let result = {
            let mut read = |p: u32| snapshot.get(&p).cloned().ok_or_else(|| format!("no page {p}"));
            let mut alloc = || {
                let p = next;
                next += 1;
                p
            };
            rewrite_named_table(PageId::primary(root), PAGE_WORDS, edits, &mut read, &mut alloc, 1000)
        };
        if let Ok(rewrite) = &result {
            store.next = next;
            for (page, bytes) in &rewrite.pages {
                store.pages.insert(*page, bytes.clone());
            }
        }
        result
    }

    fn names(store: &Store, root: u32) -> Vec<(String, RefNo)> {
        let cow = Cow {
            name_hash: name_table_hash(),
            ext: 1,
            page_words: PAGE_WORDS,
            budget: PAGE_WORDS - HEADER_DWORDS,
            read_page: &mut |_| Err(String::new()),
            alloc: &mut || 0,
            pages: Vec::new(),
            depth: 0,
            page_limit: 1000,
        };
        fn walk(cow: &Cow, store: &Store, page: u32, out: &mut Vec<(String, RefNo)>) {
            let data = &store.pages[&page];
            let (level, slots) = cow.decode(page, data).unwrap();
            for slot in &slots {
                match &slot.key {
                    Some(key) if level == 0 => out.push((key.text(), RefNo::new(slot.value[0], slot.value[1]))),
                    _ if level == 0 => {}
                    _ => walk(cow, store, slot.value[0], out),
                }
            }
        }
        let mut out = Vec::new();
        walk(&cow, store, root, &mut out);
        out
    }

    #[test]
    fn insert_places_in_key_order_and_delete_removes() {
        let mut store = Store { pages: HashMap::new(), next: 100 };
        let leaf = store.leaf(&[("/AAA", RefNo::new(1, 1)), ("/CCC", RefNo::new(1, 3))]);

        let result = rewrite(
            &mut store,
            leaf,
            &[NamedEdit { key: encode_name("/BBB"), refno: RefNo::new(1, 2), kind: NamedEditKind::Insert }],
        )
        .unwrap();
        assert_eq!(
            names(&store, result.root.page),
            vec![
                ("/AAA".into(), RefNo::new(1, 1)),
                ("/BBB".into(), RefNo::new(1, 2)),
                ("/CCC".into(), RefNo::new(1, 3)),
            ]
        );

        let result = rewrite(
            &mut store,
            result.root.page,
            &[NamedEdit { key: encode_name("/AAA"), refno: RefNo::new(0, 0), kind: NamedEditKind::Delete }],
        )
        .unwrap();
        assert_eq!(
            names(&store, result.root.page),
            vec![("/BBB".into(), RefNo::new(1, 2)), ("/CCC".into(), RefNo::new(1, 3))]
        );
    }

    #[test]
    fn a_duplicate_name_is_refused_as_conflict_561() {
        let mut store = Store { pages: HashMap::new(), next: 100 };
        let leaf = store.leaf(&[("/AAA", RefNo::new(1, 1))]);
        let error = rewrite(
            &mut store,
            leaf,
            &[NamedEdit { key: encode_name("/AAA"), refno: RefNo::new(9, 9), kind: NamedEditKind::Insert }],
        )
        .unwrap_err();
        assert!(matches!(error, NamedCowError::Duplicate { existing, .. } if existing == RefNo::new(1, 1)), "{error}");
    }

    #[test]
    fn a_leaf_that_overflows_splits_and_grows_a_root() {
        let mut store = Store { pages: HashMap::new(), next: 100 };
        // Budget is 64 - 7 = 57 dwords; each /NAMEnn is 1 + 2 + 2 = 5 dwords,
        // so eleven of them (55) is near full and one more forces a split.
        let entries: Vec<(String, RefNo)> = (0..11).map(|i| (format!("/NAME{i:02}"), RefNo::new(1, i + 1))).collect();
        let refs: Vec<(&str, RefNo)> = entries.iter().map(|(n, r)| (n.as_str(), *r)).collect();
        let leaf = store.leaf(&refs);
        let result = rewrite(
            &mut store,
            leaf,
            &[NamedEdit { key: encode_name("/NAME99"), refno: RefNo::new(1, 99), kind: NamedEditKind::Insert }],
        )
        .unwrap();
        let out = names(&store, result.root.page);
        assert_eq!(out.len(), 12);
        let mut sorted = out.clone();
        sorted.sort_by(|a, b| compare_keys(&encode_name(&a.0).0, &encode_name(&b.0).0));
        assert_eq!(out, sorted, "still in key order across the split");
    }
}
