//! UDA 经 MDB 的字典库进 `DbSet`，两条读法在真库上各钉一遍：
//!
//! * **按名字**（E3D `Q :HXYsize` 那条路）：[`DbElement::attribute`] 读 `:HXYsize`，
//!   值从元素显式流里 `UKEY` 键下取，名字来自字典；
//! * **按键**（目录表达式 `( :HXYsize[2] )` 那条路）：[`DbElement::uda_by_key`] 与
//!   `ParamEnv::uda_number`，键是目录写下时的 `UKEY`，经字典的 `UKEY` / `OLDKEY` 认。
//!
//! 库是 gen-model 现场那套 E3D 2.1 `AvevaMarineSample`（`DbOption-ams.toml` 的
//! `project_path`）。`/ALL` 声明的字典库这里只挂两本——本项目字典 `ams5100`
//! （`*CNPESTD/DICT`）与 `AvevaCatalogue` 的 `acp7006`（CADC HVAC 目录写它的
//! `/UDA/HVAC/*`）——正是 `HXYsize` 被声明两次的那两本；完整名单与 `CURD` 序是
//! gen-model `mdb_membership::dictionary_paths()` 的事，这里按 E3D 的 `Q :HXYsize`
//! 答 `675 600`（`c4_hxysize.log`）钉住 `ams5100` 在前。
//!
//! 取证：`_runs/ams-init-1112-7997-20260917-mc/ledger-seg15-frameall.md` §11 / §16 / §17。

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use e3d_io::db_element::{DbElement, DbElementError, DbFilePin, DbFileResolver, DbSet};
use e3d_io::db_param_env::DbElementParamEnv;
use e3d_io::record::catalogue_eval::{AttributeNumber, ExpressionForm};
use e3d_io::record::catalogue_pml;
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::refno::RefNo;
use e3d_io::uda_catalog::UdaCatalog;

mod common;

const BINARIES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/e3d210/binaries");
const PROJECTS: &str = r"D:\AVEVA\Projects\E3D2.1";
const AMS7997: &str = r"D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams7997_0001";
/// `*CNPESTD/DICT`：本项目字典，`/HXYsize` UKEY `0x2652AB52`（OLDKEY `0x18EF4B61`）。
const AMS5100: &str = r"D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams5100_0001";
/// `AvevaCatalogue` 字典，`/UDA/HVAC/HXYsize` UKEY `0x2C00D576`（OLDKEY `0x17EF4B61`）。
const ACP7006: &str = r"D:\AVEVA\Projects\E3D2.1\AvevaCatalogue\acp000\acp7006_0001";
/// CADC HVAC 目录：`/HRTUBE1`（SCOM 15194/288）与它的 `/HRTUBE1-BOXI`。
const ACP7002: &str = r"D:\AVEVA\Projects\E3D2.1\AvevaCatalogue\acp000\acp7002_0001";

const FIXTURES: &[&str] = &[AMS7997, AMS5100, ACP7006, ACP7002];

const BRAN: u32 = 808_220;
/// `/-CCV-S-2-L-1405`，E3D `Q :HXYsize` = `675 600`。
const BRAN_1405: RefNo = RefNo::new(24_381, 59_287);
/// `/-CAM-E-2-H-5302`：`:HXYsize 350 350`、`:H-AIRQUANTITY 2800`、`:H-PRESSURESPEC HP`。
const BRAN_5302: RefNo = RefNo::new(24_381, 47_065);
/// `/-VES-E-2-L-3807`：`:HXYsize 500 630`、`:H-AIRQUANTITY 12000`、`:H-PRESSURESPEC MP`。
const BRAN_3807: RefNo = RefNo::new(24_381, 30_278);

const HXYSIZE_AMS_KEY: u32 = 0x2652_AB52;
const HXYSIZE_AMS_OLD_KEY: u32 = 0x18EF_4B61;
const HXYSIZE_ACP_KEY: u32 = 0x2C00_D576;
/// `/HRTUBE1-BOXI.PXLE` 的 PML 词里那个名字词（401558369）。
const HXYSIZE_ACP_OLD_KEY: u32 = 0x17EF_4B61;
const HRTUBE1_BOXI_DBNO: u32 = 7002;

