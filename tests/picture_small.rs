//! GW 计划 S1（`docs/plans/2026-09-24-gw-picture-read-plan.md` §4）：物理层、
//! 头、名字块、属性表、图片表、目录回放。
//!
//! 语料：AvevaPlantSample `/APS-0001` 里最小的一张 sheet 图片
//! `apspic\30\M2013286677-4350-10000-11`（42 368 B = 331 记录）。期望值全部来自
//! 冻结的参考实现 `D:\ida_scratch\draft31\gwdump.py`（DRAFT 计划 Q8）对同一文件
//! 的输出：头 `'GW6A04    PC 06'`、名字 `'2013286677 4350 10000 0 11'`、
//! 窗口 ±210 / ±297、图片 76、目录 218 字 / 31 条，以及 31 条的 `(rec, nwords,
//! rel)` 与回放出的树。缺文件按仓里惯例跳过并说明。

mod common;

use e3d_io::picture::{PictureFile, Word};
use e3d_io::RefNo;

/// `gwdump.py` printed `(rec, rel | levels << 4, nwords)` for the 31 entries
/// (all `[7, 0, 0, rec, rel, 3, nwords]`).
const ENTRIES: [(i32, i32, i32); 31] = [
    (15, 6, 18),
    (17, 6, 11),
    (18, 6, 11),
    (19, 6, 5),
    (20, 6, 1191),
    (95, 3, 336),
    (116, 3, 6),
    (117, 35, 8),
    (118, 19, 11),
    (119, 6, 11),
    (120, 6, 5),
    (121, 6, 579),
    (158, 3, 336),
    (179, 3, 6),
    (180, 35, 8),
    (181, 19, 11),
    (182, 6, 11),
    (183, 6, 5),
    (184, 6, 710),
    (229, 3, 336),
    (250, 3, 6),
    (251, 35, 8),
    (252, 19, 11),
    (253, 6, 11),
    (254, 6, 5),
    (255, 6, 593),
    (293, 3, 336),
    (314, 3, 6),
    (315, 35, 8),
    (316, 35, 8),
    (317, 3, 5),
];

/// The pre-order walk `gwdump.py` prints: BACK, SAID, then SHEE with its four
/// VIEWs, each VIEW holding VAID then DESI > PSPOOL > {PLIN, PPTS, GEOM}.
const WALK: [usize; 31] = [
    30, 29, 0, 22, 28, 23, 24, 27, 26, 25, 15, 21, 16, 17, 20, 19, 18, 8, 14, 9, 10, 13, 12, 11, 1,
    7, 2, 3, 6, 5, 4,
];

#[test]
fn the_header_name_and_tables_read_as_gwdump_prints_them() {
    if common::missing(&[common::PLANT_PIC_SMALL]) {
        return;
    }
    let file = PictureFile::open(common::PLANT_PIC_SMALL).expect("the small sample opens");
    assert_eq!(file.bytes().len(), 42_368);
    assert_eq!(file.records(), 331);

    let header = file.header();
    assert_eq!(header.version, "GW6A04    PC 06");
    assert_eq!((header.name_rec, header.name_len), (3, 32));
    assert_eq!((header.attr_rec, header.attr_len), (5, 132));
    assert_eq!((header.pic_rec, header.pic_len), (14, 8));

    let name = file.name();
    assert_eq!(name.text, "2013286677 4350 10000 0 11");
    assert_eq!(name.owner, Some(RefNo::new(2013286677, 4350)));
    assert_eq!(name.exfi, Some(10000));
    assert_eq!(name.pvno, Some(11));

    // 属性表 33 条 × 4 字，与图形无关，原字保留；首条 [11, 890111901, 1072693248, 0]。
    let attrs = file.attrs();
    assert_eq!(attrs.len(), 132);
    let first: Vec<i32> = attrs[..4].iter().map(|w| w.as_int()).collect();
    assert_eq!(first, [11, 890_111_901, 1_072_693_248, 0]);
    assert_eq!(attrs[124].as_int(), 1, "the table ends with ids 1 and -1");
    assert_eq!(attrs[128].as_int(), -1);
    assert_eq!(attrs[129], Word::int(-890_111_901));

    // 头与名字块编回去就是记录 1 / 记录 3 的字节。
    let head = file.words_at(1).unwrap().raw(1, 21).unwrap();
    assert_eq!(e3d_io::picture::word::to_bytes(&header.encode()), head);
    let name_bytes = file.words_at(3).unwrap().raw(1, 32).unwrap();
    assert_eq!(e3d_io::picture::word::to_bytes(&name.encode()), name_bytes);

    // 331 记录里最后一条被用到的是段 #30（记录 317，5 字）之后的目录（318 起 218 字 = 14 记录）。
    assert_eq!(file.last_used_record(), 331);
}

