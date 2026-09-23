//! T1c (`docs/plans/2026-09-22-write-path-phase-2-plan.md` §2): a
//! `Transaction` opened at a session that is no longer the newest when it
//! commits. The construction is stage 3's: C0 is the sample at session S;
//! writer A commits S+1 through its own transaction; writer B, opened at S,
//! commits S+2 — a merge. Disjoint edits are the union of both (B's inserted
//! refno, which A had taken meanwhile, is re-minted); the same attribute
//! changed on both sides is refused by default (`OnClash::Refuse`, the clashes
//! named) and taken by B under `OnClash::WriterWins`; a change over a delete is
//! 562, a delete over a change 590, a name taken 561 — and a refused commit
//! writes nothing. A writer that gets its page 0 in between B's staging and
//! B's commit is merged over once.

mod common;

use common::write_fixture::{DbCopy, AMS8000};
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::engine::ScanTier;
use e3d_io::record::descriptor::{DescriptorValue, ElementExtraction};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionDate, SessionSelector};
use e3d_io::write::merge::code;
use e3d_io::write::{ClashSite, OnClash, SessionClock, SessionRequest, Transaction, TransactionError};
use e3d_io::{ReadOnlyEngine, RefNo};

use std::path::Path;

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

fn clock() -> SessionClock {
    SessionClock::fixed(
        SessionDate { year: 2026, month: 9, day: 23, hour: 16, minute: 0, second: 0, millisecond: 0 },
        SessionDate { year: 2026, month: 9, day: 23, hour: 8, minute: 0, second: 0, millisecond: 0 },
    )
}

fn provider() -> TemplateProvider {
    TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir")
}

fn request(user: &'static str, comment: &'static str) -> SessionRequest<'static> {
    SessionRequest { user, comment, clock: Some(clock()) }
}

/// The first `n` ZONEs in scan order.
fn zones(path: &Path, n: usize) -> Vec<RefNo> {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let zone_hash = db1_hash("ZONE");
    engine
        .scan_elements(ScanTier::Header)
        .expect("scan")
        .map(|e| e.expect("header"))
        .filter(|e| e.noun_hash == zone_hash)
        .map(|e| e.refno)
        .take(n)
        .collect()
}

fn extract(path: &Path, refno: RefNo) -> ElementExtraction {
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let mut prov = provider();
    engine.extract_element_with_descriptors(refno, &attlib, &mut prov).expect("extract")
}

fn value(extraction: &ElementExtraction, name: &str) -> Option<DescriptorValue> {
    extraction.attributes.iter().find(|a| a.name == name).and_then(|a| a.value.clone())
}

fn text(path: &Path, refno: RefNo, name: &str) -> Option<String> {
    match value(&extract(path, refno), name) {
        Some(DescriptorValue::Text(text)) => Some(text),
        _ => None,
    }
}

fn real(path: &Path, refno: RefNo, name: &str) -> Option<f64> {
    match value(&extract(path, refno), name) {
        Some(DescriptorValue::Real(v)) => Some(v),
        _ => None,
    }
}

fn members(path: &Path, refno: RefNo) -> Vec<RefNo> {
    let mut engine = ReadOnlyEngine::open(path).expect("open");
    let view = engine.find_element(refno).expect("lookup").unwrap_or_else(|| panic!("{refno} is indexed"));
    ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).expect("parse").members
}

fn owner(path: &Path, refno: RefNo) -> Option<RefNo> {
    ReadOnlyEngine::open(path).expect("open").find_element(refno).expect("lookup").map(|view| view.header().owner)
}

fn named(path: &Path, name: &str) -> Option<RefNo> {
    ReadOnlyEngine::open(path).expect("open").find_by_name(name).expect("lookup")
}

fn latest(path: &Path) -> u32 {
    DbView::open(path).expect("open").session().sesno
}

fn base_of(path: &Path, sesno: u32) -> u32 {
    DbView::open_at(path, SessionSelector::Exact(sesno)).expect("open").session().tail.expect("tail").base_sesno
}

/// `(deleted, inserted, modified)` between two sessions.
fn tally(path: &Path, from: u32, to: u32) -> (u64, u64, u64) {
    let mut view = DbView::open(path).expect("open");
    let diff = view.diff_sessions(SessionSelector::Exact(from), SessionSelector::Exact(to)).expect("diff");
    let tally = diff.tally();
    (tally.deleted, tally.inserted, tally.modified)
}

