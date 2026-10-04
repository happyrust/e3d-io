//! The L2 contract: one descent, one routing rule, one definition of "exists".
//!
//! Two of these are gates rather than ordinary tests.
//!
//! * **Lookup and enumeration must name the same set of keys.** The reading
//!   this crate replaced could not pass it: counting entries by scanning to the
//!   first all-zero slot invented 7025 leaf entries on `ams8000_0001`, and a
//!   key that only enumeration can see is a key that reaches a caller as an
//!   element which no lookup will ever resolve.
//! * **A cold lookup must cost the height of the tree.** Measured with a
//!   one-page cache, so a page read twice is a page counted twice and the
//!   budget cannot be met by a descent that backtracks.
//!
//! The synthetic cases run everywhere; the ones that need the installed E3D
//! fixtures say so and skip.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use e3d_io::index::{IndexCursor, IndexTreeError, IndexedRecord};
use e3d_io::page::PageId;
use e3d_io::refno::RefNo;
use e3d_io::session::DbView;

mod common;

/// Mandatory allocation gate: compare original walk order and predecessor
/// with an independent scan on latest, middle and oldest retained sessions.
#[test]
fn allocation_predecessor_and_order_on_required_corpus() {
    use e3d_io::session::SessionSelector;
    for path in common::corpus::CORPUS {
        let mut latest = DbView::open(path).expect("required Corpus must be installed");
        let chain = latest.walk_sessions().expect("retained session chain");
        let mut sessions = vec![chain[0].sesno, chain[chain.len() / 2].sesno, chain.last().unwrap().sesno];
        sessions.sort_unstable();
        sessions.dedup();
        for sesno in sessions {
            let mut view = DbView::open_at(path, SessionSelector::Exact(sesno)).unwrap();
            let records = walked(&mut view);
            assert!(records.iter().all(|r| r.refno.word0 <= i32::MAX as u32 && r.refno.word1 <= i32::MAX as u32), "{path} session {sesno}: ordinary keys must be nonnegative native words");
            for pair in records.windows(2) {
                assert!(pair[0].refno < pair[1].refno, "{path} session {sesno}: {} >= {}", pair[0].refno, pair[1].refno);
            }
            let mut targets = BTreeSet::from([RefNo::new(0, 0), RefNo::new(u32::MAX, u32::MAX)]);
            let step = (records.len() / 32).max(1);
            for r in records.iter().step_by(step) {
                targets.insert(r.refno);
                if let Some(word1) = r.refno.word1.checked_add(1) { targets.insert(RefNo::new(r.refno.word0, word1)); }
            }
            for extent in records.iter().map(|r| r.refno.word0).collect::<BTreeSet<_>>() {
                if let Some(next) = extent.checked_add(1) { targets.insert(RefNo::new(next, 0)); }
            }
            let mut worst = 0;
            for target in targets {
                let expected = records.iter().rev().find(|r| r.refno < target).copied();
                let mut cold = DbView::open_at_with_cache(path, SessionSelector::Exact(sesno), 1).unwrap();
                let before = cold.stats().pages_read;
                let got = IndexCursor::new(&mut cold).last_before(target).unwrap();
                let reads = cold.stats().pages_read - before;
                assert_eq!(got, expected, "{path} session {sesno}, before {target}");
                assert!(reads <= 6, "{path}: predecessor read {reads} pages");
                worst = worst.max(reads);
            }
            println!("ALLOC-CORPUS {path} session={sesno} keys={} order_violations=0 max_predecessor_reads={worst}", records.len());
        }
    }
}

const AMS_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const AMS7329: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7329_0001";
const AMS5100: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001";
const SAMPLES: &[&str] = &[AMS8000, AMS7329, AMS5100];

const INDEX_NOUN: u32 = 13_387_743;
const SENTINEL: u32 = 0x8000_0001;
const HEADER_DWORDS: u32 = 7;
const ENTRY_DWORDS: u32 = 4;

// ---------------------------------------------------------------- synthetic

