//! 目录件经 `CMPR` 读特性元素上的公式，`ATTRIB RPRO` 走该目录件的设计表。
//!
//! 语料：`acp7000_0001` 的 `/GAADTDAKD20033`，`CMPR` 指向 `acp7008_0001` 会话 47
//! 的 `/DAKD20033-DT-CW`。特性元素自己没有 `DTRE`。缺文件就跳过。

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use e3d_io::db_element::{DbFilePin, DbSet};
use e3d_io::db_param_env::DbElementParamEnv;
use e3d_io::record::catalogue_eval::AttributeNumber;
use e3d_io::ReadOnlyEngine;

const ACP7000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7000_0001";
const ACP7008: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaCatalogue\acp000\acp7008_0001";
const PART: &str = "/GAADTDAKD20033";

fn open() -> Arc<DbSet> {
    let set = DbSet::with_attlib_file(common::ATTLIB).expect("attlib parses");
    set.add_db(DbFilePin {
        file: PathBuf::from(ACP7000),
        template: PathBuf::from(common::CATVIR),
        db_type: Some("CATA".to_string()),
        sesno: None,
    })
    .expect("catalogue opens");
    set.add_db(DbFilePin {
        file: PathBuf::from(ACP7008),
        template: PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/schema/e3d31/binaries/provir.dat"
        )),
        db_type: Some("PROP".to_string()),
        sesno: Some(47),
    })
    .expect("property database opens at session 47");
    Arc::new(set)
}

#[test]
fn a_part_reads_the_property_formula_against_its_design_table() {
    if common::missing(&[
        ACP7000,
        ACP7008,
        common::ATTLIB,
        common::CATVIR,
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/schema/e3d31/binaries/provir.dat"
        ),
    ]) {
        return;
    }

    let mut engine = ReadOnlyEngine::open(ACP7000).expect("catalogue engine");
    let refno = engine
        .find_by_name(PART)
        .expect("name table")
        .unwrap_or_else(|| panic!("{PART} is not in the catalogue"));
    let set = open();
    let part = set.element(refno);
    let property = part
        .get_element("CMPR")
        .expect("CMPR reads")
        .expect("the part points at a property element");
    assert_eq!(property.element_type().unwrap().as_str(), "CMPD");

    let alone = DbElementParamEnv::catalogue_only(property.clone());
    let unresolved = property.get_number("CWEI", &alone).expect("CWEI exists");
    assert!(
        matches!(
            unresolved,
            AttributeNumber::Unresolved { .. }
                | AttributeNumber::Evaluated { .. }
                | AttributeNumber::Stored(_)
        ),
        "property element answers CWEI, got {unresolved:?}"
    );

    let through = part
        .catalogue_property_number("CWEI")
        .expect("CWEI through CMPR");
    match through {
        AttributeNumber::Evaluated { value, .. } | AttributeNumber::Stored(value) => {
            assert!(
                (value - 30.93).abs() < 1e-6,
                "constant formula should evaluate to 30.93, got {value}"
            );
        }
        other => panic!("CWEI through the part did not evaluate: {other:?}"),
    }

    let owner = set.element(set.metadata(7000).expect("catalogue metadata").world);
    let with_table =
        find_part_with_design_table(&set, &owner).expect("a catalogue part with PTDTRE or DTRE");
    let env = DbElementParamEnv::catalogue_only(with_table.clone());
    let mut keys = Vec::new();
    for table_name in ["PTDTRE", "DTRE"] {
        let Some(table) = with_table.get_element(table_name).ok().flatten() else {
            continue;
        };
        let Ok(rows) = table.members() else {
            continue;
        };
        for row in rows {
            let key = match row.attribute("DKEY") {
                Ok(Some(e3d_io::record::descriptor::DescriptorValue::Word { text, .. })) => text,
                Ok(Some(e3d_io::record::descriptor::DescriptorValue::Text(text))) => Some(text),
                _ => None,
            };
            if let Some(key) = key {
                keys.push(format!("{key}={:?}", env.rpro(&key)));
            }
        }
    }
    assert!(
        keys.iter().any(|line| !line.ends_with("=None")),
        "{} design-table keys did not evaluate: {keys:?}",
        with_table
            .display_name()
            .ok()
            .map(|name| name.full)
            .unwrap_or_default()
    );
}

fn find_part_with_design_table(
    set: &Arc<DbSet>,
    root: &e3d_io::db_element::DbElement,
) -> Option<e3d_io::db_element::DbElement> {
    let mut queue = vec![root.clone()];
    let mut seen = 0usize;
    while let Some(element) = queue.pop() {
        seen += 1;
        if seen > 200_000 {
            return None;
        }
        let has_table = ["PTDTRE", "DTRE"]
            .iter()
            .any(|name| element.get_element(name).ok().flatten().is_some());
        if has_table {
            return Some(element);
        }
        if let Ok(members) = element.member_refnos() {
            for refno in members.into_iter().filter(e3d_io::refno::RefNo::is_valid) {
                queue.push(set.element(refno));
            }
        }
    }
    None
}
