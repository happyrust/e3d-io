# 计划：e3d-io 写路径 · 第三期 · 里程碑 B（任务第 5–6 组：SPAMAP_Codec 与 Built_In_AABB）

状态：**评审通过**（2026-10-03，Plannotator 门 `approved`，无批注 → Q1–Q5 全按推荐项）→ **里程碑 B 已完成**（2026-10-03，B0–B9，见 §6）
日期：2026-10-03
分支 / worktree：`write-path` @ `D:\work\plant-code\old\vendor\e3d-io-write`，HEAD `14239d3`（里程碑 A 收尾，领先 `github/write-path` 14 个提交，未推送）
依据：`.kiro/specs/write-path-phase-3/` 的需求 / 设计 / 任务（下称「需求」「设计」「任务」）；`docs/evidence/2026-10-03-spamap-census.md`（下称「普查」）
上一里程碑：`docs/plans/2026-10-03-write-path-phase-3-milestone-a-plan.md`

---

## 0. 一句话

里程碑 A 落了读侧、规则注册表、取证工具和普查；里程碑 B 写「算条目」的那一半：**SPAMAP_Codec**（世界系 AABB → 条目）、**候选规则**、**Codec_Check**（`spamap-check`）、**Built_In_AABB**（BOX / CYLI 自算世界包围盒）与 `world-frame`。全部不碰 `src/write/`；产出是第 7 组取证要用的「候选 vs 语料」一致率，规则一条也不在 B 里坐实。

开工前拿 A 的普查条目先试了三条候选，四个库 28 269 条**全部对得上**（§2）。B 的第一批候选就是它们，`spamap-check` 会把这些数字变成可复现的命令。

## 1. 现状

| 项 | 状态 |
|---|---|
| HEAD | `14239d3`；全套 107 个测试二进制 645 / 0 / 30，clippy 63、`cargo doc` 25（同基线） |
| 已有、B 直接用 | `src/core_rules.rs`（27 条规则全 `Pending`，空实现类型）、`src/spatial/{mod,state}.rs`（`WorldAabb`、`SpatialState`、`ReadState`）、`src/index/spamap.rs`、`e3d-descriptor spamap` / `spamap-census` |
| B 的任务 | 5.1 Codec、5.2 candidates、5.3 `spamap-check`、5.4* Property 6、5.5 Property 5；6.1 Built_In_AABB、6.2 `world-frame` + `spamap-check` 的 AABB 一项、6.3 Property 8 |
| 依赖 | 5.2 在 5.1 之后；5.3 在 5.1、5.2 之后；6.2 在 6.1、5.3 之后；6.3 在 6.1、6.2 之后。都不依赖 IDA |

## 2. 开工前的数据（2026-10-03，普查输出的条目，一支临时脚本算）

数据：`ams8000_0001` 会话 640（3 746 条）、`ams7999_0001` 200（23 891）、`ams251142_0001` 10（327）、`TES023` 109（305），共 28 269 条。数字要由 B3 的 `spamap-check` 复算后才算数。

