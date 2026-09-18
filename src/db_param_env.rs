//! 活元素背书的 [`ParamEnv`]：目录表达式求值时，`PARAM n` 与 `ATTRIB …` 到底问谁。
//!
//! [`crate::record::catalogue_eval`] 定了 trait 与测试用的 `MapParamEnv`，
//! 注释里也写着「真实生成路径要在活元素上实现 ParamEnv」——这里就是那一个。
//! 镜像 core.dll `catdblib/GATINS`（0x51b2e1e）：`GATINS(instance)` 钉住实例，
//! 之后 `GATPAR(n)`（`exppdms`，0x51b228f）与 `EXEV*`（`exprlib`）都对着它问。
//!
//! ## 实例是两个元素，不是一个
//!
//! 这一条是量出来的，不是设计出来的。拿 ams8000 全库 2612 个路由构件走目录链
//! （`SPRE → SPCO.CATR → SCOM.PTRE → PTSE`），数表达式里出现的名字各自在哪个
//! 元素身上有值：
//!
//! | 名字 | 设计构件 | 目录件 |
//! |---|---:|---:|
//! | `ANGL` | **166** | 0 |
//! | `RADI` | **152** | 0 |
//! | `DESP` | **162** | 0 |
//! | `PARA` | 0 | **214** |
//!
//! 两边不相交，活 E3D 的 noun 导出（`noun_layout.json`，1933 个 noun）也印证：
//! `PARA` 只声明在 `SPRFILE / PRIM / SCOMPONENT / JOINT / WDETAIL / SFITTING`
//! 六个目录 noun 上，`ANGL` / `RADI` / `DESP` 只声明在设计 noun 上。所以求值
//! 环境是「设计构件 + 目录件」这一对，缺一边就有一类名字答不上。
//!
//! ## `PARAM n` 是目录件的 `PARA[n]`，不是设计构件的 `DESP[n]`
//!
//! `GATPAR` 的说法是「实例的第 n 个设计参数」，字面读像 `DESP`。**实测把这个
//! 读法否掉了**：`PTAX.PBOR` 全库 115 处存的就是 `PARAM 1`，而 FTUB
//! `24384/24774` 的 `DESP = [0, 0, 1]`——按 `DESP[1]` 读出的通径是 0；它指向的
//! `SCOM /ACP1000-TFVL-50` 的 `PARA = [50, 30, 3000, 1.5, …]`，按 `PARA[1]`
//! 读出的是 50，与元件名里的 50 mm 对得上。取 `PARA`。
//!
//! 但元组操作数**自己带波段**：Core3D `pplib/DECARG`（0x103C12B8）把 1..50 交给
//! `PARA`、51..100 减 50 交给 `DESP`、101..200 减 100 交给 `IPAR`（再往上是
//! `APAR`/`ADES`/`OPAR`/`ODES`，见 [`crate::record::catalogue_expr::ParamArray`]）。
//! 所以 HVAC 目录 `/RFJFD-BODY.PXLE = [40, 1, 52, 0]` 是 `DESP 2`，问的是设计构件
//! ——ams7997 的 129 只 DAMP、43 只 MESH 此前全按 `PARAM 52` 去问一格长的 `PARA`，
//! 体箱整只丢掉。两条读法互不矛盾：`PARAM n` 仍是 `PARA[n]`，只是 52 不是 `PARAM`。

use std::cell::Cell;

use crate::db_element::{DbElement, DbElementError};
use crate::record::catalogue_eval::{AttribRef, AttributeNumber, ParamArray, ParamEnv};
use crate::record::descriptor::DescriptorValue;

/// 属性求值的递归深度上限。
///
/// 一个属性的公式可以再读另一个属性，那个属性也可以带公式。E3D 里这条链不长，
/// 但库里没有任何东西**禁止**它绕回自己；没有闸就是一次栈溢出。8 层比实测见过
/// 的最深的一条（2 层）宽出一大截，又远不到崩的程度。
const MAX_DEPTH: u32 = 8;

/// `ATTRIB RPRO <键>` 里那个 `RPRO`。
///
/// 它不是属性——`noun_layout.json` 的 1933 个 noun 里既没有叫 `RPRO` 的 noun，
/// 也没有任何 noun 声明过这个名字的属性。它是「按名字查设计表」的引子：
/// 目录件的 `DTRE` 指向一个 `DTSE`，`DTSE` 的每个 `DATA` 成员用 `DKEY` 做键，
/// `PURP` 说这个键的值从哪来。ams8000 上 `RPRO` 共出现 1723 次，键有五个：
/// `TLEN`（1529，设计件全是 `FTUB`）、`RADI`（138，全是 `BEND`）、`YB`、`WIDT`、
/// `HLEN`。
const DESIGN_PROPERTY: &str = "RPRO";

/// `ATTRIB DDES[n]`: the design element's `DESP` with the non-numeric entries
/// blanked.
///
/// Not an attribute either — no noun in `noun_layout.json` declares it — but a
/// pseudo-attribute Core3D computes on demand (`Core3D.dll` 0x10031CE0, the
/// `909175` branch): read `DESP` through `GATRAR`, then walk it with `GATPAR`
/// and zero every entry whose type is `WORD` (623723) or anything other than
/// `DIST` / `NONE`. HVAC `DESP` arrays interleave dimensions with words
/// (`UNIPAR` marks which is which), and the CADC HVAC catalogue reads the
/// connection sizes through this view: `/RFJFD-PA1.PBOR = ( ATTRIB DDES[2 ] +
/// ATTRIB DDES[3 ] )`, `PWID = ( ATTRIB DDES[2 ] )`.
const NUMERIC_DESIGN_PARAMS: &str = "DDES";
/// `ATTRIB CPAR[n]`：设计构件视角下的**目录参数**。
///
/// 也不是属性——1933 个 noun 里没有一个声明 `CPAR`——而是 Core3D 现算的伪属性
/// （`getattlib_des/DESPRE@0x10031CE0` 的 `886899` 支）：先在当前元素上 `DGETRA(PARA)`，
/// 读不到（设计 noun 没有 `PARA`）就 `catdblib/GATGOC@0x1035CFB0` 走 `SPRE →
/// [SPCO → CATR]* → 目录件`，在目录件上再读一次 `PARA`。所以它就是「先设计侧、后目录
/// 侧」地读 `PARA`——正好是 [`DbElementParamEnv::hosts`] 的顺序。
///
/// 语料：BS 型材库 `/BS-GMSS-ANG` 的 `SPVE.PX = ( - ATTRIB CPAR[5 ] )`、
/// `PY = ( ATTRIB CPAR[4 ] - ATTRIB CPAR[1 ] )`——GMSS 被一整族 SPRF 共用，顶点公式
/// 只能问「当前实例的目录件」，`ATTRIB PARA` 在这里语义相同但目录作者写的是 `CPAR`。
/// ams7324 有 1155 只 SCTN 因这一格答不上而退成中心线。
const CATALOGUE_PARAMS: &str = "CPAR";
/// `PARA`：目录件的参数数组，[`CATALOGUE_PARAMS`] 最终读的就是它。
const CATALOGUE_PARAM_ARRAY: &str = "PARA";
/// `UNIPAR`: the per-entry unit/type words that go with `DESP`.
const DESIGN_PARAM_UNITS: &str = "UNIPAR";
/// The `WORD` type word `UNIPAR` carries for a non-numeric `DESP` entry.
const WORD_TYPE: u32 = 623_723;
/// The `DIST` type word — the one numeric type `DDES` keeps besides `NONE`.
const DIST_TYPE: u32 = 939_199;
/// The `NONE` type word.
const NONE_TYPE: u32 = 640_481;