/// A BOX inserted under `zone`, named, with `XLEN` set — the element the
/// scenarios edit, committed as its own session first.
fn seed_box(copy: &DbCopy, zone: RefNo, name: &str, xlen: &str) -> RefNo {
    let mut prov = provider();
    let mut tx = Transaction::open(&copy.path, &mut prov).expect("open tx");
    let the_box = tx.insert(zone, db1_hash("BOX"), Some(name), |_, _| Ok(())).expect("box");
    tx.set(the_box, "XLEN", xlen).expect("XLEN");
    tx.commit(&request("e3dio", "e3d-io T1c: a box to edit")).expect("commit");
    the_box
}

/// B is opened at S and A commits S+1 before B does: B's commit is a merge onto
/// S+1 — S+2 is A ∪ B with `BASESN` = S, the diff S+1..S+2 is B alone, and B's
/// inserted refno, which A had taken meanwhile, is re-minted past A's and the
/// mapping reported; every owner list and name key followed it. A transaction
/// opened explicitly at S after A committed merges the same way.
#[test]
fn a_transaction_opened_at_the_base_merges_onto_what_was_committed_meanwhile() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let all_zones = zones(&copy.path, 2);
    let (zone_a, zone_b) = (all_zones[0], all_zones[1]);
    let s = latest(&copy.path);
    let a_before = members(&copy.path, zone_a);
    let b_before = members(&copy.path, zone_b);

    // --- B opens at S and builds its batch: a named BOX under ZONE B, ZONE B's DESC ---
    let mut prov_b = provider();
    let mut b = Transaction::open(&copy.path, &mut prov_b).expect("open B");
    assert_eq!(b.base_sesno(), s);
    let box_b_given = b.insert(zone_b, db1_hash("BOX"), Some("/E3DIO-MERGE-B"), |_, _| Ok(())).expect("B's box");
    b.set(box_b_given, "XLEN", "222").expect("XLEN");
    b.set(zone_b, "DESC", "B described this zone").expect("DESC");

    // --- A commits S+1 meanwhile: a named BOX under ZONE A (the same refno B was given), ZONE A's DESC ---
    let box_a = {
        let mut prov_a = provider();
        let mut a = Transaction::open(&copy.path, &mut prov_a).expect("open A");
        let box_a = a.insert(zone_a, db1_hash("BOX"), Some("/E3DIO-MERGE-A"), |_, _| Ok(())).expect("A's box");
        a.set(box_a, "XLEN", "111").expect("XLEN");
        a.set(zone_a, "DESC", "A described this zone").expect("DESC");
        a.commit(&request("A", "e3d-io T1c: writer A")).expect("A commits");
        box_a
    };
    assert_eq!(latest(&copy.path), s + 1);
    assert_eq!(box_a, box_b_given, "both minted max + 1 from the same base");
    let after_a = copy.current_bytes();

    // --- B commits: a merge ---
    let commit = b.commit_reporting(&request("B", "e3d-io T1c: writer B merges")).expect("B merges");
    assert_eq!(commit.base_sesno, s);
    assert_eq!(commit.merged_onto, Some(s + 1));
    assert!(!commit.retried_after_move);
    assert_eq!(commit.report.written.sesno, s + 2);
    assert_eq!(commit.reminted.len(), 1, "one insert had its refno taken: {:?}", commit.reminted);
    let (given, written) = commit.reminted[0];
    assert_eq!(given, box_b_given);
    assert_eq!(written, RefNo::new(box_a.word0, box_a.word1 + 1), "re-minted past A's");
    let box_b = written;

    // --- S+2 is A ∪ B, based on S ---
    assert_eq!(latest(&copy.path), s + 2);
    assert_eq!(base_of(&copy.path, s + 2), s, "BASESN records the base B merged from");
    assert_eq!(tally(&copy.path, s, s + 2), (0, 2, 2), "two boxes, two zones");
    assert_eq!(tally(&copy.path, s + 1, s + 2), (0, 1, 1), "B alone: its box and its zone");
    assert_eq!(owner(&copy.path, box_a), Some(zone_a), "A's box survived the merge");
    assert_eq!(owner(&copy.path, box_b), Some(zone_b), "B's box is there under its owner, re-minted");
    assert_eq!(real(&copy.path, box_a, "XLEN"), Some(111.0));
    assert_eq!(real(&copy.path, box_b, "XLEN"), Some(222.0));
    assert_eq!(text(&copy.path, zone_a, "DESC").as_deref(), Some("A described this zone"));
    assert_eq!(text(&copy.path, zone_b, "DESC").as_deref(), Some("B described this zone"));
    let mut a_after = a_before.clone();
    a_after.push(box_a);
    assert_eq!(members(&copy.path, zone_a), a_after, "ZONE A lists A's box");
    let mut b_after = b_before.clone();
    b_after.push(box_b);
    assert_eq!(members(&copy.path, zone_b), b_after, "ZONE B lists B's box under its new number");
    assert_eq!(named(&copy.path, "/E3DIO-MERGE-A"), Some(box_a));
    assert_eq!(named(&copy.path, "/E3DIO-MERGE-B"), Some(box_b), "the NAME key followed the re-minted refno");
    assert_eq!(text(&copy.path, box_b, "NAME").as_deref(), Some("/E3DIO-MERGE-B"));
    // A's committed pages were not touched by B's merge (page 0 aside).
    let bytes = copy.current_bytes();
    let page_bytes = 4 * u32::from_be_bytes(copy.original[52..56].try_into().unwrap()) as usize;
    assert_eq!(&bytes[page_bytes..after_a.len()], &after_a[page_bytes..], "every page A wrote stands");

    // --- a transaction opened explicitly at S, after both, merges the same way ---
    let (zone_c_box, commit) = {
        let mut prov = provider();
        let mut c = Transaction::open_at(&copy.path, SessionSelector::Exact(s), &mut prov).expect("open at S");
        assert_eq!(c.base_sesno(), s);
        let the_box = c.insert(zone_a, db1_hash("BOX"), Some("/E3DIO-MERGE-C"), |_, _| Ok(())).expect("C's box");
        c.set(zone_a, "FUNC", "C-FUNC").expect("FUNC");
        (the_box, c.commit_reporting(&request("C", "e3d-io T1c: opened at S")).expect("C merges"))
    };
    assert_eq!(commit.merged_onto, Some(s + 2));
    assert_eq!(commit.reminted, vec![(zone_c_box, RefNo::new(box_b.word0, box_b.word1 + 1))], "past both boxes");
    let box_c = commit.reminted[0].1;
    assert_eq!(base_of(&copy.path, s + 3), s);
    assert_eq!(tally(&copy.path, s + 2, s + 3), (0, 1, 1));
    assert_eq!(text(&copy.path, zone_a, "DESC").as_deref(), Some("A described this zone"), "A's DESC stands");
    assert_eq!(text(&copy.path, zone_a, "FUNC").as_deref(), Some("C-FUNC"));
    // C's box was appended after the base's last member; the member merge
    // anchors it right after that member — its preceding shared sibling
    // (`sub_5B20CA0`) — so it lands before A's box, which the committed list
    // had appended after the same member.
    let mut a_final = a_before.clone();
    a_final.push(box_c);
    a_final.push(box_a);
    assert_eq!(members(&copy.path, zone_a), a_final, "the base's list, then C's box after its anchor, then A's");
}

