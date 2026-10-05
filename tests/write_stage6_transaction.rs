//! The transaction facade (`docs/plans/2026-09-21-write-path-next-steps.md` S6):
//! it mints a refno, keeps the owner's member list and the `NAME` table in step,
//! and hands the batch to the same primitive path a caller drives by hand today.
//!
//! The proof it adds nothing but the chores is byte-for-byte: a box inserted
//! through the facade equals the same box written through
//! `write_element_session_with` with the refno the facade minted, the same
//! clock, the same owner update — so the facade's output is the primitive
//! path's output, no more. Renames and deletes then check the two chores the
//! facade automates end to end.

mod common;

use common::write_fixture::{DbCopy, AMS8000};
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_dehash, db1_hash, AttlibData};
use e3d_io::engine::ScanTier;
use e3d_io::index::ClaimTable;
use e3d_io::meta::bucket::BucketTable;
use e3d_io::record::descriptor::{DescriptorValue, ElementExtraction};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::{MemberListKind, NounTemplate, TemplateProvider};
use e3d_io::record::text::TextFormat;
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::{
    write_element_session_with, ElementWrite, MemberPosition, RecordDraft, SessionClock, SessionRequest, Transaction,
    TransactionError, WriteOptions,
};
use e3d_io::{ReadOnlyEngine, RefNo};

use std::collections::BTreeMap;
use std::path::Path;

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 22, hour: 15, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 22, hour: 7, minute: 0, second: 0, millisecond: 0 },
    )
}

fn provider() -> TemplateProvider {
    TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir")
}

/// The first ZONE in the base session.
fn first_zone(path: &Path) -> RefNo {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let zone_hash = db1_hash("ZONE");
    engine
        .scan_elements(ScanTier::Header)
        .expect("scan")
        .map(|e| e.expect("header"))
        .find(|e| e.noun_hash == zone_hash)
        .map(|e| e.refno)
        .expect("the sample has a ZONE")
}

/// Give a fresh BOX its geometry — the same operations on either path, so the
/// bytes can be compared.
fn build_box(draft: &mut RecordDraft, template: &NounTemplate, format: TextFormat) -> Result<(), String> {
    let set = |draft: &mut RecordDraft, name: &str, value: DescriptorValue| -> Result<(), String> {
        draft
            .set_attribute(template, db1_hash(name), &value, format)
            .map_err(|e| format!("{name}: {e}"))
    };
    set(draft, "POS", DescriptorValue::RealArray(vec![1000.0, 2000.0, 3000.0]))?;
    set(draft, "XLEN", DescriptorValue::Real(300.0))?;
    set(draft, "YLEN", DescriptorValue::Real(400.0))?;
    set(draft, "ZLEN", DescriptorValue::Real(500.0))?;
    Ok(())
}

fn extraction(engine: &mut ReadOnlyEngine, attlib: &AttlibData, provider: &mut TemplateProvider, refno: RefNo) -> ElementExtraction {
    engine.extract_element_with_descriptors(refno, attlib, provider).expect("extract")
}

fn value(extraction: &ElementExtraction, name: &str) -> Option<DescriptorValue> {
    extraction.attributes.iter().find(|a| a.name == name).and_then(|a| a.value.clone())
}

