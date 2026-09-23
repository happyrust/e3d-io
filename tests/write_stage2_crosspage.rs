//! A single record whose tail blocks do not fit on one page
//! (`docs/plans/2026-09-20-dabacon-write-plan.md` §2 阶段 2b): the writer splits
//! the member and explicit blocks into page-sized segments linked by the
//! `(page, packed)` continuation `record::gather` follows, so the record reads
//! back whole across as many pages as it took. Checked on `ams8000_0001`.

mod common;

use common::write_fixture::{assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS8000};
use e3d_attlib::db1_hash;
use e3d_io::engine::ScanTier;
use e3d_io::record::block::{block_at, BlockKind};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::{write_element_session, ElementWrite, RecordDraft, SessionClock, SessionRequest};
use e3d_io::{ReadOnlyEngine, RefNo};

const PAGE0_COMMIT_WORDS: &[usize] = &[9, 10, 11];

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 21, hour: 0, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 20, hour: 16, minute: 0, second: 0, millisecond: 0 },
    )
}

/// The first ZONE, its members, and the largest word1 in use.
fn first_zone(engine: &mut ReadOnlyEngine) -> (RefNo, Vec<RefNo>, u32) {
    let zone_hash = db1_hash("ZONE");
    let mut zone = None;
    let mut max_word1 = 0u32;
    for element in engine.scan_elements(ScanTier::Header).expect("scan") {
        let element = element.expect("header");
        max_word1 = max_word1.max(element.refno.word1);
        if zone.is_none() && element.noun_hash == zone_hash {
            zone = Some(element.refno);
        }
    }
    let zone = zone.expect("the sample has a ZONE");
    let view = engine.find_element(zone).expect("read zone").expect("zone exists");
    let members = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse").members;
    (zone, members, max_word1)
}

/// A ZONE grown to two thousand members — a member block several pages long —
/// reads back with every member in order. A record the writer failed to split
/// would come back from the reader as `BlockOverflowsPage`, so a clean read of
/// all of them is the proof the continuation chain is sound.
#[test]
fn a_member_list_that_spans_many_pages_reads_back_whole() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    // The sample's pages are 2048 bytes = 512 words, as asserted below.
    let page_words = 512usize;
    let (zone, before, max_word1, base_sesno) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let (zone, members, max_word1) = first_zone(&mut engine);
        let base = DbView::open(&copy.path).unwrap().session().sesno;
        (zone, members, max_word1, base)
    };

    // Append two thousand fresh members — synthetic refnos, which the member
    // block carries verbatim; this test is about the block's on-disk layout,
    // not the members' own records.
    let added: Vec<RefNo> = (0..2000).map(|i| RefNo::new(zone.word0, max_word1 + 10_000 + i)).collect();
    let draft = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let view = engine.find_element(zone).expect("zone").unwrap();
        let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
        for member in &added {
            draft.push_member(*member);
        }
        draft
    };
    // The member block alone is far larger than a page, so the record must span.
    let member_block_words = 5 + (before.len() + added.len()) * 2;
    assert!(member_block_words > page_words * 4, "the member block is meant to span several pages");

    let report = write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io crosspage: a big member list", clock: Some(clock()) },
        &[ElementWrite::Update(draft)],
        &[],
    )
    .expect("write");
    assert_eq!(report.written.sesno, base_sesno + 1);
    // The one record needed many data pages of its own.
    let data_pages = report.appended_pages.len();
    assert!(data_pages >= 8, "a 2000-member record should append many pages, got {data_pages}");

    // Read it back: every member, in order, old ones then new ones.
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open written");
    let view = engine.find_element(zone).expect("lookup").expect("indexed");
    let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse");
    let mut expected = before.clone();
    expected.extend_from_slice(&added);
    assert_eq!(parsed.members.len(), expected.len(), "member count round-trips");
    assert_eq!(parsed.members, expected, "the whole member list survived the split");
    assert_eq!(parsed.owner, view.header().owner, "the header still reads");

    // The range diff sees exactly the one modified element.
    let mut view = DbView::open(&copy.path).expect("reopen");
    let diff = view
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(base_sesno + 1))
        .expect("diff");
    assert_eq!((diff.tally().deleted, diff.tally().inserted, diff.tally().modified), (0, 0, 1));
    drop(view);

    // Bytes: the prefix is untouched and every new page is contiguous.
    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(&copy.original, 2048, 0, 13) as usize;
    let old_last = report.previous.end.page as usize;
    let new_end = *report.appended_pages.iter().max().unwrap() as usize;
    assert_prefix_unchanged(&copy.original, &bytes, page_bytes, old_last, PAGE0_COMMIT_WORDS);
    assert_appended_contiguous(&bytes, page_bytes, old_last, new_end, &[3, 5, 7]);
}

