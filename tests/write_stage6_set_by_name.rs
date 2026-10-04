//! T1b (`docs/plans/2026-09-22-write-path-phase-2-plan.md` §2): an attribute set
//! **by name from its textual form** — `Transaction::set(refno, "DESC",
//! "反应堆厂房")`, `set(refno, "POS", "1000 2000 3000")`, `set(refno, "CREF",
//! "/E3DIO-SET-BOX")` — reads back through the descriptors as the value the text
//! names, for every storage shape the write path knows. Given E3D's dictionary
//! (`schema/e3d31/catalog/noun_attr_fields.json`), a set is held to E3D's own
//! limits before anything reaches the record: the character count of a text
//! (not the template's byte capacity), the element count of an array, an
//! attribute the dictionary does not have, a set-once attribute, a pseudo
//! attribute. Without the dictionary, only the template's shape and capacity
//! apply — the behaviour of every earlier stage.

mod common;

use common::write_fixture::{DbCopy, AMS8000};
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::engine::ScanTier;
use e3d_io::record::descriptor::{DescriptorValue, ElementExtraction};
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::{
    Dictionary, DictionaryError, SessionClock, SessionRequest, Transaction, TransactionError, ValueParseError,
};
use e3d_io::{ReadOnlyEngine, RefNo};

use std::path::Path;

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

/// The live dictionary export the schema pack carries.
const DICTIONARY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/catalog/noun_attr_fields.json");

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 23, hour: 15, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 23, hour: 7, minute: 0, second: 0, millisecond: 0 },
    )
}

fn provider() -> TemplateProvider {
    TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir")
}

