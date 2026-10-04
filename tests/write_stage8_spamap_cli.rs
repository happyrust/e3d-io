mod common;
#[path = "common/spatial_write.rs"]
mod spatial_write;
use e3d_io::{ReadOnlyEngine, RefNo};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use spatial_write::*;
use std::{fs, process::Command};

fn apply(path: &std::path::Path, ops: Value, flags: &[&str]) -> (i32, Value, String) {
    let dir = path.parent().unwrap();
    let input = dir.join("cli-ops.json");
    let report = dir.join("cli-report.json");
    fs::write(&input, ops.to_string()).unwrap();
    if report.exists() {
        fs::remove_file(&report).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_e3d-write"))
        .arg("apply")
        .arg("--db")
        .arg(path)
        .arg("--ops")
        .arg(input)
        .arg("--report")
        .arg(&report)
        .args(flags)
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        fs::read(&report)
            .ok()
            .map(|b| serde_json::from_slice(&b).unwrap())
            .unwrap_or(Value::Null),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}
fn refno(v: &Value) -> RefNo {
    let (a, b) = v.as_str().unwrap().split_once('/').unwrap();
    RefNo::new(a.parse().unwrap(), b.parse().unwrap())
}
fn identity(path: &std::path::Path) -> (Vec<u8>, u64, u32) {
    (
        Sha256::digest(fs::read(path).unwrap()).to_vec(),
        fs::metadata(path).unwrap().len(),
        ReadOnlyEngine::open(path).unwrap().session().sesno,
    )
}
fn assert_diff(path: &std::path::Path, report: &Value, before: &[e3d_io::index::SpamapEntry]) {
    let after = table(path);
    let mut registered = Vec::new();
    let mut removed = Vec::new();
    for (_, element) in report["spatial"]["elements"].as_object().unwrap() {
        registered.extend(element["registered"].as_array().unwrap().iter().cloned());
        removed.extend(element["removed"].as_array().unwrap().iter().cloned());
    }
    let as_json = |e: &e3d_io::index::SpamapEntry| json!({"key":e.key,"value":e.value});
    let mut expected: Vec<_> = after
        .iter()
        .filter(|e| !before.contains(e))
        .map(as_json)
        .collect();
    registered.sort_by_key(Value::to_string);
    expected.sort_by_key(Value::to_string);
    assert_eq!(registered, expected);
    let mut expected: Vec<_> = before
        .iter()
        .filter(|e| !after.contains(e))
        .map(as_json)
        .collect();
    removed.sort_by_key(Value::to_string);
    expected.sort_by_key(Value::to_string);
    assert_eq!(removed, expected);
    for key in [
        "policy",
        "table",
        "pages",
        "elements",
        "skipped",
        "unused_aabb",
        "unreachable",
    ] {
        assert!(report["spatial"].get(key).is_some(), "{key}");
    }
}

#[test]
fn default_require_dry_run_and_commit_have_the_same_spatial_plan() {
    let (copy, zone) = fixture();
    let before = identity(&copy.path);
    let old = table(&copy.path);
    let ops = json!([{"op":"insert","id":"box","owner":zone.to_string(),"noun":"BOX","attrs":{"XLEN":"301","YLEN":"400","ZLEN":"500","POS":"1000 2000 3000"}}]);
    let (code, dry, error) = apply(&copy.path, ops.clone(), &["--dry-run"]);
    assert_eq!(code, 0, "{error}");
    assert_eq!(identity(&copy.path), before);
    let (code, commit, error) = apply(&copy.path, ops, &[]);
    assert_eq!(code, 0, "{error}");
    assert_eq!(dry["spatial"], commit["spatial"]);
    assert_eq!(commit["spatial"]["policy"], "require");
    assert_diff(&copy.path, &commit, &old);
    let r = refno(&commit["ops"][0]["refno"]);
    assert_eq!(
        entries(&copy.path, r)[0].aabb_words(),
        [8495, 18000, 27500, 11505, 22000, 32500]
    );
}

#[test]
fn missing_boxes_refuse_with_sources_and_skip_removes_old_entries() {
    let (copy, zone) = fixture();
    let before = identity(&copy.path);
    let ops = json!([{"op":"insert","owner":zone.to_string(),"noun":"PYRA"}]);
    for flags in [
        vec!["--spatial", "require"],
        vec!["--spatial", "require", "--dry-run"],
    ] {
        let (code, report, error) = apply(&copy.path, ops.clone(), &flags);
        assert_eq!(code, 1);
        assert_eq!(report["error"]["op"], 0);
        assert!(error.contains("op #0"));
        assert!(error.contains("no usable AABB"));
        assert_eq!(identity(&copy.path), before);
    }
    let (code, seed, error) = apply(
        &copy.path,
        json!([{"op":"insert","owner":zone.to_string(),"noun":"PYRA","aabb":[0,0,0,10,20,30]}]),
        &[],
    );
    assert_eq!(code, 0, "{error}");
    let r = refno(&seed["ops"][0]["refno"]);
    let old = table(&copy.path);
    let (code, report, error) = apply(
        &copy.path,
        json!([{"op":"set","refno":r.to_string(),"attrs":{"HEIG":"100"}}]),
        &["--spatial", "skip"],
    );
    assert_eq!(code, 0, "{error}");
    assert!(entries(&copy.path, r).is_empty());
    assert_eq!(report["spatial"]["skipped"][0]["refno"], r.to_string());
    assert_diff(&copy.path, &report, &old);
}

#[test]
fn given_boxes_follow_set_move_and_last_value_without_implicitly_recomputing() {
    let (copy, zone) = fixture();
    let old = table(&copy.path);
    let ops = json!([
        {"op":"insert","id":"box","owner":zone.to_string(),"noun":"BOX","aabb":[0,0,0,1,1,1]},
        {"op":"insert","id":"zone","owner":"/*","noun":"ZONE"},
        {"op":"set","refno":"@box","attrs":{"XLEN":"400"},"aabb":[1,2,3,4,5,6]},
        {"op":"move","refno":"@box","owner":"@zone","aabb":[10,20,30,110,220,330]},
        {"op":"set","refno":"@box","aabb":[11,22,33,111,222,333]}
    ]);
    let (code, report, error) = apply(&copy.path, ops, &[]);
    assert_eq!(code, 0, "{error}");
    assert_diff(&copy.path, &report, &old);
    let r = refno(&report["ops"][0]["refno"]);
    assert_eq!(
        entries(&copy.path, r)[0].aabb_words(),
        [110, 220, 330, 1110, 2220, 3330]
    );
    let old = table(&copy.path);
    let (code, report, error) = apply(
        &copy.path,
        json!([{"op":"set","refno":r.to_string(),"aabb":[0,0,0,1,1,1]}]),
        &[],
    );
    assert_eq!(code, 0, "{error}");
    assert_eq!(table(&copy.path), old);
    assert_eq!(report["spatial"]["unused_aabb"], json!([r.to_string()]));
}

#[test]
fn bad_inputs_are_refused_before_writing_and_bad_policy_before_opening() {
    let (copy, zone) = fixture();
    let before = identity(&copy.path);
    for op in [
        json!({"op":"insert","owner":zone.to_string(),"noun":"BOX","aabb":[1,0,0,0,1,1]}),
        json!({"op":"set","refno":zone.to_string(),"aabb":[0,0,0,1,1,1]}),
        json!({"op":"delete","refno":zone.to_string(),"aabb":[0,0,0,1,1,1]}),
        json!({"op":"insert","owner":zone.to_string(),"noun":"BOX","aabb":[0,0,0,1,1,"2"]}),
    ] {
        let (code, report, error) = apply(&copy.path, json!([op]), &[]);
        assert_eq!(code, 1);
        assert!(error.contains("op #0"));
        assert_eq!(report["error"]["op"], 0);
        assert_eq!(identity(&copy.path), before);
    }
    let (code, _, error) = apply(
        &copy.path,
        json!([]),
        &["--db", "does-not-exist", "--spatial", "bad"],
    );
    assert_eq!(code, 2);
    assert!(error.contains("require, skip or off"));
    assert!(!error.contains("opening"));
    assert_eq!(identity(&copy.path), before);
}

#[test]
fn off_keeps_the_table_and_no_dictionary_keeps_compatibility() {
    let (copy, zone) = fixture();
    let old = table(&copy.path);
    let (code, report, error) = apply(
        &copy.path,
        json!([{"op":"insert","owner":zone.to_string(),"noun":"PYRA"}]),
        &["--spatial", "off", "--no-dictionary"],
    );
    assert_eq!(code, 0, "{error}");
    assert_eq!(report["spatial"]["policy"], "off");
    assert_eq!(report["spatial"]["pages"], json!([]));
    assert_eq!(table(&copy.path), old);
    assert_eq!(report["dictionary"], Value::Null);
}

#[test]
fn reminted_reports_use_final_refnos_and_keep_given_identity() {
    let (copy, zone) = fixture();
    let base = ReadOnlyEngine::open(&copy.path)
        .unwrap()
        .session()
        .sesno
        .to_string();
    let ops = json!([
        {"op":"insert","id":"box","owner":zone.to_string(),"noun":"BOX","aabb":[0,0,0,1,1,1]},
        {"op":"set","refno":"@box","aabb":[10,20,30,110,220,330]}
    ]);
    let (code, first, error) = apply(&copy.path, ops.clone(), &[]);
    assert_eq!(code, 0, "{error}");
    let occupied = refno(&first["ops"][0]["refno"]);
    let original = entries(&copy.path, occupied);
    let old = table(&copy.path);
    let (code, second, error) = apply(&copy.path, ops, &["--base", &base]);
    assert_eq!(code, 0, "{error}");
    let written = refno(&second["ops"][0]["refno"]);
    assert_ne!(written, occupied);
    assert_eq!(refno(&second["ops"][1]["refno"]), written);
    assert_eq!(
        second["spatial"]["elements"][written.to_string()]["given"],
        occupied.to_string()
    );
    assert_eq!(entries(&copy.path, occupied), original);
    assert_diff(&copy.path, &second, &old);
}

proptest::proptest! {
    #![proptest_config(property_config("proptest-regressions/spamap_cli.txt"))]
    #[test]
    fn randomized_cli_reports_match_read_side_differences(x in -1000i32..1000, extent in 1u32..500) {
        let (copy,zone)=fixture(); let before=identity(&copy.path); let old=table(&copy.path);
        let ops=json!([
            {"op":"insert","id":"box","owner":zone.to_string(),"noun":"BOX","aabb":[0,0,0,1,1,1]},
            {"op":"set","refno":"@box","aabb":[x,20,30,x+extent as i32,220,330]}
        ]);
        let (code,dry,error)=apply(&copy.path,ops.clone(),&["--dry-run"]); proptest::prop_assert_eq!(code,0,"{}",error); proptest::prop_assert_eq!(identity(&copy.path),before);
        let (code,report,error)=apply(&copy.path,ops,&[]); proptest::prop_assert_eq!(code,0,"{}",error); proptest::prop_assert_eq!(&dry["spatial"],&report["spatial"]);
        assert_diff(&copy.path,&report,&old);
        let r=refno(&report["ops"][0]["refno"]);
        proptest::prop_assert_eq!(entries(&copy.path,r)[0].aabb_words(),[(x*10) as u32,200,300,((x+extent as i32)*10) as u32,2200,3300]);
    }
}

#[test]
fn deleted_ancestor_is_reported_after_merging_the_original_base() {
    let (copy, zone) = fixture();
    let mut p = provider();
    let mut tx = e3d_io::write::Transaction::open(&copy.path, &mut p).unwrap();
    let r = insert_box(&mut tx, zone, 301.);
    tx.commit(&request()).unwrap();
    let base = ReadOnlyEngine::open(&copy.path)
        .unwrap()
        .session()
        .sesno
        .to_string();
    let mut tx = e3d_io::write::Transaction::open(&copy.path, &mut p).unwrap();
    tx.delete(zone).unwrap();
    tx.commit(&request()).unwrap();
    let old = table(&copy.path);
    let (code, report, error) = apply(
        &copy.path,
        json!([{"op":"set","refno":r.to_string(),"attrs":{"XLEN":"302"}}]),
        &["--base", &base],
    );
    assert_eq!(code, 0, "{error}");
    assert_eq!(
        report["spatial"]["unreachable"],
        json!([{"refno":r.to_string(),"ancestor":zone.to_string()}])
    );
    assert_diff(&copy.path, &report, &old);
}

#[test]
fn rejected_aabb_preserves_last_accepted_transaction_value() {
    let (copy, zone) = fixture();
    let mut p = provider();
    let mut tx = e3d_io::write::Transaction::open(&copy.path, &mut p).unwrap();
    let r = insert_box(&mut tx, zone, 301.);
    tx.set_aabb(r, [0., 0., 0., 1., 1., 1.]).unwrap();
    tx.set_aabb(r, [10., 20., 30., 110., 220., 330.]).unwrap();
    for invalid in [
        [f64::NAN, 0., 0., 1., 1., 1.],
        [1., 0., 0., 0., 1., 1.],
        [0., 0., 0., f64::INFINITY, 1., 1.],
    ] {
        assert!(tx.set_aabb(r, invalid).is_err());
    }
    tx.commit(&request()).unwrap();
    assert_eq!(
        entries(&copy.path, r)[0].aabb_words(),
        [100, 200, 300, 1100, 2200, 3300]
    );
}

#[test]
fn range_errors_refuse_require_and_skip_without_append() {
    let (copy, zone) = fixture();
    let before = identity(&copy.path);
    for policy in ["require", "skip"] {
        let (code, report, error) = apply(
            &copy.path,
            json!([{"op":"insert","owner":zone.to_string(),"noun":"BOX","aabb":[0,0,0,1e12,2e12,3e12]}]),
            &["--spatial", policy],
        );
        assert_eq!(code, 1);
        assert_eq!(report["error"]["op"], 0);
        assert!(error.contains("outside"));
        assert_eq!(identity(&copy.path), before);
    }
}

#[test]
fn ancestor_refusal_reports_the_instruction_that_forced_descendant_recompute() {
    let (copy, zone) = fixture();
    let (code, _, error) = apply(
        &copy.path,
        json!([{"op":"insert","owner":zone.to_string(),"noun":"PYRA","aabb":[0,0,0,10,20,30]}]),
        &[],
    );
    assert_eq!(code, 0, "{error}");
    let before = identity(&copy.path);
    let (code, report, error) = apply(
        &copy.path,
        json!([{"op":"set","refno":zone.to_string(),"attrs":{"POS":"100 0 0"}}]),
        &[],
    );
    assert_eq!(code, 1);
    assert_eq!(report["error"]["op"], 0);
    assert!(error.contains("op #0"));
    assert_eq!(identity(&copy.path), before);
}
