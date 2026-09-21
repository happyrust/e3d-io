//! Stage 2 of the write plan (`docs/plans/2026-09-20-dabacon-write-plan.md` §2
//! 阶段 2, 用例 A): three new `BOX`es written under a `ZONE`, with the ZONE's
//! member list grown to hold them. Everything is checked back through the
//! read side — the range diff sees three inserts and one modify, each BOX
//! reads back with the values written into it, the ZONE lists the new members,
//! and the file changed nothing before the previous session's last page bar
//! page 0's commit words.

mod common;

use common::write_fixture::{assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS8000};
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_dehash, db1_hash, lookup_system_name, AttlibData};
use e3d_io::engine::ScanTier;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::{NounTemplate, TemplateProvider};
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::record::{put_real_vec_wide, put_real_wide};
use e3d_io::write::{stage_element_session, write_element_session, ElementWrite, RecordDraft, SessionClock, SessionRequest};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;
const PAGE0_COMMIT_WORDS: &[usize] = &[9, 10, 11];

fn fixed_clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 20, hour: 15, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 20, hour: 7, minute: 0, second: 0, millisecond: 0 },
    )
}

/// The attribute's wide word offset, resolved by the same name the extractor
/// prints — a system name if E3D reserves one, else the dehashed four letters.
fn wide_offset(template: &NounTemplate, name: &str) -> usize {
    template
        .attrs
        .iter()
        .find(|attr| {
            lookup_system_name(attr.hash)
                .map(String::from)
                .or_else(|| db1_dehash(attr.hash))
                .as_deref()
                == Some(name)
        })
        .unwrap_or_else(|| panic!("BOX template has no {name}"))
        .wide
        .location
        .word_offset() as usize
}

struct NewBox {
    refno: RefNo,
    pos: [f64; 3],
    dims: [f64; 3],
}

/// A fresh unnamed BOX under `owner`, POS set so it decodes.
fn fresh_box(template: &NounTemplate, refno: RefNo, owner: RefNo) -> RecordDraft {
    let mut implicit = vec![0u32; 11];
    implicit.extend_from_slice(&template.defaults.implicit);
    put_real_vec_wide(&mut implicit, wide_offset(template, "POS"), &[1.0, 2.0, 3.0]);
    RecordDraft::new(refno, db1_hash("BOX"), owner, implicit, vec![], vec![]).expect("box draft")
}

