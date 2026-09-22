//! `RecordDraft::set_attribute` against the read side, on every element of
//! `ams8000_0001` (`docs/plans/2026-09-21-write-path-next-steps.md` S4).
//!
//! The oracle is the read side itself: decode every attribute of every record
//! through `extract_parsed_element_with_descriptors`, hand each decoded value
//! back to `set_attribute` on a draft of the same record, and the draft must
//! still compare `content_eq` to the original — word for word in the implicit
//! region past the header, field for field in the explicit stream. A shape
//! the writer got wrong (a swapped real half, a text padded differently, an
//! array count written where the reader does not look) shows up as the first
//! element that no longer round-trips, named with the attribute that moved.

mod common;

use std::collections::BTreeMap;

use common::write_fixture::{AMS251142, AMS5100, AMS8000};
use common::{ATTLIB, CATVIR, DESVIR, DICVIR};
use e3d_attlib::{db1_dehash, AttlibData};
use e3d_io::engine::{extract_parsed_element_with_descriptors, ScanTier};
use e3d_io::record::descriptor::{
    stored_expression_words, AttributeExtractionStatus, DescriptorValue, ElementExtraction,
};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::{NounTemplate, TemplateProvider};
use e3d_io::record::text::TextFormat;
use e3d_io::write::RecordDraft;
use e3d_io::{ReadOnlyEngine, RefNo};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

fn name(hash: u32) -> String {
    db1_dehash(hash).unwrap_or_else(|| format!("#{hash}"))
}

/// One decoded attribute that reads differently off the copy than off the
/// original: the implicit slot's words, or the explicit field's data.
struct Difference {
    description: String,
    /// The read side reported a `Bool` for a one-word slot whose word is
    /// neither 0 nor 1 — attlib calls the attribute a boolean, the database
    /// stores a word (`UDA.DTYP` holds `TEXT`, `REAL`, `INTE`…). The value the
    /// writer was handed had already lost the word, so this was the read
    /// side's to fix, not the writer's — and it is fixed (S8): such a word is
    /// now reported as `Word { raw }`, so this class is asserted empty below.
    bool_over_a_word: bool,
}

/// Which decoded attributes of `extraction` read differently off `copy` than
/// off `original`.
fn differences(
    original: &RecordDraft,
    copy: &RecordDraft,
    template: &NounTemplate,
    extraction: &ElementExtraction,
) -> Vec<Difference> {
    let mut out = Vec::new();
    for row in &extraction.attributes {
        let Some(attr) = template.attrs.iter().find(|a| a.hash == row.hash) else { continue };
        let offset = attr.wide.location.word_offset() as usize;
        let (before, after, residence): (Vec<u32>, Vec<u32>, String) = if offset != 0 {
            let end = (offset + attr.wide.storage_words as usize).min(original.implicit().len());
            (original.implicit()[offset..end].to_vec(), copy.implicit()[offset..end].to_vec(), format!("implicit @{offset}"))
        } else {
            let words = |draft: &RecordDraft| draft.explicit_words(row.hash).map(|(_, d)| d).unwrap_or_default();
            (words(original), words(copy), "explicit".to_string())
        };
        if before == after {
            continue;
        }
        let bool_over_a_word = attr.storage_type_code == 3
            && matches!(row.value, Some(DescriptorValue::Bool(_)))
            && before.first().is_some_and(|word| *word > 1);
        out.push(Difference {
            description: format!(
                "{} (type {}, {residence}, {:?}): {before:?} -> {after:?} value {:?}",
                row.name, attr.storage_type_code, row.status, row.value
            ),
            bool_over_a_word,
        });
    }
    if out.is_empty() {
        out.push(Difference {
            description: "no decoded attribute differs, but the records do (members, owner, header?)".into(),
            bool_over_a_word: false,
        });
    }
    out
}

/// The schema file whose templates describe `path`'s elements: `desvir.dat`
/// for a DESIGN database, `dicvir.dat` for a dictionary, `catvir.dat` for a
/// catalogue.
fn provider_for(engine: &ReadOnlyEngine) -> TemplateProvider {
    match engine.descriptor().db_type_name().as_deref() {
        Some("DESI") => TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir"),
        Some("DICT") => TemplateProvider::open(DICVIR, None).expect("dicvir"),
        Some("CATA") => TemplateProvider::open(CATVIR, None).expect("catvir"),
        other => panic!("no template file for a {other:?} database"),
    }
}

