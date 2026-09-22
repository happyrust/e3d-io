//! Storage type 19 (string list) written inline, and storage types 20–23 — an
//! int array, a real array, a text and a string list written out of the record
//! as hunks (`docs/plans/2026-09-20-dabacon-write-plan.md` §2 阶段 2c; the
//! layouts in `record::hunk`, read out of `core.dll`'s `4SCEA`/`sub_5AEC110`/
//! `sub_5AEA500`). Checked on a copy of `ams8000_0001`: the record reads back
//! through the ordinary reader with seven-word directory entries in its
//! stream, each hunk reads back through `read_hunk` and decodes to the value
//! set, a hunk precedes its record on disk, a rewrite that leaves a hunk
//! attribute alone keeps pointing at the old hunk, and a merge carries a
//! hunk the writer set.

mod common;

use common::write_fixture::{assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS8000};
use e3d_attlib::db1_hash;
use e3d_io::engine::ScanTier;
use e3d_io::page::cache::PageCache;
use e3d_io::page::io::PageReader;
use e3d_io::page::PageId;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::{ExplicitAttribute, ExplicitValue};
use e3d_io::record::hunk::{decode_hunk_value, decode_text_list, hunk_block_at, read_hunk, HunkRef, HUNK_HEADER_WORDS};
use e3d_io::record::text::TextFormat;
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::record::{text_list_data_ascii, wide_format_word};
use e3d_io::write::{write_element_session, write_merge_session, ElementWrite, RecordDraft, SessionClock, SessionRequest};
use e3d_io::{ReadOnlyEngine, RefNo};

const PAGE0_COMMIT_WORDS: &[usize] = &[9, 10, 11];
/// The sample's pages: 2048 bytes = 512 words (asserted through `page_word`).
const PAGE_WORDS: usize = 512;

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 21, hour: 9, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 21, hour: 1, minute: 0, second: 0, millisecond: 0 },
    )
}

/// The first ZONE and the largest word1 in use.
fn first_zone(engine: &mut ReadOnlyEngine) -> (RefNo, u32) {
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
    (zone.expect("the sample has a ZONE"), max_word1)
}

fn attribute(parsed: &ParsedElement, hash: u32) -> &ExplicitAttribute {
    parsed
        .explicit_attributes
        .iter()
        .find(|attribute| attribute.hash == hash)
        .unwrap_or_else(|| panic!("attribute {hash:#x} is in the stream"))
}

/// The values the test writes.
struct Values {
    keys: Vec<String>,
    vals: Vec<String>,
    ints: Vec<i32>,
    reals: Vec<f64>,
    text: String,
}

fn values() -> Values {
    Values {
        keys: vec!["KEY1".into(), "KEY2".into(), "A LONGER KEY".into(), "".into()],
        // Six hundred items of eight words each: a hunk several pages long.
        vals: (0..600).map(|i| format!("value number {i:05} of the list")).collect(),
        ints: vec![1, -2, 3, i32::MAX, i32::MIN, 0, 42],
        reals: vec![1.5, -2.25, 1.0e6, 0.0, -0.001, 310.0],
        text: "e3d-io: a text moved out of the record into a hunk".into(),
    }
}

/// A fresh GROU under `owner` carrying every kind of value under test, plus
/// one ordinary inline integer so the stream has a plain entry to keep parsing.
fn draft_with_hunks(fresh: RefNo, owner: RefNo, values: &Values) -> RecordDraft {
    let mut implicit = vec![0u32; 11];
    implicit.extend_from_slice(&[0u32; 20]);
    let mut draft = RecordDraft::new(fresh, db1_hash("GROU"), owner, implicit, vec![], vec![]).expect("draft");
    draft.set_explicit_words(db1_hash("NUMB"), 0x0C00, vec![42]);
    draft.set_explicit_text_list_ascii(db1_hash("ROKEYS"), &values.keys).expect("ROKEYS inline");
    draft.set_text_list_hunk_ascii(db1_hash("ROVALS"), &values.vals).expect("ROVALS hunk");
    draft.set_int_array_hunk(db1_hash("IPARAM"), &values.ints).expect("int hunk");
    draft.set_real_array_hunk(db1_hash("PARA"), &values.reals).expect("real hunk");
    draft.set_text_hunk_ascii(db1_hash("DESC"), &values.text).expect("text hunk");
    draft
}

