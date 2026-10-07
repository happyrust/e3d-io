//! GW 计划 S1–S2 的大文件门（`docs/plans/2026-09-24-gw-picture-read-plan.md` §4）：
//! `/APS-0001/1` 的图片 `apspic\09\M2013286677-265-10000-11`（14 MB）。
//!
//! 期望值来自 `gwdump.py` 与读法 3 第八轮：4 715 段、351 819 图元（MARKER 300 211 /
//! LINE 48 915 / CONIC 2 465 / FILL 228）。打开一次约 0.1 s，不需要 PADD 库，所以不
//! `#[ignore]`；与 76 MB 的 `--page` 清单逐行比对的那条 `#[ignore]`（几秒）；要对
//! PADD 库读 VSCA / XYPS 的那条留到 S5 的 `locate`。

mod common;

use e3d_io::picture::listing;
use e3d_io::picture::PictureFile;
use e3d_io::RefNo;

#[test]
fn the_big_sample_directory_replays_into_4715_segments() {
    if common::missing(&[common::PLANT_PIC_BIG]) {
        return;
    }
    let file = PictureFile::open(common::PLANT_PIC_BIG).expect("the big sample opens");
    assert_eq!(file.header().version, "GW6A04    PC 06");
    assert_eq!(file.name().owner, Some(RefNo::new(2013286677, 265)));
    assert_eq!(file.name().exfi, Some(10000));
    assert_eq!(file.name().pvno, Some(11));
    assert_eq!(file.pictures().len(), 1);
    let picture = &file.pictures()[0];
    assert_eq!(picture.number, 89);
    assert_eq!(picture.dir_rec, 108_247);
    assert_eq!(picture.window, [-1189.0, -841.0, 1189.0, 841.0], "A0");
    assert_eq!(picture.entries.len(), 4715);
    assert_eq!(picture.dir_len, 1 + 4715 * 7);
    // S2：kind 直方图 = 第八轮。
    assert_eq!(picture.prim_count(), 351_819);
    assert_eq!(
        listing::kind_histogram(picture),
        vec![(2, 300_211), (3, 48_915), (4, 2_465), (5, 228)]
    );
    // GEOM 段的组 id k = 第 k 个成员（2 350 段零例外）：每个有成员表的段，
    // 每个组的 id 都落在成员表里。
    let mut geom_segments = 0usize;
    for segment in picture.segments.iter().flatten() {
        let Some(user) = &segment.user else { continue };
        if user.role.is("GEOM") {
            geom_segments += 1;
            for group in &segment.groups {
                let id = group.id.expect("GEOM groups have ids");
                assert!(
                    user.member(id).is_some(),
                    "GEOM group {id} has no member entry ({} members)",
                    user.members.len()
                );
            }
        }
    }
    assert!(geom_segments > 0, "the big sample has GEOM segments");
    assert!(
        picture.entries.iter().all(|e| e.elem_type() == 3),
        "every directory entry is a segment"
    );
    assert_eq!(
        picture.walk().count(),
        4715,
        "the walk visits every entry once"
    );
    // Every entry's data lies inside the file.
    let records = file.records() as i32;
    for entry in &picture.entries {
        let last = entry.rec() + (entry.nwords() * 8 - 1) / 128;
        assert!(
            last <= records,
            "entry at record {} runs past the file",
            entry.rec()
        );
    }
    assert!(file.last_used_record() <= records);
}

/// 351 819 行图纸 mm 清单与 gwdump.py `--list --page` 逐行相同（读 76 MB 的 oracle）。
#[test]
#[ignore = "reads the 76 MB oracle listing; run with --ignored"]
fn the_big_page_listing_matches_gwdump_line_for_line() {
    let oracle = format!(
        r"{}\M2013286677-265-10000-11.page.txt",
        common::DRAFT31_LISTS
    );
    if common::missing(&[common::PLANT_PIC_BIG, &oracle]) {
        return;
    }
    let file = PictureFile::open(common::PLANT_PIC_BIG).expect("the big sample opens");
    common::assert_listing_matches(&file, true, &oracle);
}