/// The same attribute changed on both sides — an explicit one (the ZONE's
/// `DESC`) and an implicit one (the BOX's `XLEN`, named through the template)
/// — is refused by default with each clash named and nothing written; the
/// disjoint changes in the same batch (`FUNC`, `YLEN`) do not save it. Under
/// `OnClash::WriterWins` the same batch goes through and B's values stand
/// where they clashed, A's where they did not.
#[test]
fn a_same_attribute_clash_is_refused_by_default_and_taken_under_writer_wins() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let zone = zones(&copy.path, 1)[0];
    let the_box = seed_box(&copy, zone, "/E3DIO-CLASH", "100");
    let s = latest(&copy.path);

    // A: ZONE DESC, BOX XLEN and ZLEN.
    {
        let mut prov = provider();
        let mut a = Transaction::open(&copy.path, &mut prov).expect("open A");
        a.set(zone, "DESC", "A's description").expect("DESC");
        a.set(the_box, "XLEN", "101").expect("XLEN");
        a.set(the_box, "ZLEN", "301").expect("ZLEN");
        a.commit(&request("A", "e3d-io T1c: A")).expect("A commits");
    }
    let after_a = copy.current_bytes();

    // B, at S: ZONE DESC (clash) and FUNC (not), BOX XLEN (clash) and YLEN (not).
    let build_b = |b: &mut Transaction<'_>| {
        b.set(zone, "DESC", "B's description").expect("DESC");
        b.set(zone, "FUNC", "B-FUNC").expect("FUNC");
        b.set(the_box, "XLEN", "102").expect("XLEN");
        b.set(the_box, "YLEN", "202").expect("YLEN");
    };
    {
        let mut prov = provider();
        let mut b = Transaction::open_at(&copy.path, SessionSelector::Exact(s), &mut prov).expect("open B");
        build_b(&mut b);
        match b.commit(&request("B", "e3d-io T1c: B clashes")) {
            Err(TransactionError::Clashes(clashes)) => {
                assert_eq!(clashes.len(), 2, "{clashes:?}");
                let desc = clashes.iter().find(|c| c.refno == zone).expect("the ZONE's clash");
                assert_eq!(desc.site, ClashSite::ExplicitAttribute(db1_hash("DESC")));
                assert_eq!(desc.attribute, Some(db1_hash("DESC")));
                assert!(desc.detail.contains("DESC"), "{}", desc.detail);
                let xlen = clashes.iter().find(|c| c.refno == the_box).expect("the BOX's clash");
                assert!(matches!(xlen.site, ClashSite::ImplicitWord(_)), "{:?}", xlen.site);
                assert_eq!(xlen.attribute, Some(db1_hash("XLEN")), "the template names the slot's attribute");
                assert!(xlen.detail.contains("XLEN"), "{}", xlen.detail);
            }
            other => panic!("expected the clashes to be refused, got {other:?}"),
        }
    }
    assert_eq!(copy.current_bytes(), after_a, "a refused commit writes nothing");
    assert_eq!(latest(&copy.path), s + 1);

    // The same batch, the writer winning the ties.
    {
        let mut prov = provider();
        let mut b = Transaction::open_at(&copy.path, SessionSelector::Exact(s), &mut prov)
            .expect("open B")
            .with_on_clash(OnClash::WriterWins);
        build_b(&mut b);
        let commit = b.commit_reporting(&request("B", "e3d-io T1c: B wins")).expect("B merges");
        assert_eq!(commit.merged_onto, Some(s + 1));
        assert!(commit.reminted.is_empty());
    }
    assert_eq!(latest(&copy.path), s + 2);
    assert_eq!(base_of(&copy.path, s + 2), s);
    assert_eq!(tally(&copy.path, s + 1, s + 2), (0, 0, 2));
    assert_eq!(text(&copy.path, zone, "DESC").as_deref(), Some("B's description"), "the clash went B's way");
    assert_eq!(text(&copy.path, zone, "FUNC").as_deref(), Some("B-FUNC"));
    assert_eq!(real(&copy.path, the_box, "XLEN"), Some(102.0), "the clash went B's way");
    assert_eq!(real(&copy.path, the_box, "YLEN"), Some(202.0), "B's disjoint change");
    assert_eq!(real(&copy.path, the_box, "ZLEN"), Some(301.0), "A's disjoint change stands");
}

