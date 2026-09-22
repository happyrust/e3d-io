//! Tables keyed by text: the `NAME` table, and any other whose header states
//! a variable key width.
//!
//! A dabacon B+tree page (type 5) carries the same seven-word header whatever
//! the table: `page_type, name_hash, level, key_dwords, value_dwords, reserved,
//! free_dwords`. The refno index and the reverse-reference tables state a
//! fixed `key_dwords` (2 and 4) and pack their entries as a struct array
//! ([`crate::index`], [`crate::index::reference`]). The `NAME` table states
//! `key_dwords = 0xFFFFFFFF` and lays each entry out as
//!
//! ```text
//! [key_len_words][key word × key_len_words][value word × value_dwords]
//! ```
//!
//! where the key is the element's name as stored — leading `/`, four bytes to
//! a word, NUL-padded to the word — and the value is a `(word0, word1)` RefNo
//! on a leaf and a `(pgno, extno)` child address on an internal page. The
//! sentinel that opens an internal page is the entry whose **first key word**
//! is `0x80000001`, the same constant the refno index uses; its stated length
//! is whatever key it overwrote, and the words behind the first are that
//! key's leftovers. `ams8000_0001`'s root states a one-word sentinel;
//! `TES6804_0001`'s level-1 page 49875 states a seven-word one whose tail
//! still reads `.81A/45&~!cHHQ9MdM7 &:50`. Read off `ams8000_0001` at
//! session 640: root page 16306 (level 2, `value_dwords` 2, 3 entries: the
//! sentinel → 9700, `/-RX-CCV-S5041-BAR-1/F1` → 12274,
//! `/1RX03-MSUP-E0049-BAR-1` → 16304), leaf 4821 holding
//! `/-CSV-E-1-H-7802 → 24384/24942` and five more.
//!
//! `core.dll` tells this table apart from the fixed-width ones by the sign of
//! the header word: `3STS` (`sub_5B02B80`) routes through `sub_5B00F10` on
//! internal pages and locates with `sub_5B010F0` on the leaf, and both take
//! the variable-key branch when `key_dwords <= 0` as a signed word — which
//! `0xFFFFFFFF` is. Its merge code (`sub_5B07750`) picks the same tables as
//! the "name-like" user tables, reads them with a 512-word key buffer, and
//! reports two keys naming different elements as conflict 561 (write-path
//! note A3a.2). The tree is what gives `NAME` its uniqueness: a type-15
//! attribute's value is also its key here, and a second element writing the
//! same key hits error 529 (§5 of the note).
//!
//! **Ordering**, as `sub_5B00F10` / `sub_5B010F0` compare: word by word as
//! **unsigned** big-endian words, except that the word `0x80000001` is
//! smaller than every other word (so the sentinel sorts first whatever
//! follows it); when the shorter key is a prefix of the longer, the shorter
//! sorts first: `/-NX-CSV-E01` (three words) precedes `/-NX-CSV-E01-78`
//! (four). For NUL-padded ASCII that is plain byte order; for UTF-8 names it
//! is byte order too, where a signed comparison would put a word starting
//! with a lead byte `>= 0x80` first. Routing is the greatest key `<= target`
//! — `sub_5B00F10` scans the page and keeps the last entry not above the
//! target — so anything below the first real key goes to the sentinel's
//! child, the same rule as [`crate::index::choose_child`]. The separators
//! need not be live keys — `/-CCV-S-3-L-1306` routes to a leaf whose first
//! key is `/-CSV-E-1-H-7802` — which is what a deleted name leaves behind,
//! and the rule still holds.
//!
//! Read-only. The writer that will maintain this table (stage 2 of
//! `docs/plans/2026-09-20-dabacon-write-plan.md`) uses this module as its
//! acceptance reader.

use std::cmp::Ordering;

use crate::error::E3dReaderError;
use crate::page::cache::PageCache;
use crate::page::{IoStats, PageId};
use crate::record::text::{TextEncodeError, TextFormat};
use crate::refno::RefNo;
use crate::session::DbView;

