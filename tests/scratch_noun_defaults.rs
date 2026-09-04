//! Print a noun's explicit default stream so the value `Nappflag` falls back
//! to can be checked against its neighbours in the stream.
//!
//! Run with:
//! `cargo test --test scratch_noun_defaults -- --ignored --nocapture`

use e3d_attlib::{db1_dehash, db1_hash, lookup_system_name};
use e3d_io::record::template::TemplateProvider;

mod common;

const SHADOW: &str = common::SCHEMA_BINARIES;
/// `(template file, db type, nouns)`. The first group is where `Nappflag`
/// disagrees; the rest carry the other template defaults the TTY sample can
/// judge, right ones and wrong ones together.
const GROUPS: &[(&str, &str, &[&str])] = &[
    (
        "catvir.dat",
        "CATA",
        &[
            "LCYL", "SBOX", "SCON", "LPYR", "SEXT", "SPEC", "SELE", "PTAX", "PTCA", "TABGRO",
            "CATE", "BLTP",
        ],
    ),
    ("desvir.dat", "DESI", &["BEND", "WELD", "ATTA", "PIPE"]),
];

fn name_of(hash: u32) -> String {
    lookup_system_name(hash)
        .map(String::from)
        .or_else(|| db1_dehash(hash))
        .unwrap_or_else(|| format!("0x{hash:08X}"))
}

#[test]
#[ignore = "requires the extracted E3D 3.1 template files"]
fn print_explicit_default_streams() {
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
                println!("=== {noun}: no template ===");
                continue;
            };
            println!(
                "=== {noun} === implicit {} words, explicit stream {} entries",
                template.defaults.implicit.len(),
                template.defaults.explicit.len()
            );
            for entry in &template.defaults.explicit {
                println!(
                    "    {:<14} hash={:<12} type={:<3} words={:?}",
                    name_of(entry.hash),
                    entry.hash,
                    entry.storage_type_code,
                    entry.words
                );
            }
        }
    }
}
