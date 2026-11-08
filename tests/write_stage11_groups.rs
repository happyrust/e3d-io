//! Stage 11 of the write path: the GPWL / GPSET / GPITEM group facade
//! (`src/write/group.rs`), checked the way E3D's secondary hierarchy reads it
//! back — member list, `GPREF`, `SCOSEL` — on a throw-away copy of the sample.
//!
//! What these cases pin, in order of how easy each is to get wrong:
//!
//! 1. A `GPITEM` added through [`Transaction::insert_gpset_member`] lands in the
//!    GPSET's **member list** (not as a `GROU`-style reference entry), and its
//!    `GPREF` reads back as the target — including a target in another database
//!    number, which is the case the real sample shows.
//! 2. The same target twice yields one GPITEM: the second call returns
//!    `Existing` with the first item's refno, and the member list does not grow.
//!    This is the write-side half of `resolved_members_on`'s `seen` dedupe.
//! 3. `SCOSEL` written through [`Transaction::set_gpset_rules`] reads back as a
//!    type-20 int-array hunk with the same words.
//! 4. A parent of the wrong noun is refused with
//!    [`TransactionError::NotAGroupContainer`] rather than filed where the
//!    secondary hierarchy will never see it.
//!
//! The sample has no writable GPWL, so the world a GPWL needs is taken from the
//! database's own `WORL`; the GPWL and GPSET themselves are created here.

mod common;

use common::write_fixture::{DbCopy, APS250218};
use common::{ATTLIB, DESVIR};
use e3d_attlib::db1_hash;
use e3d_io::engine::ScanTier;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{SessionDate, SessionSelector, DbView};
use e3d_io::write::group::{attrs, GpsetMember};
use e3d_io::write::{
    MemberPosition, SessionClock, SessionRequest, Transaction, TransactionError,
};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 11, day: 8, hour: 10, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 11, day: 8, hour: 2, minute: 0, second: 0, millisecond: 0 },
    )
}

fn provider() -> TemplateProvider {
    TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir")
}

fn request() -> SessionRequest<'static> {
    SessionRequest { user: "e3dio", comment: "e3d-io S11 group facade", clock: Some(clock()) }
}

/// The database's `WORL`, and two existing design elements to reference — one in
/// this database, one in another (the cross-database `GPREF` the sample shows).
struct Sample {
    world: RefNo,
    here: RefNo,
    elsewhere: RefNo,
}

fn sample_of(path: &std::path::Path) -> Option<Sample> {
    let mut engine = ReadOnlyEngine::open(path).ok()?;
    let world_hash = db1_hash("WORL");
    let mut world = None;
    let mut here = None;
    let mut elsewhere = None;
    for element in engine.scan_elements(ScanTier::Header).ok()? {
        let element = element.ok()?;
        if world.is_none() && element.noun_hash == world_hash {
            world = Some(element.refno);
        }
        // A design element: something with a name and a noun that is neither a
        // group container nor the world itself.
        let noun = e3d_attlib::db1_dehash(element.noun_hash).unwrap_or_default();
        if noun.is_empty() || noun == "WORL" || noun.starts_with("GP") {
            continue;
        }
        if here.is_none() && element.refno.word0 == world?.word0 {
            here = Some(element.refno);
        } else if elsewhere.is_none() && element.refno.word0 != world?.word0 {
            elsewhere = Some(element.refno);
        }
    }
    Some(Sample { world: world?, here: here?, elsewhere: elsewhere? })
}

/// The explicit attribute `hash` of `refno`, read straight off the record with
/// its hunks resolved — a value stored out of line reads back as the array it
/// points at, not as the five-word directory entry.
fn explicit_of(path: &std::path::Path, refno: RefNo, hash: u32) -> Option<ExplicitValue> {
    let mut engine = ReadOnlyEngine::open(path).ok()?;
    let view = engine.find_element(refno).ok()??;
    let mut parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).ok()?;
    engine.resolve_hunks(&mut parsed).ok()?;
    parsed
        .explicit_attributes
        .iter()
        .find(|attr| attr.hash == hash)
        .map(|attr| attr.value.clone())
}