/// Core catalogue expressions use these design-dimension ids.
const DESIGN_RADIUS: i32 = 3;
/// `DDANGLE` 的设计维度编号。见 [`ParamEnv::design_dimension`] 的编号约定。
const DESIGN_ANGLE: i32 = 4;
/// `DDHEIGHT` 的设计维度编号（Core3D `pplib/DECARG` 的操作数 `-5` → 设计构件 `HEIG`）。
const DESIGN_HEIGHT: i32 = 5;

/// Core3D `sellib/ANFIND` (`0x103c8084`) expands numeric selection ranges by
/// 0.001 at each end before comparing `ANSW <= value <= MAXA`.
const SELECTION_RANGE_TOLERANCE: f64 = 0.001;

/// A catalogue selection is shallow in the real projects, but malformed data
/// must not be allowed to loop forever through nested `SELE` records.
const MAX_SELECTION_DEPTH: usize = 32;

/// `DATA.PURP` 的四种取值，说这一行的值该去哪儿取。
///
/// 名字与索引的对应是从 `/ACP1000-TFVL-DTSE` 上读出来的：`CTLF` 行
/// `PURP=DESP NUMB=6`，它的 `PPRO` 正好写着 `( ATTRIB DESP[6 ] )`；`APAR` 行
/// `PURP=DATA NUMB=1`，`PPRO` 写着 `( ATTRIB PARA[1 ] )`。两处自洽，`NUMB`
/// 就是下标。
mod purpose {
    /// 目录件的 `PARA[NUMB]`。
    pub const CATALOGUE_PARAM: &str = "DATA";
    /// 设计构件的 `DESP[NUMB]`。
    pub const DESIGN_PARAM: &str = "DESP";
    /// 设计构件上与 `DKEY` 同名的那个属性。
    pub const DESIGN_ATTRIBUTE: &str = "ATTR";
    /// 这一行自己带的 `PPRO` 公式。
    pub const EXPRESSION: &str = "EXPR";
}

/// 一个被实例化的构件：设计侧元素 + 它选中的目录件。
///
/// 两个都可以缺：只有目录件时能答 `PARAM n` 与 `ATTRIB PARA[i]`，只有设计构件
/// 时能答 `ATTRIB ANGL` 这类。答不上就是答不上，**不会拿另一边的同名属性顶替**
/// ——顶上去求出来的是另一个零件的尺寸，而几何照样出得来、报告照样是绿的。
pub struct DbElementParamEnv {
    design: Option<DbElement>,
    catalogue: Option<DbElement>,
    insulation: bool,
    depth: Cell<u32>,
}

impl std::fmt::Debug for DbElementParamEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbElementParamEnv")
            .field("design", &self.design.as_ref().map(DbElement::refno))
            .field("catalogue", &self.catalogue.as_ref().map(DbElement::refno))
            .finish()
    }
}

impl DbElementParamEnv {
    /// 设计构件 + 目录件。
    pub fn new(design: Option<DbElement>, catalogue: Option<DbElement>) -> Self {
        Self {
            design,
            catalogue,
            insulation: true,
            depth: Cell::new(0),
        }
    }

    /// Evaluate a narrow, uninsulated model: every positive `IPARAM n` is 0.
    pub fn without_insulation(mut self) -> Self {
        self.insulation = false;
        self
    }

    /// 只有设计构件的环境。`PARAM n` 与 `ATTRIB PARA[i]` 会答不上。
    pub fn design_only(design: DbElement) -> Self {
        Self::new(Some(design), None)
    }

    /// 只有目录件的环境。`ATTRIB ANGL` 这类设计侧的名字会答不上。
    pub fn catalogue_only(catalogue: DbElement) -> Self {
        Self::new(None, Some(catalogue))
    }

    /// Evaluate one catalogue design property (`ATTRIB RPRO <key>`).
    ///
    /// Geometry selection uses the same design-table values as profile
    /// expressions.  Exposing this narrow entry point keeps callers from
    /// reimplementing `DTRE -> DATA` selection and its `PPRO/PURP/DPRO`
    /// fallback rules.
    pub fn rpro(&self, key: &str) -> Option<f64> {
        self.design_property(key)
    }

    /// 查名字的顺序：先设计构件，再目录件。
    ///
    /// 实测两边名字集不相交（见模块头那张表），所以这个顺序在 ams8000 上**一次
    /// 都没被用到**——它是为将来某个两边都有的名字准备的。定成设计侧优先的理由：
    /// E3D 问的是实例本身，目录件是实例**指向**的东西，实例自己的值应当压过它。
    /// 这条没有实测背书，真碰上重名要重新取证。
    fn hosts(&self) -> impl Iterator<Item = &DbElement> {
        self.design.iter().chain(self.catalogue.iter())
    }

    /// 在深度预算内跑一段求值，超预算就当答不上。
    fn nested<T>(&self, run: impl FnOnce() -> Option<T>) -> Option<T> {
        if self.depth.get() >= MAX_DEPTH {
            return None;
        }
        self.depth.set(self.depth.get() + 1);
        let out = run();
        self.depth.set(self.depth.get() - 1);
        out
    }

