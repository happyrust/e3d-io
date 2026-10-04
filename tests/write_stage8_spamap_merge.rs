mod common;
#[path = "common/spatial_write.rs"]
mod spatial_write;
use e3d_io::write::{Transaction, TransactionError, WriteError};
use proptest::prelude::*;
use spatial_write::*;

proptest! {
    #![proptest_config(property_config("proptest-regressions/spamap_merge.txt"))]
    #[test]
    fn remint_preserves_the_latest_occupants_entries(a in 200u32..380, b in 200u32..380) {
        let (copy, zone) = fixture();
        let mut pa = provider(); let mut pb = provider();
        let mut a_tx = Transaction::open(&copy.path, &mut pa).unwrap();
        let mut b_tx = Transaction::open(&copy.path, &mut pb).unwrap();
        let ar = insert_box(&mut a_tx, zone, a as f64);
        let given = insert_box(&mut b_tx, zone, b as f64);
        prop_assert_eq!(ar, given);
        b_tx.set_aabb(given, [10.,20.,30.,110.,220.,330.]).unwrap();
        a_tx.commit(&request()).unwrap();
        let latest = table(&copy.path);
        let report = b_tx.commit_reporting(&request()).unwrap();
        prop_assert_eq!(report.reminted.len(), 1);
        let written = report.reminted[0].1;
        prop_assert_eq!(report.reminted[0].0, given);
        prop_assert_ne!(written, given);
        prop_assert_eq!(table(&copy.path).into_iter().filter(|e| e.refno()!=written).collect::<Vec<_>>(), latest);
        prop_assert_eq!(entries(&copy.path, written)[0].value, [100,200,300,1100,2200,3300,0]);
        prop_assert_eq!(report.spatial.elements[&written].given_as, Some(given));
    }
}

#[test]
fn moved_replans_once_from_original_inputs_and_a_second_move_is_returned() {
    let (copy, zone) = fixture();
    let before = copy.current_bytes();
    let mut pa = provider();
    let mut a = Transaction::open(&copy.path, &mut pa).unwrap();
    let ar = insert_box(&mut a, zone, 250.);
    a.commit(&request()).unwrap();
    let after_a = copy.current_bytes();
    std::fs::write(&copy.path, &before).unwrap();
    let mut pb = provider();
    let mut b = Transaction::open(&copy.path, &mut pb).unwrap();
    let given = insert_box(&mut b, zone, 301.);
    let mut calls = 0;
    let report = b
        .commit_observed(&request(), |_| {
            calls += 1;
            if calls == 1 {
                std::fs::write(&copy.path, &after_a).unwrap();
            }
        })
        .unwrap();
    assert_eq!(calls, 2);
    assert!(report.retried_after_move);
    assert_eq!(report.reminted[0].0, given);
    assert_eq!(entries(&copy.path, ar)[0].value[0], 8750);
    assert_eq!(entries(&copy.path, report.reminted[0].1)[0].value[0], 8495);

    let (copy, zone) = fixture();
    let base = copy.current_bytes();
    let mut pa = provider();
    let mut a = Transaction::open(&copy.path, &mut pa).unwrap();
    insert_box(&mut a, zone, 250.);
    a.commit(&request()).unwrap();
    let bytes_a = copy.current_bytes();
    let mut c = Transaction::open(&copy.path, &mut pa).unwrap();
    insert_box(&mut c, zone, 260.);
    c.commit(&request()).unwrap();
    let bytes_c = copy.current_bytes();
    std::fs::write(&copy.path, &base).unwrap();
    let mut pb = provider();
    let mut b = Transaction::open(&copy.path, &mut pb).unwrap();
    insert_box(&mut b, zone, 301.);
    let mut calls = 0;
    let result = b.commit_observed(&request(), |_| {
        calls += 1;
        std::fs::write(&copy.path, if calls == 1 { &bytes_a } else { &bytes_c }).unwrap();
    });
    assert_eq!(calls, 2);
    assert!(matches!(
        result,
        Err(TransactionError::Write(WriteError::Moved { .. }))
    ));
}

