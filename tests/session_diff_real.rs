//! The session-range contract: two selectors in, the increment out.
//!
//! `session::diff` is glue over three things that were each already tested —
//! the chain walk, the two-root differ and the record reader — so what this
//! file gates is the glue: that a range names the sessions it resolved, that
//! it refuses a backwards range instead of swapping it, that every change
//! comes with the record on each side it exists on, and that "a range is the
//! net difference of its ends" holds on real files against the step-by-step
//! history (`docs/plans/2026-09-03-sesno-range-increment.md` §1.3).
//!
//! The synthetic cases run everywhere; the ones that need the installed E3D
//! fixtures say so and skip.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use e3d_io::error::{E3dReaderError, SessionError};
use e3d_io::index::IndexDiff;
use e3d_io::page::PageId;
use e3d_io::record::element::ParsedElement;
use e3d_io::refno::RefNo;
use e3d_io::session::{ChangeKind, DbView, ElementDelta, SessionDiff, SessionSelector};
use e3d_io::ReadOnlyEngine;

mod common;

const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const AMS7329: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7329_0001";
const AMS5100: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001";
const AMS7000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7000_0001";
const AMS5052: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5052_0001";
const DESVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\desvir.dat";
const ATTLIB: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat";
const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

// ---------------------------------------------------------------- synthetic

const INDEX_NOUN: u32 = 13_387_743;
const SENTINEL: u32 = 0x8000_0001;
const HEADER_DWORDS: u32 = 7;
const ENTRY_DWORDS: u32 = 4;
/// 64 words a page: small enough to lay out by hand.
const WORDS: u32 = 64;
const PAGE_BYTES: usize = WORDS as usize * 4;
/// Eleven header words and one attribute word.
const RECORD_WORDS: usize = 12;

struct Db {
    pages: Vec<Vec<u8>>,
}

impl Db {
    fn new(pages: usize, latest_session_page: u32, first_session_page: u32) -> Self {
        let mut db = Self {
            pages: vec![vec![0u8; PAGE_BYTES]; pages],
        };
        db.put(0, 0x34, WORDS);
        db.put(0, 0x28, latest_session_page);
        db.put(0, 0x2C, 1);
        db.put(0, 0x30, first_session_page);
        db
    }