    /// 一个名字在宿主上读成数，含它自己可能带的公式。
    fn number(&self, name: &str) -> Option<f64> {
        self.nested(|| {
            for host in self.hosts() {
                match host.get_number(name, self) {
                    Ok(AttributeNumber::Stored(value)) => return Some(value),
                    Ok(AttributeNumber::Evaluated { value, .. }) => return Some(value),
                    // 模板里没这个名字 / 逻辑 unset / 求不出：换下一个宿主。
                    _ => continue,
                }
            }
            None
        })
    }

    /// `DESP n` / `ATTRIB DESP[n]`：设计构件的第 n 个设计参数，1 起。
    ///
    /// 先问设计构件；该槽位未存储、但当前目录件的 `DTRE` 明确声明了
    /// `PURP=DESP, NUMB=n` 时，按目录表达式的数值空槽语义返回 0。
    ///
    /// 这个回落由原生 E3D 对拍钉住：ams7999 的阀 `/1RCV071VP` 在 ATT 中
    /// `DESP=unset`，但原生 RVM 仍生成阀前后约 130/126 mm 的管段，且 APOS/LPOS
    /// 有效。它的 PTRE 是 `DESP[1] + PARA[2] * 0.5`，原生连接点距中心 115 mm，
    /// 正好是 `0 + 230 * 0.5`；若把同行 `DPRO=230` 当运行时值，结果会错误地变成
    /// 345 mm。Core3D 的 DESPRE 原始属性分支也把未存储 DESP 报为 unset，二者
    /// 一致说明 `DPRO` 是目录定义/交互默认值，不是该表达式的运行时替代值。
    fn design_parameter(&self, index: i32) -> Option<f64> {
        self.stored_design_parameter(index)
            .or_else(|| self.declared_design_parameter(index).then_some(0.0))
    }

    /// Read an actually stored `DESP` slot without applying expression-time
    /// numeric-empty semantics.  RPRO table selection needs this distinction:
    /// an absent design value selects the DATA row's `DPRO`, whereas an
    /// ordinary catalogue expression reads the same declared empty slot as 0.
    fn stored_design_parameter(&self, index: i32) -> Option<f64> {
        let slot = usize::try_from(index).ok()?.checked_sub(1)?;
        match self
            .design
            .as_ref()?
            .attribute(ParamArray::Desp.attribute())
        {
            Ok(Some(DescriptorValue::RealArray(items))) => items.get(slot).copied(),
            Ok(Some(DescriptorValue::IntArray(items))) => {
                items.get(slot).map(|value| f64::from(*value))
            }
            _ => None,
        }
    }

    /// Whether the catalogue design table declares this `DESP` slot.
    fn declared_design_parameter(&self, index: i32) -> bool {
        self.nested(|| {
            let catalogue = self.catalogue.as_ref()?;
            let table = catalogue.get_element("DTRE").ok().flatten()?;
            for row in table.members().ok()? {
                if word_text(&row, "PURP").as_deref() != Some(purpose::DESIGN_PARAM)
                    || whole_number(&row, "NUMB") != Some(index)
                {
                    continue;
                }
                return Some(true);
            }
            Some(false)
        })
        .unwrap_or(false)
    }

    /// `ATTRIB DDES[n]`：见 [`NUMERIC_DESIGN_PARAMS`]。
    ///
    /// 类型取 `UNIPAR[n]`；没有 `UNIPAR`、或它比 `DESP` 短，按 Core 的 `NONE` 读
    /// ——那正是 `GATPAR` 对没登记类型的槽位给出的答复，值原样保留。
    fn numeric_design_parameter(&self, index: i32) -> Option<f64> {
        let value = self.design_parameter(index)?;
        let slot = usize::try_from(index).ok()?.checked_sub(1)?;
        let unit = match self.design.as_ref()?.attribute(DESIGN_PARAM_UNITS) {
            Ok(Some(DescriptorValue::RawWords(words) | DescriptorValue::WordArray(words))) => {
                words.get(slot).copied().unwrap_or(NONE_TYPE)
            }
            Ok(Some(DescriptorValue::IntArray(words))) => words
                .get(slot)
                .map(|word| *word as u32)
                .unwrap_or(NONE_TYPE),
            _ => NONE_TYPE,
        };
        let numeric = unit != WORD_TYPE && (unit == DIST_TYPE || unit == NONE_TYPE);
        Some(if numeric { value } else { 0.0 })
    }

    /// 一个名字在宿主上读成数组，1 起下标。
    ///
    /// `PARA` 与 `DESP` 都是 `DOUBLE` 数组。下标越界给 `None`——`GATPAR` 越界是
    /// 223 号错，不是 0。
    fn indexed(&self, name: &str, index: i32) -> Option<f64> {
        let index = usize::try_from(index).ok()?.checked_sub(1)?;
        for host in self.hosts() {
            match host.attribute(name) {
                Ok(Some(DescriptorValue::RealArray(items))) => {
                    if let Some(value) = items.get(index) {
                        return Some(*value);
                    }
                }
                Ok(Some(DescriptorValue::IntArray(items))) => {
                    if let Some(value) = items.get(index) {
                        return Some(f64::from(*value));
                    }
                }
                _ => continue,
            }
        }
        None
    }

    /// Resolve the insulation `SCOM` selected by a design element's `ISPE`.
    ///
    /// Core3D's `sellib/SELIPC` registers the available selection answers and
    /// runs `SPSEAR`; `ANFIND` then compares each child record's inclusive
    /// `ANSW..MAXA` interval with a 0.001 tolerance.  The on-disk tree is:
    /// `SPEC -> SELE* -> SPCO -> CATR -> SCOM`.
    fn selected_insulation_component(
        &self,
        spec: &DbElement,
    ) -> Result<Option<DbElement>, DbElementError> {
        let Some(mut cursor) = self.insulation_type_branch(spec)? else {
            return Ok(None);
        };

        for _ in 0..MAX_SELECTION_DEPTH {
            match cursor.element_type()?.as_str() {
                "SPCO" => return cursor.get_element("CATR"),
                "SELE" => {
                    let Some(next) = self.select_range_child(&cursor)? else {
                        return Ok(None);
                    };
                    cursor = next;
                }
                _ => return Ok(None),
            }
        }
        Ok(None)
    }