/// A member list past 8 191 members no longer fits the format word's fourteen
/// bits. `core.dll` then writes `0x3FFF` and its reader measures the block
/// chain instead; a count left to overflow (`40 256 & 0x3FFF`) is what made E3D
/// list a 20 128-member ZONE as 3 744 members (plan §1.3 #11). The word must
/// saturate, the explicit count next to it must be untouched, and the whole
/// list must still read back.
#[test]
fn a_member_list_past_the_format_words_reach_saturates_the_count_and_reads_back_whole() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let (zone, before, max_word1, base_sesno, explicit_count_before) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let (zone, members, max_word1) = first_zone(&mut engine);
        let view = engine.find_element(zone).expect("zone").unwrap();
        let word10 = u32::from_be_bytes(view.raw_bytes[40..44].try_into().unwrap());
        let base = DbView::open(&copy.path).unwrap().session().sesno;
        (zone, members, max_word1, base, (word10 >> 14) & 0x3FFF)
    };
    assert!(explicit_count_before > 0, "the sample ZONE carries an explicit stream, so the neighbouring count is visible");

    // Enough members to need more than 0x3FFF member words: 8 200 × 2 = 16 400.
    let wanted = 8_200usize.saturating_sub(before.len());
    let added: Vec<RefNo> = (0..wanted as u32).map(|i| RefNo::new(zone.word0, max_word1 + 40_000 + i)).collect();
    let draft = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let view = engine.find_element(zone).expect("zone").unwrap();
        let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
        for member in &added {
            draft.push_member(*member);
        }
        draft
    };
    let total = before.len() + added.len();
    assert!(total * 2 > 0x3FFF, "the member word count must exceed the field");

    let report = write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io crosspage: a member list past 0x3FFF words", clock: Some(clock()) },
        &[ElementWrite::Update(draft)],
        &[],
    )
    .expect("write");
    assert_eq!(report.written.sesno, base_sesno + 1);

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open written");
    let view = engine.find_element(zone).expect("lookup").expect("indexed");
    let word10 = u32::from_be_bytes(view.raw_bytes[40..44].try_into().unwrap());
    assert_eq!(word10 & 0x3FFF, 0x3FFF, "the member count saturates at 0x3FFF: {word10:#010x}");
    assert_eq!((word10 >> 14) & 0x3FFF, explicit_count_before, "the explicit count next to it is untouched: {word10:#010x}");
    assert_eq!(word10 & 0x2000_0000, 0x2000_0000, "the wide flag stands");
    // The reader follows the block chain, not the count: every member is there.
    let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse");
    let mut expected = before.clone();
    expected.extend_from_slice(&added);
    assert_eq!(parsed.members.len(), total);
    assert_eq!(parsed.members, expected, "the whole list survived the saturated count");
    // A reader that meets 0x3FFF measures the chain, so no segment may carry
    // padding: the member segments' payloads add up to exactly `members × 2`
    // words — every segment is exactly as long as its header says.
    let bytes = &view.raw_bytes;
    let mut at = view.header().implicit_words as usize * 4;
    let mut member_payload_words = 0usize;
    while let Some(block) = block_at(bytes, at, zone) {
        if block.kind == BlockKind::Members {
            member_payload_words += block.payload(at).len() / 4;
        }
        at += block.total_bytes;
    }
    assert_eq!(at, bytes.len(), "the gathered record is exactly its blocks");
    assert_eq!(member_payload_words, total * 2, "no trailing padding in the member chain");
}

