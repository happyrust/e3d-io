//! WORLD location on real databases (RefNo-native routing, stage 3C spike, pinned).
//!
//! A database's WORLD is `RefNo(primary_extent_ref, 0)`: one point lookup, no
//! scan for the `WORL` noun. This file checks that on the real fixtures that
//! are installed and — under `E3D_CORPUS_ROOTS` — on every primary database of
//! the installed E3D 2.1/3.1 projects. Synthetic cases and exact `read_stats`
//! deltas live in `world_refno_synthetic.rs`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use e3d_attlib::db1_hash;
use e3d_io::{E3dReaderError, ReadOnlyEngine, ScanTier};

const E3D21_AMS: &str = r"D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams8000_0001";
const E3D31_AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const CHECKED_IN_TEST_DB: &str = r"D:\work\plant-code\plant-io\test-file\ams1112_0001";
const CORPUS_ROOTS_ENV: &str = "E3D_CORPUS_ROOTS";

/// Project-administration database types whose WORLD is *not* at element 0.
/// They are never element databases of an MDB (`docs/plans/2026-09-12-cross-db-
/// attribute-refno-routing-dev-plan.md` §10.2).
const ADMIN_TYPES: &[&str] = &["SYST", "COMM", "MISC"];

struct WorldCheck {
    world_found: bool,
    worl_count: usize,
    worl_matches_descriptor: bool,
    elements: usize,
}

/// `world()` by point lookup, then a header scan to prove it is the only WORL.
fn check_file(path: &Path) -> Result<WorldCheck, String> {
    let mut engine = ReadOnlyEngine::open(path)
        .map_err(|error| format!("{}: open failed: {error}", path.display()))?;
    let expected = engine.descriptor().world_refno();
    let world_found = match engine.world() {
        Ok(Some(view)) => {
            assert_eq!(view.refno, expected, "{}", path.display());
            true
        }
        Ok(None) => false,
        Err(E3dReaderError::NotAWorld { .. }) => false,
        Err(error) => return Err(format!("{}: world lookup failed: {error}", path.display())),
    };

    let worl = db1_hash("WORL");
    let mut worl_count = 0_usize;
    let mut worl_matches_descriptor = false;
    let mut elements = 0_usize;
    let scan = engine
        .scan_elements(ScanTier::Header)
        .map_err(|error| format!("{}: scan failed: {error}", path.display()))?;
    for item in scan {
        let item =
            item.map_err(|error| format!("{}: element read failed: {error}", path.display()))?;
        elements += 1;
        if item.noun_hash == worl {
            worl_count += 1;
            if item.refno == expected {
                worl_matches_descriptor = true;
            }
        }
    }
    Ok(WorldCheck {
        world_found,
        worl_count,
        worl_matches_descriptor,
        elements,
    })
}

fn is_admin_type(engine_type: Option<&str>) -> bool {
    engine_type.is_some_and(|name| {
        ADMIN_TYPES
            .iter()
            .any(|admin| admin.eq_ignore_ascii_case(name))
    })
}

fn assert_world_contract(path: &Path) -> usize {
    let db_type = ReadOnlyEngine::open(path)
        .map(|engine| engine.descriptor().db_type_name())
        .unwrap_or_else(|error| panic!("{}: open failed: {error}", path.display()));
    let check = check_file(path).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        check.worl_count,
        1,
        "{}: every database holds exactly one WORL",
        path.display()
    );
    if is_admin_type(db_type.as_deref()) {
        assert!(
            !check.world_found,
            "{}: {} databases keep their WORLD away from element 0; if this changed, update DbDescriptor::world_refno's docs",
            path.display(),
            db_type.unwrap_or_default()
        );
    } else {
        assert!(
            check.world_found && check.worl_matches_descriptor,
            "{}: WORLD must be RefNo(primary_extent_ref, 0) for a {} database",
            path.display(),
            db_type.unwrap_or_else(|| "?".to_string())
        );
    }
    check.elements
}

