//! S5 of `docs/plans/2026-09-21-write-path-next-steps.md`: text outside ASCII
//! is written the way the database header's `FORMAT` says — UTF-8 as it is
//! under `FORMAT 1`, the project code page in `&~ … &` runs under any other
//! (`record::text::encode_as`, read off `core.dll`'s `sub_54B2CE8`;
//! `docs/evidence/2026-09-22-core-dll-text-charset-write.md`) — for an
//! element's `NAME` and `DESC`, the `NAME` table key and the session's
//! comment alike, and what has no stored form is refused before anything is
//! written.
//!
//! The escaped-format oracle is E3D's own: `TES023` names its `SITE`
//! `/锅炉岛设备`, and the key E3D filed for it is `/&~9xB/5:Ih18 &`.

mod common;

use common::write_fixture::{assert_appended_contiguous, assert_prefix_unchanged, page_word, DbCopy, AMS8000, TES023};
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::engine::{extract_parsed_element_with_descriptors, ScanTier};
use e3d_io::index::encode_name_as;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::TemplateProvider;
use e3d_io::record::text::{pack_words, TextEncodeError, TextFormat};
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::{
    write_element_session, AttributeWriteError, ElementWrite, NamedEdit, NamedEditKind, RecordDraft, SessionClock,
    SessionRequest, WriteError,
};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;
const PAGE0_COMMIT_WORDS: &[usize] = &[9, 10, 11];

fn fixed_clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 22, hour: 11, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 22, hour: 3, minute: 0, second: 0, millisecond: 0 },
    )
}

/// The first `ZONE` of the database and its stored `NAME`.
fn a_named_zone(engine: &mut ReadOnlyEngine) -> (RefNo, String) {
    let zone = db1_hash("ZONE");
    engine
        .scan_elements(ScanTier::Named)
        .expect("scan")
        .map(|element| element.expect("element"))
        .find_map(|element| (element.noun_hash == zone).then(|| element.name.map(|name| (element.refno, name))).flatten())
        .expect("a named ZONE")
}

/// The decoded `DESC` of `refno`, through the read side's descriptor path.
fn decoded_desc(engine: &mut ReadOnlyEngine, refno: RefNo, attlib: &AttlibData, provider: &mut TemplateProvider) -> Option<String> {
    let text = engine.text_format();
    let view = engine.find_element(refno).expect("read").expect("indexed");
    let parsed = ParsedElement::parse_as(&view.raw_bytes, text).expect("parse");
    let template = provider.template_for(parsed.noun_hash).expect("template").expect("ZONE has a template");
    let extraction = extract_parsed_element_with_descriptors(refno, &parsed, attlib, template).expect("extract");
    extraction.attributes.iter().find(|row| row.hash == db1_hash("DESC")).and_then(|row| match &row.value {
        Some(DescriptorValue::Text(text)) => Some(text.clone()),
        _ => None,
    })
}

/// The `DESC` field's stored words: the byte count, then the packed bytes.
fn stored_desc_words(engine: &mut ReadOnlyEngine, refno: RefNo) -> Vec<u32> {
    let view = engine.find_element(refno).expect("read").expect("indexed");
    let draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
    draft.explicit_words(db1_hash("DESC")).expect("DESC is set").1
}

/// Rename `zone` to `name` and describe it as `desc`, in a session commented
/// `comment`; every text goes through the header's `FORMAT`.
fn rename_and_describe(copy: &DbCopy, zone: RefNo, old_name: &str, name: &str, desc: &str, comment: &str) -> u32 {
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let text = engine.text_format();
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let view = engine.find_element(zone).expect("read").expect("indexed");
    let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
    let template = provider.template_for(db1_hash("ZONE")).expect("template").expect("ZONE");
    draft.set_attribute(template, db1_hash("NAME"), &DescriptorValue::Text(name.into()), text).expect("NAME");
    draft.set_attribute(template, db1_hash("DESC"), &DescriptorValue::Text(desc.into()), text).expect("DESC");
    let name_edits = vec![
        NamedEdit { key: encode_name_as(old_name, text).unwrap(), refno: zone, kind: NamedEditKind::Delete },
        NamedEdit { key: encode_name_as(name, text).unwrap(), refno: zone, kind: NamedEditKind::Insert },
    ];
    drop(engine);
    let report = write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment, clock: Some(fixed_clock()) },
        &[ElementWrite::Update(draft)],
        &name_edits,
    )
    .expect("write the session");
    *report.appended_pages.iter().max().unwrap()
}

