//! Map `RefNo::word0` back to the database file that owns it, so an unresolved
//! reference in a comparison run can be turned into a `--reference-db-list` entry.
//!
//! Page 1 already answers this: every extent entry records the `ref_word0`
//! carried by every element stored in that extent. So this reads one page per
//! file rather than scanning any index.
//!
//! Run with:
//! `cargo test --test scratch_locate_ref0 -- --ignored --nocapture`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use e3d_io::ReadOnlyEngine;

const PROJECT_ROOT: &str = r"D:\AVEVA\Projects\E3D3.1";

/// The `ref0` values the 2026-08-26 TTY comparison run could not resolve.
const WANTED: &[u32] = &[15192, 13244, 13245, 15200, 24384];

/// Dabacon files are named `<three letters><digits>_<digits>` with no extension.
fn looks_like_a_database(path: &Path) -> bool {
    if path.extension().is_some() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some((stem, suffix)) = name.rsplit_once('_') else {
        return false;
    };
    let letters = stem.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    letters == 3
        && stem.len() > 3
        && stem[3..].chars().all(|c| c.is_ascii_digit())
        && !suffix.is_empty()
        && suffix.chars().all(|c| c.is_ascii_digit())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if looks_like_a_database(&path) {
            out.push(path);
        }
    }
}

#[test]
#[ignore = "requires the installed E3D 3.1 projects"]
fn locate_the_databases_that_own_each_unresolved_ref0() {
    let root = Path::new(PROJECT_ROOT);
    if !root.exists() {
        eprintln!("SKIP: {} not present", root.display());
        return;
    }

    let mut files = Vec::new();
    collect(root, &mut files);
    files.sort();
    println!("candidate database files: {}", files.len());

    // One ref0 can appear in several files: a database and its extents share a
    // prefix, and copies of the same project sit side by side under different
    // roots. Keep every hit rather than the first — picking one is the caller's
    // decision, and a silent first-wins here is exactly how the wrong copy gets
    // into a reference list.
    let mut owners: BTreeMap<u32, Vec<PathBuf>> = BTreeMap::new();
    let mut opened = 0usize;
    let mut refused = 0usize;

    for path in &files {
        let Ok(engine) = ReadOnlyEngine::open(path) else {
            refused += 1;
            continue;
        };
        opened += 1;
        for extent in &engine.file_info().extents {
            if WANTED.contains(&extent.ref_word0) {
                owners
                    .entry(extent.ref_word0)
                    .or_default()
                    .push(path.clone());
            }
        }
        let primary = engine.descriptor().primary_extent_ref;
        if WANTED.contains(&primary) {
            let bucket = owners.entry(primary).or_default();
            if !bucket.contains(path) {
                bucket.push(path.clone());
            }
        }
    }

    println!("opened {opened}, refused {refused}\n");
    for wanted in WANTED {
        match owners.get(wanted) {
            Some(paths) => {
                println!("ref0 {wanted}: {} file(s)", paths.len());
                for path in paths {
                    println!("    {}", path.strip_prefix(root).unwrap_or(path).display());
                }
            }
            None => println!("ref0 {wanted}: NOT FOUND in any opened database"),
        }
    }
}
