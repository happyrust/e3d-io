//! Copy-on-write edits to the RefNo index (`docs/plans/2026-09-20-dabacon-write-plan.md`
//! §2 阶段 2, A2.2/A5).
//!
//! A single writer that never rewrites a live page: to change the tree it
//! copies every page from the root down to each affected leaf, gives the
//! copies fresh page numbers (which the caller draws from `old_end + 1` up, so
//! they are all appended), and leaves every other page — and its old page
//! number — exactly where it was. The result references only new pages and
//! untouched old ones, which is what [`crate::index::IndexCursor`] and
//! [`crate::index::IndexDiff`] read against the previous session to report the
//! change.
//!
//! Edits are batched so that a page shared by two of them is copied once. Each
//! [`LeafEdit`] either inserts a key or replaces the value under an existing
//! one; the routing down the tree is [`crate::index::choose_child`], the same
//! rule the reader descends with, so a key lands in the leaf a later lookup
//! will find it in.
//!
//! **Keys never move on an insert.** An internal separator is the least key of
//! the subtree behind it, and `choose_child` sends a key to the child behind
//! the greatest separator at or below it — so a newly inserted key is never
//! below its leaf's existing minimum, and no separator above it has to change.
//! Only page numbers propagate up an insert that does not split. A page that
//! overflows its `page_words` splits at the median (§5 Q5); a root that splits
//! grows a new root above the pieces.

use crate::index::{
    choose_child, cmp_refno, IndexEntry, IndexNode, IndexPageError, INDEX_PAGE_TYPE,
};
use crate::meta::constants::DATA_PAGE_INDEX;
use crate::page::PageId;
use crate::refno::RefNo;

const HEADER_DWORDS: usize = 7;
const ENTRY_DWORDS: usize = 4;
const SENTINEL_WORD: u32 = 0x8000_0001;

/// What a [`LeafEdit`] does to its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// Add a key the tree does not have (a new element).
    Insert,
    /// Replace the value under a key it does have (a record that moved).
    Update,
    /// Remove a key (a deleted element). `page_no`/`packed` are ignored.
    Delete,
}

/// One change to the leaf level: a key, what to do with it, and — for an
/// insert or an update — the record address it should point at.
#[derive(Debug, Clone, Copy)]
pub struct LeafEdit {
    pub refno: RefNo,
    /// The record's page number.
    pub page_no: u32,
    /// The packed second word of the leaf value: `ext | (word_offset << 13)`
    /// with the offset in 4-byte words ([`super::record::pack_offset`]).
    pub packed: u32,
    pub kind: EditKind,
}

/// The new tree: its root, and every page the rewrite produced.
#[derive(Debug)]
pub struct IndexRewrite {
    pub root: PageId,
    /// `(page_no, bytes)` for each new page, in no particular order — the
    /// caller writes them all before the session page.
    pub pages: Vec<(u32, Vec<u8>)>,
}

#[derive(Debug, thiserror::Error)]
pub enum IndexCowError {
    #[error("index page {page_no} could not be read: {detail}")]
    Unreadable { page_no: u32, detail: String },
    #[error("index page {page_no} is not a usable index page: {source}")]
    Malformed {
        page_no: u32,
        #[source]
        source: IndexPageError,
    },
    #[error("{refno} was to be updated but the tree has no such key")]
    UpdateMissing { refno: RefNo },
    #[error("{refno} was to be deleted but the tree has no such key")]
    DeleteMissing { refno: RefNo },
    #[error("{refno} was to be inserted but the tree already has it")]
    DuplicateInsert { refno: RefNo },
    #[error("an internal page routes {refno} to a child it does not list")]
    ChildNotFound { refno: RefNo },
    #[error("the tree is deeper than the {limit} pages the file holds")]
    TooDeep { limit: u32 },
}

/// What a rewritten subtree hands back to its parent: the page that replaces
/// the original node, plus any siblings a split created, each with the least
/// key of its subtree.
struct Rewritten {
    first_page: u32,
    level: u32,
    extra: Vec<(RefNo, u32)>,
}

/// Rewrite the tree at `root` with `edits`, reading old pages through
/// `read_page` and drawing new page numbers from `alloc`.
pub fn rewrite_index(
    root: PageId,
    page_words: usize,
    edits: &[LeafEdit],
    read_page: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    alloc: &mut dyn FnMut() -> u32,
    page_limit: u32,
) -> Result<IndexRewrite, IndexCowError> {
    let mut cow = Cow {
        ext: root.ext,
        page_words,
        max_entries: (page_words - HEADER_DWORDS) / ENTRY_DWORDS,
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
        // The root split: a new root one level up, its sentinel pointing at the
        // lower piece and a separator at each higher one.
        let mut entries = vec![sentinel_entry(rewritten.first_page)];
        for (key, page) in &rewritten.extra {
            entries.push(child_entry(*key, *page));
        }
        let new_root = (cow.alloc)();
        let bytes = write_index_page(rewritten.level + 1, &entries, cow.page_words);
        cow.pages.push((new_root, bytes));
        PageId::new(cow.ext, new_root)
    };
    Ok(IndexRewrite {
        root,
        pages: cow.pages,
    })
}

