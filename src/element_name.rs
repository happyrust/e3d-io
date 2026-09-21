//! E3D 给元素显示的名字：存了 `NAME` 就是它；没存，就按元素在树里的位置现拼一个。
//!
//! 这是伪属性 `FLNM`，复刻自 `core.dll` 的 `dbgenlib/ELNAME` + `dbgenlib/LNPART`
//! （E3D 3.1，IDA 反编译）。`ELNAME` 按请求的伪属性分支，`FLNM`（hash 797856）这一支是：
//!
//! ```text
//! 元素自己有名字  → 就是 NAME，不带名词      /C-IY-1R330-B
//! 元素自己没名字  → LNPART 一转写一段，BRETUR 换成 OWNER 再来一转，
//!                   直到某一转落在有名字的祖先上
//! ```
//!
//! `LNPART` 每转写的那一段是 `<名词全词> + " " + (有名字 ? <名字> —— 收尾 : <序号> + " of " —— 继续走 OWNER)`，
//! 于是嵌套形态自然长成（前两条是 TTY 实录，见 `tests/fixtures/`）：
//!
//! ```text
//! WELD 1 of BRANCH /1RCV0101-60.3-VMB-R90-R332]R333
//! NSCYLINDER 1 of SCYLINDER 1 of GMSET /GRZGG00-G3-8
//! SITE 2                                  ← 链不上到库根：WORLD 不进名字
//! ```
//!
//! 一个元素给两种形态（[`ElementName`]）：
//!
//! * `short`——模型树一行里显示的那一句：有名 = NAME；无名 = `SITE 2` / `ZONE 4` / `BEND 3`。
//! * `full`——`FLNM` 整条：有名 = NAME；无名 = `ZONE 4 of SITE 2`、`BEND 3 of BRANCH /C-IY-1R330-B`。
//!
//! 从二进制里读出来、写代码时最容易做错的几处：
//!
//! 1. **名词是全词**。`LNPART` 走 `ATNTXT(noun, NAME)`，而 `DB_Noun::getField(NAME)` 与
//!    `DB_Noun::fullName()` 是同一段代码，给的是 `BRANCH` / `EQUIPMENT` / `TMPLATE`，不是库里
//!    存的 `BRAN` / `EQUI` / `TMPL`。全词从 [`noun_full_name`] 查。
//! 2. **分隔符是小写 ` of `**。`LNPART` 一个字符一个字符写：`0x20, 'o', 'f', 0x20`。
//! 3. **序号是同名词兄弟里的第几个**（`ITYPOS` 沿成员链只数 `actualType` 相同的），从 1 起，
//!    有名字的兄弟也占号——不是「在属主全部成员里的下标」，也不是「第几个无名兄弟」。
//!    TTY 样本里 `PTAXIS 1 of PTSET /MSIDBHP-P4-26` 是属主的第一个成员也是第一个 `PTAX`，
//!    而 `PTCAR 2 of PTSET /ANRC900-P3-1` 是第三个成员、第二个 `PTCA`。
//! 4. **收尾那一截带名词，元素自己那一截（有名时）不带**：`LNPART` 每转都先写名词，而元素
//!    自己有名字时 `ELNAME` 根本不进 `LNPART`。
//! 5. **名字不以 `/` 开头时 `LNPART` 自己补一个**。
//! 6. **`IDSPU` 有值时序号位置写 `idsp <值>`**（`GATINT(IDSPU)` 取到值就走这一支）。直读
//!    填不了它——`IDSPU` 不在任何 noun 的属性表里，是 spooling 模块的插件属性；规则照抄在
//!    [`NameLink::idspu`] 上，等能取到它的调用方。
//! 7. **链不上到库根**。WORLD 不进名字：无名 SITE 就叫 `SITE 10`，不是 `SITE 10 of WORLD /*XXX`
//!    （2026-09-15 按 E3D 实际显示定的口径）。「库根」按结构认——没有 OWNER 的那一级——不看它
//!    叫什么。
//!
//! 这里是这套规则在读侧的**唯一**实现：gen-model 的 `/tree/*`、`/element/attributes` 经
//! [`DbElement::display_name`] / [`DbElement::member_names`] 取；`e3d-descriptor` 的 `Name` 行经
//! [`display_name`]。plant-ui 库供数那一面（`aios_core::e3d_full_name`、SurrealQL
//! `fn::default_full_name`）不经 e3d-io，靠双面对拍探针逐节点比 `name` 守一致。

