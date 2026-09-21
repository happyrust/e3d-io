//! Stage 3 of the write plan (`docs/plans/2026-09-20-dabacon-write-plan.md` §2
//! 阶段 3, A3a) on `ams8000_0001`: the three-view conflict scan and the
//! disjoint merge write.
//!
//! The construction is the plan's: C0 is the sample at session S; writer A
//! commits S+1; writer B, still based on S, merges its own changes onto the
//! file and commits S+2. A clean merge is the union of both; an overlap is
//! refused — 562 a change over a delete, 590 a delete over a change, 561 a
//! name already taken. Two changes to one element reconcile attribute by
//! attribute and, on its member list, member by member — or whole, on a noun
//! whose members are references (`GROU`).

mod common;

use common::write_fixture::{assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS8000};
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::engine::ScanTier;
use e3d_io::index::encode_name;
use e3d_io::index::reference::ReferenceTable;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::record::template::{MemberListKind, NounTemplate, TemplateProvider};
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::merge::code;
use e3d_io::write::record::put_real_vec_wide;
use e3d_io::write::{
    reference_value, set_reference, write_element_session, write_merge_session, write_merge_session_with, ElementWrite,
    MergeOptions, NamedEdit, NamedEditKind, RecordDraft, SessionClock, SessionRequest, WriteError,
};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;
const PAGE0_COMMIT_WORDS: &[usize] = &[9, 10, 11];

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 20, hour: 17, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 20, hour: 9, minute: 0, second: 0, millisecond: 0 },
    )
}

fn leng_offset(provider: &mut TemplateProvider, subs_hash: u32) -> usize {
    provider
        .template_for(subs_hash)
        .expect("load")
        .expect("subs template")
        .attrs
        .iter()
        .find(|a| e3d_attlib::db1_dehash(a.hash).as_deref() == Some("LENG"))
        .expect("LENG")
        .wide
        .location
        .word_offset() as usize
}

fn pos_offset(template: &NounTemplate) -> usize {
    template
        .attrs
        .iter()
        .find(|a| e3d_attlib::lookup_system_name(a.hash) == Some("POS"))
        .expect("POS")
        .wide
        .location
        .word_offset() as usize
}

/// A fresh unnamed BOX under `owner`, its POS set so it decodes.
fn fresh_box(template: &NounTemplate, refno: RefNo, owner: RefNo) -> RecordDraft {
    let mut implicit = vec![0u32; 11];
    implicit.extend_from_slice(&template.defaults.implicit);
    put_real_vec_wide(&mut implicit, pos_offset(template), &[1.0, 2.0, 3.0]);
    RecordDraft::new(refno, db1_hash("BOX"), owner, implicit, vec![], vec![]).expect("box draft")
}

/// `zone` with `member` appended, read as of the given engine.
fn zone_with_member(engine: &mut ReadOnlyEngine, zone: RefNo, member: RefNo) -> RecordDraft {
    let view = engine.find_element(zone).expect("zone").expect("exists");
    let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
    draft.push_member(member);
    draft
}

/// `subs` with its `LENG` set to `value`, read as of the given engine.
fn subs_with_leng(engine: &mut ReadOnlyEngine, subs: RefNo, leng_at: usize, value: f64) -> RecordDraft {
    let view = engine.find_element(subs).expect("subs").expect("exists");
    let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
    draft.set_real_wide(leng_at, value);
    draft
}

/// Two ZONEs, two SUBS, and a spare RefNo base — the actors in the scenario.
struct Cast {
    zones: [RefNo; 2],
    subs: [RefNo; 2],
    fresh: [RefNo; 2],
}

fn cast(path: &str) -> Cast {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let zone_hash = db1_hash("ZONE");
    let subs_hash = db1_hash("SUBS");
    let mut zones = Vec::new();
    let mut subs = Vec::new();
    let mut max_word1 = 0u32;
    let mut word0 = 24_384;
    for element in engine.scan_elements(ScanTier::Header).expect("scan") {
        let element = element.expect("header");
        max_word1 = max_word1.max(element.refno.word1);
        if element.noun_hash == zone_hash && zones.len() < 2 {
            zones.push(element.refno);
            word0 = element.refno.word0;
        }
        if element.noun_hash == subs_hash && subs.len() < 2 {
            subs.push(element.refno);
        }
    }
    Cast {
        zones: [zones[0], zones[1]],
        subs: [subs[0], subs[1]],
        fresh: [RefNo::new(word0, max_word1 + 2000), RefNo::new(word0, max_word1 + 2001)],
    }
}

