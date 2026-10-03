# 计划：e3d-io 写路径 · 第三期 · 设计复核与里程碑 A（任务第 1–4 组）

状态：**评审通过**（2026-10-03，Plannotator 门 `approved`，无批注 → Q1–Q6 全按推荐项）→ **里程碑 A 已完成**（2026-10-03，任务第 1–4 组全部打勾，A0–A10 见 §5；全套 107 个测试二进制 645 / 0 / 30，clippy 63、`cargo doc` 25 同基线）
日期：2026-10-03
分支 / worktree：`write-path` @ `D:\work\plant-code\old\vendor\e3d-io-write`，HEAD `3e16423`（领先 `github/write-path` 1 个提交）；工作树：`tasks.md` 有 1.1 的未提交改动，`.agents/`、`.kiro/skills/`、`.kiro/specs/e3d-studio-m1/`、`tasks.meta.json` 未跟踪
依据：`.kiro/specs/write-path-phase-3/` 的 `requirements.md` / `design.md` / `tasks.md`（下称「需求」「设计」「任务」）；1.1 基线快照 `.agents/tasks/write-path-phase-3-milestone-a/baseline.md`
上一期：`docs/plans/2026-09-22-write-path-phase-2-plan.md`（T3 定「SPAMAP 升为第三期第一件」）

---

## 0. 一句话

设计把「e3d-io 写的几何能被 E3D 空间搜索找到」拆成一条主链（S：SPAMAP 读侧 → core 取证 → 编码 → 写侧 → 门）和两条支线（P：`open` ≤ 1 s 与发号下界、staging 近线性；B：页 1 桶表，按取证条件做），用规则注册表加 D7 回退把所有未坐实的 E3D 行为挡在写侧之外。骨架是对的。对照代码复核出 **1 处必须先改的设计错误**（SPAMAP 读侧不能经 `IndexPageView`，§2.1）和 **3 处后面会咬人的缺口**（金样不认样本身份、基线证据挂在未入库目录、Corpus 测试依赖未设的环境变量，§2.2–2.4）。

进度：90 个子任务里只有 1.1（基线）做完，且未提交。本计划提议先做 **里程碑 A = 任务第 1–4 组**：基线收尾、proptest、Golden 夹具与 off_compat 金样、IB_Wrapper 与证据机检、SPAMAP 读侧与普查、规则注册表与空间值类型。全部不碰 `src/write/`；做完即可进第 5–7 组（Codec、Built_In_AABB、core 取证）。

## 1. 设计要点（复述，供核对我的理解）

| 链 | 做什么 | 需求 | 卡在哪 |
|---|---|---|---|
| S | IB_Wrapper → SPAMAP_Reader / 普查 → core 取证（编码 / 覆盖 / 重算 / AABB）→ 规则注册表、SPAMAP_Codec、Built_In_AABB → SPAMAP_Writer 接 `Transaction`（合并、重铸、dry run、`Moved` 重 stage）→ `e3d-write` 的 `aabb` / `--spatial` → TTY 门 Q13 组 | 1、3–10 | 门槛：SPAMAP 维护的首个写侧提交，其父提交里须已有 Evidence_Doc、不在 `src/write/` 的读侧解码、以 Corpus 为输入且 0 失败未跳过的测试（需求 2 第 1、2 条） |
| P | 取证 Counter_Semantics 与索引键序 → Open_Fast_Path → 发号下界 + Q14；Scale_Probe → 剖析 → 金样 → staging 优化 | 11、12 | 门槛只卡发号下界；Open_Fast_Path 与 staging 优化是字节中性，按金样验收（需求 2 第 7 条） |
| B | S6b 取证 → E3D 会撞号才写页 1；若 E3D 原地重写页 1，先停下找你确认 | 13 | 条件分支，写侧同样受门槛约束 |

几条承重的机制：