/// Round-trip every element of `path`; returns rows set back by (storage
/// type, residence). Panics on the first element that does not come back.
fn round_trip(path: &str, attlib: &AttlibData) -> BTreeMap<(u32, &'static str), u64> {
    let mut engine = ReadOnlyEngine::open(path).expect("open the sample");
    let text = engine.text_format();
    let mut provider = provider_for(&engine);

    let refnos: Vec<RefNo> = engine
        .scan_elements(ScanTier::Header)
        .expect("scan")
        .map(|element| element.expect("header").refno)
        .collect();
    assert!(!refnos.is_empty(), "{path} indexes no element?");

    // (storage type, resident) -> rows round-tripped.
    let mut rows: BTreeMap<(u32, &'static str), u64> = BTreeMap::new();
    let mut elements = 0u64;
    let mut no_template = 0u64;
    let mut expressions = 0u64;
    let mut non_ascii = 0u64;
    let mut bool_over_a_word = 0u64;
    let mut bool_examples: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();

    for refno in refnos {
        let view = engine.find_element(refno).expect("lookup").expect("indexed");
        let parsed = ParsedElement::parse_as(&view.raw_bytes, text).expect("parse");
        let Some(template) = provider.template_for(parsed.noun_hash).expect("load template") else {
            no_template += 1;
            continue;
        };
        let extraction = extract_parsed_element_with_descriptors(refno, &parsed, attlib, template)
            .expect("extract");
        let original = RecordDraft::from_record_bytes(&view.raw_bytes).expect("draft");
        let mut copy = original.clone();
        elements += 1;

        for row in &extraction.attributes {
            if !matches!(
                row.status,
                AttributeExtractionStatus::Decoded | AttributeExtractionStatus::DecodedExplicit
            ) {
                continue;
            }
            if stored_expression_words(&parsed, row.hash).is_some() {
                expressions += 1;
                continue;
            }
            let value = row.value.as_ref().expect("a decoded row carries a value");
            let resident = if row.storage.location.word_offset() != 0 { "implicit" } else { "explicit" };
            match copy.set_attribute(template, row.hash, value, text) {
                Ok(()) => {
                    *rows.entry((row.storage_type_code, resident)).or_default() += 1;
                    // A non-ASCII text under a non-UTF-8 header went back
                    // through the escaped encoder (plan S5) and must still
                    // compare equal below.
                    if text != TextFormat::Utf8 && matches!(value, DescriptorValue::Text(t) if !t.is_ascii()) {
                        non_ascii += 1;
                    }
                }
                Err(error) => {
                    if errors.len() < 20 {
                        errors.push(format!(
                            "{} {refno} {} (type {}, {resident}): {error} — value {:?}",
                            name(parsed.noun_hash),
                            row.name,
                            row.storage_type_code,
                            value
                        ));
                    }
                }
            }
        }
        if !copy.content_eq(&original) {
            let differences = differences(&original, &copy, template, &extraction);
            if differences.iter().all(|d| d.bool_over_a_word) {
                bool_over_a_word += differences.len() as u64;
                if bool_examples.len() < 3 {
                    bool_examples.push(format!("{} {refno}: {}", name(parsed.noun_hash), differences[0].description));
                }
            } else if mismatches.len() < 20 {
                mismatches.push(format!(
                    "{} {refno}: {}",
                    name(parsed.noun_hash),
                    differences.iter().map(|d| d.description.as_str()).collect::<Vec<_>>().join("; ")
                ));
            }
        }
    }

    let total: u64 = rows.values().sum();
    eprintln!(
        "{path} ({text:?}): {elements} elements round-tripped ({no_template} without a template, {expressions} \
         formula-backed rows skipped, {non_ascii} non-ASCII texts written back through the escaped encoder, \
         {bool_over_a_word} words the read side reports as a Bool {bool_examples:?}); \
         {total} decoded rows set back by (storage type, residence): {rows:?}"
    );
    // Closed by S8 (plan §7, 2026-09-22): a one-word slot attlib types
    // boolean but holding a word other than 0 / 1 (`UDA.DTYP` = `TEXT` …) is
    // reported as the word, so it writes back as it was read. 185 rows of
    // `ams5100_0001` were counted here before; none may be now.
    assert_eq!(
        bool_over_a_word, 0,
        "{path}: {bool_over_a_word} words the read side collapsed to a Bool, e.g. {bool_examples:?}"
    );
    assert!(
        errors.is_empty(),
        "{path}: {} decoded rows could not be set back (first {}):\n{}",
        errors.len(),
        errors.len().min(20),
        errors.join("\n")
    );
    assert!(
        mismatches.is_empty(),
        "{path}: {} elements do not round-trip (first {}):\n{}",
        mismatches.len(),
        mismatches.len().min(20),
        mismatches.join("\n")
    );
    assert_eq!(no_template, 0, "{path}: every noun has a template in its schema file");
    rows
}

/// Every decoded attribute of every element of three Marine sample
/// databases — the main design database, the dictionary and the cable-tray
/// design database with its type-17 arrays — set back onto a draft of its
/// own record leaves the record as it was.
#[test]
fn every_decoded_attribute_of_the_samples_writes_back_word_for_word() {
    if common::missing(&[AMS8000, AMS5100, AMS251142, DESVIR, DICVIR, CATVIR, ATTLIB]) {
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");

    let mut rows: BTreeMap<(u32, &'static str), u64> = BTreeMap::new();
    for path in [AMS8000, AMS5100, AMS251142] {
        for (key, count) in round_trip(path, &attlib) {
            *rows.entry(key).or_default() += count;
        }
    }
    let total: u64 = rows.values().sum();
    assert!(total > 100_000, "the three samples hold well over 100 000 decoded rows, got {total}");
    // Every storage shape the samples store went through at least once; 8, 9
    // and 18 are set nowhere in them and rest on the unit tests.
    for storage_type in [2u32, 3, 4, 5, 6, 7, 10, 14, 15, 16, 17] {
        assert!(
            rows.keys().any(|(t, _)| *t == storage_type),
            "no decoded row of storage type {storage_type} in the samples? {rows:?}"
        );
    }
}
