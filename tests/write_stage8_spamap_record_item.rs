mod common;
#[path = "common/spatial_write.rs"]
mod spatial_write;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::write::Transaction;
use proptest::prelude::*;
use spatial_write::*;

fn slots(path: &std::path::Path, r: e3d_io::RefNo) -> Vec<u32> {
    let mut engine = e3d_io::ReadOnlyEngine::open(path).unwrap();
    let mut p = provider();
    let dictionary = e3d_attlib::AttlibData::parse_file(common::ATTLIB).unwrap();
    let extraction = engine
        .extract_element_with_descriptors(r, &dictionary, &mut p)
        .unwrap();
    match extraction
        .attributes
        .iter()
        .find(|a| a.hash == e3d_io::index::SPAMAP_NAME_HASH)
        .unwrap()
        .value
        .as_ref()
        .unwrap()
    {
        DescriptorValue::RawWords(v) | DescriptorValue::WordArray(v) => v.clone(),
        DescriptorValue::IntArray(v) => v.iter().map(|v| *v as u32).collect(),
        other => panic!("unexpected SPAMAP value {other:?}"),
    }
}
proptest! {
    #![proptest_config(property_config("proptest-regressions/spamap_record_item.txt"))]
    #[test]
    fn ancestor_changes_patch_untouched_descendant_records(size in 200u32..380, displacement in 10u32..200) {
        let (copy,zone)=fixture(); let mut p=provider();
        let mut tx=Transaction::open(&copy.path,&mut p).unwrap();
        let r=insert_box(&mut tx,zone,size as f64); tx.commit(&request()).unwrap();
        prop_assert_eq!(slots(&copy.path,r)[0],entries(&copy.path,r)[0].key[0]);
        let mut tx=Transaction::open(&copy.path,&mut p).unwrap();
        tx.set(zone,"POS",&format!("{displacement} 0 0")).unwrap();
        tx.commit(&request()).unwrap();
        let new=entries(&copy.path,r);
        prop_assert_eq!(new[0].value[0],10000-size*5+displacement*10);
        prop_assert_eq!(slots(&copy.path,r)[0],new[0].key[0]);
    }
}

#[test]
fn negative_members_expand_the_owner_box_and_recompute_the_owner() {
    let (copy, zone) = fixture();
    let mut p = provider();
    let mut tx = Transaction::open(&copy.path, &mut p).unwrap();
    let r = insert_box(&mut tx, zone, 301.);
    let text = tx.text_format();
    let negative = tx
        .insert(r, e3d_attlib::db1_hash("NBOX"), None, |d, t| {
            for name in ["XLEN", "YLEN", "ZLEN"] {
                d.set_attribute(
                    t,
                    e3d_attlib::db1_hash(name),
                    &DescriptorValue::Real(100.),
                    text,
                )?;
            }
            d.set_attribute(
                t,
                e3d_attlib::db1_hash("POS"),
                &DescriptorValue::RealArray(vec![1000., 0., 0.]),
                text,
            )?;
            Ok(())
        })
        .unwrap();
    tx.commit(&request()).unwrap();
    assert_eq!(entries(&copy.path, r)[0].value[3], 20500);
    let mut tx = Transaction::open(&copy.path, &mut p).unwrap();
    tx.set(negative, "POS", "2000 0 0").unwrap();
    let report = tx.commit_reporting(&request()).unwrap();
    assert_eq!(entries(&copy.path, r)[0].value[3], 30500);
    assert!(report.spatial.elements[&r]
        .causes
        .contains(&e3d_io::spatial::RecomputeCause::Imap3Member(negative)));
    assert_eq!(slots(&copy.path, r)[0], entries(&copy.path, r)[0].key[0]);
}