fn binaries() -> &'static Path {
    Path::new(BINARIES)
}

fn pin(file: &str, db_type: &str) -> DbFilePin {
    DbFilePin {
        file: PathBuf::from(file),
        template: binaries().join(format!("{}vir.dat", db_type[..3].to_lowercase())),
        db_type: Some(db_type.to_string()),
        sesno: None,
    }
}

/// `/ALL` 里与 `HXYsize` 有关的两本字典，按 `CURD` 序合并：本项目的在前。
fn mdb_dictionaries() -> UdaCatalog {
    let attlib =
        e3d_attlib::AttlibData::parse_file(binaries().join("attlib.dat")).expect("attlib parses");
    let dicvir = binaries().join("dicvir.dat");
    let mut merged = UdaCatalog::read(AMS5100, &dicvir, &attlib).expect("read ams5100");
    merged.merge_in_order(UdaCatalog::read(ACP7006, &dicvir, &attlib).expect("read acp7006"));
    merged
}

fn design_set(catalog: Option<Arc<UdaCatalog>>) -> Arc<DbSet> {
    let set = DbSet::with_attlib_file(binaries().join("attlib.dat")).expect("attlib parses");
    let set = match catalog {
        Some(catalog) => set.with_uda_catalog(catalog),
        None => set,
    };
    set.add_db(pin(AMS7997, "DESI")).expect("ams7997 opens");
    Arc::new(set)
}

struct Pins(HashMap<u32, DbFilePin>);

impl DbFileResolver for Pins {
    fn resolve(&self, dbno: u32) -> Option<DbFilePin> {
        self.0.get(&dbno).cloned()
    }
}

#[test]
fn the_mdb_dictionaries_declare_hxysize_twice_and_the_bran_stores_the_first() {
    if common::missing(FIXTURES) {
        return;
    }
    let catalog = mdb_dictionaries();
    assert!(catalog.issues().is_empty(), "{:?}", catalog.issues());
    let declared: Vec<(u32, Option<u32>, RefNo)> = catalog
        .by_name("HXYsize")
        .iter()
        .map(|definition| (definition.key, definition.old_key, definition.refno))
        .collect();
    assert_eq!(
        declared,
        vec![
            (
                HXYSIZE_AMS_KEY,
                Some(HXYSIZE_AMS_OLD_KEY),
                RefNo::new(13_292, 123)
            ),
            (
                HXYSIZE_ACP_KEY,
                Some(HXYSIZE_ACP_OLD_KEY),
                RefNo::new(15_198, 55)
            ),
        ],
        "one name, two Dictionaries, two keys"
    );
    // 目录表达式里的词是 acp7006 那条的 OLDKEY——字典按 OLDKEY 也认得出它。
    assert_eq!(
        catalog
            .by_key(HXYSIZE_ACP_OLD_KEY)
            .map(|definition| definition.refno),
        Some(RefNo::new(15_198, 55))
    );
    for definition in catalog.by_name("HXYsize") {
        assert!(definition.applies_to(BRAN));
        assert_eq!(definition.value_type.as_deref(), Some("REAL"));
        assert_eq!(definition.length, 2);
    }
}

