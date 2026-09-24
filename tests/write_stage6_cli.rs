//! T1d (`docs/plans/2026-09-22-write-path-phase-2-plan.md` §2): the batch
//! instruction CLI `e3d-write apply` is a shell over the `Transaction` facade
//! — one JSON file of instructions becomes one session, every instruction
//! going through the facade call it names (`insert` / `set` / `set_hunk` /
//! `rename` / `delete` / `move_to` / `reorder` / `add_reference_member` /
//! `remove_reference_member`), the report says which refnos were minted (and
//! re-minted), `--dry-run` makes every check and leaves the file byte-identical,
//! a refused instruction names itself and writes nothing, and `--base` behind
//! the newest session commits a merge with the facade's clash rule.
//!
//! The binary under test is the one Cargo built for this test run
//! (`CARGO_BIN_EXE_e3d-write`).

mod common;

use common::write_fixture::{DbCopy, AMS8000};
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::engine::ScanTier;
use e3d_io::record::descriptor::{DescriptorValue, ElementExtraction};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::hunk::HunkRef;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionSelector};
use e3d_io::{ReadOnlyEngine, RefNo};

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;
const SCHEMA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31");
const DICTIONARY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d31/catalog/noun_attr_fields.json");
const EXE: &str = env!("CARGO_BIN_EXE_e3d-write");

/// The base session number of `path`'s newest session.
fn latest(path: &Path) -> u32 {
    DbView::open(path).expect("open").session().sesno
}

fn base_sesno_of(path: &Path, sesno: u32) -> u32 {
    DbView::open_at(path, SessionSelector::Exact(sesno)).expect("open").session().tail.expect("tail").base_sesno
}

/// `(deleted, inserted, modified)` between two sessions.
fn tally(path: &Path, from: u32, to: u32) -> (u64, u64, u64) {
    let mut view = DbView::open(path).expect("open");
    let diff = view.diff_sessions(SessionSelector::Exact(from), SessionSelector::Exact(to)).expect("diff");
    let tally = diff.tally();
    (tally.deleted, tally.inserted, tally.modified)
}

/// The first `n` ZONEs in index order — the gate's owners.
fn zones(path: &Path, n: usize) -> Vec<RefNo> {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let zone_hash = db1_hash("ZONE");
    let found: Vec<RefNo> = engine
        .scan_elements(ScanTier::Header)
        .expect("scan")
        .map(|e| e.expect("header"))
        .filter(|e| e.noun_hash == zone_hash)
        .map(|e| e.refno)
        .take(n)
        .collect();
    assert_eq!(found.len(), n, "the sample has {n} ZONEs");
    found
}

fn named(path: &Path, name: &str) -> Option<RefNo> {
    ReadOnlyEngine::open(path).expect("open").find_by_name(name).expect("lookup")
}

fn parsed(path: &Path, refno: RefNo) -> ParsedElement {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let view = engine.find_element(refno).expect("find").unwrap_or_else(|| panic!("{refno} is indexed"));
    ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse")
}

fn owner_of(path: &Path, refno: RefNo) -> RefNo {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    engine.find_element(refno).expect("find").unwrap_or_else(|| panic!("{refno} is indexed")).header().owner
}

fn extract(path: &Path, refno: RefNo) -> ElementExtraction {
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let mut prov = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    engine.extract_element_with_descriptors(refno, &attlib, &mut prov).expect("extract")
}

fn value(extraction: &ElementExtraction, name: &str) -> Option<DescriptorValue> {
    extraction.attributes.iter().find(|a| a.name == name).and_then(|a| a.value.clone())
}

/// A refno as the report spells it (`w0/w1`), back to a `RefNo`.
fn refno(text: &serde_json::Value) -> RefNo {
    let text = text.as_str().expect("a refno string");
    let (w0, w1) = text.split_once('/').expect("w0/w1");
    RefNo::new(w0.parse().expect("word0"), w1.parse().expect("word1"))
}

/// What one `e3d-write apply` run left: its exit status, stdout, stderr and
/// the report it wrote.
struct Applied {
    output: Output,
    report: serde_json::Value,
}

impl Applied {
    fn ok(&self) -> bool {
        self.output.status.success()
    }
    fn code(&self) -> i32 {
        self.output.status.code().expect("an exit code")
    }
    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }
    fn op(&self, index: usize) -> &serde_json::Value {
        &self.report["ops"][index]
    }
    fn commit(&self) -> &serde_json::Value {
        &self.report["commit"]
    }
}

