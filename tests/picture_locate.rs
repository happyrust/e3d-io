//! GW 计划 S5（`docs/plans/2026-09-24-gw-picture-read-plan.md` §4）：从 PADD 库到
//! 图片文件与缩略图。
//!
//! 期望值来自读法 3 第一轮 §2.2 / §2.4（`paddprobe pics / eldata`）：Plant 61 个 PADD 库
//! 328 个 PVNO > 0 的属主 328 命中、Marine 20 个库 720 个属主 718 命中（缺的 2 个在
//! `/MARKS` 库）、子目录 = w1 mod 32 零反例；`gw\thumbs.py` 从 Plant 库拉出 30 张缩略图
//! （`/APS-0001/1` 898 字 → 3 564 字节载荷 → 3 402 字节 PNG）。每个工程要开几十个库、
//! 扫全部元素，各约 1–2 s，不 `#[ignore]`。

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use e3d_io::picture::{locate, thumbnail, PaddProject, PictureLocation};
use e3d_io::{RefNo, SchemaPack};

/// One project's picture-owner census: `(noun, owners, PVNO ≤ 0, found)`.
fn census(project: &PaddProject, pic_dir: &str) -> BTreeMap<String, (usize, usize, usize)> {
    let mut stats: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for owner in project.picture_owners().expect("scan the picture owners") {
        let noun = owner.element_type().unwrap();
        let entry = stats.entry(noun).or_default();
        entry.0 += 1;
        match locate(Path::new(pic_dir), &owner).expect("EXFI / PVNO read") {
            None => entry.1 += 1,
            Some(location) => {
                assert_eq!(
                    location.subdir,
                    format!("{:02}", owner.refno().word1 % 32),
                    "subdirectory rule"
                );
                if location.exists() {
                    entry.2 += 1;
                }
            }
        }
    }
    stats
}

fn sheets_and_overlays(stats: &BTreeMap<String, (usize, usize, usize)>) -> (usize, usize, usize) {
    ["SHEE", "OVER"]
        .iter()
        .filter_map(|noun| stats.get(*noun))
        .fold((0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2))
}

#[test]
fn every_plant_sheet_picture_is_where_the_name_rule_says() {
    if common::missing(&[common::PLANT_PADD_DIR, common::PLANT_PIC_DIR]) {
        return;
    }
    let pack = SchemaPack::open(common::SCHEMA_PACK).unwrap();
    let project =
        PaddProject::open(&pack, common::PLANT_PADD_DIR).expect("open the Plant PADD databases");
    assert_eq!(
        project.databases().len(),
        61,
        "AvevaPlantSample has 61 PADD databases"
    );
    let stats = census(&project, common::PLANT_PIC_DIR);
    let (owners, no_picture, found) = sheets_and_overlays(&stats);
    assert_eq!(owners - no_picture, 328, "{stats:?}");
    assert_eq!(found, 328, "{stats:?}");
    assert_eq!(stats["SHEE"], (301, 0, 301), "{stats:?}");
    assert_eq!(stats["OVER"], (27, 0, 27), "{stats:?}");
    // 背景与模板库的 PVNO ≤ 0，没有文件；标签库 / 符号库各有 2 个带 PVNO 5 / EXFI 35
    // 的（外抽取库的图片，不在本工程的 apspic 里）。
    assert_eq!(stats["BACK"], (121, 121, 0), "{stats:?}");
    assert_eq!(stats["SHTMPL"], (20, 20, 0), "{stats:?}");
    assert_eq!(stats["ISOLB"], (1, 1, 0), "{stats:?}");
    assert_eq!(stats["LALB"], (24, 22, 0), "{stats:?}");
    assert_eq!(stats["SYLB"], (69, 67, 0), "{stats:?}");

    // /APS-0001/1 = SHEE 2013286677/265 → apspic\09\M2013286677-265-10000-11，
    // 它在主库 EXFI 10000 之外没有别的抽取副本。
    let sheet = project
        .find_named("/APS-0001/1")
        .unwrap()
        .into_iter()
        .find(|e| e.element_type().unwrap() == "SHEE")
        .expect("the sheet /APS-0001/1");
    assert_eq!(sheet.refno(), RefNo::new(2013286677, 265));
    let location = locate(Path::new(common::PLANT_PIC_DIR), &sheet)
        .unwrap()
        .expect("a sheet with PVNO 11 has a picture");
    assert_eq!(
        location,
        PictureLocation::new(
            Path::new(common::PLANT_PIC_DIR),
            RefNo::new(2013286677, 265),
            10000,
            11
        )
    );
    assert_eq!(location.path.to_str().unwrap(), common::PLANT_PIC_BIG);
    assert!(location.exists());
    assert!(location.copies().is_empty(), "{:?}", location.copies());
}