    /// `ISPE` specs identify their top-level usage with textual `TANS=INSU`.
    /// Ignore `TEXT` documentation members.  A unique non-text child is the
    /// Core-compatible fallback for older specs without that label.
    fn insulation_type_branch(
        &self,
        spec: &DbElement,
    ) -> Result<Option<DbElement>, DbElementError> {
        let mut candidates = Vec::new();
        for child in spec.members()? {
            if child.element_type()? == "TEXT" {
                continue;
            }
            if word_or_text(&child, "TANS").as_deref() == Some("INSU") {
                return Ok(Some(child));
            }
            candidates.push(child);
        }
        Ok((candidates.len() == 1).then(|| candidates.remove(0)))
    }

    fn select_range_child(
        &self,
        selection: &DbElement,
    ) -> Result<Option<DbElement>, DbElementError> {
        let Some(question) = word_or_text(selection, "QUES") else {
            return Ok(None);
        };
        let answers = self.selection_answers(&question)?;
        if answers.is_empty() {
            return Ok(None);
        }

        for child in selection.members()? {
            if child.element_type()? == "TEXT" {
                continue;
            }
            if word_or_text(&child, "TANS").as_deref() == Some("ANY") {
                return Ok(Some(child));
            }
            let Some(minimum) = stored_real(&child, "ANSW")? else {
                continue;
            };
            let maximum = stored_real(&child, "MAXA")?.unwrap_or(minimum);
            if answers
                .iter()
                .any(|answer| selection_range_contains(minimum, maximum, *answer))
            {
                return Ok(Some(child));
            }
        }
        Ok(None)
    }

    /// Selection questions are answered by the design element and then its
    /// owner chain.  Core's unqualified PBOR path tries every registered bore;
    /// route containers expose those as HBOR/TBOR, so retain both candidates
    /// rather than guessing which end the catalogue author meant.
    fn selection_answers(&self, question: &str) -> Result<Vec<f64>, DbElementError> {
        let Some(mut cursor) = self.design.clone() else {
            return Ok(Vec::new());
        };
        let names: &[&str] = if question == "PBOR" {
            &["PBOR", "HBOR", "TBOR"]
        } else {
            &[question]
        };
        let mut answers = Vec::new();
        for _ in 0..MAX_SELECTION_DEPTH {
            for name in names {
                if !cursor.has_attribute(name)? {
                    continue;
                }
                if let Some(value) = cursor.get_number(name, self)?.value() {
                    if value.is_finite()
                        && !answers
                            .iter()
                            .any(|seen: &f64| seen.to_bits() == value.to_bits())
                    {
                        answers.push(value);
                    }
                }
            }
            let Some(owner) = cursor.owner()? else {
                break;
            };
            cursor = owner;
        }
        Ok(answers)
    }

    /// `ATTRIB RPRO <键>`：在目录件的设计表里按名字查一个量。
    ///
    /// 目录件 `DTRE` → `DTSE`，逐个 `DATA` 成员比 `DKEY`。命中的那行由 `PURP`
    /// 决定值从哪来，见 [`purpose`]。查不到、`PURP` 不认识、或值那一头是 unset
    /// 都返回 `None`——这张表本身就是「这个键的值在哪」的说明书，说明书没写的
    /// 地方不该由这里替它写。
    fn design_property(&self, key: &str) -> Option<f64> {
        self.nested(|| {
            let catalogue = self.catalogue.as_ref()?;
            let table = catalogue.get_element("DTRE").ok().flatten()?;
            for row in table.members().ok()? {
                if word_text(&row, "DKEY").as_deref() != Some(key) {
                    continue;
                }
                return self.design_property_row(&row, key);
            }
            None
        })
    }

    /// 设计表里命中的那一行怎么读成数。
    ///
    /// **先认 `PPRO`，再认 `PURP`，最后认 `DPRO`。** 这一行的 `PPRO` 往往把
    /// `PURP` + `NUMB` 指的东西直接写成公式：`RADI` 行是 `PURP=DESP NUMB=1` 且
    /// `PPRO = ( ATTRIB DESP[1 ] )`，`CTLF` 行是 `DESP` / `6` 且
    /// `( ATTRIB DESP[6 ] )`，`APAR` 行是 `DATA` / `1` 且 `( ATTRIB PARA[1 ] )`
    /// ——三行互相印证，也正是这三行让「`NUMB` 就是下标」这条读法立得住。既然
    /// 目录自己写了公式，就用它的，别用我从三行样本里归纳出来的映射。
    /// `PURP` 那条路留给 `PPRO` 是空的那些行（`HEIG` 行就是 `PURP=ATTR` 而
    /// `PPRO` 空着）。声明的源都答不上时才轮到 `DPRO`，见 [`Self::row_default`]。
    fn design_property_row(&self, row: &DbElement, key: &str) -> Option<f64> {
        // `PPRO=(ATTRIB DESP[n])` is the interactive value expression for a
        // DATA row.  If the instance does not actually store that slot, Core
        // selects DPRO.  Do not let the generic expression rule (a declared
        // numeric DESP hole evaluates to 0) turn an absent OUTA/OUTB into the
        // opposite application flag before the row default can be considered.
        if word_text(row, "PURP").as_deref() == Some(purpose::DESIGN_PARAM) {
            let index = whole_number(row, "NUMB")?;
            if self.stored_design_parameter(index).is_none() {
                return self.row_default(row);
            }
        }
        // `PPRO` 挂在这一行上，但要在**外层**环境里求：它写的 `PARA[1]` 是目录件
        // 的参数、`DESP[1]` 是设计构件的参数，都不是这一行自己的。
        let stated = self.row_formula(row, "PPRO");
        if let Some(value) = stated {
            return Some(value);
        }
        self.design_property_source(row, key)
            .or_else(|| self.row_default(row))
    }

    /// `PURP` 指定的取值源。
    fn design_property_source(&self, row: &DbElement, key: &str) -> Option<f64> {
        match word_text(row, "PURP")?.as_str() {
            purpose::CATALOGUE_PARAM => self.param(whole_number(row, "NUMB")?),
            purpose::DESIGN_PARAM => self.design_parameter(whole_number(row, "NUMB")?),
            // `NUMB` 在这一类行上也有值（`HEIG` 行是 1），但它不是下标——`HEIG`
            // 在 `FTUB` 的属性表里排第 17 位。当排序用，取值只认名字。
            purpose::DESIGN_ATTRIBUTE => {
                let design = self.design.as_ref()?;
                self.nested(|| match design.get_number(key, self) {
                    Ok(number) => number.value(),
                    Err(_) => None,
                })
            }
            // 这一类行通常由 `PPRO` 定义，而它上面已经求过了。唯一已由真实
            // E3D/RVM 对拍钉住的空槽是 FTUB 的 TLEN：目录 BOARD 的
            // `PXLE := ATTRIB RPRO TLEN` 求得 1500，实例 HEIG 同为 1500。
            // Core 将它作为 FTUB 的派生 true length；这个映射必须同时受 noun
            // 和 key 约束，不能把所有 EXPR 空槽都猜成某个同名或邻近属性。
            purpose::EXPRESSION => self.ftub_true_length(key),
            _ => None,
        }
    }