- **未坐实不执行**：`Transaction` 只拿得到 `RuleSet::confirmed()`，未坐实规则的实现槽恒为 `Pending`；候选实现只经 `candidates()` 给 Codec_Check 与测试。缺规则的元素视同「无可用 World_AABB」，交 Spatial_Policy（默认「要求」= 拒写整批，D3）。
- **拒写不靠 rollback**：SPAMAP 规划挂在记录定稿之后、写任何一页之前，是纯内存计算，拒写时文件逐字节不变。
- **关闭即 0.4.0**：「关闭」或净编辑为空时不读表、不分配页，其他结构的页号与 0.4.0 相同，由 Property 16 的金样守住；公开原语签名不变，内部传 `None`。
- **只按结束状态算**：Recompute_Set（定义 ①–⑥）与 Target_Entries 只看本批结束状态，中间状态不参与。
- **写侧复用 CoW**：`keyed_cow.rs` 泛化为 `rewrite_table::<V>`，`V = 0` 是现有键表（字节不变），`V = 7` 给 SPAMAP。

## 2. 复核发现（2026-10-03 对照代码实读）

### 2.1 必须先改：SPAMAP 读侧不能经 `IndexPageView`（影响任务 3.1）

设计「SPAMAP_Reader」节写：每页先用 `IndexPageView` 取页头和槽，「它按页头的 `key_dwords` / `value_dwords` 切槽，支持有值页」。实读 `src/index/mod.rs`，这条走不通：

- `IndexNode::decode`（`IndexPageView::decode` 走同一处）第 233–239 行要求页头字 1 等于 `DATA_PAGE_INDEX`（13 387 743，refno 索引表）。SPAMAP 页这个字是 230 902 453，**每一页都会报 `UnexpectedNoun`**。
- 就算放过这条，第 268 行的步长对所有层都是 `key_dwords + value_dwords`；SPAMAP 内结点是 4 键字 + (pgno, ext) = 6 字，页头字 4 若写的是叶值宽 7 就切错。
- `entry()`（第 306 行起）把前两个键字解成 RefNo、值解成 (page_no, packed)，是 refno 索引专用的。

同形的是 `src/index/reference.rs` 的 `decode`（第 280–386 行）：按名 hash 认表、键宽 4、叶值宽只查 level 0、内结点步长 = 键宽 + `CHILD_DWORDS`、四字全为 `SENTINEL_WORD` 认哨兵、逐槽查键序。与 SPAMAP 只差两处：叶值宽写死 0；键序只拒降序、放过相等（SPAMAP 要严格升序）。

**提议（Q1）**：把 `reference.rs::decode` 抽成按「名 hash + 叶值宽」认表的读侧 keyed-table 节点解码，`V = 0` 给反向表、`V = 7` 给 SPAMAP，与写侧 `rewrite_table::<V>` 对称。反向表的行为与错误类型不变，由既有 reference 测试和 A3 录下的金样守住。`spamap.rs` 在其上补需求 3 第 2 条余下的校验：层号、内结点无子页、页重访、跨页严格升序。设计该节与任务 3.1 的措辞随 A4 一起改。

### 2.2 金样不认样本身份（影响任务 1.3、1.4、11.2、12.3）

- `DbCopy::of`（`tests/common/write_fixture.rs:46`）直接复制 `D:\AVEVA\Projects\E3D3.1\…` 下的原件，而这些原件你还在用 E3D 改：`ams8000_0001` 9-28 被追加会话 641–646，基线里唯一的失败 `hunk_resolve_real` 就是这么来的。
- 设计的金样键是（测试二进制、线程名、`DbCopy::of` 调用点、线程内序号），不含源文件。录完金样后原件只要再被 E3D 存一次，8.13、11.5、12.4 的逐字节对比就全部失败，看着像代码回归。任务 Notes 把这几条列为「不可跳过」。

**提议（Q2）**：

