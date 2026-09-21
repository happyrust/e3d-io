//! Stage 2 of the write plan (`docs/plans/2026-09-20-dabacon-write-plan.md` §2
//! 阶段 2, 用例 B/C) on `ams8000_0001`:
//!
//! * **B** — five `SUBS` elements each have one real attribute (`LENG`,
//!   implicit) and one text attribute (its first non-`NAME` explicit text)
//!   changed. The range diff reports five modifies and nothing else, and each
//!   element's `ElementDelta` shows exactly those two changes.
//! * **C** — two leaf elements under one `BRAN` are deleted and dropped from
//!   the owner's member list. The diff reports two deletes and the owner
//!   modified, the deleted keys are gone from the index, and their old records
//!   still read at the addresses they held (the pages are not reused).

mod common;

use std::collections::HashMap;

use common::write_fixture::{assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS8000};
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::engine::ScanTier;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{ElementDelta, DbView, SessionDate, SessionSelector};
use e3d_io::write::{write_element_session, ElementWrite, RecordDraft, SessionClock, SessionRequest};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;
const PAGE0_COMMIT_WORDS: &[usize] = &[9, 10, 11];

fn fixed_clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 20, hour: 15, minute: 30, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 20, hour: 7, minute: 30, second: 0, millisecond: 0 },
    )
}

fn first_non_name_text(parsed: &ParsedElement) -> Option<(u32, String)> {
    let name = db1_hash("NAME");
    parsed.explicit_attributes.iter().find_map(|a| match &a.value {
        ExplicitValue::Text(t) if a.hash != name => Some((a.hash, t.clone())),
        _ => None,
    })
}

