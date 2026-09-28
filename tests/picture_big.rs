//! GW 计划 S1–S2 的大文件门（`docs/plans/2026-09-24-gw-picture-read-plan.md` §4）：
//! `/APS-0001/1` 的图片 `apspic\09\M2013286677-265-10000-11`（14 MB）。
//!
//! 期望值来自 `gwdump.py` 与读法 3 第八轮：4 715 段、351 819 图元。这里的
//! 目录回放只读 14 MB 并走一遍 4 715 条目录，不需要 PADD 库，所以不 `#[ignore]`；
//! 要对 PADD 库读 VSCA / XYPS 的那条留到 S5 的 `locate`。

mod common;

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
    assert_eq!(picture.entries.len(), 4715);
    assert_eq!(picture.dir_len, 1 + 4715 * 7);
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
