//! GW 计划 S3 的门（`docs/plans/2026-09-24-gw-picture-read-plan.md` §4）——也是
//! DRAFT 计划 P1 的门：两个样例工程全部图片文件解码再编码**逐字节 0 差异**，
//! 10 种 kind 的计数 = 读法 3 第八轮的 10 个数字。
//!
//! 期望值来自 `draft31\lists\corpus-roundtrip-*.txt`（gwdump.py `--roundtrip` 对同一
//! 两棵目录）：Plant 881 文件 / 191 374 段头 / 2 204 157 组头 / 8 640 993 图元；
//! Marine 1 885 / 1 365 582 / 2 581 787 / 16 232 654。全量要读 1.3 GB，`#[ignore]`。

mod common;

use std::collections::BTreeMap;

use e3d_io::picture::roundtrip::{roundtrip, roundtrip_dir, CorpusReport};
use e3d_io::picture::PictureFile;

fn kinds(report: &CorpusReport) -> Vec<(u8, usize)> {
    report.kinds.iter().map(|(&k, &n)| (k, n)).collect()
}

#[test]
fn the_small_sample_round_trips_exactly() {
    if common::missing(&[common::PLANT_PIC_SMALL]) {
        return;
    }
    let file = PictureFile::open(common::PLANT_PIC_SMALL).unwrap();
    let report = roundtrip(&file).unwrap();
    assert!(report.is_exact(), "{:?}", report.mismatches);
    assert_eq!(
        report.stats_line(),
        "{'seg_hdr_ok': 31, 'seg_hdr_bad': 0, 'grp_hdr_ok': 136, 'grp_hdr_bad': 0, 'prim_ok': 655, 'prim_bad': 0, 'hdr_ok': 1, 'pictab_ok': 1, 'dir_ok': 1, 'name_ok': 1}"
    );
}

#[test]
fn the_marine_text_samples_round_trip_exactly() {
    if common::missing(&[common::MARINE_PIC_TEXT, common::MARINE_PIC_TEXT2]) {
        return;
    }
    for path in [common::MARINE_PIC_TEXT, common::MARINE_PIC_TEXT2] {
        let file = PictureFile::open(path).unwrap();
        let report = roundtrip(&file).unwrap();
        assert!(report.is_exact(), "{path}: {:?}", report.mismatches);
    }
}

#[test]
fn the_big_sample_round_trips_exactly() {
    if common::missing(&[common::PLANT_PIC_BIG]) {
        return;
    }
    let file = PictureFile::open(common::PLANT_PIC_BIG).unwrap();
    let report = roundtrip(&file).unwrap();
    assert!(report.is_exact(), "{:?}", report.mismatches);
    assert_eq!((report.seg_hdr_ok, report.prim_ok), (4715, 351_819));
}

/// Plant：881 文件、版本 GW6A04 452 + GW9B2B 429，0 失败，计数 = gwdump 的 corpus 报告。
#[test]
#[ignore = "reads all 881 Plant pictures (≈150 MB); run with --ignored"]
fn every_plant_picture_round_trips_exactly() {
    if common::missing(&[common::PLANT_PIC_DIR]) {
        return;
    }
    let report = roundtrip_dir(common::PLANT_PIC_DIR, |_| {}).unwrap();
    assert_eq!(report.files, 881);
    assert_eq!(report.skipped, 0);
    assert_eq!(
        report.versions,
        BTreeMap::from([
            ("GW6A04    PC 06".to_string(), 452),
            ("GW9B2B    PC 06".to_string(), 429),
        ])
    );
    assert!(report.failures.is_empty(), "{:?}", report.lines());
    assert!(report.is_exact());
    assert_eq!(
        report.stats_line(),
        "{'seg_hdr_ok': 191374, 'seg_hdr_bad': 0, 'grp_hdr_ok': 2204157, 'grp_hdr_bad': 0, 'prim_ok': 8640993, 'prim_bad': 0, 'hdr_ok': 881, 'pictab_ok': 881, 'dir_ok': 881, 'name_ok': 881}"
    );
    assert_eq!(
        kinds(&report),
        vec![
            (2, 7_005_637),
            (3, 1_586_003),
            (4, 45_578),
            (5, 3_770),
            (6, 1),
            (7, 4)
        ]
    );
}

/// Marine：1 885 文件、六个版本，0 失败，10 种 kind 全部出现。
#[test]
#[ignore = "reads all 1 885 Marine pictures (≈1.2 GB); run with --ignored"]
fn every_marine_picture_round_trips_exactly() {
    if common::missing(&[common::MARINE_PIC_DIR]) {
        return;
    }
    let report = roundtrip_dir(common::MARINE_PIC_DIR, |_| {}).unwrap();
    assert_eq!(report.files, 1885);
    assert_eq!(report.skipped, 0);
    assert_eq!(
        report.versions,
        BTreeMap::from([
            ("GW5A17    PC 06".to_string(), 341),
            ("GW5A18    PC 06".to_string(), 37),
            ("GW5A19    PC 06".to_string(), 40),
            ("GW5A1A    PC 06".to_string(), 450),
            ("GW6A02    PC 06".to_string(), 1003),
            ("GW6A04    PC 06".to_string(), 14),
        ])
    );
    assert!(report.failures.is_empty(), "{:?}", report.lines());
    assert!(report.is_exact());
    assert_eq!(
        report.stats_line(),
        "{'seg_hdr_ok': 1365582, 'seg_hdr_bad': 0, 'grp_hdr_ok': 2581787, 'grp_hdr_bad': 0, 'prim_ok': 16232654, 'prim_bad': 0, 'hdr_ok': 1885, 'pictab_ok': 1885, 'dir_ok': 1885, 'name_ok': 1885}"
    );
    assert_eq!(
        kinds(&report),
        vec![
            (1, 23_776),
            (2, 13_332_163),
            (3, 2_649_312),
            (4, 181_364),
            (5, 748),
            (6, 5_480),
            (7, 144),
            (8, 1_960),
            (9, 176),
            (10, 37_531)
        ]
    );
}

/// 两个工程合计 = 第八轮 §0.2 的 10 个数字：2 766 文件、24 873 647 图元。
#[test]
#[ignore = "reads both corpora; run with --ignored"]
fn both_corpora_add_up_to_round_eight() {
    if common::missing(&[common::PLANT_PIC_DIR, common::MARINE_PIC_DIR]) {
        return;
    }
    let plant = roundtrip_dir(common::PLANT_PIC_DIR, |_| {}).unwrap();
    let marine = roundtrip_dir(common::MARINE_PIC_DIR, |_| {}).unwrap();
    assert_eq!(plant.files + marine.files, 2766);
    assert_eq!(plant.totals.prim_ok + marine.totals.prim_ok, 24_873_647);
    let mut total: BTreeMap<u8, usize> = plant.kinds.clone();
    for (&k, &n) in &marine.kinds {
        *total.entry(k).or_insert(0) += n;
    }
    assert_eq!(
        total,
        BTreeMap::from([
            (1, 23_776),
            (2, 20_337_800),
            (3, 4_235_315),
            (4, 226_942),
            (5, 4_518),
            (6, 5_481),
            (7, 148),
            (8, 1_960),
            (9, 176),
            (10, 37_531),
        ])
    );
}