    /// 行上的一个公式属性（`PPRO` / `DPRO`），在**外层**环境里求。
    fn row_formula(&self, row: &DbElement, name: &str) -> Option<f64> {
        self.nested(|| match row.get_number(name, self) {
            Ok(number) => number.value(),
            Err(_) => None,
        })
    }

    /// `DPRO`：这一行的默认值，声明的源（`PPRO` 公式、`PURP` 指的地方）都答不上
    /// 时才用。
    ///
    /// 这一列不是猜的，是目录作者**专为「这台实例没给这个源」写的**。2026-08-31
    /// 在 ams8000 上把 PTCA 的 `PX`/`PY`/`PZ` 纳入普查后量出来的分界线严丝合缝：
    /// `CTLF` 行声明 `( ATTRIB DESP[6 ] )` 而 `DPRO=( 0 )`，7529 条引用里凡设计件
    /// `DESP` 长度 ≥ 6 的全求出、长度 0/3/5 的全卡住；`OUTA`/`OUTB`（声明
    /// `DESP[4]`/`DESP[5]`，`DPRO=( 1 )`，正负号旗）在长度 5/6 与 0/3 之间同样
    /// 一刀切。默认值本身可以是公式——`LENL`/`LENR` 行的
    /// `DPRO=( ATTRIB RPRO YA / 2 )` 还会再提 `RPRO`——所以与 `PPRO` 同一条求值
    /// 路、同一个深度闸。`DPRO` 自己也空或答不上，照旧 `None`。
    ///
    /// `DPRO` 只替代 **RPRO DATA 行**里未存储的 `PURP=DESP` 值，不改变普通目录
    /// 表达式的数值空槽语义。2026-09-06 的原生 E3D RVM/ATT 对拍显示，ams7999
    /// `/1RCV071VP` 未存的 `DESP[1]/[2]` 在 PTRE 中按 0 求值，而不是取同行的
    /// 230/184；而 AMS 8000 桥架的 OUTA/OUTB 在 RPRO 选择中必须回落到 DPRO=1。
    /// 两条路径由 `stored_design_parameter` 与 `design_parameter` 明确分开。
    fn row_default(&self, row: &DbElement) -> Option<f64> {
        self.row_formula(row, "DPRO")
    }

    /// Core/E3D 的 FTUB 派生设计量 `RPRO TLEN`。
    ///
    /// 证据边界刻意很窄：只认设计实例 FTUB 的 TLEN，并读取该实例的 HEIG；
    /// 其它 noun、其它 EXPR 空槽继续返回 `None`。
    ///
    /// # 这个顶法是对的，1294 条独立对账
    ///
    /// 立这条时只有一个样本（目录 BOARD 的 `PXLE := ATTRIB RPRO TLEN` 求得 1500、
    /// 实例 `HEIG` 同为 1500），而另一种读法（把空槽当 `DPRO = (0)`）在几何上是
    /// **相反**的结论：元件零长、整段都是管身。两者只能靠真库分。
    ///
    /// 2026-08-31 用 `e3d-model/examples/tube_axis_check` 在 ams8000 上分开了。
    /// `FTUB` 的离开点整条来自这里顶出来的 `TLEN`，而它下一站的到达点来自**另一个
    /// 构件自己的 `POS` 加那个构件自己的目录 P 点**——两个毫不相干的来源。
    /// 1472 条以 `FTUB` 离开端起头的槽位里，**1294 条（87.9%）两端重合在 0.051mm
    /// 以内**。若 `TLEN` 该读成 0，这些离开点会全部退回到到达点上，零长比例是 0%。
    ///
    /// 对照组更能说明问题：`BEND`（走 `RPRO RADI`，不经这条分支）94.4%、
    /// `ELBO` 96.8%。`FTUB` 与它们同一个量级，说明 `TLEN` 没有把它往下拽——
    /// 剩下那 12% 的间隙是模型自己的几何，各 noun 都有，不是这条读法的锅。
    fn ftub_true_length(&self, key: &str) -> Option<f64> {
        if key != "TLEN" {
            return None;
        }
        let design = self.design.as_ref()?;
        if design.element_type().ok()?.as_str() != "FTUB" {
            return None;
        }
        self.nested(|| match design.get_number("HEIG", self) {
            Ok(number) => number.value(),
            Err(_) => None,
        })
    }

    /// `ATTRIB <ref> <name>`：跟着引用跳一步，在目标身上读第二个名字。
    ///
    /// 目标自己成为新的设计侧宿主，目录件跟着走——跳过去读到的公式仍可能提
    /// `PARA[i]`，那还是原来那个目录件的。
    fn through_reference(&self, reference: &str, name: &str) -> Option<f64> {
        self.nested(|| {
            for host in self.hosts() {
                let Ok(Some(target)) = host.get_element(reference) else {
                    continue;
                };
                let env = DbElementParamEnv::new(Some(target.clone()), self.catalogue.clone());
                match target.get_number(name, &env) {
                    Ok(AttributeNumber::Stored(value)) => return Some(value),
                    Ok(AttributeNumber::Evaluated { value, .. }) => return Some(value),
                    _ => continue,
                }
            }
            None
        })
    }
}

/// 一个 word 属性的文本形态（`DKEY` / `PURP` 存的都是 `db1_hash` 词）。
fn word_text(element: &DbElement, name: &str) -> Option<String> {
    match element.attribute(name) {
        Ok(Some(DescriptorValue::Word { text, .. })) => text,
        Ok(Some(DescriptorValue::Text(text))) => Some(text),
        _ => None,
    }
}

/// Selection metadata mixes WORD questions (`QUES=TEMP`) with textual answers
/// (`TANS=INSU`).  Read either representation without applying expression
/// evaluation to catalogue control records.
fn word_or_text(element: &DbElement, name: &str) -> Option<String> {
    match element.attribute(name) {
        Ok(Some(DescriptorValue::Word {
            text: Some(value), ..
        })) => Some(value),
        Ok(Some(DescriptorValue::Text(value))) => Some(value),
        _ => None,
    }
}