#[test]
fn modifying_five_subs_reports_one_real_and_one_text_change_each() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let subs_hash = db1_hash("SUBS");

    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let leng_at = provider
        .template_for(subs_hash)
        .expect("load subs")
        .expect("subs template")
        .attrs
        .iter()
        .find(|a| e3d_attlib::db1_dehash(a.hash).as_deref() == Some("LENG"))
        .expect("SUBS has LENG")
        .wide
        .location
        .word_offset() as usize;

    // Five SUBS that carry a LENG real and a non-NAME text, and the change to
    // make to each: LENG += 1000, the text → a distinct ASCII marker.
    struct Change {
        refno: RefNo,
        text_hash: u32,
        new_leng: f64,
        new_text: String,
    }
    let mut changes: Vec<Change> = Vec::new();
    let (base_sesno, index_root_before, element_count_before, old_end) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let mut count = 0usize;
        let mut picked: Vec<(RefNo, u32)> = Vec::new();
        for element in engine.scan_elements(ScanTier::Full).expect("scan") {
            let element = element.expect("element");
            count += 1;
            if element.noun_hash == subs_hash && picked.len() < 5 {
                if let Some((hash, _)) = element.parsed.as_ref().and_then(first_non_name_text) {
                    picked.push((element.refno, hash));
                }
            }
        }
        assert!(picked.len() >= 5, "need five SUBS with a text attribute, found {}", picked.len());
        for (index, (refno, text_hash)) in picked.into_iter().enumerate() {
            let extraction = engine.extract_element_with_descriptors(refno, &attlib, &mut provider).expect("extract");
            let old_leng = match extraction.attributes.iter().find(|a| a.name == "LENG").and_then(|a| a.value.clone()) {
                Some(DescriptorValue::Real(v)) => v,
                other => panic!("{refno} LENG is {other:?}"),
            };
            changes.push(Change {
                refno,
                text_hash,
                new_leng: old_leng + 1000.0,
                new_text: format!("E3DIO-EDIT-{index}"),
            });
        }
        let view = DbView::open(&copy.path).unwrap();
        (view.session().sesno, view.session().index_root, count, view.session().end.page)
    };

    // Build the five updates.
    let mut elements: Vec<ElementWrite> = Vec::new();
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        for change in &changes {
            let view = engine.find_element(change.refno).expect("read").expect("exists");
            let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
            draft.set_real_wide(leng_at, change.new_leng);
            assert!(draft.set_explicit_text_ascii(change.text_hash, &change.new_text).expect("ascii"), "{} had the text attr", change.refno);
            elements.push(ElementWrite::Update(draft));
        }
    }

    let report = write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io stage2 modify five subs", clock: Some(fixed_clock()) },
        &elements,
        &[],
    )
    .expect("write");
    let new_end = *report.appended_pages.iter().max().unwrap();

    // --- diff: five modifies, nothing added or deleted ---
    let mut view = DbView::open(&copy.path).expect("reopen");
    let text_format = view.text_format();
    let diff = view
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(base_sesno + 1))
        .expect("diff");
    let tally = diff.tally();
    assert_eq!((tally.deleted, tally.inserted, tally.modified), (0, 0, 5));

    // --- each element's delta: LENG's words and exactly the one text ---
    let changed: Vec<_> = view
        .changed_elements(&diff)
        .map(|c| c.expect("changed element"))
        .collect();
    drop(view);
    for change in &changes {
        let element = changed.iter().find(|c| c.refno == change.refno).expect("in the diff");
        let mut delta = ElementDelta::of_as(element, text_format).expect("delta");
        delta.with_attribute_rows(&attlib, &mut provider);
        assert!(delta.owner.is_none(), "{}: owner unchanged", change.refno);
        assert!(delta.members_added.is_empty() && delta.members_removed.is_empty() && !delta.members_reordered);
        assert!(!delta.is_empty(), "{}: something changed", change.refno);
        // The text: exactly one explicit attribute changed, to the marker.
        assert_eq!(delta.explicit_changed.len(), 1, "{}: one explicit change", change.refno);
        assert_eq!(delta.explicit_changed[0].hash, change.text_hash);
        assert_eq!(
            delta.explicit_changed[0].after,
            Some(ExplicitValue::Text(change.new_text.clone()))
        );
        // The real: every changed implicit word is one of LENG's two.
        assert!(!delta.implicit_words_changed.is_empty(), "{}: LENG moved", change.refno);
        for word in &delta.implicit_words_changed {
            assert!(
                word.word == leng_at || word.word == leng_at + 1,
                "{}: implicit word {} changed, not a LENG word",
                change.refno,
                word.word
            );
        }
        // The attribute-row view names LENG changing to the new value.
        let rows = delta.attribute_rows.as_ref().expect("template decoded the rows");
        let leng_row = rows.iter().find(|r| r.name == "LENG").expect("LENG row changed");
        assert_eq!(leng_row.after, Some(DescriptorValue::Real(change.new_leng)));
    }

    // --- read back, count, key order, bytes ---
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    for change in &changes {
        let extraction = engine.extract_element_with_descriptors(change.refno, &attlib, &mut provider).expect("extract");
        let leng = extraction.attributes.iter().find(|a| a.name == "LENG").and_then(|a| a.value.clone());
        assert_eq!(leng, Some(DescriptorValue::Real(change.new_leng)), "{}: LENG read back", change.refno);
    }
    let mut count = 0usize;
    let mut previous: Option<RefNo> = None;
    for element in engine.scan_elements(ScanTier::Header).expect("scan") {
        let element = element.expect("header");
        if let Some(prev) = previous {
            assert!(e3d_io::index::cmp_refno(prev, element.refno) == std::cmp::Ordering::Less);
        }
        previous = Some(element.refno);
        count += 1;
    }
    assert_eq!(count, element_count_before);

    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(&copy.original, 2048, 0, 13) as usize;
    assert_prefix_unchanged(&copy.original, &bytes, page_bytes, old_end as usize, PAGE0_COMMIT_WORDS);
    assert_appended_contiguous(&bytes, page_bytes, old_end as usize, new_end as usize, &[3, 5, 7]);
    let _ = index_root_before;
}

