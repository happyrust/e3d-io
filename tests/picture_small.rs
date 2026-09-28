//! GW 计划 S1–S2（`docs/plans/2026-09-24-gw-picture-read-plan.md` §4）：物理层、
//! 头、名字块、属性表、图片表、目录回放（S1）；段头 / 用户块 / 组 / 图元解码（S2）。
//!
//! 语料：AvevaPlantSample `/APS-0001` 里最小的一张 sheet 图片
//! `apspic\30\M2013286677-4350-10000-11`（42 368 B = 331 记录）。期望值全部来自
//! 冻结的参考实现 `D:\ida_scratch\draft31\gwdump.py`（DRAFT 计划 Q8）对同一文件
//! 的输出：头 `'GW6A04    PC 06'`、名字 `'2013286677 4350 10000 0 11'`、
//! 窗口 ±210 / ±297、图片 76、目录 218 字 / 31 条，以及 31 条的 `(rec, nwords,
//! rel)` 与回放出的树；31 段 / 136 组 / 655 图元（MARKER 368 / LINE 259 / CONIC 28），
//! 并把 655 行清单与 `draft31\lists\M2013286677-4350-10000-11.{raw,page}.txt`
//! 逐行比对。缺文件按仓里惯例跳过并说明。

mod common;

use e3d_io::picture::listing;
use e3d_io::picture::{flags, Payload, PictureFile, Transform, Word};
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

/// Within the 4 decimals `gwdump.py` prints (the stored doubles are the
/// design coordinates scaled, e.g. −508.9529999999795).
fn assert_close(got: (f64, f64), want: (f64, f64)) {
    assert!(
        (got.0 - want.0).abs() < 5e-5 && (got.1 - want.1).abs() < 5e-5,
        "{got:?} is not {want:?}"
    );
}

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

