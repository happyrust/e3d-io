# 计划：按 sesno 区间提取增量更新内容

状态：Plannotator 评审通过（2026-09-03 16:41，`{"decision":"approved"}`，无批注；§5 五问按计划默认值执行）→ P0–P7 已全部落地，验收 §4 七条全部通过（§4.1 的页预算口径于 2026-09-03 复核时修正为「树差读页 ≤ 10，链回走另计」——原句按字面不成立，实测见 §4 脚注）
日期：2026-09-03
影响仓库：`vendor/e3d-io`（本仓库）；消费方为 gen-model / 下游同步工具，本计划不改它们

---

## 0. 目标

给一对会话号 `(from, to)`，**一次调用**拿到这段区间里数据库的增量：

1. 三态键集：Deleted / Inserted / Modified，带两侧记录地址；
2. 每个键的内容：Inserted/Modified 的新记录、Deleted/Modified 的旧记录；
3. 值级差异：owner、成员列表、显式属性、按描述符解码的属性行，各自哪里变了；
4. 明确标出「记录被重写但值没变」的 Modified（`relocated_only`）。

并给 CLI 两个子命令：`sessions` 看会话链，`diff` 出上面这份结果。

**不做**：写回；NAME/TYPEX/反向引用表的会话间差异（见 §5 Q3）；多 extent（语料 543 文件 extent 全为 1）。

---

## 1. 已经测出来的事实（这份计划的地基）

以下全部是 2026-09-03 在本机实测，探针在仓库外 `%TEMP%\e3d_sesno_probe`，复现方式见 §6。

### 1.1 底层三块能力都在

| 层 | 入口 | 状态 |
|---|---|---|
| L1 会话 | `session::DbView::open_at(sesno)`、`walk_sessions()`、`ReadOnlyEngine::open_at` | 有；缺席会话报 `SessionError::NoSuchSession{requested,newest,oldest}` |
| L2 双根 diff | `index::IndexDiff::new(pages, rootA, rootB).finish()` → `CompletedIndexDiff` | 有；`tests/index_diff_real.rs` 7 项绿，剪枝页预算精确到「只读两树不共享的页」 |
| L3 记录 | `record::read_element_record(cache, ext, &IndexEntry)`、`ParsedElement::parse` | 有 |
| L3 值 | `ReadOnlyEngine::extract_element_with_descriptors` / `extract_parsed_element_with_descriptors` | 有 |

**缺的是把它们串起来的入口**：`IndexDiff` 只在 `src/index/diff.rs` 和 `index/mod.rs` 重导出处出现；`ReadOnlyEngine.cache` 私有、无 `pages_mut`，diff 只能走 `DbView`；`e3d-descriptor` 只有 describe / extract / extract-complete / list-uda / compile-metadata / emit-attr-info。

### 1.2 区间 diff 的规模与成本

| 库 | 区间 | del / ins / mod | 读页（仅树差） | 备注 |
|---|---|---|---|---|
| ams8000（18 MB，6605 键） | 263→264 | 0 / 0 / 2 | 6（全树 111） | |
| ams8000 | 250→264 | 0 / 12 / 4 | 10 | |
| ams8000 | 1→281 | 0 / 6605 / 0 | 110 | 会话 1 的树为空 |
| ams7000（104 MB，243k 键） | 237→263 | 0 / 105 / 68 971 | 7 962 | 含 6.9 万条记录双侧读取，全程 1.85 s |
| ams5052（152 MB） | 163→189 | 21 073 / 3 260 / 54 | 474 | 188→189 一个会话删 20 981 条 |
| ams5100（DICT） | 5→27 | 0 / 76 / 133 | 7 | 链里 5 直接跳 27 |

### 1.3 区间 diff 与逐步 diff 的关系

4 个库上：区间 diff 的键集 ⊆ 相邻会话逐步 diff 键集之并，缺失 **0**。
区间内「建了又删」的键只出现在逐步结果里：ams8000 1→281 逐步 46 436 个事件 / 26 256 个键，净增量 6 605；250→264 有 2 个这样的键（BOX 24384/26247、24384/26252）。

结论：**区间 diff = 两端净差；要事件史就逐步走链**。两者都能做，语义要在 API 名字里说清。

### 1.4 会话链有洞，是常态

MERGE CHANGES 回收旧会话后链上只留下部分 sesno：

```
ams7000_0001   1, 237, 238, 262, 263            （5 条）
ams5052_0001   1..7, 163..189                   （34 条）
ams7351_0001   1, 2, 116                        （3 条，1.18 GB）
ams8000_0001   1..281                           （281 条，无洞）
```

`open_at(10)` 在 ams5100 上报 `no session 10; newest is 27 and the chain reaches back to 1`——报得对，但**没有就近策略**。

