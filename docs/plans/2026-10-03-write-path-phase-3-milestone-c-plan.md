# 计划：e3d-io 写路径 · 第三期 · 里程碑 C（任务第 7 组前半：core 取证——编码、覆盖、记录项、AABB）

状态：**评审通过**（2026-10-03，Plannotator 门 `approved`，无批注 → Q1–Q5 全按推荐项）→ 里程碑 C 进行中（见 §6）
日期：2026-10-03
分支 / worktree：`write-path` @ `D:\work\plant-code\old\vendor\e3d-io-write`，HEAD `95f751f`（里程碑 B 收尾，领先 `github/write-path` 24 个提交，未推送）
依据：`.kiro/specs/write-path-phase-3/` 需求 1、2、4、5、6 与任务第 7 组；`docs/plans/2026-10-03-write-path-phase-3-milestone-b-plan.md` §6（候选与反例）
上一里程碑：`docs/plans/2026-10-03-write-path-phase-3-milestone-b-plan.md`

---

## 0. 一句话

B 把编码与 AABB 的每条规则都落成了候选，并在 Corpus 上核出一致率（Key0 28 269 / 28 269、BOX / CYLI 的 flags 1 590 / 1 590、Built_In_AABB 1 572 / 1 590），但一条也没坐实。C 读 `core.dll` 把它们坐实，或写明为何坐实不了：产出 7.1、7.2、7.4、7.5 四份 Evidence_Doc，坐实的规则在同一提交里把 `src/core_rules.rs` 改为 Confirmed。要驱动真 E3D 的重算取证（7.3）与门槛核查（7.7）留给里程碑 D（Q2）。

## 1. 现状

| 项 | 状态 |
|---|---|
| HEAD | `95f751f`；110 个测试二进制 669 / 0 / 30，clippy 63、`cargo doc` 25（同基线） |
| 已有、C 直接用 | `tools/ida/ib.ps1`（IB_Wrapper）、`check_xrefs.ps1`、`check_evidence.ps1`、`EVIDENCE_TEMPLATE.md`；`e3d-descriptor spamap-check`（四项）与 `world-frame`；`tests/spamap_codec_real.rs` / `spamap_aabb_real.rs` 在确认集下断言 `agree + waiting = total`，规则坐实后自动变成「一致数 = 总数」 |
| 环境 | 桥与 `idalib-4000` 常驻，开着 Core_IDB（SHA-256 `18D5C155…992C`，IDA 9.2，与 A8 相同） |
| C 的任务 | 7.1 编码规则、7.2 覆盖与 Property 7、7.4 SPAMAP_Record_Item、7.5 AABB 规则；7.6 的 4.20 部分按 Q3 |
| 依赖 | 第 2、3、5、6 组已完成；7.6 的 6.17 部分依赖 8.7，不在 C |

## 2. 开工前的侦察（2026-10-03，只读，经 IB_Wrapper）

| 发现 | 出处 | 对 C 的意义 |
|---|---|---|
| SAVEWORK 时 `DB_DB::internalFlush` @ `0x5907CA0` 先调 `DB_DB::getSpatialMap` @ `0x5905510`，把「有 SPAMAP 表」作末参数交给 `sub_5AADB30`（真正的 flush） | `internalFlush` 伪代码 n 38–58 | 7.1 / 7.3 的入口：从 `sub_5AADB30` 的「有表」分支往下找条目的删除、登记与键的计算 |
| `getSpatialMap` 另被 `internalFlushWithoutOrGlobal`、`getRequiredClaimsForVariantFlush` 调用 | xrefs | 合并 / 变体 flush 也维护表（8.8 用得到） |
| `DB_Noun::spatialMap()` @ `0x58DB9D0` 返回 noun 对象 +0xA8 处的字（未读则先 `ReadData`）；core.dll 内没有调用方，只在导出表里 | 伪代码、xrefs | 7.2 的强线索：覆盖可能是 noun 定义里的一个字段；若它来自 noun 模板，e3d-io 可照模板读出，`spamap.coverage` 就不必靠语料清单 |
| `DB_Spatial` 是可替换的单例（`instance` / `setInstance`），core.dll 里只有 `DB_SpatialDummy` 一种实现 | names | 空间**查询**（`getElementsInBox`）可能由别的模块注册；表的**维护**仍在 core.dll 的 flush 里（第一行） |
| 立即数 `0x0DC34AB5`（SPAMAP 名 hash）只出现在 `sub_544FCF7` @ `0x54500AC` 一处，调用方 `sub_544E4DF`；09-22 取证记过类型 14 键控属性的读写路径 `sub_5AB91B0` | instructions、xrefs | 7.4 的入口：记录里的 `SPAMAP` 属性（类型 14；原生 BOX 的值 `[299969986]` 即其 Key0）从这里写 |
| core.dll 导入 `libgeom`（54 个函数）；`D:\AVEVA\Everything3D3.1` 另有 `libgeom.dll`、`libgm.dll`、`Core3D.dll` 的 IDB | imports、目录 | BOX / CYLI 的包围盒算术可能落在 libgeom 里——Q1 |
| 立即数 10^8 出现在 11 个函数，抽看的 `sub_5528BDE` 是 FORTRAN 命名例程（误报） | instructions | 按常量找 Key0 不可靠，改从 flush 入口顺下去 |
| IDB 有 31 MB 的 `.vlizer` 段（代码虚拟化） | segments | 所需例程若被虚拟化，反编译不出——§3.2 |