/// 64 words a page, so a page holds 14 entries and the arithmetic stays small
/// enough to check by hand.
const WORDS: u32 = 64;
const PAGE_BYTES: usize = WORDS as usize * 4;

struct Db {
    pages: Vec<Vec<u8>>,
}

impl Db {
    fn new(pages: usize) -> Self {
        let mut db = Self {
            pages: vec![vec![0u8; PAGE_BYTES]; pages],
        };
        db.put(0, 0x34, WORDS);
        db.put(0, 0x28, 1);
        db.put(0, 0x2C, 1);
        db.put(0, 0x30, 1);
        db
    }

    fn put(&mut self, page: u32, at: usize, value: u32) {
        self.pages[page as usize][at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn session(&mut self, page: u32, sesno: u32, index_root: u32) {
        self.put(page, 0x00, 3);
        self.put(page, 0x04, u32::MAX);
        self.put(page, 0x0C, sesno);
        self.put(page, 0x1C, index_root);
        self.put(page, 0x20, 1);
    }

    /// An index page. `entries` are `[key_word0, key_word1, page_no, packed]`,
    /// and the free-space word is derived, the way core.dll writes it.
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
        // Residue after the live region, which core.dll does not clear.
        let first_free = (HEADER_DWORDS as usize + entries.len() * ENTRY_DWORDS as usize) * 4;
        self.pages[page as usize][first_free..].fill(0xAB);
    }

    fn write(&self, dir: &Path) -> PathBuf {
        let path = dir.join("ams1000_0001");
        let mut file = File::create(&path).unwrap();
        for page in &self.pages {
            file.write_all(page).unwrap();
        }
        file.flush().unwrap();
        path
    }
}

fn child(subtree_min: u32, page_no: u32) -> [u32; 4] {
    [0, subtree_min, page_no, 1]
}

fn sentinel(page_no: u32) -> [u32; 4] {
    [SENTINEL, SENTINEL, page_no, 1]
}

fn record(key: u32, page_no: u32, offset_words: u32) -> [u32; 4] {
    [0, key, page_no, (offset_words << 12) | 1]
}

/// Two levels: a root over three leaves holding keys 10..=60.
///
/// ```text
///        page 2  level 1   [sentinel->3] [100->4] [200->5]
/// page 3 level 0   10  20      page 4   100 150      page 5   200 210
/// ```
fn two_level_db(dir: &Path) -> PathBuf {
    let mut db = Db::new(8);
    db.session(1, 7, 2);
    db.index(2, 1, &[sentinel(3), child(100, 4), child(200, 5)]);
    db.index(3, 0, &[record(10, 6, 20), record(20, 6, 40)]);
    db.index(4, 0, &[record(100, 6, 60), record(150, 7, 20)]);
    db.index(5, 0, &[record(200, 7, 40), record(210, 7, 60)]);
    db.write(dir)
}

fn key(word1: u32) -> RefNo {
    RefNo::new(0, word1)
}

#[test]
fn strict_predecessor_matches_a_scan_across_leaf_separators() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_level_db(dir.path());
    let mut view = DbView::open(&path).unwrap();
    let all = walked(&mut view);
    for n in 0..=250 {
        let expected = all.iter().rev().find(|record| record.refno < key(n)).copied();
        let mut cold = DbView::open_at_with_cache(&path, e3d_io::session::SessionSelector::Latest, 1).unwrap();
        let before = cold.stats().pages_read;
        assert_eq!(IndexCursor::new(&mut cold).last_before(key(n)).unwrap(), expected, "target {n}");
        assert_eq!(cold.stats().pages_read - before, 2, "one page per level for target {n}");
    }
    assert_eq!(IndexCursor::new(&mut view).last().unwrap(), all.last().copied());
}

