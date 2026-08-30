//! The L2 differencing contract: two roots in, four states out.
//!
//! One gate carries this file. **A diff must say exactly what two full walks
//! say.** Enumerate the base tree, enumerate the target tree, take the key-set
//! difference and compare the record addresses — that is the answer, and the
//! differ has to reach it while declining to read most of the two trees. The
//! plan's original gate was "agree with `old-pdms-io`'s `session_index_diff`
//! on the same window"; that differ counts entries by scanning to the first
//! all-zero slot, so its whole family of structural anomaly counters is
//! residue rather than data
//! (`docs/evidence/2026-08-30-dabacon-index-page-layout-adjudication.md`) and
//! it cannot be a baseline. Self-consistency can.
//!
//! The second gate holds the prune honest. [`Pruning::Enabled`] and
//! [`Pruning::Disabled`] must produce the same change list, and the pruned run
//! is held to an exact page budget: **the pages the two trees do not share,
//! and not one page they do**, counted as the symmetric difference of the two
//! page sets. Stating it that way rather than as a ratio is what makes it a
//! claim about the differ — how much a prune is worth depends on how much of
//! the tree the session rewrote, which is a fact about the file.
//!
//! Over the 429-database corpus that budget comes to 5423 pages against the
//! 14795 a full merge reads, and over the 57 pairs with a tree to prune and a
//! narrow window, 1195 against 9034.
//!
//! The synthetic cases run everywhere; the ones that need the installed E3D
//! fixtures say so and skip.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use e3d_io::index::Pruning;
use e3d_io::index::{DiffTally, IndexCursor, IndexDiff, IndexTreeError, IndexedRecord, KeyChange};
use e3d_io::page::PageId;
use e3d_io::session::DbView;

mod common;

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

/// 64 words a page, small enough to count page reads by hand.
const WORDS: u32 = 64;
const PAGE_BYTES: usize = WORDS as usize * 4;

struct Db {
    pages: Vec<Vec<u8>>,
}

impl Db {
    fn new(pages: usize, latest_session_page: u32) -> Self {
        let mut db = Self {
            pages: vec![vec![0u8; PAGE_BYTES]; pages],
        };
        db.put(0, 0x34, WORDS);
        db.put(0, 0x28, latest_session_page);
        db.put(0, 0x2C, 1);
        db.put(0, 0x30, 1);
        db
    }