| 规则 | 候选 | 语料核对 |
|---|---|---|
| `spamap.short_axis` | 按 0.1 mm extent 取最短轴；并列时取**最后**一根（z 先于 y 先于 x） | 28 269 / 28 269；并列 6 236 条全取最后一根，0 条取第一根 |
| `spamap.bucket` | 最短轴 extent（0.1 mm）：< 600 → 1，< 2 000 → 2，< 6 000 → 3，< 20 000 → 4，否则 5，即 60 / 200 / 600 / 2 000 mm | 28 269 / 28 269。600、2 000、6 000 三处都有样本落在上一桶（下界含）；第四界只知在 (19 800, 21 200]，取 20 000 是猜的 |
| `spamap.zero_extent` | extent 为 0 落桶 1 | `ams251142_0001` 138 条、`ams7999_0001` 的零尺寸点条目全符 |
| `spamap.word_sign` | 坐标字按有符号 32 位读（负 min 以补码存） | 普查口径 Key0 不符 0 |
| `spamap.key0` | `N × 10^8 + 有符号 min`，`N = 6 × 轴 + 桶` | 普查口径 0 不符 |
| `spamap.entries_k` | 尚无候选，只有观察：k = 1 的条目与同元素 k = 0 的 AABB **全不同**。`ams8000_0001` 是「k = 0 大框 flags 3 + k = 1 小框 flags 2」；`ams7999_0001` 多为「k = 0 零尺寸点 flags 0 + k = 1 框 flags 2」 | 118 + 2 634 个元素各两条 |
| `spamap.flags` | 只见 0 / 1 / 2 / 3 / 6 / 7。BOX、CYLI 全为 2，BOX 的 `OBST` = 2：候选「flags = OBST」限 BOX / CYLI | 管件（FTUB / ELBO / BEND / WELD / VALV）模板里没有 `OBST`，flags 另有来源 |
| `aabb.ori_form` | `ORI` 存三个实数（度），如 `[-179.898, -0.1765, 89.9997]` | 复合顺序待定 |
| `aabb.compose` | 原生 BOX `24384/24777` 自身 POS / ORI 全为 0，位姿全在 owner 链（EQUI 一级）上 | 它的盘上条目是 Property 8 的第一个逐字样本 |

## 3. 复核与风险

### 3.1 候选放不进 A 定下的槽（影响 5.2，要拍板：Q1）

A 把每条规则的实现类型做成空枚举，`Slot::Confirmed(T)` 与 `Slot::Candidate(T)` 同用 `T`。空枚举造不出值，候选也就填不进去；要写候选就得给 `T` 加变体，而设计要求候选只放 `src/spatial/candidates.rs`、不进注册表。

**提议（Q1）**：槽改为 `Slot<T, C>`——`Confirmed(T)` 仍是 `core_rules.rs` 里的枚举（坐实时按 Evidence_Doc 加变体，可审），`Candidate(C)` 是该规则签名的函数指针（如 `fn(i64) -> u8`），实现写在 `candidates.rs`。`confirmed()` 里没有 `Candidate`，const 断言照旧；`src/write/` 不出现 `candidates` 的测试照旧。

### 3.2 非内建 noun 的 flags 可能算不出（D1 / D3 的风险，先记不改）

D1 让 BOX、CYLI 之外的几何 noun 由调用方给 World_AABB，但条目还要 flags。§2 显示管件的 flags 不来自 `OBST`；若坐实下来要读目录数据（SPRE 所指元件），D7 会让「要求」下这些 noun 的插入一律拒写，调用方给了 AABB 也没用。B 只在 `spamap-check` 里按 noun 分列 flags 的一致率，把这条风险量化出来；改不改 D1 / D3，等第 7 组取证后再议。

### 3.3 取整可能不是四舍五入（影响 6.3）

原生 BOX `24384/24777` 的盘上 y 最大值是 `83113`。按 09-22 取证所记的位姿（310 × 590，绕 Z 转 5°，N 8004）粗算是 8 311.39 mm，即 83 113.9 个 0.1 mm，存成 `83113` 像是向下取整；但那是取证文档里的近似位姿。B6–B8 用 owner 链上的原值精算后才能下结论，候选写两个（四舍五入 / min 向下取、max 向上取）由 Property 8 的反例分布来选。

### 3.4 E3D_Authored_Element 要「记录页 → 会话」的映射（影响 5.3）

`spamap-check` 只核 E3D 写的元素：元素最新记录所在页落在哪个会话的页区间，看该会话的 USER 是不是 `e3dio`。会话链有每个会话的末页，区间可由相邻会话推出；B3 加一个读侧小函数，并以门用副本里已知的 `e3dio` 会话做示例测试。现在的 Corpus 原件里没有 `e3dio` 会话，这一判定在原件上恒为「E3D 写的」。

## 4. 里程碑 B：步骤

