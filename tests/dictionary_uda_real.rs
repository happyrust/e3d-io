use std::path::PathBuf;
use std::sync::Arc;

use e3d_attlib::AttlibData;
use e3d_io::db_element::{DbElementError, DbFilePin, DbSet};
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::record::element::ParsedElement;
use e3d_io::record::template::TemplateProvider;
use e3d_io::refno::RefNo;
use e3d_io::uda_catalog::{UdaCatalog, UdaDefaultResolution};
use e3d_io::ReadOnlyEngine;

const DICTIONARY_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001";
const DICVIR: &str = common::DICVIR;
const ATTLIB: &str = common::ATTLIB;
const BRAN: u32 = 808_220;
const DESIGN_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001";
const DESVIR: &str = common::DESVIR;
const DESIGN_TEMPLATE_TYPE: u32 = 0x000B_0692;

/// ams7997 的风管支管 `/-CCV-S-2-L-1405`。E3D 的 `Q :HXYsize` 答 `675 600`
/// （`_runs/ams-init-1112-7997-20260917-mc/e3d-hvac-all-121918/c4_hxysize.log`）。
const HVAC_DESIGN_DB: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7997_0001";
const HVAC_BRAN: RefNo = RefNo::new(24_381, 59_287);

mod common;

const FIXTURES: &[&str] = &[DICTIONARY_DB, DICVIR, ATTLIB, DESIGN_DB, DESVIR];
const HVAC_FIXTURES: &[&str] = &[DICTIONARY_DB, DICVIR, ATTLIB, HVAC_DESIGN_DB, DESVIR];

#[test]
fn ams_dictionary_materialises_bran_uda_definitions() {
    if common::missing(FIXTURES) {
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB).expect("parse attlib authority");
    let catalog = UdaCatalog::read(DICTIONARY_DB, DICVIR, &attlib)
        .expect("decode AMS Dictionary UDA definitions");
    let bran = catalog.applicable_to(BRAN);

    println!("UDA_TOTAL={}", catalog.definitions().len());
    println!("UDA_ISSUES={}", catalog.issues().len());
    for issue in catalog.issues() {
        println!("UDA_ISSUE|{}|{}", issue.refno, issue.detail);
    }
    println!("BRAN_UDA_TOTAL={}", bran.len());
    for definition in &bran {
        println!(
            "{}|{}|{}|OLDKEY={:?}|{:?}|{:?}|{:?}",
            definition.refno,
            definition.name,
            definition.key,
            definition.old_key,
            definition.value_type,
            definition.default_text,
            definition.default_real
        );
    }
    for name in [
        "SCHrefHole",
        "TPress",
        "ROOM_NO",
        "ISOREF",
        "G_ICS",
        "ISODrawingNo",
        "PFILoose",
        "PsiSystem",
        "MDSComment",
    ] {
        for definition in catalog.by_name(name) {
            println!(
                "TTY_NAME|{}|{}|OWNER={}|ELEL={:?}",
                name, definition.refno, definition.owner, definition.applicable_nouns
            );
        }
    }

    let air = bran
        .iter()
        .find(|definition| definition.name == "H-AIRQUANTITY")
        .expect("H-AIRQUANTITY applies to BRAN");
    assert_eq!(air.refno.word0, 13_292);
    assert_eq!(air.refno.word1, 23);
    // 13292/23 is stored twice. Page 23 holds UKEY 0x17C5C0A1; page 69 holds
    // UKEY 0x2652AB93 and carries 0x17C5C0A1 in OLDKEY, so the key was
    // re-allocated and page 69 is the copy after the move. This asserted the
    // old value while the lookup still answered with the first indexed copy.
    assert_eq!(air.key, 642_952_083);
    assert_eq!(air.old_key, Some(398_835_873));
    assert_eq!(air.value_type.as_deref(), Some("REAL"));
    assert_eq!(air.default_text.as_deref(), Some("0"));
    assert_eq!(air.default_real, Some(0.0));
    assert_eq!(
        air.resolve_default(),
        UdaDefaultResolution::Value(DescriptorValue::Real(0.0))
    );
    assert_eq!(
        catalog.by_name("H-FLOWDIRECTION")[0].resolve_default(),
        UdaDefaultResolution::Value(DescriptorValue::Bool(true))
    );
    assert_eq!(
        catalog.by_name("H-PRESSURESPEC")[0].resolve_default(),
        UdaDefaultResolution::Value(DescriptorValue::Text("TRUE".into()))
    );
    assert!(matches!(
        catalog.by_name("H-TEMPERATURE")[0].resolve_default(),
        UdaDefaultResolution::Invalid { ref raw, ref expected_type }
            if raw == "TRUE" && expected_type == "REAL"
    ));
}

#[test]
fn print_secondary_ams_dictionary_udas() {
    if common::missing(FIXTURES) {
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB).expect("parse attlib authority");
    let catalog = UdaCatalog::read(
        r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5101_0001",
        DICVIR,
        &attlib,
    )
    .expect("decode secondary AMS Dictionary database");
    println!(
        "UDA_TOTAL={} UDA_ISSUES={}",
        catalog.definitions().len(),
        catalog.issues().len()
    );
    for definition in catalog.definitions() {
        println!(
            "{}|{}|{:?}",
            definition.refno, definition.name, definition.applicable_nouns
        );
    }
}

