//! `e3d-descriptor spamap-census` run for real (requirement 3.8): it leaves
//! every input byte-identical, prints the same bytes twice, lists a file that
//! is not a database and a database it cannot read, and still counts the rest.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::golden::sha256_hex;
use common::write_fixture::{copy_source, AMS251142, AMS8000, TES023};

const CENSUS: &str = env!("CARGO_BIN_EXE_e3d-descriptor");

fn census(args: &[&str]) -> Output {
    Command::new(CENSUS).arg("spamap-census").args(args).output().expect("run e3d-descriptor")
}

fn hashes(paths: &[&Path]) -> Vec<String> {
    paths.iter().map(|path| sha256_hex(&fs::read(path).expect("read an input"))).collect()
}

// Feature: write-path-phase-3, Property 4: 普查只读且确定
#[test]
fn the_census_reads_only_and_says_the_same_thing_twice() {
    if common::missing(&[AMS8000, AMS251142, TES023]) {
        return;
    }
    // The pinned copies where there are some: E3D may be saving onto the
    // installed samples while this runs, and the point here is that the
    // census does not change them.
    let ams8000 = copy_source(AMS8000);
    let tes023 = copy_source(TES023);
    let dir = tempfile::tempdir().expect("temp dir");
    let in_dir: PathBuf = dir.path().join("ams251142_0001");
    fs::copy(copy_source(AMS251142), &in_dir).expect("copy a database into the input directory");
    let notes = dir.path().join("notes.txt");
    fs::write(&notes, "not a database").expect("write a stray file");
    let truncated = dir.path().join("TES023_truncated");
    let tes023_bytes = fs::read(&tes023).expect("read TES023");
    fs::write(&truncated, &tes023_bytes[..3 * 2048]).expect("write a database cut short");

    let session = format!("{}=640", ams8000.display());
    let args = [
        "--input",
        tes023.to_str().unwrap(),
        "--input",
        ams8000.to_str().unwrap(),
        "--session",
        &session,
        "--input",
        dir.path().to_str().unwrap(),
    ];
    let inputs: [&Path; 5] = [&ams8000, &tes023, &in_dir, &notes, &truncated];
    let before = hashes(&inputs);
    let first = census(&args);
    let second = census(&args);
    assert_eq!(hashes(&inputs), before, "the census changed an input");
    assert_eq!(first.stdout, second.stdout, "two runs printed different bytes");
    assert_eq!(first.status.code(), Some(1), "a database it could not read makes the exit code 1");

    let report: serde_json::Value = serde_json::from_slice(&first.stdout).expect("the census prints JSON");
    assert_eq!(report["not_databases"], serde_json::json!([notes.display().to_string()]));
    let errors = report["errors"].as_array().expect("errors");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0]["path"], truncated.display().to_string());

    let databases = report["databases"].as_array().expect("databases");
    let paths: Vec<&str> = databases.iter().map(|database| database["path"].as_str().unwrap()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "databases come in path order");
    assert_eq!(paths.len(), 3);
    let baseline = databases.iter().find(|database| database["path"] == ams8000.display().to_string()).unwrap();
    assert_eq!(baseline["sesno"], 640);
    assert_eq!(baseline["spamap"]["pages"], 244);
    assert_eq!(baseline["spamap"]["entries"], 3_746);
    assert_eq!(baseline["dangling"]["entries"], 0);
    assert_eq!(baseline["nouns"]["BOX"]["elements_by_entry_count"]["1"], 45);
}

#[test]
fn a_census_with_nothing_wrong_exits_zero() {
    if common::missing(&[TES023]) {
        return;
    }
    let tes023 = copy_source(TES023);
    let output = census(&["--input", tes023.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(report["errors"], serde_json::json!([]));
    assert_eq!(report["databases"].as_array().map(Vec::len), Some(1));
}

#[test]
fn a_session_for_a_file_that_is_not_an_input_is_a_usage_error() {
    let output = census(&["--input", r"C:\no\such\db", "--session", r"C:\other\db=5"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a database among the inputs"));
}
