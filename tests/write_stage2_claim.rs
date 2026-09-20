//! The claim tree — the `(5, 7618377)` primary-element session index — under
//! the element write path (`docs/plans/2026-09-20-dabacon-write-plan.md` §7
//! CLAIM), on `ams8000_0001`. When a write is given the schema, the tree
//! follows the way `core.dll`'s `sub_5B0C760` keeps it: a new primary gets a
//! claim `(0, new_session)`, a deleted primary loses it, a touched primary (or
//! the primary above a touched non-primary) has its session bumped, and a
//! non-primary never becomes a key. Everything is read back through
//! `index::claim::ClaimTable`; a write without the schema carries the tree over
//! untouched.

mod common;

use std::collections::BTreeMap;

use common::write_fixture::{assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS8000};
use common::DESVIR;
use e3d_attlib::db1_hash;
use e3d_io::engine::ScanTier;
use e3d_io::index::ClaimTable;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionDate};
use e3d_io::write::{
    write_element_session, write_element_session_with, ElementWrite, RecordDraft, SessionClock, SessionRequest,
    WriteOptions,
};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;
const PAGE0_COMMIT_WORDS: &[usize] = &[9, 10, 11];

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 20, hour: 23, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 20, hour: 15, minute: 0, second: 0, millisecond: 0 },
    )
}

fn templates() -> TemplateProvider {
    TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir")
}

/// The claim tree as `refno -> (owner, session)`.
type ClaimMap = BTreeMap<(u32, u32), (u32, u32)>;

/// The whole claim tree, and its root page.
fn claims(path: &std::path::Path) -> (ClaimMap, u32) {
    let mut view = DbView::open(path).expect("open");
    let root = view.session().claim.page;
    let mut table = ClaimTable::open(&mut view);
    let map = table
        .iter()
        .map(|entry| {
            let entry = entry.expect("the claim tree walks");
            ((entry.refno.word0, entry.refno.word1), (entry.owner, entry.session))
        })
        .collect();
    (map, root)
}

fn draft_of(engine: &mut ReadOnlyEngine, refno: RefNo) -> RecordDraft {
    let view = engine.find_element(refno).expect("lookup").unwrap_or_else(|| panic!("{refno} is indexed"));
    RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft")
}

/// The first element of `noun` and the largest word1 in use.
fn survey(engine: &mut ReadOnlyEngine, noun: &str) -> (RefNo, u32) {
    let wanted = db1_hash(noun);
    let mut first = None;
    let mut max_word1 = 0;
    for element in engine.scan_elements(ScanTier::Header).expect("scan") {
        let element = element.expect("header");
        max_word1 = max_word1.max(element.refno.word1);
        if first.is_none() && element.noun_hash == wanted {
            first = Some(element.refno);
        }
    }
    (first.unwrap_or_else(|| panic!("the sample has a {noun}")), max_word1)
}

fn key(refno: RefNo) -> (u32, u32) {
    (refno.word0, refno.word1)
}

fn assert_bytes(before: &[u8], copy: &DbCopy, report: &e3d_io::write::CommitReport) {
    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(before, 2048, 0, 13) as usize;
    let old_last = report.previous.end.page as usize;
    let new_end = *report.appended_pages.iter().max().unwrap() as usize;
    assert_prefix_unchanged(before, &bytes, page_bytes, old_last, PAGE0_COMMIT_WORDS);
    assert_appended_contiguous(&bytes, page_bytes, old_last, new_end, &[3, 5, 7]);
}