/// A box inserted through the facade is byte-for-byte the same file as the box
/// written through the primitive path with the refno the facade minted.
#[test]
fn a_facade_insert_equals_the_primitive_path_byte_for_byte() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let facade_copy = DbCopy::of(AMS8000);
    let primitive_copy = DbCopy::of(AMS8000);
    let zone = first_zone(&facade_copy.path);
    let box_hash = db1_hash("BOX");
    let request = SessionRequest { user: "e3dio", comment: "e3d-io S6 facade box", clock: Some(clock()) };

    // --- the facade path ---
    let mut facade_provider = provider();
    let format = TextFormat::from_header(
        ReadOnlyEngine::open(&facade_copy.path).unwrap().descriptor().header_format,
    );
    let box_refno = {
        let mut tx = Transaction::open(&facade_copy.path, &mut facade_provider).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let refno = tx
            .insert(zone, box_hash, None, |draft, template| {
                build_box(draft, template, format).map_err(|detail| {
                    e3d_io::write::TransactionError::Parse { refno: RefNo::new(0, 0), detail }
                })
            })
            .expect("insert box");
        tx.commit(&request).expect("commit");
        refno
    };

    // --- the primitive path, with the refno the facade chose ---
    {
        let mut primitive_provider = provider();
        let template = primitive_provider.template_for(box_hash).unwrap().unwrap().clone();
        let mut implicit = vec![0u32; 11];
        implicit.extend_from_slice(&template.defaults.implicit);
        implicit[10] |= e3d_io::record::template::WIDE_LAYOUT_FLAG;
        let mut box_draft = RecordDraft::new(box_refno, box_hash, zone, implicit, vec![], vec![]).unwrap();
        build_box(&mut box_draft, &template, format).expect("build box");
        let mut zone_draft = {
            let mut engine = ReadOnlyEngine::open(&primitive_copy.path).unwrap();
            RecordDraft::from_record_bytes(&engine.find_element(zone).unwrap().unwrap().raw_bytes).unwrap()
        };
        zone_draft.push_member(box_refno);
        write_element_session_with(
            &primitive_copy.path,
            &request,
            &[ElementWrite::Insert(box_draft), ElementWrite::Update(zone_draft)],
            &[],
            WriteOptions { templates: Some(&mut primitive_provider) },
        )
        .expect("primitive write");
    }

    // One chore the primitive path does not do: page 1's counter for the box's
    // extent rises past the box (S6b). Every other byte is the same.
    let (facade, primitive) = (facade_copy.current_bytes(), primitive_copy.current_bytes());
    let page_bytes = 4 * u32::from_be_bytes(facade[52..56].try_into().unwrap()) as usize;
    assert_eq!(facade.len(), primitive.len());
    assert_eq!(&facade[..page_bytes], &primitive[..page_bytes], "page 0");
    assert_eq!(
        &facade[2 * page_bytes..],
        &primitive[2 * page_bytes..],
        "the facade must add only the chores, not change a byte of the record, index or session page"
    );
    let mut expected_page1 = primitive[page_bytes..2 * page_bytes].to_vec();
    let entry = BucketTable::parse(&expected_page1)
        .unwrap()
        .entries
        .iter()
        .position(|e| e.extent_word0 == box_refno.word0)
        .expect("the box's extent is on page 1");
    let at = (3 + 3 * entry + 1) * 4;
    expected_page1[at..at + 4].copy_from_slice(&(box_refno.word1 + 1).to_be_bytes());
    assert_eq!(&facade[page_bytes..2 * page_bytes], &expected_page1[..], "page 1 differs only in that counter");

    // --- and the box reads back with its owner, membership and geometry ---
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let mut engine = ReadOnlyEngine::open(&facade_copy.path).expect("reopen");
    let mut read_provider = provider();
    let box_extract = extraction(&mut engine, &attlib, &mut read_provider, box_refno);
    assert_eq!(box_extract.owner, zone, "the box is owned by the zone");
    match value(&box_extract, "POS") {
        Some(DescriptorValue::RealArray(pos)) => assert_eq!(pos, vec![1000.0, 2000.0, 3000.0]),
        other => panic!("POS reads back as {other:?}"),
    }
    assert_eq!(value(&box_extract, "XLEN"), Some(DescriptorValue::Real(300.0)));
    let zone_view = engine.find_element(zone).unwrap().unwrap();
    let zone_parsed = ParsedElement::parse_as(&zone_view.raw_bytes, engine.text_format()).unwrap();
    assert!(zone_parsed.members.contains(&box_refno), "the zone lists the box among its members");
    drop(engine);

    let base_sesno = {
        // The base is the session before the one just written.
        let mut view = DbView::open(&primitive_copy.path).unwrap();
        let latest = view.session().sesno;
        let diff = view
            .diff_sessions(SessionSelector::Exact(latest - 1), SessionSelector::Exact(latest))
            .expect("diff");
        assert_eq!((diff.tally().deleted, diff.tally().inserted, diff.tally().modified), (0, 1, 1));
        latest
    };
    let _ = base_sesno;
}

