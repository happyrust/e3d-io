//! Spread regression for the `Transaction` facade: many elements written under
//! many owners in one session read back whole (`docs/plans/2026-09-22-write-path-phase-2-plan.md`
//! §7, T2). This is the write-side half of the scale gate.
//!
//! Why spread across owners rather than pile 20 000 under one ZONE: the record
//! format word (word 10) holds the member-list word count in a **14-bit field**.
//! `core.dll` saturates it at `0x3FFF`; the write path here does not, so an
//! element with more than ~8 191 members (16 383 member words) overflows it —
//! a 20 000-member ZONE wrote `40 256 & 0x3FFF = 7 488` words, which a real E3D
//! read as 3 744 members (`docs/evidence/2026-09-22-core-dll-record-draft-data-flow.md`
//! ⑤; the word-10 saturation fix is plan §1.3 #11, due with T1a). This is a
//! per-element ceiling, not a per-file one, so a realistic model — children
//! spread across owners, each list well under it — round-trips whole. Each
//! owner here gets 400 members (800 words, one page over), far below the
//! overflow point, so word 10 is exact and every list reads back. The real-E3D
//! read-back of the same shape is the manual `D:\e3dio-scale\spread.mac` run
//! recorded in the plan.

mod common;

use common::write_fixture::{DbCopy, AMS8000};
use common::DESVIR;
use e3d_attlib::db1_hash;
use e3d_io::engine::ScanTier;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::{NounTemplate, TemplateProvider};
use e3d_io::record::text::TextFormat;
use e3d_io::session::{DbView, SessionSelector};
use e3d_io::write::{RecordDraft, SessionClock, SessionRequest, Transaction, TransactionError};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

/// Enough boxes per owner that the member block (two words each) overruns a
/// page, so the per-owner list exercises the cross-page split — and below the
/// per-owner cap a real E3D would enumerate.
const PER_OWNER: usize = 400;
/// A few owners, to prove the write scales across owners in one session.
const OWNERS: usize = 3;

fn provider() -> TemplateProvider {
    TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir")
}

fn first_of_noun(path: &std::path::Path, noun: &str) -> RefNo {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let hash = db1_hash(noun);
    engine
        .scan_elements(ScanTier::Header)
        .expect("scan")
        .map(|e| e.expect("header"))
        .find(|e| e.noun_hash == hash)
        .map(|e| e.refno)
        .unwrap_or_else(|| panic!("the sample has a {noun}"))
}

fn build_box(draft: &mut RecordDraft, template: &NounTemplate, format: TextFormat, i: usize) -> Result<(), TransactionError> {
    let set = |draft: &mut RecordDraft, name: &str, value: DescriptorValue| -> Result<(), TransactionError> {
        draft
            .set_attribute(template, db1_hash(name), &value, format)
            .map_err(|e| TransactionError::Parse { refno: RefNo::new(0, 0), detail: format!("{name}: {e}") })
    };
    // A distinct X per box, so the records are not identical (content_eq would
    // otherwise let the writer drop a duplicate).
    set(draft, "POS", DescriptorValue::RealArray(vec![1000.0 + i as f64, 2000.0, 3000.0]))?;
    set(draft, "XLEN", DescriptorValue::Real(300.0))?;
    set(draft, "YLEN", DescriptorValue::Real(400.0))?;
    set(draft, "ZLEN", DescriptorValue::Real(500.0))?;
    Ok(())
}

fn members_of(engine: &mut ReadOnlyEngine, refno: RefNo) -> Vec<RefNo> {
    let view = engine.find_element(refno).expect("find").expect("indexed");
    ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse").members
}

/// `OWNERS` new ZONEs under one SITE, each given `PER_OWNER` boxes, all in one
/// session: every box reads back under its own owner, each owner's member block
/// (800 words > a page) gathers cleanly, and the session diff counts the lot.
#[test]
fn spread_inserts_read_back_across_many_owners() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let site = first_of_noun(&copy.path, "SITE");
    let zone_hash = db1_hash("ZONE");
    let box_hash = db1_hash("BOX");
    let format = TextFormat::from_header(ReadOnlyEngine::open(&copy.path).unwrap().descriptor().header_format);
    let clock = SessionClock::now(0);

    let base_sesno = DbView::open(&copy.path).unwrap().session().sesno;

    // --- one session: OWNERS zones, then PER_OWNER boxes round-robin into them ---
    let (zones, boxes) = {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let mut zones = Vec::with_capacity(OWNERS);
        for z in 0..OWNERS {
            let name = format!("/E3DIO-SPREAD-Z{z:03}");
            zones.push(tx.insert(site, zone_hash, Some(&name), |_d, _t| Ok(())).expect("insert zone"));
        }
        let mut boxes: Vec<Vec<RefNo>> = vec![Vec::new(); OWNERS];
        for i in 0..OWNERS * PER_OWNER {
            let z = i % OWNERS;
            let refno = tx
                .insert(zones[z], box_hash, None, |d, t| build_box(d, t, format, i))
                .expect("insert box");
            boxes[z].push(refno);
        }
        tx.commit(&SessionRequest { user: "e3dio", comment: "e3d-io T2 spread", clock: Some(clock) })
            .expect("commit");
        (zones, boxes)
    };

    // --- read back: each zone owns exactly its PER_OWNER boxes, in order ---
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("reopen");
    let page_words = engine.page_size() as usize / 4;
    let mut total = 0usize;
    for (z, zone) in zones.iter().enumerate() {
        let members = members_of(&mut engine, *zone);
        assert_eq!(
            members.len(),
            PER_OWNER,
            "zone {z} ({zone}) should list exactly {PER_OWNER} boxes, lists {}",
            members.len()
        );
        assert_eq!(members, boxes[z], "zone {z} lists its boxes in insertion order");
        // The member block is two words per member; PER_OWNER of them overruns a
        // page, so a clean read is proof the writer split the block across pages
        // and the reader gathered it (a bad split reads as BlockOverflowsPage).
        assert!(PER_OWNER * 2 > page_words, "the test is only meaningful when the member block spans pages");
        // Every member is a BOX with the geometry written.
        for (i, member) in members.iter().enumerate() {
            let view = engine.find_element(*member).expect("find").expect("indexed box");
            let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse box");
            assert_eq!(parsed.noun_hash, box_hash, "member {i} of zone {z} is a BOX");
            assert_eq!(parsed.owner, *zone, "box's owner is its zone");
        }
        total += members.len();
    }
    assert_eq!(total, OWNERS * PER_OWNER, "every spread box reads back under its owner");

    // --- the SITE gained the new zones ---
    let site_members = members_of(&mut engine, site);
    for zone in &zones {
        assert!(site_members.contains(zone), "the SITE lists the new zone {zone}");
    }
    drop(engine);

    // --- the session diff counts the zones, the boxes, and the SITE rewrite ---
    let mut view = DbView::open(&copy.path).unwrap();
    let latest = view.session().sesno;
    let diff = view
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(latest))
        .expect("diff");
    assert_eq!(diff.tally().deleted, 0, "nothing deleted");
    assert_eq!(
        diff.tally().inserted as usize,
        OWNERS + OWNERS * PER_OWNER,
        "every zone and every box is an insert"
    );
    assert!(diff.tally().modified >= 1, "the SITE (and any owner touched) is a modify");
}