#[test]
fn predecessor_handles_empty_leaves_sentinels_and_unsigned_extents() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::new(7);
    db.session(1, 7, 2);
    db.index(2, 1, &[sentinel(3), [u32::MAX, 0, 4, 1]]);
    db.index(3, 0, &[sentinel(6), [2, u32::MAX, 6, 1]]);
    db.index(4, 0, &[[u32::MAX, 0, 6, 1], [u32::MAX, u32::MAX, 6, 1]]);
    let path = db.write(dir.path());
    let mut view = DbView::open(&path).unwrap();
    let mut cursor = IndexCursor::new(&mut view);
    assert!(cursor.last_before(RefNo::new(0, 0)).unwrap().is_none());
    assert_eq!(cursor.last_before(RefNo::new(u32::MAX, 0)).unwrap().unwrap().refno, RefNo::new(2, u32::MAX));
    assert_eq!(cursor.last().unwrap().unwrap().refno, RefNo::new(u32::MAX, u32::MAX));
    db.index(4, 0, &[]);
    let path = db.write(dir.path());
    let mut view = DbView::open(&path).unwrap();
    assert_eq!(IndexCursor::new(&mut view).last().unwrap().unwrap().refno, RefNo::new(2, u32::MAX));
    db.index(3, 0, &[]);
    let path = db.write(dir.path());
    let mut view = DbView::open(&path).unwrap();
    assert!(IndexCursor::new(&mut view).last().unwrap().is_none());
}

#[test]
fn predecessor_rejects_bad_paths_and_unsorted_keys() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::new(5);
    db.session(1, 7, 2);
    db.index(2, 1, &[sentinel(3)]);
    db.index(3, 0, &[record(20, 4, 20), record(10, 4, 40)]);
    let path = db.write(dir.path());
    let mut view = DbView::open(&path).unwrap();
    assert!(matches!(IndexCursor::new(&mut view).last_before(key(99)), Err(IndexTreeError::KeyOutOfOrder { .. })));
    db.put(3, 0, 1);
    let path = db.write(dir.path());
    let mut view = DbView::open(&path).unwrap();
    assert!(matches!(IndexCursor::new(&mut view).last(), Err(IndexTreeError::Malformed { .. })));
    db.index(3, 1, &[sentinel(2)]);
    let path = db.write(dir.path());
    let mut view = DbView::open(&path).unwrap();
    assert!(matches!(IndexCursor::new(&mut view).last(), Err(IndexTreeError::LevelNotDecreasing { .. })));
    db.index(2, 2, &[sentinel(3)]);
    db.index(3, 1, &[sentinel(2)]);
    let path = db.write(dir.path());
    let mut view = DbView::open(&path).unwrap();
    assert!(matches!(IndexCursor::new(&mut view).last(), Err(IndexTreeError::Cycle { .. })));
    db.index(2, 1, &[]);
    let path = db.write(dir.path());
    let mut view = DbView::open(&path).unwrap();
    assert!(matches!(IndexCursor::new(&mut view).last(), Err(IndexTreeError::NoChildren { .. })));
}

proptest::proptest! {
    #[test]
    fn predecessor_matches_independent_model(
        numbers in proptest::collection::btree_set(0u32..1000, 1..61),
        target in 0u32..1100,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let keys: Vec<_> = numbers.iter().copied().collect();
        let mut db = Db::new(10);
        db.session(1, 7, 2);
        let mut children = Vec::new();
        for (i, chunk) in keys.chunks(10).enumerate() {
            let page = 3 + i as u32;
            children.push(if i == 0 { sentinel(page) } else { child(chunk[0], page) });
            db.index(page, 0, &chunk.iter().map(|n| record(*n, 9, 20)).collect::<Vec<_>>());
        }
        db.index(2, 1, &children);
        let path = db.write(dir.path());
        let mut view = DbView::open(&path).unwrap();
        let found = IndexCursor::new(&mut view).last_before(key(target)).unwrap().map(|r| r.refno.word1);
        let expected = numbers.range(..target).next_back().copied();
        proptest::prop_assert_eq!(found, expected);
    }
}

fn walked(view: &mut DbView) -> Vec<IndexedRecord> {
    IndexCursor::new(view)
        .walk()
        .collect::<Result<Vec<_>, _>>()
        .expect("the tree walks")
}

