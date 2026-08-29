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
