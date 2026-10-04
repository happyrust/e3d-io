//! Allocation floor model, refusals, historical base and reminting on copies.
mod common;

use common::write_fixture::{DbCopy, AMS8000};
use e3d_io::index::IndexCursor;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionDate};
use e3d_io::spatial::SpatialPolicy;
use e3d_io::write::{CounterUse, SessionClock, SessionRequest, Transaction, TransactionError};
use e3d_io::{ReadOnlyEngine, RefNo};
use proptest::prelude::*;

fn provider() -> TemplateProvider {
    TemplateProvider::open(common::DESVIR, Some(0x000B_0692)).unwrap()
}

fn request() -> SessionRequest<'static> {
    let d = SessionDate { year: 2026, month: 10, day: 5, hour: 0, minute: 0, second: 0, millisecond: 0 };
    SessionRequest { user: "e3dio", comment: "allocation floor test", clock: Some(SessionClock::fixed(d, d)) }
}

fn owner_and_max(path: &std::path::Path) -> (RefNo, u32) {
    let mut engine = ReadOnlyEngine::open(path).unwrap();
    let owner = engine.find_by_name("/1RX03-MSUP").unwrap().unwrap();
    let max = engine.indexed_refnos().unwrap().into_iter().filter(|(r, _, _)| r.word0 == owner.word0).map(|(r, _, _)| r.word1).max().unwrap();
    (owner, max)
}

fn set_counter(copy: &DbCopy, floor: u32) {
    let mut bytes = copy.current_bytes();
    // AMS page 1's first triple is checked before altering its counter.
    assert_eq!(u32::from_be_bytes(bytes[2060..2064].try_into().unwrap()), 24384);
    bytes[2064..2068].copy_from_slice(&floor.to_be_bytes());
    std::fs::write(&copy.path, bytes).unwrap();
}

fn insert(tx: &mut Transaction<'_>, owner: RefNo, name: Option<&str>) -> RefNo {
    tx.insert(owner, e3d_attlib::db1_hash("EQUI"), name, |_, _| Ok(())).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn committed_index_matches_floor_model(delta in -2i32..24, count in 1usize..5, delete_first in any::<bool>()) {
        let copy = DbCopy::of(AMS8000);
        let (owner, max) = owner_and_max(&copy.path);
        let floor = (i64::from(max) + 1 + i64::from(delta)) as u32;
        set_counter(&copy, floor);
        let first = (max + 1).max(floor);
        let mut templates = provider();
        let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
        prop_assert_eq!(tx.allocation_basis().counter, CounterUse::NextAvailable);
        let mut expected = Vec::new();
        for i in 0..count {
            let r = insert(&mut tx, owner, None);
            prop_assert_eq!(r, RefNo::new(owner.word0, first + i as u32));
            expected.push(r);
        }
        tx.commit(&request()).unwrap();
        let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
        let deleted = if delete_first { let r=expected.remove(0); tx.delete(r).unwrap(); Some(r) } else { None };
        let next = insert(&mut tx, owner, None);
        prop_assert_eq!(next.word1, first + count as u32);
        expected.push(next);
        tx.commit(&request()).unwrap();
        let mut reader = DbView::open(&copy.path).unwrap();
        let mut cursor = IndexCursor::new(&mut reader);
        for r in expected { prop_assert_eq!(cursor.seek(r).unwrap().found.unwrap().refno, r); }
        if let Some(r) = deleted { prop_assert!(cursor.seek(r).unwrap().found.is_none()); }
    }
}

#[test]
fn unavailable_or_missing_bucket_uses_index_and_exhaustion_writes_nothing() {
    for word in [0, 1, 2, u32::MAX] {
        let copy = DbCopy::of(AMS8000);
        let (owner, max) = owner_and_max(&copy.path);
        let mut bytes = copy.current_bytes();
        if word == u32::MAX {
            set_counter(&copy, word);
            bytes = copy.current_bytes();
        } else if word == 2 {
            bytes[2056..2060].copy_from_slice(&2u32.to_be_bytes());
            std::fs::write(&copy.path, &bytes).unwrap();
        } else {
            let at = if word == 0 { 2048 } else { 2056 };
            bytes[at..at+4].copy_from_slice(&99u32.to_be_bytes());
            std::fs::write(&copy.path, &bytes).unwrap();
        }
        let hash = common::golden::sha256_hex(&bytes);
        let mut templates = provider();
        let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
        if word == u32::MAX {
            assert!(matches!(tx.insert(owner, e3d_attlib::db1_hash("EQUI"), None, |_, _| Ok(())), Err(TransactionError::RefnoExhausted { .. })));
        } else {
            let expected_use = if word == 2 { CounterUse::NextAvailable } else { CounterUse::Unavailable };
            assert_eq!(tx.allocation_basis().counter, expected_use);
            assert_eq!(insert(&mut tx, owner, None).word1, max + 1);
        }
        drop(tx);
        let after = copy.current_bytes();
        assert_eq!(after.len(), bytes.len());
        assert_eq!(common::golden::sha256_hex(&after), hash);
    }
}