/// Every hunk the record names, read through its directory entry and decoded.
fn check_hunks(path: &std::path::Path, refno: RefNo, parsed: &ParsedElement, values: &Values) -> Vec<(u32, HunkRef)> {
    let mut cache = PageCache::new(PageReader::open(path).expect("open pages"), 64);
    let text = TextFormat::Utf8;
    let mut refs = Vec::new();

    let entry = HunkRef::from_explicit(attribute(parsed, db1_hash("ROVALS"))).expect("ROVALS is a hunk");
    let (expected_words, expected_count) = text_list_data_ascii(&values.vals).unwrap();
    assert_eq!(entry.hunk_type, 23);
    assert_eq!(entry.count, expected_count, "the type-23 count is what db_cxx_put_str_vector reports");
    assert_eq!(entry.words as usize, expected_words.len());
    let payload = read_hunk(&mut cache, PageId::PRIMARY, refno, &entry).expect("read ROVALS");
    assert_eq!(payload, expected_words, "the type-23 payload is the type-19 buffer verbatim");
    assert_eq!(decode_text_list(&payload, text), Some(values.vals.clone()));
    refs.push((db1_hash("ROVALS"), entry));

    let entry = HunkRef::from_explicit(attribute(parsed, db1_hash("IPARAM"))).expect("IPARAM is a hunk");
    assert_eq!((entry.hunk_type, entry.count as usize, entry.words as usize), (20, values.ints.len(), values.ints.len()));
    let payload = read_hunk(&mut cache, PageId::PRIMARY, refno, &entry).expect("read ints");
    assert_eq!(decode_hunk_value(&entry, &payload, text), ExplicitValue::IntArray(values.ints.clone()));
    refs.push((db1_hash("IPARAM"), entry));

    let entry = HunkRef::from_explicit(attribute(parsed, db1_hash("PARA"))).expect("PARA is a hunk");
    assert_eq!((entry.hunk_type, entry.count as usize, entry.words as usize), (21, values.reals.len(), values.reals.len() * 2));
    let payload = read_hunk(&mut cache, PageId::PRIMARY, refno, &entry).expect("read reals");
    assert_eq!(decode_hunk_value(&entry, &payload, text), ExplicitValue::RealArray(values.reals.clone()));
    refs.push((db1_hash("PARA"), entry));

    let entry = HunkRef::from_explicit(attribute(parsed, db1_hash("DESC"))).expect("DESC is a hunk");
    assert_eq!((entry.hunk_type, entry.count as usize, entry.words as usize), (22, values.text.len(), values.text.len().div_ceil(4)));
    let payload = read_hunk(&mut cache, PageId::PRIMARY, refno, &entry).expect("read text");
    assert_eq!(decode_hunk_value(&entry, &payload, text), ExplicitValue::Text(values.text.clone()));
    refs.push((db1_hash("DESC"), entry));

    refs
}