#[test]
fn a_lookup_descends_from_the_session_root_and_stops_at_the_leaf() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_level_db(dir.path());
    let mut view = DbView::open(&path).unwrap();
    assert_eq!(view.index_root(), PageId::new(1, 2));

    let mut cursor = IndexCursor::new(&mut view);
    for (target, leaf, page_no, offset_words) in [
        (10u32, 3u32, 6u32, 20u32),
        (20, 3, 6, 40),
        (100, 4, 6, 60),
        (150, 4, 7, 20),
        (200, 5, 7, 40),
        (210, 5, 7, 60),
    ] {
        let hit = cursor.seek(key(target)).expect("the tree descends");
        assert_eq!(hit.depth, 2, "root and leaf, for key {target}");
        let found = hit
            .found
            .unwrap_or_else(|| panic!("key {target} is indexed"));
        assert_eq!(found.leaf, PageId::new(1, leaf));
        assert_eq!(found.loc.page, PageId::new(1, page_no));
        assert_eq!(found.loc.offset_words, offset_words);
        assert_eq!(found.loc.byte_offset(), u64::from(offset_words) * 2);
    }

    // A key that is not there is `None` — and only a key that is not there.
    for absent in [1u32, 15, 99, 250, 9_999] {
        let hit = cursor.seek(key(absent)).expect("the tree descends");
        assert!(hit.found.is_none(), "key {absent} is not indexed");
        assert_eq!(hit.depth, 2);
    }
}

/// **The existence gate, on a tree small enough to read.**
#[test]
fn enumeration_and_lookup_name_the_same_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_level_db(dir.path());
    let mut view = DbView::open(&path).unwrap();

    let enumerated = walked(&mut view);
    assert_eq!(
        enumerated
            .iter()
            .map(|record| record.refno.word1)
            .collect::<Vec<_>>(),
        vec![10, 20, 100, 150, 200, 210],
        "the walk emits keys in key order and skips the sentinel"
    );

    let mut cursor = IndexCursor::new(&mut view);
    for record in &enumerated {
        let hit = cursor
            .seek(record.refno)
            .expect("the tree descends")
            .found
            .unwrap_or_else(|| panic!("{} is enumerated but not reachable", record.refno));
        assert_eq!(
            hit, *record,
            "the two paths disagree about {}",
            record.refno
        );
    }
}

/// **Reverting to a fallback root scan turns this red.** The lookup cost is
/// measured with a one-page cache, so a page read twice counts twice.
#[test]
fn a_cold_lookup_costs_one_page_per_level() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_level_db(dir.path());

    for target in [10u32, 150, 210, 9_999] {
        let mut view =
            DbView::open_at_with_cache(&path, e3d_io::session::SessionSelector::Latest, 1).unwrap();
        let before = view.stats().pages_read;
        let hit = IndexCursor::new(&mut view)
            .seek(key(target))
            .expect("the tree descends");
        let read = view.stats().pages_read - before;
        assert_eq!(
            read,
            u64::from(hit.depth),
            "key {target} took {read} page reads for a tree of height {}",
            hit.depth
        );
        assert_eq!(view.stats().prefetched_pages, 0, "nothing was prefetched");
    }
}

/// **Reverting to "a page that will not decode means the key is absent" turns
/// this red.**
#[test]
fn a_page_that_cannot_be_read_is_an_error_not_an_absent_key() {
    let dir = tempfile::tempdir().unwrap();

    // The leaf that would hold key 150 is a main data page, not an index page.
    let mut db = Db::new(8);
    db.session(1, 7, 2);
    db.index(2, 1, &[sentinel(3), child(100, 4)]);
    db.index(3, 0, &[record(10, 6, 20)]);
    db.put(4, 0x00, 1);
    let path = db.write(dir.path());

    let mut view = DbView::open(&path).unwrap();
    let mut cursor = IndexCursor::new(&mut view);
    assert!(cursor.seek(key(10)).unwrap().found.is_some());

    let err = cursor.seek(key(150)).unwrap_err();
    assert!(
        matches!(err, IndexTreeError::Malformed { page, .. } if page == PageId::new(1, 4)),
        "{err}"
    );
    assert!(err.to_string().contains("page 4"), "{err}");

    // The walk reports it too rather than ending early with a short, clean list.
    let walk: Result<Vec<_>, _> = IndexCursor::new(&mut view).walk().collect();
    assert!(walk.is_err(), "the walk must not truncate a broken tree");
}