#[test]
fn the_picture_table_and_directory_replay_into_gwdumps_tree() {
    if common::missing(&[common::PLANT_PIC_SMALL]) {
        return;
    }
    let file = PictureFile::open(common::PLANT_PIC_SMALL).expect("the small sample opens");
    assert_eq!(
        file.pictures().len(),
        1,
        "every sample file holds one picture"
    );
    let picture = &file.pictures()[0];
    assert_eq!(picture.number, 76);
    assert_eq!(picture.dir_rec, 318);
    assert_eq!(picture.int4, 0);
    assert_eq!(picture.window, [-210.0, -297.0, 210.0, 297.0]);
    assert_eq!(picture.dir_len, 218, "1 + 31 × 7");
    assert_eq!(picture.entries.len(), 31);

    for (index, (entry, &(rec, rel, nwords))) in
        picture.entries.iter().zip(ENTRIES.iter()).enumerate()
    {
        assert_eq!(entry.words[0], 7, "entry {index} starts with its length");
        assert_eq!(
            entry.reference(),
            (0, 0),
            "Plant entries carry no DB reference"
        );
        assert_eq!(entry.rec(), rec, "entry {index} record");
        assert_eq!(entry.rel_word(), rel, "entry {index} rel word");
        assert_eq!(entry.elem_type(), 3, "entry {index} is a segment");
        assert_eq!(entry.nwords(), nwords, "entry {index} words");
    }
    // 35 = rel 3 | 2 levels << 4；19 = rel 3 | 1 level。
    assert_eq!(
        (picture.entries[7].relation(), picture.entries[7].levels()),
        (3, 2)
    );
    assert_eq!(
        (picture.entries[8].relation(), picture.entries[8].levels()),
        (3, 1)
    );

    // 树：根下 [BACK#30, SAID#29, SHEE#0]，SHEE 下四个 VIEW，VIEW 下 [VAID, DESI]，
    // DESI 下 PSPOOL，PSPOOL 下 [PLIN, PPTS, GEOM]。
    assert_eq!(picture.roots(), &[30, 29, 0]);
    assert_eq!(picture.node(0).children, vec![22, 15, 8, 1]);
    assert_eq!(picture.node(22).children, vec![28, 23]);
    assert_eq!(picture.node(23).children, vec![24]);
    assert_eq!(picture.node(24).children, vec![27, 26, 25]);
    assert_eq!(picture.node(1).children, vec![7, 2]);
    assert_eq!(picture.node(3).children, vec![6, 5, 4]);
    assert_eq!(picture.node(30).parent, None);
    assert_eq!(picture.node(28).parent, Some(22));
    assert_eq!(picture.node(4).depth, 5);
    assert_eq!(picture.node(0).depth, 1);
    assert_eq!(picture.ancestry(25), vec![0, 22, 23, 24, 25]);

    let walk: Vec<usize> = picture.walk().map(|(index, _)| index).collect();
    assert_eq!(walk, WALK.to_vec());
    let depths: Vec<usize> = picture.walk().map(|(_, depth)| depth).collect();
    assert_eq!(&depths[..10], &[1, 1, 1, 2, 3, 3, 4, 5, 5, 5]);
}
