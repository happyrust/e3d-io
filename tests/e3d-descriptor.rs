use std::path::Path;
use std::process::Command;

mod common;

const DESIGN_DB: &str =
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001";
const CATA_DB: &str =
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7351_0001";
const SYST_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\amssys";
const REFERENCE_DB: &str =
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5053_0001";
const DICTIONARY_DBS: &[&str] = &[
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001",
    r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7006_0001",
    r"D:\AVEVA\Projects\E3D3.1\SCB\scb000\scb6002_0001",
];
const ALL_DICTIONARY_DBS: &[&str] = &[
    r"D:\AVEVA\Projects\E3D3.1\SCB\scb000\scb6002_0001",
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001",
    r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7043_0001",
    r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7006_0001",
    r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7323_0001",
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5101_0001",
];
const DESVIR: &str = common::DESVIR;
const CATVIR: &str = common::CATVIR;
const DICVIR: &str = common::DICVIR;
const SYSVIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/binaries/sysvir.dat");
const ATTLIB: &str = common::ATTLIB;

#[test]
fn cli_without_command_prints_usage_and_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_e3d-descriptor"))
        .output()
        .expect("run descriptor CLI");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("e3d-descriptor describe"));
}

#[test]
fn bran_binary_and_dictionary_attributes_match_the_e3d_tty_capture() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tty = manifest.join("tests/fixtures/bran_24383_73930_attlist_tty.log");
    let catalog = manifest.join("schema/e3d31/catalog");
    let required = [DESIGN_DB, REFERENCE_DB, DESVIR, DICVIR, ATTLIB]
        .into_iter()
        .chain(DICTIONARY_DBS.iter().copied());
    if required.into_iter().any(|path| !Path::new(path).exists()) || !tty.exists() {
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_e3d-descriptor"))
        .args(["extract-complete", "--db", DESIGN_DB])
        .args(["--desvir", DESVIR, "--attlib", ATTLIB])
        .args(["--refno", "24383/73930", "--template-type", "DESI"])
        .arg("--tty-log")
        .arg(&tty)
        .arg("--catalog-layout")
        .arg(catalog.join("noun_layout.json"))
        .arg("--catalog-fields")
        .arg(catalog.join("noun_attr_fields.json"))
        .args(["--reference-db-list", REFERENCE_DB, "--uda", "on"])
        .args(["--dictionary-db-list", &DICTIONARY_DBS.join(";")])
        .args(["--dicvir", DICVIR])
        .output()
        .expect("run complete descriptor comparison");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse comparison JSON");
    assert_eq!(report["tty_row_count"], 115);
    assert_eq!(report["tty_unique_count"], 112);
    assert_eq!(report["deferred_uda_row_count"], 0);
    assert_eq!(report["omitted_row_count"], 0);
    assert_eq!(report["comparison_counts"]["equivalent"], 115);
    assert!(
        report["attributes"]
            .as_array()
            .expect("attribute rows")
            .iter()
            .all(|row| row["comparison"] == "equivalent")
    );
}

#[test]
fn all_plant_udas_match_the_e3d_tty_weld_capture() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tty = manifest.join("tests/fixtures/weld_24383_73931_attlist_tty.log");
    let catalog = manifest.join("schema/e3d31/catalog");
    let required = [DESIGN_DB, REFERENCE_DB, DESVIR, DICVIR, ATTLIB]
        .into_iter()
        .chain(ALL_DICTIONARY_DBS.iter().copied());
    if required.into_iter().any(|path| !Path::new(path).exists()) || !tty.exists() {
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_e3d-descriptor"))
        .args(["extract-complete", "--db", DESIGN_DB])
        .args(["--desvir", DESVIR, "--attlib", ATTLIB])
        .args(["--refno", "24383/73931", "--template-type", "DESI"])
        .arg("--tty-log")
        .arg(&tty)
        .arg("--catalog-layout")
        .arg(catalog.join("noun_layout.json"))
        .arg("--catalog-fields")
        .arg(catalog.join("noun_attr_fields.json"))
        .args(["--reference-db-list", REFERENCE_DB, "--uda", "on"])
        .args(["--dictionary-db-list", &ALL_DICTIONARY_DBS.join(";")])
        .args(["--dicvir", DICVIR])
        .output()
        .expect("run complete WELD descriptor comparison");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse WELD comparison JSON");
    assert_eq!(report["tty_row_count"], 92);
    assert_eq!(report["deferred_uda_row_count"], 0);
    assert_eq!(report["omitted_row_count"], 0);
    assert_eq!(report["comparison_counts"]["equivalent"], 92);
    assert!(report["attributes"]
        .as_array()
        .expect("attribute rows")
        .iter()
        .all(|row| row["comparison"] == "equivalent"));
}