    fn put(&mut self, page: u32, at: usize, value: u32) {
        self.pages[page as usize][at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    /// A session page. `previous` is the page of the session before it, or
    /// `u32::MAX` at the head of the chain.
    fn session(&mut self, page: u32, sesno: u32, index_root: u32, previous: u32) {
        self.put(page, 0x00, 3);
        self.put(page, 0x04, previous);
        self.put(page, 0x08, 1);
        self.put(page, 0x0C, sesno);
        self.put(page, 0x1C, index_root);
        self.put(page, 0x20, 1);
    }

    /// An index page. `entries` are `[key_word0, key_word1, page_no, packed]`.
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

fn child(subtree_least_key: u32, page_no: u32) -> [u32; 4] {
    [0, subtree_least_key, page_no, 1]
}

fn sentinel(page_no: u32) -> [u32; 4] {
    [SENTINEL, SENTINEL, page_no, 1]
}

fn record(key: u32, page_no: u32, offset_words: u32) -> [u32; 4] {
    [0, key, page_no, (offset_words << 12) | 1]
}

/// Two sessions over one file, sharing two of their three leaves.
///
/// ```text
/// session 7  root 2  [sentinel->3] [100->4] [200->5]
/// session 8  root 6  [sentinel->3] [100->7] [200->5]
///
/// leaf 3   10 20                    leaf 5   200 210      (shared, unread)
/// leaf 4   100@(9,60)  150@(10,20)  leaf 7   100@(11,10)  160@(11,20)
/// ```
///
/// So key 100 moved, 150 went away and 160 arrived, while the four keys under
/// leaves 3 and 5 did not change and must never be looked at.
fn shared_leaf_db(dir: &Path) -> PathBuf {
    let mut db = Db::new(12, 8);
    db.session(1, 7, 2, u32::MAX);
    db.session(8, 8, 6, 1);
    db.index(2, 1, &[sentinel(3), child(100, 4), child(200, 5)]);
    db.index(3, 0, &[record(10, 9, 20), record(20, 9, 40)]);
    db.index(4, 0, &[record(100, 9, 60), record(150, 10, 20)]);
    db.index(5, 0, &[record(200, 10, 40), record(210, 10, 60)]);
    db.index(6, 1, &[sentinel(3), child(100, 7), child(200, 5)]);
    db.index(7, 0, &[record(100, 11, 10), record(160, 11, 20)]);
    db.write(dir)
}

/// Cache capacity for a measured run: an upper bound on the pages of any
/// database in the corpus, so nothing is ever evicted and `pages_read` counts
/// distinct pages. Slots are allocated on demand, so the bound is free.
const CACHE_PAGES: usize = 16_384;

/// Run one diff on a freshly opened view, reporting what it found and what it
/// cost.
struct Run {
    changes: Vec<KeyChange>,
    tally: DiffTally,
    pages_read: u64,
}

fn run_diff(path: &Path, base: PageId, target: PageId, pruning: Pruning) -> Run {
    let mut view =
        DbView::open_at_with_cache(path, e3d_io::session::SessionSelector::Latest, CACHE_PAGES)
            .expect("the database opens");
    let before = view.stats().pages_read;
    let mut diff = IndexDiff::with_pruning(view.pages_mut(), base, target, pruning);
    let changes: Vec<KeyChange> = diff
        .by_ref()
        .collect::<Result<_, IndexTreeError>>()
        .expect("both trees walk");
    Run {
        tally: diff.tally(),
        pages_read: diff.stats().pages_read - before,
        changes,
    }
}

fn key_of(change: &KeyChange) -> u32 {
    change.refno().word1
}

#[test]
fn a_diff_names_the_changed_keys_and_leaves_the_shared_leaves_shut() {
    let dir = tempfile::tempdir().unwrap();
    let path = shared_leaf_db(dir.path());

    // The roots come off the session chain, which is the only place a root
    // comes from.
    let mut view = DbView::open(&path).unwrap();
    let sessions = view.walk_sessions().unwrap();
    assert_eq!(
        sessions.iter().map(|s| s.sesno).collect::<Vec<_>>(),
        vec![8, 7],
        "the chain runs newest first"
    );
    let base = sessions[1].index_root;
    let target = sessions[0].index_root;
    assert_eq!((base, target), (PageId::new(1, 2), PageId::new(1, 6)));
    drop(view);

    let pruned = run_diff(&path, base, target, Pruning::Enabled);
    assert_eq!(
        pruned.changes.iter().map(key_of).collect::<Vec<_>>(),
        vec![100, 150, 160],
        "changes arrive in key order"
    );
    assert!(
        matches!(pruned.changes[0], KeyChange::Modified { base, target }
            if base.loc.page == PageId::new(1, 9)
                && base.loc.offset_words == 60
                && target.loc.page == PageId::new(1, 11)
                && target.loc.offset_words == 10),
        "{:?}",
        pruned.changes[0]
    );
    assert!(matches!(pruned.changes[1], KeyChange::Deleted(record) if record.refno.word1 == 150));
    assert!(matches!(pruned.changes[2], KeyChange::Inserted(record) if record.refno.word1 == 160));

    assert_eq!(
        pruned.tally,
        DiffTally {
            deleted: 1,
            inserted: 1,
            modified: 1,
            unchanged_compared: 0,
            shared_subtrees_pruned: 2,
        },
        "leaves 3 and 5 are shared, so their four keys are unchanged unread"
    );
    assert_eq!(
        pruned.pages_read, 4,
        "the two roots and the two leaves that differ: pages 2, 6, 4 and 7"
    );

    // The same answer the long way round, which is what makes the short way
    // trustworthy.
    let walked = run_diff(&path, base, target, Pruning::Disabled);
    assert_eq!(walked.changes, pruned.changes);
    assert_eq!(
        walked.tally,
        DiffTally {
            deleted: 1,
            inserted: 1,
            modified: 1,
            unchanged_compared: 4,
            shared_subtrees_pruned: 0,
        },
        "walking finds the same four keys unchanged, one page read at a time"
    );
    assert_eq!(
        walked.pages_read, 6,
        "leaves 3 and 5 come in too: pages 2, 6, 3, 4, 7 and 5"
    );
}

/// **Reverting to "read both trees and merge" turns this red.** Two roots that
/// are the same page are the same tree, and saying so costs nothing.
#[test]
fn one_root_against_itself_reads_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = shared_leaf_db(dir.path());
    let root = PageId::new(1, 6);

    let run = run_diff(&path, root, root, Pruning::Enabled);
    assert!(run.changes.is_empty());
    assert_eq!(run.tally.shared_subtrees_pruned, 1, "the root itself");
    assert_eq!(run.pages_read, 0, "not one page of the tree was opened");
}

/// **Reverting to "a page that will not decode means those keys are gone"
/// turns this red.** A differ that swallows a bad page reports every key under
/// it as deleted, which is the most damaging wrong answer this crate can give.
#[test]
fn a_page_that_cannot_be_read_is_an_error_not_a_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::new(12, 8);
    db.session(1, 7, 2, u32::MAX);
    db.session(8, 8, 6, 1);
    db.index(2, 1, &[sentinel(3), child(100, 4)]);
    db.index(3, 0, &[record(10, 9, 20)]);
    db.index(4, 0, &[record(100, 9, 60)]);
    db.index(6, 1, &[sentinel(3), child(100, 7)]);
    // The leaf the target root names is a main data page, not an index page.
    db.put(7, 0x00, 1);
    let path = db.write(dir.path());

    let mut view = DbView::open(&path).unwrap();
    let outcome: Result<Vec<_>, _> =
        IndexDiff::new(view.pages_mut(), PageId::new(1, 2), PageId::new(1, 6)).collect();
    let error = outcome.expect_err("a broken target leaf must not read as a deletion");
    assert!(
        matches!(error, IndexTreeError::Malformed { page, .. } if page == PageId::new(1, 7)),
        "{error}"
    );
}

/// **Reverting to "merge on regardless" turns this red.** The prune bounds a
/// subtree by its least key, so a tree whose slot order is not key order would
/// have keys skipped past and reported deleted. It is refused instead.
#[test]
fn a_tree_out_of_key_order_is_refused_rather_than_merged() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::new(12, 8);
    db.session(1, 7, 2, u32::MAX);
    db.session(8, 8, 6, 1);
    db.index(2, 1, &[sentinel(3)]);
    db.index(3, 0, &[record(10, 9, 20), record(20, 9, 40)]);
    db.index(6, 1, &[sentinel(7)]);
    db.index(7, 0, &[record(30, 11, 10), record(20, 11, 20)]);
    let path = db.write(dir.path());