/// The three conflicts `core.dll` refuses a `SAVEWORK` over, each refused
/// with nothing written: 562 B changes an element A deleted, 590 B deletes an
/// element A changed, 561 B names an element with a name A took.
#[test]
fn a_change_over_a_delete_a_delete_over_a_change_and_a_taken_name_are_refused() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let zone = zones(&copy.path, 1)[0];
    let deleted_by_a = seed_box(&copy, zone, "/E3DIO-562", "1");
    let changed_by_a = seed_box(&copy, zone, "/E3DIO-590", "2");
    let s = latest(&copy.path);

    // A: deletes one box, changes the other, names a new one.
    {
        let mut prov = provider();
        let mut a = Transaction::open(&copy.path, &mut prov).expect("open A");
        a.delete(deleted_by_a).expect("delete");
        a.set(changed_by_a, "XLEN", "22").expect("XLEN");
        a.insert(zone, db1_hash("BOX"), Some("/E3DIO-561"), |_, _| Ok(())).expect("named box");
        a.commit(&request("A", "e3d-io T1c: A deletes, changes, names")).expect("A commits");
    }
    let after_a = copy.current_bytes();

    let refused = |edit: &dyn Fn(&mut Transaction<'_>), expect_code: u32| {
        let mut prov = provider();
        let mut b = Transaction::open_at(&copy.path, SessionSelector::Exact(s), &mut prov).expect("open B");
        edit(&mut b);
        match b.commit(&request("B", "e3d-io T1c: B conflicts")) {
            Err(TransactionError::Conflicts(conflicts)) => {
                assert!(conflicts.iter().any(|c| c.code == expect_code), "expected {expect_code}, got {conflicts:?}");
            }
            other => panic!("expected conflict {expect_code}, got {other:?}"),
        }
        assert_eq!(copy.current_bytes(), after_a, "a refused commit writes nothing");
    };
    refused(&|b| b.set(deleted_by_a, "XLEN", "11").expect("set"), code::CHANGED_UNDER_DELETE);
    refused(&|b| b.delete(changed_by_a).expect("delete"), code::DELETED_UNDER_CHANGE);
    refused(
        &|b| {
            // B's fresh refno collides with A's named box and is re-minted; the
            // name it wants is A's, and that is what is refused.
            b.insert(zone, db1_hash("BOX"), Some("/E3DIO-561"), |_, _| Ok(())).expect("insert");
        },
        code::NAME_TAKEN,
    );
    assert_eq!(latest(&copy.path), s + 1);
}

