//! GW 计划 S4（`docs/plans/2026-09-24-gw-picture-read-plan.md` §4）：颜色表与笔。
//!
//! 365 项颜色表按 core.dll `FWSCOL` + DruidNet `ColourTableManager.Init` 的顺序重放
//! （`src/picture/colour_data.rs` 的静态数据）。期望值：第十三轮从二进制读出、
//! 被 DRAW 自存缩略图逐像素坐实的 `draft31\gw\draft_colours.tsv`（365 行：编号、
//! 名字、双精度 RGB、8 位 RGB）——逐行比对；再钉几个计划点名的值。

mod common;

use e3d_io::picture::{colour_name, colour_rgb8, pen_name, Colour, Pen, COLOUR_TABLE_SIZE};

const DRAFT_COLOURS_TSV: &str = r"D:\ida_scratch\draft31\gw\draft_colours.tsv";

#[test]
fn every_colour_table_entry_matches_the_round_13_table() {
    if common::missing(&[DRAFT_COLOURS_TSV]) {
        return;
    }
    let tsv = std::fs::read_to_string(DRAFT_COLOURS_TSV).unwrap();
    let mut lines = tsv.lines();
    assert_eq!(lines.next(), Some("colour\tname\tr\tg\tb\tR8\tG8\tB8"));
    let mut rows = 0usize;
    for line in lines {
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 8, "{line:?}");
        let n: u16 = cols[0].parse().unwrap();
        let colour = Colour::of(n).unwrap_or_else(|| panic!("colour {n} missing"));
        assert_eq!(colour.name, cols[1], "name of {n}");
        let want: Vec<f64> = cols[2..5].iter().map(|c| c.parse().unwrap()).collect();
        let (r, g, b) = colour.rgb;
        for (got, want) in [r, g, b].iter().zip(&want) {
            assert!((got - want).abs() < 1e-12, "rgb of {n}: {got} vs {want}");
        }
        let want8: Vec<u8> = cols[5..8].iter().map(|c| c.parse().unwrap()).collect();
        let (r8, g8, b8) = colour.rgb8();
        assert_eq!([r8, g8, b8].to_vec(), want8, "8-bit rgb of {n}");
        rows += 1;
    }
    assert_eq!(rows, COLOUR_TABLE_SIZE as usize);
    assert_eq!(Colour::table().len(), 365);
}

#[test]
fn the_values_the_plan_names_are_in_the_table() {
    // GW 计划 §3 `picture_style`：365 项、1 = grey (130,130,130)、7 blue、59 sandybrown
    // (188,141,0)、101 forestgreen (31,129,0)、160 steelblue (0,141,188)、271 lightgrey
    // (189,189,189)、15 black、273–295 白、366 → None。
    assert_eq!(Colour::table().len(), 365);
    assert_eq!(
        (colour_name(1), colour_rgb8(1)),
        (Some("grey"), Some((130, 130, 130)))
    );
    assert_eq!(
        (colour_name(7), colour_rgb8(7)),
        (Some("blue"), Some((0, 0, 255)))
    );
    assert_eq!(
        (colour_name(59), colour_rgb8(59)),
        (Some("sandybrown"), Some((188, 141, 0)))
    );
    assert_eq!(
        (colour_name(101), colour_rgb8(101)),
        (Some("forestgreen"), Some((31, 129, 0)))
    );
    assert_eq!(
        (colour_name(160), colour_rgb8(160)),
        (Some("steelblue"), Some((0, 141, 188)))
    );
    assert_eq!(
        (colour_name(271), colour_rgb8(271)),
        (Some("lightgrey"), Some((189, 189, 189)))
    );
    assert_eq!(
        (colour_name(15), colour_rgb8(15)),
        (Some("black"), Some((0, 0, 0)))
    );
    for n in 273..=295u16 {
        assert_eq!(colour_rgb8(n), Some((255, 255, 255)));
    }
    assert_eq!(Colour::of(366), None);
    assert_eq!(Colour::of(0), None);
    // 学科色（第十三轮）：160 结构、101 管道、109 电缆、59 设备、7 参考网格、15 体积、271 楼板。
    assert_eq!(colour_name(109), Some("green"));
    assert_eq!(Colour::of(1).unwrap().hex(), "#828282");
    assert_eq!(Colour::of(2).unwrap().hex(), "#bc0000");
}

#[test]
fn the_pens_the_plan_names_have_their_styles() {
    // 8001 / 8004 / 8000 / 8107 / 8100。
    assert_eq!(pen_name(8001), "THIN SOLID");
    assert_eq!(
        Pen::of(8001),
        Pen::Line {
            width: 1,
            pattern: 1
        }
    );
    assert_eq!(pen_name(8004), "THIN CHAIN");
    assert_eq!(pen_name(8000), "FILL");
    assert_eq!(Pen::of(8000), Pen::Fill);
    assert_eq!(pen_name(8107), "SYSTEM");
    assert_eq!(Pen::of(8107), Pen::System(8107));
    assert_eq!(pen_name(8100), "SYSTEM");
    assert_eq!(pen_name(8115), "SYSTEM");
    assert_eq!(Pen::of(0), Pen::Other(0));
}