/// `rename` moves the name in the table and rewrites the record's `NAME`, with
/// no `NamedEdit` in the caller's hands.
#[test]
fn a_facade_rename_moves_the_name_and_rewrites_the_record() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let new_name = "/E3DIO-S6-RENAMED";

    // A named element whose noun's NAME descriptor holds the new name — a ZONE
    // (200 characters), not whatever tiny-NAME element scans first.
    let (refno, old_name) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let zone_hash = db1_hash("ZONE");
        let named = engine
            .scan_elements(ScanTier::Named)
            .expect("scan")
            .map(|e| e.expect("named"))
            .find(|e| e.name.is_some() && e.noun_hash == zone_hash)
            .expect("a named ZONE");
        assert!(engine.find_by_name(new_name).expect("lookup").is_none(), "the new name is free");
        let old = named.name.map(|n| if n.starts_with('/') { n } else { format!("/{n}") }).unwrap();
        (named.refno, old)
    };

    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.rename(refno, new_name).expect("rename");
        tx.commit(&SessionRequest { user: "e3dio", comment: "e3d-io S6 rename", clock: Some(clock()) })
            .expect("commit");
    }

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("reopen");
    assert_eq!(engine.find_by_name(new_name).expect("lookup new"), Some(refno), "the new name finds it");
    assert_eq!(engine.find_by_name(&old_name).expect("lookup old"), None, "the old name is gone");
    let view = engine.find_element(refno).unwrap().unwrap();
    let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).unwrap();
    assert_eq!(
        e3d_io::record::element::ParsedElement::parse_as(&view.raw_bytes, engine.text_format())
            .ok()
            .and_then(|p| p.explicit_attributes.iter().find_map(|a| match &a.value {
                e3d_io::record::explicit::ExplicitValue::Text(t) if a.hash == db1_hash("NAME") => Some(t.clone()),
                _ => None,
            })),
        Some(new_name.to_string()),
        "the record's NAME is the new one",
    );
    let _ = parsed;
}

/// A box inserted then deleted through the facade leaves the index and the
/// owner's member list as they were: the delete drops the key and takes the box
/// out of the zone.
#[test]
fn a_facade_delete_drops_the_key_and_the_owner_membership() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let zone = first_zone(&copy.path);
    let box_hash = db1_hash("BOX");
    let format = TextFormat::from_header(ReadOnlyEngine::open(&copy.path).unwrap().descriptor().header_format);

    // Insert a box (S+1), so the delete has a known member of a known owner.
    let box_refno = {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let refno = tx
            .insert(zone, box_hash, None, |draft, template| {
                build_box(draft, template, format).map_err(|detail| {
                    e3d_io::write::TransactionError::Parse { refno: RefNo::new(0, 0), detail }
                })
            })
            .expect("insert");
        tx.commit(&SessionRequest { user: "e3dio", comment: "e3d-io S6 insert-for-delete", clock: Some(clock()) })
            .expect("commit");
        refno
    };
    let after_insert = DbView::open(&copy.path).unwrap().session().sesno;
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).unwrap();
        assert!(engine.find_element(box_refno).unwrap().is_some(), "the box is there after the insert");
    }

    // Delete it (S+2).
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.delete(box_refno).expect("delete");
        tx.commit(&SessionRequest { user: "e3dio", comment: "e3d-io S6 delete", clock: Some(clock()) })
            .expect("commit");
    }

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("reopen");
    assert!(engine.find_element(box_refno).unwrap().is_none(), "the box's key is gone");
    let zone_view = engine.find_element(zone).unwrap().unwrap();
    let zone_parsed = ParsedElement::parse_as(&zone_view.raw_bytes, engine.text_format()).unwrap();
    assert!(!zone_parsed.members.contains(&box_refno), "the zone no longer lists the box");
    drop(engine);

    let mut view = DbView::open(&copy.path).unwrap();
    let diff = view
        .diff_sessions(SessionSelector::Exact(after_insert), SessionSelector::Exact(after_insert + 1))
        .expect("diff");
    assert_eq!(diff.tally().deleted, 1, "the delete session drops exactly the box");
    let _ = db1_dehash(box_hash);
}

// --- T1a: move_to / reorder / reference members ---

/// The first `n` ZONEs in scan order.
fn zones(path: &Path, n: usize) -> Vec<RefNo> {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let zone_hash = db1_hash("ZONE");
    engine
        .scan_elements(ScanTier::Header)
        .expect("scan")
        .map(|e| e.expect("header"))
        .filter(|e| e.noun_hash == zone_hash)
        .map(|e| e.refno)
        .take(n)
        .collect()
}

/// An element's member list as stored.
fn members(path: &Path, refno: RefNo) -> Vec<RefNo> {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let view = engine.find_element(refno).expect("lookup").unwrap_or_else(|| panic!("{refno} is indexed"));
    ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse").members
}