    fn put(&mut self, page: u32, at: usize, value: u32) {
        self.pages[page as usize][at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

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
    }

    /// A data page holding element records. Each record is `(key, owner_word1,
    /// attribute_word)` at an offset in 2-byte words, and is eleven header
    /// words plus that one attribute word, with no tail blocks.
    fn data(&mut self, page: u32, records: &[(u32, u32, u32, u32)]) {
        self.put(page, 0x00, 7);
        for &(offset_words, key, owner, attribute) in records {
            let at = offset_words as usize * 2;
            let words = [
                RECORD_WORDS as u32,
                0,
                key,
                0x000B_0692,
                0,
                owner,
                0,
                0,
                0,
                0,
                0,
                attribute,
            ];
            for (i, word) in words.iter().enumerate() {
                self.put(page, at + i * 4, *word);
            }
        }
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

fn child(least_key: u32, page_no: u32) -> [u32; 4] {
    [0, least_key, page_no, 1]
}

fn sentinel(page_no: u32) -> [u32; 4] {
    [SENTINEL, SENTINEL, page_no, 1]
}

fn record(key: u32, page_no: u32, offset_words: u32) -> [u32; 4] {
    [0, key, page_no, (offset_words << 12) | 1]
}

/// Two sessions over one file:
///
/// ```text
/// session 7  root 2  [sentinel->3] [100->4] [200->5]
/// session 8  root 6  [sentinel->3] [100->7] [200->5]
///
/// leaf 3   10 20               (shared)        leaf 5   200 210      (shared)
/// leaf 4   100@(9,70) 120@(9,100) 150@(10,10)
/// leaf 7   100@(11,10) 120@(11,70) 160@(11,40)
/// ```
///
/// Key 100's record changed its attribute word (1000 → 2000), key 120's
/// record was copied byte for byte, 150 went away and 160 arrived. With
/// `mismatch`, key 160's entry points at key 150's record instead.
fn two_session_db(dir: &Path, mismatch: bool) -> PathBuf {
    let mut db = Db::new(12, 8, 1);
    db.session(1, 7, 2, u32::MAX);
    db.session(8, 8, 6, 1);
    db.index(2, 1, &[sentinel(3), child(100, 4), child(200, 5)]);
    db.index(3, 0, &[record(10, 9, 10), record(20, 9, 40)]);
    db.index(
        4,
        0,
        &[record(100, 9, 70), record(120, 9, 100), record(150, 10, 10)],
    );
    db.index(5, 0, &[record(200, 10, 40), record(210, 10, 70)]);
    db.index(6, 1, &[sentinel(3), child(100, 7), child(200, 5)]);
    let inserted = if mismatch {
        record(160, 10, 10)
    } else {
        record(160, 11, 40)
    };
    db.index(7, 0, &[record(100, 11, 10), record(120, 11, 70), inserted]);

    db.data(
        9,
        &[
            (10, 10, 1, 7),
            (40, 20, 1, 7),
            (70, 100, 1, 1000),
            (100, 120, 1, 5),
        ],
    );
    db.data(10, &[(10, 150, 1, 7), (40, 200, 1, 7), (70, 210, 1, 7)]);
    db.data(11, &[(10, 100, 1, 2000), (40, 160, 1, 7), (70, 120, 1, 5)]);
    db.write(dir)
}

fn keys(diff: &SessionDiff) -> Vec<u32> {
    diff.changes().iter().map(|c| c.refno().word1).collect()
}

#[test]
fn a_range_resolves_both_ends_and_says_what_the_two_roots_say() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_session_db(dir.path(), false);

    let mut view = DbView::open(&path).unwrap();
    let diff = view.diff_sessions(7, 8).unwrap();
    assert_eq!((diff.from.sesno, diff.to.sesno), (7, 8));
    assert_eq!(
        (diff.from.index_root, diff.to.index_root),
        (PageId::new(1, 2), PageId::new(1, 6))
    );
    assert_eq!(
        keys(&diff),
        vec![100, 120, 150, 160],
        "changes arrive in key order"
    );
    assert_eq!(
        (
            diff.tally().modified,
            diff.tally().deleted,
            diff.tally().inserted
        ),
        (2, 1, 1)
    );

    // The same answer as running the differ on the two roots by hand.
    let by_hand = IndexDiff::new(view.pages_mut(), PageId::new(1, 2), PageId::new(1, 6))
        .finish()
        .unwrap();
    assert_eq!(diff.index.changes, by_hand.changes);

    // Selectors, not just numbers: the watermark form snaps to 7, Latest is 8.
    let snapped = view
        .diff_sessions(SessionSelector::AtOrBefore(7), SessionSelector::Latest)
        .unwrap();
    assert_eq!((snapped.from.sesno, snapped.to.sesno), (7, 8));
    assert_eq!(snapped.index.changes, diff.index.changes);

    // And the engine reaches the same function.
    let mut engine = ReadOnlyEngine::open(&path).unwrap();
    let via_engine = engine.diff_sessions(7, 8).unwrap();
    assert_eq!(via_engine.index.changes, diff.index.changes);
}

/// **Reverting to "swap the ends" turns this red.** A caller with the order
/// wrong has every insertion and deletion backwards.
#[test]
fn a_backwards_range_is_refused_not_swapped() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_session_db(dir.path(), false);
    let mut view = DbView::open(&path).unwrap();

    let err = view.diff_sessions(8, 7).unwrap_err();
    assert!(
        matches!(
            err,
            E3dReaderError::Session(SessionError::ReversedRange { from: 8, to: 7, .. })
        ),
        "{err}"
    );
    let err = view.diff_steps(8, 7).unwrap_err();
    assert!(
        matches!(
            err,
            E3dReaderError::Session(SessionError::ReversedRange { .. })
        ),
        "{err}"
    );
}

#[test]
fn the_same_session_at_both_ends_is_an_empty_diff_that_reads_no_tree_page() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_session_db(dir.path(), false);
    let mut view = DbView::open(&path).unwrap();

