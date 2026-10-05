//! Page 1 on write (S6b, requirement 13; Property 27): a `Transaction` raises
//! each listed bucket counter above every `word1` it issues, in place, keeping
//! the entries, the third word and the rest of the page as found — and only at
//! the commit, so a staged-but-not-committed batch leaves page 1 alone.
mod common;

use common::write_fixture::{DbCopy, AMS251142, AMS5100, AMS8000, TES023};
use e3d_attlib::db1_hash;
use e3d_io::engine::ScanTier;
use e3d_io::meta::bucket::BucketTable;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::spatial::SpatialPolicy;
use e3d_io::write::{
    write_element_session_with, ElementWrite, RecordDraft, SessionClock, SessionRequest, Transaction, TransactionError,
    WriteOptions,
};
use e3d_io::{ReadOnlyEngine, RefNo};
use proptest::prelude::*;

fn provider() -> TemplateProvider {
    TemplateProvider::open(common::DESVIR, Some(0x000B_0692)).unwrap()
}

fn request() -> SessionRequest<'static> {
    let d = SessionDate { year: 2026, month: 10, day: 5, hour: 0, minute: 0, second: 0, millisecond: 0 };
    SessionRequest { user: "e3dio", comment: "page 1 bucket test", clock: Some(SessionClock::fixed(d, d)) }
}

/// The owner `/1RX03-MSUP` sits in extent 24384, the first page-1 entry; its
/// highest `word1` in the latest index is the floor the counter must pass.
fn owner_and_max(path: &std::path::Path) -> (RefNo, u32) {
    let mut engine = ReadOnlyEngine::open(path).unwrap();
    let owner = engine.find_by_name("/1RX03-MSUP").unwrap().unwrap();
    let max = engine.indexed_refnos().unwrap().into_iter().filter(|(r, _, _)| r.word0 == owner.word0).map(|(r, _, _)| r.word1).max().unwrap();
    (owner, max)
}

fn insert(tx: &mut Transaction<'_>, owner: RefNo, name: Option<&str>) -> RefNo {
    tx.insert(owner, db1_hash("EQUI"), name, |_, _| Ok(())).unwrap()
}

/// AMS8000 page 1's first entry is extent 24384; its counter is word 4, its
/// third word (E3D's claim) word 5.
const EXTENT_24384_AT: usize = 2060;
const COUNTER_AT: usize = 2064;
const THIRD_WORD_AT: usize = 2068;

fn word_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn set_word(copy: &DbCopy, at: usize, value: u32) {
    let mut bytes = copy.current_bytes();
    assert_eq!(word_at(&bytes, EXTENT_24384_AT), 24384, "page 1 first entry is extent 24384");
    bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
    std::fs::write(&copy.path, bytes).unwrap();
}

/// The counter page 1 states for extent `word0` right now.
fn counter_of(copy: &DbCopy, word0: u32) -> u32 {
    BucketTable::parse(&page1(copy)).unwrap().next_word1(word0).unwrap()
}

/// The counter page 1 states for extent 24384 right now.
fn disk_counter(copy: &DbCopy) -> u32 {
    counter_of(copy, 24384)
}

fn page1(copy: &DbCopy) -> Vec<u8> {
    copy.current_bytes()[2048..4096].to_vec()
}

fn latest(copy: &DbCopy) -> u32 {
    DbView::open(&copy.path).unwrap().session().sesno
}

/// The four Corpus databases, and whether each is a design database; the
/// dictionary `ams5100_0001` is written with its own schema and its page 1
/// lists no extent.
const CORPUS: [(&str, bool); 4] = [(AMS8000, true), (AMS251142, true), (TES023, true), (AMS5100, false)];

fn templates_for(design: bool) -> TemplateProvider {
    if design { provider() } else { TemplateProvider::open(common::DICVIR, None).unwrap() }
}

