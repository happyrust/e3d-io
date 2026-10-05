//! Pre-minted primitive baseline survives changes to allocation policy.
mod common;
#[path = "../examples/write_scale_probe/batch.rs"]
mod batch;

use common::write_fixture::{DbCopy, AMS8000};
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::SessionDate;
use e3d_io::write::{write_element_session_with, ElementWrite, SessionClock, SessionRequest, WriteOptions};
use e3d_io::ReadOnlyEngine;

#[test]
fn deterministic_pre_minted_batches_read_back() {
    for count in [32, 128, 257] {
        let copy = DbCopy::of(AMS8000);
        let mut engine = ReadOnlyEngine::open(&copy.path).unwrap();
        let mut templates = TemplateProvider::open(common::DESVIR, Some(0x000B_0692)).unwrap();
        let (writes, refs) = batch::build(&mut engine, &mut templates, count).unwrap();
        let date = SessionDate { year: 2026, month: 10, day: 5, hour: 0, minute: 0, second: 0, millisecond: 0 };
        let clock = SessionClock::fixed(date, date);
        write_element_session_with(&copy.path, &SessionRequest { user: "e3dio", comment: "fixed scale baseline", clock: Some(clock) }, &writes, &[], WriteOptions { templates: Some(&mut templates) }).unwrap();
        let mut reader = ReadOnlyEngine::open(&copy.path).unwrap();
        for r in refs {
            let bytes = reader.find_element(r).unwrap().unwrap().raw_bytes;
            let element = ParsedElement::parse_as(&bytes, reader.text_format()).unwrap();
            assert_eq!(element.refno, r);
            let owner = reader.find_element(element.owner).unwrap().unwrap();
            assert!(ParsedElement::parse_as(&owner.raw_bytes, reader.text_format()).unwrap().members.contains(&r));
        }
    }
}

// Feature: write-path-phase-3, Property 23: byte-neutral staging preserves golden bytes.
#[test]
fn mixed_pre_minted_scale_batches_read_complete_member_order() {
    for n in [10_000, 100_000] {
        let copy = DbCopy::of(AMS8000);
        let mut templates = TemplateProvider::open(common::DESVIR, Some(0x000B_0692)).unwrap();
        let date = SessionDate { year: 2026, month: 10, day: 5, hour: 0, minute: 0, second: 0, millisecond: 0 };
        let request = SessionRequest { user: "e3dio", comment: "fixed mixed scale probe", clock: Some(SessionClock::fixed(date, date)) };
        let mut engine = ReadOnlyEngine::open(&copy.path).unwrap();
        let (setup, fixtures) = batch::build(&mut engine, &mut templates, 1100).unwrap();
        let ElementWrite::Update(owner) = setup.last().unwrap() else { panic!("owner missing") };
        let owner = owner.refno;
        drop(engine);
        write_element_session_with(&copy.path, &request, &setup, &[], WriteOptions { templates: Some(&mut templates) }).unwrap();
        let mut engine = ReadOnlyEngine::open(&copy.path).unwrap();
        let (writes, refs) = batch::build_mixed(&mut engine, &mut templates, owner, &fixtures, n).unwrap();
        let ElementWrite::Update(owner_draft) = writes.last().unwrap() else { panic!("owner missing") };
        let expected = owner_draft.members().to_vec();
        drop(engine);
        write_element_session_with(&copy.path, &request, &writes, &[], WriteOptions { templates: Some(&mut templates) }).unwrap();
        let mut reader = ReadOnlyEngine::open(&copy.path).unwrap();
        let bytes = reader.find_element(owner).unwrap().unwrap().raw_bytes;
        assert_eq!(ParsedElement::parse_as(&bytes, reader.text_format()).unwrap().members, expected);
        for r in refs.iter().chain(&fixtures[..1000]) {
            let bytes = reader.find_element(*r).unwrap().unwrap().raw_bytes;
            let element = ParsedElement::parse_as(&bytes, reader.text_format()).unwrap();
            assert_eq!(element.refno, *r);
            assert_eq!(element.owner, owner);
        }
        for r in &fixtures[1000..] { assert!(reader.find_element(*r).unwrap().is_none()); }
    }
}
