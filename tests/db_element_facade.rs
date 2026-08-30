//! `DbElement` 门面（S1）对真库的行为验证。
//!
//! 口径：门面必须与本 crate 的原语（`scan_elements` / `find_element` /
//! `extract_element_with_descriptors`）对同一个库、同一个会话给出同一个答案——
//! 期望值全部从原语现场推导，不写死 refno，语料升级重排时测试跟着走。
//!
//! 语料：AvevaMarineSample 的 ams8000（DESI，18 MB）与 ams5052（CATA，跨库
//! 引用的实测目标库，gen-model 侧 92 跳对拍就是这一对）。模板资产在
//! `E:\reverse\e3d\shadow_e3d31_aps_all`。缺文件按仓里惯例跳过并说明。

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use e3d_io::db_element::{template_file_for, DbElementError, DbFilePin, DbFileResolver, DbSet};
use e3d_io::record::descriptor::DescriptorValue;
use e3d_io::{ReadOnlyEngine, ScanTier};

const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";
const AMS5052: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5052_0001";
const ATTLIB: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat";
const DESVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\desvir.dat";
const CATVIR: &str = r"E:\reverse\e3d\shadow_e3d31_aps_all\catvir.dat";

const FIXTURES: [&str; 5] = [AMS8000, AMS5052, ATTLIB, DESVIR, CATVIR];

fn desi_pin() -> DbFilePin {
    DbFilePin {
        file: PathBuf::from(AMS8000),
        template: PathBuf::from(DESVIR),
        db_type: Some("DESI".to_string()),
        sesno: None,
    }
}

fn open_set() -> Arc<DbSet> {
    let set = DbSet::with_attlib_file(ATTLIB).expect("attlib parses");
    let dbno = set.add_db(desi_pin()).expect("ams8000 opens");
    assert_eq!(dbno, 8000, "descriptor states the db number");
    Arc::new(set)
}

/// 原语侧的地面真值：全库第一个「有 owner 且成员 ≥ 2」的元素。
fn ground_truth_parent() -> (e3d_io::refno::RefNo, e3d_io::record::element::ParsedElement) {
    let mut engine = ReadOnlyEngine::open(AMS8000).expect("open ams8000");
    let scan = engine.scan_elements(ScanTier::Full).expect("scan");
    for item in scan {
        let element = item.expect("scan element");
        let Some(parsed) = element.parsed else {
            continue;
        };
        if parsed.owner.is_valid() && parsed.members.len() >= 2 {
            return (element.refno, parsed);
        }
    }
    panic!("ams8000 has no element with an owner and two members?");
}

/// A/B 组：身份与导航必须与记录原语逐项一致，成员保持文件原序。
#[test]
fn identity_and_navigation_agree_with_the_engine() {
    if common::missing(&FIXTURES) {
        return;
    }
    let (refno, truth) = ground_truth_parent();
    let set = open_set();
    let element = set.element(refno);

    // A. 身份
    assert!(!element.is_null());
    assert_eq!(element.exists().unwrap(), true);
    assert_eq!(element.db_no(), 8000);
    assert_eq!(element.noun_hash().unwrap(), truth.noun_hash);
    let type_name = element.element_type().unwrap();
    assert!(!type_name.is_empty());

    // B. owner 与成员：原序逐项一致
    let owner = element.owner().unwrap().expect("truth says it has an owner");
    assert_eq!(owner.refno(), truth.owner);
    assert_eq!(element.member_refnos().unwrap(), truth.members);
    assert_eq!(element.member_count().unwrap(), truth.members.len());
    assert_eq!(
        element.first_member().unwrap().unwrap().refno(),
        truth.members[0]
    );
    assert_eq!(
        element.last_member().unwrap().unwrap().refno(),
        *truth.members.last().unwrap()
    );
    // member(i) 从 1 起数
    assert!(element.member(0).unwrap().is_none());
    assert_eq!(element.member(1).unwrap().unwrap().refno(), truth.members[0]);
    assert!(element
        .member(truth.members.len() + 1)
        .unwrap()
        .is_none());

    // 游标一次一个，走完为止
    let mut cursor = element.members().unwrap();
    assert_eq!(cursor.remaining(), truth.members.len());
    let walked: Vec<_> = cursor.by_ref().map(|member| member.refno()).collect();
    assert_eq!(walked, truth.members);
    assert_eq!(cursor.remaining(), 0);

    // 兄弟导航：老大的 previous 是 None，next 是老二
    let first = element.first_member().unwrap().unwrap();
    assert!(first.previous().unwrap().is_none());
    assert_eq!(
        first.next().unwrap().expect("two members exist").refno(),
        truth.members[1]
    );

    // 后代判定：成员是我的后代，我不是成员的后代
    assert!(first.is_descendant_of(&element).unwrap());
    assert!(!element.is_descendant_of(&first).unwrap());

    // members_of_type：按第一个成员的 noun 过滤，结果是全成员按 noun 筛出的原序子序列
    let first_noun = first.element_type().unwrap();
    let filtered = element.members_of_type(&first_noun).unwrap();
    assert!(!filtered.is_empty());
    assert!(filtered.iter().any(|member| member.refno() == first.refno()));
}

