#![allow(dead_code)]
use crate::common::write_fixture::{DbCopy, AMS8000};
use e3d_attlib::db1_hash;
use e3d_io::engine::ScanTier;
use e3d_io::index::{read_spamap, SpamapEntry, SpamapRead};
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::template::TemplateProvider;
use e3d_io::session::{DbView, SessionDate};
use e3d_io::spatial::SpatialPolicy;
use e3d_io::write::{SessionClock, SessionRequest, Transaction};
use e3d_io::{ReadOnlyEngine, RefNo};
use std::path::Path;

pub fn provider() -> TemplateProvider {
    TemplateProvider::open(crate::common::DESVIR, Some(0x000B_0692)).expect("schema is required")
}
pub fn request() -> SessionRequest<'static> {
    let date = SessionDate {
        year: 2026,
        month: 10,
        day: 4,
        hour: 8,
        minute: 0,
        second: 0,
        millisecond: 0,
    };
    SessionRequest {
        user: "e3dio",
        comment: "spatial model",
        clock: Some(SessionClock::fixed(date, date)),
    }
}
pub fn fixture() -> (DbCopy, RefNo) {
    let copy = DbCopy::of(AMS8000);
    let mut engine = ReadOnlyEngine::open(&copy.path).unwrap();
    let world = engine
        .scan_elements(ScanTier::Header)
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.noun_hash == db1_hash("WORL"))
        .unwrap()
        .refno;
    let mut templates = provider();
    let mut tx = Transaction::open(&copy.path, &mut templates)
        .unwrap()
        .with_spatial(SpatialPolicy::Off);
    let zone = tx
        .insert(world, db1_hash("ZONE"), None, |_, _| Ok(()))
        .unwrap();
    tx.commit(&request()).unwrap();
    (copy, zone)
}
pub fn insert_box(tx: &mut Transaction<'_>, owner: RefNo, size: f64) -> RefNo {
    let text = tx.text_format();
    tx.insert(owner, db1_hash("BOX"), None, |draft, template| {
        for (name, value) in [("XLEN", size), ("YLEN", 400.0), ("ZLEN", 500.0)] {
            draft
                .set_attribute(
                    template,
                    db1_hash(name),
                    &DescriptorValue::Real(value),
                    text,
                )
                .map_err(e3d_io::write::TransactionError::from)?;
        }
        draft
            .set_attribute(
                template,
                db1_hash("POS"),
                &DescriptorValue::RealArray(vec![1000.0, 2000.0, 3000.0]),
                text,
            )
            .map_err(e3d_io::write::TransactionError::from)
    })
    .unwrap()
}
pub fn table(path: &Path) -> Vec<SpamapEntry> {
    match read_spamap(&mut DbView::open(path).unwrap()).unwrap() {
        SpamapRead::NoTable { .. } => vec![],
        SpamapRead::Table { table, .. } => table.entries().to_vec(),
    }
}
pub fn entries(path: &Path, r: RefNo) -> Vec<SpamapEntry> {
    table(path).into_iter().filter(|e| e.refno() == r).collect()
}

pub fn remove_spamap(path: &Path) {
    let view = DbView::open(path).unwrap();
    let mut file = e3d_io::write::PageFile::open(path).unwrap();
    let page_no = view.session().page.page;
    let mut bytes = file.read_page(page_no).unwrap();
    let count = u32::from_be_bytes(bytes[0x31c..0x320].try_into().unwrap()) as usize;
    let slot = (0..count).find(|i| {
        let at = 0x320 + i * 12;
        u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) == e3d_io::index::SPAMAP_NAME_HASH
    });
    if let Some(slot) = slot {
        bytes.copy_within(
            0x320 + (slot + 1) * 12..0x320 + count * 12,
            0x320 + slot * 12,
        );
        bytes[0x31c..0x320].copy_from_slice(&((count - 1) as u32).to_be_bytes());
        file.write_page(page_no, &bytes).unwrap();
        file.sync().unwrap();
    }
}

pub fn property_config(seed_path: &'static str) -> proptest::test_runner::Config {
    proptest::test_runner::Config {
        cases: 16,
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct(seed_path),
        )),
        ..Default::default()
    }
}