#[test]
fn duplicate_bucket_extents_are_unavailable_and_fall_back() {
    let copy = DbCopy::of(AMS8000);
    let (owner, max) = owner_and_max(&copy.path);
    let mut bytes = copy.current_bytes();
    bytes[2056..2060].copy_from_slice(&8u32.to_be_bytes());
    let first = bytes[2060..2072].to_vec();
    bytes[2072..2084].copy_from_slice(&first);
    std::fs::write(&copy.path, &bytes).unwrap();
    let mut templates = provider();
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    assert_eq!(tx.allocation_basis().counter, CounterUse::Unavailable);
    assert!(tx.allocation_basis().bucket_error.as_ref().unwrap().contains("duplicate"));
    assert_eq!(insert(&mut tx, owner, None).word1, max + 1);
    drop(tx);
    assert_eq!(copy.current_bytes(), bytes);
}

#[test]
fn historical_base_and_remint_use_their_pinned_index_and_current_bucket() {
    let copy = DbCopy::of(AMS8000);
    let (owner, max) = owner_and_max(&copy.path);
    set_counter(&copy, max + 50);
    let mut reader = DbView::open(&copy.path).unwrap();
    let chain = reader.walk_sessions().unwrap();
    let base = chain[chain.len()/2].sesno;
    let mut a_templates = provider();
    let mut a = Transaction::open_at(&copy.path, base, &mut a_templates).unwrap().with_spatial(SpatialPolicy::Off);
    let given = insert(&mut a, owner, Some("/ALLOC-A"));
    assert_eq!(given.word1, max + 50);
    let mut b_templates = provider();
    let mut b = Transaction::open(&copy.path, &mut b_templates).unwrap().with_spatial(SpatialPolicy::Off);
    let occupied = insert(&mut b, owner, Some("/ALLOC-B"));
    assert_eq!(occupied, given);
    b.commit(&request()).unwrap();
    let report = a.commit_reporting(&request()).unwrap();
    assert_eq!(report.reminted, vec![(given, RefNo::new(owner.word0, max + 51))]);
    let mut reader = ReadOnlyEngine::open(&copy.path).unwrap();
    assert_eq!(reader.find_by_name("/ALLOC-B").unwrap().unwrap(), occupied);
    assert_eq!(reader.find_by_name("/ALLOC-A").unwrap().unwrap().word1, max + 51);
}

#[test]
fn deleting_the_live_max_does_not_lower_a_higher_bucket_floor() {
    let copy = DbCopy::of(AMS8000);
    let (owner, max) = owner_and_max(&copy.path);
    let mut templates = provider();
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    let r = insert(&mut tx, owner, None);
    tx.commit(&request()).unwrap();
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    tx.delete(r).unwrap(); tx.commit(&request()).unwrap();
    set_counter(&copy, r.word1 + 1);
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
    let next = insert(&mut tx, owner, None);
    assert_eq!(tx.allocation_basis().extents[&owner.word0].index_max_word1, Some(max));
    assert_eq!(next.word1, r.word1 + 1);
    tx.commit(&request()).unwrap();
}

#[test]
fn moved_retry_rebuilds_floor_and_remint_and_stops_after_two_attempts() {
    use e3d_io::write::WriteError;
    for twice in [false, true] {
        let copy = DbCopy::of(AMS8000);
        let (owner, max) = owner_and_max(&copy.path);
        set_counter(&copy, max + 50);
        let base = copy.current_bytes();
        let mut templates = provider();
        let mut a = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
        let occupied = insert(&mut a, owner, Some("/ALLOC-RETRY-A"));
        a.commit(&request()).unwrap();
        set_counter(&copy, max + 100);
        let first = copy.current_bytes();
        let mut c = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
        insert(&mut c, owner, Some("/ALLOC-RETRY-C")); c.commit(&request()).unwrap();
        let second = copy.current_bytes();
        std::fs::write(&copy.path, &base).unwrap();
        let mut b = Transaction::open(&copy.path, &mut templates).unwrap().with_spatial(SpatialPolicy::Off);
        let given = insert(&mut b, owner, Some("/ALLOC-RETRY-B"));
        assert_eq!(given, occupied);
        let mut calls = 0;
        let result = b.commit_observed(&request(), |_| {
            calls += 1;
            if calls == 1 || twice { std::fs::write(&copy.path, if calls == 1 { &first } else { &second }).unwrap(); }
        });
        assert_eq!(calls, 2);
        if twice {
            assert!(matches!(result, Err(TransactionError::Write(WriteError::Moved { .. }))));
            assert_eq!(copy.current_bytes(), second);
        } else {
            let report = result.unwrap();
            assert!(report.retried_after_move);
            assert_eq!(report.reminted, vec![(given, RefNo::new(owner.word0, max + 100))]);
            let mut reader = ReadOnlyEngine::open(&copy.path).unwrap();
            assert_eq!(reader.find_by_name("/ALLOC-RETRY-A").unwrap().unwrap(), occupied);
        }
    }
}
