//! Every boolean the sampled nouns declare a default for, and what that
//! default says.
//!
//! `TABGRO.Fixedb` is the one row of the 2026-08-26 TTY sample where E3D
//! prints the noun's own declared default instead of the type's zero, and the
//! six attributes that go the other way are all `WORD` or `STRING`. Whether
//! reading the declaration for booleans would be safe turns on one number:
//! how many booleans declare `true` while E3D prints `false`.
//!
//! Run with:
//! `cargo test --test scratch_bool_defaults -- --ignored --nocapture`

use e3d_attlib::{db1_dehash, db1_hash, lookup_system_name, AttlibData};
use e3d_io::record::descriptor::{extract_default, DescriptorValue};
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
            "PJOI", "FRMW", "SBFR", "SITE", "COATIN",
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
fn report_every_declared_boolean_default() {
    let attlib_path = format!("{SHADOW}\\attlib.dat");
    if !std::path::Path::new(&attlib_path).exists() {
        eprintln!("SKIP: {attlib_path} not present");
        return;
    }
    let attlib = AttlibData::parse_file(&attlib_path).expect("parse attlib");

    let mut declared_true = Vec::new();
    let mut declared_false = 0usize;
    let mut no_declaration = 0usize;
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
                // Storage type 5 is the packed bit, which is how a boolean is
                // held; nothing else can carry a boolean default.
                if !attr.authoritative || attr.storage_type_code != 5 {
                    continue;
                }
                let name = name_of(attr.hash);
                let def = attlib.attr_def(attr.hash);
                match extract_default(attr, def, &template.defaults, name.clone())
                    .and_then(|extracted| extracted.value)
                {
                    Some(DescriptorValue::Bool(true)) => {
                        declared_true.push(format!("{noun}.{name}"))
                    }
                    Some(DescriptorValue::Bool(false)) => declared_false += 1,
                    _ => no_declaration += 1,
                }
            }
        }
    }

    println!("booleans declaring true:  {}", declared_true.len());
    for entry in &declared_true {
        println!("    {entry}");
    }
    println!("booleans declaring false: {declared_false}");
    println!("booleans with no readable declaration: {no_declaration}");
}
