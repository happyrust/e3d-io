# 计划：e3d-io 写路径 · 第三期 · 里程碑 B（任务第 5–6 组：SPAMAP_Codec 与 Built_In_AABB）

状态：**评审通过**（2026-10-03，Plannotator 门 `approved`，无批注 → Q1–Q5 全按推荐项）→ 里程碑 B 进行中（见 §6）
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

（随做随填）