fn request(comment: &'static str) -> SessionRequest<'static> {
    SessionRequest { user: "e3dio", comment, clock: Some(clock()) }
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

/// An element as the read side decodes it, attlib and templates at hand.
fn extract(path: &Path, refno: RefNo) -> ElementExtraction {
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let mut prov = provider();
    engine.extract_element_with_descriptors(refno, &attlib, &mut prov).expect("extract")
}

fn value(extraction: &ElementExtraction, name: &str) -> Option<DescriptorValue> {
    extraction.attributes.iter().find(|a| a.name == name).and_then(|a| a.value.clone())
}

/// A one-word integer as the read side hands it over — `Int`, the raw `Word`
/// of a slot attlib gives no type for, or one raw word of an explicit field.
fn int(extraction: &ElementExtraction, name: &str) -> Option<i64> {
    match value(extraction, name)? {
        DescriptorValue::Int(v) => Some(i64::from(v)),
        DescriptorValue::Word { raw, .. } => Some(i64::from(raw as i32)),
        DescriptorValue::RawWords(words) if words.len() == 1 => Some(i64::from(words[0] as i32)),
        _ => None,
    }
}

/// An integer array as the read side hands it over — `IntArray`, or the raw
/// words of an explicit type-7 field it does not interpret.
fn ints(extraction: &ElementExtraction, name: &str) -> Option<Vec<i64>> {
    match value(extraction, name)? {
        DescriptorValue::IntArray(items) => Some(items.into_iter().map(i64::from).collect()),
        DescriptorValue::RawWords(words) => Some(words.into_iter().map(|w| i64::from(w as i32)).collect()),
        _ => None,
    }
}

fn named(path: &Path, name: &str) -> Option<RefNo> {
    ReadOnlyEngine::open(path).expect("open").find_by_name(name).expect("lookup")
}

fn latest(path: &Path) -> u32 {
    DbView::open(path).expect("open").session().sesno
}

/// `(deleted, inserted, modified)` between two sessions.
fn tally(path: &Path, from: u32, to: u32) -> (u64, u64, u64) {
    let mut view = DbView::open(path).expect("open");
    let diff = view.diff_sessions(SessionSelector::Exact(from), SessionSelector::Exact(to)).expect("diff");
    let tally = diff.tally();
    (tally.deleted, tally.inserted, tally.modified)
}

/// Every storage shape, set from its text on elements inserted in the same
/// transaction and on a stored one, reads back as the value the text names:
/// text (trimmed, quotes stripped), a real with a unit, a real array split on
/// blanks and commas, a word by its letters, a bool, an int, an int array, a
/// string list on `|`, a reference by `=w0/w1` and by `/NAME`, and `unset`.
/// `NAME` set this way keeps the `NAME` table in step.
#[test]
fn every_shape_set_by_name_reads_back_as_the_value_it_names() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let zone = first_zone(&copy.path);
    let s0 = latest(&copy.path);

    // --- S+1: three new elements, every value by name ---
    let (equi, the_box, fixing) = {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let equi = tx.insert(zone, db1_hash("EQUI"), Some("/E3DIO-SET-EQUI"), |_, _| Ok(())).expect("equi");
        tx.set(equi, "DESC", "  反应堆厂房 -3.320m~±0.00m  ").expect("DESC");
        tx.set(equi, "FUNC", "'PUMP FEED'").expect("FUNC");
        tx.set(equi, "POS", "1000, 2000 3000").expect("POS");
        tx.set(equi, "PURP", "pipe").expect("PURP");
        tx.set(equi, "UUIDIA", "11 12 13 14").expect("UUIDIA");

        let the_box = tx.insert(zone, db1_hash("BOX"), None, |_, _| Ok(())).expect("box");
        tx.set(the_box, "XLEN", "300mm").expect("XLEN");
        tx.set(the_box, "YLEN", "4e2").expect("YLEN");
        tx.set(the_box, "ZLEN", "500").expect("ZLEN");
        tx.set(the_box, "POS", "1 2 3").expect("POS");
        tx.set(the_box, "NAME", "/E3DIO-SET-BOX").expect("NAME");

        let fixing = tx.insert(zone, db1_hash("FIXING"), None, |_, _| Ok(())).expect("fixing");
        tx.set(fixing, "ROKEYS", "K1|K2| K3 ").expect("ROKEYS");
        tx.set(fixing, "BUIL", "TRUE").expect("BUIL");
        tx.set(fixing, "NUMB", "42").expect("NUMB");
        tx.set(fixing, "CREF", &format!("={}/{}", zone.word0, zone.word1)).expect("CREF by refno");
        tx.commit(&request("e3d-io T1b: every shape by name")).expect("commit");
        (equi, the_box, fixing)
    };
    let s1 = latest(&copy.path);
    assert_eq!(s1, s0 + 1);
    assert_eq!(tally(&copy.path, s0, s1), (0, 3, 1), "three inserts and the ZONE's list");

    let equi_x = extract(&copy.path, equi);
    assert_eq!(equi_x.owner, zone);
    assert_eq!(value(&equi_x, "NAME"), Some(DescriptorValue::Text("/E3DIO-SET-EQUI".into())));
    assert_eq!(value(&equi_x, "DESC"), Some(DescriptorValue::Text("反应堆厂房 -3.320m~±0.00m".into())), "trimmed");
    assert_eq!(value(&equi_x, "FUNC"), Some(DescriptorValue::Text("PUMP FEED".into())), "one layer of quotes stripped");
    assert_eq!(value(&equi_x, "POS"), Some(DescriptorValue::RealArray(vec![1000.0, 2000.0, 3000.0])));
    match value(&equi_x, "PURP") {
        Some(DescriptorValue::Word { raw, text }) => {
            assert_eq!(raw, db1_hash("PIPE"), "a word is its base-27 hash, upper-cased");
            assert_eq!(text.as_deref(), Some("PIPE"));
        }
        other => panic!("PURP reads back as {other:?}"),
    }
    assert_eq!(ints(&equi_x, "UUIDIA"), Some(vec![11, 12, 13, 14]));

    let box_x = extract(&copy.path, the_box);
    assert_eq!(value(&box_x, "NAME"), Some(DescriptorValue::Text("/E3DIO-SET-BOX".into())));
    assert_eq!(value(&box_x, "XLEN"), Some(DescriptorValue::Real(300.0)), "a trailing unit is tolerated");
    assert_eq!(value(&box_x, "YLEN"), Some(DescriptorValue::Real(400.0)));
    assert_eq!(value(&box_x, "ZLEN"), Some(DescriptorValue::Real(500.0)));
    assert_eq!(value(&box_x, "POS"), Some(DescriptorValue::RealArray(vec![1.0, 2.0, 3.0])));
    assert_eq!(named(&copy.path, "/E3DIO-SET-BOX"), Some(the_box), "NAME by name keeps the table in step");
    assert_eq!(named(&copy.path, "/E3DIO-SET-EQUI"), Some(equi));

    let fixing_x = extract(&copy.path, fixing);
    assert_eq!(
        value(&fixing_x, "ROKEYS"),
        Some(DescriptorValue::TextList(vec!["K1".into(), "K2".into(), "K3".into()])),
        "items trimmed"
    );
    assert_eq!(value(&fixing_x, "BUIL"), Some(DescriptorValue::Bool(true)));
    assert_eq!(int(&fixing_x, "NUMB"), Some(42));
    assert_eq!(value(&fixing_x, "CREF"), Some(DescriptorValue::RefNo(zone)));

    // --- S+2: a reference by /NAME (the box named above), a stored element's
    // DESC, and two values unset ---
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.set(fixing, "CREF", "/E3DIO-SET-BOX").expect("CREF by name");
        tx.set(zone, "DESC", "e3d-io T1b: a stored element set by name").expect("ZONE DESC");
        tx.set(equi, "DESC", "unset").expect("unset DESC");
        tx.set(equi, "UUIDIA", "UNSET").expect("unset UUIDIA");
        tx.commit(&request("e3d-io T1b: by name, unset")).expect("commit");
    }
    let s2 = latest(&copy.path);
    assert_eq!(tally(&copy.path, s1, s2), (0, 0, 3), "the FIXING, the ZONE and the EQUI");
    assert_eq!(value(&extract(&copy.path, fixing), "CREF"), Some(DescriptorValue::RefNo(the_box)), "/NAME resolved through the NAME table");
    assert_eq!(
        value(&extract(&copy.path, zone), "DESC"),
        Some(DescriptorValue::Text("e3d-io T1b: a stored element set by name".into()))
    );
    let equi_x = extract(&copy.path, equi);
    assert!(
        matches!(value(&equi_x, "DESC"), None | Some(DescriptorValue::Unset)),
        "DESC is unset again: {:?}",
        value(&equi_x, "DESC")
    );
    assert_ne!(ints(&equi_x, "UUIDIA"), Some(vec![11, 12, 13, 14]), "UUIDIA no longer holds the value");
    assert_eq!(value(&equi_x, "FUNC"), Some(DescriptorValue::Text("PUMP FEED".into())), "the rest of the record stands");
}

