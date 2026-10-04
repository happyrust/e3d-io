//! Read-side allocation diagnostic is independent of Transaction's allocator.
mod common;

use e3d_io::index::IndexCursor;
use e3d_io::session::{DbView, SessionSelector};
use serde_json::Value;
use std::process::Command;

const EXE: &str = env!("CARGO_BIN_EXE_e3d-descriptor");

#[test]
fn diagnostic_matches_scan_at_exact_retained_session_and_absent_extent() {
    let path = common::corpus::AMS8000;
    let mut latest = DbView::open(path).expect("required fixture");
    let chain = latest.walk_sessions().unwrap();
    for session in [chain[0].sesno, chain[chain.len() / 2].sesno] {
        for extent in [24384, 0, u32::MAX] {
            let mut view = DbView::open_at(path, SessionSelector::Exact(session)).unwrap();
            let max = IndexCursor::new(&mut view).walk().map(|r| r.unwrap().refno)
                .filter(|r| r.word0 == extent).map(|r| r.word1).max();
            let result = Command::new(EXE).args(["next-word1", "--db", path, "--extent", &extent.to_string(), "--session", &session.to_string()]).output().unwrap();
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            let json: Value = serde_json::from_slice(&result.stdout).unwrap();
            assert_eq!(json["sesno"], session);
            assert_eq!(json["scan_max_word1"], serde_json::to_value(max).unwrap());
            assert_eq!(json["seek_max_word1"], serde_json::to_value(max).unwrap());
            assert_eq!(json["index_next_word1"], max.unwrap_or(0) + 1);
            assert!(json["seek_page_reads"].as_u64().unwrap() <= 6);
            assert!(json["next_word1"].as_u64().unwrap() >= u64::from(max.unwrap_or(0) + 1));
            assert_eq!(json["counter_use"], "next_available");
            assert_eq!(json["bucket_floor"], json["bucket_counter"]);
            assert_eq!(json["next_word1"].as_u64().unwrap(), u64::from(max.unwrap_or(0) + 1).max(json["bucket_floor"].as_u64().unwrap_or(0)));
        }
    }
}

#[test]
fn unavailable_bucket_reports_index_fallback_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken-bucket.db");
    let mut bytes = std::fs::read(common::corpus::AMS8000).expect("required fixture");
    bytes[2048..2052].copy_from_slice(&99u32.to_be_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let result = Command::new(EXE).args(["next-word1", "--db"]).arg(&path).args(["--extent", "24384"]).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let json: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["counter_use"], "unavailable");
    assert!(json["bucket_error"].is_string());
    assert!(json["bucket_floor"].is_null());
    assert_eq!(json["next_word1"], json["index_next_word1"]);
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "read-side diagnosis must not modify the file");
}

#[test]
fn diagnostic_rejects_missing_invalid_or_nonexistent_inputs() {
    let db = common::corpus::AMS8000;
    for args in [
        vec!["next-word1", "--db", db],
        vec!["next-word1", "--db", db, "--extent", "-1"],
        vec!["next-word1", "--db", db, "--extent", "4294967296"],
        vec!["next-word1", "--db", db, "--extent", "24384", "--session", "4294967295"],
    ] {
        let result = Command::new(EXE).args(args).output().unwrap();
        assert!(!result.status.success());
        assert!(!result.stderr.is_empty());
    }
}