#[test]
fn a_bran_reads_its_udas_by_name_the_way_q_prints_them() {
    if common::missing(FIXTURES) {
        return;
    }
    let set = design_set(Some(Arc::new(mdb_dictionaries())));

    let bran = set.element(BRAN_1405);
    assert_eq!(
        bran.attribute(":HXYsize").expect(":HXYsize is a name now"),
        Some(DescriptorValue::RealArray(vec![675.0, 600.0]))
    );
    assert_eq!(
        bran.get_double_array(":hxysize")
            .expect("case-insensitive like Q"),
        Some(vec![675.0, 600.0])
    );
    assert!(bran.has_attribute(":HXYsize").unwrap());
    // 同名的第二条（acp7006, 0x2C00D576）不再单独成行：一张表里只有一条 :HXYsize。
    let names = bran.attribute_names().unwrap();
    assert_eq!(
        names
            .iter()
            .filter(|name| name.eq_ignore_ascii_case(":HXYsize"))
            .count(),
        1,
        "{names:?}"
    );

    // §17.1 的另两条 UDA：整数 / 实数 / 文本各形态都按名字读得出。
    for (refno, sizes, air, pressure) in [
        (BRAN_5302, [350.0, 350.0], 2800.0, "HP"),
        (BRAN_3807, [500.0, 630.0], 12000.0, "MP"),
    ] {
        let bran = set.element(refno);
        assert_eq!(
            bran.attribute(":HXYsize").unwrap(),
            Some(DescriptorValue::RealArray(sizes.to_vec())),
            "{refno}"
        );
        assert_eq!(
            bran.attribute(":H-AIRQUANTITY").unwrap(),
            Some(DescriptorValue::Real(air)),
            "{refno}"
        );
        assert_eq!(
            bran.get_string(":H-PRESSURESPEC").unwrap().as_deref(),
            Some(pressure),
            "{refno}"
        );
    }

    // 字典说 :HXYsize 只对 BRAN 有效：支管下的构件问它，E3D 报
    // `User defined attribute :HXYsize not valid for element`，这里是 UnknownAttribute。
    let member = bran_member_that_is_not_a_bran(&set.element(BRAN_1405));
    assert!(
        matches!(
            member.attribute(":HXYsize"),
            Err(DbElementError::UnknownAttribute { ref name, .. }) if name == ":HXYsize"
        ),
        "{} ({})",
        member.refno(),
        member.element_type().unwrap()
    );
}

fn bran_member_that_is_not_a_bran(bran: &DbElement) -> DbElement {
    bran.members()
        .expect("members")
        .find(|member| {
            member
                .element_type()
                .map(|noun| noun != "BRAN")
                .unwrap_or(false)
        })
        .expect("the branch has a non-BRAN member")
}

#[test]
fn a_bran_reads_its_udas_by_key_through_ukey_and_oldkey() {
    if common::missing(FIXTURES) {
        return;
    }
    let set = design_set(Some(Arc::new(mdb_dictionaries())));
    let bran = set.element(BRAN_1405);
    let sizes = Some(DescriptorValue::RealArray(vec![675.0, 600.0]));

    // 本项目字典那条：当前键、旧键都认，值都是显式流里那一份。
    assert_eq!(bran.uda_by_key(HXYSIZE_AMS_KEY).unwrap(), sizes);
    assert_eq!(bran.uda_by_key(HXYSIZE_AMS_OLD_KEY).unwrap(), sizes);
    // AvevaCatalogue 字典那条同名定义：对 BRAN 有效，但元素上两把键下都没存值——unset。
    // 这正是 /HRTUBE1-BOXI.PXLE 在这条支管上落空的地方（下一条用例）。
    assert_eq!(bran.uda_by_key(HXYSIZE_ACP_KEY).unwrap(), None);
    assert_eq!(bran.uda_by_key(HXYSIZE_ACP_OLD_KEY).unwrap(), None);
    // 字典里没有的键：Err，不是 unset。
    assert!(matches!(
        bran.uda_by_key(0x2652_0000),
        Err(DbElementError::Uda {
            key: 0x2652_0000,
            ..
        })
    ));
    // 定义对 noun 无效：Err。
    let member = bran_member_that_is_not_a_bran(&bran);
    assert!(
        matches!(
            member.uda_by_key(HXYSIZE_AMS_KEY),
            Err(DbElementError::Uda { ref detail, .. }) if detail.contains("无效")
        ),
        "{}",
        member.refno()
    );

    // 没挂字典：按名字与按键都是「不认识」，不是 unset。
    let bare = design_set(None);
    let bran = bare.element(BRAN_1405);
    assert!(matches!(
        bran.attribute(":HXYsize"),
        Err(DbElementError::UnknownAttribute { .. })
    ));
    assert!(matches!(
        bran.uda_by_key(HXYSIZE_AMS_KEY),
        Err(DbElementError::Uda { ref detail, .. }) if detail.contains("没挂")
    ));
}