struct Cow<'a> {
    ext: u16,
    page_words: usize,
    max_entries: usize,
    read_page: &'a mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    alloc: &'a mut dyn FnMut() -> u32,
    pages: Vec<(u32, Vec<u8>)>,
    depth: u32,
    page_limit: u32,
}

impl Cow<'_> {
    fn rewrite_node(&mut self, page_no: u32, edits: &[LeafEdit]) -> Result<Rewritten, IndexCowError> {
        self.depth += 1;
        if self.depth > self.page_limit {
            return Err(IndexCowError::TooDeep {
                limit: self.page_limit,
            });
        }
        let data = (self.read_page)(page_no).map_err(|detail| IndexCowError::Unreadable {
            page_no,
            detail,
        })?;
        let node = IndexNode::decode(&data).map_err(|source| IndexCowError::Malformed {
            page_no,
            source,
        })?;
        let level = node.level;
        let entries: Vec<IndexEntry> = node.entries().collect();

        let rebuilt = if node.is_leaf() {
            self.rewrite_leaf(entries, edits)?
        } else {
            self.rewrite_internal(entries, edits)?
        };
        self.depth -= 1;

        // Emit the rebuilt entry list as one page or, if it overflows, several.
        let chunks = self.split_entries(rebuilt);
        let mut pages = chunks.into_iter().map(|chunk| {
            let page = (self.alloc)();
            let min_key = chunk[0].refno;
            let bytes = write_index_page(level, &chunk, self.page_words);
            self.pages.push((page, bytes));
            (min_key, page)
        });
        let (_, first_page) = pages.next().expect("a node always has at least one page");
        let extra: Vec<(RefNo, u32)> = pages.collect();
        Ok(Rewritten {
            first_page,
            level,
            extra,
        })
    }

    fn rewrite_leaf(
        &mut self,
        mut entries: Vec<IndexEntry>,
        edits: &[LeafEdit],
    ) -> Result<Vec<IndexEntry>, IndexCowError> {
        for edit in edits {
            let position = entries.iter().position(|e| e.refno == edit.refno);
            match (edit.kind, position) {
                (EditKind::Insert, Some(_)) => {
                    return Err(IndexCowError::DuplicateInsert { refno: edit.refno })
                }
                (EditKind::Update, None) => {
                    return Err(IndexCowError::UpdateMissing { refno: edit.refno })
                }
                (EditKind::Delete, None) => {
                    return Err(IndexCowError::DeleteMissing { refno: edit.refno })
                }
                (EditKind::Update, Some(index)) => {
                    entries[index] = leaf_entry(edit.refno, edit.page_no, edit.packed);
                }
                (EditKind::Delete, Some(index)) => {
                    entries.remove(index);
                }
                (EditKind::Insert, None) => {
                    let at = entries
                        .iter()
                        .position(|e| cmp_refno(e.refno, edit.refno) == std::cmp::Ordering::Greater)
                        .unwrap_or(entries.len());
                    entries.insert(at, leaf_entry(edit.refno, edit.page_no, edit.packed));
                }
            }
        }
        Ok(entries)
    }

    fn rewrite_internal(
        &mut self,
        entries: Vec<IndexEntry>,
        edits: &[LeafEdit],
    ) -> Result<Vec<IndexEntry>, IndexCowError> {
        // Which child each edit descends into.
        let mut by_child: std::collections::HashMap<u32, Vec<LeafEdit>> =
            std::collections::HashMap::new();
        for edit in edits {
            let child = choose_child(entries.iter().copied(), edit.refno)
                .ok_or(IndexCowError::ChildNotFound { refno: edit.refno })?;
            by_child.entry(child).or_default().push(*edit);
        }

        let mut rebuilt = Vec::with_capacity(entries.len());
        for entry in &entries {
            let Some(child_edits) = by_child.get(&entry.page_no) else {
                rebuilt.push(*entry);
                continue;
            };
            let rewritten = self.rewrite_node(entry.page_no, child_edits)?;
            // The separator for this child is unchanged — an insert never
            // lowers a subtree's minimum — so only the page number moves.
            let mut updated = *entry;
            updated.page_no = rewritten.first_page;
            rebuilt.push(updated);
            for (key, page) in rewritten.extra {
                rebuilt.push(child_entry(key, page));
            }
        }
        Ok(rebuilt)
    }

    /// One chunk unless the entries overrun a page, then median-split into as
    /// few equal chunks as fit.
    fn split_entries(&self, entries: Vec<IndexEntry>) -> Vec<Vec<IndexEntry>> {
        if entries.len() <= self.max_entries {
            return vec![entries];
        }
        let parts = entries.len().div_ceil(self.max_entries);
        let base = entries.len() / parts;
        let remainder = entries.len() % parts;
        let mut chunks = Vec::with_capacity(parts);
        let mut rest = entries.as_slice();
        for part in 0..parts {
            let take = base + usize::from(part < remainder);
            let (chunk, tail) = rest.split_at(take);
            chunks.push(chunk.to_vec());
            rest = tail;
        }
        chunks
    }
}

