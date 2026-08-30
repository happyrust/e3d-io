//! The extent contract: a database whose tree lives in a sibling extent file
//! reads whole through the ordinary open, and a sibling that is genuinely
//! absent is still an error naming the file.
//!
//! No database of the 429-file corpus has a second extent, so everything here
//! is synthetic — these fixtures are what pins the behaviour. The address
//! semantics they encode are the crate's stated ones: a session page names its
//! index root as `(pageno, extno)`, the descended tree inherits the root's
//! extent, and record part addresses inherit the record's.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use e3d_io::record::element::ParsedElement;
use e3d_io::{ReadOnlyEngine, RefNo, ScanTier};

const INDEX_NOUN: u32 = 13_387_743;
const SENTINEL: u32 = 0x8000_0001;
const HEADER_DWORDS: u32 = 7;
const ENTRY_DWORDS: u32 = 4;

const WORDS: u32 = 64;
const PAGE_BYTES: usize = WORDS as usize * 4;

/// One extent file being written, page by page.
struct Extent {
    pages: Vec<Vec<u8>>,
}

impl Extent {
    fn new(pages: usize) -> Self {
        let mut extent = Self {
            pages: vec![vec![0u8; PAGE_BYTES]; pages],
        };
        // Every extent file states the page size in its own header.
        extent.put(0, 0x34, WORDS);
        extent
    }

