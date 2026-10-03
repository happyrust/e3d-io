//! `ReadState` on the SPAMAP baseline: a native BOX reads as the 2026-09-22
//! evidence describes it (`docs/evidence/2026-09-22-e3d-spamap-consequences.md`
//! §1 step 3), up its owner chain to the world.

mod common;

use common::corpus::{SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION};
use common::{ATTLIB, DESVIR};
use e3d_attlib::{db1_hash, AttlibData};
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::template::TemplateProvider;
use e3d_io::spatial::state::ReadState;
use e3d_io::spatial::SpatialState;
use e3d_io::RefNo;

const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;
const NATIVE_BOX: RefNo = RefNo { word0: 24_384, word1: 24_777 };

#[test]
fn a_native_box_reads_with_its_sizes_and_its_owner_chain() {
    if common::missing(&[SPAMAP_BASELINE, DESVIR, ATTLIB]) {
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB).expect("attlib");
    let templates = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE)).expect("desvir");
    let state = ReadState::open(SPAMAP_BASELINE, SPAMAP_BASELINE_SESSION, attlib, templates).expect("open session 640");
    assert_eq!(state.sesno(), SPAMAP_BASELINE_SESSION);

    assert_eq!(state.noun(NATIVE_BOX), Some(db1_hash("BOX")));
    for (name, mm) in [("XLEN", 310.0), ("YLEN", 590.0), ("ZLEN", 550.0)] {
        assert_eq!(state.attr(NATIVE_BOX, db1_hash(name)), Ok(Some(DescriptorValue::Real(mm))), "{name}");
    }
    assert_eq!(state.attr(NATIVE_BOX, db1_hash("DIAM")), Ok(None), "a BOX has no DIAM");

    // BOX <- EQUI <- ZONE <- SITE <- WORL, and the world has no owner.
    let mut chain = vec![NATIVE_BOX];
    while let Some(owner) = state.owner(*chain.last().unwrap()) {
        chain.push(owner);
        assert!(chain.len() < 16, "an owner chain that does not end: {chain:?}");
    }
    let nouns: Vec<u32> = chain.iter().map(|refno| state.noun(*refno).expect("each link exists")).collect();
    let expected: Vec<u32> = ["BOX", "EQUI", "ZONE", "SITE", "WORL"].iter().map(|noun| db1_hash(noun)).collect();
    assert_eq!(nouns, expected);

    assert_eq!(state.noun(RefNo::new(24_384, 2_000_000_000)), None, "no such element");
    assert!(state.failures().is_empty(), "{:?}", state.failures());
}