/// An element's owner and where its record sits, as stored.
fn owner_and_place(path: &Path, refno: RefNo) -> (RefNo, u32, u32) {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let view = engine.find_element(refno).expect("lookup").unwrap_or_else(|| panic!("{refno} is indexed"));
    (view.header().owner, view.page_no, view.byte_offset)
}

/// The claim tree as `refno -> (owner, session)`.
fn claims(path: &Path) -> BTreeMap<(u32, u32), (u32, u32)> {
    let mut view = DbView::open(path).expect("open");
    let mut table = ClaimTable::open(&mut view);
    table
        .iter()
        .map(|entry| {
            let entry = entry.expect("the claim tree walks");
            ((entry.refno.word0, entry.refno.word1), (entry.owner, entry.session))
        })
        .collect()
}

fn key(refno: RefNo) -> (u32, u32) {
    (refno.word0, refno.word1)
}

/// `(deleted, inserted, modified)` between two sessions.
fn tally(path: &Path, from: u32, to: u32) -> (u64, u64, u64) {
    let mut view = DbView::open(path).expect("open");
    let diff = view.diff_sessions(SessionSelector::Exact(from), SessionSelector::Exact(to)).expect("diff");
    let tally = diff.tally();
    (tally.deleted, tally.inserted, tally.modified)
}

fn latest(path: &Path) -> u32 {
    DbView::open(path).expect("open").session().sesno
}

/// A BOX inserted under `zone` through the facade, in its own session.
fn insert_box(copy: &DbCopy, zone: RefNo, comment: &str) -> RefNo {
    let format = TextFormat::from_header(ReadOnlyEngine::open(&copy.path).unwrap().descriptor().header_format);
    let mut prov = provider();
    let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
    let refno = tx
        .insert(zone, db1_hash("BOX"), None, |draft, template| {
            build_box(draft, template, format)
                .map_err(|detail| TransactionError::Parse { refno: RefNo::new(0, 0), detail })
        })
        .expect("insert");
    tx.commit(&SessionRequest { user: "e3dio", comment, clock: Some(clock()) }).expect("commit");
    refno
}