    fn put(&mut self, page: u32, at: usize, value: u32) {
        self.pages[page as usize][at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    /// Page 0 of the primary: the database descriptor.
    fn descriptor(&mut self, latest_session_page: u32) {
        self.put(0, 0x28, latest_session_page);
        self.put(0, 0x2C, 1);
        self.put(0, 0x30, latest_session_page);
    }

    /// A session page whose index root may live in another extent.
    fn session(&mut self, page: u32, sesno: u32, root_page: u32, root_ext: u32) {
        self.put(page, 0x00, 3);
        self.put(page, 0x04, u32::MAX);
        self.put(page, 0x0C, sesno);
        self.put(page, 0x1C, root_page);
        self.put(page, 0x20, root_ext);
    }

    fn index(&mut self, page: u32, level: u32, entries: &[[u32; 4]]) {
        let free = WORDS - HEADER_DWORDS - entries.len() as u32 * ENTRY_DWORDS;
        for (slot, word) in [INDEX_NOUN, level, 2, 2, 0, free].iter().enumerate() {
            self.put(page, (slot + 1) * 4, *word);
        }
        self.put(page, 0x00, 5);
        for (slot, entry) in entries.iter().enumerate() {
            let at = (HEADER_DWORDS as usize + slot * ENTRY_DWORDS as usize) * 4;
            for (word, value) in entry.iter().enumerate() {
                self.put(page, at + word * 4, *value);
            }
        }
    }

    /// An eleven-word element record. `members` is a `(page_no, packed)`
    /// address or `(0, 0)` for none; the page it names inherits the record's
    /// extent, which is the crate's stated rule for record parts.
    fn element(&mut self, page: u32, byte: usize, refno: RefNo, noun: u32, members: (u32, u32)) {
        self.put(page, 0, 7);
        for (word, value) in [
            11,
            refno.word0,
            refno.word1,
            noun,
            0,
            1,
            0,
            0,
            members.0,
            members.1,
            0,
        ]
        .iter()
        .enumerate()
        {
            self.put(page, byte + word * 4, *value);
        }
    }

    /// A member block: `[kind|words][owner][continuation][payload]`.
    fn member_block(&mut self, page: u32, byte: usize, owner: RefNo, member: RefNo) {
        self.put(page, 0, 7);
        self.put(page, byte, (2u32 << 16) | 7);
        for (word, value) in [owner.word0, owner.word1, 0, 0, member.word0, member.word1]
            .iter()
            .enumerate()
        {
            self.put(page, byte + 4 + word * 4, *value);
        }
    }

    fn write(&self, dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let mut file = File::create(&path).unwrap();
        for page in &self.pages {
            file.write_all(page).unwrap();
        }
        file.flush().unwrap();
        path
    }
}

fn sentinel(page_no: u32) -> [u32; 4] {
    [SENTINEL, SENTINEL, page_no, 1]
}

fn leaf_entry(key: u32, page_no: u32, offset_words: u32) -> [u32; 4] {
    [0, key, page_no, (offset_words << 12) | 1]
}

/// `(page_no, packed)` for a record part at `byte` of `page`.
fn part_address(page: u32, byte: usize) -> (u32, u32) {
    (page, ((byte as u32 / 2) << 12) | 1)
}

/// A database of two extent files. The primary holds the descriptor and the
/// session chain; the session names an index root in extent 2, where the whole
/// tree, the element records and a member block live.
fn primary_with_tree_in_extent_two(dir: &Path) -> PathBuf {
    let mut primary = Extent::new(2);
    primary.descriptor(1);
    primary.session(1, 7, 1, 2);

    let mut second = Extent::new(5);
    second.index(1, 1, &[sentinel(2)]);
    second.index(
        2,
        0,
        &[
            leaf_entry(10, 3, 20),
            leaf_entry(20, 3, 42),
            leaf_entry(30, 3, 64),
        ],
    );
    second.element(3, 40, RefNo::new(0, 10), 0xAAA1, (0, 0));
    second.element(3, 84, RefNo::new(0, 20), 0xAAA2, (0, 0));
    second.element(3, 128, RefNo::new(0, 30), 0xBBB1, part_address(4, 4));
    second.member_block(4, 4, RefNo::new(0, 30), RefNo::new(0, 10));

    second.write(dir, "ams1000_0002");
    primary.write(dir, "ams1000_0001")
}

/// **Reverting the open-time sibling attach turns this red.** Everything the
/// database stores past its session chain is in extent 2, and no caller names
/// that file — the open finds it, and enumeration, point lookup, record
/// gathering and the scan all read across the file boundary.
#[test]
fn a_tree_the_session_places_in_extent_two_reads_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = primary_with_tree_in_extent_two(dir.path());
    let mut engine = ReadOnlyEngine::open(&path).unwrap();

    let keys: Vec<u32> = engine
        .indexed_refnos()
        .unwrap()
        .into_iter()
        .map(|(refno, _, _)| refno.word1)
        .collect();
    assert_eq!(keys, vec![10, 20, 30], "enumeration crosses the extent");

    let view = engine
        .find_element(RefNo::new(0, 20))
        .unwrap()
        .expect("the point lookup crosses the extent");
    let parsed = ParsedElement::parse(&view.raw_bytes).unwrap();
    assert_eq!(parsed.noun_hash, 0xAAA2);

    // The record whose member list is a block on yet another extent-2 page.
    let view = engine
        .find_element(RefNo::new(0, 30))
        .unwrap()
        .expect("the record with a block resolves");
    let parsed = ParsedElement::parse(&view.raw_bytes).unwrap();
    assert_eq!(
        parsed.members,
        vec![RefNo::new(0, 10)],
        "the member block arrives from the record's own extent"
    );

    let scanned: Vec<_> = engine
        .scan_elements(ScanTier::Full)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .expect("the scan crosses the extent");
    assert_eq!(scanned.len(), 3);
    assert_eq!(
        scanned[2].parsed.as_ref().unwrap().members,
        vec![RefNo::new(0, 10)]
    );
}

/// A database that *states* an extent it does not have stays a loud error:
/// the open succeeds — the primary alone answers for the session chain — and
/// the first read that needs extent 2 names the missing file.
#[test]
fn a_stated_extent_with_no_file_is_an_error_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut primary = Extent::new(2);
    primary.descriptor(1);
    primary.session(1, 7, 1, 2);
    let path = primary.write(dir.path(), "ams1000_0001");

    let mut engine = ReadOnlyEngine::open(&path).unwrap();
    let error = engine.find_element(RefNo::new(0, 10)).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("extent 2"), "{text}");
    assert!(text.contains("ams1000_0002"), "{text}");
}
