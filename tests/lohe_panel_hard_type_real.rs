//! `ATTRIB LOHE` 找面板时认的是硬类型：楼板（FLOOR）名下的开孔件读得到板厚（e3d-model #80）。
//!
//! 语料：
//!
//! - aps250110_0001 的 `PFIT 2013286654/1059`，属主是 FLOOR `2013286654/1058`，楼板名下第一只
//!   PLOO 的 `HEIG` 是 20。E3D 导出的这块楼板上有这只开孔件切穿整厚的洞，刀高就是这个数。
//! - ams251183_0001 的 `PFIT 2013287727/2222`，属主是 PANE，板厚 5（e3d-model #50 的样本）：
//!   noun 就是 PANE 的那一支不该因为这次改动变。
//!
//! 缺文件就跳过。

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use e3d_io::db_element::{DbElement, DbFilePin, DbSet};
use e3d_io::db_param_env::DbElementParamEnv;
use e3d_io::record::catalogue_eval::{AttribRef, ParamEnv};
use e3d_io::refno::RefNo;

const APS250110: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaPlantSample\aps000\aps250110_0001";
const AMS251183: &str = r"D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251183_0001";

fn open(design: &str) -> Arc<DbSet> {
    let set = DbSet::with_attlib_file(common::ATTLIB).expect("attlib parses");
    set.add_db(DbFilePin {
        file: PathBuf::from(design),
        template: PathBuf::from(common::DESVIR),
        db_type: Some("DESI".to_string()),
        sesno: None,
    })
    .expect("design database opens");
    Arc::new(set)
}

fn lohe(element: &DbElement) -> Option<f64> {
    DbElementParamEnv::design_only(element.clone()).attribute_number(&AttribRef::named("LOHE"))
}

/// 面板名下第一只 PLOO 的 `HEIG`，直接从库里读：期望值不经被测的那条路。
fn first_loop_height(panel: &DbElement) -> f64 {
    let ring = panel
        .members()
        .expect("the panel lists its members")
        .into_iter()
        .find(|member| member.element_type().ok().as_deref() == Some("PLOO"))
        .expect("the panel owns a loop");
    ring.get_double("HEIG")
        .expect("HEIG reads")
        .expect("the loop stores its height")
}

#[test]
fn a_fitting_under_a_floor_reads_the_floors_thickness() {
    if common::missing(&[APS250110, common::ATTLIB, common::DESVIR]) {
        return;
    }
    let set = open(APS250110);
    let fitting = set.element(RefNo::new(2_013_286_654, 1059));
    assert_eq!(fitting.element_type().unwrap(), "PFIT");
    let floor = fitting
        .owner()
        .expect("the owner reads")
        .expect("the fitting has an owner");
    assert_eq!(floor.element_type().unwrap(), "FLOOR");
    assert_eq!(floor.refno(), RefNo::new(2_013_286_654, 1058));
    assert_eq!(first_loop_height(&floor), 20.0, "这块楼板厚 20");

    assert_eq!(lohe(&fitting), Some(20.0));
}

/// `CLIMB` 先比当前元素：面板自己问，答的是自己那只环。这一条的依据是反编译出来的次序
/// （`dbgenlib/CLIMB@0x53953CC`），没有 E3D 导出的样本印证——AMS、APS 两个样例项目里自己带 `SPRE`
/// 的面板类元素（AMS 584 只 PANE、125 只 GWALL，APS 32 只 PANE、16 只 FLOOR），`SPRE` 链要么走不到
/// 目录件，要么走到的目录件不读 `LOHE`，没有公式在面板身上问它。
#[test]
fn a_floor_asked_directly_answers_its_own_loop() {
    if common::missing(&[APS250110, common::ATTLIB, common::DESVIR]) {
        return;
    }
    let set = open(APS250110);
    let floor = set.element(RefNo::new(2_013_286_654, 1058));
    assert_eq!(floor.element_type().unwrap(), "FLOOR");

    assert_eq!(lohe(&floor), Some(20.0));
}

#[test]
fn a_fitting_under_a_panel_still_reads_the_panels_thickness() {
    if common::missing(&[AMS251183, common::ATTLIB, common::DESVIR]) {
        return;
    }
    let set = open(AMS251183);
    let fitting = set.element(RefNo::new(2_013_287_727, 2222));
    assert_eq!(fitting.element_type().unwrap(), "PFIT");
    let panel = fitting
        .owner()
        .expect("the owner reads")
        .expect("the fitting has an owner");
    assert_eq!(panel.element_type().unwrap(), "PANE");
    assert_eq!(first_loop_height(&panel), 5.0, "这块板厚 5");

    assert_eq!(lohe(&fitting), Some(5.0));
}
