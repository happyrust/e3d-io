//! Which chunk of `*vir.dat` a noun's default for an attribute comes from.
//!
//! `extract_default` reads chunk 1 for an attribute that lives in the implicit
//! region and chunk 2 for one that lives in the explicit stream, but the
//! extraction JSON records only that a template default was used. The TTY
//! sample says some of those defaults match E3D and some do not; telling the
//! two chunks apart is what makes that split readable.
//!
//! Emits TSV on stdout so the comparison output can be joined against it:
//! `noun<TAB>attribute<TAB>chunk<TAB>words`.
//!
//! Run with:
//! `cargo test --test scratch_default_chunks -- --ignored --nocapture`

use e3d_attlib::{db1_dehash, db1_hash, lookup_system_name, AttlibData};
use e3d_io::attlib_metadata::AttributeMetadataCatalog;
use e3d_io::record::template::TemplateProvider;

mod common;

const SHADOW: &str = common::SCHEMA_BINARIES;

/// Every noun the 2026-08-26 TTY sample covers, by template file.
const GROUPS: &[(&str, &str, &[&str])] = &[
    (
        "catvir.dat",
        "CATA",
        &[
            "LCYL", "LPYR", "LSNO", "SBOX", "SCON", "SCTO", "SCYL", "SDIS", "SDSH", "SRTO", "SSLC",
            "SSPH", "SEXT", "SREV", "SVER", "SLOO", "NSCY", "PTAX", "PTCA", "PTMI", "SPEC", "SELE",
            "SPCO", "CATE", "GMSE", "PTSE", "BLTP", "BTSE", "SDTE", "TABGRO", "TABHEA", "TABITE",
            "TABHQU", "TABQUE", "SCOM", "SBOL", "MBOL", "LTAB", "LINE", "DATA", "GPART", "PRTELE",
            "COCO", "CCTA",
        ],
    ),
    (
        "desvir.dat",
        "DESI",
        &[
            "BEND", "WELD", "ATTA", "PIPE", "ELBO", "REDU", "FTUB", "STRU", "SUBJ", "SCOJ", "SCTN",
            "PJOI", "FRMW", "SBFR", "SITE", "COATIN", "SUBJ",
        ],
    ),
];

fn name_of(hash: u32) -> String {
    lookup_system_name(hash)
        .map(String::from)
        .or_else(|| db1_dehash(hash))
        .unwrap_or_else(|| format!("0x{hash:08X}"))
}

#[test]
#[ignore = "requires the extracted E3D 3.1 template files"]
fn report_which_chunk_each_default_comes_from() {
    let attlib_path = format!("{SHADOW}\\attlib.dat");
    let attlib = AttlibData::parse_file(&attlib_path).expect("parse attlib");
    let metadata =
        AttributeMetadataCatalog::parse_file(&attlib_path, &attlib).expect("attlib metadata");
    println!("noun\tattribute\treport\tchunk\twords");
    for (file, db_type, nouns) in GROUPS {
        let path = format!("{SHADOW}\\{file}");
        if !std::path::Path::new(&path).exists() {
            eprintln!("SKIP: {path} not present");
            continue;
        }
        let mut provider =
            TemplateProvider::open(&path, Some(db1_hash(db_type))).expect("open template");
        for noun in *nouns {
            let Ok(Some(template)) = provider.template_for(db1_hash(noun)) else {
                continue;
            };
            for attr in &template.attrs {
                if !attr.authoritative {
                    continue;
                }
                let report = metadata.report_name(attr.hash).unwrap_or_default();
                let (chunk, words) = if attr.wide.location.word_offset() == 0 {
                    match template.defaults.explicit_for(attr.hash) {
                        Some(entry) => ("explicit", format!("{:?}", entry.words)),
                        None => continue,
                    }
                } else if template.defaults.implicit.is_empty() {
                    continue;
                } else {
                    ("implicit", String::new())
                };
                println!("{noun}\t{}\t{report}\t{chunk}\t{words}", name_of(attr.hash));
            }
        }
    }
}
