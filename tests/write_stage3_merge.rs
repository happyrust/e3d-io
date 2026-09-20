//! Stage 3 of the write plan (`docs/plans/2026-09-20-dabacon-write-plan.md` §2
//! 阶段 3, A3a) on `ams8000_0001`: the three-view conflict scan and the
//! disjoint merge write.
//!
//! The construction is the plan's: C0 is the sample at session S; writer A
//! commits S+1; writer B, still based on S, merges its own disjoint changes
//! onto the file and commits S+2. A clean merge is the union of both; an
//! overlap is refused — 562 a change over a delete, 590 a delete over a
//! change, 561 a name already taken, and (this crate's limit) two changes to
//! one element.

mod common;

use common::write_fixture::{assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS8000};
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::engine::ScanTier;
use e3d_io::index::encode_name;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::record::template::{NounTemplate, TemplateProvider};
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::merge::code;
use e3d_io::write::record::put_real_vec_wide;
use e3d_io::write::{
    write_element_session, write_merge_session, ElementWrite, NamedEdit, NamedEditKind, RecordDraft,
    SessionClock, SessionRequest, WriteError,
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