### 1.5 Modified 是「记录被重写」，不是「值变了」

比较的是 `(page, offset)`：

- ams8000 250→264 的 FTUB 24384/23262：位置变、原始字节变，但 64 个属性行 **0** 变化（上移 500 mm 又还原，共 7 次）；
- ams5052 163→189：1 条 Modified 两侧原始字节**完全一致**（纯搬迁）；
- ams7000 237→263 的 68 971 条 Modified 全部字节不同（升级会话真改了）。

### 1.6 成员列表 / owner 变化不在属性行里

STRU 24384/26202 members 0→1、WORL 13244/0 members 54→52，`extract_element_with_descriptors` 两侧对比都是 0 行变化。要靠 `ParsedElement.members` / `.owner`。

### 1.7 Deleted 的旧记录读得出来

BOX 24384/26247 在 253→254 被删，base 位置 `PageId{1,8731}+132` 仍读到 204 B 完整记录——copy-on-write 没回收。所以「删了什么」可以给全文，不只给 refno。

### 1.8 内容解码必须选对模板

对 DICT 库 ams5100 用 `desvir.dat` 抽取，全部 `noun descriptor template is missing`。`DbDescriptor::db_type_name()`（工作树未提交）+ `db_element::template_file_for(dir, db_type)` 已能派生正确文件名。

### 1.9 测试夹具已漂移

`tests/session_l1_contract.rs` 两项红：硬编码 ams8000 「at 264 / latest page 8897」，样例库已被 CODEX 会话写到 281 / 9277。非代码 bug，但断言必须改成读盘比对，否则回归门不可信。

---

## 2. 设计

四层，每层只依赖下一层已有的东西，新增代码集中在 `src/session/diff.rs`（新）和 `src/bin/e3d-descriptor.rs`。

### 2.1 L1：区间端点解析

```rust
pub enum SessionSelector {
    Latest,
    Exact(u32),
    /// 链上 sesno ≤ n 的最新一条。给下游「水位线 n 之后的一切」用：
    /// n 被 merge 掉时宁可多报（从更早的留存会话起算），绝不漏报。
    AtOrBefore(u32),
}
```

- `session::resolve` 加一臂；`Exact` 语义不变。
- `NoSuchSession` 错误补两个字段 `nearest_older: Option<u32>`、`nearest_newer: Option<u32>`，调用方拿到报错就知道洞的两端在哪。
- 走链一次同时解出两个端点（`resolve_pair`），不要开两次库。

### 2.2 L2：`diff_sessions`

```rust
pub struct SessionDiff {
    pub from: SessionPage,   // 实际解析到的会话，不是调用方传的数字
    pub to: SessionPage,
    pub index: CompletedIndexDiff,
}

impl DbView {
    pub fn diff_sessions(&mut self, from: impl Into<SessionSelector>, to: impl Into<SessionSelector>)
        -> Result<SessionDiff, E3dReaderError>;
}
impl ReadOnlyEngine {
    pub fn diff_sessions(...)  // 同签名，委托同一个 free fn
}
```

- 实现放 `session::diff::diff_between(pages, descriptor, from, to)`，两个类型都委托它，不写第二份。
- `from.sesno == to.sesno` → 空 diff；`from` 在链上比 `to` 新 → `SessionError::ReversedRange{from,to}`，不静默交换。
- 走链上的逐步事件史另给 `DbView::diff_steps(from, to) -> Vec<SessionDiff>`，名字里带 steps。

### 2.3 L3：`ChangedElement`

```rust
pub enum ChangeKind { Inserted, Deleted, Modified { relocated_only: bool } }

pub struct ChangedElement {
    pub refno: RefNo,
    pub kind: ChangeKind,
    pub before: Option<ElementRecordView>,   // Deleted / Modified：base 位置
    pub after:  Option<ElementRecordView>,   // Inserted / Modified：target 位置
}

impl SessionDiff {
    /// 流式，一次一条；ams7000 一个区间 6.9 万条 Modified，不能全攒在内存里。
    pub fn elements<'a>(&'a self, pages: &'a mut PageCache) -> impl Iterator<Item = Result<ChangedElement, E3dReaderError>> + 'a;
}
```

- `relocated_only = before.raw_bytes == after.raw_bytes`。
- 读不出来的记录是 `Err`，不是跳过——和 `IndexDiff` 对坏页的态度一致。

### 2.4 L4：`ElementDelta`（值级）

```rust
pub struct ElementDelta {
    pub refno: RefNo,
    pub owner: Option<(RefNo, RefNo)>,
    pub members_added: Vec<RefNo>,
    pub members_removed: Vec<RefNo>,
    pub explicit_changed: Vec<(u32 /*hash*/, Option<ExplicitValue>, Option<ExplicitValue>)>,
    pub implicit_words_changed: Vec<(usize, u32, u32)>,
    /// 有模板时才填：按描述符解码后哪几行变了
    pub attribute_rows: Option<Vec<(String, Option<DescriptorValue>, Option<DescriptorValue>)>>,
}
```