fn leaf_entry(refno: RefNo, page_no: u32, packed: u32) -> IndexEntry {
    IndexEntry {
        refno,
        page_no,
        offset_words: packed >> 12,
        flag: (packed & 0xFFF) as u16,
    }
}

fn child_entry(key: RefNo, page_no: u32) -> IndexEntry {
    IndexEntry {
        refno: key,
        page_no,
        offset_words: 0,
        flag: 1,
    }
}

fn sentinel_entry(page_no: u32) -> IndexEntry {
    child_entry(RefNo::new(SENTINEL_WORD, SENTINEL_WORD), page_no)
}

/// Serialise an index page: the seven-word header then the entries, the rest
/// left zero, `free_dwords` set to match.
fn write_index_page(level: u32, entries: &[IndexEntry], page_words: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; page_words * 4];
    let mut put = |word: usize, value: u32| {
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_be_bytes());
    };
    let free = page_words - HEADER_DWORDS - entries.len() * ENTRY_DWORDS;
    put(0, INDEX_PAGE_TYPE);
    put(1, DATA_PAGE_INDEX);
    put(2, level);
    put(3, 2);
    put(4, 2);
    put(5, 0);
    put(6, free as u32);
    let mut at = HEADER_DWORDS;
    for entry in entries {
        put(at, entry.refno.word0);
        put(at + 1, entry.refno.word1);
        put(at + 2, entry.page_no);
        put(at + 3, (entry.offset_words << 12) | u32::from(entry.flag));
        at += ENTRY_DWORDS;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexNode;
    use std::collections::HashMap;

    const PAGE_WORDS: usize = 32;

    /// A tiny tree store keyed by page number, so a rewrite can be read back.
    struct Store {
        pages: HashMap<u32, Vec<u8>>,
        next: u32,
    }

    impl Store {
        fn leaf(&mut self, entries: &[(RefNo, u32)]) -> u32 {
            let entries: Vec<IndexEntry> = entries
                .iter()
                .map(|(refno, page)| leaf_entry(*refno, *page, (1 << 13) | 1))
                .collect();
            let page = self.next;
            self.next += 1;
            self.pages.insert(page, write_index_page(0, &entries, PAGE_WORDS));
            page
        }

        fn internal(&mut self, level: u32, entries: &[IndexEntry]) -> u32 {
            let page = self.next;
            self.next += 1;
            self.pages
                .insert(page, write_index_page(level, entries, PAGE_WORDS));
            page
        }
    }

    fn rewrite(store: &mut Store, root: u32, edits: &[LeafEdit]) -> IndexRewrite {
        let snapshot: HashMap<u32, Vec<u8>> = store.pages.clone();
        let mut next = store.next;
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
            rewrite_index(
                PageId::primary(root),
                PAGE_WORDS,
                edits,
                &mut read,
                &mut alloc,
                1000,
            )
            .expect("rewrite")
        };
        store.next = next;
        for (page, bytes) in &result.pages {
            store.pages.insert(*page, bytes.clone());
        }
        result
    }

    /// Walk a tree to its leaves in key order.
    fn leaves(store: &Store, root: u32) -> Vec<(RefNo, u32, u32)> {
        fn walk(store: &Store, page: u32, out: &mut Vec<(RefNo, u32, u32)>) {
            let data = &store.pages[&page];
            let node = IndexNode::decode(data).unwrap();
            if node.is_leaf() {
                for entry in node.entries() {
                    out.push((entry.refno, entry.page_no, entry.offset_words));
                }
            } else {
                for entry in node.entries() {
                    walk(store, entry.page_no, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(store, root, &mut out);
        out
    }

    fn edit(refno: RefNo, page_no: u32, kind: EditKind) -> LeafEdit {
        LeafEdit {
            refno,
            page_no,
            packed: (1 << 13) | 1,
            kind,
        }
    }

    #[test]
    fn an_insert_lands_in_key_order_and_leaves_the_rest_untouched() {
        let mut store = Store {
            pages: HashMap::new(),
            next: 100,
        };
        let left = store.leaf(&[(RefNo::new(10, 0), 5), (RefNo::new(20, 0), 6)]);
        let right = store.leaf(&[(RefNo::new(40, 0), 7), (RefNo::new(50, 0), 8)]);
        let root = store.internal(
            1,
            &[
                sentinel_entry(left),
                child_entry(RefNo::new(40, 0), right),
            ],
        );

        let result = rewrite(&mut store, root, &[edit(RefNo::new(30, 0), 9, EditKind::Insert)]);
        let keys: Vec<RefNo> = leaves(&store, result.root.page)
            .into_iter()
            .map(|(refno, _, _)| refno)
            .collect();
        assert_eq!(
            keys,
            vec![
                RefNo::new(10, 0),
                RefNo::new(20, 0),
                RefNo::new(30, 0),
                RefNo::new(40, 0),
                RefNo::new(50, 0),
            ]
        );
        // The right subtree had no edit: its page number is unchanged.
        let root_node = store.pages[&result.root.page].clone();
        let node = IndexNode::decode(&root_node).unwrap();
        let right_child = node.entries().nth(1).unwrap().page_no;
        assert_eq!(right_child, right, "an untouched subtree keeps its page");
    }

    #[test]
    fn an_update_replaces_the_value_under_an_existing_key() {
        let mut store = Store {
            pages: HashMap::new(),
            next: 100,
        };
        let leaf = store.leaf(&[(RefNo::new(10, 0), 5), (RefNo::new(20, 0), 6)]);

        let result = rewrite(
            &mut store,
            leaf,
            &[LeafEdit {
                refno: RefNo::new(20, 0),
                page_no: 999,
                packed: (44 << 13) | 1,
                kind: EditKind::Update,
            }],
        );
        let updated = leaves(&store, result.root.page)
            .into_iter()
            .find(|(refno, _, _)| *refno == RefNo::new(20, 0))
            .unwrap();
        assert_eq!(updated.1, 999, "the value's page moved");
        assert_eq!(updated.2, 44 << 1, "and its 2-byte word offset with it");
    }

    #[test]
    fn a_delete_removes_the_key_and_leaves_the_others() {
        let mut store = Store {
            pages: HashMap::new(),
            next: 100,
        };
        let leaf = store.leaf(&[
            (RefNo::new(10, 0), 5),
            (RefNo::new(20, 0), 6),
            (RefNo::new(30, 0), 7),
        ]);

        let result = rewrite(&mut store, leaf, &[edit(RefNo::new(20, 0), 0, EditKind::Delete)]);
        let keys: Vec<RefNo> = leaves(&store, result.root.page)
            .into_iter()
            .map(|(refno, _, _)| refno)
            .collect();
        assert_eq!(keys, vec![RefNo::new(10, 0), RefNo::new(30, 0)], "the middle key is gone");

        // Deleting a key the tree does not have is an error, not a silent no-op.
        let snapshot: HashMap<u32, Vec<u8>> = store.pages.clone();
        let mut next = store.next;
        let mut read = |page_no: u32| snapshot.get(&page_no).cloned().ok_or_else(|| format!("no page {page_no}"));
        let mut alloc = || {
            let page = next;
            next += 1;
            page
        };
        let error = rewrite_index(
            PageId::primary(result.root.page),
            PAGE_WORDS,
            &[edit(RefNo::new(99, 0), 0, EditKind::Delete)],
            &mut read,
            &mut alloc,
            1000,
        )
        .unwrap_err();
        assert!(matches!(error, IndexCowError::DeleteMissing { .. }), "{error}");
    }

    #[test]
    fn a_leaf_that_overflows_splits_and_the_root_grows() {
        let mut store = Store {
            pages: HashMap::new(),
            next: 100,
        };
        // A single leaf at capacity: (32 - 7) / 4 = 6 entries.
        let entries: Vec<(RefNo, u32)> = (0..6).map(|i| (RefNo::new(i * 10, 0), 500 + i)).collect();
        let leaf = store.leaf(&entries);

        let result = rewrite(&mut store, leaf, &[edit(RefNo::new(5, 0), 900, EditKind::Insert)]);
        // The root is no longer the leaf: it split and a new root points at
        // the pieces.
        assert_ne!(result.root.page, leaf);
        let root = IndexNode::decode(&store.pages[&result.root.page]).unwrap();
        assert!(!root.is_leaf(), "the new root is internal");
        assert_eq!(root.entries().next().unwrap().refno, RefNo::new(SENTINEL_WORD, SENTINEL_WORD));
        let keys: Vec<RefNo> = leaves(&store, result.root.page)
            .into_iter()
            .map(|(refno, _, _)| refno)
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_by(|a, b| cmp_refno(*a, *b));
        assert_eq!(keys, sorted, "keys stay in order across the split");
        assert_eq!(keys.len(), 7);
    }
}