/// A new element with an inline string list and four hunks reads back whole:
/// the stream holds the directory entries, every hunk decodes to the value
/// set, the hunks lie before the record on disk under kind-3 headers naming
/// the element, the attribute and the type, and the long one spans pages.
#[test]
fn hunks_and_a_text_list_read_back_through_their_directory_entries() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let values = values();
    let (zone, max_word1, base_sesno) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let (zone, max_word1) = first_zone(&mut engine);
        let base = DbView::open(&copy.path).unwrap().session().sesno;
        (zone, max_word1, base)
    };
    let fresh = RefNo::new(zone.word0, max_word1 + 40_000);
    let draft = draft_with_hunks(fresh, zone, &values);
    assert_eq!(draft.pending_hunks().len(), 4);

    let report = write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "e3d-io hunks: 19 inline, 20-23 out of record", clock: Some(clock()) },
        &[ElementWrite::Insert(draft)],
        &[],
    )
    .expect("write");
    assert_eq!(report.written.sesno, base_sesno + 1);

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open written");
    let view = engine.find_element(fresh).expect("lookup").expect("indexed");
    let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse");
    assert_eq!(parsed.owner, zone);
    assert_eq!(parsed.explicit_attributes.len(), 6, "NUMB, ROKEYS and four directory entries");

    // The plain entry and the inline list.
    assert_eq!(attribute(&parsed, db1_hash("NUMB")).value, ExplicitValue::Integer(42));
    let keys = attribute(&parsed, db1_hash("ROKEYS"));
    assert_eq!(keys.type_code, 19 << 10, "an inline string list is a type-19 entry");
    let (key_words, _) = text_list_data_ascii(&values.keys).unwrap();
    assert_eq!(keys.data_words as usize, key_words.len(), "its length is the buffer's, with no count word");
    assert_eq!(keys.value, ExplicitValue::TextList(values.keys.clone()), "the stream decoder takes the list apart");
    assert_eq!(decode_text_list(&key_words, TextFormat::Utf8), Some(values.keys.clone()));
    assert!(HunkRef::from_explicit(keys).is_none(), "an inline list is not a hunk");

    // Every directory entry is five words under its hunk type, and each hunk
    // reads back to the value set.
    for hash in [db1_hash("ROVALS"), db1_hash("IPARAM"), db1_hash("PARA"), db1_hash("DESC")] {
        let entry = attribute(&parsed, hash);
        assert_eq!(entry.data_words, 5, "a directory entry is five data words");
        assert!(matches!(&entry.value, ExplicitValue::RawWords(words) if words[0] == 1));
    }
    let hunks = check_hunks(&copy.path, fresh, &parsed, &values);

    // The engine resolves the four directory entries into the values they
    // point at — the entries keep their hunk type codes, only the values
    // change — and the descriptor path then reports each as the decoded
    // explicit value it is, not as five address words.
    {
        let mut resolved = parsed.clone();
        assert_eq!(engine.resolve_hunks(&mut resolved).expect("read every chain"), 4);
        assert_eq!(engine.resolve_hunks(&mut resolved).expect("nothing left to resolve"), 0);
        let expect = |hash: u32, value: ExplicitValue| {
            let entry = attribute(&resolved, hash);
            assert!(e3d_io::record::hunk::is_hunk_type(entry.type_code >> 10), "the type code still says hunk");
            assert_eq!(entry.data_words, 5, "the stream entry is still the five-word directory");
            assert_eq!(entry.value, value, "{hash:#x}");
        };
        expect(db1_hash("ROVALS"), ExplicitValue::TextList(values.vals.clone()));
        expect(db1_hash("IPARAM"), ExplicitValue::IntArray(values.ints.clone()));
        expect(db1_hash("PARA"), ExplicitValue::RealArray(values.reals.clone()));
        expect(db1_hash("DESC"), ExplicitValue::Text(values.text.clone()));
        assert_eq!(attribute(&resolved, db1_hash("ROKEYS")).value, ExplicitValue::TextList(values.keys.clone()));
        assert_eq!(attribute(&resolved, db1_hash("NUMB")).value, ExplicitValue::Integer(42));

        if !common::missing(&[common::DESVIR, common::ATTLIB]) {
            use e3d_io::record::descriptor::{AttributeExtractionStatus, DescriptorValue};
            let attlib = e3d_attlib::AttlibData::parse_file(common::ATTLIB).expect("attlib");
            let mut provider =
                e3d_io::record::template::TemplateProvider::open(common::DESVIR, Some(0x000B_0692)).expect("desvir");
            let extraction = engine
                .extract_element_with_descriptors(fresh, &attlib, &mut provider)
                .expect("extract with the hunks read");
            let wanted: [(u32, DescriptorValue); 5] = [
                (db1_hash("ROKEYS"), DescriptorValue::TextList(values.keys.clone())),
                (db1_hash("ROVALS"), DescriptorValue::TextList(values.vals.clone())),
                (db1_hash("IPARAM"), DescriptorValue::IntArray(values.ints.clone())),
                (db1_hash("PARA"), DescriptorValue::RealArray(values.reals.clone())),
                (db1_hash("DESC"), DescriptorValue::Text(values.text.clone())),
            ];
            let mut checked = 0;
            for row in &extraction.attributes {
                if let Some((_, value)) = wanted.iter().find(|(hash, _)| *hash == row.hash) {
                    assert_eq!(row.status, AttributeExtractionStatus::DecodedExplicit, "{}", row.name);
                    assert_eq!(row.value.as_ref(), Some(value), "{}", row.name);
                    checked += 1;
                }
            }
            assert!(checked >= 1, "the GROU template declares at least one of the five attributes written");
            eprintln!("descriptor path: {checked} of the five hunk / list attributes are in GROU's template and decode");
        }
    }

    // The format word counts the stream with the directory entries in it.
    let stream_words: usize = parsed.explicit_attributes.iter().map(|a| 2 + a.data_words as usize).sum();
    assert_eq!(stream_words, (2 + 1) + (2 + key_words.len()) + 4 * 7);
    let word10 = u32::from_be_bytes(view.raw_bytes[40..44].try_into().unwrap());
    assert_eq!(word10, wide_format_word(0, stream_words));

    // On disk: every hunk is on a page this session appended, before the
    // record, under a kind-3 header that names the element, the attribute and
    // the hunk type; the long string list spans pages.
    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(&copy.original, 2048, 0, 13) as usize;
    assert_eq!(page_bytes, PAGE_WORDS * 4);
    let first_new = report.previous.end.page + 1;
    for (hash, entry) in &hunks {
        let address = entry.address().expect("a placed hunk has an address");
        assert!(address.page_no >= first_new, "hunk of {hash:#x} is on a new page");
        assert!(
            address.page_no <= view.page_no,
            "hunk of {hash:#x} (page {}) is laid down before its record (page {})",
            address.page_no,
            view.page_no
        );
        let page = &bytes[address.page_no as usize * page_bytes..(address.page_no as usize + 1) * page_bytes];
        let block = hunk_block_at(page, address.byte_offset(), fresh).expect("a kind-3 block of this element");
        assert_eq!((block.hash, block.hunk_type), (*hash, entry.hunk_type), "the header names attribute and type");
        let payload_words = (block.total_bytes / 4) - HUNK_HEADER_WORDS;
        if entry.hunk_type == 23 {
            assert!(block.continues_at.is_some(), "six hundred strings do not fit one page");
            assert_eq!(address.byte_offset() + block.total_bytes, page_bytes, "a continuing segment fills its page");
        } else {
            assert!(block.continues_at.is_none());
            assert_eq!(payload_words, entry.words as usize, "a whole hunk is one block of exactly its words");
        }
        // Another element's refno is not this block; a tail-block walk stops here.
        assert!(hunk_block_at(page, address.byte_offset(), zone).is_none());
        assert!(e3d_io::record::block::block_at(page, address.byte_offset(), fresh).is_none());
    }

    // The range diff sees one new element; bytes before the write are untouched
    // and the new pages are contiguous.
    let mut dbview = DbView::open(&copy.path).expect("reopen");
    let diff = dbview
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(base_sesno + 1))
        .expect("diff");
    assert_eq!((diff.tally().deleted, diff.tally().inserted, diff.tally().modified), (0, 1, 0));
    drop(dbview);
    let old_last = report.previous.end.page as usize;
    let new_end = *report.appended_pages.iter().max().unwrap() as usize;
    assert_prefix_unchanged(&copy.original, &bytes, page_bytes, old_last, PAGE0_COMMIT_WORDS);
    assert_appended_contiguous(&bytes, page_bytes, old_last, new_end, &[3, 5, 7]);

    // --- a rewrite that changes one hunk and leaves the others alone ---
    let (new_ints, old_refs) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let view = engine.find_element(fresh).expect("lookup").unwrap();
        let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("redraft");
        assert!(draft.pending_hunks().is_empty(), "placed hunks are not pending");
        let before: Vec<(u32, HunkRef)> = hunks.iter().map(|(hash, _)| (*hash, draft.hunk_ref(*hash).unwrap())).collect();
        let new_ints = vec![7, 8, 9];
        draft.set_int_array_hunk(db1_hash("IPARAM"), &new_ints).expect("new ints");
        draft.set_explicit_words(db1_hash("NUMB"), 0x0C00, vec![43]);
        assert_eq!(draft.pending_hunks().len(), 1);
        write_element_session(
            &copy.path,
            &SessionRequest { user: "e3dio", comment: "e3d-io hunks: one hunk replaced", clock: Some(clock()) },
            &[ElementWrite::Update(draft)],
            &[],
        )
        .expect("rewrite");
        (new_ints, before)
    };
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open rewritten");
    let view = engine.find_element(fresh).expect("lookup").unwrap();
    let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse");
    assert_eq!(attribute(&parsed, db1_hash("NUMB")).value, ExplicitValue::Integer(43));
    let mut cache = PageCache::new(PageReader::open(&copy.path).expect("pages"), 64);
    for (hash, before) in &old_refs {
        let after = HunkRef::from_explicit(attribute(&parsed, *hash)).unwrap();
        if *hash == db1_hash("IPARAM") {
            assert_ne!((after.page_no, after.packed), (before.page_no, before.packed), "the replaced hunk is a new one");
            assert!(after.page_no > before.page_no, "the new hunk is on a page this session appended");
            let payload = read_hunk(&mut cache, PageId::PRIMARY, fresh, &after).expect("read new ints");
            assert_eq!(decode_hunk_value(&after, &payload, TextFormat::Utf8), ExplicitValue::IntArray(new_ints.clone()));
            // The old hunk is still on disk, untouched, and still reads.
            let payload = read_hunk(&mut cache, PageId::PRIMARY, fresh, before).expect("read old ints");
            assert_eq!(decode_hunk_value(before, &payload, TextFormat::Utf8), ExplicitValue::IntArray(values.ints.clone()));
        } else {
            assert_eq!(&after, before, "an untouched hunk attribute keeps pointing at the same hunk");
        }
    }
    check_hunks(&copy.path, fresh, &parsed, &Values { ints: new_ints, ..values });
    let mut dbview = DbView::open(&copy.path).expect("reopen");
    let diff = dbview
        .diff_sessions(SessionSelector::Exact(base_sesno + 1), SessionSelector::Exact(base_sesno + 2))
        .expect("diff");
    assert_eq!((diff.tally().deleted, diff.tally().inserted, diff.tally().modified), (0, 0, 1));
}