/// The explicit side of the same field: a stream of 16 383 words or more
/// states `0x3FFF` too, and `core.dll` then measures the chain — dropping the
/// zero words at the end of its last segment. So a saturated stream whose last
/// data word is zero is refused before anything is written
/// (`WriteError::SaturatedBlockEndsInZero`), and the same stream ending in a
/// non-zero word is written, states `0x3FFF` beside an untouched member count,
/// and reads back whole.
#[test]
fn a_saturated_explicit_stream_is_refused_when_it_ends_in_zero_and_written_otherwise() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let (zone, member_count, max_word1, base_sesno) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let (zone, members, max_word1) = first_zone(&mut engine);
        let base = DbView::open(&copy.path).unwrap().session().sesno;
        (zone, members.len(), max_word1, base)
    };
    let fresh = RefNo::new(zone.word0, max_word1 + 50_000);
    let big = db1_hash("DESP");
    // 16 400 data words + the field's two header words = 16 402 stream words.
    let data: Vec<u32> = (1..=16_400u32).collect();
    let draft_with = |last: u32| {
        let mut data = data.clone();
        *data.last_mut().unwrap() = last;
        let mut implicit = vec![0u32; 11];
        implicit.extend_from_slice(&[0u32; 20]);
        let mut draft = RecordDraft::new(fresh, db1_hash("GROU"), zone, implicit, vec![], vec![]).expect("draft");
        draft.set_explicit_words(big, 7 << 10, data);
        draft
    };
    let request = |comment: &'static str| SessionRequest { user: "e3dio", comment, clock: Some(clock()) };

    // Ending in zero: refused, file untouched.
    let refused = write_element_session(&copy.path, &request("e3d-io crosspage: saturated stream ending in 0"), &[ElementWrite::Insert(draft_with(0))], &[]);
    match refused {
        Err(e3d_io::write::WriteError::SaturatedBlockEndsInZero { refno, block, words }) => {
            assert_eq!((refno, block, words), (fresh, "explicit", 16_402));
        }
        other => panic!("expected SaturatedBlockEndsInZero, got {other:?}"),
    }
    assert_eq!(copy.current_bytes(), copy.original, "a refused write changes no byte");
    assert_eq!(DbView::open(&copy.path).unwrap().session().sesno, base_sesno);

    // Ending in a non-zero word: written, saturated, whole.
    let report = write_element_session(&copy.path, &request("e3d-io crosspage: saturated stream"), &[ElementWrite::Insert(draft_with(1))], &[])
        .expect("write");
    assert_eq!(report.written.sesno, base_sesno + 1);
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open written");
    let view = engine.find_element(fresh).expect("lookup").expect("indexed");
    let word10 = u32::from_be_bytes(view.raw_bytes[40..44].try_into().unwrap());
    assert_eq!((word10 >> 14) & 0x3FFF, 0x3FFF, "the explicit count saturates: {word10:#010x}");
    assert_eq!(word10 & 0x3FFF, 0, "no members, and the member count is not polluted: {word10:#010x}");
    let written = RecordDraft::from_record_bytes(&view.raw_bytes).expect("redraft");
    let (type_code, words) = written.explicit_words(big).expect("the field is there");
    assert_eq!(type_code, 7 << 10);
    assert_eq!(words.len(), 16_400);
    assert_eq!((words[0], words[16_398], words[16_399]), (1, 16_399, 1), "the whole value, its last word included");
    let zone_members = members_of(&mut engine, zone);
    assert_eq!(zone_members.len(), member_count, "the ZONE was not touched by a write that did not name it");
}

fn members_of(engine: &mut ReadOnlyEngine, refno: RefNo) -> Vec<RefNo> {
    let view = engine.find_element(refno).expect("lookup").expect("indexed");
    ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse").members
}

/// A fresh element with both a member block that spans off its start page and
/// an explicit block after it: the explicit block lands on a continuation page,
/// so the header slot that names it is back-patched into the start page after
/// that page was already set aside. Both lists read back whole.
#[test]
fn a_record_that_spans_still_finds_its_explicit_block() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let (zone, max_word1, base_sesno) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let (zone, _, max_word1) = first_zone(&mut engine);
        let base = DbView::open(&copy.path).unwrap().session().sesno;
        (zone, max_word1, base)
    };

    let fresh = RefNo::new(zone.word0, max_word1 + 20_000);
    let members: Vec<RefNo> = (0..1500).map(|i| RefNo::new(zone.word0, max_word1 + 30_000 + i)).collect();
    let desc = db1_hash("DESC");
    let text = "e3d-io crosspage explicit after a long member list";
    let draft = {
        let mut implicit = vec![0u32; 11];
        // A modest implicit region; the point is the tail blocks.
        implicit.extend_from_slice(&[0u32; 20]);
        let mut draft = RecordDraft::new(fresh, db1_hash("GROU"), zone, implicit, members.clone(), vec![]).expect("draft");
        draft.add_explicit_text_ascii(desc, 0x2800, text).expect("desc");
        draft
    };

    write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io crosspage: members then explicit", clock: Some(clock()) },
        &[ElementWrite::Insert(draft)],
        &[],
    )
    .expect("write");

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open written");
    let view = engine.find_element(fresh).expect("lookup").expect("indexed");
    let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse");
    assert_eq!(parsed.members, members, "the member list read back across pages");
    let desc_value = parsed
        .explicit_attributes
        .iter()
        .find(|attr| attr.hash == desc)
        .map(|attr| attr.value.clone());
    assert_eq!(
        desc_value,
        Some(ExplicitValue::Text(text.to_string())),
        "the explicit block, which landed on a continuation page, still reads"
    );

    let mut view = DbView::open(&copy.path).expect("reopen");
    let diff = view
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(base_sesno + 1))
        .expect("diff");
    assert_eq!((diff.tally().deleted, diff.tally().inserted, diff.tally().modified), (0, 1, 0));
}
