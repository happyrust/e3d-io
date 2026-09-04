use std::path::Path;
use std::process::Command;

const DESIGN_DB: &str =
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001";
const REFERENCE_DB: &str =
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5053_0001";
const DICTIONARY_DBS: &[&str] = &[
    r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001",
    r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7006_0001",
    r"D:\AVEVA\Projects\E3D3.1\SCB\scb000\scb6002_0001",
];
const DESVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\desvir.dat";
const DICVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\dicvir.dat";
const ATTLIB: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat";

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
