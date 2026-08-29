//! Census of the page-0 / page-1 header fields over every database of a real
//! project.
//!
//! Two fields used to be read as lengths and are not:
//!
//! * descriptor +0x38, read as `total_pages`, is the primary extent's refno
//!   prefix. `ams7330_0001` states 15522 and is 644 pages long.
//! * page 1 word 2, read as `page_size`, is the extent table's cursor. It only
//!   takes the values 2, 5, 8, 11 — `2 + 3n` for n allocated extents — and the
//!   old reader accepted it only when `n * 256` or `n * 1024` happened to land
//!   on 2048, which locked it out of most of the project.
//!
//! Run with:
//! `cargo test --test header_field_census -- --ignored --nocapture`

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

const DEFAULT_PROJECT: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";
const EXTENT_STRIDE: u32 = 0x2000;

mod common;

const FIXTURES: &[&str] = &[DEFAULT_PROJECT];

/// Extensionless files of at least two pages, which is what a database looks
/// like from the outside. Project directories also hold plain-text defaults
/// files that pass this description, so the caller still has to tell them
/// apart by what page 1 says.
fn candidate_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| e.path().extension().is_none())
        .filter(|e| e.metadata().map(|m| m.len() >= 4096).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    files.sort();
    files
}

/// What the reader used to demand of page 1 word 2 before it was identified.
fn old_page_size_rule_accepts(word: u32) -> bool {
    word == 2048
        || word.saturating_mul(4) == 2048
        || word.saturating_mul(256) == 2048
        || word.saturating_mul(1024) == 2048
}

#[test]
fn header_fields_are_refno_prefixes_not_lengths() {
    if common::missing(FIXTURES) {
        return;
    }
    let project = env::var("E3D_CENSUS_PROJECT").unwrap_or_else(|_| DEFAULT_PROJECT.into());
    let files = candidate_files(Path::new(&project));
    if files.is_empty() {
        eprintln!("SKIP: no databases under {project}");
        return;
    }

    let mut opened = 0usize;
    let mut not_a_database: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut page_sizes: BTreeMap<u32, usize> = BTreeMap::new();
    let mut cursors: BTreeMap<u32, usize> = BTreeMap::new();
    let mut rejected_by_old_rule = 0usize;
    let mut pages_hidden_by_old_bound = 0u64;
    let mut worst_truncation: Option<(String, u32, u32)> = None;

    for path in &files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let engine = match ReadOnlyEngine::open_with_cache(path, 4) {
            Ok(engine) => engine,
            Err(err) => {
                failed.push(format!("{name}: {err}"));
                continue;
            }
        };
        let desc = engine.descriptor();
        let info = engine.file_info();

        // The defaults directories hold plain-text files that are also
        // extensionless and larger than two pages. Page 1 is what tells them
        // apart: every real database signs it 0xFFFFFFFF.
        if info.signature != u32::MAX {
            not_a_database.push(name);
            continue;
        }
        opened += 1;
        assert_eq!(info.page_flag, 1, "{name}: unexpected page-1 flag");
        *page_sizes.entry(desc.page_size).or_default() += 1;
        *cursors.entry(info.table_end_word).or_default() += 1;

        if !old_page_size_rule_accepts(info.table_end_word) {
            rejected_by_old_rule += 1;
        }

        let primary = RefNo::new(desc.primary_extent_ref, 1);
        assert_eq!(
            primary.dbno(),
            desc.db_mark,
            "{name}: primary extent ref should decode back to db_mark"
        );
        assert_eq!(
            primary.extent(),
            1,
            "{name}: the descriptor's extent should be the first one"
        );
        assert_eq!(
            info.table_end_word as usize,
            2 + 3 * info.extents.len(),
            "{name}: extent table cursor disagrees with the entries read"
        );

        for (position, r) in info.extent_refs(desc).iter().enumerate() {
            let entry = RefNo::new(*r, 1);
            assert_eq!(
                entry.dbno(),
                desc.db_mark,
                "{name}: extent ref {r} does not decode back to db_mark"
            );
            assert_eq!(
                entry.extent() as usize,
                position + 1,
                "{name}: extent refs should be consecutive from 1"
            );
            assert_eq!(
                *r,
                desc.primary_extent_ref + position as u32 * EXTENT_STRIDE,
                "{name}: each extent should sit 0x2000 above the previous"
            );
        }

        // The old bound was `min(primary_extent_ref, physical_pages)`.
        let physical = engine.readable_page_count();
        let old_bound = desc.primary_extent_ref.min(physical);
        if old_bound < physical {
            pages_hidden_by_old_bound += u64::from(physical - old_bound);
            let hidden = physical - old_bound;
            if worst_truncation
                .as_ref()
                .is_none_or(|(_, _, w)| hidden > *w)
            {
                worst_truncation = Some((name.clone(), physical, hidden));
            }
        }
    }

    println!("project: {project}");
    println!(
        "databases opened: {opened} / {} candidates ({} not databases: {not_a_database:?})",
        files.len(),
        not_a_database.len()
    );
    println!("descriptor page_size: {page_sizes:?}");
    println!("page-1 extent table cursor: {cursors:?}");
    println!(
        "would have been refused as a bad page size by the old rule: {rejected_by_old_rule} / {opened}"
    );
    println!("pages hidden by the old page bound: {pages_hidden_by_old_bound}");
    if let Some((name, physical, hidden)) = &worst_truncation {
        println!("worst truncation: {name} — {hidden} of {physical} pages unreachable");
    }

    assert!(
        failed.is_empty(),
        "databases that failed to open: {failed:?}"
    );
    assert!(
        cursors.keys().all(|c| (c - 2) % 3 == 0),
        "every cursor should be 2 + 3n: {cursors:?}"
    );
}
