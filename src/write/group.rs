//! GPWL / GPSET / GPITEM 的便捷写入口 —— E3D 二级层级（secondary hierarchy）那一套。
//!
//! 底层原语 [`Transaction::insert`] / [`RecordDraft::set_explicit_refno`] /
//! [`RecordDraft::set_int_array_hunk`] 早就在了，但「往一个分组里加一个设计元素」这件事要串四步：
//! 建 `GPITEM`、拿它的 refno、把 `GPREF` 写成目标、同组去重。散在调用方就各写各的，
//! 而这四步里错任何一步的后果都不是报错，是**读侧静默空组**——所以收在这里。
//!
//! 语义照真库样本（`docs/plans/2026-11-08-gpwl-gpset-group-nodes.md` §0）：
//!
//! ```text
//! WORL ──members──▶ GPWL ──members──▶ GPSET ──members──▶ GPITEM ──GPREF──▶ 设计元素(可跨库)
//!                                                    └─SCOSEL──▶ 规则求值成员
//! ```
//!
//! 三个容易做错的点：
//!
//! 1. **`GPITEM` 是成员链上的元素，真身靠 `GPREF` 隐式属性指过去**。所以加成员是
//!    「插一个 GPITEM + 写它的 GPREF」，**不是** [`Transaction::add_reference_member`]——
//!    后者是 `GROU` 那种引用列表 noun 的路子，`GPITEM` 不走这条。搞混的后果是成员链上直接
//!    出现真身 refno，E3D 展开时找不到 GPITEM，分组变空。
//! 2. **`GPREF` 可以跨库**（真库里 GPITEM 在 250218、目标在 250209）。[`Transaction`] 按单库
//!    文件打开，[`RecordDraft::set_explicit_refno`] 只写 refno 值、不校验目标存在，所以
//!    **目标存在性由调用方在写之前查**；写进悬空引用，读侧就落到 `target_missing` 那个坏槽。
//! 3. **同组去重**：读侧 `resolved_members_on` 对同一真身只留第一次，写侧也就不该造出
//!    「一个 target 两个 GPITEM」这种状态。去重放在这里，和读侧对偶。
//!
//! 只在副本库演练：这些 API 写的是真实 dabacon 文件，写坏了不可恢复。

use e3d_attlib::db1_hash;

use super::transaction::{MemberPosition, Transaction, TransactionError};
use crate::refno::RefNo;

/// `GPREF` 的属性名。`GPITEM` 模板声明了它（desvir.dat 2.10 实锤），是隐式属性，
/// 值走 [`RecordDraft::set_explicit_refno`]。
const GPREF: &str = "GPREF";
/// `SCOSEL` 的属性名。`GPSET` 模板声明它是 int 数组，值走 type-20 hunk。
const SCOSEL: &str = "SCOSEL";

/// [`Transaction::insert_gpset_member`] 的结果：新造的 GPITEM，或本组已有的那个。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpsetMember {
    /// 新插了一个 GPITEM。
    Inserted(RefNo),
    /// 本组已经收着这个 target，没有新增。
    Existing(RefNo),
}

impl GpsetMember {
    /// 无论新增还是既有，收着这个成员的 GPITEM 的 refno——读侧 `via` 字段就是它。
    pub fn item(self) -> RefNo {
        match self {
            Self::Inserted(item) | Self::Existing(item) => item,
        }
    }
}

impl<'a> Transaction<'a> {
    /// 在 `world`（`WORL`）下建一个 `GPWL`。
    ///
    /// 父不是 `WORL` 时回 [`TransactionError::NotAGroupContainer`]。
    pub fn insert_gpwl(&mut self, world: RefNo, name: &str) -> Result<RefNo, TransactionError> {
        self.require_noun(world, "WORL", "GPWL 的父")?;
        let noun = db1_hash("GPWL");
        self.insert(world, noun, Some(name), |_, _| Ok(()))
    }

    /// 在 `gpwl` 下建一个 `GPSET`。父不是 `GPWL` 时回 [`TransactionError::NotAGroupContainer`]。
    pub fn insert_gpset(&mut self, gpwl: RefNo, name: &str) -> Result<RefNo, TransactionError> {
        self.require_noun(gpwl, "GPWL", "GPSET 的父")?;
        let noun = db1_hash("GPSET");
        self.insert(gpwl, noun, Some(name), |_, _| Ok(()))
    }