/// Where a batch inserts on `path`: under a ZONE in an extent page 1 lists (an
/// EQUI there takes its `word1` from that counter), and under the WORL, whose
/// primary extent page 1 does not list. The dictionary has no ZONE.
fn owners(path: &std::path::Path) -> (Option<RefNo>, RefNo) {
    let listed: Vec<u32> = BucketTable::parse(&std::fs::read(path).unwrap()[2048..4096])
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.extent_word0)
        .collect();
    let (zone, world) = (db1_hash("ZONE"), db1_hash("WORL"));
    let (mut listed_zone, mut the_world) = (None, None);
    let mut engine = ReadOnlyEngine::open(path).unwrap();
    for element in engine.scan_elements(ScanTier::Header).unwrap() {
        let element = element.unwrap();
        if listed_zone.is_none() && element.noun_hash == zone && listed.contains(&element.refno.word0) {
            listed_zone = Some(element.refno);
        }
        if the_world.is_none() && element.noun_hash == world {
            the_world = Some(element.refno);
        }
    }
    (listed_zone, the_world.expect("every database has its WORL"))
}

/// Property 27, read back through `BucketTable::parse` alone: word 2 still
/// counts the entries (2 + 3n), the entries keep their order and their third
/// words, no counter goes down, and each sits above the newest index and every
/// `word1` the batch issued in its extent. No word but a counter differs.
fn assert_property_27(copy: &DbCopy, before: &[u8], issued: &[RefNo]) {
    let after = page1(copy);
    let (old, new) = (BucketTable::parse(before).unwrap(), BucketTable::parse(&after).unwrap());
    assert_eq!(word_at(&after, 8) as usize, 2 + 3 * new.entries.len(), "word 2 is 2 + 3n");
    let kept = |table: &BucketTable| table.entries.iter().map(|e| (e.extent_word0, e.extra)).collect::<Vec<_>>();
    assert_eq!(kept(&new), kept(&old), "the entries, their order and their third words are kept");
    let indexed = ReadOnlyEngine::open(&copy.path).unwrap().indexed_refnos().unwrap();
    let mut masked = after.clone();
    for (index, (was, is)) in old.entries.iter().zip(&new.entries).enumerate() {
        let extent = is.extent_word0;
        assert!(is.next_word1 >= was.next_word1, "extent {extent}: the counter went down");
        let index_max = indexed.iter().filter(|(r, _, _)| r.word0 == extent).map(|(r, _, _)| r.word1).max();
        let issued_max = issued.iter().filter(|r| r.word0 == extent).map(|r| r.word1).max();
        for max in [index_max, issued_max].into_iter().flatten() {
            assert!(is.next_word1 > max, "extent {extent}: the floor {} does not pass {max}", is.next_word1);
        }
        let at = (3 + 3 * index + 1) * 4;
        masked[at..at + 4].copy_from_slice(&was.next_word1.to_be_bytes());
    }
    assert_eq!(masked, before, "only counter words change");
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, failure_persistence: None, ..ProptestConfig::default() })]

    /// The committed counter is one past every `word1` the batch issued and
    /// never below where it started — whether the counter began behind the
    /// index (historical back-debt) or ahead of it.
    // Feature: write-path-phase-3, Property 27: 页 1 桶表写侧（仅当需求 13 第 3 条成立）
    #[test]
    fn the_counter_ends_above_every_issued_word1(delta in -5i32..24, count in 1usize..5) {
        let copy = DbCopy::of(AMS8000);
        let (owner, max) = owner_and_max(&copy.path);
        let floor = (i64::from(max) + 1 + i64::from(delta)) as u32;
        set_word(&copy, COUNTER_AT, floor);
        let before = disk_counter(&copy);
        let first = (max + 1).max(floor);

        let mut templates = provider();
        let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
        let mut issued = Vec::new();
        for _ in 0..count { issued.push(insert(&mut tx, owner, None)); }
        tx.commit(&request()).unwrap();

        let after = disk_counter(&copy);
        let highest_issued = issued.iter().map(|r| r.word1).max().unwrap();
        prop_assert_eq!(after, highest_issued + 1, "one past the highest issued word1");
        prop_assert_eq!(after, first + count as u32);
        prop_assert!(after >= before, "a counter never goes down");

        // The next transaction hands out the raised counter: no number reused.
        let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
        prop_assert_eq!(insert(&mut tx, owner, None).word1, after);
    }

    /// Property 27 on the four Corpus databases. Inserts into a listed extent
    /// raise its counter one past the last `word1` issued; inserts into the
    /// primary extent, which page 1 does not list, and a batch of renames,
    /// value edits and deletes raise nothing and leave page 1 byte-for-byte —
    /// as every batch does on the dictionary, whose page 1 lists no extent.
    // Feature: write-path-phase-3, Property 27: 页 1 桶表写侧（仅当需求 13 第 3 条成立）
    #[test]
    fn property_27_holds_on_the_four_corpus_databases(
        db in 0usize..CORPUS.len(),
        listed in 0usize..3,
        primary in 0usize..3,
        edits in any::<bool>(),
    ) {
        let (path, design) = CORPUS[db];
        let copy = DbCopy::of(path);
        let (listed_zone, world) = owners(&copy.path);
        let mut templates = templates_for(design);

        let before = page1(&copy);
        let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
        let mut issued = Vec::new();
        if let Some(zone) = listed_zone {
            for _ in 0..listed { issued.push(insert(&mut tx, zone, None)); }
        }
        let primary_noun = db1_hash(if design { "SITE" } else { "UWRL" });
        for _ in 0..primary { issued.push(tx.insert(world, primary_noun, None, |_, _| Ok(())).unwrap()); }
        tx.commit(&request()).unwrap();
        assert_property_27(&copy, &before, &issued);
        match listed_zone.filter(|_| listed > 0) {
            Some(zone) => {
                let was = BucketTable::parse(&before).unwrap().next_word1(zone.word0).unwrap();
                assert_eq!(counter_of(&copy, zone.word0), was + listed as u32, "{path}: one past the last EQUI");
            }
            None => assert_eq!(page1(&copy), before, "{path}: nothing issued in a listed extent, page 1 as found"),
        }

        if edits && !issued.is_empty() {
            let before = page1(&copy);
            let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
            tx.rename(issued[0], "/E3DIO-P27").unwrap();
            if design { tx.set(issued[0], "DESC", "Property 27").unwrap(); }
            if issued.len() > 1 { tx.delete(issued[issued.len() - 1]).unwrap(); }
            tx.commit(&request()).unwrap();
            assert_eq!(page1(&copy), before, "{path}: renames, value edits and deletes raise nothing");
            assert_property_27(&copy, &before, &[]);
        }
    }
}

