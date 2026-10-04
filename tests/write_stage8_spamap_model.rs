mod common;
#[path = "common/spatial_write.rs"]
mod spatial_write;
use e3d_io::write::Transaction;
use proptest::prelude::*;
use spatial_write::*;

proptest! {
    #![proptest_config(property_config("proptest-regressions/spamap_model.txt"))]
    #[test]
    fn final_geometry_matches_an_independent_box_model(size in 200u32..380, middle in 150u32..390) {
        let (copy, zone) = fixture();
        let original_table = table(&copy.path);
        let before = copy.current_bytes();
        let mut templates = provider();
        let mut tx = Transaction::open(&copy.path, &mut templates).unwrap();
        let r = insert_box(&mut tx, zone, middle as f64);
        tx.set(r, "XLEN", &size.to_string()).unwrap();
        let staged = tx.stage(&request()).unwrap();
        let dry_report = staged.spatial().clone();
        prop_assert_eq!(table(&copy.path), original_table.clone());
        staged.rollback().unwrap();
        prop_assert_eq!(copy.current_bytes(), before);
        let mut templates = provider();
        let mut tx = Transaction::open(&copy.path, &mut templates).unwrap();
        let r2 = insert_box(&mut tx, zone, size as f64);
        prop_assert_eq!(r, r2);
        let report = tx.commit_observed(&request(), |_| assert_eq!(table(&copy.path), original_table)).unwrap();
        prop_assert_eq!(report.spatial, dry_report);
        let actual = entries(&copy.path, r);
        prop_assert_eq!(actual.len(), 1);
        let x = size * 5;
        prop_assert_eq!(actual[0].value, [10000-x, 18000, 27500, 10000+x, 22000, 32500, 0]);
        prop_assert_eq!(actual[0].key, [300000000+10000-x, r.word0, r.word1, 0]);
        prop_assert_eq!(table(&copy.path).into_iter().filter(|e| e.refno()!=r).collect::<Vec<_>>(), original_table);
    }
}

#[test]
fn a_same_key_value_change_replaces_the_old_value_and_a_reverted_edit_is_empty() {
    let (copy, zone) = fixture();
    let mut templates = provider();
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap();
    let r = insert_box(&mut tx, zone, 301.0);
    tx.commit(&request()).unwrap();
    let old = entries(&copy.path, r);
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap();
    tx.set(r, "YLEN", "420").unwrap();
    let report = tx.commit_reporting(&request()).unwrap();
    let new = entries(&copy.path, r);
    assert_eq!(old[0].key, new[0].key);
    assert_ne!(old[0].value, new[0].value);
    assert_eq!(report.spatial.elements[&r].removed, old);
    assert_eq!(report.spatial.elements[&r].registered, new);
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap();
    tx.set(r, "YLEN", "430").unwrap();
    tx.set(r, "YLEN", "420").unwrap();
    tx.set_aabb(r, [0., 0., 0., 1., 1., 1.]).unwrap();
    let report = tx.commit_reporting(&request()).unwrap();
    assert!(report.spatial.pages.is_empty());
    assert_eq!(report.spatial.unused_aabb, vec![r]);
    assert_eq!(entries(&copy.path, r), new);
}