use std::collections::HashMap;
use std::sync::OnceLock;

use e3d_attlib::{db1_dehash, db1_hash};

use crate::db_element::{noun_name, DbElement, DbElementError};
use crate::noun_catalog::NounCatalog;
use crate::record::element::ParsedElement;
use crate::record::explicit::ExplicitValue;
use crate::refno::RefNo;
use crate::ReadOnlyEngine;

/// 短名词 → 名词全词，只收两者不同的那些（`BRAN` → `BRANCH`）。
///
/// 由 gen-model `scripts/gen_noun_full_names.py` 从活 E3D 导出的 `noun_layout.json`
/// （每条同时带 `short` 与 `noun`，即 `schema/e3d31/catalog/noun_layout.json`）生成。
/// 全表 4.3 MB 太大，短名词与全词相同的那 1418 条也没有查表的必要——查不到就退回短名词，
/// 正是对的答案。
const NOUN_FULL_NAMES_JSON: &str = include_str!("element_name/noun_full_names.json");

fn noun_full_names() -> &'static HashMap<String, String> {
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(NOUN_FULL_NAMES_JSON)
            .expect("element_name/noun_full_names.json is the table this crate ships")
    })
}

/// 短名词的名词全词：`BRAN` → `BRANCH`，`SITE` → `SITE`，未登记的原样返回。
pub fn noun_full_name(noun: &str) -> &str {
    let noun = noun.trim();
    noun_full_names()
        .get(noun)
        .map(String::as_str)
        .unwrap_or(noun)
}

/// `FLNM` 名字链上的一级：元素自己、它的 OWNER、OWNER 的 OWNER……
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NameLink {
    /// 库里存的短名词（`BRAN`）。拼名字时由 [`noun_full_name`] 换成全词。
    pub noun: String,
    /// 存着的 `NAME`，去掉首尾空白；无名元素是空串。
    pub name: String,
    /// 在属主的**同名词**成员里排第几（`ITYPOS`，从 1 起）。有名字的那一级用不上。
    pub ordinal: usize,
    /// 这一级是库根（没有 OWNER 的那个元素，DESI 里就是 WORLD）。库根不进名字，
    /// 链碰到它就收手。
    pub is_root: bool,
    /// `IDSPU`：有值时序号位置写 `idsp <值>`。见模块头第 6 条。
    pub idspu: Option<i64>,
}

impl NameLink {
    pub fn named(noun: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            noun: noun.into(),
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn unnamed(noun: impl Into<String>, ordinal: usize) -> Self {
        Self {
            noun: noun.into(),
            ordinal,
            ..Default::default()
        }
    }

    /// 库根：只用来让链在这里收手，本身不写进名字。
    pub fn root() -> Self {
        Self {
            is_root: true,
            ..Default::default()
        }
    }

    fn is_named(&self) -> bool {
        !self.name.is_empty()
    }

    /// 序号那一格写什么：`IDSPU` 有值就是 `idsp <值>`，否则是同名词序号。
    fn ordinal_text(&self) -> String {
        match self.idspu {
            Some(idspu) => format!("idsp {idspu}"),
            None => self.ordinal.to_string(),
        }
    }

    /// 无名这一级自己那一段：`<名词全词> <序号>`。
    fn head(&self) -> String {
        format!("{} {}", noun_full_name(&self.noun), self.ordinal_text())
    }
}

/// E3D 给一个元素显示的名字，两种形态一起给。
///
/// 有名字的元素三个字段是同一串（就是 NAME）；无名元素 `stored` 是 `None`，
/// `short` 是模型树一行显示的 `SITE 2`，`full` 是整条 `FLNM`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementName {
    /// 文件里存的 `NAME`（去首尾空白）；无名元素是 `None`。
    pub stored: Option<String>,
    /// 短形态：有名 = NAME；无名 = `<名词全词> <同名词序号>`，如 `SITE 2`、`BEND 3`。
    pub short: String,
    /// 全形态（`FLNM`）：有名 = NAME；无名 = `<短形态> of <属主链尾巴>`，
    /// 如 `BEND 3 of BRANCH /C-IY-1R330-B`；属主链直接顶到库根时与 `short` 相同。
    pub full: String,
}