/// C 组：typed getter 的投影必须与描述符抽取逐属性一致——同形取值相等，
/// 异形要么 `TypeMismatch` 要么（Unset/无值）`None`，不允许静默给错形。
#[test]
fn typed_getters_agree_with_descriptor_extraction() {
    if common::missing(&FIXTURES) {
        return;
    }
    let (refno, _) = ground_truth_parent();
    let set = open_set();
    let element = set.element(refno);
    let extraction = element.extraction().unwrap();

    let mut checked = 0usize;
    for attribute in &extraction.attributes {
        let name = attribute.name.as_str();
        let Some(value) = &attribute.value else {
            continue;
        };
        match value {
            DescriptorValue::Unset => {
                assert_eq!(element.attribute(name).unwrap(), None, "{name}");
            }
            DescriptorValue::Bool(expected) => {
                assert_eq!(element.get_bool(name).unwrap(), Some(*expected), "{name}");
            }
            DescriptorValue::Int(expected) => {
                assert_eq!(element.get_integer(name).unwrap(), Some(*expected), "{name}");
                assert_eq!(
                    element.get_double(name).unwrap(),
                    Some(f64::from(*expected)),
                    "{name}: Int 无损加宽"
                );
            }
            DescriptorValue::Real(expected) => {
                assert_eq!(element.get_double(name).unwrap(), Some(*expected), "{name}");
                // 同名要 Int 就是异形，必须报 TypeMismatch 而不是四舍五入
                let error = element.get_integer(name).unwrap_err();
                assert!(
                    matches!(error, DbElementError::TypeMismatch { .. }),
                    "{name}: {error}"
                );
            }
            DescriptorValue::Text(expected) => {
                assert_eq!(
                    element.get_string(name).unwrap().as_deref(),
                    Some(expected.as_str()),
                    "{name}"
                );
            }
            DescriptorValue::Word { text, .. } => {
                if let Some(text) = text {
                    assert_eq!(
                        element.get_string(name).unwrap().as_deref(),
                        Some(text.as_str()),
                        "{name}"
                    );
                }
            }
            DescriptorValue::RefNo(expected) => {
                let got = element.get_ref(name).unwrap();
                if expected.is_valid() {
                    assert_eq!(got, Some(*expected), "{name}");
                } else {
                    assert_eq!(got, None, "{name}: Nulref 归一成 None");
                }
            }
            DescriptorValue::IntArray(expected) => {
                assert_eq!(
                    element.get_integer_array(name).unwrap().as_deref(),
                    Some(expected.as_slice()),
                    "{name}"
                );
            }
            DescriptorValue::RealArray(expected) => {
                assert_eq!(
                    element.get_double_array(name).unwrap().as_deref(),
                    Some(expected.as_slice()),
                    "{name}"
                );
                if expected.len() == 3 {
                    assert_eq!(
                        element.get_position(name).unwrap(),
                        Some([expected[0], expected[1], expected[2]]),
                        "{name}"
                    );
                }
            }
            DescriptorValue::RefNoArray(expected) => {
                assert_eq!(
                    element.get_ref_array(name).unwrap().as_deref(),
                    Some(expected.as_slice()),
                    "{name}"
                );
            }
            // 其余形状走 attribute() 原样出——投影留给消费方
            other => {
                assert_eq!(element.attribute(name).unwrap().as_ref(), Some(other), "{name}");
            }
        }
        checked += 1;
    }
    assert!(checked > 0, "地面真值元素一个带值属性都没有？");

    // 模板不认识的属性名 = 报错，不是 None
    let error = element.attribute("ZZZZNOPE").unwrap_err();
    assert!(
        matches!(error, DbElementError::UnknownAttribute { .. }),
        "{error}"
    );
    assert_eq!(element.has_attribute("ZZZZNOPE").unwrap(), false);
}

/// E 组：名字查找与存储名往返。原生语义没有 name 索引，这条就是一次全树扫。
#[test]
fn name_lookup_round_trips() {
    if common::missing(&FIXTURES) {
        return;
    }
    // 原语侧：第一个存了 NAME 的元素
    let mut engine = ReadOnlyEngine::open(AMS8000).expect("open ams8000");
    let (refno, name) = engine
        .scan_elements(ScanTier::Named)
        .expect("scan")
        .filter_map(|item| {
            let element = item.expect("scan element");
            element.name.map(|name| (element.refno, name))
        })
        .next()
        .expect("ams8000 stores at least one NAME");

    let set = open_set();
    let element = set.element(refno);
    assert_eq!(element.stored_name().unwrap().as_deref(), Some(name.as_str()));

    let hits = set.find_named(8000, &name).unwrap();
    assert!(
        hits.contains(&refno),
        "find_named({name:?}) = {hits:?} 不含 {refno}"
    );
}