/// With the dictionary, a set is held to what E3D allows — the gate's rules:
/// a text to `SIZE` **characters** (`DESC` 120, where the template's 480 bytes
/// would take more), an array to `SIZE` elements (`POS` 3), an attribute the
/// dictionary lacks refused (`RATM`), a set-once attribute (`UUIDIA`) to one
/// setting — in the same transaction or a later one, until it is unset — and
/// a pseudo attribute (`USRWEI`) to none. Each refusal leaves the draft as it
/// was; what passed is what is written. Without the dictionary the same
/// transaction takes the 121 characters, as every earlier stage did.
#[test]
fn the_dictionary_holds_a_set_to_what_e3d_allows() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB, DICTIONARY]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let zone = first_zone(&copy.path);
    let dictionary = Dictionary::read(DICTIONARY).expect("the pack's dictionary export");
    assert!(dictionary.len() > 1_000, "the live export has {} attributes", dictionary.len());
    assert_eq!(dictionary.entry(db1_hash("DESC")).and_then(|e| e.size), Some(120));
    assert_eq!(dictionary.entry(db1_hash("POS")).and_then(|e| e.size), Some(3));
    assert!(dictionary.entry(db1_hash("RATM")).is_none(), "RATM is what E3D says it does not have");
    let text = |n: usize| "x".repeat(n);
    let s0 = latest(&copy.path);

    let equi = {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off).with_dictionary(&dictionary);
        let equi = tx.insert(zone, db1_hash("EQUI"), None, |_, _| Ok(())).expect("equi");
        // ② a text: SIZE characters, though the template would take 480 bytes.
        assert!(matches!(
            tx.set(equi, "DESC", &text(121)),
            Err(TransactionError::Dictionary(DictionaryError::TextTooLong { chars: 121, max: 120, .. }))
        ));
        tx.set(equi, "DESC", &text(120)).expect("120 characters pass");
        // ③ an array: SIZE elements.
        assert!(matches!(
            tx.set(equi, "POS", "1 2 3 4"),
            Err(TransactionError::Dictionary(DictionaryError::TooManyElements { count: 4, max: 3, .. }))
        ));
        tx.set(equi, "POS", "1 2 3").expect("three pass");
        // ① not in the dictionary, though the template stores it.
        assert!(matches!(
            tx.set(equi, "RATM", "1"),
            Err(TransactionError::Dictionary(DictionaryError::NotInDictionary { .. }))
        ));
        // ④ set once; pseudo.
        tx.set(equi, "UUIDIA", "1 2 3 4").expect("the first UUIDIA");
        assert!(matches!(
            tx.set(equi, "UUIDIA", "5 6 7 8"),
            Err(TransactionError::Dictionary(DictionaryError::SetOnce { .. }))
        ));
        assert!(matches!(
            tx.set(equi, "USRWEI", "1"),
            Err(TransactionError::Dictionary(DictionaryError::Pseudo { .. }))
        ));
        tx.commit(&request("e3d-io T1b: within the dictionary")).expect("commit");
        equi
    };
    let s1 = latest(&copy.path);
    assert_eq!(tally(&copy.path, s0, s1), (0, 1, 1));
    let x = extract(&copy.path, equi);
    assert_eq!(value(&x, "DESC"), Some(DescriptorValue::Text(text(120))), "what passed is what was written");
    assert_eq!(value(&x, "POS"), Some(DescriptorValue::RealArray(vec![1.0, 2.0, 3.0])));
    assert_eq!(ints(&x, "UUIDIA"), Some(vec![1, 2, 3, 4]));
    assert_ne!(int(&x, "RATM"), Some(1), "the refused RATM was not written: {:?}", value(&x, "RATM"));

    // Set once holds across transactions; `unset` is not a setting, and once
    // unset the attribute takes a value again.
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off).with_dictionary(&dictionary);
        assert!(matches!(
            tx.set(equi, "UUIDIA", "5 6 7 8"),
            Err(TransactionError::Dictionary(DictionaryError::SetOnce { .. }))
        ));
        tx.set(equi, "UUIDIA", "unset").expect("unsetting is allowed");
        tx.set(equi, "UUIDIA", "5 6 7 8").expect("and then it is unset, so it may be set");
        tx.commit(&request("e3d-io T1b: set once, unset, set")).expect("commit");
    }
    assert_eq!(ints(&extract(&copy.path, equi), "UUIDIA"), Some(vec![5, 6, 7, 8]));

    // Without the dictionary only the template speaks: 121 characters go in,
    // a fourth POS element is still refused, by the descriptor this time.
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.set(equi, "DESC", &text(121)).expect("the template allows 480 bytes");
        assert!(matches!(tx.set(equi, "POS", "1 2 3 4"), Err(TransactionError::Attribute(_))));
        tx.set(equi, "RATM", "1").expect("the template stores RATM");
        tx.commit(&request("e3d-io T1b: the template alone")).expect("commit");
    }
    let x = extract(&copy.path, equi);
    assert_eq!(value(&x, "DESC"), Some(DescriptorValue::Text(text(121))));
    assert_eq!(int(&x, "RATM"), Some(1), "stored, as E3D's own RATM is — it is E3D that will not read it");
}