use super::reference::TABLE_PAGE_TYPE;

const DWORD: usize = 4;
const HEADER_DWORDS: usize = 7;

/// `key_dwords` of a table whose keys carry their own length word.
pub const VARIABLE_KEY_DWORDS: u32 = 0xFFFF_FFFF;

/// The first key word of the entry that opens an internal page — the
/// left-most child's slot. `core.dll` compares it as smaller than any other
/// word, so an entry starting with it sorts first whatever its other words
/// hold.
pub const SENTINEL_WORD: u32 = 0x8000_0001;

/// Width of the value on every page of a `NAME`-shaped table: a RefNo on a
/// leaf, `(pgno, extno)` above it.
pub const NAMED_VALUE_DWORDS: u32 = 2;

/// The db1 hash of `NAME`, the table every design, catalogue and dictionary
/// database in the corpus carries.
pub fn name_table_hash() -> u32 {
    e3d_attlib::db1_hash("NAME")
}

/// A key as the table stores it: packed words, NUL-padded to the last word.
///
/// [`encode_name`] builds one from text; [`NamedKey::text`] reads one back.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamedKey(pub Vec<u32>);

impl NamedKey {
    /// The bytes of the words, trailing NULs dropped.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out: Vec<u8> = self.0.iter().flat_map(|w| w.to_be_bytes()).collect();
        while out.last() == Some(&0) {
            out.pop();
        }
        out
    }

    /// The key as text, its bytes taken as UTF-8. `FORMAT 1` files store
    /// UTF-8 as it lies; on an escaped-format file a non-ASCII name comes
    /// back in its `&~ … &` form — [`NamedKey::text_as`] with the header's
    /// word restores it.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes()).into_owned()
    }

    /// The key as text under the header's `FORMAT`
    /// ([`crate::record::text::decode_as`]): on an escaped-format file the
    /// `&~ … &` runs come back as the characters they hold.
    pub fn text_as(&self, format: TextFormat) -> String {
        crate::record::text::decode_as(&self.bytes(), format)
    }

    pub fn words(&self) -> &[u32] {
        &self.0
    }
}

/// Pack a name the way the table keys it: its UTF-8 bytes, four to a word,
/// big endian, NUL-padded to the word. An empty name is one word of zeros.
/// This is the key of a `FORMAT 1` file; [`encode_name_as`] keys a name
/// under whatever the header says.
pub fn encode_name(name: &str) -> NamedKey {
    key_of(name.as_bytes())
}

/// Pack a name the way the table of a file with this header `FORMAT` keys
/// it: the name stored as its text is ([`crate::record::text::encode_as`] —
/// UTF-8 under `FORMAT 1`, the escaped project code page under any other,
/// where `TES6804_0001` keys `…/45&~!cHHQ9MdM7 &:50`), then four bytes to a
/// word. The key and the record's `NAME` value are the same bytes.
pub fn encode_name_as(name: &str, format: TextFormat) -> Result<NamedKey, TextEncodeError> {
    Ok(key_of(&crate::record::text::encode_as(name, format)?))
}

fn key_of(bytes: &[u8]) -> NamedKey {
    let mut words = crate::record::text::pack_words(bytes);
    if words.is_empty() {
        words.push(0);
    }
    NamedKey(words)
}

/// Key order as `core.dll` compares it (`sub_5B00F10`, `sub_5B010F0`):
/// unsigned word by word, [`SENTINEL_WORD`] below every other word, then the
/// shorter key first.
pub fn compare_keys(a: &[u32], b: &[u32]) -> Ordering {
    for (&x, &y) in a.iter().zip(b.iter()) {
        match compare_words(x, y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    a.len().cmp(&b.len())
}

fn compare_words(x: u32, y: u32) -> Ordering {
    match (x == SENTINEL_WORD, y == SENTINEL_WORD) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => x.cmp(&y),
    }
}

/// One leaf entry: a name and the element it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedEntry {
    pub key: NamedKey,
    pub refno: RefNo,
}