/// D 组：跨库引用跳转走池；目标库不在池里时问 resolver（一次），
/// resolver 供出文件即自动补开——gen-model 的 CataDbLocator 语义。
#[test]
fn a_cross_db_reference_jumps_through_the_resolver() {
    if common::missing(&FIXTURES) {
        return;
    }

    struct Ams5052;
    impl DbFileResolver for Ams5052 {
        fn resolve(&self, dbno: u32) -> Option<DbFilePin> {
            (dbno == 5052).then(|| DbFilePin {
                file: PathBuf::from(AMS5052),
                template: PathBuf::from(CATVIR),
                db_type: Some("CATA".to_string()),
                sesno: None,
            })
        }
    }

    let attlib =
        e3d_attlib::AttlibData::parse_file(ATTLIB).expect("attlib parses");
    let set = Arc::new(DbSet::with_resolver(Arc::new(attlib), Box::new(Ams5052)));
    set.add_db(desi_pin()).expect("ams8000 opens");

    // 原语侧找地面真值：全库第一个带「指向 5052 的引用属性」的元素
    // （ams8000 六千余键，全扫秒级）。
    let mut engine = ReadOnlyEngine::open(AMS8000).expect("open ams8000");
    let candidates: Vec<_> = engine
        .scan_elements(ScanTier::Header)
        .expect("scan")
        .map(|item| item.expect("scan element").refno)
        .collect();

    let mut jump: Option<(e3d_io::refno::RefNo, String, e3d_io::refno::RefNo)> = None;
    'outer: for refno in candidates {
        let element = set.element(refno);
        let Ok(extraction) = element.extraction() else {
            continue; // 个别 noun 无模板不是本测试的对象
        };
        for attribute in &extraction.attributes {
            if let Some(DescriptorValue::RefNo(target)) = &attribute.value {
                if target.is_valid() && target.dbno() == 5052 {
                    jump = Some((refno, attribute.name.clone(), *target));
                    break 'outer;
                }
            }
        }
    }
    let (source, attr_name, expected_target) =
        jump.expect("ams8000 全库没有指向 5052 的引用？（实测 82% 命名引用跨库）");

    // 门面侧：get_element 给句柄，第一次读时经 resolver 自动开 5052
    assert!(!set.db_numbers().contains(&5052), "还没读就不该开");
    let element = set.element(source);
    let target = element
        .get_element(&attr_name)
        .unwrap()
        .expect("引用非空");
    assert_eq!(target.refno(), expected_target);
    assert_eq!(target.db_no(), 5052);
    assert_eq!(target.exists().unwrap(), true, "跨库目标在 5052 的索引里");
    let target_type = target.element_type().unwrap();
    assert!(!target_type.is_empty());
    assert!(
        set.db_numbers().contains(&5052),
        "resolver 应已把 5052 开进池"
    );
    assert_eq!(set.pinned_sesno(5052).is_some(), true);
}

/// 拿错模板开库必须当场报错——用 desvir 抽 CATA 库读出来的是别人的 schema。
#[test]
fn a_wrong_template_is_refused_at_open() {
    if common::missing(&FIXTURES) {
        return;
    }
    let set = DbSet::with_attlib_file(ATTLIB).expect("attlib parses");
    let error = set
        .add_db(DbFilePin {
            file: PathBuf::from(AMS5052),
            template: PathBuf::from(DESVIR),
            db_type: Some("CATA".to_string()),
            sesno: None,
        })
        .unwrap_err();
    assert!(
        matches!(error, DbElementError::Open { .. }),
        "desvir 的 template_type 不是 CATA：{error}"
    );
}

/// 时点语义：`None` = 开库冻最新（冻结点可读回）；`Some(n)` = 按 n 打开。
#[test]
fn the_pinned_session_is_frozen_and_reopenable() {
    if common::missing(&FIXTURES) {
        return;
    }
    let set = open_set();
    let frozen = set.pinned_sesno(8000).expect("开过就有冻结点");

    let again = DbSet::with_attlib_file(ATTLIB).expect("attlib parses");
    let mut pin = desi_pin();
    pin.sesno = Some(frozen);
    again.add_db(pin).expect("按冻结点重开");
    assert_eq!(again.pinned_sesno(8000), Some(frozen));

    // 模板文件名派生帮手与真实资产对上
    assert_eq!(
        template_file_for(std::path::Path::new(r"E:\reverse\e3d\shadow_e3d31_aps_all"), "DESI")
            .unwrap(),
        PathBuf::from(DESVIR)
    );
}