#[test]
fn a_disjoint_merge_is_the_union_of_both_writers() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let subs_hash = db1_hash("SUBS");
    let leng_at = leng_offset(&mut provider, subs_hash);
    let box_template = provider.template_for(db1_hash("BOX")).expect("box").expect("box template").clone();
    let cast = cast(AMS8000);

    let s = DbView::open(&copy.path).unwrap().session().sesno;

    // --- writer A commits S+1: a new box under zone[0], and subs[0]'s LENG ---
    let a_elements = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let subs_leng = subs_with_leng(&mut engine, cast.subs[0], leng_at, 1000.0);
        let zone = zone_with_member(&mut engine, cast.zones[0], cast.fresh[0]);
        vec![
            ElementWrite::Insert(fresh_box(&box_template, cast.fresh[0], cast.zones[0])),
            ElementWrite::Update(zone),
            ElementWrite::Update(subs_leng),
        ]
    };
    write_element_session(
        &copy.path,
        &SessionRequest { user: "A", comment: "writer A", clock: Some(clock()) },
        &a_elements,
        &[],
    )
    .expect("A writes S+1");

    let after_a = copy.current_bytes();
    let s1_end = DbView::open(&copy.path).unwrap().session().end.page;

    // --- writer B, still based on S, merges its own disjoint changes ---
    let b_elements = {
        let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("open base");
        let subs_leng = subs_with_leng(&mut base, cast.subs[1], leng_at, 2000.0);
        let zone = zone_with_member(&mut base, cast.zones[1], cast.fresh[1]);
        vec![
            ElementWrite::Insert(fresh_box(&box_template, cast.fresh[1], cast.zones[1])),
            ElementWrite::Update(zone),
            ElementWrite::Update(subs_leng),
        ]
    };
    let report = write_merge_session(
        &copy.path,
        &SessionRequest { user: "B", comment: "writer B merges", clock: Some(clock()) },
        s,
        &b_elements,
        &[],
    )
    .expect("B merges S+2");
    let new_end = *report.appended_pages.iter().max().unwrap();

    // --- the two range diffs ---
    let mut view = DbView::open(&copy.path).expect("reopen");
    assert_eq!(view.session().sesno, s + 2);
    let tail = view.session().tail.expect("tail");
    assert_eq!(tail.base_sesno, s, "BASESN records the base B merged from, not the latest");

    let whole = view
        .diff_sessions(SessionSelector::Exact(s), SessionSelector::Exact(s + 2))
        .expect("diff S..S+2");
    assert_eq!(
        (whole.tally().deleted, whole.tally().inserted, whole.tally().modified),
        (0, 2, 4),
        "S..S+2 is A ∪ B: two new boxes, four modified (two zones, two subs)"
    );
    let just_b = view
        .diff_sessions(SessionSelector::Exact(s + 1), SessionSelector::Exact(s + 2))
        .expect("diff S+1..S+2");
    assert_eq!(
        (just_b.tally().deleted, just_b.tally().inserted, just_b.tally().modified),
        (0, 1, 2),
        "S+1..S+2 is B alone"
    );
    drop(view);

    // --- both writers' values are present ---
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    assert!(engine.find_element(cast.fresh[0]).unwrap().is_some(), "A's box survived the merge");
    assert!(engine.find_element(cast.fresh[1]).unwrap().is_some(), "B's box is there");
    let mut leng = |refno| {
        match engine
            .extract_element_with_descriptors(refno, &attlib, &mut provider)
            .unwrap()
            .attributes
            .iter()
            .find(|a| a.name == "LENG")
            .and_then(|a| a.value.clone())
        {
            Some(DescriptorValue::Real(v)) => v,
            other => panic!("{refno} LENG {other:?}"),
        }
    };
    assert_eq!(leng(cast.subs[0]), 1000.0, "A's LENG");
    assert_eq!(leng(cast.subs[1]), 2000.0, "B's LENG");

    // --- B's merge left A's committed pages untouched ---
    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(&copy.original, 2048, 0, 13) as usize;
    assert_prefix_unchanged(&after_a, &bytes, page_bytes, s1_end as usize, PAGE0_COMMIT_WORDS);
    assert_appended_contiguous(&bytes, page_bytes, s1_end as usize, new_end as usize, &[3, 5, 7]);
}

/// Run A's S+1, then a B merge expected to be refused, and assert nothing moved.
fn refused_merge(
    build_a: impl FnOnce(&mut ReadOnlyEngine, &Cast) -> Vec<ElementWrite>,
    build_b: impl FnOnce(&mut ReadOnlyEngine, &Cast) -> (Vec<ElementWrite>, Vec<NamedEdit>),
    expect_code: u32,
) {
    let copy = DbCopy::of(AMS8000);
    let cast = cast(AMS8000);
    let s = DbView::open(&copy.path).unwrap().session().sesno;

    let a_elements = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        build_a(&mut engine, &cast)
    };
    write_element_session(
        &copy.path,
        &SessionRequest { user: "A", comment: "A", clock: Some(clock()) },
        &a_elements,
        &[],
    )
    .expect("A writes S+1");
    let after_a = copy.current_bytes();

    let (b_elements, b_names) = {
        let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("open base");
        build_b(&mut base, &cast)
    };
    let result = write_merge_session(
        &copy.path,
        &SessionRequest { user: "B", comment: "B", clock: Some(clock()) },
        s,
        &b_elements,
        &b_names,
    );
    match result {
        Err(WriteError::MergeConflicts(conflicts)) => {
            assert!(
                conflicts.iter().any(|c| c.code == expect_code),
                "expected conflict {expect_code}, got {conflicts:?}"
            );
        }
        other => panic!("expected a refused merge, got {other:?}"),
    }
    assert_eq!(copy.current_bytes(), after_a, "a refused merge writes nothing");
}

#[test]
fn a_change_over_a_delete_is_refused_562() {
    if common::missing(&[AMS8000]) {
        return;
    }
    // A deletes a leaf under a BRAN; B, based on S, changes that same leaf.
    let victim = std::cell::Cell::new(RefNo::new(0, 0));
    refused_merge(
        |engine, _| {
            let (owner, leaf) = leaf_under(engine, db1_hash("BRAN"));
            victim.set(leaf);
            let mut draft = RecordDraft::from_record_bytes(&engine.find_element(owner).unwrap().unwrap().raw_bytes).unwrap();
            draft.remove_member(leaf);
            vec![ElementWrite::Delete(leaf), ElementWrite::Update(draft)]
        },
        |base, _| {
            let mut draft = RecordDraft::from_record_bytes(&base.find_element(victim.get()).unwrap().unwrap().raw_bytes).unwrap();
            // Force a real change by bumping an implicit attribute word.
            if let Some(word) = draft.implicit_word_mut(11) {
                *word ^= 1;
            }
            (vec![ElementWrite::Update(draft)], vec![])
        },
        code::CHANGED_UNDER_DELETE,
    );
}

#[test]
fn a_delete_over_a_change_is_refused_590() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let victim = std::cell::Cell::new(RefNo::new(0, 0));
    refused_merge(
        |engine, cast| {
            victim.set(cast.subs[0]);
            let mut draft = RecordDraft::from_record_bytes(&engine.find_element(cast.subs[0]).unwrap().unwrap().raw_bytes).unwrap();
            if let Some(word) = draft.implicit_word_mut(11) {
                *word ^= 1;
            }
            vec![ElementWrite::Update(draft)]
        },
        |_base, _| (vec![ElementWrite::Delete(victim.get())], vec![]),
        code::DELETED_UNDER_CHANGE,
    );
}

#[test]
fn a_name_already_taken_is_refused_561() {
    if common::missing(&[AMS8000]) {
        return;
    }
    refused_merge(
        |_engine, cast| {
            // A commits something innocuous so S+1 != S.
            let template = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE))
                .expect("desvir")
                .template_for(db1_hash("BOX"))
                .expect("box")
                .expect("box template")
                .clone();
            vec![ElementWrite::Insert(fresh_box(&template, cast.fresh[0], cast.zones[0]))]
        },
        |base, cast| {
            // B tries to name its box with a name already in the table.
            let taken = existing_name(base);
            (vec![], vec![NamedEdit { key: encode_name(&taken), refno: cast.fresh[1], kind: NamedEditKind::Insert }])
        },
        code::NAME_TAKEN,
    );
}

