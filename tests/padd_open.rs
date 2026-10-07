//! DRAFT 计划 P0（`docs/plans/2026-09-28-draft-module-next-steps-plan.md` §3）：
//! PADD（DRAFT 图纸库）的模板 `padvir.dat` 登进 3.1 schema pack，PADD 库经与
//! DESI / CATA 同一个 `DbSet` 门面打开——读法 3 第一轮就说「读侧 e3d-io 现有能力
//! 足够，无需改代码」，这里把那句话钉成测试。
//!
//! 语料：AvevaPlantSample 的 `aps250133_0001`（PADD，17.6 MB）。期望值来自读法 3
//! 第一轮报告 §2.1（`paddprobe dump` 对同一文件实读）：4 923 个元素、50 种名词、
//! 12 个 SHEE；`/APS-0001/1` 是 SHEE，`EXFI 10000`、`PVNO 11`，属主 DRWG `/APS-0001`。
//! 缺文件按仓里惯例跳过并说明。

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use e3d_attlib::db1_hash;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::{DbElement, DbFilePin, DbSet, ReadOnlyEngine, ScanTier, SchemaPack};

const APS250133: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaPlantSample\aps000\aps250133_0001";

/// A word-typed attribute read as the number its raw word is (`EXFI`, `PVNO`).
fn word_number(element: &DbElement, name: &str) -> Option<i32> {
    match element.attribute(name).expect("attribute reads")? {
        DescriptorValue::Word { raw, .. } => Some(raw as i32),
        DescriptorValue::Int(int) => Some(int),
        other => panic!("{name} is {other:?}, neither Word nor Int"),
    }
}

/// 模板登记本身不依赖工程数据：pack 里有 `padvir.dat`，manifest 把它登在 `PADD` 下。
#[test]
fn the_shipped_pack_registers_the_padd_template() {
    let pack = SchemaPack::open(common::SCHEMA_PACK).expect("the shipped 3.1 pack opens");
    assert!(
        pack.template_types().any(|db_type| db_type == "PADD"),
        "manifest.json binaries.templates lacks PADD: {:?}",
        pack.template_types().collect::<Vec<_>>()
    );
    let padvir = pack.template("PADD").expect("PADD names a template file");
    assert_eq!(padvir.file_name().unwrap(), "padvir.dat");
    assert!(padvir.is_file(), "{} is not a file", padvir.display());
    // 大小写不敏感，与 DESI / CATA 一样。
    assert_eq!(pack.template("padd").unwrap(), padvir);
}

/// PADD 库走 `DbSet` + `padvir.dat`：元素普查与第一轮的 paddprobe 数字相同，
/// SHEE 按名找得到、整数属性按描述子读得出、属主链通。
#[test]
fn a_padd_database_opens_through_the_dbset_facade() {
    if common::missing(&[APS250133, common::ATTLIB]) {
        return;
    }
    let pack = SchemaPack::open(common::SCHEMA_PACK).expect("pack opens");
    let set = DbSet::with_attlib_file(common::ATTLIB).expect("attlib parses");
    let dbno = set
        .add_db(DbFilePin {
            file: PathBuf::from(APS250133),
            template: pack.template("PADD").expect("PADD template"),
            db_type: Some("PADD".to_string()),
            sesno: None,
        })
        .expect("aps250133 opens with padvir.dat");
    assert_eq!(dbno, 250133, "the file header states the db number");
    assert_eq!(set.db_type(dbno).as_deref(), Some("PADD"));
    let set = Arc::new(set);

    // 普查（索引层，一页一元素）：4 923 元素、50 种名词、12 SHEE —— 第一轮 §2.1。
    let shee = db1_hash("SHEE");
    let mut engine = ReadOnlyEngine::open(APS250133).expect("open aps250133");
    let mut total = 0usize;
    let mut nouns = BTreeSet::new();
    let mut sheets = Vec::new();
    for item in engine.scan_elements(ScanTier::Header).expect("scan") {
        let element = item.expect("indexed element header");
        total += 1;
        nouns.insert(element.noun_hash);
        if element.noun_hash == shee {
            sheets.push(element.refno);
        }
    }
    assert_eq!(total, 4923, "element count");
    assert_eq!(nouns.len(), 50, "distinct nouns");
    assert_eq!(sheets.len(), 12, "SHEE count");

    // 按名找 sheet，读它的图片定位属性（第一轮 §2.2：文件名 = M<w0>-<w1>-<EXFI>-<PVNO>）。
    let found = set
        .find_named(dbno, "/APS-0001/1")
        .expect("NAME table lookup");
    assert_eq!(found.len(), 1, "one element is named /APS-0001/1: {found:?}");
    let sheet = set.element(found[0]);
    assert!(sheets.contains(&sheet.refno()), "the named element is one of the 12 SHEE");
    assert_eq!(sheet.element_type().unwrap(), "SHEE");
    // `padvir.dat` 把 EXFI / PVNO 的数据类型定成 word（E3D 的 Q 打出来是整数，
    // 描述子却不是 int）：门面照描述子回 `Word { raw, .. }`，数就是 `raw`——
    // paddprobe 的 `word_attr` 一直是这么读的，这里钉住这个形。
    assert_eq!(
        word_number(&sheet, "EXFI"),
        Some(10000),
        "EXFI = extract db number, {:?}",
        sheet.attribute("EXFI")
    );
    assert_eq!(
        word_number(&sheet, "PVNO"),
        Some(11),
        "PVNO = picture version, {:?}",
        sheet.attribute("PVNO")
    );

    // 属主链：SHEE ← DRWG /APS-0001 ← REGI ← DEPT ← WORL。
    let drawing = sheet.owner().unwrap().expect("a sheet has an owner");
    assert_eq!(drawing.element_type().unwrap(), "DRWG");
    assert_eq!(drawing.stored_name().unwrap().as_deref(), Some("/APS-0001"));
    let region = drawing.owner().unwrap().expect("DRWG has an owner");
    assert_eq!(region.element_type().unwrap(), "REGI");
    let department = region.owner().unwrap().expect("REGI has an owner");
    assert_eq!(department.element_type().unwrap(), "DEPT");

    // 视图：每个 VIEW 存相机，VSCA 是正的图纸比例（第一轮：/APS-0001/1/V1 VSCA 0.01）。
    let views = sheet.members_of_type("VIEW").expect("members of type VIEW");
    assert!(!views.is_empty(), "/APS-0001/1 has views");
    for view in &views {
        let scale = view
            .get_double("VSCA")
            .expect("VSCA reads")
            .expect("VSCA is set on a placed view");
        assert!(scale > 0.0, "VSCA {scale} on {}", view.refno());
    }
    assert!(
        views.iter().any(|view| view.get_double("VSCA").unwrap() == Some(0.01)),
        "one view is drawn at 1:100"
    );
}