    let mut view = DbView::open(&path).unwrap();
    let outcome: Result<Vec<_>, _> =
        IndexDiff::new(view.pages_mut(), PageId::new(1, 2), PageId::new(1, 6)).collect();
    let error = outcome.expect_err("a leaf out of key order must not merge");
    assert!(
        matches!(error, IndexTreeError::KeyOutOfOrder { .. }),
        "{error}"
    );
}

// -------------------------------------------------------------- real files

/// Every page one tree is built from, found without the differ's help: decode
/// a node, take its children, repeat. Slot order, no routing.
fn tree_pages(view: &mut DbView, root: PageId) -> BTreeSet<PageId> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(page) = pending.pop() {
        if !seen.insert(page) {
            continue;
        }
        let bytes = view.pages_mut().get(page).expect("the page reads").to_vec();
        let node = e3d_io::index::IndexPageView::decode(&bytes).expect("the page is an index page");
        if node.is_leaf() {
            continue;
        }
        pending.extend(
            node.entries
                .iter()
                .map(|entry| PageId::new(page.ext, entry.page_no)),
        );
    }
    seen
}

/// Every key of one tree, by key, read the long way.
fn walk_root(view: &mut DbView, root: PageId, path: &str) -> BTreeMap<(u32, u32), IndexedRecord> {
    let mut out = BTreeMap::new();
    let mut cursor = IndexCursor::at_root(view.pages_mut(), root);
    for record in cursor.walk() {
        let record = record.unwrap_or_else(|error| panic!("{path}: walking {root}: {error}"));
        let previous = out.insert((record.refno.word0, record.refno.word1), record);
        assert!(
            previous.is_none(),
            "{path}: {} is enumerated twice under {root}",
            record.refno
        );
    }
    out
}

/// The answer, by definition: two full enumerations and a key-set difference.
/// Also reports how many keys the target tree holds, for scale.
fn changes_by_full_enumeration(
    view: &mut DbView,
    base_root: PageId,
    target_root: PageId,
    path: &str,
) -> (Vec<KeyChange>, usize) {
    let base = walk_root(view, base_root, path);
    let target = walk_root(view, target_root, path);
    let keys = target.len();

    let mut out: Vec<KeyChange> = Vec::new();
    for (key, record) in &base {
        match target.get(key) {
            None => out.push(KeyChange::Deleted(*record)),
            Some(newer) if newer.loc != record.loc => out.push(KeyChange::Modified {
                base: *record,
                target: *newer,
            }),
            Some(_) => {}
        }
    }
    for (key, record) in &target {
        if !base.contains_key(key) {
            out.push(KeyChange::Inserted(*record));
        }
    }
    out.sort_by_key(|change| (change.refno().word0, change.refno().word1));
    (out, keys)
}