| 步 | 任务 | 产出 | 验收 |
|---|---|---|---|
| B0 | 3.1（Q1） | `core_rules.rs` 槽改 `Slot<T, C>`，各规则的候选函数签名定在注册表旁；design 同步 | 既有测试、const 断言照过 |
| B1 | 5.1 | `src/spatial/codec.rs`：`encode` 按设计次序（取整 → 范围 → 最短轴 → 零 extent → 桶 → Key0 → 条目数与 k → flags），结果三分（`Entries` / `NeedsUnconfirmed` / `OutOfRange`），`key0_of` | 单测覆盖每步缺规则即 `NeedsUnconfirmed`、不出部分条目 |
| B2 | 5.2 | `src/spatial/candidates.rs`：§2 的候选，每个注释写明假设与 §2 的出处；`CandidatePick` 按规则选候选 | 只经 `RuleSet::candidates` 可达 |
| B3 | 5.3 | `src/spatial/check.rs`（纯函数计数）+ `e3d-descriptor spamap-check --input … [--rules confirmed\|candidates] [--exclude <文件>] [--refno]`：Key0 / 条目数与 k / flags 三项分开报一致数 / 总数、前 100 个反例及所依规则名，按 noun 分列；页 → 会话 → USER 判定（3.4） | 在 Corpus 上跑出候选一致率，复现 §2 的数字 |
| B4 | 5.4* | `tests/spamap_codec_props.rs`：Property 6（随机 `CandidatePick`、边界与越界坐标、并列、零 extent） | 256 例 |
| B5 | 5.5 | `tests/spamap_codec_real.rs`：Property 5（三项分开计数；向单条注入一项失配只改对应计数） | 坐实前只作数据，不作通过判据 |
| B6 | 6.1 | `src/spatial/aabb.rs`：`FrameComposer` / `ChainComposer`（`ORI` 三角度、复合顺序各写候选），`BuiltInAabb`（BOX 用 XLEN / YLEN / ZLEN、CYLI 用 DIAM / HEIG，解析解候选），`UndecodableAttr` 照设计 | 单测 |
| B7 | 6.2 | `e3d-descriptor world-frame --db --refno`；`spamap-check` 增 Built_In_AABB 一项 | — |
| B8 | 6.3 | `tests/spamap_aabb_real.rs`：Property 8，样本含 BOX `24384/24777`；记反例分布，供 3.3 与 `aabb.arithmetic` | 坐实前只作数据 |
| B9 | 检查点 | Build_Checks 对照基线；候选一致率填进 Spec_Ledger「规则坐实状态总表」备注 | 不新增失败，clippy / doc 不增 |

顺序：B0 → B1 → B2 → B3 → B4 / B5 并行 → B6 → B7 → B8 → B9。

## 5. 待你拍板

批准本计划 = 全按推荐项（A）。要改哪条，在对应行批注。

| Q | 问题 | 推荐（A） | 备选 |
|---|---|---|---|
| Q1 | 候选怎么进槽（§3.1） | `Slot<T, C>`：确认用注册表里的枚举，候选用函数指针、实现放 `candidates.rs` | B：实现类型改 trait 对象，确认与候选同一类型（类型不再挡住「确认集里放候选」，只靠 const 断言与测试）；C：两者都用函数指针 |
| Q2 | 里程碑范围 | 第 5–6 组 | B：再加两件互不依赖的读侧小件——11.3（`IndexCursor::last_before` 与 `next-word1`）、12.1（Scale_Probe 骨架）；C：再加第 7 组 7.1 的编码规则取证（要读 core） |
| Q3 | 候选写多少 | 每条规则 1–2 个，按 §2 的数据挑 | B：每条只写 1 个；C：凡语料能分辨的都写，由 `spamap-check` 比 |
| Q4 | BOX / CYLI 的算术精度 | 先按 f64 实现，Property 8 记反例分布再定 | B：直接按 f32；C：两套都实现、`CandidatePick` 切换 |
| Q5 | 提交节奏 | 沿用里程碑 A：每步自动提交，结束统一回报 | B：每步回报、你回「提交」再提交 |

## 6. 执行记录