/// A name given through `set` or `rename` supersedes what the transaction had
/// queued for the element: an insert renamed before commit files its final
/// name only, two renames file one net change, a rename back to the stored
/// name files none, and a rename followed by a delete leaves no key at all.
#[test]
fn a_name_set_twice_in_one_transaction_files_the_last_one_only() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let zone = first_zone(&copy.path);
    let box_hash = db1_hash("BOX");

    // Inserted as FIRST, renamed to SECOND before the commit.
    let the_box = {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let the_box = tx.insert(zone, box_hash, Some("/E3DIO-N-FIRST"), |_, _| Ok(())).expect("box");
        tx.set(the_box, "NAME", "/E3DIO-N-SECOND").expect("rename the pending insert");
        tx.commit(&request("e3d-io T1b: insert, rename")).expect("commit");
        the_box
    };
    assert_eq!(named(&copy.path, "/E3DIO-N-FIRST"), None, "the first name was never filed");
    assert_eq!(named(&copy.path, "/E3DIO-N-SECOND"), Some(the_box));
    assert_eq!(value(&extract(&copy.path, the_box), "NAME"), Some(DescriptorValue::Text("/E3DIO-N-SECOND".into())));

    // Renamed twice: one net change.
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.rename(the_box, "/E3DIO-N-THIRD").expect("rename");
        tx.set(the_box, "NAME", "/E3DIO-N-FOURTH").expect("rename again");
        tx.commit(&request("e3d-io T1b: rename twice")).expect("commit");
    }
    assert_eq!(named(&copy.path, "/E3DIO-N-SECOND"), None);
    assert_eq!(named(&copy.path, "/E3DIO-N-THIRD"), None, "the name passed through was never filed");
    assert_eq!(named(&copy.path, "/E3DIO-N-FOURTH"), Some(the_box));

    // Renamed and renamed back: the table is not touched, the record is rewritten as it was.
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.rename(the_box, "/E3DIO-N-FIFTH").expect("rename");
        tx.rename(the_box, "/E3DIO-N-FOURTH").expect("and back");
        tx.commit(&request("e3d-io T1b: rename and back")).expect("commit");
    }
    assert_eq!(named(&copy.path, "/E3DIO-N-FIFTH"), None);
    assert_eq!(named(&copy.path, "/E3DIO-N-FOURTH"), Some(the_box));

    // Unset through `set`, then named again in a later transaction.
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.set(the_box, "NAME", "unset").expect("unname");
        tx.commit(&request("e3d-io T1b: unname")).expect("commit");
    }
    assert_eq!(named(&copy.path, "/E3DIO-N-FOURTH"), None, "the key went with the name");
    assert!(matches!(value(&extract(&copy.path, the_box), "NAME"), None | Some(DescriptorValue::Unset)));
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.set(the_box, "NAME", "/E3DIO-N-SIXTH").expect("name again");
        tx.commit(&request("e3d-io T1b: name again")).expect("commit");
    }
    assert_eq!(named(&copy.path, "/E3DIO-N-SIXTH"), Some(the_box));

    // Renamed, then deleted: no key survives.
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        tx.rename(the_box, "/E3DIO-N-SEVENTH").expect("rename");
        tx.delete(the_box).expect("delete");
        tx.commit(&request("e3d-io T1b: rename, delete")).expect("commit");
    }
    assert_eq!(named(&copy.path, "/E3DIO-N-SIXTH"), None);
    assert_eq!(named(&copy.path, "/E3DIO-N-SEVENTH"), None, "a deleted element's queued name is withdrawn");
    assert!(ReadOnlyEngine::open(&copy.path).unwrap().find_element(the_box).unwrap().is_none());
}