struct Census {
    base_sesno: u32,
    target_sesno: u32,
    keys: usize,
    changes: usize,
    pruned_subtrees: u64,
    pruned_reads: u64,
    walked_reads: u64,
}

impl Census {
    /// Whether this pair can say anything about the prune: a tree with room to
    /// skip in, and a window narrow enough that most of it should be shared.
    fn measures_the_prune(&self) -> bool {
        self.walked_reads >= TREE_WORTH_PRUNING && self.changes * NARROW_WINDOW < self.keys
    }
}

/// A tree big enough for skipping to be worth anything. Below this the two
/// roots, their children and a changed leaf already are the tree, so there is
/// no headroom to measure a saving in and none to demand.
const TREE_WORTH_PRUNING: u64 = 40;

/// A window narrow enough that most of the tree ought to be shared. Two
/// consecutive sessions usually touch a handful of elements, but the chain is
/// not always consecutive — `ams5100_0001` runs 5 then 27 and changes 209 of
/// its 238 keys, and a session pair that rewrote the database shares nothing
/// to skip. Demanding a saving there would be demanding one that is not
/// available.
const NARROW_WINDOW: usize = 20;

/// Totals across a set of databases, for the claim that has to be measured
/// rather than argued: pruning reads far less than merging everything.
#[derive(Default)]
struct Totals {
    paired: usize,
    pruned_subtrees: u64,
    pruned_reads: u64,
    walked_reads: u64,
    /// The same three, over the pairs the prune is actually for: a tree worth
    /// pruning, seen through a narrow window.
    measurable: usize,
    measurable_pruned_reads: u64,
    measurable_walked_reads: u64,
}

impl Totals {
    fn add(&mut self, census: &Census) {
        self.paired += 1;
        self.pruned_subtrees += census.pruned_subtrees;
        self.pruned_reads += census.pruned_reads;
        self.walked_reads += census.walked_reads;

        if census.measures_the_prune() {
            self.measurable += 1;
            self.measurable_pruned_reads += census.pruned_reads;
            self.measurable_walked_reads += census.walked_reads;
        }
    }

    /// **What the prune is worth, in pages.**
    ///
    /// Per database `audit` already holds the exact budget — read the pages the
    /// two trees do not share and none that they do — which is the strongest
    /// form of the claim and the one that cannot be tuned. What it does not
    /// say is whether that budget is worth having, and that is not a property
    /// of the differ at all: it depends on how much of the tree the session
    /// rewrote. `ams251309_0001` rewrote 13 of its 25 leaves in a session that
    /// changed two keys, and there is nothing there to save.
    ///
    /// So the worth is claimed where it can be: over the pairs with a tree to
    /// prune and a narrow window, in total. A regression that quietly stopped
    /// pruning would fail the exact budget first; this is the number that says
    /// why anyone should care.
    fn assert_pruning_pays(&self, what: &str) {
        assert!(self.paired > 0, "{what}: no database had two sessions");
        assert!(
            self.pruned_subtrees > 0,
            "{what}: not one shared subtree was skipped across {} databases",
            self.paired
        );
        assert!(
            self.measurable > 0,
            "{what}: no pair had a tree over {TREE_WORTH_PRUNING} pages and a narrow window, \
             so nothing measured the prune"
        );
        assert!(
            self.measurable_pruned_reads * 3 <= self.measurable_walked_reads,
            "{what}: over {} pairs with something to prune, pruning read {} pages \
             and merging everything read {}",
            self.measurable,
            self.measurable_pruned_reads,
            self.measurable_walked_reads
        );
    }
}

