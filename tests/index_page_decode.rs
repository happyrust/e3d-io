//! Regression tests for the adjudicated index page reading.
//!
//! `docs/evidence/2026-08-30-dabacon-index-page-layout-adjudication.md` settled
//! three long-running disagreements against 429 real databases. Nothing in the
//! suite could tell the readings apart before — a round-trip test passes just
//! as well when the same wrong decoder is used on both sides — so each test
//! here is built to go red if the decoder is reverted to the reading it
//! replaced, and says which one.

mod common;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use e3d_io::index::{IndexPageError, IndexPageView, INDEX_PAGE_TYPE};
use e3d_io::refno::RefNo;

const INDEX_NOUN: u32 = 13_387_743;
const MAIN_DATA_NOUN: u32 = 7_618_377;
const PAGE_DWORDS: usize = 512;
const HEADER_DWORDS: usize = 7;
const ENTRY_DWORDS: usize = 4;
const SENTINEL: u32 = 0x8000_0001;
const CAPACITY: usize = (PAGE_DWORDS - HEADER_DWORDS) / ENTRY_DWORDS;

const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const AMS7329: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7329_0001";
const AMS5100: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001";
const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

/// Lays out an index page the way core.dll does, so a test can name the bytes
/// it is judging instead of asserting against a hexdump.
struct PageBuilder {
    page_type: u32,
    noun: u32,
    level: u32,
    key_dwords: u32,
    value_dwords: u32,
    free_dwords: Option<u32>,
    /// What fills the page before the header and the live entries are written
    /// over it. `0xAB` stands in for the residue core.dll leaves behind when a
    /// slot falls out of the live region.
    fill: u8,
    entries: Vec<[u32; 4]>,
}

impl PageBuilder {
    fn at_level(level: u32) -> Self {
        Self {
            page_type: INDEX_PAGE_TYPE,
            noun: INDEX_NOUN,
            level,
            key_dwords: 2,
            value_dwords: 2,
            free_dwords: None,
            fill: 0,
            entries: Vec::new(),
        }
    }

    fn leaf() -> Self {
        Self::at_level(0)
    }

    /// A record entry: key, the page holding the record, and its start in
    /// 2-byte words from the top of that page.
    fn record(mut self, key: u32, page_no: u32, offset_words: u32) -> Self {
        self.entries
            .push([0, key, page_no, (offset_words << 12) | 1]);
        self
    }

    /// A child pointer. Its key is the least key of the subtree behind it.
    fn child(mut self, subtree_min: u32, page_no: u32) -> Self {
        self.entries.push([0, subtree_min, page_no, 1]);
        self
    }

    /// The left-most child, which carries a page-local key constant instead of
    /// a RefNo.
    fn sentinel_child(mut self, page_no: u32) -> Self {
        self.entries.push([SENTINEL, SENTINEL, page_no, 1]);
        self
    }

    fn fill(mut self, byte: u8) -> Self {
        self.fill = byte;
        self
    }

    fn page_type(mut self, page_type: u32) -> Self {
        self.page_type = page_type;
        self
    }

    fn noun(mut self, noun: u32) -> Self {
        self.noun = noun;
        self
    }

    fn widths(mut self, key_dwords: u32, value_dwords: u32) -> Self {
        self.key_dwords = key_dwords;
        self.value_dwords = value_dwords;
        self
    }

    fn free_dwords(mut self, free: u32) -> Self {
        self.free_dwords = Some(free);
        self
    }

    fn build(&self) -> Vec<u8> {
        let stride = (self.key_dwords + self.value_dwords).max(1) as usize;
        let used = self.entries.len() * stride;
        let free = self
            .free_dwords
            .unwrap_or((PAGE_DWORDS - HEADER_DWORDS - used) as u32);

        let mut page = vec![self.fill; PAGE_DWORDS * 4];
        let header = [
            self.page_type,
            self.noun,
            self.level,
            self.key_dwords,
            self.value_dwords,
            0,
            free,
        ];
        for (i, dword) in header.iter().enumerate() {
            put(&mut page, i, *dword);
        }
        for (slot, entry) in self.entries.iter().enumerate() {
            let at = HEADER_DWORDS + slot * stride;
            put(&mut page, at, entry[0]);
            put(&mut page, at + 1, entry[1]);
            put(&mut page, at + self.key_dwords as usize, entry[2]);
            put(&mut page, at + self.key_dwords as usize + 1, entry[3]);
        }
        page
    }
}