候选侧的数据（B，§6）：Key0 28 269 / 28 269；条目数与 k 差的恰是带 k = 1 条目的元素；BOX / CYLI 的 flags 1 590 / 1 590；Built_In_AABB 1 572 / 1 590，各备选都更差，18 个反例全在 ams7999、无一差 ±1，多数像祖先改动后条目没有重算。

## 3. 复核与风险

### 3.1 AABB 算术可能不在 core.dll（影响 7.5，要拍板：Q1）

需求把 E3D_Core 定为 `core.dll`，「坐实」出处 ① 要的是 Core_IDB 里的地址。若 BOX / CYLI 的包围盒由 core.dll 调 libgeom 的导出函数算出，① 只能落在 core.dll 的调用点上（传了什么、取回什么）；libgeom 里面的算术要么另开 libgeom 的 IDB（spec 要改），要么只靠出处 ②（Corpus 复现、0 反例）。走 ② 时那 18 个反例须先按需求 4.17 剔除，而剔除要 7.3 坐实「祖先改位姿不重算后代」——那在 D。

### 3.2 代码虚拟化

`.vlizer` 段里的例程反编译不出。遇到时记下进入虚拟化段的调用点，作为 ① 的边界；规则按 ② 坐实或标未坐实。不做动态调试（spec 未覆盖）。

### 3.3 最短轴并列、零 extent 只能以 ① 坐实（需求 4.18）

语料分不出并列时取哪根轴，B 的候选「取最后一根」只是与语料不矛盾。7.1 必须在反编译里找到比较顺序；找不到就保持未坐实，写侧遇到并列或零 extent 的元素按 D7 回退。

### 3.4 条目数与 k（`spamap.entries_k`）

k = 1 的条目 B 没有候选，只知道「k = 1 的框与同元素 k = 0 的框全不同」。7.1 要从 flush 里找出第二条目从哪来；若来源是调用方给不出的输入（如目录几何），按需求 6.15 记为 MissingInput，写侧对这些元素回退。

### 3.5 4.20 示例需要「E3D 为它写出条目」的那一会话

需求 4.20 的输入是 BOX `24384/26484` 在 E3D 为它写出条目那一会话中的状态（09-22 门用副本的会话 648），现在的原件与快照里都没有这一会话。7.6 的 4.20 部分要么随 D 的探针让 E3D 再改一次 `XLEN`，要么用 B1 的合成状态单测代替——Q3。

## 4. 里程碑 C：步骤

每份 Evidence_Doc 按 `tools/ida/EVIDENCE_TEMPLATE.md` 写、过 `check_evidence.ps1`；每轮首尾跑 `ib.ps1 env`，本轮用到的地址（至少 5 个，含一个被大量引用的）跑 `check_xrefs.ps1`。坐实的规则在同一提交里落三样：Evidence_Doc、`core_rules.rs` 的实现变体与 `RULES` 状态、用到它的 `src/spatial/` 代码（放哪见 Q4）。