- 前四项只要 `ParsedElement`，不需要模板，任何库类型都能出。
- `attribute_rows` 需要 `attlib` + 按 `db_type_name()` 选出的 `*vir.dat`，走 `extract_parsed_element_with_descriptors`（不重新 `find_element`，记录已经在手上）。
- 模板缺失是 `attribute_rows = None` 并在结果里留原因，不是整条失败。

### 2.5 CLI

```
e3d-descriptor sessions --db PATH
    # 每行：sesno  page  machine  comment  stamp_words
e3d-descriptor diff --db PATH --from N --to N [--at-or-before] [--steps]
                    [--content] [--desvir PATH --attlib PATH]
    # JSON：{from,to,tally,changes:[{refno,kind,relocated_only,before,after,delta}]}
```

---

## 3. 分步执行

| 步 | 内容 | 文件 | 门 |
|---|---|---|---|
| P0 | 修夹具漂移：sesno / latest_ses_pgno 改为读盘（+0x28 页的 +0x0C 词） | `tests/session_l1_contract.rs` | `cargo test --test session_l1_contract` 全绿 |
| P1 | `SessionSelector::AtOrBefore`、`NoSuchSession` 补邻居、`resolve_pair` | `src/session/mod.rs`、`src/error.rs` | 合成三会话夹具 + ams5100（5,27）：`AtOrBefore(10)`→5，`Exact(10)`→Err 且 nearest=(5,27) |
| P2 | `diff_between` / `DbView::diff_sessions` / `ReadOnlyEngine::diff_sessions` / `diff_steps` | `src/session/diff.rs`（新）、`src/engine.rs` | 合成 shared_leaf_db 7→8 = IndexDiff 同答案；ams8000 newest-1→newest 与 `tests/index_diff_real.rs::audit` 同答案；反向区间 Err |
| P3 | `ChangedElement` + 流式 `elements()` | `src/session/diff.rs` | ams8000 253→254：Deleted BOX 的 before 有 204 B；ams5052 163→189：`relocated_only` 计数 = 1 |
| P4 | `ElementDelta` | `src/session/delta.rs`（新） | STRU 24384/26202 250→264：members_added=[24384/26248]；FTUB 263→264：attribute_rows 含 POS、SPAMAP 两行 |
| P5 | CLI `sessions` / `diff` | `src/bin/e3d-descriptor.rs` | `diff --db ams8000 --from 250 --to 264` 输出 16 条，与 §1.2 一致 |
| P6 | 区间 ⊆ 逐步 的一致性门（4 个样例库） | `tests/session_diff_real.rs`（新） | 缺失 0 |
| P7 | README Status 一行、CHANGELOG | `README.md`、`CHANGELOG.md` | — |

P0 独立可先合。P1→P2→P3→P4 串行；P5 依赖 P3（`--content`）与 P4（`delta`）；P6 依赖 P2。

### 3.1 落地记录（2026-09-03）

| 步 | 落点 | 门的实测 |
|---|---|---|
| P0 | `tests/session_l1_contract.rs`：`newest_sesno_on_disk` / `assert_well_formed_chain`，三处硬编码全改读盘 | 14 项绿（原 9 绿 2 红 + 新增 3） |
| P1 | `SessionSelector::AtOrBefore`；`SessionError::NoSuchSession{nearest_older,nearest_newer}`；`ChainWalk` 惰性链游标，`walk_chain`/`resolve`/`resolve_pair` 三者共用；`ReadOnlyEngine::open_at` 改走 `session::resolve`，接受任意选择器 | ams5100 `Exact(10)` → "either side of it are 5 and 27"；`AtOrBefore(10)` → 5，读 2 页 |
| P2 | `src/session/diff.rs`：`diff_between` / `diff_steps` / `SessionDiff`；`DbView` 与 `ReadOnlyEngine` 各一层薄壳（`pages_mut` 不暴露，Q5） | ams7000 `--from 200` 报错并指出 1/237；`--at-or-before on` 解析到 1→263 |
| P3 | `ChangedElement` / `ChangeKind::Modified{relocated_only}` / 流式 `ChangedElements`；地址上的记录 refno 不符 → `IndexRecordMismatch` | ams5052 163→189 `relocated_only` 恰 1 条；ams7000 237→263 68 971 条双侧全读 |
| P4 | `src/session/delta.rs`：`ElementDelta` 五个维度 + `with_attribute_rows`；`engine::extract_parsed_element_with_descriptors` 抽成自由函数 | FTUB 上移会话：`POS` z 差 500，词 17 / 39 同变；`STRU` members_added=[26248] |
| P5 | `e3d-descriptor sessions` / `diff`（`--at-or-before` `--steps` `--content` `--limit` `--desvir --attlib`） | `diff --from 250 --to 264` 16 条，与 §1.2 一致 |
| P6 | `tests/session_diff_real.rs` 14 项（7 合成 + 6 真实 + 1 全语料 ignored） | ams8000 / ams7329 / ams5100 / ams7000 / ams5052：range ⊆ steps，缺失 0；ams5052 全史 306 945 键净变、433 752 键被碰过，126 807 建了又删 |
| P7 | README Status 与 API 表、CHANGELOG Added/Changed、本节 | — |