/// `move_to` is `INCLUDE`: the BOX's owner word changes and nothing else in
/// its record does, ZONE A's list loses it, ZONE B's list gains it at the end,
/// exactly three records are rewritten, and both ZONEs — primaries — have their
/// claim sessions bumped while every other claim stands. Moving it back with an
/// `After` anchor lands it right behind that member.
#[test]
fn move_to_reparents_the_box_and_rewrites_exactly_both_owners() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let all_zones = zones(&copy.path, 40);
    let zone_a = all_zones[0];
    let claimed = claims(&copy.path);
    let zone_b = *all_zones[1..]
        .iter()
        .find(|z| claimed.contains_key(&key(**z)) && !members(&copy.path, **z).is_empty())
        .expect("a second, claimed ZONE with members");
    assert!(claimed.contains_key(&key(zone_a)), "ZONE A is a claimed primary");

    let the_box = insert_box(&copy, zone_a, "e3d-io T1a: a box to move");
    let s1 = latest(&copy.path);
    let a_before = members(&copy.path, zone_a);
    let b_before = members(&copy.path, zone_b);
    let claims_before = claims(&copy.path);
    assert!(a_before.contains(&the_box) && !b_before.contains(&the_box));
    let box_before = {
        let mut engine = ReadOnlyEngine::open(&copy.path).unwrap();
        RecordDraft::from_record_bytes(&engine.find_element(the_box).unwrap().unwrap().raw_bytes).unwrap()
    };

    // --- S+2: A → B, last ---
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.move_to(the_box, zone_b, MemberPosition::Last).expect("move");
        tx.commit(&SessionRequest { user: "e3dio", comment: "e3d-io T1a: move A -> B", clock: Some(clock()) })
            .expect("commit");
    }
    let s2 = latest(&copy.path);
    assert_eq!(s2, s1 + 1);
    assert_eq!(owner_and_place(&copy.path, the_box).0, zone_b, "the record's owner word is B");
    let a_after: Vec<RefNo> = a_before.iter().copied().filter(|m| *m != the_box).collect();
    assert_eq!(members(&copy.path, zone_a), a_after, "A's list lost exactly the box, order kept");
    let mut b_after = b_before.clone();
    b_after.push(the_box);
    assert_eq!(members(&copy.path, zone_b), b_after, "B's list gained the box at the end");
    assert_eq!(tally(&copy.path, s1, s2), (0, 0, 3), "the box and the two owners, nothing else");
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).unwrap();
        let box_after = RecordDraft::from_record_bytes(&engine.find_element(the_box).unwrap().unwrap().raw_bytes).unwrap();
        assert_eq!(box_after.owner, zone_b);
        assert_eq!(&box_after.implicit()[11..], &box_before.implicit()[11..], "the implicit values are as stored");
        assert_eq!(box_after.explicit_stream_words(), box_before.explicit_stream_words(), "the explicit stream too");
        assert_eq!(box_after.members(), box_before.members());
    }
    let claims_after = claims(&copy.path);
    assert_eq!(claims_after.len(), claims_before.len(), "no claim key came or went");
    assert_eq!(claims_after[&key(zone_a)].1, s2, "ZONE A's claim session was bumped");
    assert_eq!(claims_after[&key(zone_b)].1, s2, "ZONE B's claim session was bumped");
    for (refno, value) in &claims_before {
        if *refno == key(zone_a) || *refno == key(zone_b) {
            continue;
        }
        assert_eq!(claims_after.get(refno), Some(value), "{refno:?} untouched");
    }

    // --- S+3: back to A, right after A's first member ---
    let anchor = a_after[0];
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.move_to(the_box, zone_a, MemberPosition::After(anchor)).expect("move back");
        tx.commit(&SessionRequest { user: "e3dio", comment: "e3d-io T1a: move B -> A after", clock: Some(clock()) })
            .expect("commit");
    }
    let s3 = latest(&copy.path);
    assert_eq!(owner_and_place(&copy.path, the_box).0, zone_a);
    let mut expected_a = a_after.clone();
    expected_a.insert(1, the_box);
    assert_eq!(members(&copy.path, zone_a), expected_a, "the box sits right after the anchor");
    assert_eq!(members(&copy.path, zone_b), b_before, "B is as it was before the box came");
    assert_eq!(tally(&copy.path, s2, s3), (0, 0, 3));
}

/// `reorder` is `REORDER`: only the owner's record is rewritten — the member's
/// own record keeps its page and offset — and `First` / `After` / `Before`
/// each put it where they say.
#[test]
fn reorder_moves_a_member_within_its_owner_and_rewrites_only_the_owner() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let zone = first_zone(&copy.path);
    let the_box = insert_box(&copy, zone, "e3d-io T1a: a box to reorder");
    let s1 = latest(&copy.path);
    let before = members(&copy.path, zone);
    assert!(before.len() >= 4, "the sample ZONE has members to reorder among");
    assert_eq!(before.last(), Some(&the_box), "the insert appended it");
    let place_before = owner_and_place(&copy.path, the_box);

    let commit = |at: MemberPosition, comment: &str| {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.reorder(the_box, at).expect("reorder");
        tx.commit(&SessionRequest { user: "e3dio", comment, clock: Some(clock()) }).expect("commit");
        latest(&copy.path)
    };
    let others: Vec<RefNo> = before.iter().copied().filter(|m| *m != the_box).collect();

    let s2 = commit(MemberPosition::First, "e3d-io T1a: reorder first");
    let mut expected = others.clone();
    expected.insert(0, the_box);
    assert_eq!(members(&copy.path, zone), expected, "First puts it at the front");
    assert_eq!(tally(&copy.path, s1, s2), (0, 0, 1), "only the ZONE is rewritten");
    assert_eq!(owner_and_place(&copy.path, the_box), place_before, "the box's record was not touched");

    let s3 = commit(MemberPosition::After(others[1]), "e3d-io T1a: reorder after");
    let mut expected = others.clone();
    expected.insert(2, the_box);
    assert_eq!(members(&copy.path, zone), expected, "After(m) puts it right behind m");
    assert_eq!(tally(&copy.path, s2, s3), (0, 0, 1));

    let s4 = commit(MemberPosition::Before(others[0]), "e3d-io T1a: reorder before");
    let mut expected = others.clone();
    expected.insert(0, the_box);
    assert_eq!(members(&copy.path, zone), expected, "Before(first) puts it at the front");
    assert_eq!(tally(&copy.path, s3, s4), (0, 0, 1));

    let s5 = commit(MemberPosition::Last, "e3d-io T1a: reorder last");
    assert_eq!(members(&copy.path, zone), before, "Last restores the insert order");
    assert_eq!(tally(&copy.path, s4, s5), (0, 0, 1));
    assert_eq!(owner_and_place(&copy.path, the_box), place_before);
}

