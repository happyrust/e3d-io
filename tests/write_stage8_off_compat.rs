mod common;
#[path = "common/spatial_write.rs"]
mod spatial_write;
use e3d_io::spatial::SpatialPolicy;
use e3d_io::write::{
    stage_element_session_with, ElementWrite, RecordDraft, Transaction, WriteOptions,
};
use spatial_write::*;

#[test]
fn off_keeps_the_primitive_bytes_and_existing_spamap_root() {
    let (copy, zone) = fixture();
    let other = tempfile::tempdir().unwrap();
    let path = other.path().join("db");
    std::fs::write(&path, copy.current_bytes()).unwrap();
    let old_table = table(&copy.path);
    let mut p = provider();
    let mut tx = Transaction::open(&copy.path, &mut p)
        .unwrap()
        .with_spatial(SpatialPolicy::Off);
    tx.set(zone, "DESC", "off compatibility").unwrap();
    let report = tx.commit_reporting(&request()).unwrap();
    let mut engine = e3d_io::ReadOnlyEngine::open(&path).unwrap();
    let mut draft =
        RecordDraft::from_record_bytes(&engine.find_element(zone).unwrap().unwrap().raw_bytes)
            .unwrap();
    let text = engine.text_format();
    let template = p.template_for(draft.noun_hash).unwrap().unwrap();
    draft
        .set_attribute(
            template,
            e3d_attlib::db1_hash("DESC"),
            &e3d_io::record::descriptor::DescriptorValue::Text("off compatibility".into()),
            text,
        )
        .unwrap();
    stage_element_session_with(
        &path,
        &request(),
        &[ElementWrite::Update(draft)],
        &[],
        WriteOptions {
            templates: Some(&mut p),
        },
    )
    .unwrap()
    .commit()
    .unwrap();
    assert_eq!(copy.current_bytes(), std::fs::read(path).unwrap());
    assert_eq!(table(&copy.path), old_table);
    assert!(report.spatial.pages.is_empty());
    // The original 17 fixed-clock golden hashes are exercised by the adapted
    // write_stage6 targets in the required full-suite check; never re-recorded.
}