#[test]
fn two_writers_add_members_to_one_owner() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let box_template = provider.template_for(db1_hash("BOX")).expect("box").expect("template").clone();
    let cast = cast(AMS8000);
    let owner = cast.zones[0];
    let (box_a, box_b) = (cast.fresh[0], cast.fresh[1]);
    let s = DbView::open(&copy.path).unwrap().session().sesno;

    // A adds box_a under the owner.
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let zone = zone_with_member(&mut engine, owner, box_a);
        write_element_session(
            &copy.path,
            &SessionRequest { user: "A", comment: "A adds box A", clock: Some(clock()) },
            &[ElementWrite::Insert(fresh_box(&box_template, box_a, owner)), ElementWrite::Update(zone)],
            &[],
        )
        .expect("A writes S+1");
    }

    // B, based on S, adds box_b under the same owner, and merges.
    {
        let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("base");
        let zone = zone_with_member(&mut base, owner, box_b);
        write_merge_session(
            &copy.path,
            &SessionRequest { user: "B", comment: "B adds box B", clock: Some(clock()) },
            s,
            &[ElementWrite::Insert(fresh_box(&box_template, box_b, owner)), ElementWrite::Update(zone)],
            &[],
        )
        .expect("B merges S+2");
    }

    // Both members are under the owner, and both boxes are indexed.
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    assert!(engine.find_element(box_a).unwrap().is_some(), "A's box is indexed");
    assert!(engine.find_element(box_b).unwrap().is_some(), "B's box is indexed");
    let zone = engine.find_element(owner).unwrap().unwrap();
    let members = ParsedElement::parse_as(&zone.raw_bytes, engine.text_format()).unwrap().members;
    assert!(members.contains(&box_a), "A's member survived the merge");
    assert!(members.contains(&box_b), "B's member merged in");

    let mut view = DbView::open(&copy.path).expect("view");
    let whole = view
        .diff_sessions(SessionSelector::Exact(s), SessionSelector::Exact(s + 2))
        .expect("diff");
    assert_eq!(
        (whole.tally().deleted, whole.tally().inserted, whole.tally().modified),
        (0, 2, 1),
        "two new boxes and the one owner modified"
    );
}

#[test]
fn a_moved_member_lands_after_its_anchor_when_merged() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let box_template = provider.template_for(db1_hash("BOX")).expect("box").expect("template").clone();
    let cast = cast(AMS8000);
    let new_box = cast.fresh[0];
    let s = DbView::open(&copy.path).unwrap().session().sesno;

    // An owner with at least three members, so a move is not just a swap.
    let (owner, base_members) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        owner_with_members(&mut engine, db1_hash("ZONE"), 3)
    };
    let (first, last) = (base_members[0], *base_members.last().unwrap());

    // A appends a box to the owner: the committed list is base + box.
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let zone = zone_with_member(&mut engine, owner, new_box);
        write_element_session(
            &copy.path,
            &SessionRequest { user: "A", comment: "A adds a box", clock: Some(clock()) },
            &[ElementWrite::Insert(fresh_box(&box_template, new_box, owner)), ElementWrite::Update(zone)],
            &[],
        )
        .expect("A writes S+1");
    }

    // B, based on S, moves the owner's first member behind its last, and merges.
    {
        let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("base");
        let view = base.find_element(owner).expect("owner").expect("exists");
        let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
        assert_eq!(draft.members(), &base_members[..], "B's base list is the one scanned");
        assert!(draft.move_member_after(first, Some(last)));
        write_merge_session(
            &copy.path,
            &SessionRequest { user: "B", comment: "B reorders a member", clock: Some(clock()) },
            s,
            &[ElementWrite::Update(draft)],
            &[],
        )
        .expect("B merges S+2");
    }

    // The moved member sits right after its anchor, the old tail; A's box,
    // which B never saw, keeps its committed place behind it.
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let zone = engine.find_element(owner).unwrap().unwrap();
    let members = ParsedElement::parse_as(&zone.raw_bytes, engine.text_format()).unwrap().members;
    let mut expected: Vec<RefNo> = base_members[1..].to_vec();
    expected.push(first);
    expected.push(new_box);
    assert_eq!(members, expected, "st 2: first member re-anchored after the old last, A's box still behind");
    assert!(engine.find_element(new_box).unwrap().is_some(), "A's box is still indexed");
    drop(engine);

    let mut view = DbView::open(&copy.path).expect("view");
    let whole = view
        .diff_sessions(SessionSelector::Exact(s), SessionSelector::Exact(s + 2))
        .expect("diff");
    assert_eq!(
        (whole.tally().deleted, whole.tally().inserted, whole.tally().modified),
        (0, 1, 1),
        "one new box and the one owner modified"
    );
}

#[test]
fn a_reference_list_noun_merges_its_members_whole() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");

    // Word 3 of the template header tells the kinds apart: GROU's members are
    // references, ZONE's and BRAN's are their children, a WELD owns nothing.
    let mut kind = |noun: &str| {
        provider
            .template_for(db1_hash(noun))
            .expect("load")
            .expect("template")
            .member_list_kind
    };
    assert_eq!(kind("GROU"), MemberListKind::Referenced);
    assert_eq!(kind("ZONE"), MemberListKind::Owned);
    assert_eq!(kind("BRAN"), MemberListKind::Owned);
    assert_eq!(kind("WELD"), MemberListKind::None);

    let cast = cast(AMS8000);
    let (s0, s1, z0, z1) = (cast.subs[0], cast.subs[1], cast.zones[0], cast.zones[1]);

    // With the schema at hand the group's list is one attribute: B's whole
    // list replaces A's, and A's addition is gone.
    let whole = group_merge(&mut provider, &cast, true);
    assert_eq!(whole, vec![s0, s1, z1], "kind == 2: the writer's whole list, A's addition lost");

    // Without it every noun merges member by member, and both additions land.
    let by_member = group_merge(&mut provider, &cast, false);
    assert_eq!(by_member, vec![s0, s1, z1, z0], "no schema: member by member, B's addition after its anchor");
}

