//! Shared guard for the tests that read the installed E3D 3.1 fixtures.
//!
//! These used to carry `#[ignore]`. That hides an absent fixture and a real
//! failure equally well: four assertions here were red while `cargo test`
//! reported a clean run, including two the changelog lists as regressions.
//!
//! A test that calls [`missing`] runs wherever the files exist and reports the
//! skip where they do not, so a plain `cargo test` on this machine is the
//! whole truth. Only the whole-file scans that take the best part of a minute
//! keep `#[ignore]`, and they say so in the reason string.

#![allow(dead_code)]

pub mod corpus;
pub mod golden;
pub mod spamap_pages;
pub mod spamap_session;
pub mod write_fixture;

use std::path::Path;

use e3d_io::engine::ScanTier;
use e3d_io::refno::RefNo;
use e3d_io::ReadOnlyEngine;

/// The 3.1 schema pack that ships in this repository, and the installation
/// files it carries.
///
/// Every one of these used to be spelled `E:\reverse\e3d\shadow_e3d31_aps_all`
/// in each test that wanted it. That is one machine's drive: everywhere else
/// the whole set is absent at once and every test reports a skip, which reads
/// like a clean run. Anchoring on `CARGO_MANIFEST_DIR` instead makes the
/// fixtures part of the checkout — a working tree whose `schema/e3d31/binaries/`
/// is filled needs no environment and no drive letter, and the paths are
/// written down once rather than twenty-odd times.
///
/// `binaries/` is committed, so a fresh clone has them. [`missing`] still
/// reports an absent one as a skip, for a tree that has removed them or a
/// pack that does not carry the file.
macro_rules! pack_binary {
    ($file:literal) => {
        concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries/", $file)
    };
}

pub const SCHEMA_PACK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31");
pub const SCHEMA_BINARIES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries");
pub const DESVIR: &str = pack_binary!("desvir.dat");
pub const ATTLIB: &str = pack_binary!("attlib.dat");
pub const CATVIR: &str = pack_binary!("catvir.dat");
pub const DICVIR: &str = pack_binary!("dicvir.dat");

/// The sample DESIGN database and the `attlib.dat` that belongs beside it, in
/// the sibling `plant-io` checkout.
///
/// Neither can move into the schema pack: the database is 99 MB of project
/// data, and `attlib.dat` carries that project's UDA definitions, so it is
/// project-scoped where the pack is release-scoped. The pack's own
/// [`ATTLIB`] holds a different UDA set — pairing it with this database
/// resolves UDA hashes against the wrong table.
///
/// These were spelled `pdms-io-fork` until that checkout was renamed, which
/// left every test that named it skipping on a path nothing writes to.
pub const TEST_FILE_DB: &str = r"D:\work\plant-code\plant-io\test-file\ams1112_0001";
pub const TEST_FILE_ATTLIB: &str = r"D:\work\plant-code\plant-io\test-file\attlib.dat";

/// NXTR, the noun the element-decoding tests on [`TEST_FILE_DB`] are written
/// against. The database indexes 1156 of them.
pub const NXTR_HASH: u32 = 0x000DBF71;

/// The DRAFT fixtures: the two AVEVA sample projects' picture directories
/// (`<proj>pic\NN\M<w0>-<w1>-<EXFI>-<PVNO>`, 881 Plant + 1 885 Marine files)
/// and the Plant PADD databases. Sample-project data, not committed
/// (`docs/plans/2026-09-24-gw-picture-read-plan.md` Q2).
pub const PLANT_PIC_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaPlantSample\apspic";
pub const MARINE_PIC_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\amspic";
pub const PLANT_PADD_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaPlantSample\aps000";
pub const MARINE_PADD_DIR: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000";

/// The 30 Plant sheet thumbnails `draft31\gw\thumbs.py` pulled out of the
/// PADD databases (`<sheet name with / → _>.png`, plus `thumbs.tsv`: db,
/// refno, name, EXFI, PVNO, png, picture).
pub const DRAFT31_THUMBS: &str = r"D:\ida_scratch\draft31\gw\thumbs";