/// Write `ops` (JSON text) beside the copy and run `e3d-write apply` on it
/// with `--report`, plus `extra` flags.
fn apply(copy: &DbCopy, tag: &str, ops: &str, extra: &[&str]) -> Applied {
    let dir = copy.path.parent().expect("the copy has a directory");
    let ops_path: PathBuf = dir.join(format!("{tag}.ops.json"));
    let report_path: PathBuf = dir.join(format!("{tag}.report.json"));
    fs::write(&ops_path, ops).expect("write the instruction file");
    let _ = fs::remove_file(&report_path);
    let output = Command::new(EXE)
        .arg("apply")
        .args(["--db", copy.path.to_str().expect("utf-8 path")])
        .args(["--ops", ops_path.to_str().expect("utf-8 path")])
        .args(["--schema", SCHEMA])
        .args(["--report", report_path.to_str().expect("utf-8 path")])
        .args(extra)
        .output()
        .expect("run e3d-write");
    let report = match fs::read_to_string(&report_path) {
        Ok(text) => serde_json::from_str(&text).expect("the report is JSON"),
        Err(_) => serde_json::Value::Null,
    };
    Applied { output, report }
}

/// The instruction file of the gate's first sessions rolled into one: a BOX
/// under the first ZONE, a GPWL under the WORL with a GROU referencing the
/// ZONE and the BOX, an EQUI with `DESC` / `UUIDIA` as hunks, a FIXING with a
/// string list inline and as a hunk, and three named BOXes under the second
/// ZONE (two referenced by the GROU).
const FIRST: &str = r##"{
  "comment": "e3d-io T1d: the gate's first sessions as one instruction file",
  "ops": [
    {"op": "insert", "id": "box", "owner": "/1RX03-MSUP", "noun": "box",
     "attrs": {"POS": "1000 2000 3000", "XLEN": "300", "YLEN": "400", "ZLEN": "500"}},
    {"op": "insert", "id": "gpwl", "owner": "/*", "noun": "GPWL"},
    {"op": "insert", "id": "grou", "owner": "@gpwl", "noun": "GROU"},
    {"op": "reference", "group": "@grou", "member": "/1RX03-MSUP"},
    {"op": "reference", "group": "@grou", "member": "@box"},
    {"op": "insert", "id": "equi", "owner": "/1RX03-MSUP", "noun": "EQUI", "name": "/E3DIO-CLI-EQUI",
     "attrs": {"DESC": {"value": "e3d-io T1d: a DESC stored out of the record", "hunk": true},
               "UUIDIA": {"value": "11 12 13 14", "hunk": true},
               "FUNC": "CLI-PATH", "PURP": "pipe"}},
    {"op": "insert", "id": "fixing", "owner": "/1RX03-MSUP", "noun": "FIXING",
     "attrs": {"ROKEYS": "K1|K2", "ROVALS": {"value": "V1|V2", "hunk": true}, "CREF": "/1RX03-MSUP", "BUIL": "true", "NUMB": "42"}},
    {"op": "insert", "id": "a", "owner": "/1RX03-MCT", "noun": "BOX", "name": "/E3DIO-CLI-A", "attrs": {"POS": "1000 2000 3000", "XLEN": "300"}},
    {"op": "insert", "id": "b", "owner": "/1RX03-MCT", "noun": "BOX", "name": "/E3DIO-CLI-B", "attrs": {"POS": "2000 2000 3000", "XLEN": "300"}},
    {"op": "insert", "id": "c", "owner": "/1RX03-MCT", "noun": "BOX", "name": "/E3DIO-CLI-C", "attrs": {"POS": "3000 2000 3000", "XLEN": "300"}, "at": {"before": "@a"}},
    {"op": "reference", "group": "@grou", "member": "@a"},
    {"op": "reference", "group": "@grou", "member": "@b", "at": "first"}
  ]
}"##;