fn members_of(path: &std::path::Path, refno: RefNo) -> Vec<RefNo> {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let view = engine.find_element(refno).expect("read").expect("exists");
    ParsedElement::parse_as(&view.raw_bytes, engine.text_format())
        .expect("parse")
        .members
}

/// A GPWL and a GPSET written into the copy, then a member added — the whole
/// chain, read back through the member lists and `GPREF`.
#[test]
fn a_group_written_through_the_facade_reads_back_through_the_member_lists() {
    if common::missing(&[APS250218, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(APS250218);
    let Some(sample) = sample_of(&copy.path) else {
        eprintln!("APS250218 has no WORL or no design element to reference; skipping");
        return;
    };
    let mut provider = provider();

    let (gpwl, gpset, member) = {
        let mut tx = Transaction::open(&copy.path, &mut provider)
            .expect("open tx")
            .with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let gpwl = tx.insert_gpwl(sample.world, "/E3DIO-TEST-GPWL").expect("gpwl");
        let gpset = tx.insert_gpset(gpwl, "/E3DIO-TEST-GPWL/SET-1").expect("gpset");
        let member = tx
            .insert_gpset_member(gpset, sample.here, MemberPosition::Last)
            .expect("member");
        tx.commit(&request()).expect("commit");
        (gpwl, gpset, member)
    };

    // The GPWL is a member of the world, the GPSET of the GPWL — the chain E3D
    // walks to reach a secondary hierarchy at all.
    assert!(
        members_of(&copy.path, sample.world).contains(&gpwl),
        "the world lists the new GPWL"
    );
    assert!(
        members_of(&copy.path, gpwl).contains(&gpset),
        "the GPWL lists the new GPSET"
    );

    // The GPITEM is a member of the GPSET, and its GPREF is the target.
    let members = members_of(&copy.path, gpset);
    assert_eq!(members.len(), 1, "the GPSET holds exactly the one GPITEM");
    let item = members[0];
    assert_eq!(member.item(), item, "the facade reports the GPITEM it created");
    match explicit_of(&copy.path, item, db1_hash(attrs::GPREF)) {
        Some(ExplicitValue::RefNo(target)) => {
            assert_eq!(target, sample.here, "GPREF reads back as the target")
        }
        other => panic!("GPREF reads back as {other:?}"),
    }

    // And the session really is a new one: three inserts (GPWL, GPSET, GPITEM)
    // and one modify — the WORL. The GPWL's own member list is filled in at
    // staging from the queued member op, so the GPWL is an insert, not a second
    // modify.
    let mut view = DbView::open(&copy.path).expect("open view");
    let latest = view.session().sesno;
    let diff = view
        .diff_sessions(SessionSelector::Exact(latest - 1), SessionSelector::Exact(latest))
        .expect("diff");
    assert_eq!(
        (diff.tally().inserted, diff.tally().modified),
        (3, 1),
        "three inserts (GPWL, GPSET, GPITEM) and one owner update (WORL)"
    );
}

/// The same target twice is one GPITEM — the write-side half of the read side's
/// `seen` dedupe.
#[test]
fn adding_the_same_target_twice_keeps_one_gpitem() {
    if common::missing(&[APS250218, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(APS250218);
    let Some(sample) = sample_of(&copy.path) else {
        eprintln!("APS250218 has no WORL or no design element to reference; skipping");
        return;
    };
    let mut provider = provider();

    let (first, second, members) = {
        let mut tx = Transaction::open(&copy.path, &mut provider)
            .expect("open tx")
            .with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let gpwl = tx.insert_gpwl(sample.world, "/E3DIO-DEDUPE").expect("gpwl");
        let gpset = tx.insert_gpset(gpwl, "/E3DIO-DEDUPE/SET").expect("gpset");
        let first = tx
            .insert_gpset_member(gpset, sample.here, MemberPosition::Last)
            .expect("first");
        let second = tx
            .insert_gpset_member(gpset, sample.here, MemberPosition::Last)
            .expect("second");
        tx.commit(&request()).expect("commit");
        (first, second, members_of(&copy.path, gpset))
    };

    assert!(matches!(first, GpsetMember::Inserted(_)), "the first call inserts");
    assert!(matches!(second, GpsetMember::Existing(_)), "the second finds it");
    assert_eq!(first.item(), second.item(), "both name the same GPITEM");
    assert_eq!(members.len(), 1, "the GPSET still holds one member");
}

/// A target in another database number goes into `GPREF` as itself — the write
/// side does not care which database the target lives in, and must not.
#[test]
fn a_cross_database_target_is_stored_as_its_own_refno() {
    if common::missing(&[APS250218, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(APS250218);
    let Some(sample) = sample_of(&copy.path) else {
        eprintln!("APS250218 has no WORL or no design element to reference; skipping");
        return;
    };
    assert_ne!(
        sample.here.word0, sample.elsewhere.word0,
        "the sample has an element in another database number"
    );
    let mut provider = provider();

    let item = {
        let mut tx = Transaction::open(&copy.path, &mut provider)
            .expect("open tx")
            .with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let gpwl = tx.insert_gpwl(sample.world, "/E3DIO-CROSS").expect("gpwl");
        let gpset = tx.insert_gpset(gpwl, "/E3DIO-CROSS/SET").expect("gpset");
        let member = tx
            .insert_gpset_member(gpset, sample.elsewhere, MemberPosition::Last)
            .expect("member");
        tx.commit(&request()).expect("commit");
        member.item()
    };

    match explicit_of(&copy.path, item, db1_hash(attrs::GPREF)) {
        Some(ExplicitValue::RefNo(target)) => assert_eq!(
            target, sample.elsewhere,
            "the cross-database refno is stored whole, database number included"
        ),
        other => panic!("GPREF reads back as {other:?}"),
    }
}

/// `SCOSEL` written as an int array reads back as the same words.
#[test]
fn scosel_reads_back_as_the_words_it_was_written_with() {
    if common::missing(&[APS250218, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(APS250218);
    let Some(sample) = sample_of(&copy.path) else {
        eprintln!("APS250218 has no WORL or no design element to reference; skipping");
        return;
    };
    let mut provider = provider();
    // A shape the read side's decoder recognises: header, class segment, rule
    // group, filter segment with one `before` over a text literal.
    let words: Vec<i32> = vec![
        9, 100_016, 1, 100_009, 0, 100_007, 0x1234, 100_015, 3, 1, 1307, 118, 0,
    ];

    let gpset = {
        let mut tx = Transaction::open(&copy.path, &mut provider)
            .expect("open tx")
            .with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let gpwl = tx.insert_gpwl(sample.world, "/E3DIO-RULES").expect("gpwl");
        let gpset = tx.insert_gpset(gpwl, "/E3DIO-RULES/SET").expect("gpset");
        tx.set_gpset_rules(gpset, &words).expect("rules");
        tx.commit(&request()).expect("commit");
        gpset
    };

    match explicit_of(&copy.path, gpset, db1_hash(attrs::SCOSEL)) {
        Some(ExplicitValue::IntArray(read)) => assert_eq!(read, words, "SCOSEL round-trips"),
        other => panic!("SCOSEL reads back as {other:?}"),
    }
}

/// A GPSET filed under something that is not a GPWL is refused, not written.
#[test]
fn a_set_under_a_non_gpwl_parent_is_refused() {
    if common::missing(&[APS250218, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(APS250218);
    let Some(sample) = sample_of(&copy.path) else {
        eprintln!("APS250218 has no WORL or no design element to reference; skipping");
        return;
    };
    let mut provider = provider();

    let mut tx = Transaction::open(&copy.path, &mut provider)
        .expect("open tx")
        .with_spatial(e3d_io::spatial::SpatialPolicy::Off);
    // The world is not a GPWL, so a GPSET under it would be invisible to the
    // secondary hierarchy.
    let error = tx
        .insert_gpset(sample.world, "/E3DIO-WRONG-PARENT")
        .expect_err("a GPSET under WORL must be refused");
    assert!(
        matches!(error, TransactionError::NotAGroupContainer { .. }),
        "refused as a group container problem, got {error:?}"
    );
    // A GPWL under something that is not WORL is refused the same way.
    let error = tx
        .insert_gpwl(sample.here, "/E3DIO-WRONG-WORLD")
        .expect_err("a GPWL under a design element must be refused");
    assert!(matches!(error, TransactionError::NotAGroupContainer { .. }));
    drop(tx);
}