impl ElementName {
    /// 有名字的元素：三个字段都是这一串。
    pub fn named(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            stored: Some(name.clone()),
            short: name.clone(),
            full: name,
        }
    }

    /// 无名元素：`noun` 是短名词，`ordinal` 是同名词序号，`tail` 是属主链拼好的尾巴
    /// （[`owner_tail`]，同一批兄弟共用一条；直接挂在库根下时是空串）。
    pub fn unnamed(noun: &str, ordinal: usize, tail: &str) -> Self {
        Self::from_chain_head(&NameLink::unnamed(noun, ordinal), tail)
    }

    /// `chain[0]` 是元素自己，其后是 OWNER 链（到第一个有名字的祖先、或到库根）。
    pub fn from_chain(chain: &[NameLink]) -> Self {
        let Some(element) = chain.first() else {
            return Self {
                stored: None,
                short: String::new(),
                full: String::new(),
            };
        };
        if element.is_named() {
            return Self::named(element.name.clone());
        }
        Self::from_chain_head(element, &owner_tail(&chain[1..]))
    }

    fn from_chain_head(element: &NameLink, tail: &str) -> Self {
        let short = element.head();
        let full = if tail.is_empty() {
            short.clone()
        } else {
            format!("{short} of {tail}")
        };
        Self {
            stored: None,
            short,
            full,
        }
    }

    pub fn is_named(&self) -> bool {
        self.stored.is_some()
    }
}

/// `FLNM`：`chain[0]` 是元素自己，其后是 OWNER 链，直到第一个有名字的祖先。
///
/// 就是 [`ElementName::from_chain`] 的 `full`。链走到头都没有名字（顶到库根还是无名，
/// 或者调用方截短了链）时，最后一段的 ` of ` 尾巴会被去掉——E3D 到不了这一步，
/// 这里只是不让残缺的链拼出一个以 ` of ` 结尾的名字。
pub fn full_name(chain: &[NameLink]) -> String {
    ElementName::from_chain(chain).full
}

/// 一条属主链走完 `LNPART`：每一级都先写名词全词，落在有名字的那级收尾，碰到库根就停。
///
/// 喂属主链进来就得到同一批兄弟共用的尾巴：`BRANCH /C-IY-1R330-B`、
/// `TMPLATE 1 of EQUIPMENT /03SKID3-EQUIP1`；属主就是库根时是空串。
pub fn owner_tail(chain: &[NameLink]) -> String {
    let mut out = String::new();
    for link in chain {
        if link.is_root {
            break;
        }
        out.push_str(noun_full_name(&link.noun));
        out.push(' ');
        if link.is_named() {
            out.push_str(&slashed(&link.name));
            return out;
        }
        out.push_str(&link.ordinal_text());
        out.push_str(" of ");
    }
    out.truncate(out.trim_end_matches(" of ").len());
    out
}

/// 无名元素的默认全名，属主那条尾巴已经拼好时用（同一批兄弟共用一条尾巴）。
pub fn with_tail(noun: &str, ordinal: usize, tail: &str) -> String {
    ElementName::unnamed(noun, ordinal, tail).full
}

/// 无名元素的短形态：`<名词全词> <序号>`。
pub fn short_name(noun: &str, ordinal: usize) -> String {
    ElementName::unnamed(noun, ordinal, "").short
}