/// A `GROU`'s members are references: `add_reference_member` /
/// `remove_reference_member` edit its list and nothing else — the referenced
/// elements keep their owners and their records — while a child insert under a
/// ZONE that the group also lists changes the ZONE's list, not the group's.
/// The refusals: a child cannot be moved under a reference list, a child list
/// is not edited as references, an element cannot be moved beneath itself,
/// and an anchor must be a member.
#[test]
fn reference_members_are_edited_on_the_group_and_nowhere_else() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let all_zones = zones(&copy.path, 2);
    let (zone_a, zone_b) = (all_zones[0], all_zones[1]);
    let world = ReadOnlyEngine::open(&copy.path).unwrap().world().unwrap().unwrap().refno;
    let format = TextFormat::from_header(ReadOnlyEngine::open(&copy.path).unwrap().descriptor().header_format);
    {
        let mut prov = provider();
        let grou = prov.template_for(db1_hash("GROU")).unwrap().unwrap();
        assert_eq!(grou.member_list_kind, MemberListKind::Referenced, "GROU's list is a reference list");
    }
    let s0 = latest(&copy.path);
    let a_before = members(&copy.path, zone_a);
    let (a_owner, ..) = owner_and_place(&copy.path, zone_a);

    // --- S+1: GPWL under the WORL, GROU under it referencing ZONE A and a new
    // BOX that is a child of ZONE A ---
    let (gpwl, grou, the_box) = {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let gpwl = tx.insert(world, db1_hash("GPWL"), None, |_, _| Ok(())).expect("gpwl");
        let grou = tx.insert(gpwl, db1_hash("GROU"), None, |_, _| Ok(())).expect("grou");
        tx.add_reference_member(grou, zone_a, MemberPosition::Last).expect("reference the zone");
        let the_box = tx
            .insert(zone_a, db1_hash("BOX"), None, |draft, template| {
                build_box(draft, template, format).map_err(|detail| TransactionError::Parse { refno: RefNo::new(0, 0), detail })
            })
            .expect("box");
        tx.add_reference_member(grou, the_box, MemberPosition::Last).expect("reference the box");
        tx.commit(&SessionRequest { user: "e3dio", comment: "e3d-io T1a: a group with references", clock: Some(clock()) })
            .expect("commit");
        (gpwl, grou, the_box)
    };
    let s1 = latest(&copy.path);
    assert_eq!(members(&copy.path, grou), vec![zone_a, the_box], "the group lists both references in order");
    assert_eq!(members(&copy.path, gpwl), vec![grou], "the GPWL owns the group as a child");
    assert_eq!(owner_and_place(&copy.path, zone_a).0, a_owner, "the ZONE keeps its owner");
    assert_eq!(owner_and_place(&copy.path, the_box).0, zone_a, "the BOX is the ZONE's child, not the group's");
    let mut a_after = a_before.clone();
    a_after.push(the_box);
    assert_eq!(members(&copy.path, zone_a), a_after, "the child insert went to the ZONE's list");
    // GPWL + GROU + BOX inserted; WORL + ZONE A rewritten for membership.
    assert_eq!(tally(&copy.path, s0, s1), (0, 3, 2));

    // --- S+2: ZONE B in front, ZONE A out — only the group is rewritten ---
    let box_place = owner_and_place(&copy.path, the_box);
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.add_reference_member(grou, zone_b, MemberPosition::First).expect("reference zone B");
        tx.remove_reference_member(grou, zone_a).expect("drop zone A");
        tx.commit(&SessionRequest { user: "e3dio", comment: "e3d-io T1a: edit the references", clock: Some(clock()) })
            .expect("commit");
    }
    let s2 = latest(&copy.path);
    assert_eq!(members(&copy.path, grou), vec![zone_b, the_box]);
    assert_eq!(tally(&copy.path, s1, s2), (0, 0, 1), "only the group");
    assert_eq!(owner_and_place(&copy.path, the_box), box_place, "the referenced BOX was not touched");
    assert_eq!(members(&copy.path, zone_a), a_after, "the ZONE's own list is not the group's");

    // --- the refusals, none of which writes ---
    let bytes_before = copy.current_bytes();
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        assert!(
            matches!(tx.move_to(the_box, grou, MemberPosition::Last), Err(TransactionError::OwnerIsReferenceList { owner }) if owner == grou),
            "a child cannot be moved under a reference list"
        );
        assert!(
            matches!(tx.add_reference_member(zone_a, the_box, MemberPosition::Last), Err(TransactionError::NotAReferenceList { owner }) if owner == zone_a),
            "a ZONE's list is its children, not references"
        );
        assert!(
            matches!(tx.remove_reference_member(grou, zone_a), Err(TransactionError::NotAMember { owner, member }) if owner == grou && member == zone_a),
            "ZONE A is no longer a reference member"
        );
        assert!(
            matches!(tx.move_to(zone_a, the_box, MemberPosition::Last), Err(TransactionError::WouldOwnItself { refno, owner }) if refno == zone_a && owner == the_box),
            "a ZONE cannot be moved under its own BOX"
        );
        assert!(
            matches!(tx.reorder(the_box, MemberPosition::After(gpwl)), Err(TransactionError::AnchorNotAMember { owner, anchor }) if owner == zone_a && anchor == gpwl),
            "the anchor must be a member of the owner"
        );
        assert!(
            matches!(tx.move_to(the_box, zone_b, MemberPosition::Before(gpwl)), Err(TransactionError::AnchorNotAMember { owner, anchor }) if owner == zone_b && anchor == gpwl),
            "so must a move's anchor"
        );
        // Nothing was queued that would write; a commit of nothing is refused
        // upstream, so just drop the transaction.
    }
    assert_eq!(copy.current_bytes(), bytes_before, "a refused operation writes nothing");
}