/// A hunk a writer sets on an element another writer changed meanwhile comes
/// through the merge: the committed side's inline change and the writer's
/// hunk are both in the merged record, and the hunk is placed and readable.
#[test]
fn a_merge_carries_the_writers_hunk() {
    if common::missing(&[AMS8000]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let values = values();
    let (zone, base_sesno) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let (zone, _) = first_zone(&mut engine);
        (zone, DbView::open(&copy.path).unwrap().session().sesno)
    };
    let numb = db1_hash("NUMB");
    let para = db1_hash("PARA");

    // A commits S+1: an inline integer on the zone.
    {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let view = engine.find_element(zone).unwrap().unwrap();
        let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).unwrap();
        draft.set_explicit_words(numb, 0x0C00, vec![7]);
        write_element_session(
            &copy.path,
            &SessionRequest { user: "A", comment: "A: NUMB", clock: Some(clock()) },
            &[ElementWrite::Update(draft)],
            &[],
        )
        .expect("A writes S+1");
    }
    // B, based on S, sets a real-array hunk on the same zone and merges S+2.
    {
        let mut base = ReadOnlyEngine::open_at(&copy.path, base_sesno).expect("open base");
        let view = base.find_element(zone).unwrap().unwrap();
        let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).unwrap();
        draft.set_real_array_hunk(para, &values.reals).unwrap();
        write_merge_session(
            &copy.path,
            &SessionRequest { user: "B", comment: "B: PARA as a hunk", clock: Some(clock()) },
            base_sesno,
            &[ElementWrite::Update(draft)],
            &[],
        )
        .expect("B merges S+2");
    }

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open merged");
    let view = engine.find_element(zone).unwrap().unwrap();
    let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse");
    assert_eq!(attribute(&parsed, numb).value, ExplicitValue::Integer(7), "A's change survives the merge");
    let entry = HunkRef::from_explicit(attribute(&parsed, para)).expect("B's hunk is in the merged record");
    assert_eq!(entry.hunk_type, 21);
    assert!(entry.address().is_some(), "the merge placed the hunk");
    let mut cache = PageCache::new(PageReader::open(&copy.path).expect("pages"), 64);
    let payload = read_hunk(&mut cache, PageId::PRIMARY, zone, &entry).expect("read the merged hunk");
    assert_eq!(decode_hunk_value(&entry, &payload, TextFormat::Utf8), ExplicitValue::RealArray(values.reals.clone()));
    let mut dbview = DbView::open(&copy.path).expect("reopen");
    let diff = dbview
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(base_sesno + 2))
        .expect("diff");
    assert_eq!((diff.tally().deleted, diff.tally().inserted, diff.tally().modified), (0, 0, 1));
}