/// `LNPART` 写名字前会看第一个字符：不是 `/` 就补一个。
fn slashed(name: &str) -> String {
    if name.starts_with('/') {
        name.to_string()
    } else {
        format!("/{name}")
    }
}

/// 文件里存的 `NAME` 原文；没存、或存的全是空白，就是 `None`。
pub fn stored_name(parsed: &ParsedElement) -> Option<String> {
    let name_hash = db1_hash("NAME");
    parsed
        .explicit_attributes
        .iter()
        .rev()
        .find(|attribute| attribute.hash == name_hash)
        .and_then(|attribute| match &attribute.value {
            ExplicitValue::Text(text) if !text.trim().is_empty() => Some(text.clone()),
            _ => None,
        })
}

/// 拼名字用的 `NAME`：去掉首尾空白。`FLNM` 里名字前后不该带着存储时的空白。
fn naming_stored_name(parsed: &ParsedElement) -> Option<String> {
    stored_name(parsed).map(|name| name.trim().to_string())
}

// ---- DbElement 门面：gen-model 的树与属性接口走这里 -----------------------------

/// 一个元素的显示名（[`DbElement::display_name`]）。
///
/// 元素自己一次记录读；无名时再读属主链（属主有名字——绝大多数情况——就只有一级），
/// 并沿属主的成员表数一次同名词序号。整批兄弟要名字用 [`member_names`]，
/// 别在循环里逐个调这条：尾巴与序号那两样东西整批是共用的。
pub(crate) fn element_name(element: &DbElement) -> Result<ElementName, DbElementError> {
    let parsed = element.parsed()?;
    if let Some(name) = naming_stored_name(&parsed) {
        return Ok(ElementName::named(name));
    }
    let noun = noun_name(parsed.noun_hash);
    let Some(owner) = element.owner()? else {
        // 自己就是库根还没名字（E3D 里不会有）：没有兄弟可数，链上也没有别人。
        return Ok(ElementName::unnamed(&noun, 1, ""));
    };
    let ordinal = type_ordinal(&owner, element.refno(), parsed.noun_hash)?;
    let tail = owner_tail(&owner_links(&owner)?);
    Ok(ElementName::unnamed(&noun, ordinal, &tail))
}

/// 一个属主的全部成员连同各自的显示名，文件原序（[`DbElement::member_names`]）。
///
/// 按属主成批算而不是逐个元素算，是因为两样东西整批共用：同名词序号要沿整条成员链数
/// （逐个算就是 O(n²) 次记录读），属主那条尾巴（`… of EQUIPMENT /03SKID3-EQUIP1`）对
/// 同一批兄弟又完全相同。每个成员一次记录读——列树本来就要读它的 noun 与 NAME；
/// 全员有名字时一次属主链读也不做。
pub(crate) fn member_names(
    owner: &DbElement,
) -> Result<Vec<(DbElement, ElementName)>, DbElementError> {
    let mut out = Vec::new();
    let mut ordinals: HashMap<u32, usize> = HashMap::new();
    let mut tail: Option<String> = None;
    for member in owner.members()? {
        // 成员表里的 Nulref 占位不是元素：没有 noun 可数，也没有名字可给。
        if member.is_null() {
            continue;
        }
        let parsed = member.parsed()?;
        // `ITYPOS` 数的是同名词兄弟，命名与否都占一个号。
        let ordinal = {
            let slot = ordinals.entry(parsed.noun_hash).or_insert(0);
            *slot += 1;
            *slot
        };
        let name = match naming_stored_name(&parsed) {
            Some(name) => ElementName::named(name),
            None => {
                if tail.is_none() {
                    tail = Some(owner_tail(&owner_links(owner)?));
                }
                ElementName::unnamed(
                    &noun_name(parsed.noun_hash),
                    ordinal,
                    tail.as_deref().unwrap_or_default(),
                )
            }
        };
        out.push((member, name));
    }
    Ok(out)
}