#[test]
fn three_boxes_and_a_grown_zone_read_back_on_ams8000() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");

    // --- pick a ZONE to own the boxes, and read what it holds now ---
    let (zone_refno, zone_members_before, base_sesno, index_root_before, element_count_before, max_word1) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let zone_hash = db1_hash("ZONE");
        let mut zone = None;
        let mut count = 0usize;
        let mut max_word1 = 0u32;
        for element in engine.scan_elements(ScanTier::Header).expect("scan") {
            let element = element.expect("header");
            count += 1;
            max_word1 = max_word1.max(element.refno.word1);
            if zone.is_none() && element.noun_hash == zone_hash {
                zone = Some(element.refno);
            }
        }
        let zone_refno = zone.expect("the sample has a ZONE");
        let view = engine.find_element(zone_refno).expect("read zone").expect("zone exists");
        let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse zone");
        let sesno = DbView::open(&copy.path).unwrap().session().sesno;
        let root = DbView::open(&copy.path).unwrap().session().index_root;
        (zone_refno, parsed.members, sesno, root, count, max_word1)
    };

    // --- build three fresh BOX records under that ZONE ---
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let box_hash = db1_hash("BOX");
    let box_template = provider.template_for(box_hash).expect("load box").expect("box template").clone();
    let pos_at = wide_offset(&box_template, "POS");
    let xlen_at = wide_offset(&box_template, "XLEN");
    let ylen_at = wide_offset(&box_template, "YLEN");
    let zlen_at = wide_offset(&box_template, "ZLEN");

    let boxes = [
        NewBox { refno: RefNo::new(zone_refno.word0, max_word1 + 1000), pos: [1000.0, 2000.0, 3000.0], dims: [300.0, 400.0, 500.0] },
        NewBox { refno: RefNo::new(zone_refno.word0, max_word1 + 1001), pos: [1500.0, 2500.0, 3500.0], dims: [310.0, 410.0, 510.0] },
        NewBox { refno: RefNo::new(zone_refno.word0, max_word1 + 1002), pos: [1750.0, 2750.0, 3750.0], dims: [320.0, 420.0, 520.0] },
    ];

    // The three refnos must be free.
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        for b in &boxes {
            assert!(engine.find_element(b.refno).expect("lookup").is_none(), "{} must be unused", b.refno);
        }
    }

    let mut elements: Vec<ElementWrite> = Vec::new();
    for b in &boxes {
        let mut implicit = vec![0u32; 11];
        implicit.extend_from_slice(&box_template.defaults.implicit);
        put_real_vec_wide(&mut implicit, pos_at, &b.pos);
        put_real_wide(&mut implicit, xlen_at, b.dims[0]);
        put_real_wide(&mut implicit, ylen_at, b.dims[1]);
        put_real_wide(&mut implicit, zlen_at, b.dims[2]);
        let draft = RecordDraft::new(b.refno, box_hash, zone_refno, implicit, vec![], vec![]).expect("box draft");
        elements.push(ElementWrite::Insert(draft));
    }

    // --- the ZONE, re-emitted with the three boxes appended to its members ---
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let view = engine.find_element(zone_refno).expect("read zone").expect("zone");
        let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("zone draft");
        for b in &boxes {
            draft.push_member(b.refno);
        }
        elements.push(ElementWrite::Update(draft));
    }

    // --- write the session ---
    let report = write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io stage2 three boxes", clock: Some(fixed_clock()) },
        &elements,
        &[],
    )
    .expect("write the element session");
    assert_eq!(report.written.sesno, base_sesno + 1);
    let new_end = *report.appended_pages.iter().max().unwrap();
    assert_eq!(report.written.page.page, new_end, "the session page is the last one written");

    // --- the range diff: three inserts, one modify, nothing deleted ---
    let mut view = DbView::open(&copy.path).expect("reopen");
    assert_eq!(view.session().sesno, base_sesno + 1);
    assert_ne!(view.session().index_root, index_root_before, "the index root moved");
    let diff = view
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(base_sesno + 1))
        .expect("diff");
    let tally = diff.tally();
    assert_eq!(
        (tally.deleted, tally.inserted, tally.modified),
        (0, 3, 1),
        "three new boxes and the modified zone"
    );
    drop(view);

    // --- each box reads back with the values written into it ---
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open written");
    for b in &boxes {
        let view = engine.find_element(b.refno).expect("lookup box").unwrap_or_else(|| panic!("{} indexed", b.refno));
        let header = view.header();
        assert_eq!(header.noun_hash, box_hash, "{} is a BOX", b.refno);
        assert_eq!(header.owner, zone_refno, "{} is owned by the zone", b.refno);

        let extraction = engine
            .extract_element_with_descriptors(b.refno, &attlib, &mut provider)
            .unwrap_or_else(|e| panic!("extract {}: {e:?}", b.refno));
        assert_eq!(extraction.noun_name.as_deref(), Some("BOX"));
        assert_real_vec(&extraction, "POS", &b.pos);
        assert_real(&extraction, "XLEN", b.dims[0]);
        assert_real(&extraction, "YLEN", b.dims[1]);
        assert_real(&extraction, "ZLEN", b.dims[2]);
    }

    // --- the zone now lists the three boxes, at the end, in order ---
    let zone_view = engine.find_element(zone_refno).expect("zone").unwrap();
    let zone = ParsedElement::parse_as(&zone_view.raw_bytes, engine.text_format()).expect("parse zone");
    assert_eq!(zone.members.len(), zone_members_before.len() + 3);
    assert_eq!(&zone.members[..zone_members_before.len()], &zone_members_before[..], "old members kept in order");
    let appended: Vec<RefNo> = zone.members[zone_members_before.len()..].to_vec();
    assert_eq!(appended, boxes.iter().map(|b| b.refno).collect::<Vec<_>>(), "new members appended in order");

    // --- element count, index integrity, key order ---
    let mut count = 0usize;
    let mut previous: Option<RefNo> = None;
    let mut seen_boxes = 0usize;
    for element in engine.scan_elements(ScanTier::Header).expect("scan") {
        let element = element.expect("header");
        if let Some(prev) = previous {
            assert!(
                e3d_io::index::cmp_refno(prev, element.refno) == std::cmp::Ordering::Less,
                "keys strictly ascending: {prev} then {}",
                element.refno
            );
        }
        previous = Some(element.refno);
        if boxes.iter().any(|b| b.refno == element.refno) {
            seen_boxes += 1;
        }
        count += 1;
    }
    assert_eq!(count, element_count_before + 3);
    assert_eq!(seen_boxes, 3, "all three boxes are enumerated");
    for b in &boxes {
        let found = engine.search_index(b.refno).expect("search").expect("indexed");
        assert_eq!(found.entry.refno, b.refno);
    }

    // --- the bytes: prefix untouched but page 0's commit words, contiguous new pages ---
    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(&copy.original, 2048, 0, 13) as usize;
    let old_last = report.previous.end.page as usize;
    assert_prefix_unchanged(&copy.original, &bytes, page_bytes, old_last, PAGE0_COMMIT_WORDS);
    assert_eq!(page_word(&bytes, page_bytes, 0, 10), report.written.page.page);
    assert_appended_contiguous(&bytes, page_bytes, old_last, new_end as usize, &[3, 5, 7]);
}