- **2026-10-03 14:53 评审**：Plannotator `{"decision":"approved"}`，无批注；用户回「按推荐项开工里程碑 B」。计划提交 `44b3ea5`。
- **B0（Q1）**：`Slot<T, C>`，`C` 是 `core_rules::candidate` 里的函数类型（codec 用到的九条规则），其余规则 `C = NoCandidate`；`RuleSet::candidates(&CandidatePick)` 只填仍为 `Pending` 的槽。含函数指针的类型去掉 `PartialEq`（比较无意义，且触发 `unpredictable_function_pointer_comparisons`）。`ReadState` 改为先读记录头、属性用到才抽，新增 `record_page`。提交 `59c73b0`。
- **B1、B2（5.1、5.2）**：`src/spatial/codec.rs`——`SpamapCodec::{encode, key0_of}`，八步按设计次序，遇 `Pending` 记名后接着走、最后不出任何条目（`zero_extent` 只在最短轴 extent 确知为 0 时才查）；`Encoded::{Entries, NeedsUnconfirmed, MissingInput}`、`CodecError::{OutOfRange, Undecodable}`，Key0 越界报该轴可表示的 min 区间。为免规则坐实后 codec 静默失效，加 `Implementation<C>` trait：确认实现与候选同签名调用（现为空枚举的 `match *self {}`）。`src/spatial/candidates.rs`（`#[doc(hidden)]`）：§2 的候选（就近取整 / 外扩取整两个、有符号字、最短轴取最后、零 extent 交桶规则、桶界 600 / 2 000 / 6 000 / 20 000、Key0 落非负有符号字、单条 k = 0、BOX / CYLI flags = OBST、覆盖按普查 43 个有 / 51 个无、BRAN / PJOI / SCOJ 不说），`pick()` 为默认组合。单测 11 条：候选把需求 4.20 的 BOX 编成 `[300008495, 24384, 26484, 0]` / `[8495, 18000, 27500, 11505, 22000, 32500, 2]`；确认集下列出途经的七条 `Pending` 规则、不出条目。提交 `24b8292`。
- **B3（5.3）**：`src/spatial/check.rs`——三项分开计数：Key0 按每条自身六字（`key0_of`）、条目数与 k 按元素首条的框只跑取整与条目规则（codec 新增 `entries_of`，不受 Key0、flags 牵连）、flags 只跑 flags 规则（新增 `flags_of`），各报一致数 / 总数、按 noun 分列、前 100 个反例及所待规则名；`Authors` 按会话链「末页 ≥ 记录页的第一个会话」定 USER，非 `e3dio` 即 E3D 写的。`e3d-descriptor spamap-check --input … [--session …] (--schema | --desvir --attlib) [--rules confirmed|candidates] [--exclude] [--refno]`，`--rules` 缺省 `confirmed`。实跑候选（release 不到 1 s）：**Key0 28 269 / 28 269**；条目数与 k：ams251142 327 / 327、TES023 305 / 305、ams8000（会话 640）3 507 / 3 628、ams7999 18 404 / 21 257——差的恰是带 k = 1 条目的 121 与 2 853 个元素；**flags：BOX / CYLI 1 590 / 1 590 全对**，其余 noun 记为 `unconfirmed:spamap.flags`。与 §2 的临时脚本数字一致。提交 `1f54a48`。
- **B4（5.4\*）**：`tests/spamap_codec_props.rs`（Property 6，默认 256 例）——随机 `CandidatePick`：八条规则各从 `candidates.rs` 的候选与替身（无符号字、并列取第一根、零 extent 自成桶 1、20 m 起不给桶、无符号 Key0、第二条目自带框〔同框 / z 压扁 / x 移出字〕、缺输入、只给 k = 0 的 flags）里挑，或留 `Pending`；框含 Key0 与有 / 无符号字各边界 ±2 mm、远超、并列、零 extent。期望在测试里按设计次序逐步推出、不看 codec 的记账：越界须是确实越界的坐标之一（值不在所报区间内），`NeedsUnconfirmed` 的规则名逐一相等，`Entries` 整组相等（含 `relied_on`）。写它时修了 codec 一处：最短轴 extent 为 0 而 `spamap.zero_extent` 仍 `Pending` 时，原先落到桶规则去猜桶，可能按猜的桶报 Key0 越界；现在桶记为未知、只报 `NeedsUnconfirmed`（codec 单测补一条）。变异核对：撤掉该修复，256 例 10 次里 4 次抓到、2 048 例必抓；那次留下的失败种子（z 轴压扁在 204 748 364.7 mm、`zero_extent` 为 `Pending`）随 `tests/spamap_codec_props.proptest-regressions` 入库，此后每次先重跑。提交 `9e9ef33`。
- **B5（5.5）**：`tests/spamap_codec_real.rs`（Property 5）——基线会话 640 与 Corpus 各库最新会话各核两遍。确认集下断言每项 `agree + waiting = total`：`check::Item` 新增 `waiting`，即因 `Pending` 规则而缺的反例数（D7 回退，不算反例）；规则坐实后同一断言就是「一致数 = 总数，只按剔除清单剔除」，测试不用改。候选集只打印数据：Key0 28 269 / 28 269；条目数与 k 3 507 / 3 628（640、646 相同）、18 404 / 21 257、327 / 327、305 / 305；flags 的 BOX / CYLI 1 590 / 1 590——与 B3 一致；两套规则下各项总数相同，无读不了的记录。注入：在基线表上按项挑「单独核对时一致」的条目 / 元素，改 Key0、k 或 flags 一字（`TestRunner` 默认 256 例），断言只有该项一致数减 1、总数与 `waiting` 不变，另两项计数（含按 noun）不动。E3D_Authored_Element 的判定移进库（`Authors::e3d_wrote`），`spamap-check` 与测试共用；`spamap-check` 各项输出加 `waiting`。整个文件 debug 约 5 s。提交 `378784e`。
- **B6（6.1）**：`src/spatial/aabb.rs`——`WorldFrame`（`rotation` 的列为自身三轴）、`Link`、`FrameComposer` / `ChainComposer`（上溯到 WORL，链上每级读 POS / ORI；ORI 交 `aabb.ori_form`、复合交 `aabb.compose`、取值交 `aabb.arithmetic`；链断或超 64 级为 `NoWorld`）、`BuiltInAabb::compute` → `AabbOutcome::{Aabb, NeedsUnconfirmed, NotBuiltIn, NoWorld}`、`BUILT_IN_NOUNS = [BOX, CYLI]`（D1 唯一落点）。所需属性在问任何规则之前全部读完，`UndecodableAttr` 与规则状态无关（需求 6.8）；`aabb.degenerate` 只在某个尺寸 ≤ 0 时才查。`aabb.*` 六条规则加候选函数类型（`core_rules::candidate`），codec 的 `Walk` 移到 `spatial` 共用。候选：ORI 为三个角度（度），R = Rz·Ry·Rx（备选 Rz·Rx·Ry）；每级位姿都在其 owner 系中复合；BOX 以原点为中心的精确外包；CYLI 精确外包（备选：自身外包盒转到世界）；负尺寸取绝对值；f64。开工前探了原生 BOX `24384/24777`：自身 POS / ORI 为 0，EQUI `24384/24776` 的 POS `[-2821.27, 8003.96, 1400]`、ORI `[180, 0, -175]`（即「Y is N 5 W and Z is D」），ZONE / SITE 在原点。候选算出的世界盒经 codec 候选编码，与盘上条目逐字相同：键 `[299969986, 24384, 24777, 0]`、值 `[-30014, 76966, 11250, -26411, 83113, 16750, 2]`。§3.3 说的「像向下取整」是取证文档里的近似位姿造成的，按 owner 链原值算就是就近取整。单测 7 条，含需求 6.17 的 BOX 精确等于 x [849.5, 1150.5]、y [1800, 2200]、z [2750, 3250]。提交 `fdd7f98`。
- **B7（6.2）**：`e3d-descriptor world-frame --db --refno [--session] (--schema | --desvir --attlib) [--rules confirmed|candidates]`，`--rules` 缺省 `confirmed`：印 owner 链（每级 POS / ORI 原值）、复合出的世界系（原点与三轴）、World_AABB 与 codec 编出的条目；原生 BOX 会话 640 在候选下与盘上条目逐字相同。`spamap-check` 增 `built_in_aabb` 一项（需求 6.9，`check::Checked.built_in_aabb`）：每个 E3D 写的、有条目的 BOX / CYLI，由内建 AABB 算框、codec 编码，与盘上该元素的全部条目逐字比较，`waiting` 同前。候选实跑（release 0.4 s）：ams8000 会话 646 79 / 79（BOX 45 / 45、CYLI 34 / 34），ams7999 1 292 / 1 310（BOX 261 / 275、CYLI 1 031 / 1 035），TES023 199 / 201（CYLI 135 / 137），ams251142 没有 BOX / CYLI；合计 1 570 / 1 590。20 个反例分三类：成对的大偏差（如 −1 291 / +2 575、−908 / −2 733，各落在两个元素上，像 ORI 中间角的转序）、单侧 1.5–33 mm 的小偏差、TES023 两个 CYLI 盘上是 0.5 mm 见方的框而候选算出零尺寸（`aabb.degenerate` 的线索）——B8 逐类记分布。提交 `15b83d3`。
- **B8（6.3）**：`tests/spamap_aabb_real.rs`（Property 8）——基线会话 640 与 Corpus 各库最新会话：每个 E3D 写的、有条目的 BOX / CYLI 经 `ReadState` 算框、codec 编码，与盘上条目逐字比较。判据：确认集下 `built_in_aabb` 项 `agree + waiting = total`（坐实后即为判据，只按需求 6.16 记入 Spec_Ledger 的容差放宽），Codec_Check 与本测试逐元素走查的计数一致，原生 BOX 在样本中。候选数据：默认组合 ams8000（640、646）79 / 79，TES023 201 / 201，ams7999 1 292 / 1 310，合计 1 572 / 1 590。各备选都更差：ORI 按 y→x→z 转，ams7999 降到 1 120、基线 78；CYLI 用自身外包盒，718 / 78；尺寸取绝对值，TES023 199——默认组合就是数据支持的那组。据此把 `aabb.degenerate` 的默认候选改为「尺寸 0 量作 0.5 mm」：TES023 两个 DIAM 0、HEIG 0 的 CYLI（`17419/184`、`17419/284`）盘上是原点 ±0.25 mm 的框；取绝对值留作备选。反例分布：18 个全在 ams7999，没有一个差 ±1（0.1 mm）——f64 与就近取整在 1 572 个元素上无一例外，§3.3 与 `aabb.arithmetic` 暂无单精度迹象。12 个差 ≥ 10 mm、6 个差 1.5–2.2 mm：多数集中在 EQUI `24383/76459` 下几个 SUBE 的 BOX（成对出现：−1 291 / +2 575、−908 / −2 733、−907 / +2 733，另有 Key0 的 N 差 1），以及几处单侧 1.5 mm、28–33 mm，像是祖先改动后后代条目没有重算（7.3 的 `spamap.descendants.*`，也是需求 4.17 剔除清单的候选）；另有两个绕 Z 转 −35° 的 CYLI（`24383/71552`、`71555`）x 向两侧各宽 2.1 mm，原因未明。逐个列在测试输出里，供 7.5。Corpus 会话的打开移到 `tests/common/spamap_session.rs`，Property 5、8 共用。提交 `ec16de7`。
- **B9（检查点）**：Build_Checks（`CARGO_TARGET_DIR=target-phase3`；原始日志与解析脚本只在本机 `.agents/tasks/write-path-phase-3-milestone-b/`，不入库）：110 个测试二进制 + Doc-tests 1 个，669 / 0 / 30（里程碑 A 末是 107 个、645 / 0 / 30；新增 3 个测试二进制，B1–B8 共加 24 条）；clippy 63 个 warning（各目标汇总 85，lib test 的 22 个是重复）、`cargo doc` 25 个——都与基线相同。报 skip 的仍是基线那 7 个（缺 `evidence/*/sample.tsv` 或 `E3D_CORPUS_ROOTS`），新加的 Corpus 测试无一 skip。候选一致率写进 Spec_Ledger「规则坐实状态总表」备注（编码 9 条、`aabb.*` 6 条），B8 的反例线索另记进 `spamap.recompute` / `spamap.descendants` / `spamap.record_item` 三行；所有规则仍为「未坐实」。**里程碑 B 完成**：任务第 5、6 组全部打勾。