impl NamedEntry {
    pub fn name(&self) -> String {
        self.key.text()
    }
}

/// Why a named table could not be read. Every rejection names the page.
#[derive(Debug, thiserror::Error)]
pub enum NamedTableError {
    #[error("this session declares no table named {name_hash}")]
    NoSuchTable { name_hash: u32 },

    #[error("{page} could not be read: {detail}")]
    Unreadable { page: PageId, detail: String },

    #[error("{page} is {len} bytes, too small for the {needed}-byte table page header")]
    PageTooSmall {
        page: PageId,
        len: usize,
        needed: usize,
    },

    #[error("{page} length {len} is not a whole number of 4-byte words")]
    PageNotDwordAligned { page: PageId, len: usize },

    #[error("{page} is page type {found}, expected {expected} for a table page")]
    UnexpectedPageType {
        page: PageId,
        found: u32,
        expected: u32,
    },

    #[error("{page} belongs to table {found}, but it was reached as a page of table {expected}")]
    WrongTable {
        page: PageId,
        found: u32,
        expected: u32,
    },

    #[error(
        "table {name_hash} states a key width of {key_dwords:#x} and a value width of \
         {value_dwords} on {page}; a name table states {VARIABLE_KEY_DWORDS:#x} and {NAMED_VALUE_DWORDS}"
    )]
    NotANamedTable {
        page: PageId,
        name_hash: u32,
        key_dwords: u32,
        value_dwords: u32,
    },

    #[error(
        "free_dwords is {free_dwords} on {page} but only {usable_dwords} dwords follow the header"
    )]
    FreeSpaceOutOfRange {
        page: PageId,
        free_dwords: u32,
        usable_dwords: u32,
    },

    #[error("{page}: the entry at word {at} states a {key_len}-word key and runs past the {used} words in use")]
    EntryOverrunsPage {
        page: PageId,
        at: usize,
        key_len: u32,
        used: usize,
    },

    #[error("{page}: the entries end at word {at} but {used} words are in use")]
    TrailingWords { page: PageId, at: usize, used: usize },

    #[error("{page} emits {found:?} after {previous:?}, so slot order is not key order")]
    KeyOutOfOrder {
        page: PageId,
        previous: NamedKey,
        found: NamedKey,
    },

    #[error("{parent} is at level {parent_level}, so its child {child} must be at level {expected}, but it is at {found}")]
    LevelNotDecreasing {
        parent: PageId,
        parent_level: u32,
        child: PageId,
        expected: u32,
        found: u32,
    },

    #[error("the tree revisits {revisited} after {after} pages")]
    Cycle { revisited: PageId, after: u32 },

    #[error("{page} is an internal page at level {level} with no entries, so the descent has nowhere to go")]
    NoChildren { page: PageId, level: u32 },

    #[error("the walk has visited {visited} pages, more than the {limit} the file holds")]
    TooManyPages { visited: u32, limit: u32 },
}

impl From<NamedTableError> for E3dReaderError {
    fn from(error: NamedTableError) -> Self {
        E3dReaderError::Other(error.to_string())
    }
}

/// One slot of a decoded page.
#[derive(Debug, Clone)]
struct Slot {
    /// `None` on the sentinel, whose first key word is [`SENTINEL_WORD`].
    key: Option<NamedKey>,
    /// The two value words: a RefNo on a leaf, `(pgno, extno)` above.
    value: [u32; 2],
}

/// A decoded page, owned so the walk can hold one frame per level.
#[derive(Debug)]
struct Node {
    level: u32,
    slots: Vec<Slot>,
}

impl Node {
    fn is_leaf(&self) -> bool {
        self.level == 0
    }