fn stored_real(element: &DbElement, name: &str) -> Result<Option<f64>, DbElementError> {
    Ok(match element.attribute(name)? {
        Some(DescriptorValue::Real(value)) => Some(value),
        Some(DescriptorValue::Int(value)) => Some(f64::from(value)),
        _ => None,
    })
}

fn selection_range_contains(minimum: f64, maximum: f64, answer: f64) -> bool {
    answer >= minimum - SELECTION_RANGE_TOLERANCE && answer <= maximum + SELECTION_RANGE_TOLERANCE
}

/// 一个整数属性，不管它存成 word 还是 int。
fn whole_number(element: &DbElement, name: &str) -> Option<i32> {
    match element.attribute(name) {
        Ok(Some(DescriptorValue::Int(value))) => Some(value),
        Ok(Some(DescriptorValue::Word { raw, .. })) => i32::try_from(raw).ok(),
        _ => None,
    }
}

impl ParamEnv for DbElementParamEnv {
    /// `PARAM n` —— 目录件的 `PARA[n]`，1 起。见模块头的取证。
    ///
    /// **下标越过 `PARA` 的长度是 0，不是「答不上」。** Core3D `pplib/DECARG`
    /// (`Core3D.dll` 0x103C12B8) 的 `PARAM` 波段（操作数 1..=50）用 `GATPAR` 取这一格，
    /// 只要它置了错号就把操作数当场折成 0 接着算：越界那号（223）额外发条消息，
    /// 「这一格没有」那号（202）连消息都不发，两条路的**值都是 0**——公式不作废，
    /// 图元也不丢。`IPARAM` 早就按同一个做法读（`exppdms/GTGETP`，见 [`Self::iparam`]）。
    ///
    /// SLYK 的 `/Y42X-16C-FBP/GMSE_001` 正落在这条上：`SDSH 16596/17611` 的 `PHEI`
    /// 写着 `PARAM 9`，而那只 SCOM 的 `PARA` 只有 8 格。E3D 求出 0，于是走 CRCATI
    /// 碟臂（`0x10345ACC` case 702883/908861）`PHEI = 0` 那一路，出的是
    /// `gm_CreateCylinder(PDIA/2, 1.0)` 的 1 mm 圆盘；80 根严尺子的基准里 116 只阀门
    /// 各有这么一条 `Cylinder r=50 h=1`，我们这里答 `None` 时整只 SDSH 连同那块几何
    /// 一起没了。
    ///
    /// 只折「`PARA` 读得出来、下标落在它之后」这一种。没有目录件、`PARA` 读不出来
    /// 仍是 `None`：那是问不到，不是问到了零个——模块头第一条说的就是这个分别。
    /// `ATTRIB PARA[i]` 那条 PML 路不在此列，它没有同样的取证。
    fn param(&self, n: i32) -> Option<f64> {
        let index = usize::try_from(n).ok()?.checked_sub(1)?;
        let catalogue = self.catalogue.as_ref()?;
        match catalogue.attribute("PARA") {
            Ok(Some(DescriptorValue::RealArray(items))) => {
                Some(items.get(index).copied().unwrap_or(0.0))
            }
            _ => None,
        }
    }

    /// 元组操作数各波段（Core3D `pplib/DECARG`）：`PARAM` 问目录件、`DESP` 问设计
    /// 构件、`IPARAM` 走保温选件；`APARAM`/`ADESP`/`OPARAM`/`ODESP` 四个波段在
    /// 现有语料（ams1112/7997/8000 + 三目录库）里一次都没出现，没有取证前不答。
    fn parameter(&self, array: ParamArray, n: i32) -> Option<f64> {
        match array {
            ParamArray::Para => self.param(n),
            ParamArray::Desp => self.design_parameter(n),
            ParamArray::IPar => self.iparam(n),
            ParamArray::APar | ParamArray::ADes | ParamArray::OPar | ParamArray::ODes => None,
        }
    }

    /// `IPARAM n` —— 保温参数。
    ///
    /// Core `exppdms/GTGETP` (`core.dll` 0x51B27D8) 对 IPAR 有一条独立于普通
    /// PARA/DESP 的规则：请求下标超过现有 IPAR 数量时返回数值 0，而不是 223。
    /// 无 `ISPE` 属性或 `ISPE` 为 Nulref 都是零个保温参数；这正是无保温目录公式
    /// `SUM PARAM n IPARAM 1` 应继续求值的情况。
    ///
    /// 非空 `ISPE` 按 Core 的 `SPEC -> SELE -> SPCO -> CATR -> SCOM` 路径选件。
    /// 选不到 SPCO 等价于零个 IPAR；请求任意正下标仍按 GTGETP 的特殊分支返回 0。
    /// 解析/跨库读取错误则保留为 `None`，不能把数据库缺失伪装成零保温。
    fn iparam(&self, n: i32) -> Option<f64> {
        if n <= 0 {
            return None;
        }
        if !self.insulation {
            return Some(0.0);
        }
        let design = self.design.as_ref()?;
        let has_ispe = design
            .attribute_names()
            .ok()?
            .iter()
            .any(|name| name == "ISPE");
        if !has_ispe {
            return Some(0.0);
        }
        let spec = match design.get_element("ISPE") {
            Ok(None) => return Some(0.0),
            Ok(Some(spec)) => spec,
            Err(_) => return None,
        };
        let component = match self.selected_insulation_component(&spec) {
            Ok(Some(component)) => component,
            Ok(None) => return Some(0.0),
            Err(_) => return None,
        };
        let index = usize::try_from(n).ok()?.checked_sub(1)?;
        match component.attribute("PARA") {
            Ok(Some(DescriptorValue::RealArray(items))) => {
                Some(items.get(index).copied().unwrap_or(0.0))
            }
            Ok(Some(DescriptorValue::IntArray(items))) => {
                Some(items.get(index).map_or(0.0, |value| f64::from(*value)))
            }
            Ok(None) => Some(0.0),
            Ok(Some(_)) | Err(_) => None,
        }
    }