    let before = view.stats().pages_read;
    let diff = view.diff_sessions(8, 8).unwrap();
    assert!(diff.is_empty());
    assert_eq!(diff.tally().shared_subtrees_pruned, 1, "the root itself");
    assert_eq!(
        view.stats().pages_read - before,
        0,
        "session 8 is cached from open, and one root against itself reads nothing"
    );
}

#[test]
fn an_absent_end_is_the_session_error_not_an_empty_diff() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_session_db(dir.path(), false);
    let mut view = DbView::open(&path).unwrap();

    let err = view.diff_sessions(6, 8).unwrap_err();
    assert!(
        matches!(
            err,
            E3dReaderError::Session(SessionError::NoSuchSession {
                requested: 6,
                nearest_newer: Some(7),
                ..
            })
        ),
        "{err}"
    );
}

#[test]
fn every_change_comes_with_the_record_on_each_side_it_exists_on() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_session_db(dir.path(), false);
    let mut view = DbView::open(&path).unwrap();
    let diff = view.diff_sessions(7, 8).unwrap();

    let elements: Vec<_> = view
        .changed_elements(&diff)
        .collect::<Result<_, _>>()
        .expect("every record reads");
    assert_eq!(elements.len(), 4);

    let modified = &elements[0];
    assert_eq!(modified.refno, RefNo::new(0, 100));
    assert_eq!(
        modified.kind,
        ChangeKind::Modified {
            relocated_only: false
        }
    );
    let before = modified.before.as_ref().expect("read at the base address");
    let after = modified.after.as_ref().expect("read at the target address");
    assert_eq!((before.page_no, after.page_no), (9, 11));
    assert_eq!(before.raw_bytes.len(), RECORD_WORDS * 4);
    assert_ne!(before.raw_bytes, after.raw_bytes);

    let relocated = &elements[1];
    assert_eq!(relocated.refno, RefNo::new(0, 120));
    assert_eq!(
        relocated.kind,
        ChangeKind::Modified {
            relocated_only: true
        },
        "same bytes at a new address is a copy, and says so"
    );

    let deleted = &elements[2];
    assert_eq!(deleted.refno, RefNo::new(0, 150));
    assert_eq!(deleted.kind, ChangeKind::Deleted);
    assert!(deleted.after.is_none());
    let gone = deleted
        .before
        .as_ref()
        .expect("the old record is still on its page");
    assert_eq!(
        ParsedElement::parse(&gone.raw_bytes).unwrap().refno,
        RefNo::new(0, 150)
    );

    let inserted = &elements[3];
    assert_eq!(inserted.refno, RefNo::new(0, 160));
    assert_eq!(inserted.kind, ChangeKind::Inserted);
    assert!(inserted.before.is_none());
    assert_eq!(inserted.record().page_no, 11);

    // The delta reads the one word that changed, and nothing for the copy.
    let delta = ElementDelta::of(modified).unwrap();
    assert_eq!(delta.implicit_words_changed.len(), 1);
    assert_eq!(delta.implicit_words_changed[0].word, 11);
    assert_eq!(
        (
            delta.implicit_words_changed[0].before,
            delta.implicit_words_changed[0].after
        ),
        (Some(1000), Some(2000))
    );
    assert!(delta.owner.is_none() && delta.members_added.is_empty());
    assert!(ElementDelta::of(relocated).unwrap().is_empty());
}