/// `/APS-0001`'s smallest sheet picture (SHEE 2013286677/4350, 42 KB: 31
/// segments, 655 primitives) and its biggest (`/APS-0001/1`, SHEE
/// 2013286677/265, 14 MB: 4 715 segments, 351 819 primitives).
pub const PLANT_PIC_SMALL: &str =
    r"D:\AVEVA\Projects\E3D3.1\AvevaPlantSample\apspic\30\M2013286677-4350-10000-11";
pub const PLANT_PIC_BIG: &str =
    r"D:\AVEVA\Projects\E3D3.1\AvevaPlantSample\apspic\09\M2013286677-265-10000-11";

/// Two Marine background pictures: `M2013287597-16864-10000-1` holds TEXT
/// (kind 1) and INSTANCE primitives, `M2013287597-1792-10000-1` TEXT2
/// (kind 10) with multi-run strings (`SHOP&/TEST`, `Block &&&/Unit No.`,
/// `&USEC.(Right)`), BOUNDARY and BOUNDCONIC.
pub const MARINE_PIC_TEXT: &str =
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\amspic\00\M2013287597-16864-10000-1";
pub const MARINE_PIC_TEXT2: &str =
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\amspic\00\M2013287597-1792-10000-1";

/// Listings the frozen reference implementation (`draft31\gwdump.py`, DRAFT
/// plan Q8) wrote for those files — `<name>.raw.txt` in local coordinates,
/// `<name>.page.txt` / `marine_<name>.page.txt` in sheet mm. The Rust
/// listing is diffed against them line for line.
pub const DRAFT31_LISTS: &str = r"D:\ida_scratch\draft31\lists";

/// Diff the `gwdump.py` listing of `file` (`--list`, `--page` when `page`)
/// against the Rust one, line for line, and panic on the first difference.
/// `oracle` is the file the Python script wrote.
pub fn assert_listing_matches(file: &e3d_io::picture::PictureFile, page: bool, oracle: &str) {
    let expected = std::fs::read_to_string(oracle).expect("the oracle listing reads");
    let expected: Vec<&str> = expected.lines().collect();
    let mut ours = vec![e3d_io::picture::listing::list_header(file, page)];
    ours.extend(e3d_io::picture::listing::list_lines(file, page, false));
    for (i, (ours, theirs)) in ours.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            ours, theirs,
            "line {i} of {oracle} differs (ours first, gwdump.py second)"
        );
    }
    assert_eq!(
        ours.len(),
        expected.len(),
        "{oracle}: line counts differ (ours, gwdump.py)"
    );
}

/// True when any fixture is absent, after printing which. Call it as the first
/// line of a test and return.
pub fn missing(paths: &[&str]) -> bool {
    let absent: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|path| !Path::new(path).exists())
        .collect();
    if absent.is_empty() {
        return false;
    }
    eprintln!("skipped: fixtures not installed: {}", absent.join(", "));
    true
}

/// The first element, in RefNo order, whose noun `wanted` accepts — its RefNo
/// and noun hash — or `None` when the session indexes no such element.
///
/// The tests that call this used to name `17496/9621`: the NXTR that stood
/// first in the fixture of the day, and an element no database on this machine
/// indexes now, so every one of them failed on `ElementNotFound` the moment
/// the fixture path was made to resolve. The index is where an element's
/// existence is stated, so the element is asked of it. The header tier reads
/// one page per element, and the scan stops at the first hit.
pub fn first_element_with_noun(
    engine: &mut ReadOnlyEngine,
    mut wanted: impl FnMut(u32) -> bool,
) -> Option<(RefNo, u32)> {
    engine
        .scan_elements(ScanTier::Header)
        .expect("walk the session's index")
        .map(|element| element.expect("read an indexed element's header"))
        .find(|element| wanted(element.noun_hash))
        .map(|element| (element.refno, element.noun_hash))
}

/// The first element of `noun_hash` in RefNo order. See
/// [`first_element_with_noun`].
pub fn first_element_of_noun(engine: &mut ReadOnlyEngine, noun_hash: u32) -> Option<RefNo> {
    first_element_with_noun(engine, |noun| noun == noun_hash).map(|(refno, _)| refno)
}