/// Every instruction becomes the facade call it names, in one session; the
/// report has the refnos; the file reads back as the instructions said.
#[test]
fn an_instruction_file_is_applied_as_one_session_through_the_facade() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB, DICTIONARY]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let all = zones(&copy.path, 3);
    let (zone, zone2, zone3) = (all[0], all[1], all[2]);
    assert_eq!(named(&copy.path, "/1RX03-MSUP"), Some(zone), "the instruction files name the first ZONE");
    assert_eq!(named(&copy.path, "/1RX03-MCT"), Some(zone2));
    assert_eq!(named(&copy.path, "/1RX03-LCT"), Some(zone3));
    let s0 = latest(&copy.path);

    // --- S+1: the first file ---
    let first = apply(&copy, "first", FIRST, &[]);
    assert!(first.ok(), "stdout:\n{}\nstderr:\n{}", first.stdout(), first.stderr());
    let s1 = latest(&copy.path);
    assert_eq!(s1, s0 + 1, "one file, one session");
    assert_eq!(first.commit()["written"]["sesno"], s1);
    assert_eq!(first.commit()["previous"]["sesno"], s0);
    assert_eq!(first.commit()["merged_onto"], serde_json::Value::Null, "written on the base");
    assert_eq!(first.commit()["reminted"].as_array().map(Vec::len), Some(0));
    assert_eq!(first.report["error"], serde_json::Value::Null);
    assert_eq!(first.report["base_sesno"], s0);
    assert_eq!(first.report["dictionary"].as_u64().map(|n| n > 1_000), Some(true), "the pack's dictionary is on by default");
    // 8 inserts; modified: ZONE 1, WORL, ZONE 2 (the GROU is inserted, so its list is part of the insert).
    assert_eq!(tally(&copy.path, s0, s1), (0, 8, 3));
    let session = DbView::open(&copy.path).expect("open").session().clone();
    assert_eq!(session.user, "e3dio");
    assert_eq!(session.comment, "e3d-io T1d: the gate's first sessions as one instruction file", "the file's comment");

    let the_box = refno(&first.op(0)["refno"]);
    assert_eq!(first.op(0)["given"], first.op(0)["refno"], "nothing re-minted on the base");
    assert_eq!(first.op(0)["op"], "insert");
    assert_eq!(first.op(0)["noun"], "BOX", "the noun is upper-cased");
    assert_eq!(refno(&first.op(0)["owner"]), zone, "/NAME resolved to the ZONE");
    assert_eq!(owner_of(&copy.path, the_box), zone);
    let box_x = extract(&copy.path, the_box);
    assert_eq!(value(&box_x, "POS"), Some(DescriptorValue::RealArray(vec![1000.0, 2000.0, 3000.0])));
    assert_eq!(value(&box_x, "XLEN"), Some(DescriptorValue::Real(300.0)));
    assert_eq!(value(&box_x, "ZLEN"), Some(DescriptorValue::Real(500.0)));

    let gpwl = refno(&first.op(1)["refno"]);
    let grou = refno(&first.op(2)["refno"]);
    let world = ReadOnlyEngine::open(&copy.path).expect("open").world().expect("world").expect("has a WORL").header().refno;
    assert_eq!(refno(&first.op(1)["owner"]), world, "/* is the WORL");
    assert_eq!(owner_of(&copy.path, gpwl), world);
    assert_eq!(owner_of(&copy.path, grou), gpwl, "@gpwl named the insert before");
    let a = refno(&first.op(7)["refno"]);
    let b = refno(&first.op(8)["refno"]);
    let c = refno(&first.op(9)["refno"]);
    assert_eq!(parsed(&copy.path, grou).members, vec![b, zone, the_box, a], "references in the order asked: B first, then ZONE, BOX, A");
    assert_eq!(owner_of(&copy.path, the_box), zone, "a reference leaves the member's owner alone");

    let equi = refno(&first.op(5)["refno"]);
    assert_eq!(named(&copy.path, "/E3DIO-CLI-EQUI"), Some(equi), "the insert's name is in the table");
    let equi_p = parsed(&copy.path, equi);
    let hunks: Vec<(u32, u16)> = equi_p
        .explicit_attributes
        .iter()
        .filter_map(|attribute| HunkRef::from_explicit(attribute).map(|entry| (attribute.hash, entry.hunk_type)))
        .collect();
    assert!(hunks.contains(&(db1_hash("DESC"), 22)), "DESC is a type-22 hunk: {hunks:?}");
    assert!(hunks.contains(&(db1_hash("UUIDIA"), 20)), "UUIDIA is a type-20 hunk: {hunks:?}");
    assert!(!hunks.iter().any(|(hash, _)| *hash == db1_hash("FUNC")), "FUNC is inline");
    let equi_x = extract(&copy.path, equi);
    assert_eq!(value(&equi_x, "DESC"), Some(DescriptorValue::Text("e3d-io T1d: a DESC stored out of the record".into())));
    assert_eq!(value(&equi_x, "FUNC"), Some(DescriptorValue::Text("CLI-PATH".into())));
    assert!(matches!(value(&equi_x, "PURP"), Some(DescriptorValue::Word { raw, .. }) if raw == db1_hash("PIPE")));

    let fixing = refno(&first.op(6)["refno"]);
    let fixing_x = extract(&copy.path, fixing);
    assert_eq!(value(&fixing_x, "ROKEYS"), Some(DescriptorValue::TextList(vec!["K1".into(), "K2".into()])));
    assert_eq!(value(&fixing_x, "ROVALS"), Some(DescriptorValue::TextList(vec!["V1".into(), "V2".into()])));
    assert_eq!(value(&fixing_x, "CREF"), Some(DescriptorValue::RefNo(zone)), "/NAME in a value resolved by the facade");
    assert_eq!(value(&fixing_x, "BUIL"), Some(DescriptorValue::Bool(true)));

    let zone2_members = parsed(&copy.path, zone2).members;
    let tail = &zone2_members[zone2_members.len() - 3..];
    assert_eq!(tail, [c, a, b], "C inserted before A (`at`), then A, B in order");
    assert_eq!(parsed(&copy.path, zone).members.iter().filter(|m| [the_box, equi, fixing].contains(m)).count(), 3);
    assert_eq!(named(&copy.path, "/E3DIO-CLI-C"), Some(c));

    // --- S+2: the second file — moves, a reorder, an unreference, a rename to
    // Chinese, a stored element's DESC, a delete, an attribute unset ---
    let second = format!(
        r##"{{
  "user": "cli",
  "comment": "e3d-io 写路径门：中文 NAME / DESC / 会话注释",
  "ops": [
    {{"op": "move", "refno": "/E3DIO-CLI-A", "owner": "/1RX03-LCT"}},
    {{"op": "reorder", "refno": "/E3DIO-CLI-C", "at": {{"after": "/E3DIO-CLI-B"}}}},
    {{"op": "unreference", "group": "={grou}", "member": "/E3DIO-CLI-B"}},
    {{"op": "rename", "refno": "{the_box}", "name": "/E3DIO-盒子"}},
    {{"op": "set", "refno": "/1RX03-MSUP", "attrs": {{"DESC": "反应堆厂房 -3.320m~±0.00m"}}}},
    {{"op": "update", "refno": "/E3DIO-CLI-EQUI", "attrs": {{"FUNC": "unset"}}}},
    {{"op": "delete", "refno": "@nobody"}}
  ]
}}"##
    );
    // `@nobody` names no insert of this file: refused at the instruction, nothing written.
    let bytes = copy.current_bytes();
    let refused = apply(&copy, "second-refused", &second, &[]);
    assert!(!refused.ok());
    assert_eq!(refused.code(), 1);
    assert!(refused.stderr().contains("op #6 (delete @nobody)"), "stderr:\n{}", refused.stderr());
    assert!(refused.stderr().contains("no earlier insert in this file has id \"nobody\""), "stderr:\n{}", refused.stderr());
    assert_eq!(refused.report["error"]["op"], 6);
    assert_eq!(refused.report["commit"], serde_json::Value::Null);
    assert_eq!(copy.current_bytes(), bytes, "a refused file writes nothing");
    assert_eq!(latest(&copy.path), s1);

    let second = second.replace(r#"{"op": "delete", "refno": "@nobody"}"#, &format!(r#"{{"op": "delete", "refno": "{fixing}"}}"#));
    let second = apply(&copy, "second", &second, &[]);
    assert!(second.ok(), "stdout:\n{}\nstderr:\n{}", second.stdout(), second.stderr());
    let s2 = latest(&copy.path);
    assert_eq!(s2, s1 + 1);
    // Modified: A (owner word), ZONE 2, ZONE 3, GROU, the BOX (name), ZONE 1 (DESC + FIXING gone), EQUI; deleted: FIXING.
    assert_eq!(tally(&copy.path, s1, s2), (1, 0, 7));
    let session = DbView::open(&copy.path).expect("open").session().clone();
    assert_eq!(session.user, "cli", "the file's user");
    assert_eq!(session.comment, "e3d-io 写路径门：中文 NAME / DESC / 会话注释", "the comment as typed, in UTF-8");
    assert_eq!(owner_of(&copy.path, a), zone3, "move_to");
    assert_eq!(parsed(&copy.path, zone3).members.last(), Some(&a), "at the end by default");
    let zone2_members = parsed(&copy.path, zone2).members;
    assert!(!zone2_members.contains(&a));
    let tail = &zone2_members[zone2_members.len() - 2..];
    assert_eq!(tail, [b, c], "reorder: C after B");
    assert_eq!(parsed(&copy.path, grou).members, vec![zone, the_box, a], "B unreferenced; the others stand");
    assert_eq!(named(&copy.path, "/E3DIO-盒子"), Some(the_box), "renamed in the record and the table");
    assert_eq!(value(&extract(&copy.path, zone), "DESC"), Some(DescriptorValue::Text("反应堆厂房 -3.320m~±0.00m".into())));
    assert!(matches!(value(&extract(&copy.path, equi), "FUNC"), None | Some(DescriptorValue::Unset)), "unset through set");
    assert!(ReadOnlyEngine::open(&copy.path).expect("open").find_element(fixing).expect("find").is_none(), "deleted");
    assert!(!parsed(&copy.path, zone).members.contains(&fixing));
    assert_eq!(second.op(0)["op"], "move");
    assert_eq!(refno(&second.op(0)["owner"]), zone3);
    assert_eq!(second.op(0)["at"], "last");
    assert_eq!(second.op(1)["at"]["after"], b.to_string());
    assert_eq!(second.op(6)["op"], "delete");
}