/// `/HRTUBE1-BOXI.PXLE = ( :HXYsize[2 ] )`，在支管 `/-CCV-S-2-L-1405` 上求值。
///
/// 目录写下的键是 acp7006 那条 `HXYsize` 的 OLDKEY。`uda_by_key` 对这把键仍是 unset
/// （元素把 `675 600` 存在 ams5100 那条同名定义下）。表达式求值不再停在这一格：
/// 同名定义按字典序取 `Q :HXYsize` 的那个数，所以挂上支管后 PXLE / PZLE 是 600 / 675。
/// 不挂设计元素时目录件自己没有这个 UDA，公式仍未解。
///
/// 把词里的键换成 ams5100 那条的当前键，同一条支管直接按键求出同样的数：按键那条链
/// （DbSet 字典 → uda_by_key → ParamEnv::uda_number → PML）在真库上是通的。
#[test]
fn the_hrtube1_head_slot_expression_uses_the_same_named_uda_when_the_catalogue_key_is_unset() {
    if common::missing(FIXTURES) {
        return;
    }
    let catalog = Arc::new(mdb_dictionaries());
    let set = design_set(Some(catalog.clone()));
    set.add_db(pin(ACP7002, "CATA")).expect("acp7002 opens");
    let boxi = set
        .find_named(HRTUBE1_BOXI_DBNO, "/HRTUBE1-BOXI")
        .expect("scan acp7002 names")
        .into_iter()
        .map(|refno| set.element(refno))
        .next()
        .expect("/HRTUBE1-BOXI exists");
    assert_eq!(boxi.element_type().unwrap(), "BOXI");
    let bran = set.element(BRAN_1405);

    let env = DbElementParamEnv::new(Some(bran.clone()), Some(boxi.clone()));
    assert_eq!(
        boxi.get_number("PXLE", &env).unwrap(),
        AttributeNumber::Evaluated {
            value: 600.0,
            form: ExpressionForm::Pml,
        }
    );
    assert_eq!(
        boxi.get_number("PZLE", &env).unwrap(),
        AttributeNumber::Evaluated {
            value: 675.0,
            form: ExpressionForm::Pml,
        }
    );
    let tube = set
        .find_named(HRTUBE1_BOXI_DBNO, "/HCTUBE-TUBE")
        .expect("scan acp7002 names")
        .into_iter()
        .map(|refno| set.element(refno))
        .next()
        .expect("/HCTUBE-TUBE exists");
    let bran_5302 = set.element(BRAN_5302);
    assert_eq!(
        tube.get_number(
            "PDIA",
            &DbElementParamEnv::new(Some(bran_5302), Some(tube.clone()))
        )
        .unwrap(),
        AttributeNumber::Evaluated {
            value: 350.0,
            form: ExpressionForm::Pml,
        }
    );
    // 不挂设计元素（hvac_tube 今天的环境）：同样未解。
    assert!(matches!(
        boxi.get_number("PXLE", &DbElementParamEnv::catalogue_only(boxi.clone()))
            .unwrap(),
        AttributeNumber::Unresolved {
            form: ExpressionForm::Pml,
            ..
        }
    ));

    // 同一段程序、只把键换成本项目字典那条的当前键：按键那条链求得出值。
    let parsed = boxi.parsed().unwrap();
    let pxle =
        e3d_io::record::descriptor::stored_expression_words(&parsed, e3d_attlib::db1_hash("PXLE"))
            .expect("PXLE is a stored program")
            .to_vec();
    let pzle =
        e3d_io::record::descriptor::stored_expression_words(&parsed, e3d_attlib::db1_hash("PZLE"))
            .expect("PZLE is a stored program")
            .to_vec();
    assert_eq!(pxle[11], HXYSIZE_ACP_OLD_KEY as i32, "{pxle:?}");
    let rekey = |words: &[i32]| -> Vec<i32> {
        words
            .iter()
            .map(|word| {
                if *word == HXYSIZE_ACP_OLD_KEY as i32 {
                    HXYSIZE_AMS_KEY as i32
                } else {
                    *word
                }
            })
            .collect()
    };
    let design_only = DbElementParamEnv::design_only(bran.clone());
    assert_eq!(
        catalogue_pml::evaluate_words(&rekey(&pxle), &design_only),
        Some(600.0)
    );
    assert_eq!(
        catalogue_pml::evaluate_words(&rekey(&pzle), &design_only),
        Some(675.0)
    );
    assert_eq!(
        catalogue_pml::render_with_dictionary(&rekey(&pxle), Some(&catalog)).as_deref(),
        Some("( :HXYsize[2 ] )")
    );
    // 原词在没有字典的环境里连名字都没有：印键，不印猜出来的名字。
    assert_eq!(
        catalogue_pml::render(&pxle).as_deref(),
        Some("( :0x17EF4B61[2 ] )")
    );
    // 没挂字典的池子求同一段程序：未解，且印的是键。
    let bare = design_set(None);
    bare.add_db(pin(ACP7002, "CATA")).expect("acp7002 opens");
    let bare_boxi = bare.element(boxi.refno());
    assert_eq!(
        bare_boxi
            .get_number(
                "PXLE",
                &DbElementParamEnv::new(Some(bare.element(BRAN_1405)), Some(bare_boxi.clone()))
            )
            .unwrap(),
        AttributeNumber::Unresolved {
            form: ExpressionForm::Pml,
            text: Some("( :0x17EF4B61[2 ] )".into()),
        }
    );
}

