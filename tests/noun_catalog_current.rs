use std::path::PathBuf;

use e3d_attlib::db1_hash;
use e3d_io::noun_catalog::NounCatalog;

#[test]
fn checked_in_e3d31_catalog_parses_with_exported_counts_and_order() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("catalog/e3d31");
    let catalog = NounCatalog::read(
        root.join("noun_layout.json"),
        root.join("noun_attr_fields.json"),
    )
    .unwrap();

    assert_eq!(catalog.noun_count(), 1_933);
    assert_eq!(catalog.declaration_count(), 22_049);
    assert_eq!(catalog.field_count(), 4_271);
    assert_eq!(catalog.attribute_field_value_count(), 203_454);

    let bran = catalog.noun(db1_hash("BRAN")).expect("BRAN catalog row");
    assert!(!bran.attrs.is_empty());
    assert_eq!(bran.noun, "BRANCH");
    assert_eq!(bran.short_name, "BRAN");
    assert_eq!(bran.attrs.first().unwrap().name, "NAME");

    // DBMDAT is an internal physical template rather than a public
    // DbElementType. Its descriptor remains available from desvir.dat and is
    // deliberately not fabricated in the logical catalog.
    assert!(catalog.noun(db1_hash("DBMDAT")).is_none());
}