// Feature: write-path-phase-3, Property 27: 页 1 桶表写侧（仅当需求 13 第 3 条成立）
#[test]
fn the_third_word_claim_is_kept_as_found() {
    let copy = DbCopy::of(AMS8000);
    let (owner, _max) = owner_and_max(&copy.path);
    // A crashed E3D session's residual claim: e3d-io must not clear it.
    set_word(&copy, THIRD_WORD_AT, 7);
    let mut templates = provider();
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    let issued = insert(&mut tx, owner, None);
    tx.commit(&request()).unwrap();
    assert_eq!(disk_counter(&copy), issued.word1 + 1, "counter still rises");
    assert_eq!(word_at(&copy.current_bytes(), THIRD_WORD_AT), 7, "the claim is left as found");
}

#[test]
fn a_dropped_staging_and_a_dry_run_leave_page_1_untouched() {
    let copy = DbCopy::of(AMS8000);
    let (owner, _max) = owner_and_max(&copy.path);
    let before = page1(&copy);
    let mut templates = provider();

    // Staged, not committed: page 1 is only written at the commit.
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    insert(&mut tx, owner, None);
    let staged = tx.stage(&request()).unwrap();
    assert_eq!(page1(&copy), before, "staging does not touch page 1");
    staged.rollback().unwrap();
    assert_eq!(page1(&copy), before, "rollback leaves page 1 as it was");
}