/// The two-phase element write: after staging, the file still reads as the old
/// session (a crash between the two syncs leaves it there); committing finishes
/// it, and rolling back a fresh staging restores the bytes.
#[test]
fn a_staged_element_write_reads_as_the_old_session_until_commit() {
    if common::missing(&[AMS8000, DESVIR]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let box_template = provider.template_for(db1_hash("BOX")).expect("box").expect("template").clone();
    let cast_zone;
    let fresh;
    let (base_sesno, base_count, before_bytes) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let zone_hash = db1_hash("ZONE");
        let mut zone = None;
        let mut count = 0usize;
        let mut max_word1 = 0u32;
        let mut word0 = 24_384;
        for element in engine.scan_elements(ScanTier::Header).expect("scan") {
            let element = element.expect("header");
            count += 1;
            max_word1 = max_word1.max(element.refno.word1);
            if zone.is_none() && element.noun_hash == zone_hash {
                zone = Some(element.refno);
                word0 = element.refno.word0;
            }
        }
        cast_zone = zone.expect("a ZONE");
        fresh = RefNo::new(word0, max_word1 + 3000);
        (DbView::open(&copy.path).unwrap().session().sesno, count, copy.current_bytes())
    };

    // Stage a box insert + owner update, but do not commit.
    let staged = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let mut zone = RecordDraft::from_record_bytes(&engine.find_element(cast_zone).unwrap().unwrap().raw_bytes).unwrap();
        zone.push_member(fresh);
        stage_element_session(
            &copy.path,
            &SessionRequest { user: "e3dio", comment: "staged box", clock: Some(fixed_clock()) },
            &[ElementWrite::Insert(fresh_box(&box_template, fresh, cast_zone)), ElementWrite::Update(zone)],
            &[],
        )
        .expect("stage")
    };
    assert_eq!(staged.previous().sesno, base_sesno);
    assert_eq!(staged.staged().sesno, base_sesno + 1);
    assert!(staged.appended_pages().len() > 1, "an element write appends record and tree pages, not just the session page");

    // The staged pages are on disk, but page 0 is not: readers still see the
    // old session, and the box is not indexed yet.
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open mid-state");
        assert_eq!(engine.session().sesno, base_sesno, "readers still see the old session");
        assert!(engine.find_element(fresh).unwrap().is_none(), "the box is not indexed until commit");
        assert_eq!(engine.scan_elements(ScanTier::Header).unwrap().count(), base_count);
    }

    // Committing the same staging finishes the write.
    let report = staged.commit().expect("commit");
    assert_eq!(report.written.sesno, base_sesno + 1);
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open after commit");
    assert_eq!(engine.session().sesno, base_sesno + 1);
    assert!(engine.find_element(fresh).unwrap().is_some(), "the box is indexed after commit");

    // A fresh staging rolled back restores the bytes.
    let copy2 = DbCopy::of(AMS8000);
    let staged = {
        let mut engine = ReadOnlyEngine::open(&copy2.path).expect("open");
        let mut zone = RecordDraft::from_record_bytes(&engine.find_element(cast_zone).unwrap().unwrap().raw_bytes).unwrap();
        zone.push_member(fresh);
        stage_element_session(
            &copy2.path,
            &SessionRequest { user: "e3dio", comment: "rolled back", clock: Some(fixed_clock()) },
            &[ElementWrite::Insert(fresh_box(&box_template, fresh, cast_zone)), ElementWrite::Update(zone)],
            &[],
        )
        .expect("stage")
    };
    assert_ne!(copy2.current_bytes().len(), copy2.original.len(), "staging grew the file");
    staged.rollback().expect("rollback");
    assert_eq!(copy2.current_bytes(), copy2.original, "rollback restores the bytes");
    let _ = before_bytes;
}

fn attribute_value<'a>(
    extraction: &'a e3d_io::record::descriptor::ElementExtraction,
    name: &str,
) -> &'a DescriptorValue {
    extraction
        .attributes
        .iter()
        .find(|attr| attr.name == name)
        .and_then(|attr| attr.value.as_ref())
        .unwrap_or_else(|| panic!("no decoded {name}"))
}

fn assert_real(extraction: &e3d_io::record::descriptor::ElementExtraction, name: &str, want: f64) {
    match attribute_value(extraction, name) {
        DescriptorValue::Real(got) => assert!((got - want).abs() < 1e-9, "{name}: {got} != {want}"),
        other => panic!("{name} is {other:?}, not a real"),
    }
}

fn assert_real_vec(extraction: &e3d_io::record::descriptor::ElementExtraction, name: &str, want: &[f64]) {
    match attribute_value(extraction, name) {
        DescriptorValue::RealArray(got) => {
            assert_eq!(got.len(), want.len(), "{name} length");
            for (g, w) in got.iter().zip(want) {
                assert!((g - w).abs() < 1e-9, "{name}: {g} != {w}");
            }
        }
        other => panic!("{name} is {other:?}, not a real array"),
    }
}
