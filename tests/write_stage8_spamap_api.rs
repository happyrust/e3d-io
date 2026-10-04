mod common;
#[path = "common/spatial_write.rs"]
mod spatial_write;
use e3d_io::spatial::SpatialError;
use e3d_io::write::{Transaction, TransactionError};
use proptest::prelude::*;
use spatial_write::*;
proptest! {
    #![proptest_config(property_config("proptest-regressions/spamap_api.txt"))]
    #[test]
    fn the_last_valid_box_survives_a_rejected_replacement(x in -1000i32..1000) {
        let (copy, zone) = fixture();
        let mut p = provider();
        let mut tx = Transaction::open(&copy.path, &mut p).unwrap();
        let r = insert_box(&mut tx, zone, 301.);
        tx.set_aabb(r, [0.,0.,0.,1.,1.,1.]).unwrap();
        tx.set_aabb(r, [x as f64,0.,0.,x as f64+10.,20.,30.]).unwrap();
        prop_assert!(matches!(tx.set_aabb(r,[f64::NAN,0.,0.,1.,1.,1.]), Err(TransactionError::Spatial(SpatialError::InvalidAabb { .. }))), "invalid AABB rejected");
        prop_assert!(tx.set_aabb(r,[10.,0.,0.,0.,1.,1.]).is_err());
        prop_assert!(matches!(tx.set_aabb(zone,[0.,0.,0.,1.,1.,1.]), Err(TransactionError::Spatial(SpatialError::NotGeometric { .. }))), "non-geometric rejected");
        tx.commit(&request()).unwrap();
        prop_assert_eq!(entries(&copy.path,r)[0].value[0] as i32, x*10);
    }
}