/// A refused batch writes nothing, page 1 included: B, opened at the base,
/// inserts into extent 24384 and clashes with A on the ZONE's `DESC`.
#[test]
fn a_refused_batch_leaves_page_1_as_found() {
    let copy = DbCopy::of(AMS8000);
    let zone = owners(&copy.path).0.expect("AMS8000 has a ZONE in a listed extent");
    let s = latest(&copy);
    let mut templates = provider();
    let mut a = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    a.set(zone, "DESC", "A's description").unwrap();
    a.commit(&request()).unwrap();
    let after_a = copy.current_bytes();

    let mut b = Transaction::open_at(&copy.path, SessionSelector::Exact(s), &mut templates)
        .unwrap()
        .with_spatial(SpatialPolicy::Off);
    insert(&mut b, zone, None);
    b.set(zone, "DESC", "B's description").unwrap();
    let refused = b.commit(&request());
    assert!(matches!(refused, Err(TransactionError::Clashes(_))), "{refused:?}");
    assert_eq!(copy.current_bytes(), after_a, "a refused commit writes nothing, page 1 included");
}

/// Staging never writes page 1, so a staging whose commit finds another
/// writer's session in place leaves it alone: the observer sees page 1 as it
/// was before each staging, and only the retried commit raises it — past the
/// other writer's number and B's, re-minted beyond it.
// Feature: write-path-phase-3, Property 27: 页 1 桶表写侧（仅当需求 13 第 3 条成立）
#[test]
fn a_moved_staging_leaves_page_1_until_the_retry_commits() {
    let copy = DbCopy::of(AMS8000);
    let zone = owners(&copy.path).0.expect("AMS8000 has a ZONE in a listed extent");
    let bytes_s = copy.current_bytes();
    let mut templates = provider();
    let a_equi = {
        let mut a = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
        let refno = insert(&mut a, zone, None);
        a.commit(&request()).unwrap();
        refno
    };
    let bytes_a = copy.current_bytes();
    std::fs::write(&copy.path, &bytes_s).unwrap();

    let mut b = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    assert_eq!(insert(&mut b, zone, None), a_equi, "B is handed the number A took");
    let mut seen = Vec::new();
    let commit = b
        .commit_observed(&request(), |_| {
            seen.push(page1(&copy));
            if seen.len() == 1 {
                std::fs::write(&copy.path, &bytes_a).unwrap();
            }
        })
        .unwrap();
    assert!(commit.retried_after_move);
    assert_eq!(seen, [bytes_s[2048..4096].to_vec(), bytes_a[2048..4096].to_vec()], "staged, not committed: page 1 as it was");
    let b_equi = RefNo::new(a_equi.word0, a_equi.word1 + 1);
    assert_eq!(commit.reminted, vec![(a_equi, b_equi)]);
    assert_eq!(disk_counter(&copy), b_equi.word1 + 1);
    assert_property_27(&copy, &bytes_a[2048..4096], &[b_equi]);
}

/// A merge commit counts the re-minted refno: A and B open at the same base of
/// TES023 and are handed the same number in extent 17419; A commits it, B's
/// merge re-mints its own past A's, and page 1 ends one past the re-minted one.
// Feature: write-path-phase-3, Property 27: 页 1 桶表写侧（仅当需求 13 第 3 条成立）
#[test]
fn a_merge_commit_raises_page_1_past_the_reminted_refno() {
    let copy = DbCopy::of(TES023);
    let zone = owners(&copy.path).0.expect("TES023 has a ZONE in its listed extent");
    let s = latest(&copy);
    let (mut for_a, mut for_b) = (provider(), provider());
    let mut a = Transaction::open(&copy.path, &mut for_a).unwrap().with_spatial(SpatialPolicy::Off);
    let mut b = Transaction::open(&copy.path, &mut for_b).unwrap().with_spatial(SpatialPolicy::Off);
    let (a_equi, b_given) = (insert(&mut a, zone, None), insert(&mut b, zone, None));
    assert_eq!(a_equi, b_given, "both handed the counter");
    a.commit(&request()).unwrap();
    let after_a = page1(&copy);
    assert_eq!(counter_of(&copy, zone.word0), a_equi.word1 + 1);

    let commit = b.commit_reporting(&request()).unwrap();
    assert_eq!(commit.merged_onto, Some(s + 1));
    let b_equi = RefNo::new(a_equi.word0, a_equi.word1 + 1);
    assert_eq!(commit.reminted, vec![(b_given, b_equi)]);
    assert_eq!(counter_of(&copy, zone.word0), b_equi.word1 + 1, "one past the re-minted EQUI");
    assert_property_27(&copy, &after_a, &[b_equi]);
}