    /// The slot to descend through for `target`: the greatest key `<= target`,
    /// else the sentinel's slot, else the first slot.
    fn route(&self, target: &[u32]) -> Option<usize> {
        let mut best: Option<usize> = None;
        let mut leftmost: Option<usize> = None;
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(key) = &slot.key else {
                if leftmost.is_none() {
                    leftmost = Some(index);
                }
                continue;
            };
            if leftmost.is_none() {
                leftmost = Some(index);
            }
            if compare_keys(key.words(), target) == Ordering::Greater {
                continue;
            }
            match best {
                Some(b) => {
                    let best_key = self.slots[b].key.as_ref().expect("best is never the sentinel");
                    if compare_keys(key.words(), best_key.words()) == Ordering::Greater {
                        best = Some(index);
                    }
                }
                None => best = Some(index),
            }
        }
        best.or(leftmost)
    }

    fn children(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.iter().map(|slot| slot.value[0])
    }

    fn entries(&self) -> impl Iterator<Item = NamedEntry> + '_ {
        self.slots.iter().filter_map(|slot| {
            slot.key.as_ref().map(|key| NamedEntry {
                key: key.clone(),
                refno: RefNo::new(slot.value[0], slot.value[1]),
            })
        })
    }
}

/// Decode one page. **The only page-to-node step for named tables.**
fn decode(page: PageId, data: &[u8], name_hash: u32) -> Result<Node, NamedTableError> {
    if !data.len().is_multiple_of(DWORD) {
        return Err(NamedTableError::PageNotDwordAligned {
            page,
            len: data.len(),
        });
    }
    if data.len() < HEADER_DWORDS * DWORD {
        return Err(NamedTableError::PageTooSmall {
            page,
            len: data.len(),
            needed: HEADER_DWORDS * DWORD,
        });
    }
    let word = |index: usize| {
        u32::from_be_bytes(
            data[index * DWORD..index * DWORD + DWORD]
                .try_into()
                .expect("slice is exactly four bytes"),
        )
    };

    let page_type = word(0);
    if page_type != TABLE_PAGE_TYPE {
        return Err(NamedTableError::UnexpectedPageType {
            page,
            found: page_type,
            expected: TABLE_PAGE_TYPE,
        });
    }
    let found = word(1);
    if found != name_hash {
        return Err(NamedTableError::WrongTable {
            page,
            found,
            expected: name_hash,
        });
    }
    let level = word(2);
    let key_dwords = word(3);
    let value_dwords = word(4);
    if key_dwords != VARIABLE_KEY_DWORDS || value_dwords != NAMED_VALUE_DWORDS {
        return Err(NamedTableError::NotANamedTable {
            page,
            name_hash,
            key_dwords,
            value_dwords,
        });
    }
    let free_dwords = word(6);
    let usable_dwords = data.len() / DWORD - HEADER_DWORDS;
    if free_dwords as usize > usable_dwords {
        return Err(NamedTableError::FreeSpaceOutOfRange {
            page,
            free_dwords,
            usable_dwords: usable_dwords as u32,
        });
    }
    let used = usable_dwords - free_dwords as usize;
    let end = HEADER_DWORDS + used;

    let mut slots = Vec::new();
    let mut previous: Option<NamedKey> = None;
    let mut at = HEADER_DWORDS;
    while at < end {
        let key_len = word(at);
        let key_len_words = key_len as usize;
        if key_len == 0 || at + 1 + key_len_words + NAMED_VALUE_DWORDS as usize > end {
            return Err(NamedTableError::EntryOverrunsPage {
                page,
                at,
                key_len,
                used,
            });
        }
        let words: Vec<u32> = (0..key_len_words).map(|i| word(at + 1 + i)).collect();
        let value = [word(at + 1 + key_len_words), word(at + 2 + key_len_words)];
        // The sentinel is known by its first word alone; the words behind it
        // are whatever key it overwrote (a name never starts with 0x80).
        let sentinel = words[0] == SENTINEL_WORD;
        let key = (!sentinel).then_some(NamedKey(words));
        if let (Some(previous), Some(key)) = (&previous, &key) {
            if compare_keys(key.words(), previous.words()) == Ordering::Less {
                return Err(NamedTableError::KeyOutOfOrder {
                    page,
                    previous: previous.clone(),
                    found: key.clone(),
                });
            }
        }
        if key.is_some() {
            previous = key.clone();
        }
        slots.push(Slot { key, value });
        at += 1 + key_len_words + NAMED_VALUE_DWORDS as usize;
    }
    if at != end {
        return Err(NamedTableError::TrailingWords { page, at, used });
    }
    Ok(Node { level, slots })
}

