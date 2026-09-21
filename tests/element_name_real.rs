//! 显示名（`FLNM`，[`e3d_io::element_name`]）对真库的行为验证。
//!
//! 口径：地面真值是 E3D 自己印出来的——`tests/fixtures/weld_24383_73931_attlist_tty.log`
//! 里那行 `Name WELD 1 of BRANCH /1RCV0101-60.3-VMB-R90-R332]R333`——以及从记录原语
//! （`stored_name` / `noun_hash` / `member_refnos`）现场推出来的序号，不写死别的 refno。
//!
//! 语料：AvevaMarineSample 的 ams7999（TTY 样本所在库）与 ams8000（DESI，18 MB）。
//! 模板资产在仓内的 `schema/e3d31/binaries/`（见 `tests/common`）。缺文件按仓里惯例跳过并说明。

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use e3d_io::db_element::{DbElement, DbFilePin, DbSet};
use e3d_io::element_name::noun_full_name;
use e3d_io::refno::RefNo;
use e3d_io::ElementName;

const AMS7999: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001";
const AMS8000: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001";

fn open(file: &str) -> (Arc<DbSet>, u32) {
    let set = DbSet::with_attlib_file(common::ATTLIB).expect("attlib parses");
    let dbno = set
        .add_db(DbFilePin {
            file: PathBuf::from(file),
            template: PathBuf::from(common::DESVIR),
            db_type: Some("DESI".to_string()),
            sesno: None,
        })
        .expect("the design database opens");
    (Arc::new(set), dbno)
}

fn world(set: &Arc<DbSet>, dbno: u32) -> DbElement {
    let world = set.element(set.metadata(dbno).expect("metadata").world);
    assert!(
        world.owner().unwrap().is_none(),
        "the WORLD is the element without an owner"
    );
    world
}

/// 成员表原序，去掉 Nulref 占位——`member_names` 不给它们起名。
fn valid_members(owner: &DbElement) -> Vec<RefNo> {
    owner
        .member_refnos()
        .unwrap()
        .into_iter()
        .filter(RefNo::is_valid)
        .collect()
}

/// 名字里的属主名：`LNPART` 不以 `/` 开头就补一个。
fn slashed(name: &str) -> String {
    if name.starts_with('/') {
        name.to_string()
    } else {
        format!("/{name}")
    }
}

/// TTY 实录那只焊口：短形态是 `WELD 1`，全形态与 E3D 印的一字不差；属主 BRAN 有名字，
/// 三个字段同一串；整批与单个给同一个答案。
#[test]
fn the_weld_of_the_tty_sample_gets_the_name_e3d_printed() {
    if common::missing(&[AMS7999, common::ATTLIB, common::DESVIR]) {
        return;
    }
    let (set, _) = open(AMS7999);
    let weld = set.element(RefNo::new(24383, 73931));
    let name = weld.display_name().unwrap();
    assert_eq!(name.stored, None);
    assert!(!name.is_named());
    assert_eq!(name.short, "WELD 1");
    assert_eq!(
        name.full,
        "WELD 1 of BRANCH /1RCV0101-60.3-VMB-R90-R332]R333"
    );

    let bran = weld.owner().unwrap().expect("a WELD sits in a BRAN");
    assert_eq!(bran.refno(), RefNo::new(24383, 73930));
    assert_eq!(
        bran.display_name().unwrap(),
        ElementName::named("/1RCV0101-60.3-VMB-R90-R332]R333")
    );

    let batch = bran.member_names().unwrap();
    assert_eq!(
        batch.iter().map(|(member, _)| member.refno()).collect::<Vec<_>>(),
        valid_members(&bran),
        "member_names keeps the stored member order"
    );
    let (_, in_batch) = batch
        .iter()
        .find(|(member, _)| member.refno() == weld.refno())
        .expect("the WELD is a member of its owner");
    assert_eq!(*in_batch, name);
}