/// Back debt left by a writer that did not raise page 1 — the low-level
/// staging API, or an e3d-io before S6b — is paid off by the next Transaction,
/// even one that issues nothing in that extent.
// Feature: write-path-phase-3, Property 27: 页 1 桶表写侧（仅当需求 13 第 3 条成立）
#[test]
fn back_debt_from_the_low_level_path_is_paid_off_by_the_next_transaction() {
    let copy = DbCopy::of(AMS251142);
    let zone = owners(&copy.path).0.expect("ams251142 has a ZONE in its listed extent");
    let counter = counter_of(&copy, zone.word0);
    let mut templates = provider();

    let past = RefNo::new(zone.word0, counter + 2);
    let template = templates.template_for(db1_hash("EQUI")).unwrap().unwrap().clone();
    let equi = RecordDraft::from_template(past, zone, &template).unwrap();
    let mut zone_draft = {
        let mut engine = ReadOnlyEngine::open(&copy.path).unwrap();
        RecordDraft::from_record_bytes(&engine.find_element(zone).unwrap().unwrap().raw_bytes).unwrap()
    };
    zone_draft.push_member(past);
    let before = page1(&copy);
    write_element_session_with(
        &copy.path,
        &request(),
        &[ElementWrite::Insert(equi), ElementWrite::Update(zone_draft)],
        &[],
        WriteOptions { templates: Some(&mut templates) },
    )
    .unwrap();
    assert_eq!(page1(&copy), before, "the low-level path leaves page 1 alone: the counter is behind the index");

    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    tx.rename(zone, "/E3DIO-BACK-DEBT").unwrap();
    tx.commit(&request()).unwrap();
    assert_eq!(counter_of(&copy, zone.word0), past.word1 + 1, "paid off: one past the low-level EQUI");
    assert_property_27(&copy, &before, &[]);
}

#[test]
fn counters_already_ahead_of_the_index_are_left_alone() {
    // A batch that issues nothing into a listed extent, on a page 1 whose
    // counters already sit above that extent's index, raises nothing: page 1
    // is not rewritten at all.
    let copy = DbCopy::of(AMS8000);
    let (_owner, max) = owner_and_max(&copy.path);
    set_word(&copy, COUNTER_AT, max + 1000);
    let before = page1(&copy);
    let mut templates = provider();
    let tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    tx.commit(&request()).unwrap();
    assert_eq!(page1(&copy), before, "page 1 is unchanged when nothing must rise");
}

#[test]
fn a_counter_raised_without_page_0_keeps_the_previous_session_and_does_not_reuse() {
    // The crash window: page 1 written, page 0 not. Simulate by raising the
    // counter in place and leaving page 0 — a reader must still see the old
    // session, and the next writer must hand out from the raised counter.
    let copy = DbCopy::of(AMS8000);
    let (owner, max) = owner_and_max(&copy.path);
    let before_session = DbView::open(&copy.path).unwrap().session().sesno;
    set_word(&copy, COUNTER_AT, max + 40);
    assert_eq!(DbView::open(&copy.path).unwrap().session().sesno, before_session, "page 0 untouched: same session");

    let mut templates = provider();
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    let issued = insert(&mut tx, owner, None);
    assert_eq!(issued.word1, max + 40, "the next number comes from the raised counter, none reused");
    tx.commit(&request()).unwrap();
    assert_eq!(disk_counter(&copy), max + 41, "counter does not go backwards");
}

#[test]
fn a_page_1_with_no_entries_is_never_written() {
    // ams5100 is a dictionary that never grew: page 1 lists no extent, so the
    // write side has nothing to raise and leaves the page as found.
    let copy = DbCopy::of(AMS5100);
    let before = page1(&copy);
    if let Ok(table) = BucketTable::parse(&before) {
        assert!(table.entries.is_empty(), "ams5100 page 1 lists no extent");
    }
    let mut templates = provider();
    let tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    tx.commit(&request()).unwrap();
    assert_eq!(page1(&copy), before, "a no-entry page 1 is left byte-for-byte");
}