/// One named table of one session-pinned view.
pub struct NamedTable<'a> {
    pages: &'a mut PageCache,
    root: PageId,
    name_hash: u32,
    page_limit: u32,
}

impl<'a> NamedTable<'a> {
    /// The table `name_hash` names in the view's session, or `NoSuchTable`.
    pub fn open(view: &'a mut DbView, name_hash: u32) -> Result<Self, NamedTableError> {
        let root = view
            .table_root(name_hash)
            .ok_or(NamedTableError::NoSuchTable { name_hash })?;
        Ok(Self::at_root(view.pages_mut(), root, name_hash))
    }

    /// A root the caller names — another session's table, or a freshly
    /// written one.
    pub fn at_root(pages: &'a mut PageCache, root: PageId, name_hash: u32) -> Self {
        let page_limit = pages.physical_page_count().max(1);
        Self {
            pages,
            root,
            name_hash,
            page_limit,
        }
    }

    pub fn root(&self) -> PageId {
        self.root
    }

    pub fn name_hash(&self) -> u32 {
        self.name_hash
    }

    pub fn stats(&self) -> IoStats {
        self.pages.stats()
    }

    fn node(&mut self, page: PageId) -> Result<Node, NamedTableError> {
        let data = self
            .pages
            .get(page)
            .map_err(|source| NamedTableError::Unreadable {
                page,
                detail: source.to_string(),
            })?;
        decode(page, data, self.name_hash)
    }

    /// The value stored under exactly `key`, by one descent: the tree's height
    /// in page reads, whatever the table's size.
    pub fn lookup(&mut self, key: &NamedKey) -> Result<Option<RefNo>, NamedTableError> {
        let mut page = self.root;
        let mut visited = 0u32;
        let mut parent: Option<(PageId, u32)> = None;
        loop {
            visited += 1;
            if visited > self.page_limit {
                return Err(NamedTableError::TooManyPages {
                    visited,
                    limit: self.page_limit,
                });
            }
            let node = self.node(page)?;
            if let Some((parent_page, parent_level)) = parent {
                if node.level + 1 != parent_level {
                    return Err(NamedTableError::LevelNotDecreasing {
                        parent: parent_page,
                        parent_level,
                        child: page,
                        expected: parent_level.wrapping_sub(1),
                        found: node.level,
                    });
                }
            }
            if node.is_leaf() {
                return Ok(node
                    .entries()
                    .find(|entry| entry.key == *key)
                    .map(|entry| entry.refno));
            }
            let Some(slot) = node.route(key.words()) else {
                return Err(NamedTableError::NoChildren {
                    page,
                    level: node.level,
                });
            };
            let child = node.slots[slot].value[0];
            let child_ext = node.slots[slot].value[1];
            // A stated extent names the file; 0 means "this one", as the
            // session page's addresses do. Every sampled entry states 1.
            let ext = match u16::try_from(child_ext) {
                Ok(0) | Err(_) => page.ext,
                Ok(ext) => ext,
            };
            parent = Some((page, node.level));
            page = PageId::new(ext, child);
        }
    }

    /// The element named `name`, if the table has it. Names are matched as
    /// stored — with their leading `/`, case and all — the name keyed as a
    /// `FORMAT 1` file keys it ([`encode_name`]); [`Self::lookup_name_as`]
    /// takes the header's word.
    pub fn lookup_name(&mut self, name: &str) -> Result<Option<RefNo>, NamedTableError> {
        self.lookup(&encode_name(name))
    }

