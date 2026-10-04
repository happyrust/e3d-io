mod common;
#[path = "common/spatial_write.rs"]
mod spatial_write;
use e3d_io::write::Transaction;
use spatial_write::*;

#[test]
fn requirement_6_17_box_registers_its_exact_world_bounds() {
    let (copy, zone) = fixture();
    let mut templates = provider();
    let mut tx = Transaction::open(&copy.path, &mut templates).unwrap();
    let r = insert_box(&mut tx, zone, 301.0);
    let report = tx.commit_reporting(&request()).unwrap();
    let entries = entries(&copy.path, r);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        &entries[0].value[..6],
        &[8495, 18000, 27500, 11505, 22000, 32500]
    );
    assert_eq!(entries[0].key, [300008495, r.word0, r.word1, 0]);
    assert_eq!(report.spatial.elements[&r].registered, entries);
    assert!(!report.spatial.pages.is_empty());
}

#[test]
fn the_first_entries_create_a_readable_spamap_table() {
    let (copy, zone) = fixture();
    remove_spamap(&copy.path);
    assert!(table(&copy.path).is_empty());
    let mut p = provider();
    let mut tx = Transaction::open(&copy.path, &mut p).unwrap();
    let r = insert_box(&mut tx, zone, 301.);
    let report = tx.commit_reporting(&request()).unwrap();
    assert!(matches!(
        report.spatial.table,
        e3d_io::spatial::SpamapTableChange::Created { .. }
    ));
    assert_eq!(entries(&copy.path, r).len(), 1);
}
