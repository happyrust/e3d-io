mod common;
#[path = "common/spatial_write.rs"]
mod spatial_write;
use e3d_io::spatial::{SpatialError, SpatialPolicy};
use e3d_io::write::{Transaction, TransactionError};
use proptest::prelude::*;
use spatial_write::*;

proptest! {
    #![proptest_config(property_config("proptest-regressions/spamap_refuse.txt"))]
    #[test]
    fn a_missing_box_obeys_policy_and_never_leaves_a_partial_write(mode in 0u8..3) {
        let (copy, zone)=fixture();
        let before=copy.current_bytes();
        let policy=[SpatialPolicy::Require,SpatialPolicy::Skip,SpatialPolicy::Off][mode as usize];
        let mut p=provider();
        let mut tx=Transaction::open(&copy.path,&mut p).unwrap().with_spatial(policy);
        let r=tx.insert(zone,e3d_attlib::db1_hash("PYRA"),None,|_,_| Ok(())).unwrap();
        let result=tx.stage(&request());
        if mode==0 {
            assert!(matches!(result,Err(TransactionError::Spatial(SpatialError::MissingAabb(_)))));
            prop_assert_eq!(copy.current_bytes(),before);
        } else {
            let staged=result.unwrap();
            if mode==1 { prop_assert_eq!(staged.spatial().skipped[0].refno,r); }
            staged.rollback().unwrap();
            prop_assert_eq!(copy.current_bytes(),before);
        }
    }
}

#[test]
fn out_of_range_rejects_both_policies_and_bad_tables_only_block_recomputing_batches() {
    let (copy, zone) = fixture();
    for policy in [SpatialPolicy::Require, SpatialPolicy::Skip] {
        let before = copy.current_bytes();
        let mut p = provider();
        let mut tx = Transaction::open(&copy.path, &mut p)
            .unwrap()
            .with_spatial(policy);
        let r = insert_box(&mut tx, zone, 301.);
        tx.set_aabb(r, [-1e12, 0., 0., 1e12, 10., 10.]).unwrap();
        assert!(matches!(
            tx.stage(&request()),
            Err(TransactionError::Spatial(SpatialError::OutOfRange { .. }))
        ));
        assert_eq!(copy.current_bytes(), before);
    }
    let root = e3d_io::session::DbView::open(&copy.path)
        .unwrap()
        .session()
        .table_root(e3d_io::index::SPAMAP_NAME_HASH)
        .unwrap()
        .page;
    let mut file = e3d_io::write::PageFile::open(&copy.path).unwrap();
    let mut page = file.read_page(root).unwrap();
    page[..4].copy_from_slice(&99u32.to_be_bytes());
    file.write_page(root, &page).unwrap();
    file.sync().unwrap();
    drop(file);
    for policy in [SpatialPolicy::Require, SpatialPolicy::Skip] {
        let before = copy.current_bytes();
        let mut p = provider();
        let mut tx = Transaction::open(&copy.path, &mut p)
            .unwrap()
            .with_spatial(policy);
        insert_box(&mut tx, zone, 301.);
        assert!(matches!(
            tx.stage(&request()),
            Err(TransactionError::Spatial(SpatialError::MalformedTable(_)))
        ));
        assert_eq!(copy.current_bytes(), before);
    }
    let before = copy.current_bytes();
    let mut p = provider();
    let mut tx = Transaction::open(&copy.path, &mut p).unwrap();
    tx.set(zone, "DESC", "an unrelated update").unwrap();
    tx.stage(&request()).unwrap().rollback().unwrap();
    assert_eq!(copy.current_bytes(), before);
}

#[test]
fn skip_removes_old_entries_and_undecodable_geometry_refuses_the_entire_batch() {
    let (copy, zone) = fixture();
    let mut p = provider();
    let mut tx = Transaction::open(&copy.path, &mut p).unwrap();
    let r = tx
        .insert(zone, e3d_attlib::db1_hash("PYRA"), None, |_, _| Ok(()))
        .unwrap();
    tx.set_aabb(r, [0., 0., 0., 10., 20., 30.]).unwrap();
    tx.commit(&request()).unwrap();
    assert!(!entries(&copy.path, r).is_empty());
    let mut tx = Transaction::open(&copy.path, &mut p)
        .unwrap()
        .with_spatial(SpatialPolicy::Skip);
    tx.set(r, "HEIG", "100").unwrap();
    let report = tx.commit_reporting(&request()).unwrap();
    assert!(entries(&copy.path, r).is_empty());
    assert_eq!(report.spatial.skipped[0].refno, r);
    for policy in [SpatialPolicy::Require, SpatialPolicy::Skip] {
        let before = copy.current_bytes();
        let mut tx = Transaction::open(&copy.path, &mut p)
            .unwrap()
            .with_spatial(policy);
        tx.insert(zone, e3d_attlib::db1_hash("BOX"), None, |d, t| {
            let pos = t
                .attrs
                .iter()
                .find(|a| a.hash == e3d_attlib::db1_hash("POS"))
                .unwrap();
            // POS has room for three doubles, never four.
            *d.implicit_word_mut(pos.wide.location.word_offset() as usize)
                .unwrap() = 4;
            Ok(())
        })
        .unwrap();
        assert!(matches!(
            tx.stage(&request()),
            Err(TransactionError::Spatial(
                SpatialError::UndecodableAttr { .. }
            ))
        ));
        assert_eq!(copy.current_bytes(), before);
    }
}

#[test]
fn a_full_directory_refuses_table_creation_before_any_append() {
    let (copy, zone) = fixture();
    remove_spamap(&copy.path);
    let view = e3d_io::session::DbView::open(&copy.path).unwrap();
    let page_no = view.session().page.page;
    let root = view.session().index_root;
    let mut file = e3d_io::write::PageFile::open(&copy.path).unwrap();
    let mut page = file.read_page(page_no).unwrap();
    let count = u32::from_be_bytes(page[0x31c..0x320].try_into().unwrap()) as usize;
    let room = (0x7dc - 0x320) / 12;
    for i in count..room {
        let at = 0x320 + i * 12;
        page[at..at + 4].copy_from_slice(&(0x70000000 + i as u32).to_be_bytes());
        page[at + 4..at + 8].copy_from_slice(&root.page.to_be_bytes());
        page[at + 8..at + 12].copy_from_slice(&u32::from(root.ext).to_be_bytes());
    }
    page[0x31c..0x320].copy_from_slice(&(room as u32).to_be_bytes());
    file.write_page(page_no, &page).unwrap();
    file.sync().unwrap();
    drop(file);
    let before = copy.current_bytes();
    let mut p = provider();
    let mut tx = Transaction::open(&copy.path, &mut p).unwrap();
    insert_box(&mut tx, zone, 301.);
    assert!(matches!(
        tx.stage(&request()),
        Err(TransactionError::Spatial(
            SpatialError::DirectoryFull { .. }
        ))
    ));
    assert_eq!(copy.current_bytes(), before);
}