    /// [`Self::lookup_name`] with the name keyed under the header's `FORMAT`
    /// ([`encode_name_as`]), so a non-ASCII name is found on an
    /// escaped-format file too. A name the format cannot store is `Ok(None)`:
    /// no table holds it.
    pub fn lookup_name_as(&mut self, name: &str, format: TextFormat) -> Result<Option<RefNo>, NamedTableError> {
        match encode_name_as(name, format) {
            Ok(key) => self.lookup(&key),
            Err(_) => Ok(None),
        }
    }

    /// Every entry in key order.
    pub fn iter(&mut self) -> NamedWalk<'_> {
        NamedWalk {
            pages: self.pages,
            name_hash: self.name_hash,
            stack: Vec::new(),
            leaf: Vec::new(),
            next_leaf_entry: 0,
            visited: 0,
            page_limit: self.page_limit,
            seen: Vec::new(),
            pending_root: Some(self.root),
            failed: false,
        }
    }
}

struct Frame {
    page: PageId,
    level: u32,
    children: Vec<u32>,
    next: usize,
}

/// Entries of one named table in key order, one leaf at a time.
pub struct NamedWalk<'a> {
    pages: &'a mut PageCache,
    name_hash: u32,
    stack: Vec<Frame>,
    leaf: Vec<NamedEntry>,
    next_leaf_entry: usize,
    visited: u32,
    page_limit: u32,
    seen: Vec<PageId>,
    pending_root: Option<PageId>,
    failed: bool,
}

impl NamedWalk<'_> {
    fn load(&mut self, page: PageId, parent: Option<(PageId, u32)>) -> Result<Node, NamedTableError> {
        self.visited += 1;
        if self.visited > self.page_limit {
            return Err(NamedTableError::TooManyPages {
                visited: self.visited,
                limit: self.page_limit,
            });
        }
        if self.seen.contains(&page) {
            return Err(NamedTableError::Cycle {
                revisited: page,
                after: self.visited - 1,
            });
        }
        self.seen.push(page);
        let data = self
            .pages
            .get(page)
            .map_err(|source| NamedTableError::Unreadable {
                page,
                detail: source.to_string(),
            })?;
        let node = decode(page, data, self.name_hash)?;
        if let Some((parent_page, parent_level)) = parent {
            if node.level + 1 != parent_level {
                return Err(NamedTableError::LevelNotDecreasing {
                    parent: parent_page,
                    parent_level,
                    child: page,
                    expected: parent_level.wrapping_sub(1),
                    found: node.level,
                });
            }
        }
        Ok(node)
    }

    /// Descend from `page` to the left-most leaf below it, pushing frames.
    fn descend(&mut self, mut page: PageId, mut parent: Option<(PageId, u32)>) -> Result<(), NamedTableError> {
        loop {
            let node = self.load(page, parent)?;
            if node.is_leaf() {
                self.leaf = node.entries().collect();
                self.next_leaf_entry = 0;
                return Ok(());
            }
            let children: Vec<u32> = node.children().collect();
            if children.is_empty() {
                return Err(NamedTableError::NoChildren {
                    page,
                    level: node.level,
                });
            }
            let first = children[0];
            self.stack.push(Frame {
                page,
                level: node.level,
                children,
                next: 1,
            });
            parent = Some((page, node.level));
            page = PageId::new(page.ext, first);
        }
    }

    fn advance(&mut self) -> Result<bool, NamedTableError> {
        while let Some(frame) = self.stack.last_mut() {
            if frame.next < frame.children.len() {
                let child = frame.children[frame.next];
                frame.next += 1;
                let parent = Some((frame.page, frame.level));
                let ext = frame.page.ext;
                self.descend(PageId::new(ext, child), parent)?;
                return Ok(true);
            }
            self.stack.pop();
        }
        Ok(false)
    }
}

