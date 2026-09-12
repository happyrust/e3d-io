//! Corpus reconciliation for the RefNo-native routing plan.
//!
//! Every live index key in a database must decode to the db number stated by
//! that file's descriptor. A mismatch is a stop condition for replacing the
//! legacy Ref0 affiliation scan.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use e3d_io::{ReadOnlyEngine, ScanTier};

const E3D21_AMS: &str = r"D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams8000_0001";
const E3D31_AMS: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const CHECKED_IN_TEST_DB: &str = r"D:\work\plant-code\plant-io\test-file\ams1112_0001";

/// Corpus roots for the full gate, `;`-separated directories. Unset means the
/// gate is skipped (not ignored): it is a stage-transition check that only the
/// machine holding the installations can run, and it must never be a silent
/// no-op there. Run it in release mode; the debug scan of ~39M element headers
/// takes minutes.
///
/// ```text
/// E3D_CORPUS_ROOTS="D:\AVEVA\Projects\E3D2.1;D:\AVEVA\Projects\E3D3.1" \
///   cargo test --release --test refno_dbno_corpus
/// ```
const CORPUS_ROOTS_ENV: &str = "E3D_CORPUS_ROOTS";

fn configured_corpus_roots() -> Vec<PathBuf> {
    std::env::var_os(CORPUS_ROOTS_ENV)
        .map(|value| {
            std::env::split_paths(&value)
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn reconcile_file(path: &Path) -> usize {
    let mut engine = ReadOnlyEngine::open(path)
        .unwrap_or_else(|error| panic!("{}: open failed: {error}", path.display()));
    let dbno = engine.descriptor().db_mark;
    assert_ne!(dbno, 0, "{}: descriptor dbno is zero", path.display());

    let mut elements = 0_usize;
    for item in engine
        .scan_elements(ScanTier::Header)
        .unwrap_or_else(|error| panic!("{}: index walk failed: {error}", path.display()))
    {
        let element =
            item.unwrap_or_else(|error| panic!("{}: element read failed: {error}", path.display()));
        assert_eq!(
            element.refno.dbno(),
            dbno,
            "{}: indexed RefNo {} decodes to db {}, descriptor says db {}",
            path.display(),
            element.refno,
            element.refno.dbno(),
            dbno,
        );
        elements += 1;
    }
    elements
}

#[test]
fn available_real_database_fixtures_have_only_native_dbno_keys() {
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
        elements += reconcile_file(path);
    }
    assert!(elements > 0, "the available real files indexed no elements");
    eprintln!(
        "RefNo/dbno reconciliation passed: files={} elements={elements}",
        paths.len()
    );
}

fn looks_like_primary_database(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    // Database file names are `<3 ASCII letters><dbno>_<extent>` or the SYS/COM/MIS/GLB
    // admin names. Anything non-ASCII cannot be one, and `split_at(3)` below would
    // panic on a multi-byte char boundary (installed projects carry Chinese-named
    // spreadsheets and notes next to the databases).
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

/// Full installed-corpus gate: every primary database under every root named by
/// [`CORPUS_ROOTS_ENV`] is opened and header-scanned. Skips (loudly) when the
/// variable is unset; run it before moving between plan stages.
///
/// Last recorded run (2026-09-12, `D:\AVEVA\Projects\E3D2.1` + `E3D3.1`):
/// 883 + 1,064 files, 16,154,107 + 22,736,906 elements, zero mismatches.
#[test]
fn every_installed_corpus_key_decodes_to_its_descriptor_dbno() {
    let roots = configured_corpus_roots();
    if roots.is_empty() {
        eprintln!("skipped: {CORPUS_ROOTS_ENV} is not set; the full corpus gate needs the installed E3D projects");
        return;
    }

    let mut total_files = 0_usize;
    let mut total_elements = 0_usize;

    for root in &roots {
        assert!(
            root.is_dir(),
            "corpus root from {CORPUS_ROOTS_ENV} is not a directory: {}",
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

        let mut root_elements = 0_usize;
        for path in &files {
            root_elements += reconcile_file(path);
        }
        assert!(
            root_elements > 0,
            "corpus root indexed no elements: {}",
            root.display()
        );
        eprintln!(
            "{}: files={} elements={root_elements}",
            root.display(),
            files.len()
        );
        total_files += files.len();
        total_elements += root_elements;
    }

    eprintln!(
        "combined RefNo/dbno reconciliation passed: roots={} files={total_files} elements={total_elements}",
        roots.len()
    );
}