/// S2：段头、用户块、组、图元——计数与 gwdump 摘要相同，几个有代表性的段逐字段核对。
#[test]
fn segments_groups_and_primitives_decode_as_gwdump_reports_them() {
    if common::missing(&[common::PLANT_PIC_SMALL]) {
        return;
    }
    let file = PictureFile::open(common::PLANT_PIC_SMALL).expect("the small sample opens");
    let picture = &file.pictures()[0];
    assert_eq!(picture.segments.iter().flatten().count(), 31);
    assert_eq!(picture.prim_count(), 655);
    let groups: usize = picture
        .segments
        .iter()
        .flatten()
        .map(|s| s.groups.len())
        .sum();
    assert_eq!(groups, 136, "group headers gwdump's roundtrip counted");
    assert_eq!(
        listing::kind_histogram(picture),
        vec![(2, 368), (3, 259), (4, 28)]
    );

    // SEG#0 SHEE：flags 0x7，一个组（id 9），图框 LINE 笔 8107 颜色 1。
    let sheet = picture.segment(0).unwrap();
    assert_eq!(sheet.flags, 0x7);
    assert_eq!(sheet.label(), "SHEE=2013286677/4350");
    assert_eq!(sheet.transform, None);
    assert_eq!(sheet.groups.len(), 1);
    assert_eq!(sheet.groups[0].id, Some(9));
    let frame = &sheet.groups[0].prims[0];
    assert_eq!((frame.kind, frame.pen, frame.colour), (3, 8107, 1));
    assert_eq!(
        frame.payload,
        Payload::Points(vec![
            (0.0, 0.0),
            (210.0, 0.0),
            (210.0, 297.0),
            (0.0, 297.0),
            (0.0, 0.0)
        ])
    );
    assert_eq!(frame.count(), 4);
    assert_eq!(frame.npts(), 5);

    // SEG#22 VIEW：平移 (60.98, 177.81)，flags 0xf；SEG#23 DESI：缩放 0.0106，flags 0x10f。
    let view = picture.segment(22).unwrap();
    assert_eq!(view.flags, 0xf);
    assert_eq!(view.label(), "VIEW=2013286677/4359");
    assert_eq!(view.transform, Some([1.0, 0.0, 0.0, 1.0, 60.98, 177.81]));
    assert!(view.groups.is_empty());
    let design = picture.segment(23).unwrap();
    assert_eq!(design.flags, 0x10f);
    assert_eq!(design.label(), "DESI");
    // VSCA 1:94.6 = 0.01057（gwdump 的 %.4f 打成 0.0106）。
    let scale = design.transform.expect("DESI carries the view scale");
    assert!((scale[0] - 0.01057).abs() < 1e-12, "{scale:?}");
    assert_eq!(scale[0], scale[3]);
    assert_eq!(&scale[1..3], &[0.0, 0.0]);
    assert_eq!(&scale[4..], &[0.0, 0.0]);
    assert_eq!(design.extra4, None, "0x10f has bit 8, not bit 4");
    assert_eq!(
        picture.path_labels(23),
        vec!["SHEE=2013286677/4350", "VIEW=2013286677/4359", "DESI"]
    );
    // 复合变换：VIEW 平移 ∘ DESI 缩放——设计 mm 的 (100, 200) 落到图纸上。
    let composed = picture.composed_transform(23);
    assert_eq!(
        composed,
        Transform([1.0, 0.0, 0.0, 1.0, 60.98, 177.81]).then_outer(&Transform(scale))
    );
    let (x, y) = composed.apply(100.0, 200.0);
    assert!((x - (60.98 + 1.057)).abs() < 1e-9, "{x}");
    assert!((y - (177.81 + 2.114)).abs() < 1e-9, "{y}");
    assert_eq!(picture.composed_transform(0), Transform::IDENTITY);

    // SEG#26 PPTS=1/0：不可见（flags 0x6），11 个成员 / 11 组 / 92 个 MARKER，
    // 组 1 = ELBO 2013286676/2398，首点 (-508.953, -2360.57) 笔 8100 颜色 1。
    let ppts = picture.segment(26).unwrap();
    assert_eq!(ppts.flags, 0x6);
    assert!(!ppts.is_visible());
    assert_eq!(ppts.label(), "PPTS=1/0");
    let user = ppts.user.as_ref().unwrap();
    assert_eq!(user.members.len(), 11);
    assert_eq!(ppts.groups.len(), 11);
    assert_eq!(ppts.prim_count(), 92);
    assert_eq!(ppts.groups[0].id, Some(1));
    let member = user.member(1).unwrap();
    assert_eq!(
        (member.noun.as_str(), member.w0, member.w1),
        ("ELBO", 2013286676, 2398)
    );
    assert_eq!(user.member(2).unwrap().noun, "TEE");
    let marker = &ppts.groups[0].prims[0];
    assert_eq!((marker.kind, marker.pen, marker.colour), (2, 8100, 1));
    match &marker.payload {
        Payload::Points(points) => {
            assert_eq!(points.len(), 1);
            assert_close(points[0], (-508.953, -2360.57));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        listing::group_label(ppts, &ppts.groups[0]),
        "grp1=ELBO 2013286676/2398"
    );

    // SEG#25 GEOM：19 成员 / 19 组 / 62 图元，含两段半圆 (w = 1) 的 CONIC。
    let geom = picture.segment(25).unwrap();
    assert_eq!(geom.label(), "GEOM");
    assert_eq!(geom.user.as_ref().unwrap().members.len(), 19);
    assert_eq!(geom.groups.len(), 19);
    assert_eq!(geom.prim_count(), 62);
    let conic = geom
        .prims()
        .map(|(_, p)| p)
        .find(|p| p.kind == 4)
        .expect("GEOM has a CONIC");
    match &conic.payload {
        Payload::Conic { start, spans } => {
            assert_close(*start, (-237.6231, -2381.8999));
            assert_eq!(spans.len(), 2);
            assert!(spans.iter().all(|s| s.w == 1.0), "two half circles");
            assert_eq!(spans[1].end, *start, "a full circle closes on its start");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(conic.count(), 2);
    assert_eq!(conic.npts(), 3);
    assert_eq!(conic.pen, 8001);
    assert_eq!(conic.colour, 101, "pipes are colour 101 forestgreen");

    // BACK / SAID / VAID 段：只有用户块，没有组；VAID 挂在 VIEW 下。
    assert_eq!(picture.segment(30).unwrap().label(), "BACK=2013286677/4350");
    assert_eq!(picture.segment(29).unwrap().label(), "SAID=2013286677/4350");
    assert_eq!(picture.segment(28).unwrap().label(), "VAID=2013286677/4359");
    assert_eq!(picture.segment(28).unwrap().groups.len(), 3);
    assert_eq!(picture.segment(28).unwrap().prim_count(), 0);
    assert!(picture
        .segments
        .iter()
        .flatten()
        .all(|s| s.flags & flags::DB_REFERENCE == 0));
}

/// S2 的门：655 行清单与 gwdump.py `--list`（局部坐标）逐行相同。
#[test]
fn the_primitive_listing_matches_gwdump_line_for_line() {
    let oracle = format!(
        r"{}\M2013286677-4350-10000-11.raw.txt",
        common::DRAFT31_LISTS
    );
    if common::missing(&[common::PLANT_PIC_SMALL, &oracle]) {
        return;
    }
    let file = PictureFile::open(common::PLANT_PIC_SMALL).expect("the small sample opens");
    common::assert_listing_matches(&file, false, &oracle);
}

/// 同一张清单经复合变换落到图纸 mm（`--page`）——与 gwdump 逐行相同。
#[test]
fn the_page_listing_matches_gwdump_line_for_line() {
    let oracle = format!(
        r"{}\M2013286677-4350-10000-11.page.txt",
        common::DRAFT31_LISTS
    );
    if common::missing(&[common::PLANT_PIC_SMALL, &oracle]) {
        return;
    }
    let file = PictureFile::open(common::PLANT_PIC_SMALL).expect("the small sample opens");
    common::assert_listing_matches(&file, true, &oracle);
}

/// 摘要的前几行与 gwdump 相同（树的每一行也比）。
#[test]
fn the_summary_prints_as_gwdump_does() {
    if common::missing(&[common::PLANT_PIC_SMALL]) {
        return;
    }
    let file = PictureFile::open(common::PLANT_PIC_SMALL).expect("the small sample opens");
    let lines = listing::summary(&file);
    assert_eq!(lines[1], "size      : 42368 bytes = 331 records of 128");
    assert_eq!(
        lines[2],
        "header    : version 'GW6A04    PC 06'  name rec 3 len 32  attr rec 5 len 132  pictab rec 14 len 8"
    );
    assert_eq!(lines[3], "name      : '2013286677 4350 10000 0 11'");
    assert_eq!(
        lines[4],
        "picture 76 : dir rec 318 (len 218 words, 31 entries)  int4=0  window=(-210,-297,210,297)"
    );
    assert_eq!(
        lines[5],
        "  segments 31  primitives 655  by kind: MARKER(2)=368, LINE(3)=259, CONIC(4)=28"
    );
    assert_eq!(lines[6], "  by class : ARC=28, LINE=259, POINT=368");
    assert_eq!(lines[7], "  -- tree (segments) --");
    assert_eq!(
        lines[8],
        "    SEG#30 rec=317 words=5 flags=0x5 BACK=2013286677/4350 xf=- groups=0 prims=0"
    );
    assert_eq!(
        lines[10],
        "    SEG#0 rec=15 words=18 flags=0x7 SHEE=2013286677/4350 xf=- groups=1 prims=1"
    );
    assert_eq!(
        lines[11],
        "      SEG#22 rec=252 words=11 flags=0xf VIEW=2013286677/4359 xf=(1,0,0,1,60.98,177.81) groups=0 prims=0"
    );
    assert_eq!(
        lines[13],
        "        SEG#23 rec=253 words=11 flags=0x10f DESI xf=(0.0106,0,0,0.0106,0,0) groups=0 prims=0"
    );
    assert_eq!(
        lines[16],
        "            SEG#26 rec=293 words=336 flags=0x6 PPTS=1/0 xf=- groups=11 prims=92  members=11"
    );
    assert_eq!(lines.len(), 8 + 31);
}