/// `--dry-run` runs the whole batch through the staging — the shapes, the
/// dictionary, the records, the merge's scan on a moved-on base — and rolls it
/// back: exit 0 when a commit would have gone through, the report saying which
/// session was staged, and the file byte-identical.
#[test]
fn a_dry_run_makes_every_check_and_leaves_the_file_byte_identical() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB, DICTIONARY]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let s0 = latest(&copy.path);
    let before = copy.current_bytes();

    let dry = apply(&copy, "dry", FIRST, &["--dry-run"]);
    assert!(dry.ok(), "stdout:\n{}\nstderr:\n{}", dry.stdout(), dry.stderr());
    assert_eq!(copy.current_bytes(), before, "byte-identical after the dry run");
    assert_eq!(latest(&copy.path), s0);
    assert_eq!(dry.report["dry_run"], true);
    assert_eq!(dry.report["commit"], serde_json::Value::Null);
    assert_eq!(dry.report["staged"]["sesno"], s0 + 1);
    assert_eq!(dry.report["staged"]["previous_sesno"], s0);
    assert_eq!(dry.report["staged"]["rolled_back"], true);
    assert!(dry.report["staged"]["pages"].as_u64().is_some_and(|pages| pages > 0));
    assert_eq!(dry.report["ops"].as_array().map(Vec::len), Some(12), "every instruction was taken");
    assert!(dry.stdout().contains("nothing committed"), "stdout:\n{}", dry.stdout());

    // A dry run refuses what a commit would refuse, and still writes nothing.
    let bad = FIRST.replace(r#""XLEN": "300", "YLEN": "400""#, r#""XLEN": "wide", "YLEN": "400""#);
    let dry_bad = apply(&copy, "dry-bad", &bad, &["--dry-run"]);
    assert!(!dry_bad.ok());
    assert!(dry_bad.stderr().contains("op #0") && dry_bad.stderr().contains("\"wide\" is not a number"), "stderr:\n{}", dry_bad.stderr());
    assert_eq!(copy.current_bytes(), before);

    // The same file for real: the dry run's session number is the one written.
    let real = apply(&copy, "real", FIRST, &[]);
    assert!(real.ok(), "stderr:\n{}", real.stderr());
    assert_eq!(real.commit()["written"]["sesno"], s0 + 1);
    assert_eq!(real.op(0)["refno"], dry.op(0)["refno"], "the dry run showed the refnos the commit minted");
    let after_real = copy.current_bytes();

    // On a base the file has moved past, the dry run makes the merge's scan too
    // — here the given refno is taken by the session in between, which the
    // report says — and the file is still left as it was.
    let merge = r##"[{"op": "insert", "owner": "/1RX03-MSUP", "noun": "BOX", "name": "/E3DIO-DRY-MERGE", "attrs": {"XLEN": "555"}}]"##;
    let dry_merge = apply(&copy, "dry-merge", merge, &["--dry-run", "--base", &s0.to_string()]);
    assert!(dry_merge.ok(), "stderr:\n{}", dry_merge.stderr());
    assert_eq!(dry_merge.report["base_sesno"], s0);
    assert_eq!(dry_merge.report["latest_before"], s0 + 1);
    assert_eq!(dry_merge.op(0)["taken_in_latest"], true, "the same max + 1 as the first file's BOX");
    assert_eq!(dry_merge.report["staged"]["sesno"], s0 + 2);
    assert_eq!(copy.current_bytes(), after_real, "byte-identical after a merge's dry run");
    assert_eq!(latest(&copy.path), s0 + 1);
}

