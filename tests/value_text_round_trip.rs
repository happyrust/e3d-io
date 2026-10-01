//! `Display` for `DescriptorValue` is text that `write::value::parse_value`
//! reads back into the stored words.
//!
//! The oracle is `write_stage4_set_attribute.rs`'s, one step longer: every
//! decoded attribute of three E3D 3.1 sample databases is printed, the text is
//! parsed for the attribute it came from, and the parsed value is set back on
//! a draft of the element's own record. The draft must still compare
//! `content_eq` to the original, so a text that loses a bit anywhere — a real
//! printed short, a word printed under a name that hashes elsewhere, a text
//! trimmed or unquoted — shows up as the element that no longer comes back.
//!
//! Four kinds of row have no text to go through and are counted, not set: a
//! formula held in place of a number, a value stored out of the record as a
//! hunk (its directory entry is not the value), an empty array, and a
//! reference array the decoder hands over as its stored words
//! (`RawWords([count, word0, word1, …])` for type 17 — `HCREF` / `TCREF` in
//! `ams251142_0001`), whose words print as integers where the parser wants
//! references.

mod common;

use std::collections::BTreeMap;

use common::write_fixture::{AMS251142, AMS5100, AMS8000};
use common::{ATTLIB, CATVIR, DESVIR, DICVIR};
use e3d_attlib::{db1_dehash, AttlibData};
use e3d_io::engine::{extract_parsed_element_with_descriptors, ScanTier};
use e3d_io::record::descriptor::{
    stored_expression_words, AttributeExtractionStatus, DescriptorValue, StorageShape,
};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::hunk::is_hunk_type;
use e3d_io::record::template::TemplateProvider;
use e3d_io::write::{parse_value, RecordDraft};
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

fn name(hash: u32) -> String {
    db1_dehash(hash).unwrap_or_else(|| format!("#{hash}"))
}

fn provider_for(engine: &ReadOnlyEngine) -> TemplateProvider {
    match engine.descriptor().db_type_name().as_deref() {
        Some("DESI") => TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir"),
        Some("DICT") => TemplateProvider::open(DICVIR, None).expect("dicvir"),
        Some("CATA") => TemplateProvider::open(CATVIR, None).expect("catvir"),
        other => panic!("no template file for a {other:?} database"),
    }
}

fn is_empty_array(value: &DescriptorValue) -> bool {
    match value {
        DescriptorValue::IntArray(items) => items.is_empty(),
        DescriptorValue::RealArray(items) => items.is_empty(),
        DescriptorValue::RefNoArray(items) => items.is_empty(),
        DescriptorValue::WordArray(items) | DescriptorValue::RawWords(items) => items.is_empty(),
        DescriptorValue::BoolArray(items) => items.is_empty(),
        _ => false,
    }
}

/// Counts of rows by what happened to them.
#[derive(Default)]
struct Tally {
    set: BTreeMap<u32, u64>,
    formulas: u64,
    hunks: u64,
    empty_arrays: u64,
    stored_reference_words: u64,
}

