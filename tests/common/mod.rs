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