/// On a fresh copy: a GPWL under the WORL and a GROU under it referring to
/// the two SUBS (session S); A adds zone 0 to the group (S+1); B, based on S,
/// adds zone 1 and merges (S+2), with or without the schema. Returns the
/// group's merged member list, having checked the referenced elements are
/// untouched and the diff sees one modified element.
fn group_merge(provider: &mut TemplateProvider, cast: &Cast, with_schema: bool) -> Vec<RefNo> {
    let copy = DbCopy::of(AMS8000);
    let gpwl_template = provider.template_for(db1_hash("GPWL")).expect("gpwl").expect("template").clone();
    let grou_template = provider.template_for(db1_hash("GROU")).expect("grou").expect("template").clone();
    let (gpwl, grou) = (cast.fresh[0], cast.fresh[1]);
    let (s0, s1, z0, z1) = (cast.subs[0], cast.subs[1], cast.zones[0], cast.zones[1]);

    // Session S: WORL → GPWL → GROU, the group referring to two SUBS owned elsewhere.
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let world = engine.world().expect("world").expect("has a WORL");
        let mut world_draft = RecordDraft::from_record_bytes(&world.raw_bytes).expect("world draft");
        world_draft.push_member(gpwl);
        let gpwl_draft = fresh_element(&gpwl_template, gpwl, world.refno, vec![grou]);
        let grou_draft = fresh_element(&grou_template, grou, gpwl, vec![s0, s1]);
        write_element_session(
            &copy.path,
            &SessionRequest { user: "setup", comment: "a group world and a group", clock: Some(clock()) },
            &[ElementWrite::Insert(gpwl_draft), ElementWrite::Insert(grou_draft), ElementWrite::Update(world_draft)],
            &[],
        )
        .expect("setup writes S");
    }
    let s = DbView::open(&copy.path).unwrap().session().sesno;

    // A adds zone 0 to the group: S+1.
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let group = zone_with_member(&mut engine, grou, z0);
        write_element_session(
            &copy.path,
            &SessionRequest { user: "A", comment: "A adds zone 0 to the group", clock: Some(clock()) },
            &[ElementWrite::Update(group)],
            &[],
        )
        .expect("A writes S+1");
    }

    // B, based on S, adds zone 1 to the group and merges.
    {
        let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("base");
        let group = zone_with_member(&mut base, grou, z1);
        let options = MergeOptions { templates: with_schema.then_some(&mut *provider), ..Default::default() };
        write_merge_session_with(
            &copy.path,
            &SessionRequest { user: "B", comment: "B adds zone 1 to the group", clock: Some(clock()) },
            s,
            &[ElementWrite::Update(group)],
            &[],
            options,
        )
        .expect("B merges S+2");
    }

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    // The referenced elements are untouched: still indexed, still owned by
    // whoever owned them, not by the group.
    for referenced in [s0, s1, z0, z1] {
        let owner = engine.find_element(referenced).unwrap().expect("still indexed").header().owner;
        assert_ne!(owner, grou, "{referenced} is referenced by the group, not owned by it");
    }
    let group = engine.find_element(grou).unwrap().expect("group indexed");
    assert_eq!(group.header().owner, gpwl);
    let members = ParsedElement::parse_as(&group.raw_bytes, engine.text_format()).unwrap().members;
    drop(engine);

    let mut view = DbView::open(&copy.path).expect("view");
    let whole = view
        .diff_sessions(SessionSelector::Exact(s), SessionSelector::Exact(s + 2))
        .expect("diff");
    assert_eq!(
        (whole.tally().deleted, whole.tally().inserted, whole.tally().modified),
        (0, 0, 1),
        "S..S+2 modified the group once and nothing else"
    );
    members
}

/// A fresh unnamed element of `template`'s noun under `owner`, carrying the
/// noun's default implicit record and the given members.
fn fresh_element(template: &NounTemplate, refno: RefNo, owner: RefNo, members: Vec<RefNo>) -> RecordDraft {
    let mut implicit = vec![0u32; 11];
    implicit.extend_from_slice(&template.defaults.implicit);
    RecordDraft::new(refno, template.noun_hash, owner, implicit, members, vec![]).expect("draft")
}

/// An element of `noun` with at least `min` members, and its member list.
fn owner_with_members(engine: &mut ReadOnlyEngine, noun: u32, min: usize) -> (RefNo, Vec<RefNo>) {
    for element in engine.scan_elements(ScanTier::Full).expect("scan") {
        let element = element.expect("element");
        if element.noun_hash != noun {
            continue;
        }
        if let Some(parsed) = element.parsed.as_ref() {
            if parsed.members.len() >= min {
                return (element.refno, parsed.members.clone());
            }
        }
    }
    panic!("no {noun:#x} with {min} members");
}

#[test]
fn a_merge_that_converges_to_the_committed_value_writes_no_record() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let cast = cast(AMS8000);
    let victim = cast.subs[0];
    let s = DbView::open(&copy.path).unwrap().session().sesno;
    let base_word = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        implicit_word(&mut engine, victim, 11)
    };
    let converged = base_word ^ 0xABCD;

    // A commits word 11 = converged.
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let mut draft = RecordDraft::from_record_bytes(&engine.find_element(victim).unwrap().unwrap().raw_bytes).unwrap();
        *draft.implicit_word_mut(11).unwrap() = converged;
        write_element_session(
            &copy.path,
            &SessionRequest { user: "A", comment: "A sets the value", clock: Some(clock()) },
            &[ElementWrite::Update(draft)],
            &[],
        )
        .expect("A writes S+1");
    }

    // B, based on S, sets word 11 to the same value A landed on, then merges.
    let report = {
        let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("base");
        let mut draft = RecordDraft::from_record_bytes(&base.find_element(victim).unwrap().unwrap().raw_bytes).unwrap();
        *draft.implicit_word_mut(11).unwrap() = converged;
        write_merge_session(
            &copy.path,
            &SessionRequest { user: "B", comment: "B converges", clock: Some(clock()) },
            s,
            &[ElementWrite::Update(draft)],
            &[],
        )
        .expect("B merges S+2")
    };

    // The merged record equals what A committed, so nothing was written for it:
    // the session appended one page, its own session page, and no record or
    // tree copy.
    assert_eq!(report.appended_pages.len(), 1, "only the session page was appended");

    let mut view = DbView::open(&copy.path).expect("view");
    let just_b = view
        .diff_sessions(SessionSelector::Exact(s + 1), SessionSelector::Exact(s + 2))
        .expect("diff");
    assert_eq!(
        (just_b.tally().deleted, just_b.tally().inserted, just_b.tally().modified),
        (0, 0, 0),
        "S+1..S+2 changed nothing — the element was de-duplicated"
    );
    drop(view);

    // The element still reads the converged value, at the address A gave it.
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    assert_eq!(implicit_word(&mut engine, victim, 11), converged);
}