    /// 设计维度关键字（`DDRADIUS` = 3、`DDANGLE` = 4、`DDHEIGHT` = 5）。
    ///
    /// `DDANGLE` 出现在 529 个 `PTAX` 的
    /// `PAXI` 上（`X DDANGLE Y` 152 处、`X DDANGLE -Y` 377 处），落在弯头这类
    /// 有转角的构件上，而弯头两端 P 点轴的夹角**必须等于**设计构件的 `ANGL`
    /// ——这是一条能在真库上逐条核的等式，不是一个说得通的猜测。
    /// [`crate::record::axis_spec::direction`] 求出的轴按这条对过账。
    ///
    /// `DDRADIUS` 由 AMS7997 的 `/YKS-AABAABESE120-1PL55` 弯头钉住：它的
    /// `PTAX.PDIS=[40,2,-3,-4]` 是 `TANF DDRADIUS DDANGLE`，而设计 BEND 明确存有
    /// `RADI=134, ANGL=90`。Core 用这两个设计维度定位 P1/P2；漏掉半径会让明明
    /// 存在的 `SCTO.PAAX=P1` 被误报成缺轴。
    ///
    /// `DDHEIGHT` 由 ams251142 的变长桥架直段钉住：`CTRAY 2013287686/11`（`HEIG=2955`）
    /// 的 `SPRE → /SW5/SL3/750/V` 两根侧轨 `SEXT.PHEI=[40,1,-5,0]`，即 `DDHEIGHT`；
    /// 同一 GMSE 的另一根侧轨 `PZ=( ATTRIB RPRO DISP - ATTRIB HEIG )` 明写读设计构件
    /// 的 `HEIG`，两条对得上——定长的 `/SW5/SL3/750` 这两格都是 `PARA[10]`（3000）。
    /// Core3D `pplib/DECARG` 把操作数 `-5` 直接交 `HEIG`（见 e3d-model
    /// `evidence/20260902-hvac-ctray-census/README.md` 的操作数表）。
    fn design_dimension(&self, id: i32) -> Option<f64> {
        match id {
            DESIGN_RADIUS => self.number("RADI"),
            DESIGN_ANGLE => self.number("ANGL"),
            DESIGN_HEIGHT => self.number("HEIG"),
            _ => None,
        }
    }

    fn attribute_number(&self, attribute: &AttribRef<'_>) -> Option<f64> {
        // `ATTRIB NAMN OF PSPE OF PIPE`：属主链怎么走没定死（连接词那几个词现在
        // 读不出，见 catalogue_pml 的 parse_owner_chain）。P 点的公式一条都没走
        // 到这儿，先不猜。
        if !attribute.owner_chain.is_empty() {
            return None;
        }
        match (attribute.subscript, attribute.qualifier) {
            // 两个同时出现的形态没见过，也没有约定的读法。
            (Some(_), Some(_)) => None,
            (Some(subscript), None) => {
                // 下标从栈上取，本来就是数；只有整数才是下标。
                let index = match subscript.fract() == 0.0 {
                    true => subscript as i32,
                    false => return None,
                };
                // PML writes the insulation form as `ATTRIB IPAR[n]`, while
                // the compact catalogue tuple writes `IPARAM n`.  They both
                // reach Core GTGETP's special IPAR branch, including its
                // no-insulation/out-of-range numeric-zero rule; routing the
                // former through ordinary array lookup incorrectly produced
                // `None` for the same expression.
                if attribute.name == "IPAR" {
                    self.iparam(index)
                } else if attribute.name == ParamArray::Desp.attribute() {
                    self.design_parameter(index)
                } else if attribute.name == NUMERIC_DESIGN_PARAMS {
                    self.numeric_design_parameter(index)
                } else if attribute.name == CATALOGUE_PARAMS {
                    // DESPRE：当前元素的 `PARA`，读不到再去目录件上读——就是 `hosts()`
                    // 的顺序，所以直接按 `PARA` 走同一条索引路。
                    self.indexed(CATALOGUE_PARAM_ARRAY, index)
                } else {
                    self.indexed(attribute.name, index)
                }
            }
            (None, Some(qualifier)) => match attribute.name {
                DESIGN_PROPERTY => self.design_property(qualifier),
                reference => self.through_reference(reference, qualifier),
            },
            (None, None) => self.number(attribute.name),
        }
    }

    /// `( :HXYsize[2] )`——按目录写下的键读一个 UDA，经宿主所在 `DbSet` 挂的字典
    /// （[`DbElement::uda_by_key`]）。宿主顺序同 [`Self::hosts`]：先设计构件再目录件。
    ///
    /// 一个宿主的字典**认**这个键（对它的 noun 有效）就到此为止：有值取值，unset 就是
    /// 答不上——不再去另一边找同键的值，那是另一个零件的尺寸。只有字典对这个宿主
    /// 说「不认识 / 无效」才换下一个宿主。哪个宿主都不认（没挂字典、挂的字典里没有
    /// 这个键）就是 `None`，公式按未解报出去，不猜。
    ///
    /// ams7997 的风管头槽位就停在「unset」这一格：`/HRTUBE1-BOXI.PXLE` 写的键是
    /// `AvevaCatalogue` 字典（acp7006）那条 `HXYsize` 的 `OLDKEY`，BRAN 上的 `675 600`
    /// 却存在本项目字典（ams5100）那条同名定义的键下——两条定义、一个名字，目录指的
    /// 是设计元素没存值的那条。E3D 对这些槽位报的 `unable to evaluate … PXLE` 正是
    /// 同一次落空（`catalogue_pml` 模块头）。
    fn uda_number(&self, key: u32, subscript: Option<f64>) -> Option<f64> {
        for host in self.hosts() {
            let value = match host.uda_by_key(key) {
                Ok(Some(value)) => value,
                Ok(None) => return None,
                Err(_) => continue,
            };
            return uda_value_number(&value, subscript);
        }
        None
    }
}