/// A `FORMAT 0` file: the name and description go in as `&~ … &` runs in the
/// project code page — byte for byte the form E3D filed `/锅炉岛设备` under —
/// the table finds the name, the read side decodes both, the session comment
/// too; a character with no escaped form is refused and nothing is written.
#[test]
fn an_escaped_format_file_names_and_describes_in_the_project_code_page() {
    if common::missing(&[TES023, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(TES023);
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");

    // E3D's own escaped name: the encoder gives the key E3D filed, and the
    // table answers to the name through it.
    let (zone, old_name, site, base_sesno, old_end) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        assert_eq!(engine.text_format(), TextFormat::Escaped, "TES023 is a FORMAT 0 file");
        let key = encode_name_as("/锅炉岛设备", TextFormat::Escaped).unwrap();
        assert_eq!(key.text(), "/&~9xB/5:Ih18 &");
        assert_eq!(key.text_as(TextFormat::Escaped), "/锅炉岛设备");
        let site = db1_hash("SITE");
        let site = engine
            .scan_elements(ScanTier::Named)
            .expect("scan")
            .map(|e| e.expect("element"))
            .find(|e| e.noun_hash == site && e.name.as_deref() == Some("/锅炉岛设备"))
            .expect("the SITE E3D named in Chinese")
            .refno;
        assert_eq!(engine.find_by_name("/锅炉岛设备").expect("lookup"), Some(site), "found under its escaped key");
        let (zone, old_name) = a_named_zone(&mut engine);
        assert!(old_name.is_ascii(), "{old_name}");
        let view = DbView::open(&copy.path).unwrap();
        (zone, old_name, site, view.session().sesno, view.session().end.page)
    };

    let name = "/反应堆厂房-E3DIO";
    let desc = "反应堆厂房 -3.320m~±0.00m";
    let comment = "e3d-io S5：反应堆厂房";
    let new_end = rename_and_describe(&copy, zone, &old_name, name, desc, comment);

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("reopen");
    // The record: `DESC` is the escaped bytes, counted after escaping, and
    // decodes back; `NAME` likewise.
    let stored = b"&~74S&6Q3'7? & -3.320m~&~!@ &0.00m";
    let mut expected = vec![stored.len() as u32];
    expected.extend(pack_words(stored));
    assert_eq!(stored_desc_words(&mut engine, zone), expected);
    assert_eq!(decoded_desc(&mut engine, zone, &attlib, &mut provider).as_deref(), Some(desc));
    // The table: the new name is found through its escaped key, the old one
    // is gone, E3D's own escaped name still answers, and the key's bytes are
    // the record's `NAME` bytes.
    assert_eq!(engine.find_by_name(name).expect("lookup"), Some(zone));
    assert_eq!(engine.find_by_name(&old_name).expect("lookup"), None);
    assert_eq!(engine.find_by_name("/锅炉岛设备").expect("lookup"), Some(site));
    let key = {
        let mut table = engine.name_table().expect("NAME table");
        table
            .iter()
            .map(|e| e.expect("entry"))
            .find(|e| e.refno == zone)
            .expect("the zone is in the table")
            .key
    };
    assert_eq!(key.text(), "/&~74S&6Q3'7? &-E3DIO");
    assert_eq!(key.text_as(TextFormat::Escaped), name);
    let element_name = engine
        .scan_elements(ScanTier::Named)
        .expect("scan")
        .map(|e| e.expect("element"))
        .find(|e| e.refno == zone)
        .and_then(|e| e.name);
    assert_eq!(element_name.as_deref(), Some(name), "the record's NAME reads back as the name");
    // The session page: the comment is escaped there too and reads back.
    let mut view = DbView::open(&copy.path).expect("view");
    assert_eq!(view.session().sesno, base_sesno + 1);
    assert_eq!(view.session().comment, comment);
    let diff = view
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(base_sesno + 1))
        .expect("diff");
    assert_eq!((diff.tally().deleted, diff.tally().inserted, diff.tally().modified), (0, 0, 1));
    drop(view);
    drop(engine);

    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(&copy.original, 2048, 0, 13) as usize;
    assert_prefix_unchanged(&copy.original, &bytes, page_bytes, old_end as usize, PAGE0_COMMIT_WORDS);
    assert_appended_contiguous(&bytes, page_bytes, old_end as usize, new_end as usize, &[3, 5, 7]);

    // What core.dll refuses (error 638), this refuses before writing: `–` is
    // GBK A843, a trail byte below 0xA0, so it has no escaped form.
    let before = copy.current_bytes();
    let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
    let view = engine.find_element(zone).unwrap().unwrap();
    let mut draft = RecordDraft::from_record_bytes(&view.raw_bytes).unwrap();
    let template = provider.template_for(db1_hash("ZONE")).unwrap().unwrap();
    let refused = draft.set_attribute(template, db1_hash("DESC"), &DescriptorValue::Text("a–b".into()), TextFormat::Escaped);
    assert!(
        matches!(refused, Err(AttributeWriteError::Text { source: TextEncodeError::Unmappable { ch: '–', .. }, .. })),
        "{refused:?}"
    );
    assert!(matches!(encode_name_as("/a–b", TextFormat::Escaped), Err(TextEncodeError::Unmappable { .. })));
    drop(engine);
    let refused = write_element_session(
        &copy.path,
        &SessionRequest { user: "e3dio", comment: "a–b", clock: Some(fixed_clock()) },
        &[ElementWrite::Update(draft)],
        &[],
    );
    assert!(matches!(refused, Err(WriteError::Text { field: "DESC", .. })), "{refused:?}");
    assert_eq!(copy.current_bytes(), before, "a refused session leaves the file as it was");
}