impl Iterator for NamedWalk<'_> {
    type Item = Result<NamedEntry, NamedTableError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        if let Some(root) = self.pending_root.take() {
            if let Err(error) = self.descend(root, None) {
                self.failed = true;
                return Some(Err(error));
            }
        }
        loop {
            if self.next_leaf_entry < self.leaf.len() {
                let entry = self.leaf[self.next_leaf_entry].clone();
                self.next_leaf_entry += 1;
                return Some(Ok(entry));
            }
            match self.advance() {
                Ok(true) => continue,
                Ok(false) => return None,
                Err(error) => {
                    self.failed = true;
                    return Some(Err(error));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_pack_four_bytes_to_a_word_and_compare_as_bytes() {
        assert_eq!(encode_name("/F1").0, vec![0x2F46_3100]);
        assert_eq!(
            encode_name("/-NX-CSV-E01").0,
            vec![0x2F2D_4E58, 0x2D43_5356, 0x2D45_3031]
        );
        assert_eq!(encode_name("").0, vec![0]);
        assert_eq!(encode_name("/-NX-CSV-E01").text(), "/-NX-CSV-E01");
        assert_eq!(encode_name("/F1").text(), "/F1");

        let shorter = encode_name("/-NX-CSV-E01");
        let longer = encode_name("/-NX-CSV-E01-78");
        assert_eq!(compare_keys(shorter.words(), longer.words()), Ordering::Less);
        assert_eq!(compare_keys(longer.words(), shorter.words()), Ordering::Greater);
        assert_eq!(
            compare_keys(encode_name("/-CSV-E").words(), encode_name("/-CSV-S").words()),
            Ordering::Less
        );
        assert_eq!(compare_keys(shorter.words(), shorter.words()), Ordering::Equal);

        // Unsigned: a UTF-8 lead byte at the start of a word is above ASCII,
        // where a signed comparison would put it below.
        assert_eq!(
            compare_keys(encode_name("/ABC梭").words(), encode_name("/ABCD").words()),
            Ordering::Greater
        );
        // The sentinel word is below everything, whatever follows it.
        let sentinel = [SENTINEL_WORD, 0x2E38_3141, 0x2F34_3526];
        assert_eq!(compare_keys(&sentinel, encode_name("/").words()), Ordering::Less);
        assert_eq!(compare_keys(encode_name("/ZZZ").words(), &sentinel), Ordering::Greater);
        assert_eq!(compare_keys(&sentinel, &[SENTINEL_WORD]), Ordering::Greater, "same word, then length");
    }

    /// `TES6804_0001`'s level-1 page 49875: the sentinel states seven words,
    /// the six behind `0x80000001` being the tail of the name it replaced.
    #[test]
    fn a_sentinel_is_known_by_its_first_word_whatever_its_length() {
        let mut words: Vec<u32> = vec![TABLE_PAGE_TYPE, 639_374, 1, VARIABLE_KEY_DWORDS, 2, 0, 0];
        let leftover = encode_name("/xxx.81A/45&~!cHHQ9MdM7 &:50");
        let mut sentinel = leftover.0.clone();
        sentinel[0] = SENTINEL_WORD;
        assert_eq!(sentinel.len(), 7);
        words.push(sentinel.len() as u32);
        words.extend(&sentinel);
        words.extend([49_876, 1]);
        for (name, child) in [("/GD87-M-BTSE-20-39-19", 49_877), ("/GD87-M-BTSE-20-45-9", 49_878)] {
            let key = encode_name(name);
            words.push(key.0.len() as u32);
            words.extend(&key.0);
            words.extend([child, 1]);
        }
        let free = 512 - words.len();
        words[6] = free as u32;
        words.resize(512, 0);
        let page: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();

        let node = decode(PageId::primary(49_875), &page, 639_374).expect("in order once the sentinel is a sentinel");
        assert_eq!(node.level, 1);
        assert!(node.slots[0].key.is_none(), "seven words starting with the sentinel word are the sentinel");
        assert_eq!(node.entries().count(), 2);
        let children: Vec<u32> = node.children().collect();
        let route = |name: &str| node.route(encode_name(name).words()).map(|i| children[i]);
        assert_eq!(route("/GD87-M-BTSE-20-39-1"), Some(49_876), "below the first real key: the sentinel's child");
        assert_eq!(route("/GD87-M-BTSE-20-39-19"), Some(49_877));
        assert_eq!(route("/GD87-M-BTSE-20-45-9"), Some(49_878));
    }

    /// A page shaped like `ams8000_0001`'s root 16306: sentinel → 9700, two
    /// separators → 12274 and 16304.
    fn root_page() -> Vec<u8> {
        let mut words: Vec<u32> = vec![TABLE_PAGE_TYPE, 639_374, 2, VARIABLE_KEY_DWORDS, 2, 0, 0];
        let mut body: Vec<u32> = vec![1, SENTINEL_WORD, 9700, 1];
        let k1 = encode_name("/-RX-CCV-S5041-BAR-1/F1");
        body.push(k1.0.len() as u32);
        body.extend(&k1.0);
        body.extend([12_274, 1]);
        let k2 = encode_name("/1RX03-MSUP-E0049-BAR-1");
        body.push(k2.0.len() as u32);
        body.extend(&k2.0);
        body.extend([16_304, 1]);
        words.extend(&body);
        let free = 512 - words.len();
        words[6] = free as u32;
        words.resize(512, 0);
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    #[test]
    fn decodes_a_root_page_and_routes_by_greatest_key_at_or_below() {
        let node = decode(PageId::primary(16_306), &root_page(), 639_374).unwrap();
        assert_eq!(node.level, 2);
        assert_eq!(node.slots.len(), 3);
        assert!(node.slots[0].key.is_none(), "first slot is the sentinel");
        let children: Vec<u32> = node.children().collect();
        assert_eq!(children, vec![9700, 12_274, 16_304]);

        let route = |name: &str| node.route(encode_name(name).words()).map(|i| children[i]);
        assert_eq!(route("/-CSV-E-1-H-7802"), Some(9700), "below the first key: the sentinel's child");
        assert_eq!(route("/-RX-CCV-S5041-BAR-1/F1"), Some(12_274), "equal to a separator: that child");
        assert_eq!(route("/-RX-ZZZ"), Some(12_274));
        assert_eq!(route("/1RX03-MSUP-E0049-BAR-1"), Some(16_304));
        assert_eq!(route("/ZZZ"), Some(16_304));
    }

    #[test]
    fn a_page_of_another_shape_is_refused_with_the_reason() {
        let set_word = |page: &mut Vec<u8>, index: usize, word: u32| {
            page[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        };

        let mut page = root_page();
        set_word(&mut page, 1, 639_375); // another table's hash
        assert!(matches!(
            decode(PageId::primary(1), &page, 639_374),
            Err(NamedTableError::WrongTable { .. })
        ));

        let mut page = root_page();
        set_word(&mut page, 3, 2); // key_dwords = 2: a fixed-width table
        assert!(matches!(
            decode(PageId::primary(1), &page, 639_374),
            Err(NamedTableError::NotANamedTable { key_dwords: 2, .. })
        ));

        let mut page = root_page();
        set_word(&mut page, 7, 200); // first key claims 200 words
        assert!(matches!(
            decode(PageId::primary(1), &page, 639_374),
            Err(NamedTableError::EntryOverrunsPage { key_len: 200, .. })
        ));

        let mut page = root_page();
        set_word(&mut page, 6, 486); // free_dwords three short: the last entry is cut
        assert!(matches!(
            decode(PageId::primary(1), &page, 639_374),
            Err(NamedTableError::EntryOverrunsPage { .. }) | Err(NamedTableError::TrailingWords { .. })
        ));
    }
}