| 步 | 任务 | 产出 | 验收 |
|---|---|---|---|
| C0 | 7.1 前置 | 从 `sub_5AADB30` 的「有表」分支追到条目的删除 / 登记，以及取整、最短轴、桶、Key0、k、flags 各自的例程；判定 AABB 从哪来（core.dll 自算 / libgeom / 别的回调）。只写进 7.1 文档的「入口」节，不提交代码 | 每个编码步骤都有一个函数地址，或明确写出「追不到」及原因 |
| C1 | 7.1 | `docs/evidence/<日期>-spamap-encoding.md`：`spamap.{word_sign, rounding, short_axis, zero_extent, bucket, key0, entries_k, flags}` 与 `table_shape`、`table_header_rest`；坐实的改 Confirmed | 每条规则有 ① 或写明未坐实；Key0 链上的规则都坐实时，`spamap-check --rules confirmed` 在 Corpus 上 Key0 一致数 = 总数；Property 5 照过 |
| C2 | 7.2 | `…-spamap-coverage.md`：判定条件（noun 字段，及 noun 之外的因素）、逐 noun 三态、按 Corpus 所用 noun 模板列 Geometric_Noun 清单；`tests/spamap_codec_real.rs` 加 Property 7 覆盖对账 | 坐实前以候选跑、只作数据；坐实后每库 0 反例 |
| C3 | 7.4 | `…-spamap-record-item.md`：逐 noun 是否写类型 14 的 `SPAMAP` 属性，及其字数、布局、与 Key0 / k 的对应；Corpus 中带与不带的个数（`spamap-census` 增一项，或测试里统计） | 结论写入 `spamap.record_item.<NOUN>` |
| C4 | 7.5 | `…-spamap-aabb.md`：`aabb.{ori_form, compose, box, cyli, degenerate, arithmetic}`，取证源按 Q1；18 个反例按会话链逐个记下条目何时写、此后元素或祖先改过什么，作 D 的剔除清单草稿 | 每条规则有出处或写明未坐实；坐实的补进 `aabb.rs` 并改 Confirmed |
| C5 | 7.6（4.20 部分） | 按 Q3（推荐：留给 D，C 只在 Spec_Ledger 记待办） | — |
| C6 | 检查点 | Build_Checks 对照基线；Spec_Ledger「规则坐实状态总表」填 C 的规则（状态、Evidence_Doc、提交） | 不新增失败，clippy / doc 不增 |

顺序：C0 → C1 → C2 / C3 / C4（互不依赖，按 C0 追到的入口先后做）→ C5 → C6。

## 5. 待你拍板

批准本计划 = 全按推荐项（A）。要改哪条，在对应行批注。

| Q | 问题 | 推荐（A） | 备选 |
|---|---|---|---|
| Q1 | AABB 规则的取证源（§3.1） | 只认 core.dll：① 落在 core.dll 的调用点与参数，libgeom 内的算术按 ②（B8 的 Corpus 复现）坐实；18 个反例待 D 剔除后才可能坐实 | B：另开 libgeom（或 Core3D）的 IDB 作出处，需求 1 与术语表改为允许「core.dll 及其导入的 libgeom.dll」；C：`aabb.*` 全按 ②，7.5 不读 IDA |
| Q2 | 里程碑范围 | 7.1、7.2、7.4、7.5（7.6 的 4.20 按 Q3）；7.3 重算取证、7.7 门槛核查、7.8 留给 D | B：整个第 7 组，含驱动真 E3D 的 7.3 探针；C：只做 7.1，做完再定 |
| Q3 | 4.20 示例（§3.5） | 留给 D：随 7.3 的探针让 E3D 改一次 BOX 的 `XLEN`，用那一会话作输入 | B：现在就跑一次门，得到那一会话；C：以 B1 的合成状态单测代替，Spec_Ledger 记偏差 |
| Q4 | 坐实规则的实现放哪 | 新建 `src/spatial/rules.rs`（不隐藏）放坐实的实现，`core_rules.rs` 的变体调用它；`candidates.rs` 只留未坐实的候选与备选 | B：直接写在 `core_rules.rs` 的变体旁；C：仍放 `candidates.rs`，确认变体转调（「`src/write/` 不出现 candidates」的测试要改） |
| Q5 | 提交节奏 | 沿用：每步自动提交，结束统一回报 | B：每步回报、你回「提交」再提交 |

## 6. 执行记录

（空）