/// **Reverting to an unchecked descent turns this red.** core.dll refuses the
/// same thing: "Expected page of level %d, but got level %d".
#[test]
fn a_child_at_the_wrong_level_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::new(8);
    db.session(1, 7, 2);
    db.index(2, 2, &[sentinel(3)]);
    // Level 2's child must be level 1; this one is a leaf.
    db.index(3, 0, &[record(10, 6, 20)]);
    let path = db.write(dir.path());

    let mut view = DbView::open(&path).unwrap();
    let err = IndexCursor::new(&mut view).seek(key(10)).unwrap_err();
    assert!(
        matches!(
            err,
            IndexTreeError::LevelNotDecreasing {
                parent_level: 2,
                expected: 1,
                found: 0,
                ..
            }
        ),
        "{err}"
    );
}

/// **Reverting to the old iterator's `pending.clear(); Ok(())` turns this
/// red** — a tree that loops used to come back as a short list of good
/// entries and a successful return.
#[test]
fn a_tree_that_points_back_at_itself_is_refused_by_both_paths() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::new(8);
    db.session(1, 7, 2);
    db.index(2, 2, &[sentinel(3)]);
    db.index(3, 1, &[sentinel(2)]);
    let path = db.write(dir.path());

    let mut view = DbView::open(&path).unwrap();
    let err = IndexCursor::new(&mut view).seek(key(10)).unwrap_err();
    assert!(matches!(err, IndexTreeError::Cycle { .. }), "{err}");

    let walk: Result<Vec<_>, _> = IndexCursor::new(&mut view).walk().collect();
    let err = walk.unwrap_err();
    assert!(matches!(err, IndexTreeError::Cycle { .. }), "{err}");
}

/// **Adding a second descent turns this red.**
///
/// The stack this replaces carried four of them — a production lookup with no
/// cycle guard and no level check, a differ that had both, a debug copy with a
/// hard-coded RefNo in it, and a printing one — and they disagreed about where
/// the sentinel sits. Routing lives in one function; nothing outside the index
/// module may pick a child page — save the copy-on-write writer
/// (`write/index_cow.rs`), which builds a new tree the reader will descend and
/// so must route keys by the very same rule, reusing `choose_child` rather
/// than carrying a fifth copy.
#[test]
fn choosing_a_child_has_one_implementation() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let sources = rust_sources(&root);
    assert!(sources.len() > 10, "found only {} sources", sources.len());

    let mut definitions = Vec::new();
    let mut callers = BTreeSet::new();
    let mut comparers = BTreeSet::new();
    for (path, text) in &sources {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if text.contains("pub fn choose_child") {
            definitions.push(relative.clone());
        }
        if text.contains("choose_child(") || text.contains("select_child(") {
            callers.insert(relative.clone());
        }
        if text.contains("cmp_refno(") {
            comparers.insert(relative);
        }
    }

    assert_eq!(
        definitions,
        vec!["index/mod.rs".to_string()],
        "the routing rule must be defined exactly once"
    );
    assert_eq!(
        callers.iter().cloned().collect::<Vec<_>>(),
        vec![
            "index/cursor.rs".to_string(),
            "index/mod.rs".to_string(),
            "write/index_cow.rs".to_string(),
        ],
        "only the index module and the copy-on-write writer may choose a child page"
    );
    assert_eq!(
        comparers.iter().cloned().collect::<Vec<_>>(),
        vec![
            "index/diff.rs".to_string(),
            "index/mod.rs".to_string(),
            // The claim tree is keyed by the same 2-word refno the index is,
            // so its copy-on-write writer orders keys through the same
            // `cmp_refno` rather than carrying its own.
            "write/claim_cow.rs".to_string(),
            "write/index_cow.rs".to_string(),
        ],
        "key ordering is unsigned in one place; the writers insert by that same order"
    );
}