#[test]
fn every_marine_sheet_picture_but_the_two_marks_sheets_is_on_disk() {
    if common::missing(&[common::MARINE_PADD_DIR, common::MARINE_PIC_DIR]) {
        return;
    }
    let pack = SchemaPack::open(common::SCHEMA_PACK).unwrap();
    let project =
        PaddProject::open(&pack, common::MARINE_PADD_DIR).expect("open the Marine PADD databases");
    assert_eq!(
        project.databases().len(),
        20,
        "AvevaMarineSample has 20 PADD databases"
    );
    let stats = census(&project, common::MARINE_PIC_DIR);
    let (owners, no_picture, found) = sheets_and_overlays(&stats);
    assert_eq!(owners - no_picture, 720, "{stats:?}");
    assert_eq!(found, 718, "{stats:?}");
    assert_eq!(stats["SHEE"], (702, 0, 700), "{stats:?}");
    assert_eq!(stats["OVER"], (18, 0, 18), "{stats:?}");
    assert_eq!(stats["BACK"], (149, 149, 0), "{stats:?}");
    // 缺的两张是 /MARKS 那个库（dbno 相同）里的两个 SHEE：/MARKS 自己和一个无名的。
    let mut missing = Vec::new();
    for owner in project.picture_owners().unwrap() {
        if let Some(location) = locate(Path::new(common::MARINE_PIC_DIR), &owner).unwrap() {
            if !location.exists() {
                missing.push(owner);
            }
        }
    }
    assert_eq!(
        missing.len(),
        2,
        "{:?}",
        missing.iter().map(|e| e.refno()).collect::<Vec<_>>()
    );
    let marks = project
        .find_named("/MARKS")
        .unwrap()
        .into_iter()
        .find(|e| e.element_type().unwrap() == "SHEE")
        .expect("the sheet named /MARKS");
    for sheet in &missing {
        assert_eq!(sheet.element_type().unwrap(), "SHEE");
        assert_eq!(
            sheet.refno().dbno(),
            marks.refno().dbno(),
            "{}",
            sheet.refno()
        );
    }
    assert!(missing.iter().any(|e| e.refno() == marks.refno()));
    // Marine 同一属主同一版本有多个 EXFI 副本：16864 的版本 1 在 10000 之外还有 7 份。
    let sheet = project.element(RefNo::new(2013287597, 16864));
    let location = locate(Path::new(common::MARINE_PIC_DIR), &sheet)
        .unwrap()
        .expect("BACK / SHEE 16864 has a picture");
    assert_eq!(location.path.to_str().unwrap(), common::MARINE_PIC_TEXT);
    let copies = location.copies();
    assert_eq!(copies.len(), 7, "{copies:?}");
    assert!(copies.iter().all(|p| p
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("M2013287597-16864-")));
}