#[test]
fn cata_binary_and_dictionary_attributes_match_the_e3d_tty_capture() {
    const CATA_DICTIONARY_DBS: &[&str] = &[
        r"D:\AVEVA\Projects\E3D3.1\SCB\scb000\scb6002_0001",
        r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001",
        r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7043_0001",
        r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7006_0001",
        r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7323_0001",
        r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5101_0001",
    ];
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tty = manifest.join("tests/fixtures/cate_23735_3_attlist_tty.log");
    let catalog = manifest.join("schema/e3d31/catalog");
    let required = [CATA_DB, CATVIR, DICVIR, ATTLIB]
        .into_iter()
        .chain(CATA_DICTIONARY_DBS.iter().copied());
    if required.into_iter().any(|path| !Path::new(path).exists()) || !tty.exists() {
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_e3d-descriptor"))
        .args(["extract-complete", "--db", CATA_DB])
        .args(["--desvir", CATVIR, "--attlib", ATTLIB])
        .args(["--refno", "23735/3", "--template-type", "CATA"])
        .arg("--tty-log")
        .arg(&tty)
        .arg("--catalog-layout")
        .arg(catalog.join("noun_layout.json"))
        .arg("--catalog-fields")
        .arg(catalog.join("noun_attr_fields.json"))
        .args(["--reference-db-list", CATA_DB, "--uda", "on"])
        .args(["--dictionary-db-list", &CATA_DICTIONARY_DBS.join(";")])
        .args(["--dicvir", DICVIR])
        .output()
        .expect("run complete CATA descriptor comparison");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse CATA comparison JSON");
    assert_eq!(report["tty_row_count"], 46);
    assert_eq!(report["tty_unique_count"], 37);
    assert_eq!(report["deferred_uda_row_count"], 0);
    assert_eq!(report["omitted_row_count"], 0);
    assert_eq!(report["comparison_counts"]["equivalent"], 46);
    assert!(report["attributes"]
        .as_array()
        .expect("attribute rows")
        .iter()
        .all(|row| row["comparison"] == "equivalent"));
}

#[test]
fn dictionary_runtime_attributes_match_the_e3d_tty_capture() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tty = manifest.join("tests/fixtures/uda_13292_58_attlist_tty.log");
    let catalog = manifest.join("schema/e3d31/catalog");
    let dictionary_db = DICTIONARY_DBS[0];
    if [dictionary_db, DICVIR, ATTLIB]
        .into_iter()
        .any(|path| !Path::new(path).exists())
        || !tty.exists()
    {
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_e3d-descriptor"))
        .args(["extract-complete", "--db", dictionary_db])
        .args(["--desvir", DICVIR, "--attlib", ATTLIB])
        .args(["--refno", "13292/58", "--template-type", "DICT"])
        .arg("--tty-log")
        .arg(&tty)
        .arg("--catalog-layout")
        .arg(catalog.join("noun_layout.json"))
        .arg("--catalog-fields")
        .arg(catalog.join("noun_attr_fields.json"))
        .args(["--reference-db-list", dictionary_db])
        .output()
        .expect("run complete DICT descriptor comparison");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse DICT comparison JSON");
    assert_eq!(report["tty_row_count"], 45);
    assert_eq!(report["tty_unique_count"], 45);
    assert_eq!(report["deferred_uda_row_count"], 0);
    assert_eq!(report["omitted_row_count"], 0);
    assert_eq!(report["comparison_counts"]["equivalent"], 45);
    assert!(report["attributes"]
        .as_array()
        .expect("attribute rows")
        .iter()
        .all(|row| row["comparison"] == "equivalent"));
    for (name, evaluator) in [
        ("TUNIt", "uda_default_unit_falls_back_to_dimension"),
        ("Udname", "uda_name_from_element_name"),
    ] {
        let row = report["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(row["binary_status"]["kind"], "runtime_derived");
        assert_eq!(row["binary_status"]["evaluator"], evaluator);
    }
}

#[test]
fn system_world_attributes_match_the_e3d_tty_capture() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tty = manifest.join("tests/fixtures/worl_24575_3_attlist_tty.log");
    let catalog = manifest.join("schema/e3d31/catalog");
    let required = [SYST_DB, SYSVIR, DICVIR, ATTLIB]
        .into_iter()
        .chain(DICTIONARY_DBS.iter().copied());
    if required.into_iter().any(|path| !Path::new(path).exists()) || !tty.exists() {
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_e3d-descriptor"))
        .args(["extract-complete", "--db", SYST_DB])
        .args(["--desvir", SYSVIR, "--attlib", ATTLIB])
        .args(["--refno", "24575/3", "--template-type", "SYST"])
        .arg("--tty-log")
        .arg(&tty)
        .arg("--catalog-layout")
        .arg(catalog.join("noun_layout.json"))
        .arg("--catalog-fields")
        .arg(catalog.join("noun_attr_fields.json"))
        .args(["--reference-db-list", SYST_DB, "--uda", "on"])
        .args(["--dictionary-db-list", &DICTIONARY_DBS.join(";")])
        .args(["--dicvir", DICVIR])
        .output()
        .expect("run complete SYST descriptor comparison");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse SYST comparison JSON");
    assert_eq!(report["tty_row_count"], 5);
    assert_eq!(report["tty_unique_count"], 5);
    assert_eq!(report["deferred_uda_row_count"], 0);
    assert_eq!(report["omitted_row_count"], 0);
    assert_eq!(report["comparison_counts"]["equivalent"], 5);
    assert!(report["attributes"]
        .as_array()
        .expect("attribute rows")
        .iter()
        .all(|row| row["comparison"] == "equivalent"));
}