/// The word at `word` of the element's implicit region.
fn implicit_word(engine: &mut ReadOnlyEngine, refno: RefNo, word: usize) -> u32 {
    let view = engine.find_element(refno).unwrap().unwrap();
    let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).unwrap();
    u32::from_be_bytes(parsed.implicit_data[word * 4..word * 4 + 4].try_into().unwrap())
}

#[test]
fn two_changes_to_one_element_merge_attribute_by_attribute() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let cast = cast(AMS8000);
    let victim = cast.subs[0];
    let s = DbView::open(&copy.path).unwrap().session().sesno;

    // The two attribute words A and B will each change, and their base values.
    let (a_word, b_word) = (11usize, 12usize);
    let (base_a, base_b) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        (implicit_word(&mut engine, victim, a_word), implicit_word(&mut engine, victim, b_word))
    };

    // A commits S+1 changing word A of the element.
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let mut draft = RecordDraft::from_record_bytes(&engine.find_element(victim).unwrap().unwrap().raw_bytes).unwrap();
        *draft.implicit_word_mut(a_word).unwrap() = base_a ^ 0xABCD;
        write_element_session(
            &copy.path,
            &SessionRequest { user: "A", comment: "A changes word A", clock: Some(clock()) },
            &[ElementWrite::Update(draft)],
            &[],
        )
        .expect("A writes S+1");
    }

    // B, based on S, changes word B of the same element, and merges.
    {
        let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("base");
        let mut draft = RecordDraft::from_record_bytes(&base.find_element(victim).unwrap().unwrap().raw_bytes).unwrap();
        *draft.implicit_word_mut(b_word).unwrap() = base_b ^ 0x1234;
        write_merge_session(
            &copy.path,
            &SessionRequest { user: "B", comment: "B changes word B", clock: Some(clock()) },
            s,
            &[ElementWrite::Update(draft)],
            &[],
        )
        .expect("B merges S+2");
    }

    // Both attribute changes survive on the one element.
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    assert!(engine.find_element(victim).unwrap().is_some());
    assert_eq!(implicit_word(&mut engine, victim, a_word), base_a ^ 0xABCD, "A's attribute survived");
    assert_eq!(implicit_word(&mut engine, victim, b_word), base_b ^ 0x1234, "B's attribute merged in");

    let mut view = DbView::open(&copy.path).expect("view");
    let whole = view
        .diff_sessions(SessionSelector::Exact(s), SessionSelector::Exact(s + 2))
        .expect("diff");
    assert_eq!(
        (whole.tally().deleted, whole.tally().inserted, whole.tally().modified),
        (0, 0, 1),
        "the one element is modified once across S..S+2"
    );
}

/// The keyed tables follow a merge as they follow a plain write: A commits a
/// box (S+1, no table moves); B, based on S, points the first `SPRE` edge's
/// referrer at another specification and merges with the schema at hand (S+2)
/// — the `SPRE` table loses `[A, R]`, gains `[B, R]`, and every other table
/// keeps its root page.
#[test]
fn a_merge_keeps_the_reverse_reference_tables_in_step() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let box_template = provider.template_for(db1_hash("BOX")).expect("box").expect("box template").clone();
    let cast = cast(AMS8000);
    let spre = db1_hash("SPRE");
    let s = DbView::open(&copy.path).unwrap().session().sesno;

    let edges = |view: &mut DbView| -> Vec<[u32; 4]> {
        ReferenceTable::open(view, spre)
            .expect("SPRE table")
            .iter()
            .map(|e| {
                let e = e.unwrap();
                [e.target.word0, e.target.word1, e.referrer.word0, e.referrer.word1]
            })
            .collect()
    };
    let (before, roots_before) = {
        let mut view = DbView::open(&copy.path).unwrap();
        let roots: Vec<(u32, u32)> = view.tables().iter().map(|t| (t.name_hash, t.root.page)).collect();
        (edges(&mut view), roots)
    };
    let first = before[0];
    let (a, r) = (RefNo::new(first[0], first[1]), RefNo::new(first[2], first[3]));
    let b = RefNo::new(a.word0, a.word1 + 7);

    // A commits S+1: a box under zone[0].
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let zone = zone_with_member(&mut engine, cast.zones[0], cast.fresh[0]);
        write_element_session(
            &copy.path,
            &SessionRequest { user: "A", comment: "A adds a box", clock: Some(clock()) },
            &[ElementWrite::Insert(fresh_box(&box_template, cast.fresh[0], cast.zones[0])), ElementWrite::Update(zone)],
            &[],
        )
        .expect("A writes S+1");
    }

    // B, based on S, re-points R's SPRE and merges with the schema.
    {
        let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("base");
        let mut draft = RecordDraft::from_record_bytes(&base.find_element(r).unwrap().unwrap().raw_bytes).unwrap();
        let template = provider.template_for(draft.noun_hash).unwrap().expect("R's template").clone();
        assert_eq!(reference_value(&draft, &template, spre), Some(a));
        assert!(set_reference(&mut draft, &template, spre, b));
        write_merge_session_with(
            &copy.path,
            &SessionRequest { user: "B", comment: "B re-points an SPRE", clock: Some(clock()) },
            s,
            &[ElementWrite::Update(draft)],
            &[],
            MergeOptions { templates: Some(&mut provider), ..Default::default() },
        )
        .expect("B merges S+2");
    }

    let mut view = DbView::open(&copy.path).expect("view");
    assert_eq!(view.session().sesno, s + 2);
    let after = edges(&mut view);
    let mut expected: Vec<[u32; 4]> = before.iter().copied().filter(|k| *k != first).collect();
    expected.push([b.word0, b.word1, r.word0, r.word1]);
    expected.sort();
    assert_eq!(after, expected, "SPRE lost [A, R] and gained [B, R]");
    for (table, root) in &roots_before {
        let now = view.table_root(*table).expect("table kept").page;
        if *table == spre {
            assert_ne!(now, *root, "SPRE was rewritten");
        } else {
            assert_eq!(now, *root, "table {table} kept its root through both sessions");
        }
    }
    let whole = view
        .diff_sessions(SessionSelector::Exact(s), SessionSelector::Exact(s + 2))
        .expect("diff");
    assert_eq!(
        (whole.tally().deleted, whole.tally().inserted, whole.tally().modified),
        (0, 1, 2),
        "A's box and zone, B's referrer"
    );
}

