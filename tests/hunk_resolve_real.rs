//! The one hunk E3D itself wrote in the installed corpus, read back as the
//! value it is (`docs/plans/2026-09-21-write-path-next-steps.md` S8).
//!
//! `DBMDAT 24384/26333` of `ams8000_0001` carries `DBMVAL` as a type-20 hunk
//! — 5 709 ints in a kind-3 block chain from page 16456 word 12 — found by
//! the S4 round-trip census and checked word for word against what this
//! crate writes (plan §7, S4). Parsed on its own the record shows the
//! five-word directory entry; through the engine, which has the pages, the
//! entry resolves to the array, and the descriptor path reports the array.

mod common;

use common::write_fixture::AMS8000;
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::record::descriptor::{AttributeExtractionStatus, DescriptorValue};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::explicit::ExplicitValue;
use e3d_io::record::hunk::{HunkRef, DIRECTORY_WORDS};
use e3d_io::record::template::TemplateProvider;
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

/// The element E3D wrote the hunk on, and the hunk's shape as the census found it.
const DBMDAT: RefNo = RefNo { word0: 24_384, word1: 26_333 };
const DBMVAL_WORDS: usize = 5_709;

#[test]
fn e3ds_own_int_array_hunk_resolves_to_its_5709_ints() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let mut engine = ReadOnlyEngine::open(AMS8000).expect("open the sample");
    let dbmval = db1_hash("DBMVAL");

    // Off the record alone: the directory entry, five words under `20 << 10`.
    let view = engine.find_element(DBMDAT).expect("lookup").expect("the sample indexes DBMDAT 24384/26333");
    let mut parsed = ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse");
    assert_eq!(parsed.noun_hash, db1_hash("DBMDAT"));
    let entry = parsed
        .explicit_attributes
        .iter()
        .find(|attribute| attribute.hash == dbmval)
        .expect("DBMVAL is in the explicit stream");
    assert_eq!(entry.type_code, 20 << 10);
    assert_eq!(entry.data_words as usize, DIRECTORY_WORDS);
    let hunk = HunkRef::from_explicit(entry).expect("a directory entry");
    assert_eq!((hunk.hunk_type, hunk.count as usize, hunk.words as usize), (20, DBMVAL_WORDS, DBMVAL_WORDS));
    assert_eq!(hunk.page_no, 16_456, "the chain E3D wrote starts on page 16456");
    let others: Vec<e3d_io::record::explicit::ExplicitAttribute> =
        parsed.explicit_attributes.iter().filter(|attribute| attribute.hash != dbmval).cloned().collect();

    // Through the engine: exactly that one entry resolves, to the ints.
    assert_eq!(engine.resolve_hunks(&mut parsed).expect("read the chain"), 1);
    let resolved = parsed
        .explicit_attributes
        .iter()
        .find(|attribute| attribute.hash == dbmval)
        .expect("still in the stream")
        .clone();
    assert_eq!(resolved.type_code, 20 << 10, "the entry still says where the value lived");
    let ExplicitValue::IntArray(ints) = &resolved.value else {
        panic!("DBMVAL resolves to an int array, got {:?}", resolved.value);
    };
    assert_eq!(ints.len(), DBMVAL_WORDS);
    assert!(HunkRef::from_explicit(&resolved).is_none(), "a resolved entry is no longer a directory");
    for other in &others {
        let same = parsed.explicit_attributes.iter().find(|attribute| attribute.hash == other.hash).unwrap();
        assert_eq!(same, other, "no other entry changed");
    }
    assert_eq!(engine.resolve_hunks(&mut parsed).expect("nothing left"), 0);

    // Through the descriptor path, which resolves before decoding.
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let extraction = engine
        .extract_element_with_descriptors(DBMDAT, &attlib, &mut provider)
        .expect("extract");
    let row = extraction
        .attributes
        .iter()
        .find(|row| row.hash == dbmval)
        .expect("DBMDAT's template declares DBMVAL");
    assert_eq!(row.storage_type_code, 7, "the template calls it a word array; the record keeps it as a type-20 hunk");
    assert_eq!(row.status, AttributeExtractionStatus::DecodedExplicit);
    match &row.value {
        Some(DescriptorValue::IntArray(values)) => assert_eq!(values.len(), DBMVAL_WORDS),
        Some(DescriptorValue::WordArray(values)) | Some(DescriptorValue::RawWords(values)) => {
            assert_eq!(values.len(), DBMVAL_WORDS);
            assert!(
                values.iter().zip(ints).all(|(word, int)| *word as i32 == *int),
                "the words are the ints the chain holds"
            );
        }
        other => panic!("DBMVAL decodes to the 5 709-element array, got {other:?}"),
    }
    eprintln!("DBMDAT {DBMDAT} DBMVAL: {} ints through the descriptor path as {:?}", ints.len(), row.value.as_ref().map(shape));
}

fn shape(value: &DescriptorValue) -> &'static str {
    match value {
        DescriptorValue::IntArray(_) => "IntArray",
        DescriptorValue::WordArray(_) => "WordArray",
        DescriptorValue::RawWords(_) => "RawWords",
        _ => "other",
    }
}