1. 1.3 时建一个钉死的样本快照目录（如 `D:\e3dio-fixtures\phase3\`），从原件复制一次并记下 SHA-256 与长度；`write_fixture.rs` 的源路径优先取快照，快照不在时退回原件。
2. 金样每条另存源文件的 SHA-256；比对时源哈希不符，单独报「样本漂移：期望 X，实得 Y，须重录」，不与字节不符混在一起。

第 1 条让漂移不发生，第 2 条让它发生时一眼认得出。顺带：只在录制模式或金样里有这个键时才求哈希，免得每个副本都多算 34 MB。

### 2.3 基线证据挂在未入库目录（影响 1.1 收尾）

Spec_Ledger「基线数字」写「详见 `.agents/tasks/write-path-phase-3-milestone-a/baseline.md`」，但 `.agents/` 没入库；1.1 对 `tasks.md` 的改动也没提交，`tasks.meta.json` 未跟踪。换台机器或清一次工作树，这本账就断了。

**提议（Q3）**：把 `baseline.md` 存为 `docs/evidence/2026-10-01-write-path-phase-3-baseline.md`（原始日志不入库，文中注明只在本机），Ledger 改指它，与 `tasks.md` 的 1.1 改动同一提交。

### 2.4 Corpus 测试会因环境变量未设而 skip（影响 3.4、5.5、6.3、7.2）

需求 2 第 2 条要求以 Corpus 为输入的测试「0 失败且未跳过」。基线里两条既有 Corpus 测试（`refno_dbno_corpus`、`world_refno_corpus`）因 `E3D_CORPUS_ROOTS` 未设而 skip。第三期的 Corpus 测试若也读这个变量，门槛过不去。

**做法（无须决策，不同意请批注）**：第三期 Corpus 测试按需求「术语」的定义写死清单（`ams8000_0001`、`ams7999_0001`、`ams7351_0001`、`ams251142_0001`、`TES023`），放 `tests/common/`；`E3D_CORPUS_ROOTS` 只作额外扩充。

### 2.5 已核实成立的前提

| 前提 | 核实结果 |
|---|---|
| 1.4 在当前 HEAD 录的金样就是 0.4.0 的字节 | `3e16423` 相对 `c61b097` 在 `src/write/` 只改了 `value.rs` 的 `mod tests`（+67 行），写出字节不变 |
| `write_stage6_*` 的批次用固定时钟 | 全部 `SessionClock::fixed`，只有 `write_stage6_spread.rs:91` 用 `SessionClock::now`（设计已点名剔除）。另：`write_stage6_cli` 真跑 bin，而 `e3d-write.rs:711` 是 `clock: None`，这批也会因两遍不同被剔除；要纳入就得给 CLI 加隐藏的固定时钟参数，本里程碑不做，剔除项记入 Ledger |
| Property 14 能在 stage 与 page 0 之间逼出 `Moved` | `tests/write_stage6_merge.rs:390` 已用 `commit_observed` 做到，并断言 `retried_after_move` |
| 8.7 的下游穷尽 `match` / 结构体字面量 | `rg "TransactionCommit\|TransactionError"` 搜遍 `old/`（排除 target 与临时 worktree），只命中 e3d-io 自己；8.7 动手时按规定再查一次 |
| 取证依赖 | IDA_Bridge 在 `C:\Users\dpc\.agents-old\tools\ida-bridge`（`src\ida_bridge`、`.venv` 都在），Core_IDB `D:\ida_scratch\plant3\core.dll.i64` 在；`IDA_BRIDGE_HOME` 未设 |
| SPAMAP_Baseline 不受原件追加影响 | 按 `Exact(640)` 取；原件当前 SHA-256 `3CAB5D7D…E023`，与基线快照相同 |

### 2.6 风险（不改设计，提前说清）

- **交付取决于第 7 组**：E3D 空间搜索能找到我们写的几何，要到第 8–10 组才看得到；第 7 组能坐实哪些编码规则，决定 Codec 能否在默认构建里执行。哪条坐不实，D7 就让「要求」下相关的几何插入全部拒写——这是设计有意的，但意味着第 7 组的结论直接决定第三期能不能交付。第 5 组的 `spamap-check --rules candidates` 一出一致率就该看，作为早期信号。
- **`spamap.coverage` 是逐 noun 三态**：坐实前，ZONE、EQUI 这类 T3 已见 0 条目的 noun 在「要求」下也会被当成缺 World_AABB 而拒写，要靠 7.2 坐实为「非几何」。
- **`hunk_resolve_real` 既有失败**：它读最新会话，改成按 `Exact(640)` 取就能根治（一行）。里程碑 A 要不要顺手修，见 Q5。

## 3. 里程碑 A：任务第 1–4 组（不碰 `src/write/`）

### 3.1 步骤

| 步 | 任务 | 产出 | 验收 |
|---|---|---|---|
| A0 | 1.1 收尾、§2.3 | `docs/evidence/2026-10-01-write-path-phase-3-baseline.md`；`tasks.md` 1.1 引用改指它 | 提交 |
| A1 | 1.2 | `[dev-dependencies] proptest = "=x.y.z"`（查当日 crates.io 最新稳定版），同步 `Cargo.lock`；`proptest-regressions/` 不进 `.gitignore` | 相对基线不新增失败 |
| A2 | 1.3、§2.2 | `tests/common/golden.rs`；`DbCopy` 加 `Drop` 与 `#[track_caller]`；`tools/golden/record.ps1`；样本快照与源 SHA | 无金样时行为不变；人为改副本一个字节能报出；改源文件报「样本漂移」 |
| A3 | 1.4 | `tests/golden/off_compat.json`；Ledger「Golden 剔除键」，备注所用样本的 SHA-256 与长度 | 两遍录制一致；剔除项逐条附原因 |
| A4 | 3.1、3.2、3.3\*、§2.1 | keyed-table 解码；`src/index/spamap.rs`；`tests/common/spamap_pages.rs`；`tests/spamap_reader_props.rs`；设计与任务措辞更正 | Property 1、3 过；既有 reference 测试与 A3 金样不变 |
| A5 | 3.4\*、3.5、3.6\*、§2.4 | `e3d-descriptor spamap` / `spamap-census`；`tests/spamap_reader_real.rs`；`tests/spamap_census_real.rs`；Corpus 清单 | SPAMAP_Baseline 实读填入 Ledger（期望 244 页 / 3 746 条 / 3 628 个 refno / k=0 3 625 条 / k=1 121 条）；Property 4 过 |
| A6 | 4.1、4.2、4.3\* | `src/core_rules.rs`；`src/spatial/{mod,state}.rs`；`tests/core_rules_registry.rs` | 规则全为 `Unconfirmed`；const 断言在；`src/write/` 下不出现 `candidates` |
| A7 | 2.1、2.2、2.3、2.4\* | `tools/ida/`：`ib.ps1`、`xrefs_to.py`、`queries/xrefs_to.sql`、`check_xrefs.ps1`、`EVIDENCE_TEMPLATE.md`、`check_evidence.ps1`、`selftest.ps1` | `selftest.ps1` 离线过（退出码 90 / 91 / 92 三条路径） |
| A8 | 2.5\*（要起桥与 idalib） | Ledger「IB_Wrapper 实机冒烟」 | 多实例 / 被占用 / 超时三条路径；`check_xrefs.ps1` ≥ 5 个地址；`env` 首尾哈希相同 |
| A9 | 3.7 | `docs/evidence/2026-10-xx-spamap-census.md`，并更正 09-22 取证 §0 第 7 条 | `check_evidence.ps1` 机检过 |
| A10 | 2.6、3.8 | Build_Checks 对照基线 | 相对 626 / 1 / 30 不新增失败；clippy 0 error；`cargo doc` warning 不增 |