/// **Giving the differ a descent of its own turns this red.**
///
/// Two-root differencing merges two enumerations rather than routing by key,
/// so it never calls `choose_child` and the assertion above stays at two
/// files. What it does share is the step below `choose_child`: deciding that a
/// page is a node, at which level, and reading its entries. If that step were
/// copied into `index/diff.rs`, the differ could accept a page the walk
/// rejects — and then a key present in both trees would come back deleted.
///
/// The copy-on-write writer (`write/index_cow.rs`) reads nodes too — it copies
/// the pages on the path to each edited leaf — and it does so through the same
/// `IndexNode::decode`, so a page it writes is one the walk will read back the
/// same way. It reuses the decoder rather than carrying its own.
#[test]
fn reading_a_page_as_a_node_has_one_implementation() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let sources = rust_sources(&root);

    let mut definitions = Vec::new();
    let mut callers = BTreeSet::new();
    let mut decoders = BTreeSet::new();
    for (path, text) in &sources {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if text.contains("fn read_node") {
            definitions.push(relative.clone());
        }
        if text.contains("read_node(") {
            callers.insert(relative.clone());
        }
        if text.contains("IndexNode::decode(") {
            decoders.insert(relative);
        }
    }

    assert_eq!(
        definitions,
        vec!["index/cursor.rs".to_string()],
        "the page-to-node step must be defined exactly once"
    );
    assert_eq!(
        callers.iter().cloned().collect::<Vec<_>>(),
        vec!["index/cursor.rs".to_string(), "index/diff.rs".to_string()],
        "lookup, enumeration and differencing all read a node the same way"
    );
    assert_eq!(
        decoders.iter().cloned().collect::<Vec<_>>(),
        vec![
            "index/cursor.rs".to_string(),
            "index/mod.rs".to_string(),
            "write/index_cow.rs".to_string(),
        ],
        "nothing outside the decoder, the tree walk and the copy-on-write writer may turn a page into a node"
    );
}

fn rust_sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("source directory reads") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).expect("source reads");
                out.push((path, text));
            }
        }
    }
    out
}

// -------------------------------------------------------------- real files

struct Census {
    keys: usize,
    height: u32,
    leaves: usize,
}

/// Walk one database, then look every enumerated key back up.
fn audit(path: &str) -> Census {
    let mut view = DbView::open(path).expect("sample database opens");
    let sesno = view.selected_session();

    let mut enumerated: BTreeMap<(u32, u32), IndexedRecord> = BTreeMap::new();
    let mut leaves = BTreeSet::new();
    let mut order = Vec::new();
    for record in IndexCursor::new(&mut view).walk() {
        let record = record.unwrap_or_else(|error| panic!("{path} session {sesno}: {error}"));
        leaves.insert(record.leaf);
        order.push((record.refno.word0, record.refno.word1));
        let previous = enumerated.insert((record.refno.word0, record.refno.word1), record);
        assert!(
            previous.is_none(),
            "{path}: {} is enumerated twice; the entry count is being read wrong",
            record.refno
        );
    }
    assert!(!enumerated.is_empty(), "{path} indexes nothing");

    let mut ascending = order.clone();
    ascending.sort_unstable();
    assert_eq!(
        order, ascending,
        "{path}: the walk must emit keys in key order, so slot order is key order"
    );

    // The gate: every enumerated key is reachable, and reaches the same record.
    let mut height = 0;
    let mut cursor = IndexCursor::new(&mut view);
    for record in enumerated.values() {
        let hit = cursor
            .seek(record.refno)
            .unwrap_or_else(|error| panic!("{path}: seeking {} failed: {error}", record.refno));
        height = height.max(hit.depth);
        let found = hit.found.unwrap_or_else(|| {
            panic!(
                "{path}: {} is enumerated from {} but no lookup reaches it",
                record.refno, record.leaf
            )
        });
        assert_eq!(
            found, *record,
            "{path}: the two paths disagree about {}",
            record.refno
        );
    }

    // And the other direction: keys the walk never emitted are not reachable.
    // Neighbours of real keys are the interesting ones — they land in the same
    // leaf, so a lookup that answered from the wrong slot would be caught here.
    let mut probed = 0;
    for (word0, word1) in enumerated.keys().copied() {
        for delta in [1u32, 2, 7] {
            let absent = RefNo::new(word0, word1.wrapping_add(delta));
            if enumerated.contains_key(&(absent.word0, absent.word1)) {
                continue;
            }
            probed += 1;
            let hit = cursor.seek(absent).expect("the tree descends");
            assert!(
                hit.found.is_none(),
                "{path}: {absent} is not indexed but a lookup answered with {:?}",
                hit.found
            );
        }
    }
    assert!(probed > 0, "{path}: no absent key was probed");

    Census {
        keys: enumerated.len(),
        height,
        leaves: leaves.len(),
    }
}