/// What the file cannot mean, or the facade will not do, is refused at the
/// instruction with a message that names it and the reason, exit code 1, the
/// report carrying the error, and not a byte written; the command line's own
/// mistakes are exit code 2.
#[test]
fn a_refused_instruction_names_itself_and_writes_nothing() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB, DICTIONARY]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let zone = zones(&copy.path, 1)[0];
    // Something to delete and something named, from a file that goes through.
    let setup = apply(
        &copy,
        "setup",
        r##"[{"op": "insert", "id": "e", "owner": "/1RX03-MSUP", "noun": "EQUI", "name": "/E3DIO-REFUSE-EQUI", "attrs": {"UUIDIA": "1 2 3 4"}}]"##,
        &[],
    );
    assert!(setup.ok(), "stderr:\n{}", setup.stderr());
    let equi = refno(&setup.op(0)["refno"]);
    let s1 = latest(&copy.path);
    let bytes = copy.current_bytes();

    let cases: Vec<(&str, String, usize, &[&str])> = vec![
        (
            "attr-typo",
            r##"[{"op": "set", "refno": "/1RX03-MSUP", "attrs": {"DESCR": "x"}}]"##.to_string(),
            0,
            &["op #0 (set", "DESCR", "has no attribute named DESCR"],
        ),
        (
            "attr-of-another-noun",
            r##"[{"op": "insert", "owner": "/1RX03-MSUP", "noun": "BOX", "attrs": {"DESC": "x"}}]"##.to_string(),
            0,
            &["op #0 (insert BOX under /1RX03-MSUP: DESC)", "has no attribute named DESC"],
        ),
        (
            "value-unreadable",
            r##"[{"op": "insert", "owner": "/1RX03-MSUP", "noun": "BOX", "attrs": {"XLEN": "three hundred"}}]"##.to_string(),
            0,
            &["op #0", "XLEN = \"three hundred\"", "is not a number"],
        ),
        (
            "refno-not-indexed",
            r##"[{"op": "set", "refno": "=24384/2147483000", "attrs": {"DESC": "x"}}]"##.to_string(),
            0,
            &["op #0 (set =24384/2147483000)", "is not indexed in the base session", "@id"],
        ),
        (
            "name-unknown",
            r##"[{"op": "insert", "owner": "/E3DIO-NOBODY", "noun": "BOX"}]"##.to_string(),
            0,
            &["op #0 (insert BOX under /E3DIO-NOBODY)", "no element of the base session is so named"],
        ),
        (
            "id-unknown",
            r##"[{"op": "insert", "owner": "@nobody", "noun": "BOX"}]"##.to_string(),
            0,
            &["op #0", "no earlier insert in this file has id \"nobody\""],
        ),
        (
            "owner-deleted",
            format!(r##"[{{"op": "delete", "refno": "{equi}"}}, {{"op": "insert", "owner": "{equi}", "noun": "BOX"}}]"##),
            1,
            &["op #1 (insert BOX under", "is deleted in this transaction"],
        ),
        (
            "set-on-deleted",
            format!(r##"[{{"op": "delete", "refno": "{equi}"}}, {{"op": "set", "refno": "{equi}", "attrs": {{"FUNC": "x"}}}}]"##),
            1,
            &["op #1 (set", "is deleted in this transaction and cannot be set"],
        ),
        (
            "dictionary-121",
            format!(r##"[{{"op": "set", "refno": "{equi}", "attrs": {{"DESC": "{}"}}}}]"##, "x".repeat(121)),
            0,
            &["op #0 (set", "DESC: 121 characters, the dictionary allows 120"],
        ),
        (
            "dictionary-not-in",
            format!(r##"[{{"op": "set", "refno": "{equi}", "attrs": {{"RATM": "1"}}}}]"##),
            0,
            &["RATM is not an attribute in E3D's dictionary"],
        ),
        (
            "dictionary-set-once",
            format!(r##"[{{"op": "set", "refno": "{equi}", "attrs": {{"UUIDIA": "5 6 7 8"}}}}]"##),
            0,
            &["UUIDIA", "set"],
        ),
        (
            "hunk-of-a-scalar",
            format!(r##"[{{"op": "set", "refno": "{equi}", "attrs": {{"PURP": {{"value": "PIPE", "hunk": true}}}}}}]"##),
            0,
            &["PURP cannot be stored as a hunk"],
        ),
        (
            "anchor-not-a-member",
            format!(r##"[{{"op": "reorder", "refno": "{equi}", "at": {{"before": "/1RX03-MCT"}}}}]"##),
            0,
            &["op #0 (reorder", "is not a member of"],
        ),
        (
            "move-under-a-group-noun",
            format!(r##"[{{"op": "insert", "id": "g", "owner": "/*", "noun": "GPWL"}}, {{"op": "insert", "id": "grou", "owner": "@g", "noun": "GROU"}}, {{"op": "move", "refno": "{equi}", "owner": "@grou"}}]"##),
            2,
            &["op #2 (move", "reference-list noun"],
        ),
        (
            "name-taken",
            r##"[{"op": "insert", "owner": "/1RX03-MSUP", "noun": "BOX", "name": "/E3DIO-REFUSE-EQUI"}]"##.to_string(),
            usize::MAX,
            &["commit:", "/E3DIO-REFUSE-EQUI", "561"],
        ),
    ];
    for (tag, ops, at_op, expect) in cases {
        let run = apply(&copy, tag, &ops, &[]);
        assert!(!run.ok(), "{tag}: went through:\n{}", run.stdout());
        assert_eq!(run.code(), 1, "{tag}");
        let stderr = run.stderr();
        for needle in expect {
            assert!(stderr.contains(needle), "{tag}: stderr lacks {needle:?}:\n{stderr}");
        }
        assert_eq!(run.report["commit"], serde_json::Value::Null, "{tag}");
        if at_op == usize::MAX {
            assert_eq!(run.report["error"]["op"], serde_json::Value::Null, "{tag}: refused at the commit, not at an instruction");
        } else {
            assert_eq!(run.report["error"]["op"], at_op, "{tag}");
        }
        assert!(run.report["error"]["message"].as_str().is_some_and(|m| m.contains(expect[0])), "{tag}: the report carries the reason");
        assert_eq!(copy.current_bytes(), bytes, "{tag}: not a byte written");
        assert_eq!(latest(&copy.path), s1, "{tag}");
    }

    // The file itself, and the fields of an instruction.
    let malformed: Vec<(&str, &str, &str)> = vec![
        ("not-json", r#"[{"op": "insert","#, "not JSON"),
        ("not-a-list", r#"{"comment": "x"}"#, "an instruction file is an array, or an object with \"ops\""),
        ("not-an-op", r#"[{"op": "explode"}]"#, "\"op\": \"explode\" is not an instruction"),
        ("no-op", r#"[{"noun": "BOX"}]"#, "\"op\" is required"),
        ("stray-field", r#"[{"op": "delete", "refno": "=1/2", "nane": "x"}]"#, "unknown field \"nane\" on a delete"),
        ("bad-element", r#"[{"op": "delete", "refno": "twelve"}]"#, "\"twelve\" is not an element"),
        ("bad-position", r#"[{"op": "reorder", "refno": "=1/2", "at": "middle"}]"#, "\"middle\" is not a position"),
        ("both-anchors", r#"[{"op": "reorder", "refno": "=1/2", "at": {"after": "=1/2", "before": "=1/3"}}]"#, "either"),
        ("attr-not-text", r#"[{"op": "set", "refno": "=1/2", "attrs": {"XLEN": 300}}]"#, "attrs.XLEN: a text, or"),
        ("duplicate-id", r#"[{"op": "insert", "id": "a", "owner": "/*", "noun": "GPWL"}, {"op": "insert", "id": "a", "owner": "/*", "noun": "GPWL"}]"#, "id \"a\" is already op #0's"),
    ];
    for (tag, ops, expect) in malformed {
        let run = apply(&copy, tag, ops, &[]);
        assert_eq!(run.code(), 1, "{tag}");
        assert!(run.stderr().contains(expect), "{tag}: stderr lacks {expect:?}:\n{}", run.stderr());
        assert_eq!(run.report["error"]["op"], serde_json::Value::Null, "{tag}: the file, not an instruction");
        assert_eq!(copy.current_bytes(), bytes, "{tag}");
    }

    // The command line: exit 2, before the database is touched.
    let usage = |args: &[&str]| {
        let output = Command::new(EXE).args(args).output().expect("run");
        (output.status.code(), String::from_utf8_lossy(&output.stderr).into_owned())
    };
    let db = copy.path.to_str().expect("utf-8");
    assert!(matches!(usage(&[]), (Some(2), ref e) if e.contains("usage:")));
    assert!(matches!(usage(&["apply", "--db", db]), (Some(2), ref e) if e.contains("--ops is required")));
    assert!(matches!(usage(&["apply", "--db", db, "--ops", "x", "--on-clash", "bogus"]), (Some(2), ref e) if e.contains("refuse or writer-wins")));
    assert!(matches!(usage(&["apply", "--db", db, "--ops", "x", "--base", "soon"]), (Some(2), ref e) if e.contains("not a session number")));
    assert!(matches!(usage(&["apply", "--db", db, "--ops", "x", "--frobnicate"]), (Some(2), ref e) if e.contains("unknown flag")));
    assert!(matches!(usage(&["fly"]), (Some(2), ref e) if e.contains("unknown command")));
    assert_eq!(copy.current_bytes(), bytes);
    let _ = zone;
}

/// `--base` behind the newest session: the commit is a merge onto the newest
/// — the report says what it merged onto and which refno it re-minted — and
/// the facade's clash rule holds: the same attribute changed on both sides is
/// refused with nothing written, unless `--on-clash writer-wins`.
#[test]
fn a_base_behind_the_newest_session_merges_and_reports_the_reminting() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB, DICTIONARY]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let all = zones(&copy.path, 2);
    let (zone, zone2) = (all[0], all[1]);
    let s0 = latest(&copy.path);

    // A: on the base — a BOX (max + 1) and ZONE 2's FUNC.
    let a = apply(
        &copy,
        "a",
        r##"[{"op": "insert", "owner": "/1RX03-MSUP", "noun": "BOX", "name": "/E3DIO-A", "attrs": {"XLEN": "100"}},
             {"op": "set", "refno": "/1RX03-MCT", "attrs": {"FUNC": "A-SIDE", "DESC": "A wrote this"}}]"##,
        &[],
    );
    assert!(a.ok(), "stderr:\n{}", a.stderr());
    let a_box = refno(&a.op(0)["refno"]);
    let s1 = latest(&copy.path);
    assert_eq!(s1, s0 + 1);

    // B: opened at S, commits after A — a merge onto S+1, its BOX re-minted
    // past A's, its own attribute (PURP) composing with A's on ZONE 2.
    let b = apply(
        &copy,
        "b",
        r##"[{"op": "insert", "id": "b", "owner": "/1RX03-MSUP", "noun": "BOX", "name": "/E3DIO-B", "attrs": {"XLEN": "200"}},
             {"op": "set", "refno": "/1RX03-MCT", "attrs": {"PURP": "PIPE"}}]"##,
        &["--base", &s0.to_string()],
    );
    assert!(b.ok(), "stdout:\n{}\nstderr:\n{}", b.stdout(), b.stderr());
    let s2 = latest(&copy.path);
    assert_eq!(s2, s1 + 1);
    assert_eq!(b.report["base_sesno"], s0);
    assert_eq!(b.report["latest_before"], s1);
    assert_eq!(b.commit()["merged_onto"], s1);
    assert_eq!(b.commit()["base_sesno"], s0);
    assert_eq!(b.commit()["written"]["sesno"], s2);
    assert_eq!(base_sesno_of(&copy.path, s2), s0, "BASESN is the base");
    let reminted = b.commit()["reminted"].as_array().expect("a list");
    assert_eq!(reminted.len(), 1);
    assert_eq!(refno(&reminted[0]["given"]), a_box, "both sides minted max + 1 from the same base");
    let b_box = refno(&reminted[0]["written"]);
    assert_ne!(b_box, a_box);
    assert_eq!(refno(&b.op(0)["given"]), a_box);
    assert_eq!(refno(&b.op(0)["refno"]), b_box, "the instruction's refno is the one written");
    assert_eq!(b.op(0)["taken_in_latest"], true);
    assert!(b.stdout().contains(&format!("re-minted        : {a_box} -> {b_box}")), "stdout:\n{}", b.stdout());
    assert_eq!(named(&copy.path, "/E3DIO-A"), Some(a_box));
    assert_eq!(named(&copy.path, "/E3DIO-B"), Some(b_box), "the NAME key followed the re-minting");
    let members = parsed(&copy.path, zone).members;
    assert!(members.contains(&a_box) && members.contains(&b_box), "both boxes under the ZONE");
    let zone2_x = extract(&copy.path, zone2);
    assert_eq!(value(&zone2_x, "FUNC"), Some(DescriptorValue::Text("A-SIDE".into())), "A's, untouched by B");
    assert!(matches!(value(&zone2_x, "PURP"), Some(DescriptorValue::Word { raw, .. }) if raw == db1_hash("PIPE")), "B's, composed");
    assert_eq!(tally(&copy.path, s1, s2), (0, 1, 2), "B alone: its BOX, the ZONE's list, ZONE 2");

    // C: opened at S too, changing ZONE 2's FUNC that A changed — a clash,
    // refused by default with nothing written; taken under writer-wins.
    let bytes = copy.current_bytes();
    let c_ops = r##"[{"op": "set", "refno": "/1RX03-MCT", "attrs": {"FUNC": "C-SIDE"}}]"##;
    let c = apply(&copy, "c", c_ops, &["--base", &s0.to_string()]);
    assert!(!c.ok());
    assert_eq!(c.code(), 1);
    assert!(c.stderr().contains("merge refused by 1 clash(es)") && c.stderr().contains("FUNC was changed here and in the committed session"), "stderr:\n{}", c.stderr());
    assert_eq!(c.report["error"]["op"], serde_json::Value::Null, "the commit, not an instruction");
    assert_eq!(c.report["on_clash"], "refuse");
    assert_eq!(copy.current_bytes(), bytes);
    assert_eq!(latest(&copy.path), s2);

    let c = apply(&copy, "c-wins", c_ops, &["--base", &s0.to_string(), "--on-clash", "writer-wins"]);
    assert!(c.ok(), "stderr:\n{}", c.stderr());
    assert_eq!(c.report["on_clash"], "writer-wins");
    assert_eq!(c.commit()["merged_onto"], s2);
    assert_eq!(latest(&copy.path), s2 + 1);
    let zone2_x = extract(&copy.path, zone2);
    assert_eq!(value(&zone2_x, "FUNC"), Some(DescriptorValue::Text("C-SIDE".into())), "the writer's where they clashed");
    assert_eq!(value(&zone2_x, "DESC"), Some(DescriptorValue::Text("A wrote this".into())), "A's where they did not");
    assert!(matches!(value(&zone2_x, "PURP"), Some(DescriptorValue::Word { raw, .. }) if raw == db1_hash("PIPE")), "B's where they did not");

    // A conflict (562: changed here, deleted there) is refused the same way.
    let bytes = copy.current_bytes();
    let d = apply(&copy, "d", r##"[{"op": "delete", "refno": "/E3DIO-A"}]"##, &[]);
    assert!(d.ok(), "stderr:\n{}", d.stderr());
    let e = apply(&copy, "e", &format!(r##"[{{"op": "set", "refno": "{a_box}", "attrs": {{"XLEN": "1"}}}}]"##), &["--base", &s2.to_string()]);
    assert!(!e.ok());
    assert!(e.stderr().contains("conflict") && e.stderr().contains("562"), "stderr:\n{}", e.stderr());
    assert_ne!(copy.current_bytes(), bytes, "D went through");
    assert_eq!(latest(&copy.path), s2 + 2, "E did not");
}