全量 `cargo test --release`：66 个测试二进制、351 项通过、0 失败。

---

## 4. 验收

1. `DbView::open(ams8000).diff_sessions(250, 264)` 返回 tally `{deleted:0, inserted:12, modified:4}`；树差读页（`IndexDiff` 只读两树不共享的页）≤ 10，链回走另计——`resolve_pair` 从最新会话沿链回走到 `from`，每回退一个会话 1 页，不在这 10 页里。[^p41]
2. `diff_sessions(AtOrBefore(200), Latest)` 在 ams7000 上解析到 `from.sesno == 1`，并在结果里能看到这一点。
3. `diff_sessions(Exact(200), Latest)` 在 ams7000 上报 `NoSuchSession{requested:200, nearest_older:Some(1), nearest_newer:Some(237)}`。
4. ams5052 163→189 `elements()` 里 `relocated_only == true` 恰好 1 条。
5. 4 个样例库：`diff_sessions(a,b)` 键集 ⊆ `diff_steps(a,b)` 键集之并。
6. `cargo test` 全绿，含 `session_l1_contract`。
7. `e3d-descriptor sessions --db ams7000` 打出 5 行，sesno 为 1,237,238,262,263。

[^p41]: 原句写的是「`stats().pages_read` 增量 ≤ 10」，2026-09-03 复核时按字面实测不成立：`DbView::open(ams8000)` 后 `diff_sessions(250, 264)` 的 `pages_read` 增量是 **41** = 链回走 31 页（会话 281 → 250，每会话 1 页）+ 树差 10 页；`open_at(264)` 后再 diff 是 24 = 14 + 10。§1.2 表里的「读页 10」从一开始就只算了 `IndexDiff`，验收句子把它写成了整次调用的预算。树差那 10 页守住了；链回走的价格随 `newest - from` 线性增长，是链表结构的固有成本，要压它只能让 `DbView` 缓存一次走过的链（`diff_steps` 目前同一次调用走链两次：`resolve_pair` 与 `walk_chain`），另立计划。这条目前没有自动化测试守着，`tests/session_diff_real.rs` 只在「同一会话两端」的合成用例上断言了 `pages_read` 增量为 0。

---

## 5. 待评审拍板

- **Q1 洞的默认策略**：`diff --from 200` 在 ams7000 上，默认是报错（要求显式 `--at-or-before`）还是默认就近？计划取「默认报错」——多报比漏报安全，但静默多报会让下游以为 1→263 之间全变了。
- **Q2 值级差异是否本期做**：P4 是唯一需要模板的一层，也是最重的一层。如果下游只要「哪些 refno 动了 + 新旧记录字节」，P4 可以后置。
- **Q3 NAME / TYPEX / 反向引用表**：session 页的表目录（`TableRoot`，工作树未提交）是 copy-on-write 的，同样能双根 diff。是否纳入本期？计划不纳入。
- **Q4 `diff_steps` 的输出量**：ams8000 1→281 是 46 436 个事件。是给 `Vec<SessionDiff>` 还是也做成流式？计划先 `Vec`，每步只含键集不含记录。
- **Q5 `ReadOnlyEngine` 是否暴露 `pages_mut`**：不暴露则 `diff_sessions` 在两个类型上各写一层薄壳；暴露则 `DbView` 与 `ReadOnlyEngine` 的边界进一步模糊。计划不暴露。

---

## 6. 复现

```powershell
# 探针（仓库外）
cd $env:TEMP\e3d_sesno_probe ; cargo build --release
D:\Rust\target\release\e3d_sesno_probe.exe D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 250 264 6
D:\Rust\target\release\e3d_sesno_probe.exe D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7000_0001 237 263 3
D:\Rust\target\release\e3d_sesno_probe.exe D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5052_0001 163 189 2
D:\Rust\target\release\e3d_sesno_probe.exe D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams5100_0001 10 27 3   # 洞

# 现有门
cargo test --release --test index_diff_real --test session_l1_contract
```