/// What has no reading is refused with the draft untouched: an attribute the
/// noun's template lacks, a text that is not a value of the attribute's shape,
/// a `/NAME` nobody has, an element deleted in the transaction — and none of
/// it reaches the file.
#[test]
fn what_cannot_be_read_is_refused_and_writes_nothing() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let zone = first_zone(&copy.path);
    let the_box = {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let the_box = tx.insert(zone, db1_hash("BOX"), Some("/E3DIO-REFUSE"), |_, _| Ok(())).expect("box");
        tx.set(the_box, "XLEN", "1").expect("XLEN");
        tx.commit(&request("e3d-io T1b: a box to refuse on")).expect("commit");
        the_box
    };
    let bytes_before = copy.current_bytes();
    let sesno_before = latest(&copy.path);
    {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        assert!(
            matches!(tx.set(the_box, "NOPE", "1"), Err(TransactionError::UnknownAttribute { name, .. }) if name == "NOPE"),
            "a name no attribute has"
        );
        assert!(
            matches!(tx.set(the_box, "DESC", "x"), Err(TransactionError::UnknownAttribute { .. })),
            "an attribute of another noun"
        );
        assert!(
            matches!(
                tx.set(the_box, "XLEN", "three hundred"),
                Err(TransactionError::Value { name, input, source: ValueParseError::Token { .. } }) if name == "XLEN" && input == "three hundred"
            ),
            "a text that is not a number"
        );
        assert!(
            matches!(tx.set(the_box, "XLEN", ""), Err(TransactionError::Value { source: ValueParseError::Empty, .. })),
            "an empty text"
        );
        assert!(
            matches!(tx.set(the_box, "POS", "1 2 three"), Err(TransactionError::Value { source: ValueParseError::Token { token, .. }, .. }) if token == "three")
        );
        assert!(
            matches!(tx.set(the_box, "PURP", "P1"), Err(TransactionError::Value { source: ValueParseError::NotAWord { .. }, .. })),
            "a word must be letters"
        );
        let fixing = tx.insert(zone, db1_hash("FIXING"), None, |_, _| Ok(())).expect("fixing");
        assert!(
            matches!(
                tx.set(fixing, "CREF", "/E3DIO-NOBODY"),
                Err(TransactionError::Value { source: ValueParseError::NameNotFound { name }, .. }) if name == "/E3DIO-NOBODY"
            ),
            "a name nobody has"
        );
        assert!(
            matches!(tx.set(fixing, "CREF", "24384"), Err(TransactionError::Value { source: ValueParseError::Token { .. }, .. })),
            "half a refno"
        );
        tx.delete(the_box).expect("delete");
        assert!(
            matches!(tx.set(the_box, "XLEN", "2"), Err(TransactionError::Deleted { what: "set", .. })),
            "a deleted element takes nothing"
        );
        assert!(matches!(tx.set(RefNo::new(zone.word0, 0x7FFF_FFF0), "XLEN", "2"), Err(TransactionError::NotFound { .. })));
        // Dropped without a commit.
    }
    assert_eq!(copy.current_bytes(), bytes_before, "a refused set writes nothing");
    assert_eq!(latest(&copy.path), sesno_before);
    assert_eq!(value(&extract(&copy.path, the_box), "XLEN"), Some(DescriptorValue::Real(1.0)), "the box is as it was");
}

