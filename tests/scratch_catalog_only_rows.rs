//! Why the ten `missing_binary_value` rows of the 2026-08-26 TTY sample had no
//! row at all.
//!
//! The first guess was that the noun template gives those attributes no
//! descriptor, so the extraction never produces a row for them. It does not
//! hold: every one of the ten has a descriptor, at a sane offset and width.
//! What is missing is the heading. `attlib.dat` names `BANG`
//! `BangleDoubleConverter` and `PTDM` nothing at all, so the `Bangle` and
//! `Ptdiameter` rows of `q att` reached no attribute — while the catalog's
//! exported `QTXT` holds the heading E3D actually printed. `DETA` is headed
//! `Detref` by a name no source we read holds at all, and is declared `BOOL`
//! against a two-word reference slot.
//!
//! This reports, for each of the ten: what the catalog declares, what each
//! name source calls it, what the template descriptor says, and whether the
//! element holds an explicit record under the hash (or under its negation,
//! which is where a catalogue formula lives).
//!
//! Run with:
//! `cargo test --test scratch_catalog_only_rows -- --ignored --nocapture`

use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::attlib_metadata::AttributeMetadataCatalog;
use e3d_io::noun_catalog::NounCatalog;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::TemplateProvider;
use e3d_io::{ReadOnlyEngine, RefNo};

const AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const SHADOW: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all";

/// `(database, template file, template type, refno, noun, catalog attribute)`
/// for every row the sample reports as missing.
const CASES: &[(&str, &str, &str, u32, u32, &str, &str)] = &[
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 226148, "GPART", "DETA",
    ),
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 226147, "PRTELE", "DETA",
    ),
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 60087, "SPCO", "DETA",
    ),
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 235090, "SPCO", "DETA",
    ),
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 67329, "LSNO", "PTDI",
    ),
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 254678, "LSNO", "PTDI",
    ),
    (
        "ams8000_0001", "desvir.dat", "DESI", 24384, 60, "PJOI", "BANG",
    ),
    (
        "ams8000_0001", "desvir.dat", "DESI", 24384, 54, "SCOJ", "BANG",
    ),
    (
        "ams8000_0001", "desvir.dat", "DESI", 24384, 50, "SCTN", "BANG",
    ),
    (
        "ams8000_0001", "desvir.dat", "DESI", 24384, 55, "SUBJ", "BANG",
    ),
    // Still disagreeing: the catalog calls each of these something the
    // descriptor or E3D's own printing contradicts.
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 85074, "BTSE", "BDIA",
    ),
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 254838, "BTSE", "BDIA",
    ),
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 522, "SDTE", "SKEYSCALE",
    ),
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 226163, "TABGRO", "FIXEDB",
    ),
    // A control: the same `BDIA` on a noun where it already reads correctly.
    (
        "ams7000_0001", "catvir.dat", "CATA", 15192, 67302, "BLTP", "BDIA",
    ),
];

#[test]
#[ignore = "requires the installed E3D 3.1 projects and extracted templates"]
fn report_why_the_missing_rows_have_no_descriptor() {
    let attlib_path = format!("{SHADOW}\\attlib.dat");
    if !std::path::Path::new(&attlib_path).exists() {
        eprintln!("SKIP: {attlib_path} not present");
        return;
    }
    let attlib = AttlibData::parse_file(&attlib_path).expect("parse attlib");
    let metadata =
        AttributeMetadataCatalog::parse_file(&attlib_path, &attlib).expect("attlib metadata");
    let layout = std::fs::read_to_string("catalog/e3d31/noun_layout.json").expect("read layout");
    let fields =
        std::fs::read_to_string("catalog/e3d31/noun_attr_fields.json").expect("read fields");
    let catalog = NounCatalog::parse(&layout, &fields).expect("parse catalog");

    for (db, vir, template_type, word0, word1, noun, attribute) in CASES {
        let refno = RefNo::new(*word0, *word1);
        let hash = db1_hash(attribute);
        println!("--- {noun} {refno} {attribute} (hash {hash}) ---");

        let declared = catalog
            .noun(db1_hash(noun))
            .and_then(|entry| entry.attrs.iter().find(|attr| attr.hash == hash));
        match declared {
            Some(attr) => println!(
                "    catalog: type={} name={} array={} pseudo={} def={}",
                attr.type_name, attr.name, attr.is_array, attr.is_pseudo, attr.default_probe
            ),
            None => println!("    catalog: NOT DECLARED"),
        }
        println!(
            "    attlib short: {:?}  report: {:?}   catalog QTXT: {:?}   RPTX: {:?}",
            metadata.get(hash).map(|item| &item.short_name),
            metadata.report_name(hash),
            catalog
                .attribute_fields(hash)
                .and_then(|record| record.fields.get("QTXT"))
                .and_then(|value| value.get("s")),
            catalog
                .attribute_fields(hash)
                .and_then(|record| record.fields.get("RPTX"))
                .and_then(|value| value.get("s")),
        );

        let mut provider =
            TemplateProvider::open(format!("{SHADOW}\\{vir}"), Some(db1_hash(template_type)))
                .expect("open template");
        match provider.template_for(db1_hash(noun)) {
            Ok(Some(template)) => {
                let descriptor = template.attrs.iter().find(|attr| attr.hash == hash);
                match descriptor {
                    Some(attr) => println!(
                        "    template: wide offset {} words {} | compact offset {} words {} \
                         | storage_type_code {} | attlib def {:?}",
                        attr.wide.location.word_offset(),
                        attr.wide.storage_words,
                        attr.compact.location.word_offset(),
                        attr.compact.storage_words,
                        attr.storage_type_code,
                        attlib
                            .attr_def(hash)
                            .map(|def| (def.data_type, def.size)),
                    ),
                    None => println!("    template: NO DESCRIPTOR ({} rows)", template.attrs.len()),
                }
            }
            other => println!("    template: unavailable ({other:?})"),
        }

        let mut engine = ReadOnlyEngine::open(format!("{AMS}\\{db}")).expect("open database");
        let view = engine
            .find_element(refno)
            .expect("find element")
            .expect("element present");
        let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse element");
        let direct = parsed
            .explicit_attributes
            .iter()
            .find(|attr| attr.hash == hash);
        let negated = parsed
            .explicit_attributes
            .iter()
            .find(|attr| attr.hash == hash.wrapping_neg());
        println!(
            "    element: {} explicit records; direct {:?}; negated {:?}",
            parsed.explicit_attributes.len(),
            direct.map(|attr| &attr.value),
            negated.map(|attr| &attr.value)
        );
        let _ = &attlib;
    }
}