/// Extract flush flips the same-attribute tie-break. A (committed) and B
/// (writer, based on S) change the same implicit word of one element; the
/// plain merge keeps B's value, the extract-flush merge keeps A's — otherwise
/// the two writes are identical.
#[test]
fn an_extract_flush_merge_keeps_the_committed_value_where_plain_keeps_the_writers() {
    if common::missing(&[AMS8000]) {
        return;
    }
    // Run the same A-then-B construction twice, once plain and once flushed.
    let outcome = |flush: bool| -> u32 {
        let copy = DbCopy::of(AMS8000);
        let cast = cast(AMS8000);
        let victim = cast.subs[0];
        let s = DbView::open(&copy.path).unwrap().session().sesno;
        let word = 12usize;
        let base_value = implicit_word(&mut ReadOnlyEngine::open(&copy.path).unwrap(), victim, word);

        // A commits S+1, setting the word to a committed value.
        {
            let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
            let mut draft = RecordDraft::from_record_bytes(&engine.find_element(victim).unwrap().unwrap().raw_bytes).unwrap();
            *draft.implicit_word_mut(word).unwrap() = base_value ^ 0x0AAA;
            write_element_session(
                &copy.path,
                &SessionRequest { user: "A", comment: "A sets the word", clock: Some(clock()) },
                &[ElementWrite::Update(draft)],
                &[],
            )
            .expect("A writes S+1");
        }
        // B, based on S, sets the same word to its own value and merges.
        {
            let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("base");
            let mut draft = RecordDraft::from_record_bytes(&base.find_element(victim).unwrap().unwrap().raw_bytes).unwrap();
            *draft.implicit_word_mut(word).unwrap() = base_value ^ 0x5000;
            write_merge_session_with(
                &copy.path,
                &SessionRequest { user: "B", comment: "B sets the word", clock: Some(clock()) },
                s,
                &[ElementWrite::Update(draft)],
                &[],
                MergeOptions { extract_flush: flush, ..Default::default() },
            )
            .expect("B merges S+2");
        }
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        implicit_word(&mut engine, victim, word)
    };

    let base_value = {
        let copy = DbCopy::of(AMS8000);
        let cast = cast(AMS8000);
        implicit_word(&mut ReadOnlyEngine::open(&copy.path).unwrap(), cast.subs[0], 12)
    };
    assert_eq!(outcome(false), base_value ^ 0x5000, "plain SAVEWORK keeps the writer B's value");
    assert_eq!(outcome(true), base_value ^ 0x0AAA, "extract flush keeps the committed A's value");
}

/// The element's member list as of the given engine.
fn members_of(engine: &mut ReadOnlyEngine, refno: RefNo) -> Vec<RefNo> {
    let view = engine.find_element(refno).expect("read").expect("indexed");
    ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse").members
}

/// The element's owner as of the given engine.
fn owner_of(engine: &mut ReadOnlyEngine, refno: RefNo) -> RefNo {
    engine.find_element(refno).expect("read").expect("indexed").header().owner
}

/// The `(deleted, inserted, modified)` tally between two sessions.
fn tally(path: &std::path::Path, from: u32, to: u32) -> (u64, u64, u64) {
    let mut view = DbView::open(path).expect("view");
    let diff = view
        .diff_sessions(SessionSelector::Exact(from), SessionSelector::Exact(to))
        .expect("diff");
    let tally = diff.tally();
    (tally.deleted, tally.inserted, tally.modified)
}

/// `sub_5B20CA0`'s gate on an addition (`st 1`), plain `SAVEWORK` arm: a
/// member the writer added to a list is left out when the committed side
/// deleted the element since the base.
///
/// A deletes a leaf M (out of its owner P's list and out of the index) and
/// commits S+1. B, based on S, appends M to another owner O's list without
/// touching M itself — the shape a reference list takes, and the only shape
/// that reaches the gate: had B changed M too, the scan would have refused the
/// merge as 562. Merged, O's list is the committed one; M is not put back in,
/// so the merged O equals the committed O and the INDEX de-duplication writes
/// no record at all. Extract flush gates the same case (M reads as owner
/// `(0,0)` in latest, ≠ P in base).
#[test]
fn an_added_member_the_committed_side_deleted_is_left_out_when_merged() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let cast = cast(AMS8000);
    let s = DbView::open(&copy.path).unwrap().session().sesno;
    let other_owner = cast.zones[1];

    // A: delete leaf M from under its owner P.
    let (owner_p, leaf) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let (owner_p, leaf) = leaf_under(&mut engine, db1_hash("BRAN"));
        assert!(!members_of(&mut engine, other_owner).contains(&leaf), "M is not already under O");
        let mut p = RecordDraft::from_record_bytes(&engine.find_element(owner_p).unwrap().unwrap().raw_bytes).unwrap();
        p.remove_member(leaf);
        write_element_session(
            &copy.path,
            &SessionRequest { user: "A", comment: "A deletes M", clock: Some(clock()) },
            &[ElementWrite::Delete(leaf), ElementWrite::Update(p)],
            &[],
        )
        .expect("A writes S+1");
        (owner_p, leaf)
    };
    let committed_o = members_of(&mut ReadOnlyEngine::open(&copy.path).unwrap(), other_owner);
    assert!(
        ReadOnlyEngine::open(&copy.path).unwrap().find_element(leaf).unwrap().is_none(),
        "M is out of the index at S+1"
    );

    // B, based on S, appends M to O's list, plain and then flushed.
    for (flush, expected_sesno) in [(false, s + 2), (true, s + 3)] {
        let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("base");
        let o = zone_with_member(&mut base, other_owner, leaf);
        let report = write_merge_session_with(
            &copy.path,
            &SessionRequest { user: "B", comment: "B adds the deleted M to O", clock: Some(clock()) },
            s,
            &[ElementWrite::Update(o)],
            &[],
            MergeOptions { extract_flush: flush, ..Default::default() },
        )
        .expect("B merges");
        assert_eq!(report.written.sesno, expected_sesno);
        // M was gated out, so the merged O equals the committed O: nothing but
        // the session page was written.
        assert_eq!(report.appended_pages.len(), 1, "flush={flush}: only the session page was appended");
        assert_eq!(tally(&copy.path, expected_sesno - 1, expected_sesno), (0, 0, 0), "flush={flush}: no element changed");
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        assert_eq!(members_of(&mut engine, other_owner), committed_o, "flush={flush}: O's list is the committed one, without M");
        assert!(engine.find_element(leaf).unwrap().is_none(), "flush={flush}: M stays deleted");
        assert!(!members_of(&mut engine, owner_p).contains(&leaf), "flush={flush}: P's list stays without M");
    }
}