/// 根层：WORLD 的成员。无名 SITE 叫 `SITE n`——n 是同名词序号，后面不跟 ` of WORLD /*`；
/// 有名字的三个字段同一串；每个成员单个算与整批算一致。
#[test]
fn world_members_do_not_carry_the_world_in_their_names() {
    if common::missing(&[AMS7999, AMS8000, common::ATTLIB, common::DESVIR]) {
        return;
    }
    let mut unnamed_seen = 0;
    for file in [AMS7999, AMS8000] {
        let (set, dbno) = open(file);
        let world = world(&set, dbno);
        let batch = world.member_names().unwrap();
        assert!(!batch.is_empty(), "{file}: the WORLD has members");

        let mut per_noun: HashMap<u32, usize> = HashMap::new();
        for (member, name) in &batch {
            let noun_hash = member.noun_hash().unwrap();
            let ordinal = {
                let slot = per_noun.entry(noun_hash).or_insert(0);
                *slot += 1;
                *slot
            };
            match member.stored_name().unwrap() {
                Some(stored) => {
                    assert_eq!(name.stored.as_deref(), Some(stored.trim()));
                    assert_eq!(name.short, stored.trim());
                    assert_eq!(name.full, stored.trim());
                }
                None => {
                    unnamed_seen += 1;
                    let expected = format!(
                        "{} {ordinal}",
                        noun_full_name(&member.element_type().unwrap())
                    );
                    assert_eq!(name.short, expected, "{file}: {}", member.refno());
                    assert_eq!(
                        name.full, expected,
                        "{file}: {} — the WORLD must not appear in the name",
                        member.refno()
                    );
                }
            }
            assert_eq!(member.display_name().unwrap(), *name, "{file}: {}", member.refno());
        }
    }
    eprintln!("unnamed WORLD members checked: {unnamed_seen}");
}

/// `LNPART` 的递推，对树里前几百个无名元素逐个验：
///
/// ```text
/// X 无名、属主 O 是库根      → X.full == X.short
/// X 无名、O 有名字           → X.full == X.short + " of " + <O 名词全词> + " " + /O.name
/// X 无名、O 也无名           → X.full == X.short + " of " + O.full
/// ```
///
/// 右边全部由属主自己的 `display_name` 与记录原语推出，不依赖被测函数对 X 的答案。
#[test]
fn the_full_form_recurs_through_the_owner_chain() {
    if common::missing(&[AMS8000, common::ATTLIB, common::DESVIR]) {
        return;
    }
    const ENOUGH: usize = 300;
    let (set, dbno) = open(AMS8000);
    let mut stack = vec![world(&set, dbno)];
    let mut checked = 0;
    let mut nested = 0;
    while let Some(owner) = stack.pop() {
        if checked >= ENOUGH {
            break;
        }
        let owner_name = owner.display_name().unwrap();
        let owner_is_root = owner.owner().unwrap().is_none();
        let owner_segment = match &owner_name.stored {
            Some(stored) => format!(
                "{} {}",
                noun_full_name(&owner.element_type().unwrap()),
                slashed(stored)
            ),
            None => owner_name.full.clone(),
        };
        let batch = owner.member_names().unwrap();
        assert_eq!(
            batch.iter().map(|(member, _)| member.refno()).collect::<Vec<_>>(),
            valid_members(&owner),
            "{}: member_names keeps the stored member order",
            owner.refno()
        );
        for (member, name) in batch {
            if name.is_named() {
                assert_eq!(name.short, name.full, "{}", member.refno());
            } else {
                checked += 1;
                let expected = if owner_is_root {
                    name.short.clone()
                } else {
                    format!("{} of {owner_segment}", name.short)
                };
                assert_eq!(name.full, expected, "{}", member.refno());
                assert_eq!(member.display_name().unwrap(), name, "{}", member.refno());
                if !owner_is_root && owner_name.stored.is_none() {
                    nested += 1;
                }
            }
            if member.member_count().unwrap() > 0 {
                stack.push(member);
            }
        }
    }
    eprintln!("unnamed elements checked: {checked} (owner also unnamed: {nested})");
    assert!(checked > 0, "ams8000 has unnamed elements");
}