/// `set_hunk` stores a value set by name **out of the record**: the explicit
/// stream carries a hunk directory entry of the type the shape picks (22 for a
/// text, 21 for reals, 20 for ints, 23 for a string list) and the value reads
/// back through the descriptors — which resolve the chain — as the text names
/// it. The dictionary applies as to `set`. What is not a hunk's to hold is
/// refused with nothing written: a scalar, a reference, `unset`, `NAME`, an
/// attribute with an implicit slot, more elements than the descriptor counts.
#[test]
fn a_value_set_by_name_as_a_hunk_is_stored_out_of_the_record_and_reads_back() {
    use e3d_io::record::element::ParsedElement;
    use e3d_io::record::hunk::HunkRef;
    use e3d_io::write::AttributeWriteError;

    if common::missing(&[AMS8000, DESVIR, ATTLIB, DICTIONARY]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let zone = first_zone(&copy.path);
    let dictionary = Dictionary::read(DICTIONARY).expect("dictionary");
    let mut desc = String::from("e3d-io T1d: DESC set by name and stored as a type-22 hunk ");
    while desc.len() < 120 {
        desc.push('@'); // padded to the dictionary's DESC limit: the gate's hunk size
    }
    let desp: Vec<f64> = (1..=100).map(|i| 1000.0 + f64::from(i) + 0.25).collect();
    let desp_text = desp.iter().map(|v| format!("{v}")).collect::<Vec<_>>().join(" ");
    let s0 = latest(&copy.path);

    let (equi, fixing) = {
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off).with_dictionary(&dictionary);
        let equi = tx.insert(zone, db1_hash("EQUI"), Some("/E3DIO-HUNK-EQUI"), |_, _| Ok(())).expect("equi");
        tx.set_hunk(equi, "DESC", &desc).expect("DESC as a hunk");
        tx.set_hunk(equi, "DESP", &desp_text).expect("DESP as a hunk");
        tx.set_hunk(equi, "UUIDIA", "11 12 13 14").expect("UUIDIA as a hunk");
        let fixing = tx.insert(zone, db1_hash("FIXING"), None, |_, _| Ok(())).expect("fixing");
        tx.set(fixing, "ROKEYS", "K1|K2").expect("ROKEYS inline");
        tx.set_hunk(fixing, "ROVALS", "V1|V2").expect("ROVALS as a hunk");

        // The dictionary and the shape rules apply before anything is placed.
        assert!(matches!(
            tx.set_hunk(equi, "DESC", &"x".repeat(121)),
            Err(TransactionError::Dictionary(DictionaryError::TextTooLong { chars: 121, max: 120, .. }))
        ));
        assert!(matches!(
            tx.set_hunk(equi, "UUIDIA", "1 2 3 4"),
            Err(TransactionError::Dictionary(DictionaryError::SetOnce { .. }))
        ), "set-once holds for a hunk too");
        assert!(matches!(tx.set_hunk(equi, "FUNC", "unset"), Err(TransactionError::NotAHunk { name, .. }) if name == "FUNC"), "unset is set's");
        assert!(matches!(tx.set_hunk(equi, "PURP", "PIPE"), Err(TransactionError::NotAHunk { .. })), "a word is a scalar");
        assert!(matches!(tx.set_hunk(fixing, "CREF", "nulref"), Err(TransactionError::NotAHunk { .. })), "a reference is a scalar");
        assert!(matches!(tx.set_hunk(equi, "NAME", "/E3DIO-NO"), Err(TransactionError::NotAHunk { .. })));
        assert!(matches!(tx.set_hunk(equi, "POS", "1 2 3"), Err(TransactionError::NotAHunk { .. })), "POS has an implicit slot");
        let too_many = (1..=101).map(|i| i.to_string()).collect::<Vec<_>>().join(" ");
        assert!(matches!(
            tx.set_hunk(equi, "DESP", &too_many),
            Err(TransactionError::Dictionary(DictionaryError::TooManyElements { count: 101, max: 100, .. }))
        ), "the dictionary's count comes first");
        tx.commit(&request("e3d-io T1d: hunks set by name")).expect("commit");
        (equi, fixing)
    };
    let s1 = latest(&copy.path);
    assert_eq!(s1, s0 + 1);
    assert_eq!(tally(&copy.path, s0, s1), (0, 2, 1));
    {
        // Without the dictionary the descriptor's own count still bounds a hunk array.
        let mut prov = provider();
        let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx").with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        let too_many = (1..=101).map(|i| i.to_string()).collect::<Vec<_>>().join(" ");
        assert!(matches!(
            tx.set_hunk(equi, "DESP", &too_many),
            Err(TransactionError::Attribute(AttributeWriteError::TooManyElements { count: 101, max: 100, .. }))
        ));
    }

    // On disk: hunk directory entries of the types the shapes pick.
    let entries = |refno: RefNo| -> Vec<(u32, u16)> {
        let mut engine = ReadOnlyEngine::open(&copy.path).expect("open");
        let view = engine.find_element(refno).expect("find").expect("indexed");
        let parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse");
        parsed
            .explicit_attributes
            .iter()
            .filter_map(|attribute| HunkRef::from_explicit(attribute).map(|entry| (attribute.hash, entry.hunk_type)))
            .collect()
    };
    let equi_entries = entries(equi);
    assert!(equi_entries.contains(&(db1_hash("DESC"), 22)), "DESC is a type-22 hunk: {equi_entries:?}");
    assert!(equi_entries.contains(&(db1_hash("DESP"), 21)), "DESP is a type-21 hunk: {equi_entries:?}");
    assert!(equi_entries.contains(&(db1_hash("UUIDIA"), 20)), "UUIDIA is a type-20 hunk: {equi_entries:?}");
    let fixing_entries = entries(fixing);
    assert_eq!(fixing_entries, vec![(db1_hash("ROVALS"), 23)], "ROVALS is the one hunk; ROKEYS is inline");

    // Read back through the descriptors, which resolve the chains.
    let equi_x = extract(&copy.path, equi);
    assert_eq!(value(&equi_x, "DESC"), Some(DescriptorValue::Text(desc)));
    assert_eq!(value(&equi_x, "DESP"), Some(DescriptorValue::RealArray(desp)));
    assert_eq!(ints(&equi_x, "UUIDIA"), Some(vec![11, 12, 13, 14]));
    assert!(matches!(value(&equi_x, "FUNC"), None | Some(DescriptorValue::Unset)), "the refused sets placed nothing");
    let fixing_x = extract(&copy.path, fixing);
    assert_eq!(value(&fixing_x, "ROKEYS"), Some(DescriptorValue::TextList(vec!["K1".into(), "K2".into()])));
    assert_eq!(value(&fixing_x, "ROVALS"), Some(DescriptorValue::TextList(vec!["V1".into(), "V2".into()])));
}