顺序约束：A2 → A3 → A4（金样先录，A4 改 `reference.rs` 时才有字节守门）；A7 的 `check_evidence.ps1` 先于 A9。A5、A6、A7 之间互不依赖。

### 3.2 不在里程碑 A

第 5–6 组（Codec、candidates、Built_In_AABB、`spamap-check`、`world-frame`）、第 7 组取证、第 8 组起的全部写侧。支线 11–13 里只有 12.1（Scale_Probe 骨架）在 wave 0，本里程碑不做，留给 P 链统一排。

### 3.3 提交与节奏

任务约定「何时提交由用户确认」。提议每步一个提交，首行写任务号（如 `test(golden): 1.3 …`）；每步做完在 BajieAsk 回报 diff 摘要与测试数字，你回「提交」再提交（Q4 可改为批量授权）。cargo 命令沿用 `CARGO_TARGET_DIR=target-phase3`。

## 4. 待你拍板

批准本计划 = 全按推荐项（A）。要改哪条，在对应行批注。

| Q | 问题 | 推荐（A） | 备选 |
|---|---|---|---|
| Q1 | SPAMAP 读侧解码放哪（§2.1） | 把 `reference.rs::decode` 抽成按叶值宽参数化的 keyed-table 解码，SPAMAP 与反向表共用 | B：`spamap.rs` 自写一份（约 100 行重复，反向表零改动）；C：放宽 `IndexNode` 的 noun 检查（不推荐：`entry` 解码是 refno 索引专用） |
| Q2 | 金样与样本漂移（§2.2） | 钉死样本快照 + 金样存源 SHA，漂移单独报 | B：只存源 SHA；C：照设计不改 |
| Q3 | 基线证据入库（§2.3） | 存为 `docs/evidence/…-baseline.md`，与 `tasks.md` 1.1 同提交；`.agents/tasks/` 与 `tasks.meta.json` 加进 `.gitignore` | B：同 A，但那两项保持未跟踪、不进 `.gitignore`；C：`.agents/tasks/…` 整个入库（含 643 KB 原始日志） |
| Q4 | 提交授权 | 每步回报，你回「提交」再提交 | B：里程碑 A 内按 §3.3 格式自动提交，结束统一回报 |
| Q5 | 可选测试与顺手修 | 第 1–4 组的 `*` 可选测试全做（3.4 本就是门槛要的 Corpus 测试）；`hunk_resolve_real` 改按 `Exact(640)` 根治 | B：可选测试全做，`hunk_resolve_real` 不动；C：跳过可选测试 |
| Q6 | 里程碑范围 | 第 1–4 组（本计划） | B：扩到第 1–6 组（连 Codec、AABB 候选）；C：只做第 1 组再议 |