/// A new `ZONE` (primary) under a `SITE`, with a `BOX` (not primary) under it:
/// the ZONE gains a claim `(0, S+1)`, the SITE it hangs under has its session
/// bumped to S+1, the BOX becomes no key, every other claim is untouched — and
/// deleting the ZONE and BOX in the next session withdraws only the ZONE's
/// claim.
#[test]
fn a_new_primary_is_claimed_a_child_is_not_and_a_delete_withdraws_it() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let mut provider = templates();

    let (before, root_before) = claims(&copy.path);
    let s = DbView::open(&copy.path).unwrap().session().sesno;

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let (site, max_word1) = survey(&mut engine, "SITE");
    let (zone_control, _) = survey(&mut engine, "ZONE");
    let new_zone = RefNo::new(site.word0, max_word1 + 3000);
    let new_box = RefNo::new(site.word0, max_word1 + 3001);
    assert!(before.contains_key(&key(site)), "the SITE is a claimed primary");
    assert!(before.contains_key(&key(zone_control)), "the control ZONE is a claimed primary");
    assert!(!before.contains_key(&key(new_zone)), "the new refnos are unused");

    let zone_template = provider.template_for(db1_hash("ZONE")).unwrap().unwrap().clone();
    let box_template = provider.template_for(db1_hash("BOX")).unwrap().unwrap().clone();
    assert!(zone_template.is_primary, "ZONE is a primary noun");
    assert!(!box_template.is_primary, "BOX is not a primary noun");

    let new_zone_draft = {
        let mut implicit = vec![0u32; 11];
        implicit.extend_from_slice(&zone_template.defaults.implicit);
        RecordDraft::new(new_zone, db1_hash("ZONE"), site, implicit, vec![new_box], vec![]).unwrap()
    };
    let new_box_draft = {
        let mut implicit = vec![0u32; 11];
        implicit.extend_from_slice(&box_template.defaults.implicit);
        RecordDraft::new(new_box, db1_hash("BOX"), new_zone, implicit, vec![], vec![]).unwrap()
    };
    let mut site_draft = draft_of(&mut engine, site);
    site_draft.push_member(new_zone);
    let control_session_before = before[&key(zone_control)];
    drop(engine);

    // --- S+1: the ZONE, the BOX, and the SITE grown to own the ZONE ---
    let report = write_element_session_with(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io claim: a new zone and box", clock: Some(clock()) },
        &[
            ElementWrite::Insert(new_zone_draft),
            ElementWrite::Insert(new_box_draft),
            ElementWrite::Update(site_draft),
        ],
        &[],
        WriteOptions { templates: Some(&mut provider) },
    )
    .expect("write S+1");
    assert_eq!(report.written.sesno, s + 1);
    assert_bytes(&copy.original, &copy, &report);
    let bytes_after1 = copy.current_bytes();

    let (after1, root_after1) = claims(&copy.path);
    assert_ne!(root_after1, root_before, "the claim tree moved");
    assert_eq!(after1.get(&key(new_zone)), Some(&(0, s + 1)), "the new ZONE is claimed for this session");
    assert_eq!(after1.get(&key(new_box)), None, "the BOX is not a primary and gets no claim");
    assert_eq!(after1.get(&key(site)), Some(&(0, s + 1)), "the SITE's session was bumped");
    assert_eq!(after1.get(&key(zone_control)), Some(&control_session_before), "an untouched primary is unchanged");
    // Every other entry is exactly as it was.
    for (refno, value) in &before {
        if *refno == key(site) {
            continue;
        }
        assert_eq!(after1.get(refno), Some(value), "{refno:?} must be untouched");
    }
    assert_eq!(after1.len(), before.len() + 1, "exactly the new ZONE was added");

    // Read back through the point lookup too.
    {
        let mut view = DbView::open(&copy.path).unwrap();
        let mut table = ClaimTable::open(&mut view);
        assert_eq!(table.get(new_zone).unwrap().map(|e| (e.owner, e.session)), Some((0, s + 1)));
        assert_eq!(table.get(new_box).unwrap(), None);
    }

    // --- S+2: delete the ZONE and its BOX, taking the ZONE out of the SITE ---
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let mut site_draft = draft_of(&mut engine, site);
    assert!(site_draft.remove_member(new_zone), "the SITE lists the new ZONE");
    drop(engine);
    let report = write_element_session_with(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io claim: delete the zone", clock: Some(clock()) },
        &[
            ElementWrite::Delete(new_zone),
            ElementWrite::Delete(new_box),
            ElementWrite::Update(site_draft),
        ],
        &[],
        WriteOptions { templates: Some(&mut provider) },
    )
    .expect("write S+2");
    assert_bytes(&bytes_after1, &copy, &report);

    let (after2, _) = claims(&copy.path);
    assert_eq!(after2.get(&key(new_zone)), None, "the deleted ZONE's claim is withdrawn");
    assert_eq!(after2.get(&key(site)), Some(&(0, s + 2)), "the SITE's session bumped again");
    assert_eq!(after2.len(), before.len(), "the tree is back to its original size");
    for (refno, value) in &before {
        if *refno == key(site) {
            continue;
        }
        assert_eq!(after2.get(refno), Some(value), "{refno:?} still untouched after the delete");
    }
}