#[test]
fn deleting_two_leaves_removes_them_and_leaves_their_old_records_readable() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let bran_hash = db1_hash("BRAN");

    // An owner BRAN with at least two 0-member children that are in its list.
    let (owner, targets, old_addrs, base_sesno, element_count_before, old_end, old_member_count) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let mut leaves_of: HashMap<RefNo, Vec<RefNo>> = HashMap::new();
        let mut count = 0usize;
        for element in engine.scan_elements(ScanTier::Full).expect("scan") {
            let element = element.expect("element");
            count += 1;
            if element.parsed.as_ref().is_some_and(|p| p.members.is_empty()) {
                leaves_of.entry(element.owner).or_default().push(element.refno);
            }
        }
        // Pick a BRAN owner with >= 2 leaf children.
        let mut chosen = None;
        for (owner, children) in &leaves_of {
            if children.len() < 2 {
                continue;
            }
            let is_bran = engine.find_element(*owner).ok().flatten().is_some_and(|v| v.header().noun_hash == bran_hash);
            if is_bran {
                chosen = Some((*owner, children[0], children[1]));
                break;
            }
        }
        let (owner, t1, t2) = chosen.expect("a BRAN with two leaf children");

        // Confirm both are members of the owner, and record their addresses.
        let owner_view = engine.find_element(owner).expect("owner").expect("exists");
        let owner_parsed = ParsedElement::parse_as(&owner_view.raw_bytes, engine.text_format()).expect("parse owner");
        assert!(owner_parsed.members.contains(&t1) && owner_parsed.members.contains(&t2), "targets are members");
        let addr = |engine: &mut ReadOnlyEngine, refno| {
            let found = engine.search_index(refno).expect("search").expect("indexed");
            (found.entry.page_no, found.entry.offset_words)
        };
        let old_addrs = [addr(&mut engine, t1), addr(&mut engine, t2)];
        let view = DbView::open(&copy.path).unwrap();
        (owner, [t1, t2], old_addrs, view.session().sesno, count, view.session().end.page, owner_parsed.members.len())
    };

    // Delete the two, and re-emit the owner without them.
    let mut elements: Vec<ElementWrite> = vec![
        ElementWrite::Delete(targets[0]),
        ElementWrite::Delete(targets[1]),
    ];
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let owner_view = engine.find_element(owner).expect("owner").expect("exists");
        let mut draft = RecordDraft::from_record_bytes(&owner_view.raw_bytes).expect("owner draft");
        assert!(draft.remove_member(targets[0]) && draft.remove_member(targets[1]));
        elements.push(ElementWrite::Update(draft));
    }

    let report = write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io stage2 delete two leaves", clock: Some(fixed_clock()) },
        &elements,
        &[],
    )
    .expect("write");
    let new_end = *report.appended_pages.iter().max().unwrap();

    // --- diff: two deletes, the owner modified ---
    let mut view = DbView::open(&copy.path).expect("reopen");
    let diff = view
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(base_sesno + 1))
        .expect("diff");
    let tally = diff.tally();
    assert_eq!((tally.deleted, tally.inserted, tally.modified), (2, 0, 1));
    drop(view);

    // --- the keys are gone; the owner no longer lists them; old records stand ---
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    for (target, (page, offset)) in targets.iter().zip(old_addrs) {
        assert!(engine.find_element(*target).expect("lookup").is_none(), "{target} is gone from the index");
        let old = engine.read_element_direct(*target, page, offset).expect("old record still on disk");
        assert_eq!(old.header().refno, *target, "the old record bytes are intact");
    }
    let owner_view = engine.find_element(owner).expect("owner").expect("exists");
    let owner_parsed = ParsedElement::parse_as(&owner_view.raw_bytes, engine.text_format()).expect("parse owner");
    assert_eq!(owner_parsed.members.len(), old_member_count - 2);
    assert!(!owner_parsed.members.contains(&targets[0]) && !owner_parsed.members.contains(&targets[1]));

    let mut count = 0usize;
    let mut previous: Option<RefNo> = None;
    for element in engine.scan_elements(ScanTier::Header).expect("scan") {
        let element = element.expect("header");
        if let Some(prev) = previous {
            assert!(e3d_io::index::cmp_refno(prev, element.refno) == std::cmp::Ordering::Less);
        }
        previous = Some(element.refno);
        count += 1;
    }
    assert_eq!(count, element_count_before - 2);

    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(&copy.original, 2048, 0, 13) as usize;
    assert_prefix_unchanged(&copy.original, &bytes, page_bytes, old_end as usize, PAGE0_COMMIT_WORDS);
    assert_appended_contiguous(&bytes, page_bytes, old_end as usize, new_end as usize, &[3, 5, 7]);
}
