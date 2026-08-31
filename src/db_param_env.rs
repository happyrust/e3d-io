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

use std::cell::Cell;

use crate::db_element::DbElement;
use crate::record::catalogue_eval::{AttribRef, AttributeNumber, ParamEnv};
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

/// `DDANGLE` 的设计维度编号。见 [`ParamEnv::design_dimension`] 的编号约定。
const DESIGN_ANGLE: i32 = 4;

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
            depth: Cell::new(0),
        }
    }

    /// 只有设计构件的环境。`PARAM n` 与 `ATTRIB PARA[i]` 会答不上。
    pub fn design_only(design: DbElement) -> Self {
        Self::new(Some(design), None)
    }

    /// 只有目录件的环境。`ATTRIB ANGL` 这类设计侧的名字会答不上。
    pub fn catalogue_only(catalogue: DbElement) -> Self {
        Self::new(None, Some(catalogue))
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
    /// **先认 `PPRO`，再认 `PURP`。** 这一行的 `PPRO` 往往把 `PURP` + `NUMB`
    /// 指的东西直接写成公式：`RADI` 行是 `PURP=DESP NUMB=1` 且
    /// `PPRO = ( ATTRIB DESP[1 ] )`，`CTLF` 行是 `DESP` / `6` 且
    /// `( ATTRIB DESP[6 ] )`，`APAR` 行是 `DATA` / `1` 且 `( ATTRIB PARA[1 ] )`
    /// ——三行互相印证，也正是这三行让「`NUMB` 就是下标」这条读法立得住。既然
    /// 目录自己写了公式，就用它的，别用我从三行样本里归纳出来的映射。
    /// `PURP` 那条路留给 `PPRO` 是空的那些行（`HEIG` 行就是 `PURP=ATTR` 而
    /// `PPRO` 空着）。
    fn design_property_row(&self, row: &DbElement, key: &str) -> Option<f64> {
        // `PPRO` 挂在这一行上，但要在**外层**环境里求：它写的 `PARA[1]` 是目录件
        // 的参数、`DESP[1]` 是设计构件的参数，都不是这一行自己的。
        let stated = self.nested(|| match row.get_number("PPRO", self) {
            Ok(number) => number.value(),
            Err(_) => None,
        });
        if let Some(value) = stated {
            return Some(value);
        }

        match word_text(row, "PURP")?.as_str() {
            purpose::CATALOGUE_PARAM => self.param(whole_number(row, "NUMB")?),
            purpose::DESIGN_PARAM => {
                let index = usize::try_from(whole_number(row, "NUMB")?)
                    .ok()?
                    .checked_sub(1)?;
                match self.design.as_ref()?.attribute("DESP") {
                    Ok(Some(DescriptorValue::RealArray(items))) => items.get(index).copied(),
                    _ => None,
                }
            }
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
    fn param(&self, n: i32) -> Option<f64> {
        let index = usize::try_from(n).ok()?.checked_sub(1)?;
        let catalogue = self.catalogue.as_ref()?;
        match catalogue.attribute("PARA") {
            Ok(Some(DescriptorValue::RealArray(items))) => items.get(index).copied(),
            _ => None,
        }
    }

    /// `IPARAM n` —— 保温参数。
    ///
    /// **还没接。** ams8000 的 108 个目录件身上一个 `IPAR` 都没有，路由构件的
    /// `ISPE`（保温规格引用）实测也全是 Nulref，所以本库里没有一条表达式走得到
    /// 这里。等有带保温的语料再按它取证；现在返回 `None` 而不是 0——0 会让保温层
    /// 厚度静默变成没有。
    fn iparam(&self, _n: i32) -> Option<f64> {
        None
    }

    /// 设计维度关键字（`DDRADIUS` = 3、`DDANGLE` = 4、`DDHEIGHT` = 5）。
    ///
    /// **只接了 `DDANGLE`，因为只有它拿得到验证。** 它出现在 529 个 `PTAX` 的
    /// `PAXI` 上（`X DDANGLE Y` 152 处、`X DDANGLE -Y` 377 处），落在弯头这类
    /// 有转角的构件上，而弯头两端 P 点轴的夹角**必须等于**设计构件的 `ANGL`
    /// ——这是一条能在真库上逐条核的等式，不是一个说得通的猜测。
    /// [`crate::record::axis_spec::direction`] 求出的轴按这条对过账。
    ///
    /// `DDRADIUS` / `DDHEIGHT` 仍不接：ams8000 一处都没有，接了既无从验证，
    /// 又会在将来某份语料上悄悄生效。
    fn design_dimension(&self, id: i32) -> Option<f64> {
        match id {
            DESIGN_ANGLE => self.number("ANGL"),
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
                self.indexed(attribute.name, index)
            }
            (None, Some(qualifier)) => match attribute.name {
                DESIGN_PROPERTY => self.design_property(qualifier),
                reference => self.through_reference(reference, qualifier),
            },
            (None, None) => self.number(attribute.name),
        }
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