/// **Reverting to "carry the refno through unchecked" turns this red.** An
/// address that resolves to somebody else's record is a different fact from
/// an address that resolves to nothing, and neither is a change to report.
#[test]
fn a_record_that_answers_to_another_refno_is_an_error_not_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_session_db(dir.path(), true);
    let mut view = DbView::open(&path).unwrap();
    let diff = view.diff_sessions(7, 8).unwrap();

    let outcome: Vec<Result<_, _>> = view.changed_elements(&diff).collect();
    let error = outcome[3]
        .as_ref()
        .expect_err("key 160 points at key 150's record");
    assert!(
        matches!(
            error,
            E3dReaderError::IndexRecordMismatch { refno, found, .. }
                if *refno == RefNo::new(0, 160) && *found == RefNo::new(0, 150)
        ),
        "{error}"
    );
}

#[test]
fn steps_over_two_sessions_are_the_one_diff_between_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_session_db(dir.path(), false);
    let mut view = DbView::open(&path).unwrap();

    let steps = view.diff_steps(7, 8).unwrap();
    assert_eq!(steps.len(), 1);
    let range = view.diff_sessions(7, 8).unwrap();
    assert_eq!(
        (steps[0].from.sesno, steps[0].to.sesno),
        (range.from.sesno, range.to.sesno)
    );
    assert_eq!(steps[0].index.changes, range.index.changes);
    assert_eq!(steps[0].tally(), range.tally());
    assert!(view.diff_steps(8, 8).unwrap().is_empty());
}

// -------------------------------------------------------------- real files

fn key_set(diff: &SessionDiff) -> BTreeSet<(u32, u32)> {
    diff.changes()
        .iter()
        .map(|c| (c.refno().word0, c.refno().word1))
        .collect()
}

/// **A range is the net difference of its ends.** Every key the range names
/// was touched by some step, and the keys the steps touched that the range
/// does not name were created and deleted again inside it — which is what
/// makes the two functions two functions.
fn range_against_steps(path: &str) -> (usize, usize, usize) {
    let mut view = DbView::open_at_with_cache(path, SessionSelector::Latest, 16_384).unwrap();
    let chain = view.walk_sessions().unwrap();
    let (oldest, newest) = (chain.last().unwrap().sesno, chain.first().unwrap().sesno);

    let range = view.diff_sessions(oldest, newest).unwrap();
    assert_eq!((range.from.sesno, range.to.sesno), (oldest, newest));
    let steps = view.diff_steps(oldest, newest).unwrap();
    assert_eq!(
        steps.len(),
        chain.len() - 1,
        "{path}: one step per retained session pair"
    );
    for (step, pair) in steps
        .iter()
        .zip(chain.iter().rev().collect::<Vec<_>>().windows(2))
    {
        assert_eq!(
            (step.from.sesno, step.to.sesno),
            (pair[0].sesno, pair[1].sesno)
        );
    }

    let range_keys = key_set(&range);
    let step_keys: BTreeSet<(u32, u32)> = steps.iter().flat_map(key_set).collect();
    let missing: Vec<_> = range_keys.difference(&step_keys).collect();
    assert!(
        missing.is_empty(),
        "{path}: {} keys the range names were touched by no step: {:?}",
        missing.len(),
        &missing[..missing.len().min(5)]
    );
    let transient = step_keys.difference(&range_keys).count();
    (range_keys.len(), step_keys.len(), transient)
}

#[test]
fn a_range_names_a_subset_of_what_its_steps_name() {
    const SAMPLES: &[&str] = &[AMS8000, AMS7329, AMS5100, AMS7000, AMS5052];
    if common::missing(SAMPLES) {
        return;
    }
    for path in SAMPLES {
        let (range, steps, transient) = range_against_steps(path);
        println!("{path}: range keys={range} step keys={steps} transient={transient}");
    }
}