/// `owner` 起的 OWNER 链，到第一个有名字的祖先、或到库根为止——`FLNM` 的尾巴。
///
/// 每一级先看它有没有 OWNER：没有就是库根，**不看名字**就收手（WORLD 有名字 `/*XXX`，
/// 但不进名字）。有 OWNER 而有名字的收尾；无名的为它在属主成员表里数一次同名词序号，
/// 再往上走。链转回头是坏文件，报 [`DbElementError::OwnerCycle`]，不猜。
fn owner_links(owner: &DbElement) -> Result<Vec<NameLink>, DbElementError> {
    let mut links = Vec::new();
    let mut visited: Vec<RefNo> = Vec::new();
    let mut current = owner.clone();
    loop {
        if visited.contains(&current.refno()) {
            return Err(DbElementError::OwnerCycle {
                start: owner.refno(),
                at: current.refno(),
            });
        }
        visited.push(current.refno());
        let parsed = current.parsed()?;
        let Some(next) = current.owner()? else {
            links.push(NameLink::root());
            break;
        };
        let noun = noun_name(parsed.noun_hash);
        if let Some(name) = naming_stored_name(&parsed) {
            links.push(NameLink::named(noun, name));
            break;
        }
        let ordinal = type_ordinal(&next, current.refno(), parsed.noun_hash)?;
        links.push(NameLink::unnamed(noun, ordinal));
        current = next;
    }
    Ok(links)
}

/// `ITYPOS`：`child` 在 `owner` 的同名词成员里排第几，从 1 起。
///
/// `child` 不在成员表里是数据不一致，这里给的是「排在全部同名词成员之后」；
/// 导航面上的 [`DbElement::next`] 对同一情形报错，名字这一路宁可给一个能显示的。
fn type_ordinal(owner: &DbElement, child: RefNo, noun_hash: u32) -> Result<usize, DbElementError> {
    let mut ordinal = 1;
    for member in owner.members()? {
        if member.refno() == child {
            break;
        }
        if !member.is_null() && member.noun_hash()? == noun_hash {
            ordinal += 1;
        }
    }
    Ok(ordinal)
}

// ---- ReadOnlyEngine 一路：`e3d-descriptor` 的 `Name` 行 -------------------------

/// 上溯最多走这么多级。样本里的嵌套远不到这个数；上限只是让坏文件里的环不至于挂住调用方。
const MAX_DEPTH: usize = 32;

/// 单库引擎上的同一条规则，给 `e3d-descriptor` 用：读不到元素、或链在坏文件里转回头时
/// 是 `None`——一个残缺的名字比承认没有答案更糟。
///
/// 反哈希不出的 noun（UDET）从 `catalog` 取短名；两条路名词全词都经 [`noun_full_name`]。
pub fn display_name(
    engine: &mut ReadOnlyEngine,
    catalog: &NounCatalog,
    refno: RefNo,
) -> Option<String> {
    Some(ElementName::from_chain(&engine_chain(engine, Some(catalog), refno)?).full)
}

fn engine_chain(
    engine: &mut ReadOnlyEngine,
    catalog: Option<&NounCatalog>,
    refno: RefNo,
) -> Option<Vec<NameLink>> {
    let parsed = read(engine, refno)?;
    let noun = noun_word(parsed.noun_hash, catalog);
    if let Some(name) = naming_stored_name(&parsed) {
        return Some(vec![NameLink::named(noun, name)]);
    }
    if is_null(parsed.owner) {
        return Some(vec![NameLink::unnamed(noun, 1)]);
    }
    let ordinal = engine_type_ordinal(engine, parsed.owner, refno, parsed.noun_hash)?;
    let mut chain = vec![NameLink::unnamed(noun, ordinal)];

    let mut visited = vec![refno];
    let mut current = parsed.owner;
    loop {
        if visited.len() >= MAX_DEPTH || visited.contains(&current) {
            return None;
        }
        visited.push(current);
        let parsed = read(engine, current)?;
        if is_null(parsed.owner) {
            chain.push(NameLink::root());
            break;
        }
        let noun = noun_word(parsed.noun_hash, catalog);
        if let Some(name) = naming_stored_name(&parsed) {
            chain.push(NameLink::named(noun, name));
            break;
        }
        let ordinal = engine_type_ordinal(engine, parsed.owner, current, parsed.noun_hash)?;
        chain.push(NameLink::unnamed(noun, ordinal));
        current = parsed.owner;
    }
    Some(chain)
}

