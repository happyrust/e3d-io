//! GW 计划 S2 的 Marine 门（`docs/plans/2026-09-24-gw-picture-read-plan.md` §4）：
//! Plant 图片里一条文字都没有，TEXT（kind 1）、TEXT2（kind 10）、INSTANCE、
//! BOUNDARY / BOUNDCONIC 只在 AvevaMarineSample 的背景图片里出现。
//!
//! 语料：`amspic\00\M2013287597-16864-10000-1`（TEXT 57 / LINE 2 / INSTANCE 7）与
//! `amspic\00\M2013287597-1792-10000-1`（TEXT2 78，含 `SHOP&/TEST`、
//! `Block &&&/Unit No.`、`&USEC.(Right)` 这样的多 run 源串；LINE 6 / CONIC 1 /
//! INSTANCE 7 / BOUNDARY 1 / BOUNDCONIC 1）。清单与 `draft31\lists\marine_<名>.page.txt`
//! （gwdump.py `--list --page`）逐行相同；属性表长度 0 的头也要能开。

mod common;

use e3d_io::picture::listing;
use e3d_io::picture::{Payload, PictureFile, Text2Body};
use e3d_io::RefNo;

#[test]
fn a_marine_background_with_text_and_instances_lists_as_gwdump_does() {
    let oracle = format!(
        r"{}\marine_M2013287597-16864-10000-1.page.txt",
        common::DRAFT31_LISTS
    );
    if common::missing(&[common::MARINE_PIC_TEXT, &oracle]) {
        return;
    }
    let file = PictureFile::open(common::MARINE_PIC_TEXT).expect("the Marine sample opens");
    let header = file.header();
    assert_eq!(header.version, "GW6A02    PC 06");
    assert_eq!(
        (header.attr_rec, header.attr_len),
        (0, 0),
        "no attribute table"
    );
    assert!(file.attrs().is_empty());
    assert_eq!(file.name().owner, Some(RefNo::new(2013287597, 16864)));
    let picture = &file.pictures()[0];
    assert_eq!(picture.number, 1);
    assert_eq!(picture.int4, 1);
    assert_eq!(picture.window, [-914.4, -609.6, 914.4, 609.6]);
    assert_eq!(picture.entries.len(), 21);
    assert_eq!(picture.prim_count(), 66);
    assert_eq!(
        listing::kind_histogram(picture),
        vec![(1, 57), (3, 2), (6, 7)]
    );

    // 标题栏里的 TEXT 'sjr'：FONT 11 RIGHT CENTRE HBODY，字高 3。
    let text = picture
        .segments
        .iter()
        .flatten()
        .flat_map(|s| s.prims().map(|(_, p)| p))
        .find(|p| p.kind == 1 && p.text().as_deref() == Some("sjr"))
        .expect("the TEXT 'sjr'");
    match &text.payload {
        Payload::Text(t) => {
            assert_eq!(t.text(), "sjr");
            assert_eq!(t.format().to_string(), "FONT    11,RIGHT,CENTRE,HBODY");
            assert_eq!(t.geom[3], 3.0, "up vector length = character height");
            assert_eq!(
                t.tail.len(),
                1,
                "3 chars leave one residue byte in the word"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(text.count(), 3);

    // 树序第一条图元是 BNTE=15199/27821 段里的 INSTANCE：引用别的图片里的段，带变换。
    let (first_index, _) = picture
        .walk()
        .find(|&(i, _)| picture.segment(i).is_some_and(|s| s.prim_count() > 0))
        .expect("a segment with primitives");
    let first = picture.segment(first_index).unwrap();
    assert_eq!(first.label(), "BNTE=15199/27821");
    assert_eq!(
        picture.path_labels(first_index),
        vec!["BACK=2013287597/16864", "BNTE=15199/27821"]
    );
    let instance = first.prims().next().map(|(_, p)| p).unwrap();
    assert_eq!(instance.kind, 6);
    assert_eq!(
        instance.payload,
        Payload::Instance {
            reference: (15199, 27821),
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        }
    );
    assert_eq!((instance.pen, instance.colour), (0, 0));

    common::assert_listing_matches(&file, true, &oracle);
}

#[test]
fn a_marine_background_with_text2_runs_lists_as_gwdump_does() {
    let oracle = format!(
        r"{}\marine_M2013287597-1792-10000-1.page.txt",
        common::DRAFT31_LISTS
    );
    if common::missing(&[common::MARINE_PIC_TEXT2, &oracle]) {
        return;
    }
    let file = PictureFile::open(common::MARINE_PIC_TEXT2).expect("the Marine sample opens");
    let picture = &file.pictures()[0];
    assert_eq!(picture.int4, -1);
    assert_eq!(picture.window, [-297.0, -210.0, 297.0, 210.0]);
    assert_eq!(picture.entries.len(), 84);
    assert_eq!(picture.prim_count(), 94);
    assert_eq!(
        listing::kind_histogram(picture),
        vec![(3, 6), (4, 1), (6, 7), (8, 1), (9, 1), (10, 78)]
    );

    // 每个 TEXT2 的 run 都解得开，源串重建出转义。
    let mut sources = Vec::new();
    for segment in picture.segments.iter().flatten() {
        for (_, prim) in segment.prims() {
            if let Payload::Text2(t) = &prim.payload {
                assert!(
                    matches!(t.body, Text2Body::Runs(_)),
                    "TEXT2 at word {} did not parse into runs",
                    prim.pos
                );
                sources.push(t.source().unwrap());
                assert_eq!(prim.count(), t.srclen as usize);
            }
        }
    }
    assert_eq!(sources.len(), 78);
    for expected in [
        "?drawno?",
        "SHOP&/TEST",
        "WEIG.&/(KG)",
        "Block &&&/Unit No.",
        "&USEC.(Right)",
        "&UELEC.",
        "&UPLAN",
    ] {
        assert!(
            sources.iter().any(|s| s == expected),
            "{expected:?} not among the rebuilt sources"
        );
    }
    // 'SHOP&/TEST'：两个 run，第二个 newline = 1；srclen = 8 + 4 + 2 = 14。
    let shop = picture
        .segments
        .iter()
        .flatten()
        .flat_map(|s| s.prims().map(|(_, p)| p))
        .find_map(|p| match &p.payload {
            Payload::Text2(t) if t.source().as_deref() == Some("SHOP&/TEST") => Some(t),
            _ => None,
        })
        .unwrap();
    let runs = shop.runs().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].text().as_str(), runs[0].newline), ("SHOP", 0));
    assert_eq!((runs[1].text().as_str(), runs[1].newline), ("TEST", 1));
    assert_eq!(shop.srclen, 14);
    assert_eq!(shop.text(), "SHOP\nTEST");
    assert_eq!(shop.format().to_string(), "FONT     5,RIGHT,LBODY,BASE");

    common::assert_listing_matches(&file, true, &oracle);
}