/// `sub_5B20CA0`'s gate on an addition (`st 1`), fourth-slot arm: under
/// extract flush a member the writer added is left out when the committed side
/// changed the element's `OWNREF` since the base — so the list agrees with the
/// owner `sub_5B211C0` keeps for it; plain `SAVEWORK` does not look at the
/// owner and adds it.
///
/// A moves leaf M from P to Q (M's owner, P's and Q's lists) and commits S+1.
/// B, based on S, moves the same M from P to O. Flushed: M keeps the committed
/// owner Q, O's list does not gain M, P's list loses it on both sides — every
/// merged record equals its committed one and no record is written. Plain: M
/// takes B's owner O, O's list gains M, and — `sub_5B211C0`'s fix-up — Q, the
/// owner the committed record named and whose record B never handed in, loses
/// M from its list.
#[test]
fn an_added_member_the_committed_side_re_parented_is_left_out_under_extract_flush() {
    if common::missing(&[AMS8000]) {
        return;
    }
    struct Outcome {
        appended: usize,
        changed: (u64, u64, u64),
        m_owner: RefNo,
        o_members: Vec<RefNo>,
        committed_o: Vec<RefNo>,
        q_holds_m: bool,
        leaf: RefNo,
        owner_o: RefNo,
        owner_q: RefNo,
    }
    let outcome = |flush: bool| -> Outcome {
        let copy = DbCopy::of(AMS8000);
        let cast = cast(AMS8000);
        let (owner_o, owner_q) = (cast.zones[0], cast.zones[1]);
        let s = DbView::open(&copy.path).unwrap().session().sesno;

        // A: move M from P to Q.
        let (owner_p, leaf) = {
            let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
            let (owner_p, leaf) = leaf_under(&mut engine, db1_hash("BRAN"));
            assert!(owner_p != owner_o && owner_p != owner_q);
            let mut m = RecordDraft::from_record_bytes(&engine.find_element(leaf).unwrap().unwrap().raw_bytes).unwrap();
            m.owner = owner_q;
            let mut p = RecordDraft::from_record_bytes(&engine.find_element(owner_p).unwrap().unwrap().raw_bytes).unwrap();
            p.remove_member(leaf);
            let q = zone_with_member(&mut engine, owner_q, leaf);
            write_element_session(
                &copy.path,
                &SessionRequest { user: "A", comment: "A moves M to Q", clock: Some(clock()) },
                &[ElementWrite::Update(m), ElementWrite::Update(p), ElementWrite::Update(q)],
                &[],
            )
            .expect("A writes S+1");
            (owner_p, leaf)
        };
        let committed_o = members_of(&mut ReadOnlyEngine::open(&copy.path).unwrap(), owner_o);

        // B, based on S, moves the same M from P to O and merges.
        let report = {
            let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("base");
            let mut m = RecordDraft::from_record_bytes(&base.find_element(leaf).unwrap().unwrap().raw_bytes).unwrap();
            assert_eq!(m.owner, owner_p, "at S, M is under P");
            m.owner = owner_o;
            let mut p = RecordDraft::from_record_bytes(&base.find_element(owner_p).unwrap().unwrap().raw_bytes).unwrap();
            p.remove_member(leaf);
            let o = zone_with_member(&mut base, owner_o, leaf);
            write_merge_session_with(
                &copy.path,
                &SessionRequest { user: "B", comment: "B moves M to O", clock: Some(clock()) },
                s,
                &[ElementWrite::Update(m), ElementWrite::Update(p), ElementWrite::Update(o)],
                &[],
                MergeOptions { extract_flush: flush, ..Default::default() },
            )
            .expect("B merges S+2")
        };

        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        assert!(!members_of(&mut engine, owner_p).contains(&leaf), "flush={flush}: both sides took M out of P");
        Outcome {
            appended: report.appended_pages.len(),
            changed: tally(&copy.path, s + 1, s + 2),
            m_owner: owner_of(&mut engine, leaf),
            o_members: members_of(&mut engine, owner_o),
            committed_o,
            q_holds_m: members_of(&mut engine, owner_q).contains(&leaf),
            leaf,
            owner_o,
            owner_q,
        }
    };

    // Extract flush: the committed owner Q stands, O does not gain M, and the
    // merge writes nothing — every merged record equals the committed one.
    let flushed = outcome(true);
    assert_eq!(flushed.m_owner, flushed.owner_q, "extract flush: both moved M, the committed owner Q stands");
    assert_eq!(flushed.o_members, flushed.committed_o, "extract flush: M's owner changed under B, so O's list does not gain M");
    assert!(!flushed.o_members.contains(&flushed.leaf));
    assert!(flushed.q_holds_m, "extract flush: Q's committed list keeps M");
    assert_eq!(flushed.appended, 1, "extract flush: only the session page was appended");
    assert_eq!(flushed.changed, (0, 0, 0), "extract flush: S+1..S+2 changed nothing");

    // Plain SAVEWORK: the writer's owner O wins, O's list gains M at the end
    // where B put it, and Q — the owner the committed record named, whose
    // record B never handed in — lets go of M.
    let plain = outcome(false);
    assert_eq!(plain.m_owner, plain.owner_o, "plain: M's owner is B's O");
    assert_eq!(
        plain.o_members,
        [plain.committed_o.as_slice(), &[plain.leaf]].concat(),
        "plain: O's list is the committed one plus M"
    );
    assert!(!plain.q_holds_m, "plain: M is taken out of Q's list (sub_5B211C0), though B never touched Q");
    assert!(plain.appended > 1, "plain: records were written");
    assert_eq!(plain.changed, (0, 0, 3), "plain: M (owner), O (list) and Q (list) changed; P equals the committed P");
}

