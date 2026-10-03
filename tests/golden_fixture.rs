//! The golden-hash fixture itself (`tests/common/golden.rs`): copies are named
//! by test crate, test and order; a file that ends as recorded passes; a file
//! that does not and a sample that moved under the recording both fail, and
//! say which of the two happened.

mod common;

use std::fs;
use std::panic::Location;
use std::path::Path;

use common::golden::{self, Finished, Recorded};

#[test]
fn copies_are_named_by_test_crate_test_and_order() {
    assert_eq!(golden::next_key(), "golden_fixture::copies_are_named_by_test_crate_test_and_order#0");
    assert_eq!(golden::next_key(), "golden_fixture::copies_are_named_by_test_crate_test_and_order#1");
}

#[test]
fn a_changed_file_and_a_moved_sample_fail_differently() {
    let dir = tempfile::tempdir().expect("temp dir");
    let sample = b"the sample as it was copied".to_vec();
    let written = b"the copy as the test left it".to_vec();
    let path = dir.path().join("copy");
    fs::write(&path, &written).expect("write the copy");
    let copy = Finished {
        key: "golden_fixture::example#0",
        source: Path::new("sample"),
        original: &sample,
        path: &path,
        made_at: Location::caller(),
    };
    let recorded = |source_sha256: String| {
        vec![(
            "example.json".to_owned(),
            Recorded {
                source: "sample".to_owned(),
                source_sha256,
                sha256: golden::sha256_hex(&written),
                len: written.len() as u64,
            },
        )]
    };

    let on_this_sample = recorded(golden::sha256_hex(&sample));
    assert_eq!(golden::verify(&on_this_sample, &copy), Ok(()));

    let mut one_byte_off = written.clone();
    one_byte_off[0] ^= 1;
    fs::write(&path, &one_byte_off).expect("change one byte");
    let why = golden::verify(&on_this_sample, &copy).expect_err("a changed byte is caught");
    assert!(why.contains("ends as") && !why.contains("sample drift"), "{why}");
    fs::write(&path, &written).expect("put the byte back");

    let on_another_sample = recorded(golden::sha256_hex(b"the sample before E3D saved onto it"));
    let why = golden::verify(&on_another_sample, &copy).expect_err("a moved sample is caught");
    assert!(why.contains("sample drift"), "{why}");
}

#[test]
fn golden_files_are_the_json_files_of_the_directory() {
    assert!(golden::load(Path::new(r"C:\no\such\golden\dir")).is_empty());

    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(
        dir.path().join("off_compat.json"),
        r#"{ "entries": { "t::a#0": { "source": "s", "source_sha256": "AA", "sha256": "BB", "len": 3 } } }"#,
    )
    .expect("write a golden file");
    fs::write(dir.path().join("README.txt"), "not a golden file").expect("write a stray file");
    let loaded = golden::load(dir.path());
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded["t::a#0"],
        vec![(
            "off_compat.json".to_owned(),
            Recorded { source: "s".into(), source_sha256: "AA".into(), sha256: "BB".into(), len: 3 }
        )]
    );
}