/// Diff the newest two sessions of one database three ways and require the
/// same answer from all three.
///
/// The pair is the newest two links of the session **chain**, which is not
/// always `(N-1, N)`: `ams5100_0001` runs 5 then 27, and the chain is the
/// authority on what came before what.
///
/// `None` when the database has only ever had one session, so there is no
/// pair to compare.
fn audit(path: &str) -> Option<Census> {
    let mut view = DbView::open(path).expect("sample database opens");
    let sessions = view.walk_sessions().expect("the session chain walks");
    if sessions.len() < 2 {
        return None;
    }
    let target = sessions[0].clone();
    let base = sessions[1].clone();

    let (expected, keys) =
        changes_by_full_enumeration(&mut view, base.index_root, target.index_root, path);
    let base_pages = tree_pages(&mut view, base.index_root);
    let target_pages = tree_pages(&mut view, target.index_root);
    drop(view);

    let unshared = base_pages.symmetric_difference(&target_pages).count() as u64;
    let both = base_pages.union(&target_pages).count() as u64;

    let file = Path::new(path);
    let pruned = run_diff(file, base.index_root, target.index_root, Pruning::Enabled);
    let walked = run_diff(file, base.index_root, target.index_root, Pruning::Disabled);

    // The gate: the short way and the long way say what the definition says.
    assert_eq!(
        pruned.changes, expected,
        "{path}: sessions {} -> {}: the diff and two full walks disagree",
        base.sesno, target.sesno
    );
    assert_eq!(
        walked.changes, expected,
        "{path}: sessions {} -> {}: unpruned diff and two full walks disagree",
        base.sesno, target.sesno
    );
    assert_eq!(
        pruned.tally.deleted + pruned.tally.inserted + pruned.tally.modified,
        expected.len() as u64,
        "{path}: the tally must count what was emitted"
    );
    // **The cost gate, stated exactly rather than as a ratio.** How much a
    // prune saves is a fact about the file — `ams251309_0001` rewrote 13 of
    // its 25 leaves in a session that changed two keys, and no differ can skip
    // a page the two trees do not share. What the differ owes is the other
    // half: it must read the pages they do not share, and **not one page they
    // do**. So the budget is the symmetric difference of the two page sets,
    // exactly, and merging everything costs their union.
    assert_eq!(
        pruned.pages_read,
        unshared,
        "{path}: the two trees hold {} pages each and share all but {unshared}, \
         but pruning read {} of them",
        base_pages.len(),
        pruned.pages_read
    );
    assert_eq!(
        walked.pages_read, both,
        "{path}: merging everything must read the union of the two trees, {both} pages"
    );

    let census = Census {
        base_sesno: base.sesno,
        target_sesno: target.sesno,
        keys,
        changes: expected.len(),
        pruned_subtrees: pruned.tally.shared_subtrees_pruned,
        pruned_reads: pruned.pages_read,
        walked_reads: walked.pages_read,
    };

    // A subtree that was skipped is a page that was not read, so skipping any
    // at all has to show up here. How much it shows up by is a question for
    // the totals: see `Totals::assert_pruning_pays`.
    assert!(
        census.pruned_subtrees == 0 || census.pruned_reads < census.walked_reads,
        "{path}: {} subtrees were skipped and it read {} pages anyway, against {}",
        census.pruned_subtrees,
        census.pruned_reads,
        census.walked_reads
    );

    Some(census)
}

/// **The self-consistency gate on real databases.**
#[test]
fn a_two_root_diff_says_what_two_full_walks_say() {
    if common::missing(SAMPLES) {
        return;
    }
    let mut totals = Totals::default();
    for path in SAMPLES {
        let Some(census) = audit(path) else {
            println!("{path}: one session only, no pair to diff");
            continue;
        };
        totals.add(&census);
        println!(
            "{path}: sessions {} -> {} keys={} changes={} subtrees_pruned={} pages_read pruned={} walked={}",
            census.base_sesno,
            census.target_sesno,
            census.keys,
            census.changes,
            census.pruned_subtrees,
            census.pruned_reads,
            census.walked_reads
        );
    }
    totals.assert_pruning_pays("the sample databases");
}

/// The same audit over the corpus the page layout was adjudicated against.
#[test]
#[ignore = "diffs and twice fully enumerates the newest two sessions of 429 databases; run with --ignored"]
fn a_two_root_diff_agrees_with_two_full_walks_across_the_whole_corpus() {
    if common::missing(&[AMS_DIR]) {
        return;
    }
    let mut files = 0;
    let mut changes = 0;
    let mut totals = Totals::default();
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
        files += 1;
        let path = entry.path().to_string_lossy().into_owned();
        let Some(census) = audit(&path) else {
            continue;
        };
        changes += census.changes;
        totals.add(&census);
    }
    assert_eq!(files, 429, "the adjudication used 429 files");
    totals.assert_pruning_pays("the corpus");
    println!(
        "files={files} paired={} changes={changes} subtrees_pruned={} \
         pages_read pruned={} walked={} \
         (of those, {} pairs measure the prune: pruned={} walked={})",
        totals.paired,
        totals.pruned_subtrees,
        totals.pruned_reads,
        totals.walked_reads,
        totals.measurable,
        totals.measurable_pruned_reads,
        totals.measurable_walked_reads
    );
}