fn put(page: &mut [u8], dword_index: usize, value: u32) {
    let at = dword_index * 4;
    page[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

fn be32(page: &[u8], byte_offset: usize) -> u32 {
    u32::from_be_bytes(page[byte_offset..byte_offset + 4].try_into().unwrap())
}

fn keys(view: &IndexPageView) -> Vec<u32> {
    view.entries
        .iter()
        .filter(|entry| !entry.is_start_marker())
        .map(|entry| entry.refno.word1)
        .collect()
}

/// The reading this decoder replaced: stop at the first all-zero slot.
fn scan_to_zero_count(page: &[u8]) -> usize {
    (0..CAPACITY)
        .take_while(|slot| {
            let at = (HEADER_DWORDS + slot * ENTRY_DWORDS) * 4;
            page[at..at + ENTRY_DWORDS * 4]
                .iter()
                .any(|&byte| byte != 0)
        })
        .count()
}

/// The reading this decoder replaced: a run of keys, then a run of values.
fn soa_keys(page: &[u8], count: usize) -> Vec<(u32, u32)> {
    (0..count)
        .map(|i| {
            let at = HEADER_DWORDS * 4 + i * 8;
            (be32(page, at), be32(page, at + 4))
        })
        .collect()
}

fn ascending<T: PartialOrd>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

/// **Reverting to "scan until an all-zero slot" turns this red.**
///
/// core.dll does not clear a slot when it leaves the live region, so the bytes
/// after the last entry are whatever was written there before, not zeros. On
/// `ams8000_0001` scanning read 13 630 leaf entries where the page headers
/// declare 6605, inventing 6790 duplicate RefNos along the way.
#[test]
fn entry_count_comes_from_free_dwords_not_from_a_zero_slot() {
    let page = PageBuilder::leaf()
        .record(10, 100, 20)
        .record(20, 100, 60)
        .record(30, 101, 20)
        .fill(0xAB)
        .build();

    let view = IndexPageView::decode(&page).expect("a well-formed leaf page");

    assert_eq!(view.entries.len(), 3, "three entries are live");
    assert_eq!(keys(&view), vec![10, 20, 30]);
    assert_eq!(
        view.free_dwords as usize,
        PAGE_DWORDS - HEADER_DWORDS - 3 * ENTRY_DWORDS
    );
    assert!(
        !view
            .entries
            .iter()
            .any(|entry| entry.refno.word0 == 0xABAB_ABAB),
        "residue must not be read back as an entry"
    );

    assert_eq!(
        scan_to_zero_count(&page),
        CAPACITY,
        "the old reading would have taken the whole page; if it stops early \
         here the fixture no longer separates the two readings"
    );
}

/// **Reverting to SoA entries turns this red.**
///
/// The 2026-08-13 core.dll report read `sub_5AFFCB0` as a run of keys followed
/// by a run of values and marked that a fact. No SoA reading was clean on any
/// of the 429 sample databases: the keys come out shuffled, because the second
/// key it reads is really the first entry's value words.
#[test]
fn entries_are_aos_and_soa_reads_the_keys_out_of_order() {
    let root = PageBuilder::at_level(1)
        .sentinel_child(40)
        .child(100, 41)
        .child(200, 42)
        .fill(0xAB)
        .build();

    let view = IndexPageView::decode(&root).expect("a well-formed internal page");
    assert_eq!(keys(&view), vec![100, 200], "AoS keys ascend");
    assert_eq!(view.entries.len(), 3);
    assert_eq!(view.leftmost_child(), Some(40));

    let shuffled = soa_keys(&root, view.entries.len());
    assert!(
        !ascending(&shuffled),
        "the same bytes read as SoA must come out unsorted, else this fixture \
         cannot tell the two layouts apart; got {shuffled:?}"
    );

    // The tree those pointers describe still resolves, which SoA would break.
    let low = PageBuilder::leaf().record(10, 200, 20).fill(0xAB).build();
    let mid = PageBuilder::leaf().record(100, 201, 20).fill(0xAB).build();
    let high = PageBuilder::leaf().record(200, 202, 20).fill(0xAB).build();
    for (target, expected_child, leaf) in [
        (10, 40, &low),
        (100, 41, &mid),
        (150, 41, &mid),
        (200, 42, &high),
    ] {
        let key = RefNo::new(0, target);
        assert_eq!(
            view.select_child(key),
            Some(expected_child),
            "key {target} routes to page {expected_child}"
        );
        let leaf_view = IndexPageView::decode(leaf).expect("a well-formed leaf page");
        if target == 10 || target == 100 || target == 200 {
            assert!(
                leaf_view.find_in_leaf(key).is_some(),
                "key {target} is in the leaf the descent reached"
            );
        }
    }
}

/// **Reverting to "return an empty page" turns this red.**
///
/// A page that is not an index page and a page with no entries are different
/// answers, and a caller handed an empty entry list cannot tell which it got.
#[test]
fn a_malformed_page_is_an_error_not_an_empty_entry_list() {
    let sound = PageBuilder::leaf().record(10, 100, 20).fill(0xAB);
    assert!(IndexPageView::decode(&sound.build()).is_ok());

    let cases: Vec<(&str, Vec<u8>, IndexPageError)> = vec![
        (
            "page_type of a main data page",
            PageBuilder::leaf().record(10, 100, 20).page_type(1).build(),
            IndexPageError::UnexpectedPageType {
                found: 1,
                expected: INDEX_PAGE_TYPE,
            },
        ),
        (
            "noun of the main data table",
            PageBuilder::leaf()
                .record(10, 100, 20)
                .noun(MAIN_DATA_NOUN)
                .build(),
            IndexPageError::UnexpectedNoun {
                found: MAIN_DATA_NOUN,
                expected: INDEX_NOUN,
            },
        ),
        (
            "a key too narrow to hold a RefNo",
            PageBuilder::leaf().record(10, 100, 20).widths(1, 2).build(),
            IndexPageError::KeyTooNarrow {
                key_dwords: 1,
                needed: 2,
            },
        ),
        (
            "a value too narrow to hold page_no and packed",
            PageBuilder::leaf().record(10, 100, 20).widths(2, 1).build(),
            IndexPageError::ValueTooNarrow {
                value_dwords: 1,
                needed: 2,
            },
        ),
        (
            "more free space than the page has",
            PageBuilder::leaf()
                .record(10, 100, 20)
                .free_dwords(PAGE_DWORDS as u32)
                .build(),
            IndexPageError::FreeSpaceOutOfRange {
                free_dwords: PAGE_DWORDS as u32,
                usable_dwords: (PAGE_DWORDS - HEADER_DWORDS) as u32,
            },
        ),
        (
            // 505 dwords follow the header and an entry is 4, so a page that
            // core.dll wrote always leaves `free ≡ 1 (mod 4)` — `free_dwords(1)`
            // is a legitimately full page of 126 entries, not a malformed one.
            // Anything outside that residue class cannot be whole entries.
            "a live region that is not whole entries",
            PageBuilder::leaf()
                .record(10, 100, 20)
                .free_dwords(2)
                .build(),
            IndexPageError::FreeSpaceNotEntryAligned {
                used_dwords: (PAGE_DWORDS - HEADER_DWORDS - 2) as u64,
                stride_dwords: 4,
            },
        ),
    ];

    for (what, page, expected) in cases {
        match IndexPageView::decode(&page) {
            Ok(view) => panic!(
                "{what} decoded to {} entries instead of being rejected",
                view.entries.len()
            ),
            Err(actual) => assert_eq!(actual, expected, "{what}"),
        }
    }

    assert!(matches!(
        IndexPageView::decode(&[0u8; 16]),
        Err(IndexPageError::PageTooSmall { .. })
    ));
    assert!(matches!(
        IndexPageView::decode(&[0u8; 30]),
        Err(IndexPageError::PageNotDwordAligned { .. })
    ));
}

/// **Reverting to greatest-key routing turns this red.**
///
/// An internal key is the least key of the subtree behind it. Routing to the
/// first key `>= target` instead sends every element that opens a subtree to
/// the wrong child: 6439 of 6605 leaf entries on `ams8000_0001`.
#[test]
fn routing_takes_the_last_key_at_or_below_the_target() {
    let root = PageBuilder::at_level(1)
        .sentinel_child(40)
        .child(100, 41)
        .child(200, 42)
        .fill(0xAB)
        .build();
    let view = IndexPageView::decode(&root).expect("a well-formed internal page");

    // 100 and 200 open their subtrees, so they belong to the child they name.
    assert_eq!(view.select_child(RefNo::new(0, 100)), Some(41));
    assert_eq!(view.select_child(RefNo::new(0, 200)), Some(42));

    assert_eq!(view.select_child(RefNo::new(0, 1)), Some(40));
    assert_eq!(view.select_child(RefNo::new(0, 150)), Some(41));
    assert_eq!(view.select_child(RefNo::new(0, 9_999)), Some(42));

    // A target that *equals* a key lands on the same child under either
    // reading, so the contrast has to be drawn between the keys. There
    // greatest-key routing is wrong twice over: it sends everything below the
    // first key into that key's child instead of the sentinel's, and
    // everything between two keys one child too far right.
    for target in [RefNo::new(0, 1), RefNo::new(0, 150)] {
        assert_ne!(
            greatest_key_child(&view, target),
            view.select_child(target),
            "target {target} must separate the two readings, or it proves nothing"
        );
    }
    assert_eq!(greatest_key_child(&view, RefNo::new(0, 1)), Some(41));
    assert_eq!(greatest_key_child(&view, RefNo::new(0, 150)), Some(42));
}

/// Routing under the reading the old comments described: an entry's key bounds
/// its subtree from above, so the child to take is the one behind the first key
/// at or above the target.
fn greatest_key_child(view: &IndexPageView, target: RefNo) -> Option<u32> {
    view.entries
        .iter()
        .filter(|entry| !entry.is_start_marker())
        .find(|entry| (entry.refno.word0, entry.refno.word1) >= (target.word0, target.word1))
        .map(|entry| entry.page_no)
        .or_else(|| view.leftmost_child())
}

struct Db {
    file: File,
    page_size: usize,
    page_count: u32,
}

impl Db {
    fn open(path: &str) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut header = [0u8; 0x40];
        file.read_exact(&mut header)?;
        // 0x34 counts 4-byte words, not bytes.
        let page_size = be32(&header, 0x34) as usize * 4;
        Ok(Self {
            file,
            page_size,
            page_count: (len / page_size.max(1) as u64) as u32,
        })
    }

    fn page(&mut self, page_no: u32) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; self.page_size];
        self.file
            .seek(SeekFrom::Start(page_no as u64 * self.page_size as u64))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }
}