/// `ams5100_0001` keeps sessions 1..5 and 27, and 5→27 is the "upgrade
/// session" that rewrote most of a small Dictionary database. The step
/// across the hole is the range: nothing in between is retained to step to.
#[test]
fn a_step_across_a_hole_is_the_diff_of_the_sessions_either_side() {
    if common::missing(&[AMS5100]) {
        return;
    }
    let mut view = DbView::open(AMS5100).unwrap();
    let range = view.diff_sessions(5, 27).unwrap();
    assert_eq!(
        (
            range.tally().deleted,
            range.tally().inserted,
            range.tally().modified
        ),
        (0, 76, 133)
    );
    let steps = view.diff_steps(5, 27).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].index.changes, range.index.changes);

    // Every one of the 209 records reads, on both sides where it exists.
    let read = view.changed_elements(&range).count();
    assert_eq!(read, 209);
    let failures = view
        .changed_elements(&range)
        .filter(|element| element.is_err())
        .count();
    assert_eq!(failures, 0);
}

/// A deleted element's record is still at its old address: the pages of a
/// retained session are never reused. Found on the chain rather than named,
/// because the sample database is live and its sessions move.
#[test]
fn a_deleted_element_still_reads_its_whole_record() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let mut view = DbView::open_at_with_cache(AMS8000, SessionSelector::Latest, 16_384).unwrap();
    let chain = view.walk_sessions().unwrap();
    let (oldest, newest) = (chain.last().unwrap().sesno, chain.first().unwrap().sesno);
    let steps = view.diff_steps(oldest, newest).unwrap();

    let Some(step) = steps.iter().rev().find(|step| step.tally().deleted > 0) else {
        println!("{AMS8000}: no retained session deletes anything; nothing to check");
        return;
    };
    let mut deletions = 0;
    for element in view.changed_elements(step) {
        let element = element.expect("every record of the step reads");
        if element.kind != ChangeKind::Deleted {
            continue;
        }
        deletions += 1;
        let before = element.before.as_ref().expect("a deletion has a before");
        assert!(element.after.is_none());
        let parsed = ParsedElement::parse(&before.raw_bytes).expect("the old record parses whole");
        assert_eq!(parsed.refno, element.refno);
        assert_eq!(parsed.total_size, before.raw_bytes.len());
    }
    assert_eq!(deletions as u64, step.tally().deleted);
    println!(
        "{AMS8000}: sessions {} -> {}: {deletions} deleted record(s) read back whole",
        step.from.sesno, step.to.sesno
    );
}

/// `ams5052_0001` 163→189: 21 073 deleted, 3 260 inserted, 54 modified, and
/// exactly one of the 54 is byte-for-byte the same record at a new address.
/// A consumer that takes Modified for "values changed" reprocesses that one
/// for nothing; this is the number that says the flag is worth having.
#[test]
fn a_copied_record_is_modified_by_the_index_and_relocated_only_by_the_bytes() {
    if common::missing(&[AMS5052]) {
        return;
    }
    let mut engine = ReadOnlyEngine::open_with_cache(AMS5052, 16_384).unwrap();
    let diff = engine.diff_sessions(163, 189).unwrap();
    assert_eq!(
        (
            diff.tally().deleted,
            diff.tally().inserted,
            diff.tally().modified
        ),
        (21_073, 3_260, 54)
    );

    let mut relocated_only = 0;
    let mut modified = 0;
    for element in engine.changed_elements(&diff) {
        let element = element.expect("every record reads");
        if let ChangeKind::Modified {
            relocated_only: copy,
        } = element.kind
        {
            modified += 1;
            if copy {
                relocated_only += 1;
                assert!(ElementDelta::of(&element).unwrap().is_empty());
            }
        }
    }
    assert_eq!(modified, 54);
    assert_eq!(relocated_only, 1);
}

/// `ams7000_0001` 237→263 is the largest range in the sample set: 68 971
/// records rewritten by an upgrade session, every one of them read on both
/// sides through the streaming iterator.
#[test]
fn the_largest_sample_range_streams_every_record_on_both_sides() {
    if common::missing(&[AMS7000]) {
        return;
    }
    let mut view = DbView::open_at_with_cache(AMS7000, SessionSelector::Latest, 65_536).unwrap();
    let diff = view.diff_sessions(237, 263).unwrap();
    assert_eq!(
        (
            diff.tally().deleted,
            diff.tally().inserted,
            diff.tally().modified
        ),
        (0, 105, 68_971)
    );
    let mut both_sides = 0;
    for element in view.changed_elements(&diff) {
        let element = element.expect("every record reads");
        if element.before.is_some() && element.after.is_some() {
            both_sides += 1;
        }
    }
    assert_eq!(both_sides, 68_971);
}

