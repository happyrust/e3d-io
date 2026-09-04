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

use std::path::Path;

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