fn engine_type_ordinal(
    engine: &mut ReadOnlyEngine,
    owner: RefNo,
    child: RefNo,
    noun_hash: u32,
) -> Option<usize> {
    let members = read(engine, owner)?.members;
    let mut ordinal = 1;
    for member in members {
        if member == child {
            break;
        }
        if !is_null(member) && read(engine, member)?.noun_hash == noun_hash {
            ordinal += 1;
        }
    }
    Some(ordinal)
}

/// 短名词：反哈希得到就是它；反不出（UDET）从目录取短名；都没有印十六进制，不编一个。
fn noun_word(noun_hash: u32, catalog: Option<&NounCatalog>) -> String {
    if let Some(short) = db1_dehash(noun_hash) {
        return short;
    }
    catalog
        .and_then(|catalog| catalog.noun(noun_hash))
        .map(|noun| {
            if noun.short_name.is_empty() {
                noun.noun.clone()
            } else {
                noun.short_name.clone()
            }
        })
        .filter(|noun| !noun.is_empty())
        .unwrap_or_else(|| noun_name(noun_hash))
}

fn is_null(refno: RefNo) -> bool {
    !refno.is_valid()
}

fn read(engine: &mut ReadOnlyEngine, refno: RefNo) -> Option<ParsedElement> {
    let view = engine.find_element(refno).ok()??;
    ParsedElement::parse_as(&view.raw_bytes, engine.text_format()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表里收的是两者不同的那些；相同的（SITE/ZONE/PIPE…）查不到，退回短名词即可。
    #[test]
    fn short_nouns_expand_to_the_words_e3d_prints() {
        assert_eq!(noun_full_name("BRAN"), "BRANCH");
        assert_eq!(noun_full_name("EQUI"), "EQUIPMENT");
        assert_eq!(noun_full_name("TMPL"), "TMPLATE");
        assert_eq!(noun_full_name("WORL"), "WORLD");
        assert_eq!(noun_full_name("SUPPO"), "SUPPORT");
        assert_eq!(noun_full_name("SITE"), "SITE");
        assert_eq!(noun_full_name("SNOU"), "SNOUT");
        assert_eq!(noun_full_name(":SDS_Pole"), ":SDS_Pole");
    }

    /// 有名字的元素只给名字——名词是 `LNPART` 写的，而 `ELNAME` 这一支根本不进 `LNPART`。
    #[test]
    fn a_named_element_is_just_its_name_in_both_forms() {
        let chain = [NameLink::named("BRAN", "/C-IY-1R330-B")];
        let name = ElementName::from_chain(&chain);
        assert_eq!(name, ElementName::named("/C-IY-1R330-B"));
        assert!(name.is_named());
        assert_eq!(full_name(&chain), "/C-IY-1R330-B");
    }

    /// `core.dll` 的两个实测形态：一层与嵌套两层，收尾的祖先都带名词。
    #[test]
    fn unnamed_elements_chain_up_to_the_first_named_ancestor() {
        let simple = [
            NameLink::unnamed("WELD", 1),
            NameLink::named("BRAN", "/1RCV0101-60.3-VMB-R90-R332]R333"),
        ];
        let name = ElementName::from_chain(&simple);
        assert_eq!(name.stored, None);
        assert_eq!(name.short, "WELD 1");
        assert_eq!(
            name.full,
            "WELD 1 of BRANCH /1RCV0101-60.3-VMB-R90-R332]R333"
        );

        let nested = [
            NameLink::unnamed("SNOU", 1),
            NameLink::unnamed("TMPL", 1),
            NameLink::named("EQUI", "/03SKID3-EQUIP1"),
        ];
        let name = ElementName::from_chain(&nested);
        assert_eq!(name.short, "SNOUT 1");
        assert_eq!(
            name.full,
            "SNOUT 1 of TMPLATE 1 of EQUIPMENT /03SKID3-EQUIP1"
        );
    }

    /// 库根不进名字：截图里那条无名 SITE 就叫 `SITE 10`，后面不跟 ` of WORLD /*XXX`，
    /// 短形态与全形态相同。
    #[test]
    fn the_chain_stops_before_the_database_root() {
        let site = [NameLink::unnamed("SITE", 10), NameLink::root()];
        let name = ElementName::from_chain(&site);
        assert_eq!(name.short, "SITE 10");
        assert_eq!(name.full, "SITE 10");
        assert_eq!(owner_tail(&[NameLink::root()]), "");

        // 无名 SITE 下的无名 ZONE：全形态多一段，短形态还是自己那一段。
        let zone = [
            NameLink::unnamed("ZONE", 4),
            NameLink::unnamed("SITE", 2),
            NameLink::root(),
        ];
        let name = ElementName::from_chain(&zone);
        assert_eq!(name.short, "ZONE 4");
        assert_eq!(name.full, "ZONE 4 of SITE 2");
    }

    /// 分隔符是小写 ` of `：`LNPART` 写的是 `0x20 'o' 'f' 0x20`。
    #[test]
    fn the_separator_is_lowercase() {
        let chain = [NameLink::unnamed("ZONE", 1), NameLink::named("SITE", "/S1")];
        assert!(full_name(&chain).contains(" of "));
        assert!(!full_name(&chain).contains(" OF "));
    }

    /// 名字不以 `/` 开头时 `LNPART` 补一个——只补祖先那一截；元素自己有名字时原样给。
    #[test]
    fn a_name_without_a_leading_slash_gets_one_in_the_tail() {
        let chain = [NameLink::unnamed("BEND", 2), NameLink::named("BRAN", "B1")];
        assert_eq!(full_name(&chain), "BEND 2 of BRANCH /B1");
        assert_eq!(full_name(&[NameLink::named("BRAN", "B1")]), "B1");
    }

    /// `IDSPU` 有值时顶掉序号，两种形态都是。
    #[test]
    fn idspu_replaces_the_ordinal() {
        let mut bend = NameLink::unnamed("BEND", 3);
        bend.idspu = Some(7);
        let chain = [bend, NameLink::named("BRAN", "/B1")];
        let name = ElementName::from_chain(&chain);
        assert_eq!(name.short, "BEND idsp 7");
        assert_eq!(name.full, "BEND idsp 7 of BRANCH /B1");
    }

    /// 尾巴单独拼出来，再跟每个兄弟的名词与序号拼一次，结果与整条链一次拼完相同。
    #[test]
    fn a_shared_tail_rebuilds_the_same_name() {
        let owner_chain = [
            NameLink::unnamed("TMPL", 1),
            NameLink::named("EQUI", "/03SKID3-EQUIP1"),
        ];
        let tail = owner_tail(&owner_chain);
        assert_eq!(tail, "TMPLATE 1 of EQUIPMENT /03SKID3-EQUIP1");

        let mut whole = vec![NameLink::unnamed("SNOU", 1)];
        whole.extend(owner_chain.iter().cloned());
        assert_eq!(ElementName::unnamed("SNOU", 1, &tail), ElementName::from_chain(&whole));
        assert_eq!(with_tail("SNOU", 1, &tail), full_name(&whole));
        assert_eq!(short_name("SNOU", 1), "SNOUT 1");
    }

    /// 链走到头还没碰到有名字的祖先：不留 ` of ` 尾巴；空链是空名。
    #[test]
    fn a_chain_with_no_named_ancestor_does_not_end_in_of() {
        let chain = [NameLink::unnamed("ZONE", 4), NameLink::unnamed("SITE", 1)];
        assert_eq!(full_name(&chain), "ZONE 4 of SITE 1");
        assert_eq!(with_tail("SITE", 4, ""), "SITE 4");
        assert!(full_name(&[]).is_empty());
        assert_eq!(ElementName::from_chain(&[]).short, "");
    }
}