## 5. 执行记录

- **2026-10-03 13:45 评审**：Plannotator `annotate --gate` 结果 `{"decision":"approved"}`，无批注；用户在 BajieAsk 回「按推荐项直接开工」。
- **A0（1.1 收尾，Q3）**：基线快照存为 `docs/evidence/2026-10-01-write-path-phase-3-baseline.md`（原始日志留本机）；Spec_Ledger「基线数字」改指它；`.gitignore` 加 `/.agents/tasks/` 与 `.kiro/specs/*/tasks.meta.json`。
- **Q5 顺手修**：`tests/hunk_resolve_real.rs` 改用 `ReadOnlyEngine::open_at(AMS8000, 640)`，单跑通过。
- **A1（1.2）**：`[dev-dependencies] proptest = "=1.11.0"`（crates.io 当日最新稳定版，MSRV 1.85；本机 rustc 1.100.0-nightly）；`Cargo.lock` 只新增 proptest 及其依赖，既有包版本不变。全套 `cargo test --no-fail-fast`：101 个测试二进制 + 1 个 Doc-tests，**627 / 0 / 30**，55 s——相对基线 626 / 1 / 30 只多出 Q5 修好的那一条。
- **提交**：用户回「提交，后续步骤自动提交」→ 里程碑 A 余下各步改按 Q4 备选 B（自动提交、结束统一回报）。`e5729ae` 计划、`0a76cc8` 1.1 收尾、`1b9d1b5` `hunk_resolve_real`、`b2c675d` 1.2。
- **A2（1.3，Q2）**：`tests/common/golden.rs`（键 = 测试 crate + 线程名 + 线程内序号；`DbCopy::of` 的调用点只进报错信息——**偏离设计**：8.10、11.8 会在被守的测试里加行，按行号成键会让编辑点以下的金样静默失配，design 与 tasks 1.3 已同步改）；`DbCopy` 加 `Drop` 与 `#[track_caller]`，源取 `copy_source`（快照优先）；金样另存源 SHA，漂移与字节不符分开报；只在录制或金样有该键时才求哈希。`tools/golden/snapshot.ps1` 已钉 `D:\e3dio-fixtures\phase3`（ams8000_0001 34 037 760 字节 `3CAB5D7D…E023`、ams5100_0001、ams251142_0001、TES023，见 `MANIFEST.tsv`）；`tools/golden/record.ps1`。自检 `tests/golden_fixture.rs` 3 条（键形、一字节之差与样本漂移分开报、只读 `*.json`），`--test-threads=1` 下线程名照样是测试名。全套 102 个测试二进制 **630 / 0 / 30**，37 s；clippy 63 warning / 0 error，与基线相同。提交 `d040f18`。
- **A3（1.4）**：`tools/golden/record.ps1 -Out tests/golden/off_compat.json -Test write_stage6_{transaction,merge,set_by_name,spread,cli}` 在 `d040f18`（干净树）上两遍，34 s：留 **17 条**，剔除 6 条——`write_stage6_cli` 5 条（`e3d-write` 用 `clock: None`）与 `write_stage6_spread` 1 条（`SessionClock::now`），正是 §2.5 预判的两类，无意外的不确定性。17 条全取自快照 ams8000_0001（`3CAB5D7D…E023`）。比对模式端到端核过：正常跑 5 个二进制全过；把一条的写出哈希改掉 → 该测试失败并报「ends as … recorded …」；把源哈希改掉 → 报「sample drift, not a code change」；复原后全过。首次实跑发现 `record.ps1` 里 `$runs` 与 `[int]$Runs` 撞名（PowerShell 变量不分大小写），改名 `$records`。提交 `3e57a4d`。
- **A4（3.1、3.2、3.3，Q1）+ 3.4**：`src/index/keyed.rs` 的 `decode::<V>` 由 `reference.rs::decode` 抽出（反向表 `V = 0`、SPAMAP `V = 7`），`reference.rs` 改为调用它并把错误一一映射回原 `ReferenceTableError`、键序检查留在本表，6 条既有单测全过；`TABLE_PAGE_TYPE` 移入 `keyed.rs`，`reference::TABLE_PAGE_TYPE` 以 `pub use` 保留原路径。`src/index/spamap.rs`：`read_spamap(view)` / `read_spamap_tree(root, read_page)`、`SpamapRead::{NoTable, Table { sesno, table }}`、`SpamapTable::{entries, for_refno}`（首次查询建 refno 索引）、`SpamapReadError` 带页号；需求 3 第 2 条十一项全查。API 相对 design 两处小改：会话由 `DbView::open_at` 选定（不再另传 `SessionSelector`）、读页闭包收 `PageId` 且不再传页长，design 已同步。`tests/common/spamap_pages.rs` 建树器（不经 `src/write/`）；`tests/spamap_reader_props.rs` Property 1、3 各 256 例 + 空表 1 条，首跑即过。**真库**：`tests/spamap_reader_real.rs` 读 SPAMAP_Baseline（原件会话 640）得 **244 页 / 3 746 条 / 3 628 个 refno / k=0 3 625 / k=1 121，与需求 3 第 3 条五项全符**；Corpus 各库最新会话按 refno 查询全对，`ams7351_0001` 会话 116 无 SPAMAP 表。`tests/common/corpus.rs` 写死需求的五个库（§2.4；未接 `E3D_CORPUS_ROOTS`——它在既有测试里指整套安装目录、递归遍历，含义不同）。全套 104 个测试二进制 **635 / 0 / 30**；clippy 63 / 0 error（`reference.rs` 的两条 `manual_is_multiple_of` 随代码挪进 `keyed.rs`，总数不变）；`cargo doc` 25，同基线。提交 `7e3f43b`。
- **A5（3.5、3.6）**：`e3d-descriptor spamap --db [--session] [--refno]`（条目按 11 个盘上原字、无符号输出——Key0 可过 2^31，按有符号印会改掉原值）与 `spamap-census --input … [--session FILE=N]…`（自带参数解析：`--input` / `--session` 可重复，共用的 `parse_options` 一键一值装不下）。普查按需求 3 第 7、10–12 条出：每库会话号、根页 / 页数 / 条目数或 `no_table`、索引元素数、悬空条目数与前 100 条、Key0 不符条目数与前 100 条（按库列，按 noun 的直方图里另有 `key0_mismatch` 一格）；逐 noun：元素按条目数 0 / 1 / 2 / 3+、(轴序, 桶)、flags、k、按元素的 k 组合。(轴序, 桶) 的普查口径照 09-22 取证 §0 第 7 条（`key0 = N × 10^8 + 某轴有符号 min`，`N = 6 × 轴 + 桶 1–5`），口径原文随输出印出。目录里首页解析不出库描述的文件列为非库文件；读不了的库列错误、其余照算、最后退出码 1。实跑：ams8000 会话 640 = 3 746 条 / 244 页 / 6 639 个元素 / 0 悬空 / 0 Key0 不符，BOX 45 个各 1 条、CYLI 34 个各 1 条、EQUI 77 个 0 条，与 09-22 取证一致。`tests/spamap_census_real.rs` 3 条（Property 4：输入前后 SHA-256 不变、两遍输出逐字节相同、非库文件与截短库各归其位、退出码 1 / 0 / 2）。全套 105 个测试二进制 **638 / 0 / 30**；clippy 63 / 0 error，同基线。提交 `45c6ba4`。
- **A6（4.1、4.2、4.3）**：`src/core_rules.rs`——`RuleId`（27 条，`#[non_exhaustive]`）、`RULES`（`const`，好进 const 断言；每行另带 `governs` 一句）、`RuleStatus`；每条规则一个槽 `Slot<T> { Pending, Confirmed(T), Candidate(T) }`，**各实现类型都是空枚举**，于是 `RuleSet::confirmed()` 里只能是 `Pending`，「不预设任何取值」由类型系统担保；const 断言：确认集里非 `Pending` 的槽必须在 `RULES` 里是 `Confirmed`、且确认集不含候选；`#[doc(hidden)] RuleSet::candidates(&CandidatePick)`（候选未写，暂返回确认集）；`recompute_row` / `member_order_row` 恒 `Unconfirmed`，`descendants_included` 恒 `false`（D7、D8 唯一落点）。`src/spatial/mod.rs`：`Axis`、`WorldAabb::{new, from_mm}`（非有限、min > max 即拒）、`SpatialPolicy`（`#[default] Require`，D3 唯一落点）、`SpatialState`、`UndecodableAttr`；`src/spatial/state.rs` 的 `ReadState` 按会话经描述子路径读元素并缓存，读不了的记录列入 `failures()`。`SpatialState::attr` 返回 `DescriptorValue`（design 写的是 AttrValue；读写两侧共用的解码值就是它，design 已改）。`tests/core_rules_registry.rs` 4 条（名唯一、Confirmed 规则的出处「结论」节含规则名、`src/write/` 不出现 candidates、结论节查找器自检）；`tests/spatial_read_state_real.rs`：会话 640 的原生 BOX `24384/24777` 读出 XLEN 310 / YLEN 590 / ZLEN 550、owner 链 BOX ← EQUI ← ZONE ← SITE ← WORL，与 09-22 取证一致。全套 107 个测试二进制 **645 / 0 / 30**；clippy 63、`cargo doc` 25，同基线。提交 `5eaa810`。
- **A7（2.1–2.4）**：`tools/ida/ib.ps1`（PS 5.1，无 param 块，`-c` / `-f` 原样进 `$args`）——查找顺序照需求 1.1 写死三处、缺项逐条报（90）；子进程 `PYTHONPATH` 放所选 `src` 并自检 `import ida_bridge` 落在它下面（90）；请求前拒存盘：`--save`、`supervisor save`、`--out-idb` 指向 Core_IDB（不分大小写、`/` `\` 同）、`-c` / `-f` 含 `idb.save` 或 `save_database`、`supervisor stop` 指向 `idaui-<n>` 或 PID（91）；2 s 连桥（92，印所试地址与起桥 / start-idalib 命令，Target_IDB 不存在时只说不存在）；`exec` / `reset` / `supervisor stop` 未给实例号时按 `list --json` 的 `meta.idb_path` 解析（93 无、94 多个或被他人 stateful 占用并逐个列出）；`exec` 时限默认 60 s，到时 `taskkill /T` 只停客户端并印停 / 重开命令（95）；其余照转退出码。子命令 `env`（IDB 路径与 SHA-256、实例所报 IDA 版本、HEAD）、`xrefs 0x…`。核对 ida-bridge 实装后在脚本头注明：入口 `python -m ida_bridge`；`list --json` 字段；**`exec` 按 `--sql` → `-f` → `-c` 执行**，故 design 写的「先 `-c XREF_TARGET=` 注入再 `-f`」行不通，改为文件定义 `xrefs_to(ea)`、`-c` 调用（design 已改）；`supervisor stop`：idalib 退出时 `close_database(save=False)`、UI 经 `idc.qexit(0)` 关库即写盘，故后者拒绝（design 已注）。`xrefs_to.py`（`XREF_NOFLOW` / `XREF_FAR`、BADADDR 或满 100 000 条即停、截断标 `truncated`）、`queries/xrefs_to.sql`、`check_xrefs.ps1`、`EVIDENCE_TEMPLATE.md`、`check_evidence.ps1`（中文节名按码点拼，避开 PS 5.1 按 ANSI 读无 BOM 文件）、`selftest.ps1`。**离线自检 11 项全 PASS**（90 × 2、92 × 2、91 × 7，91 各项期间本地监听口零连接）。`check_evidence.ps1` 对样例：好稿只报「tools/ida 文件未入库」（提交前预期），坏稿逐条报出非入口命令、临时目录、实例号、非十六进制。提交 `ebad935`。
- **A8（2.5）**：本机桥与 `idalib-4000`（开着 Core_IDB）是常驻环境，未另起实例，只做只读查询，借用后均已归还。实机：未给实例号的 `exec` 自动落到 `idalib-4000`；被占用（94，持有者与释放均如期，释放后无占用）；超时（95，3.85 s 返回，实例存活）；`check_xrefs.ps1` 6 个地址全 PASS，含 `__security_check_cookie` 10 516 条与 `operator new` 10 319 条；`env` 首尾 SHA-256 同为 `18D5C155…992C`（Property 25）。途中修了 `check_xrefs.ps1` 两处：ida-bridge 的 SQL 结果行是按列名的对象而非数组；单次结果超过 10 000 行即被拒（`result exceeded transport limit`），改为分页读。多实例实机构造不出（IDA 锁住已开的 IDB），`selftest.ps1` 增一组替身 ida_bridge 用例覆盖 93 / 94 / 转发（共 18 项全 PASS）。结果填入 Spec_Ledger「IB_Wrapper 实机冒烟」。提交 `7d7d67d`。
- **A9（3.7）**：普查加每库 `totals`（`f917440`），在 `f917440` 上首末各跑 `ib.ps1 env`（Core_IDB SHA-256 不变），Corpus 五库按普查时最新会话逐库钉死（ams8000 646 / ams7999 200 / ams7351 116 / ams251142 10 / TES023 109）一次、SPAMAP_Baseline（会话 640）一次，输出入库为 `docs/evidence/assets/2026-10-03-spamap-census-{corpus,baseline-640}.json`，重跑逐字节相同。写 `docs/evidence/2026-10-03-spamap-census.md`，`check_evidence.ps1` PASS。要点：四个有表的库共 28 269 条、基线 3 746 条，**悬空 0、普查口径下 Key0 不符 0**；会话 641–646 没动 SPAMAP；`ams7351_0001` 是目录 / 数据库（DATA 250 万、DTSE 74 万…），无几何、无表；flags 只见 0 / 1 / 2 / 3 / 6 / 7，1 只在电缆桥架库；`TES023` 五个桶三轴齐全，最宜拟合桶界。**更正 09-22 取证 §0 第 7 条**：flags 直方图（3 747）是副本会话 648 的表（= 会话 640 + E3D 给 e3d-io BOX 补的那条 flags 2）；N 直方图（3 746）与会话 640 有两格不同（x3 49 / 48、z1 1 250 / 1 251），两张表都对不上，副本已重置无从复核，以会话 640 普查读数为准；原文已加更正注。
- **A10（2.6、3.8）**：Build_Checks 对照基线——**107 个测试二进制 + 1 个 Doc-tests，645 / 0 / 30**（基线 101 / 626 / 1 / 30：多 6 个二进制、19 条通过，既有失败修掉）；clippy 63 warning / 0 error、`cargo doc` 25，均同基线；报 skip 的只有既有两条 `E3D_CORPUS_ROOTS`，本里程碑新增测试无一 skip；`selftest.ps1` 18 项全 PASS。

### 里程碑 A 小结与下一步

对设计的四处改动都已回写 design / tasks：SPAMAP 读侧走 `keyed.rs`（不经 `IndexPageView`）；金样键不含调用点、副本取自钉死快照；`SpatialState::attr` 返回 `DescriptorValue`；`xrefs_to.py` 由 `-f` 定义、`-c` 调用。下一里程碑按依赖排第 5–6 组（SPAMAP_Codec、`candidates.rs`、`spamap-check`、Built_In_AABB、`world-frame`），随后第 7 组取证——本普查与 `spamap-check --rules candidates` 的一致率是 7.1 的早期信号。另起计划经 Plannotator 评审后开工。