/// Another writer gets its page 0 in between B's staging and B's page-0
/// write: the commit finds `LSPGNO` moved, and the facade stages B once more —
/// now a merge onto the other writer's session — before it gives up.
#[test]
fn a_writer_that_commits_between_staging_and_page_0_is_merged_over_once() {
    if common::missing(&[AMS8000, DESVIR, ATTLIB]) {
        return;
    }
    let copy = DbCopy::of(AMS8000);
    let all_zones = zones(&copy.path, 2);
    let (zone_a, zone_b) = (all_zones[0], all_zones[1]);
    let s = latest(&copy.path);
    let bytes_s = copy.current_bytes();

    // The other writer's timeline, captured: A commits S+1 on the copy, then
    // the file is put back to S — so it can be dropped in "between" later.
    let box_a = {
        let mut prov = provider();
        let mut a = Transaction::open(&copy.path, &mut prov).expect("open A");
        let box_a = a.insert(zone_a, db1_hash("BOX"), Some("/E3DIO-MOVED-A"), |_, _| Ok(())).expect("A's box");
        a.commit(&request("A", "e3d-io T1c: the writer in between")).expect("A commits");
        box_a
    };
    let bytes_a = copy.current_bytes();
    std::fs::write(&copy.path, &bytes_s).expect("back to S");
    assert_eq!(latest(&copy.path), s);

    // B at S; its first staging lands on top of S; before its page 0 goes in,
    // the file becomes what A committed (A's page 0 names A's session page).
    let mut prov = provider();
    let mut b = Transaction::open(&copy.path, &mut prov).expect("open B");
    let box_b_given = b.insert(zone_b, db1_hash("BOX"), Some("/E3DIO-MOVED-B"), |_, _| Ok(())).expect("B's box");
    b.set(zone_b, "DESC", "B was here").expect("DESC");
    let mut stagings = Vec::new();
    let commit = b
        .commit_observed(&request("B", "e3d-io T1c: B, moved once"), |staged| {
            stagings.push(staged.staged().sesno);
            if stagings.len() == 1 {
                std::fs::write(&copy.path, &bytes_a).expect("A gets in between");
            }
        })
        .expect("B commits on the second staging");
    assert_eq!(stagings, vec![s + 1, s + 2], "staged on S, moved, staged again on S+1");
    assert!(commit.retried_after_move);
    assert_eq!(commit.merged_onto, Some(s + 1), "the second staging was a merge onto A's session");
    assert_eq!(commit.report.written.sesno, s + 2);
    assert_eq!(commit.reminted, vec![(box_b_given, RefNo::new(box_a.word0, box_a.word1 + 1))]);
    let box_b = commit.reminted[0].1;

    assert_eq!(latest(&copy.path), s + 2);
    assert_eq!(base_of(&copy.path, s + 2), s);
    assert_eq!(owner(&copy.path, box_a), Some(zone_a), "A's box is there");
    assert_eq!(owner(&copy.path, box_b), Some(zone_b), "and B's, re-minted");
    assert_eq!(named(&copy.path, "/E3DIO-MOVED-A"), Some(box_a));
    assert_eq!(named(&copy.path, "/E3DIO-MOVED-B"), Some(box_b));
    assert_eq!(text(&copy.path, zone_b, "DESC").as_deref(), Some("B was here"));
    assert_eq!(tally(&copy.path, s, s + 2), (0, 2, 2));
}