/// The value-level reading on a real edit: the CODEX sessions of
/// `ams8000_0001` move one FTUB up 500 mm and back. Found by session comment
/// rather than by number, because the file keeps growing.
#[test]
fn a_moved_ftub_reads_as_a_500mm_change_in_pos() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let attlib = e3d_attlib::AttlibData::parse_file(ATTLIB).unwrap();
    let mut provider =
        e3d_io::record::template::TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE))
            .unwrap();

    let mut view = DbView::open_at_with_cache(AMS8000, SessionSelector::Latest, 16_384).unwrap();
    let chain = view.walk_sessions().unwrap();
    let Some(up) = chain
        .iter()
        .find(|s| s.comment.contains("up 500mm"))
        .cloned()
    else {
        println!("{AMS8000}: no 'up 500mm' session on the chain; nothing to check");
        return;
    };
    let Some(before) = up
        .previous
        .and_then(|page| chain.iter().find(|s| s.page == page))
    else {
        panic!("session {} has a predecessor on the chain", up.sesno);
    };

    let diff = view.diff_sessions(before.sesno, up.sesno).unwrap();
    let mut ftubs = 0;
    for element in view.changed_elements(&diff) {
        let element = element.unwrap();
        let mut delta = ElementDelta::of(&element).unwrap();
        if delta.noun_name.as_deref() != Some("FTUB") {
            continue;
        }
        ftubs += 1;
        delta.with_attribute_rows(&attlib, &mut provider);
        let rows = delta
            .attribute_rows
            .as_ref()
            .unwrap_or_else(|| panic!("{:?}", delta.attribute_rows_unavailable));
        let pos = rows
            .iter()
            .find(|row| row.name == "POS")
            .expect("POS is among the changed rows");
        let z = |value: &Option<e3d_io::record::descriptor::DescriptorValue>| match value {
            Some(e3d_io::record::descriptor::DescriptorValue::RealArray(xyz)) => xyz[2],
            other => panic!("POS is a real array, got {other:?}"),
        };
        assert_eq!(z(&pos.after) - z(&pos.before), 500.0);
        assert!(
            delta.implicit_words_changed.len() >= 2,
            "the z word and SPAMAP change together: {:?}",
            delta.implicit_words_changed
        );
    }
    assert_eq!(
        ftubs, 1,
        "sessions {} -> {} move one FTUB",
        before.sesno, up.sesno
    );
}

/// Every sample database diffs its newest two sessions through the range API
/// with the same answer the raw differ gives on the same two roots.
#[test]
#[ignore = "diffs the newest two sessions of every sample database twice; run with --ignored"]
fn the_range_api_agrees_with_the_differ_across_the_corpus() {
    if common::missing(&[AMS]) {
        return;
    }
    let mut files = 0;
    for entry in std::fs::read_dir(AMS).unwrap() {
        let entry = entry.unwrap();
        let metadata = entry.metadata().unwrap();
        let len = metadata.len();
        if !metadata.is_file() || len >= 25 * 1024 * 1024 || len <= 20 * 1024 {
            continue;
        }
        let path = entry.path();
        let mut view = DbView::open(&path).unwrap();
        let chain = view.walk_sessions().unwrap();
        if chain.len() < 2 {
            continue;
        }
        files += 1;
        let diff = view.diff_sessions(chain[1].sesno, chain[0].sesno).unwrap();
        let by_hand = IndexDiff::new(view.pages_mut(), chain[1].index_root, chain[0].index_root)
            .finish()
            .unwrap();
        assert_eq!(diff.index.changes, by_hand.changes, "{}", path.display());
    }
    assert!(files > 0);
    println!("{files} databases with two or more sessions agree");
}