    /// 给 `gpset` 加一个显式成员：建 `GPITEM`，把它的 `GPREF` 写成 `target`。
    ///
    /// 返回的 [`GpsetMember`] 里是收着这个成员的 GPITEM（新增的，或本组已有的那个）。
    /// **`target` 必须已经存在**——这里不查，因为 [`Transaction`] 只开着 `gpset` 所在的库，
    /// 而 `target` 可以在别的库。写前校验是调用方的事。
    ///
    /// 同 target 已在本组时不重复加，回 [`GpsetMember::Existing`]。判断靠读一遍现有成员的
    /// `GPREF`，和读侧 `resolved_members_on` 的 `seen` 去重对偶。
    pub fn insert_gpset_member(
        &mut self,
        gpset: RefNo,
        target: RefNo,
        at: MemberPosition,
    ) -> Result<GpsetMember, TransactionError> {
        self.require_noun(gpset, "GPSET", "成员的父")?;
        if let Some(item) = self.gpset_item_for(gpset, target)? {
            return Ok(GpsetMember::Existing(item));
        }
        let noun = db1_hash("GPITEM");
        let gpref = db1_hash(GPREF);
        let item = self.insert(gpset, noun, None, move |draft, _template| {
            draft.set_explicit_refno(gpref, target);
            Ok(())
        })?;
        if at != MemberPosition::Last {
            // `insert` 把新元素放在末尾；调用方要别的位次就再排一次。
            self.reorder(item, at)?;
        }
        Ok(GpsetMember::Inserted(item))
    }

    /// 给 `gpset` 写规则成员：`SCOSEL` 的 int 数组原样写进去。
    ///
    /// 编码由调用方给——e3d-io 不认识 `SCOSEL` 的栈机格式（那是 gen-model 的
    /// `data_interface::scosel`），收 `&[i32]` 是为了不让这层依赖上层。
    pub fn set_gpset_rules(
        &mut self,
        gpset: RefNo,
        scosel: &[i32],
    ) -> Result<(), TransactionError> {
        self.require_noun(gpset, "GPSET", "规则成员的宿主")?;
        let hash = db1_hash(SCOSEL);
        let draft = self.draft_mut(gpset, "set SCOSEL")?;
        draft.set_int_array_hunk(hash, scosel)?;
        Ok(())
    }

    /// `gpset` 现有的成员里，哪个 GPITEM 的 `GPREF` 指向 `target`。
    ///
    /// 用 [`Transaction::members_after_batch`] 而不是 `members_of`：同一个事务里刚插进去的
    /// GPITEM 只进了 `member_ops` 队列，还没进 GPSET 的 draft，`members_of` 看不到它——
    /// 那会让「同一 target 加两次」在同一个事务里漏判。
    fn gpset_item_for(
        &mut self,
        gpset: RefNo,
        target: RefNo,
    ) -> Result<Option<RefNo>, TransactionError> {
        let gpref = db1_hash(GPREF);
        let item_noun = db1_hash("GPITEM");
        for member in self.members_after_batch(gpset)? {
            if self.noun_of(member)? != item_noun {
                continue;
            }
            let Some(words) = self.explicit_words_of(member, gpref)? else {
                continue;
            };
            if words.len() == 2 && RefNo::new(words[0], words[1]) == target {
                return Ok(Some(member));
            }
        }
        Ok(None)
    }
}

/// `GPWL` / `GPSET` / `GPITEM` 的属性名，给调用方按名字取 hash 用，免得自己抄字符串。
pub mod attrs {
    /// `GPITEM` 指向真身的隐式属性。
    pub const GPREF: &str = super::GPREF;
    /// `GPSET` 的规则成员（int 数组）。
    pub const SCOSEL: &str = super::SCOSEL;
}

#[cfg(test)]
mod tests {
    //! 这里只验不碰文件也能验的部分：属性名与结果类型的取值。
    //! 真正写库的用例在 `tests/write_stage11_groups.rs`，跑在样本库的临时副本上。

    use super::*;

    #[test]
    fn the_attribute_names_are_the_ones_the_templates_declare() {
        // 抄错一个字母就是一个查不到的属性，而 `set_explicit_refno` 不会报错。
        assert_eq!(attrs::GPREF, "GPREF");
        assert_eq!(attrs::SCOSEL, "SCOSEL");
        assert_ne!(db1_hash("GPITEM"), 0);
    }

    #[test]
    fn a_member_reports_the_item_that_holds_it() {
        let item = RefNo::new(7, 9);
        assert_eq!(GpsetMember::Inserted(item).item(), item);
        assert_eq!(GpsetMember::Existing(item).item(), item);
    }
}