/// Updating an existing primary bumps its session and nothing else's; a write
/// with no templates leaves the claim tree exactly where it was.
#[test]
fn updating_a_primary_bumps_only_its_session_and_no_templates_moves_no_claim() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let mut provider = templates();
    let (before, root_before) = claims(&copy.path);
    let s = DbView::open(&copy.path).unwrap().session().sesno;

    // A SUBS is a primary; change one implicit word so its record is rewritten.
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let (subs, _) = survey(&mut engine, "SUBS");
    assert!(before.contains_key(&key(subs)), "the SUBS is a claimed primary");
    let mut subs_draft = draft_of(&mut engine, subs);
    drop(engine);
    let word = 12;
    let flipped = subs_draft.implicit()[word] ^ 0x55;
    *subs_draft.implicit_word_mut(word).unwrap() = flipped;

    // --- S+1 with templates: the SUBS's session is bumped, others intact ---
    write_element_session_with(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io claim: touch a subs", clock: Some(clock()) },
        &[ElementWrite::Update(subs_draft)],
        &[],
        WriteOptions { templates: Some(&mut provider) },
    )
    .expect("write S+1");
    let (after1, root_after1) = claims(&copy.path);
    assert_ne!(root_after1, root_before, "the claim tree moved");
    assert_eq!(after1.get(&key(subs)), Some(&(0, s + 1)), "the SUBS session was bumped");
    assert_eq!(after1.len(), before.len(), "no key added or removed");
    for (refno, value) in &before {
        if *refno == key(subs) {
            continue;
        }
        assert_eq!(after1.get(refno), Some(value), "{refno:?} untouched");
    }

    // --- S+2 with no templates: a box, and the claim tree is carried over ---
    let root_before2 = DbView::open(&copy.path).unwrap().session().claim;
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let (zone, max_word1) = survey(&mut engine, "ZONE");
    let new_box = RefNo::new(zone.word0, max_word1 + 5000);
    let box_template = provider.template_for(db1_hash("BOX")).unwrap().unwrap().clone();
    let box_draft = {
        let mut implicit = vec![0u32; 11];
        implicit.extend_from_slice(&box_template.defaults.implicit);
        RecordDraft::new(new_box, db1_hash("BOX"), zone, implicit, vec![], vec![]).unwrap()
    };
    let mut zone_draft = draft_of(&mut engine, zone);
    zone_draft.push_member(new_box);
    drop(engine);
    write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io claim: a box, no templates", clock: Some(clock()) },
        &[ElementWrite::Insert(box_draft), ElementWrite::Update(zone_draft)],
        &[],
    )
    .expect("write S+2");
    let root_after2 = DbView::open(&copy.path).unwrap().session().claim;
    assert_eq!(root_after2, root_before2, "without templates the claim root is carried over verbatim");
    let (after2, _) = claims(&copy.path);
    assert_eq!(after2, after1, "and every claim entry is exactly as S+1 left it");
    // The ZONE, though touched, was not reclaimed because no templates were given.
    assert_eq!(after2.get(&key(zone)).map(|(_, sess)| *sess), before.get(&key(zone)).map(|(_, sess)| *sess));
}