#[test]
fn bran_materialises_every_direct_dictionary_uda_and_defaults() {
    if common::missing(FIXTURES) {
        return;
    }
    let attlib = AttlibData::parse_file(ATTLIB).expect("parse attlib authority");
    let catalog = UdaCatalog::read(DICTIONARY_DB, DICVIR, &attlib)
        .expect("decode AMS Dictionary UDA definitions");
    let mut provider = TemplateProvider::open(DESVIR, Some(DESIGN_TEMPLATE_TYPE))
        .expect("open Design descriptors");
    let mut engine = ReadOnlyEngine::open(DESIGN_DB).expect("open AMS Design database");
    let refno = RefNo::new(24383, 73930);
    let mut extraction = engine
        .extract_element_with_descriptors(refno, &attlib, &mut provider)
        .expect("extract BRAN");
    let view = engine
        .find_element(refno)
        .expect("find BRAN")
        .expect("BRAN exists");
    let parsed = ParsedElement::parse(&view.raw_bytes).expect("parse BRAN");

    assert_eq!(
        catalog.materialize_for_element(&mut extraction, &parsed.explicit_attributes),
        22
    );
    let value = |name: &str| {
        extraction
            .attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .and_then(|attribute| attribute.value.clone())
    };
    assert_eq!(value(":H-AIRQUANTITY"), Some(DescriptorValue::Real(0.0)));
    assert_eq!(value(":H-FLOWDIRECTION"), Some(DescriptorValue::Bool(true)));
    assert_eq!(
        value(":H-PRESSURESPEC"),
        Some(DescriptorValue::Text("TRUE".into()))
    );
    assert_eq!(value(":H-TEMPERATURE"), Some(DescriptorValue::Unset));
}

/// `DbSet` 的 UDA 钩子：挂上字典库，`DbElement` 才按名字答得出 `:HXYsize`。
///
/// 这条钉的是几何那一侧的读法。`DbElementParamEnv` 求 `( :HXYsize[2] )` 是拿
/// `host.attribute(":HXYsize")` 读的，而 `DbSet` 从前没有任何 UDA 入口——
/// 抽取只走描述符，名字在 attlib 里查不到，那一支于是静悄悄地求不出值
/// （ams7997 的 7 条风管头槽位就是这么一根管都没长出来的）。
///
/// 两侧都钉：不挂字典库必须仍是 `UnknownAttribute`（默认不挂，挂上会改变几何，
/// 见 `with_uda_catalog` 的说明），挂上之后读出的值要与 E3D 的 `Q :HXYsize` 逐位相同。
#[test]
fn a_dbset_answers_hxysize_by_name_only_once_the_dictionary_is_attached() {
    if common::missing(HVAC_FIXTURES) {
        return;
    }
    let pin = || DbFilePin {
        file: PathBuf::from(HVAC_DESIGN_DB),
        template: PathBuf::from(DESVIR),
        db_type: Some("DESI".to_string()),
        sesno: None,
    };

    let bare = DbSet::with_attlib_file(ATTLIB).expect("attlib parses");
    bare.add_db(pin()).expect("ams7997 opens");
    let bare = Arc::new(bare);
    let branch = bare.element(HVAC_BRAN);
    assert!(matches!(
        branch.attribute(":HXYsize"),
        Err(DbElementError::UnknownAttribute { name, .. }) if name == ":HXYsize"
    ));

    let attlib = AttlibData::parse_file(ATTLIB).expect("parse attlib authority");
    let catalog = UdaCatalog::read(DICTIONARY_DB, DICVIR, &attlib)
        .expect("decode AMS Dictionary UDA definitions");
    let definition = catalog
        .by_name("HXYsize")
        .first()
        .copied()
        .expect("the Dictionary defines HXYsize")
        .clone();
    // 0x2652AB52 是元素显式流里那个裸键。它是字典分配的 UKEY，不是 refno——
    // 按 refno 拆出来的「dbno 1618」是假线索，那个库并不存在。
    assert_eq!(definition.key, 0x2652_AB52);
    assert_eq!(definition.value_type.as_deref(), Some("REAL"));
    assert_eq!(definition.length, 2);
    assert!(definition.applies_to(808_220), "HXYsize applies to BRAN");

    let wired = DbSet::with_attlib_file(ATTLIB)
        .expect("attlib parses")
        .with_uda_catalog(Arc::new(catalog));
    wired.add_db(pin()).expect("ams7997 opens");
    let wired = Arc::new(wired);
    let branch = wired.element(HVAC_BRAN);
    assert_eq!(
        branch.attribute(":HXYsize").expect("HXYsize is now a name"),
        Some(DescriptorValue::RealArray(vec![675.0, 600.0]))
    );
    assert!(
        branch
            .attribute_names()
            .expect("names")
            .iter()
            .any(|name| name == ":HXYsize"),
        "the materialised UDA joins the declared attribute set"
    );
}