#[test]
fn the_thirty_plant_thumbnails_come_out_as_the_pngs_thumbs_py_wrote() {
    let tsv = format!(r"{}\thumbs.tsv", common::DRAFT31_THUMBS);
    if common::missing(&[common::PLANT_PADD_DIR, &tsv]) {
        return;
    }
    // thumbs.tsv：db, refno, name, exfi, pvno, png, picture。同名 sheet（两个库都有
    // /APS-0001/1…）写到同一个 png，磁盘上留下的是最后一行的——只对那些比字节。
    let rows: Vec<Vec<String>> = std::fs::read_to_string(&tsv)
        .unwrap()
        .lines()
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect();
    assert_eq!(rows.len(), 30);
    let mut last_writer: BTreeMap<String, usize> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        last_writer.insert(row[5].clone(), i);
    }

    let pack = SchemaPack::open(common::SCHEMA_PACK).unwrap();
    let project = PaddProject::open(&pack, common::PLANT_PADD_DIR).unwrap();
    let mut extracted = 0usize;
    let mut compared = 0usize;
    let mut differing = Vec::new();
    // 同名 sheet（aps250133 与 aps250501 是同一套图的两份库）的缩略图应两两相同。
    let mut by_name: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        let (w0, w1) = row[1].split_once('/').unwrap();
        let sheet = project.element(RefNo::new(w0.parse().unwrap(), w1.parse().unwrap()));
        assert_eq!(sheet.element_type().unwrap(), "SHEE");
        assert_eq!(
            sheet.stored_name().unwrap().as_deref(),
            Some(row[2].as_str())
        );
        assert_eq!(
            sheet.members_of_type("THUMBN").unwrap().len(),
            1,
            "{}",
            row[2]
        );
        let thumb = thumbnail(&sheet)
            .unwrap()
            .unwrap_or_else(|| panic!("{} has a thumbnail", row[2]));
        extracted += 1;
        assert!(thumb.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(thumb.png.ends_with(b"IEND\xae\x42\x60\x82"));
        // IHDR：128 × 128。
        assert_eq!(&thumb.png[12..16], b"IHDR");
        assert_eq!(
            u32::from_be_bytes(thumb.png[16..20].try_into().unwrap()),
            128
        );
        assert_eq!(
            u32::from_be_bytes(thumb.png[20..24].try_into().unwrap()),
            128
        );
        by_name
            .entry(row[2].clone())
            .or_default()
            .push(thumb.png.clone());
        if last_writer[&row[5]] == i {
            let want = std::fs::read(&row[5]).unwrap();
            compared += 1;
            if thumb.png != want {
                differing.push((row[2].clone(), thumb.png.len(), want.len()));
            }
        }
    }
    assert_eq!(extracted, 30);
    assert_eq!(compared, last_writer.len());
    assert_eq!(compared, 18);
    for (name, pngs) in &by_name {
        assert!(
            pngs.windows(2).all(|w| w[0] == w[1]),
            "{name}: the two databases' copies differ"
        );
    }
    // thumbs.py 靠 `paddprobe dump` 的文本树配对 SHEE 与 ATTDAT，而那份树每层只印
    // 前几个成员（"… +3 more"），/APS-1000/1 的 THUMBN 被截掉，配到了别的元素的
    // 194 字节空白 PNG 上；按成员表走的 `thumbnail()` 给出 2 574 字节的真图，两份库
    // 一致。其余 17 张逐字节相同。
    assert_eq!(
        differing,
        vec![("/APS-1000/1".to_string(), 2574, 194)],
        "{differing:?}"
    );

    // /APS-0001/1 of aps250133: 898 words → 3 564-byte payload → 3 402-byte PNG.
    let sheet = project.element(RefNo::new(2013286677, 265));
    let thumb = thumbnail(&sheet).unwrap().unwrap();
    assert_eq!(thumb.payload.len(), 3564);
    assert_eq!(thumb.png.len(), 3402);
    assert_ne!(thumb.uuid, [0, 0, 0, 0]);
    // An element without a THUMBN member (the sheet's DRWG) has no thumbnail.
    let drawing = sheet.owner().unwrap().unwrap();
    assert_eq!(drawing.element_type().unwrap(), "DRWG");
    assert_eq!(thumbnail(&drawing).unwrap(), None);
}
