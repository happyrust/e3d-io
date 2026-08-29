use e3d_io::record::descriptor::AttributeExtractionStatus;

#[test]
fn descriptor_api_is_public_from_record_module() {
    assert_eq!(
        AttributeExtractionStatus::Decoded,
        AttributeExtractionStatus::Decoded
    );
}