fn round_trip(path: &str, attlib: &AttlibData) -> Tally {
    let mut engine = ReadOnlyEngine::open(path).expect("open the sample");
    let text_format = engine.text_format();
    let mut provider = provider_for(&engine);
    let refnos: Vec<RefNo> = engine
        .scan_elements(ScanTier::Header)
        .expect("scan")
        .map(|element| element.expect("header").refno)
        .collect();
    assert!(!refnos.is_empty(), "{path} indexes no element?");

    let mut tally = Tally::default();
    let mut refused: Vec<String> = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();

    for refno in refnos {
        let view = engine.find_element(refno).expect("lookup").expect("indexed");
        let parsed = ParsedElement::parse_as(&view.raw_bytes, text_format).expect("parse");
        let template = provider
            .template_for(parsed.noun_hash)
            .expect("load template")
            .unwrap_or_else(|| panic!("{path}: {} has no template", name(parsed.noun_hash)));
        let extraction =
            extract_parsed_element_with_descriptors(refno, &parsed, attlib, template).expect("extract");
        let original = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
        let mut copy = original.clone();

        for row in &extraction.attributes {
            if !matches!(
                row.status,
                AttributeExtractionStatus::Decoded | AttributeExtractionStatus::DecodedExplicit
            ) {
                continue;
            }
            if stored_expression_words(&parsed, row.hash).is_some() {
                tally.formulas += 1;
                continue;
            }
            if parsed
                .explicit_attributes
                .iter()
                .any(|entry| entry.hash == row.hash && is_hunk_type(entry.type_code >> 10))
            {
                tally.hunks += 1;
                continue;
            }
            let value = row.value.as_ref().expect("a decoded row carries a value");
            if is_empty_array(value) {
                tally.empty_arrays += 1;
                continue;
            }
            let attr = template
                .attrs
                .iter()
                .find(|attr| attr.hash == row.hash)
                .expect("an extracted row comes from a template attribute");
            if matches!(value, DescriptorValue::RawWords(_))
                && StorageShape::of(attr.storage_type_code) == Some(StorageShape::Reference)
            {
                tally.stored_reference_words += 1;
                continue;
            }
            let text = value.to_string();
            let reread = match parse_value(attr, &text, &mut |_| Ok(None)) {
                Ok(reread) => reread,
                Err(error) => {
                    if refused.len() < 20 {
                        refused.push(format!(
                            "{} {refno} {} (type {}): {value:?} printed as {text:?}: {error}",
                            name(parsed.noun_hash),
                            row.name,
                            row.storage_type_code
                        ));
                    }
                    continue;
                }
            };
            match copy.set_attribute(template, row.hash, &reread, text_format) {
                Ok(()) => *tally.set.entry(row.storage_type_code).or_default() += 1,
                Err(error) => {
                    if refused.len() < 20 {
                        refused.push(format!(
                            "{} {refno} {} (type {}): {text:?} read back as {reread:?}, not set: {error}",
                            name(parsed.noun_hash),
                            row.name,
                            row.storage_type_code
                        ));
                    }
                }
            }
        }
        if !copy.content_eq(&original) && mismatches.len() < 20 {
            let changed: Vec<String> = extraction
                .attributes
                .iter()
                .filter(|row| {
                    matches!(
                        row.status,
                        AttributeExtractionStatus::Decoded | AttributeExtractionStatus::DecodedExplicit
                    )
                })
                .filter_map(|row| {
                    let value = row.value.as_ref()?;
                    let attr = template.attrs.iter().find(|attr| attr.hash == row.hash)?;
                    let offset = attr.wide.location.word_offset() as usize;
                    let (before, after) = if offset != 0 {
                        let end = (offset + attr.wide.storage_words as usize).min(original.implicit().len());
                        (original.implicit()[offset..end].to_vec(), copy.implicit()[offset..end].to_vec())
                    } else {
                        let words = |draft: &RecordDraft| draft.explicit_words(row.hash).map(|(_, data)| data).unwrap_or_default();
                        (words(&original), words(&copy))
                    };
                    (before != after).then(|| format!("{} {value:?} as {:?}: {before:?} -> {after:?}", row.name, value.to_string()))
                })
                .collect();
            mismatches.push(format!("{} {refno}: {}", name(parsed.noun_hash), changed.join("; ")));
        }
    }

    let total: u64 = tally.set.values().sum();
    eprintln!(
        "{path}: {total} decoded rows went through their text and back ({} formulas, {} hunks, {} empty arrays, \
         {} reference arrays as stored words skipped); by storage type: {:?}",
        tally.formulas, tally.hunks, tally.empty_arrays, tally.stored_reference_words, tally.set
    );
    assert!(
        refused.is_empty(),
        "{path}: decoded values whose text does not read back (first {}):\n{}",
        refused.len(),
        refused.join("\n")
    );
    assert!(
        mismatches.is_empty(),
        "{path}: elements whose attributes do not come back through their text (first {}):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    tally
}

/// Every decoded attribute of the three Marine samples `write_stage4` uses —
/// design, dictionary, and the cable-tray design with its type-17 arrays —
/// printed and read back, writes back word for word.
#[test]
fn every_decoded_value_reads_back_from_its_text() {
    if common::missing(&[AMS8000, AMS5100, AMS251142, DESVIR, DICVIR, CATVIR, ATTLIB]) {
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let mut set: BTreeMap<u32, u64> = BTreeMap::new();
    for path in [AMS8000, AMS5100, AMS251142] {
        for (storage_type, count) in round_trip(path, &attlib).set {
            *set.entry(storage_type).or_default() += count;
        }
    }
    let total: u64 = set.values().sum();
    assert!(total > 100_000, "the three samples hold well over 100 000 decoded rows, got {total}");
    // Type 17 is absent on purpose: the samples' type-17 arrays reach the
    // text as stored words (see the module note).
    for storage_type in [2u32, 3, 4, 5, 6, 7, 10, 14, 15, 16] {
        assert!(set.contains_key(&storage_type), "no decoded row of storage type {storage_type} went through its text: {set:?}");
    }
}