#[derive(Default)]
struct SplitTally {
    index_pages: usize,
    leaf_entries: usize,
    /// Per candidate split bit, how many records would start past the end of
    /// the page they name.
    past_end: [usize; 9],
    /// Per candidate split bit, how many would start off a 4-byte boundary.
    misaligned: [usize; 9],
}

const FIRST_SPLIT_BIT: u32 = 8;

/// Where `packed` splits into `offset_words` and `flag`, measured rather than
/// assumed. Records start on a 4-byte boundary inside the page they name;
/// those two facts alone leave exactly one split standing.
fn tally_splits(path: &str, tally: &mut SplitTally) {
    let mut db = Db::open(path).expect("sample database opens");
    for page_no in 0..db.page_count {
        let Ok(page) = db.page(page_no) else { continue };
        let Ok(view) = IndexPageView::decode(&page) else {
            continue;
        };
        tally.index_pages += 1;
        assert_eq!(view.page_type, INDEX_PAGE_TYPE);
        assert_eq!(view.noun, INDEX_NOUN);
        assert_eq!(view.key_dwords, 2, "page {page_no} of {path}");
        assert_eq!(view.value_dwords, 2, "page {page_no} of {path}");
        assert_eq!(
            view.entries.len(),
            (db.page_size / 4 - HEADER_DWORDS - view.free_dwords as usize) / ENTRY_DWORDS,
            "page {page_no} of {path}"
        );
        if !view.is_leaf() {
            continue;
        }
        for entry in view.entries.iter().filter(|e| !e.is_start_marker()) {
            tally.leaf_entries += 1;
            let packed = (entry.offset_words << 12) | entry.flag as u32;
            for (slot, bit) in (FIRST_SPLIT_BIT..=16).enumerate() {
                let start = (packed >> bit) as usize * 2;
                if start >= db.page_size {
                    tally.past_end[slot] += 1;
                }
                if start % 4 != 0 {
                    tally.misaligned[slot] += 1;
                }
            }
        }
    }
}

