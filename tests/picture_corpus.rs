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
use e3d_io::picture::{ArcStats, PictureFile};

fn kinds(report: &CorpusReport) -> Vec<(u8, usize)> {
    report.kinds.iter().map(|(&k, &n)| (k, n)).collect()
}

/// S4：一个文件里所有 conic 图元的弧几何核对。
fn arc_stats(file: &PictureFile) -> ArcStats {
    let mut stats = ArcStats::default();
    for picture in file.pictures() {
        for segment in picture.segments.iter().flatten() {
            stats.add(&ArcStats::of(segment.prims().map(|(_, p)| p)));
        }
    }
    stats
}

/// S4：整棵 `pic\` 树的弧几何核对（`gw\verify_conic.py` 的计数）。
fn corpus_arc_stats(root: &str) -> ArcStats {
    let mut stats = ArcStats::default();
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(file) = PictureFile::open(&path) {
                stats.add(&arc_stats(&file));
            }
        }
    }
    stats
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

/// S4：小样本 28 条 CONIC 的每一段弧、大样本 2 465 条的每一段，M 都在弦的垂直
/// 平分线上且 |M − mid| / 半弦 = √w。
#[test]
fn the_plant_samples_arcs_agree_with_their_chords() {
    if common::missing(&[common::PLANT_PIC_SMALL, common::PLANT_PIC_BIG]) {
        return;
    }
    let small = arc_stats(&PictureFile::open(common::PLANT_PIC_SMALL).unwrap());
    assert_eq!(
        small.spans, 51,
        "28 CONIC primitives: full circles and single arcs"
    );
    assert_eq!(small.straight, 0);
    assert_eq!(small.degenerate, 0);
    assert!(small.curved_all_consistent(), "{small:?}");
    let big = arc_stats(&PictureFile::open(common::PLANT_PIC_BIG).unwrap());
    assert!(big.spans > 2_465, "{big:?}");
    assert_eq!(big.straight, 0);
    assert!(big.curved_all_consistent(), "{big:?}");
}

/// S4 的门：Plant 58 629 条弧零例外（第二轮 `verify_conic.py` 的数字）；Marine 447 819
/// 条里 w > 0 的零例外，w = 0 的直段 317 939 条（`verify_conic.py` 按 6 位小数归并的
/// 直方图记 317 959，多出的 20 条是 0 < w < 5e-7 的极短弧）里有 161 946 条 M 不在弦上
/// ——M 对直段无意义，`Span::points` 与 gwpic 一样画成直线。
#[test]
#[ignore = "walks both corpora; run with --ignored"]
fn every_curved_span_in_both_corpora_agrees_with_its_chord() {
    if common::missing(&[common::PLANT_PIC_DIR, common::MARINE_PIC_DIR]) {
        return;
    }
    let plant = corpus_arc_stats(common::PLANT_PIC_DIR);
    assert_eq!(plant.spans, 58_629, "{plant:?}");
    assert_eq!(plant.straight, 0);
    assert_eq!(plant.off_bisector, 0);
    assert_eq!(plant.bulge_mismatch, 0);
    assert_eq!(plant.degenerate, 0);

    let marine = corpus_arc_stats(common::MARINE_PIC_DIR);
    assert_eq!(marine.spans, 285_873 + 161_946, "{marine:?}");
    assert_eq!(marine.off_bisector, 0, "{marine:?}");
    assert_eq!(
        marine.bulge_mismatch, 0,
        "every w > 0 span is exact: {marine:?}"
    );
    assert_eq!(marine.straight, 317_939, "{marine:?}");
    assert_eq!(marine.straight_with_offset, 161_946, "{marine:?}");
    assert_eq!(marine.degenerate, 0);
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