/// `sub_5B211C0`'s fix-up on its own: an element the merge re-parents is taken
/// out of the member list of the owner the committed record names, whether or
/// not the writer handed that owner's record in, and once only when it did.
///
/// A commits an unrelated change. B, based on S, moves leaf M from P to O —
/// handing in M and O only, or M, P and O — plain or flushed (the committed
/// side left M's owner alone, so both modes take B's owner). Merged: P's list
/// is without M, O's has it, M's owner is O, and exactly M, O and P changed.
#[test]
fn re_parenting_an_element_takes_it_out_of_the_committed_owners_list() {
    if common::missing(&[AMS8000]) {
        return;
    }
    for (flush, hand_in_p) in [(false, false), (true, false), (false, true)] {
        let copy = DbCopy::of(AMS8000);
        let cast = cast(AMS8000);
        let owner_o = cast.zones[0];
        let s = DbView::open(&copy.path).unwrap().session().sesno;
        let case = format!("flush={flush}, hand_in_p={hand_in_p}");

        // A: an unrelated change, so B's write is a real merge.
        let (owner_p, leaf) = {
            let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
            let (owner_p, leaf) = leaf_under(&mut engine, db1_hash("BRAN"));
            assert_ne!(owner_p, owner_o);
            let mut subs = RecordDraft::from_record_bytes(&engine.find_element(cast.subs[0]).unwrap().unwrap().raw_bytes).unwrap();
            *subs.implicit_word_mut(11).unwrap() ^= 1;
            write_element_session(
                &copy.path,
                &SessionRequest { user: "A", comment: "A changes something else", clock: Some(clock()) },
                &[ElementWrite::Update(subs)],
                &[],
            )
            .expect("A writes S+1");
            (owner_p, leaf)
        };
        let committed_p = members_of(&mut ReadOnlyEngine::open(&copy.path).unwrap(), owner_p);
        let committed_o = members_of(&mut ReadOnlyEngine::open(&copy.path).unwrap(), owner_o);
        assert!(committed_p.contains(&leaf));

        // B, based on S, moves M from P to O.
        let report = {
            let mut base = ReadOnlyEngine::open_at(&copy.path, s).expect("base");
            let mut m = RecordDraft::from_record_bytes(&base.find_element(leaf).unwrap().unwrap().raw_bytes).unwrap();
            m.owner = owner_o;
            let mut elements = vec![ElementWrite::Update(m), ElementWrite::Update(zone_with_member(&mut base, owner_o, leaf))];
            if hand_in_p {
                let mut p = RecordDraft::from_record_bytes(&base.find_element(owner_p).unwrap().unwrap().raw_bytes).unwrap();
                p.remove_member(leaf);
                elements.push(ElementWrite::Update(p));
            }
            write_merge_session_with(
                &copy.path,
                &SessionRequest { user: "B", comment: "B moves M to O", clock: Some(clock()) },
                s,
                &elements,
                &[],
                MergeOptions { extract_flush: flush, ..Default::default() },
            )
            .expect("B merges S+2")
        };

        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        assert_eq!(owner_of(&mut engine, leaf), owner_o, "{case}: M's owner is O");
        assert_eq!(
            members_of(&mut engine, owner_o),
            [committed_o.as_slice(), &[leaf]].concat(),
            "{case}: O's list gained M at the end"
        );
        let p_members = members_of(&mut engine, owner_p);
        assert_eq!(
            p_members,
            committed_p.iter().copied().filter(|m| *m != leaf).collect::<Vec<_>>(),
            "{case}: P's list is the committed one without M, the rest in order"
        );
        assert_eq!(tally(&copy.path, s + 1, s + 2), (0, 0, 3), "{case}: exactly M, O and P changed");
        assert!(report.appended_pages.len() > 1, "{case}: records were written");
    }
}

/// A `(owner, leaf)` pair: an element of `owner_noun` with a 0-member child in
/// its list.
fn leaf_under(engine: &mut ReadOnlyEngine, owner_noun: u32) -> (RefNo, RefNo) {
    let mut leaves_of: std::collections::HashMap<RefNo, RefNo> = std::collections::HashMap::new();
    for element in engine.scan_elements(ScanTier::Full).expect("scan") {
        let element = element.expect("element");
        if element.parsed.as_ref().is_some_and(|p| p.members.is_empty()) {
            leaves_of.entry(element.owner).or_insert(element.refno);
        }
    }
    let text = engine.text_format();
    for (owner, leaf) in leaves_of {
        let Some(view) = engine.find_element(owner).ok().flatten() else { continue };
        if view.header().noun_hash != owner_noun {
            continue;
        }
        let parsed = ParsedElement::parse_as(&view.raw_bytes, text).unwrap();
        if parsed.members.contains(&leaf) {
            return (owner, leaf);
        }
    }
    panic!("no leaf under a {owner_noun:#x}");
}

/// A name already stored on some element (with its leading `/`).
fn existing_name(engine: &mut ReadOnlyEngine) -> String {
    let name_hash = db1_hash("NAME");
    let text = engine.text_format();
    let named: Vec<RefNo> = engine
        .scan_elements(ScanTier::Named)
        .expect("scan")
        .filter_map(|e| {
            let e = e.expect("element");
            e.name.map(|_| e.refno)
        })
        .collect();
    for refno in named {
        let view = engine.find_element(refno).unwrap().unwrap();
        let parsed = ParsedElement::parse_as(&view.raw_bytes, text).unwrap();
        if let Some(name) = parsed.explicit_attributes.iter().find_map(|a| match &a.value {
            ExplicitValue::Text(t) if a.hash == name_hash => Some(t.clone()),
            _ => None,
        }) {
            return name;
        }
    }
    panic!("no named element");
}