/// An element inserted and moved in one transaction lands only under its final
/// owner — the owner it passed through is not rewritten — and a move followed
/// by a delete leaves neither owner listing it.
#[test]
fn a_move_within_the_inserting_transaction_touches_only_the_final_owner() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let all_zones = zones(&copy.path, 2);
    let (zone_a, zone_b) = (all_zones[0], all_zones[1]);
    let format = TextFormat::from_header(ReadOnlyEngine::open(&copy.path).unwrap().descriptor().header_format);
    let s0 = latest(&copy.path);
    let a_before = members(&copy.path, zone_a);
    let b_before = members(&copy.path, zone_b);

    let the_box = {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let the_box = tx
            .insert(zone_a, db1_hash("BOX"), None, |draft, template| {
                build_box(draft, template, format).map_err(|detail| TransactionError::Parse { refno: RefNo::new(0, 0), detail })
            })
            .expect("box");
        tx.move_to(the_box, zone_b, MemberPosition::First).expect("move the pending insert");
        tx.commit(&SessionRequest { user: "e3dio", comment: "e3d-io T1a: insert then move", clock: Some(clock()) })
            .expect("commit");
        the_box
    };
    let s1 = latest(&copy.path);
    assert_eq!(owner_and_place(&copy.path, the_box).0, zone_b);
    assert_eq!(members(&copy.path, zone_a), a_before, "ZONE A was only passed through");
    let mut b_after = b_before.clone();
    b_after.insert(0, the_box);
    assert_eq!(members(&copy.path, zone_b), b_after);
    assert_eq!(tally(&copy.path, s0, s1), (0, 1, 1), "the box and ZONE B only");

    // Move it to A and delete it in one go: A must not end up listing it.
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.move_to(the_box, zone_a, MemberPosition::Last).expect("move");
        tx.delete(the_box).expect("delete");
        tx.commit(&SessionRequest { user: "e3dio", comment: "e3d-io T1a: move then delete", clock: Some(clock()) })
            .expect("commit");
    }
    let s2 = latest(&copy.path);
    assert!(ReadOnlyEngine::open(&copy.path).unwrap().find_element(the_box).unwrap().is_none(), "the box is gone");
    assert_eq!(members(&copy.path, zone_a), a_before, "A never got it");
    assert_eq!(members(&copy.path, zone_b), b_before, "B lost it");
    let (deleted, inserted, _) = tally(&copy.path, s1, s2);
    assert_eq!((deleted, inserted), (1, 0));
}