fn assert_bit_twelve_is_the_only_split(tally: &SplitTally) {
    assert!(tally.leaf_entries > 0, "no leaf entries were examined");
    for (slot, bit) in (FIRST_SPLIT_BIT..=16).enumerate() {
        let sound = tally.past_end[slot] == 0 && tally.misaligned[slot] == 0;
        assert_eq!(
            sound,
            bit == 12,
            "split at bit {bit}: {} of {} records start past the end of their \
             page and {} start off a 4-byte boundary",
            tally.past_end[slot],
            tally.leaf_entries,
            tally.misaligned[slot],
        );
    }
}

/// **Reverting the `packed` split to any other bit turns this red.**
#[test]
fn packed_splits_at_bit_twelve_on_the_adjudication_samples() {
    if common::missing(&[AMS8000, AMS7329, AMS5100]) {
        return;
    }
    let mut tally = SplitTally::default();
    for path in [AMS8000, AMS7329, AMS5100] {
        tally_splits(path, &mut tally);
    }
    assert_bit_twelve_is_the_only_split(&tally);
    println!(
        "index_pages={} leaf_entries={}",
        tally.index_pages, tally.leaf_entries
    );
}

/// The same measurement over the whole corpus the adjudication used.
#[test]
#[ignore = "reads all 429 sample databases; run with --ignored"]
fn packed_splits_at_bit_twelve_across_the_whole_corpus() {
    if common::missing(&[AMS_DIR]) {
        return;
    }
    let mut tally = SplitTally::default();
    let mut files = 0;
    for entry in std::fs::read_dir(AMS_DIR).expect("sample directory reads") {
        let entry = entry.expect("directory entry");
        let metadata = entry.metadata().expect("entry metadata");
        let name = entry.file_name().to_string_lossy().into_owned();
        let sampled = name.starts_with("ams")
            && name.len() > 5
            && name.as_bytes()[name.len() - 5] == b'_'
            && name[name.len() - 4..].bytes().all(|b| b.is_ascii_digit())
            && name[3..name.len() - 5].bytes().all(|b| b.is_ascii_digit());
        if !metadata.is_file() || !sampled {
            continue;
        }
        if metadata.len() >= 25 * 1024 * 1024 || metadata.len() <= 20 * 1024 {
            continue;
        }
        tally_splits(&entry.path().to_string_lossy(), &mut tally);
        files += 1;
    }
    assert_eq!(files, 429, "the adjudication used 429 files");
    assert_bit_twelve_is_the_only_split(&tally);
    println!(
        "files={files} index_pages={} leaf_entries={}",
        tally.index_pages, tally.leaf_entries
    );
}