#[test]
fn a_given_box_is_invalidated_by_a_committed_ancestor_pose_change() {
    let (copy, zone) = fixture();
    let mut pa = provider();
    let mut pb = provider();
    let mut setup = Transaction::open(&copy.path, &mut pa).unwrap();
    let r = insert_box(&mut setup, zone, 301.);
    setup.commit(&request()).unwrap();
    let mut b = Transaction::open(&copy.path, &mut pb).unwrap();
    b.set(r, "XLEN", "302").unwrap();
    b.set_aabb(r, [0., 0., 0., 1., 1., 1.]).unwrap();
    let mut a = Transaction::open(&copy.path, &mut pa).unwrap();
    a.set(zone, "POS", "100 0 0").unwrap();
    a.commit(&request()).unwrap();
    let report = b.commit_reporting(&request()).unwrap();
    assert_eq!(
        report.spatial.elements[&r].aabb,
        e3d_io::spatial::AabbSource::BuiltIn
    );
    assert_eq!(entries(&copy.path, r)[0].value[0], 9490);
}

#[test]
fn a_deleted_ancestor_is_reported_and_latest_descendants_are_recomputed() {
    let (copy, zone) = fixture();
    let mut pa = provider();
    let mut pb = provider();
    let mut setup = Transaction::open(&copy.path, &mut pa).unwrap();
    let r = insert_box(&mut setup, zone, 301.);
    setup.commit(&request()).unwrap();
    let mut b = Transaction::open(&copy.path, &mut pb).unwrap();
    b.set(r, "XLEN", "302").unwrap();
    let mut a = Transaction::open(&copy.path, &mut pa).unwrap();
    a.delete(zone).unwrap();
    a.commit(&request()).unwrap();
    let report = b.commit_reporting(&request()).unwrap();
    assert!(entries(&copy.path, r).is_empty());
    assert_eq!(report.spatial.unreachable, vec![(r, zone)]);

    let (copy, zone) = fixture();
    let mut pa = provider();
    let mut pb = provider();
    let mut b = Transaction::open(&copy.path, &mut pb).unwrap();
    b.set(zone, "POS", "100 0 0").unwrap();
    let mut a = Transaction::open(&copy.path, &mut pa).unwrap();
    let r = insert_box(&mut a, zone, 301.);
    a.commit(&request()).unwrap();
    let report = b.commit_reporting(&request()).unwrap();
    assert_eq!(entries(&copy.path, r)[0].value[0], 9495);
    assert!(report.spatial.elements.contains_key(&r));
}

#[test]
fn invalidated_non_builtin_boxes_refuse_or_skip_with_the_reason() {
    for policy in [
        e3d_io::spatial::SpatialPolicy::Require,
        e3d_io::spatial::SpatialPolicy::Skip,
    ] {
        let (copy, zone) = fixture();
        let mut pa = provider();
        let mut pb = provider();
        let mut setup = Transaction::open(&copy.path, &mut pa).unwrap();
        let r = setup
            .insert(zone, e3d_attlib::db1_hash("PYRA"), None, |_, _| Ok(()))
            .unwrap();
        setup.set_aabb(r, [0., 0., 0., 10., 20., 30.]).unwrap();
        setup.commit(&request()).unwrap();
        let mut b = Transaction::open(&copy.path, &mut pb)
            .unwrap()
            .with_spatial(policy);
        b.set(r, "HEIG", "200").unwrap();
        b.set_aabb(r, [0., 0., 0., 10., 20., 30.]).unwrap();
        let mut a = Transaction::open(&copy.path, &mut pa)
            .unwrap()
            .with_spatial(e3d_io::spatial::SpatialPolicy::Off);
        a.set(zone, "POS", "100 0 0").unwrap();
        a.commit(&request()).unwrap();
        let before = copy.current_bytes();
        match b.commit_reporting(&request()) {
            Err(TransactionError::Spatial(e3d_io::spatial::SpatialError::MissingAabb(missing)))
                if policy == e3d_io::spatial::SpatialPolicy::Require =>
            {
                assert!(missing[0]
                    .reasons
                    .contains(&e3d_io::spatial::MissingReason::Invalidated));
                assert_eq!(copy.current_bytes(), before);
            }
            Ok(report) if policy == e3d_io::spatial::SpatialPolicy::Skip => {
                assert!(report.spatial.skipped[0]
                    .reasons
                    .contains(&e3d_io::spatial::MissingReason::Invalidated));
                assert!(entries(&copy.path, r).is_empty());
            }
            other => panic!("unexpected invalidated-given result {other:?}"),
        }
    }
}