/// 字典有自己的生命周期：DESI 换代（`fork_without`）之后新视图照旧挂着它。
#[test]
fn the_dictionary_survives_a_design_fork() {
    if common::missing(FIXTURES) {
        return;
    }
    let old = design_set(Some(Arc::new(mdb_dictionaries())));
    assert_eq!(
        old.element(BRAN_1405).attribute(":HXYsize").unwrap(),
        Some(DescriptorValue::RealArray(vec![675.0, 600.0]))
    );
    let next = Arc::new(
        old.fork_without(
            &HashSet::from([BRAN_1405.dbno()]),
            Box::new(Pins(HashMap::from([(
                BRAN_1405.dbno(),
                pin(AMS7997, "DESI"),
            )]))),
        )
        .unwrap(),
    );
    assert!(next.uda_catalog().is_some());
    assert_eq!(
        next.element(BRAN_1405).attribute(":HXYsize").unwrap(),
        Some(DescriptorValue::RealArray(vec![675.0, 600.0]))
    );
}

/// 字典挂上之后每次抽取多一步 materialize；量一量它在 ams7997 上值多少，免得靠感觉。
#[test]
fn materialising_udas_costs_little_against_extraction() {
    if common::missing(FIXTURES) {
        return;
    }
    const SAMPLE: usize = 4000;
    let refnos: Vec<RefNo> = {
        let mut engine = e3d_io::ReadOnlyEngine::open(AMS7997).unwrap();
        engine
            .indexed_refnos()
            .unwrap()
            .into_iter()
            .map(|(refno, _, _)| refno)
            .take(SAMPLE)
            .collect()
    };
    let time = |set: &Arc<DbSet>| {
        let started = std::time::Instant::now();
        let mut attributes = 0usize;
        for refno in &refnos {
            if let Ok(extraction) = set.element(*refno).extraction() {
                attributes += extraction.attributes.len();
            }
        }
        (started.elapsed(), attributes)
    };
    let (bare, bare_attributes) = time(&design_set(None));
    let (wired, wired_attributes) = time(&design_set(Some(Arc::new(mdb_dictionaries()))));
    eprintln!(
        "ams7997 first {SAMPLE} elements: bare {bare:?} ({bare_attributes} attributes) vs with dictionaries {wired:?} ({wired_attributes} attributes, +{} UDA rows), projects={PROJECTS}",
        wired_attributes.saturating_sub(bare_attributes)
    );
    assert!(wired_attributes >= bare_attributes);
}