#[test]
fn installed_fixtures_locate_their_world_without_a_scan() {
    let paths = [E3D21_AMS, E3D31_AMS, CHECKED_IN_TEST_DB]
        .into_iter()
        .map(Path::new)
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        eprintln!("skipped: no real E3D database fixture is installed");
        return;
    }
    let mut elements = 0_usize;
    for path in &paths {
        elements += assert_world_contract(path);
    }
    eprintln!(
        "WORLD contract held on {} real fixture(s), {elements} elements scanned for the cross-check",
        paths.len()
    );
}

fn looks_like_primary_database(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    if name.contains('.') || !name.is_ascii() || name.len() < 4 {
        return false;
    }
    let name = name.to_ascii_lowercase();
    let (prefix, tail) = name.split_at(3);
    if !prefix.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return false;
    }
    if matches!(tail, "sys" | "com" | "mis" | "glb")
        || tail.bytes().all(|byte| byte.is_ascii_digit())
    {
        return true;
    }
    let Some((dbno, extent)) = tail.rsplit_once('_') else {
        return false;
    };
    !dbno.is_empty() && dbno.bytes().all(|byte| byte.is_ascii_digit()) && extent == "0001"
}

fn collect_primary_databases(root: &Path, output: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("{}: directory read failed: {error}", root.display()));
    for entry in entries {
        let entry = entry
            .unwrap_or_else(|error| panic!("{}: directory entry failed: {error}", root.display()));
        let file_type = entry.file_type().unwrap_or_else(|error| {
            panic!("{}: file type failed: {error}", entry.path().display())
        });
        if file_type.is_dir() {
            collect_primary_databases(&entry.path(), output);
        } else if file_type.is_file() && looks_like_primary_database(&entry.file_name()) {
            output.push(entry.path());
        }
    }
}

/// Full installed-corpus gate, `E3D_CORPUS_ROOTS`-driven like the dbno gate in
/// `refno_dbno_corpus.rs`; skips loudly when unset. Run in release mode.
///
/// Last recorded run (2026-09-12, `D:\AVEVA\Projects\E3D2.1` + `E3D3.1`):
/// 1,947 databases, 1,905 element/catalogue/dictionary databases with the
/// WORLD at `primary_extent_ref / 0`, 42 admin databases (SYST/COMM/MISC)
/// with it elsewhere, zero other exceptions.
#[test]
fn every_installed_database_keeps_its_world_where_the_descriptor_says() {
    let roots: Vec<PathBuf> = std::env::var_os(CORPUS_ROOTS_ENV)
        .map(|value| {
            std::env::split_paths(&value)
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    if roots.is_empty() {
        eprintln!("skipped: {CORPUS_ROOTS_ENV} is not set; the WORLD corpus gate needs the installed E3D projects");
        return;
    }

    let mut files_total = 0_usize;
    let mut elements_total = 0_usize;
    let mut admin_total = 0_usize;
    for root in &roots {
        assert!(
            root.is_dir(),
            "corpus root is not a directory: {}",
            root.display()
        );
        let mut files = Vec::new();
        collect_primary_databases(root, &mut files);
        files.sort();
        files.dedup();
        assert!(
            !files.is_empty(),
            "corpus root contains no primary databases: {}",
            root.display()
        );
        for path in &files {
            let db_type = ReadOnlyEngine::open(path)
                .map(|engine| engine.descriptor().db_type_name())
                .unwrap_or_else(|error| panic!("{}: open failed: {error}", path.display()));
            if is_admin_type(db_type.as_deref()) {
                admin_total += 1;
            }
            elements_total += assert_world_contract(path);
        }
        eprintln!("{}: files={}", root.display(), files.len());
        files_total += files.len();
    }
    eprintln!(
        "WORLD contract held: roots={} files={files_total} admin_databases={admin_total} elements_scanned={elements_total}",
        roots.len()
    );
}