/// **The existence gate on real databases.**
#[test]
fn lookup_and_enumeration_agree_on_the_sample_databases() {
    if common::missing(SAMPLES) {
        return;
    }
    for path in SAMPLES {
        let census = audit(path);
        println!(
            "{path}: keys={} leaves={} height={}",
            census.keys, census.leaves, census.height
        );
    }
}

/// **The I/O budget gate on a real tree.** A fresh view with a one-page cache
/// per lookup, so every page the descent touches is a physical read.
#[test]
fn a_cold_lookup_on_a_real_tree_costs_its_height() {
    if common::missing(SAMPLES) {
        return;
    }
    for path in SAMPLES {
        let mut view = DbView::open(path).expect("sample database opens");
        let sample: Vec<RefNo> = IndexCursor::new(&mut view)
            .walk()
            .take(4_000)
            .map(|record| record.expect("the tree walks").refno)
            .collect();
        let file_pages = u64::from(view.pages().physical_page_count());
        drop(view);

        /// A 2048-byte page holds 126 entries, so a tree of this height
        /// addresses 126^6 keys. A lookup deeper than that is not descending a
        /// B+tree, whatever the file size — which is the point: the cost must
        /// not track how big the database is.
        const DEEPEST_PLAUSIBLE_TREE: u64 = 6;

        let mut worst = 0u64;
        for refno in sample.iter().step_by(97) {
            let mut view =
                DbView::open_at_with_cache(path, e3d_io::session::SessionSelector::Latest, 1)
                    .expect("sample database opens");
            let before = view.stats().pages_read;
            let hit = IndexCursor::new(&mut view)
                .seek(*refno)
                .expect("the tree descends");
            let read = view.stats().pages_read - before;
            assert!(hit.found.is_some(), "{path}: {refno} went missing");
            assert_eq!(
                read,
                u64::from(hit.depth),
                "{path}: {refno} read {read} pages for a tree of height {}",
                hit.depth
            );
            worst = worst.max(read);
        }
        assert!(
            worst <= DEEPEST_PLAUSIBLE_TREE,
            "{path}: a lookup reading {worst} of {file_pages} pages is not a tree descent"
        );
        println!("{path}: worst lookup read {worst} pages of {file_pages}");
    }
}

/// The same audit over the corpus the page layout was adjudicated against.
#[test]
#[ignore = "walks and re-seeks every key of 429 databases; run with --ignored"]
fn lookup_and_enumeration_agree_across_the_whole_corpus() {
    if common::missing(&[AMS_DIR]) {
        return;
    }
    let mut files = 0;
    let mut keys = 0;
    let mut tallest = 0;
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
        let census = audit(&entry.path().to_string_lossy());
        files += 1;
        keys += census.keys;
        tallest = tallest.max(census.height);
    }
    assert_eq!(files, 429, "the adjudication used 429 files");
    println!("files={files} keys={keys} tallest_tree={tallest}");
}