/// A `FORMAT 1` file: the same texts go in as their UTF-8 bytes — a character
/// the escaped form has no place for included — and read back the same way.
#[test]
fn a_utf8_file_stores_chinese_text_as_utf8() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let (zone, old_name, base_sesno, old_end) = {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        assert_eq!(engine.text_format(), TextFormat::Utf8, "ams8000 is a FORMAT 1 file");
        let (zone, old_name) = a_named_zone(&mut engine);
        let view = DbView::open(&copy.path).unwrap();
        (zone, old_name, view.session().sesno, view.session().end.page)
    };

    let name = "/梭式泄压阀-E3DIO";
    let desc = "反应堆厂房 – 梭式泄压阀 ±0.00m";
    let comment = "e3d-io S5：UTF-8 原字节";
    let new_end = rename_and_describe(&copy, zone, &old_name, name, desc, comment);

    let mut engine = ReadOnlyEngine::open(&copy.path).expect("reopen");
    let mut expected = vec![desc.len() as u32];
    expected.extend(pack_words(desc.as_bytes()));
    assert_eq!(stored_desc_words(&mut engine, zone), expected, "UTF-8 bytes as they are, counted in bytes");
    assert_eq!(decoded_desc(&mut engine, zone, &attlib, &mut provider).as_deref(), Some(desc));
    assert_eq!(engine.find_by_name(name).expect("lookup"), Some(zone));
    assert_eq!(engine.find_by_name(&old_name).expect("lookup"), None);
    let key = {
        let mut table = engine.name_table().expect("NAME table");
        table.iter().map(|e| e.expect("entry")).find(|e| e.refno == zone).expect("in the table").key
    };
    assert_eq!(key.text(), name, "the key is the UTF-8 bytes");
    assert_eq!(key.bytes(), name.as_bytes());
    let mut view = DbView::open(&copy.path).expect("view");
    assert_eq!(view.session().comment, comment);
    let diff = view
        .diff_sessions(SessionSelector::Exact(base_sesno), SessionSelector::Exact(base_sesno + 1))
        .expect("diff");
    assert_eq!((diff.tally().deleted, diff.tally().inserted, diff.tally().modified), (0, 0, 1));
    drop(view);
    drop(engine);

    let bytes = copy.current_bytes();
    let page_bytes = 4 * page_word(&copy.original, 2048, 0, 13) as usize;
    assert_prefix_unchanged(&copy.original, &bytes, page_bytes, old_end as usize, PAGE0_COMMIT_WORDS);
    assert_appended_contiguous(&bytes, page_bytes, old_end as usize, new_end as usize, &[3, 5, 7]);
}