/// 一个 UDA 值读成数：标量直接给（`[1]` 也算是它自己，与 `ATTRIB PARA` 写成 `[1, 1]`
/// 的那条回落同口径），数组按 1 起的下标取；文本、逻辑、引用都不是数。
fn uda_value_number(value: &DescriptorValue, subscript: Option<f64>) -> Option<f64> {
    let index = match subscript {
        None => None,
        Some(subscript) if subscript.fract() == 0.0 && subscript >= 1.0 => {
            Some(subscript as usize - 1)
        }
        Some(_) => return None,
    };
    match (value, index) {
        (DescriptorValue::Real(real), None | Some(0)) => Some(*real),
        (DescriptorValue::Int(int), None | Some(0)) => Some(f64::from(*int)),
        (DescriptorValue::RealArray(items), Some(index)) => items.get(index).copied(),
        (DescriptorValue::IntArray(items), Some(index)) => {
            items.get(index).map(|item| f64::from(*item))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没有宿主时，每一条路都得答「不知道」。
    ///
    /// 这条钉的是本模块唯一真正危险的失效模式：答不上的时候返回 0。0 是一个
    /// 合法的距离、一个合法的通径，下游分不出它是「这里就是 0」还是「没问出
    /// 来」，几何照样建、报告照样绿，错要到看图的人觉得哪里不对时才发现。
    #[test]
    fn an_empty_environment_answers_nothing_rather_than_zero() {
        let env = DbElementParamEnv::new(None, None);

        assert_eq!(env.param(1), None);
        assert_eq!(env.iparam(1), None);
        assert_eq!(env.design_dimension(3), None);
        assert_eq!(env.attribute_number(&AttribRef::named("ANGL")), None);
        assert_eq!(
            env.attribute_number(&AttribRef::subscripted("PARA", 1.0)),
            None
        );
        assert_eq!(
            env.attribute_number(&AttribRef {
                name: DESIGN_PROPERTY,
                subscript: None,
                qualifier: Some("TLEN"),
                owner_chain: &[],
            }),
            None
        );
    }

    /// `PARAM n` 是 1 起的，0 和负数不是「第一个」。
    ///
    /// `GATPAR` 越界报 223 号错。这里没有目录件可问，但下标先于取值算：
    /// 0 与负数在拿到数组之前就该被判掉，不该悄悄折成 `PARA[1]`。
    #[test]
    fn a_parameter_index_below_one_is_out_of_range() {
        let env = DbElementParamEnv::new(None, None);
        assert_eq!(env.param(0), None);
        assert_eq!(env.param(-1), None);
    }

    /// Core3D `ANFIND` widens both range ends by exactly 0.001.  This is
    /// important at nominal catalogue boundaries such as PBOR 100/100.001.
    #[test]
    fn selection_ranges_use_core_anfinds_inclusive_tolerance() {
        assert!(selection_range_contains(15.0, 100.0, 14.999));
        assert!(selection_range_contains(15.0, 100.0, 100.001));
        assert!(!selection_range_contains(15.0, 100.0, 14.9989));
        assert!(!selection_range_contains(15.0, 100.0, 100.0011));
    }

    /// `CPAR` 走的是 `PARA` 那条索引路：没有宿主时同样答不上，而不是 0。
    /// 越界与非整数下标沿用 `PARA` 的判法（下面两条测试）。
    #[test]
    fn catalogue_params_without_hosts_answer_nothing() {
        let env = DbElementParamEnv::new(None, None);
        assert_eq!(
            env.attribute_number(&AttribRef::subscripted(CATALOGUE_PARAMS, 5.0)),
            None
        );
        assert_eq!(
            env.attribute_number(&AttribRef::subscripted(CATALOGUE_PARAMS, 0.0)),
            None
        );
    }

    /// 下标得是整数。`PARA[1.5]` 没有读法，截断成 `PARA[1]` 是编答案。
    #[test]
    fn a_fractional_subscript_is_not_an_index() {
        let env = DbElementParamEnv::new(None, None);
        assert_eq!(
            env.attribute_number(&AttribRef::subscripted("PARA", 1.5)),
            None
        );
    }

    /// 属主链形态（`ATTRIB NAMN OF PSPE OF PIPE`）还没取证，先一律不答。
    #[test]
    fn an_owner_chain_is_declined_whole() {
        let env = DbElementParamEnv::new(None, None);
        let owners = ["PSPE", "PIPE"];
        assert_eq!(
            env.attribute_number(&AttribRef {
                name: "NAMN",
                subscript: None,
                qualifier: None,
                owner_chain: &owners,
            }),
            None
        );
    }

    /// 没有宿主就没有字典，按键读 UDA 也得答「不知道」，不是 0。
    #[test]
    fn a_uda_key_without_hosts_answers_nothing() {
        let env = DbElementParamEnv::new(None, None);
        assert_eq!(env.uda_number(0x2652_AB52, Some(2.0)), None);
        assert_eq!(env.uda_number(0x2652_AB52, None), None);
    }

    /// UDA 值读成数的口径：标量整体（`[1]` 也是它自己），数组按 1 起下标，越界与
    /// 非整数下标、非数值形态一律答不上。
    #[test]
    fn a_uda_value_reads_as_a_number_only_where_it_is_one() {
        let sizes = DescriptorValue::RealArray(vec![675.0, 600.0]);
        assert_eq!(uda_value_number(&sizes, Some(1.0)), Some(675.0));
        assert_eq!(uda_value_number(&sizes, Some(2.0)), Some(600.0));
        assert_eq!(uda_value_number(&sizes, Some(3.0)), None);
        assert_eq!(uda_value_number(&sizes, Some(0.0)), None);
        assert_eq!(uda_value_number(&sizes, Some(1.5)), None);
        assert_eq!(uda_value_number(&sizes, None), None, "an array is not a number");
        let scalar = DescriptorValue::Real(2800.0);
        assert_eq!(uda_value_number(&scalar, None), Some(2800.0));
        assert_eq!(uda_value_number(&scalar, Some(1.0)), Some(2800.0));
        assert_eq!(uda_value_number(&scalar, Some(2.0)), None);
        assert_eq!(uda_value_number(&DescriptorValue::Int(7), None), Some(7.0));
        assert_eq!(
            uda_value_number(&DescriptorValue::IntArray(vec![3, 4]), Some(2.0)),
            Some(4.0)
        );
        assert_eq!(uda_value_number(&DescriptorValue::Text("HP".into()), None), None);
        assert_eq!(uda_value_number(&DescriptorValue::Bool(true), None), None);
    }

    /// 深度闸关上之后，再深的一层直接答不上，而不是继续往下钻。
    #[test]
    fn the_depth_guard_closes_at_the_limit() {
        let env = DbElementParamEnv::new(None, None);
        env.depth.set(MAX_DEPTH);
        assert_eq!(env.nested(|| Some(1.0)), None);
        // 闸关掉之后深度不该被这次尝试改掉。
        assert_eq!(env.depth.get(), MAX_DEPTH);

        env.depth.set(MAX_DEPTH - 1);
        assert_eq!(env.nested(|| Some(1.0)), Some(1.0));
        assert_eq!(env.depth.get(), MAX_DEPTH - 1);
    }
}
