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

### C0 入口（2026-10-03，只读，经 IB_Wrapper；查询脚本 `tools/ida/pseudo.ps1`、`tools/ida/sql.ps1` 本步新增）

**结论：SPAMAP 的条目计算不在 core.dll，在 Core3D.dll 的 FORTRAN 库 `maplib` / `mapboxlib` / `dbboxlib`**；core.dll 只出三样：类型 14 键控表属性的通用读写（表与记录项）、noun 字段 IMAP、盒算术 `boxlib`。§2 第一行的入口不成立：`sub_5AADB30` 只是例程 380 的包装（`sub_5AC0810(380, …)`，extract 间刷写），`internalFlush` 也不是 SAVEWORK；core.dll 里 SPAMAP 名 hash 的唯一立即数 `sub_544FCF7` 是完整性检查 `integ/GTTREF`。

Core3D_IDB：`D:\ida_scratch\plant3\Core3D.dll.i64`（桥上常驻，C0 结束时文件 SHA-256 `03129C29…A712`；Core_IDB 首末仍是 `18D5C155…992C`），IDB 记录的输入 SHA-256 `E43EC507…D790` 与门用 E3D 的 `Core3D.dll`（`E:\reverse\e3d\shadow_e3d31_gen_model_test`）相同。Core3D 导入 `ATT_SPAMAP`、`DB_Spatial::setInstance`，`DB_SpatialActual` / `CLI_SpatialMap::BuildSpatialMap` 都在它里面（安装目录里另有 AfiModeling、FunctionalModelling、MarAccessor 引用 `ATT_SPAMAP`，未读）。

| 环节 | 例程 | 读到的行为 | 对照 B 的候选 |
|---|---|---|---|
| 走树 | Core3D `mapboxlib/MPBOX` @ 0x102FB475 | 逐级取 noun 的 IMAP（`ATNINT(noun, 847458)`，指令 @ 0x102FB8E7）：1 / 5 → `MPBOX2` 记本元素；7 → 看 owner 的 IMAP，为 2 当 1、否则当 3；0 → 不再下探；BRAN 与 `HTCOMP` 真者走 `MPBXBR`（带 tube） | 覆盖 = noun 的 IMAP（7.2） |
| 取框 | Core3D `MPBOX2` @ 0x102FBD62 → `dbboxlib/BOXELE` @ 0x1036A31C | 元素有 WBOX 就用它，flags 按 OBST 1–3 置第 OBST 位；否则 BOXELE 按 noun 的 BOXTYP 分派：3 = XLEN/YLEN/ZLEN → core `MBXBOX`，2 = HEIG/DIAM → core `MBXSNO`，这些尺寸小于 0.5 一律取 0.5；OBST 1–2 置 flags 第 OBST 位；IMAP 5 再并入 IMAP 3 成员的框 | `aabb.degenerate`：负尺寸也取 0.5，不是取绝对值 |
| 盒算术 | core `boxlib/MBXBOX` @ 0x55FACE8、`MBXSNO` @ 0x55FCA18、`MBXPOI` @ 0x55FB7B4 | BOX：中心 ± Σ\|R·半长\|；CYLI：两端圆（`MBXCIR`）各自的框求并 | box 同候选；cyli 待读 `MBXCIR`（B8 的 2.1 mm） |
| 插入 | Core3D `mapboxlib/MBXGO` @ 0x102FD8C2 → `maplib/MAPINS` @ 0x1038F5E4 | 框有效 → MAPINS，否则 `MAPREM` 删条目。MAPINS：每轴 datum = trunc（\|值\| 较大的一端 / 10^8 mm）；取整 `f_ldnint_val(10 × (坐标 − datum × 10^8))`（libifcoremd，Fortran DNINT）；flags = 三个逻辑量按 1 / 2 / 4 相加；下标 = tube ? 2 : 1；旧条目六个值都在 ±1 内、flags 与 datum 不变、旧 Key0 与旧框相符（±1）则不重写；`BTSELE(SPAMAP)` 不过不写；`DPUTIT(SPAMAP, 下标, Key0, 7 字)` 交 core | rounding 同候选；k = 1 是 tube 条目；「±1 不重写」是 7.3 与 B8 反例的线索 |
| Key0 | Core3D `maplib/MAPKEY` @ 0x1039277C、`MAPKY2` @ 0x1039210C | 最短轴：z ≤ x 且 z ≤ y 取 z，否则 y ≤ x 且 y ≤ z 取 y，否则 x（并列取后一根）；桶按最短 extent：< 600、< 2 000、< 6 000、< 20 000、< 200 000 依次 1–5，否则桶 6 且轴强制 x；min 先 `% 5×10^7` 再夹到 ±5×10^7；Key0 = min + 10^8 × (6 × (轴 − 1) + 桶) | 轴、桶 1–5、Key0 同候选；新增桶 6 与 ±5 km 取模 |
| 表与记录项 | core `DPUTIT` @ 0x5AB0E60 → 例程 147 → `sub_5AB91B0` | 类型 14：键 = [值, refno 高, refno 低, 下标 − 1]，删旧键、插新键，条目值 = 调用方给的 7 字；523 时 `sub_5AFF6D0` 建表；最后 `4SCEA` 写记录项（各下标的 Key0 数组，空槽 0x80000001） | `table_shape`、`record_item` 可在 core.dll 坐实 |
| 覆盖字段 | core `DB_Noun::spatialMap` @ 0x58DB9D0 ← `DB_Noun::ReadData` @ 0x58D6D20 n 79 | 字典属性 847458（IMAP）读进 noun +0xA8；core 内无调用方，Core3D 经 `ATNINT` 读同一字段 | |

**影响**：需求 1 把 E3D_Core 定为 core.dll，出处 ① 只认 Core_IDB。照已批的 Q1 = A，7.1 的八条编码规则与 `aabb.ori_form` / `compose` / `degenerate` 都拿不到 ①，需求 4.18 的并列取轴也就坐实不了；能落在 core.dll 的只有 `table_shape`、`table_header_rest`、`record_item` 与 `aabb.box` / `cyli` 的盒算术。是否把 Core3D.dll 纳入 E3D_Core 交用户拍板，定之前不进 C1。

**用户拍板（2026-10-03 16:39，决策卡）：纳入 Core3D.dll。** 随即改 spec 与工具：术语表 E3D_Core = core.dll + Core3D.dll，新增 Core3D_IDB、E3D_IDB（= Core_IDB 或 Core3D_IDB），「坐实」出处 ①、需求 1 第 5 / 7 / 9 条、需求 4 第 4 / 18 条、需求 5 第 10 条、需求 6 第 16 条、需求 11 第 10 条改认 E3D_IDB；`ib.ps1` 拒存扩到任一 E3D_IDB 与 Target_IDB；`check_evidence.ps1` 按 IDB 分组核首末 SHA-256；模板加 Core3D_IDB 三行；`selftest.ps1` 加四例（22 项全过）。提交 `e67b1c0`。Q1 的口径随之变为「core.dll 与 Core3D.dll 都认」：7.5 的变换（`WSTRAM`）与尺寸下限（`BOXELE`）也可按 ① 坐实。

### C1（7.1，2026-10-03）

`docs/evidence/2026-10-03-spamap-encoding.md`（`check_evidence.ps1` PASS，两个 IDB 首末 SHA-256 不变，8 个地址 `check_xrefs` PASS，含 2 478 条引用的 `0x6453B98`）。**坐实 8 条**：`spamap.rounding`（`MAPINS` 的 Fortran DNINT）、`word_sign`（有符号；Key0 ∈ [5×10^7, 1.85×10^9]；core `sub_5B00F10` 有符号比键；值域取 datum 为 0 的 ±999 999 999）、`short_axis`（并列取后一根，≥ 20 m 改取 x）、`zero_extent`（无特例）、`bucket`（600 / 2 000 / 6 000 / 20 000 / 200 000，≥ 200 000 为桶 6）、`key0`（min % 5×10^7 再夹，+ 10^8 × N）、`table_shape`（core `sub_5AB91B0` 键 [Key0, refno, refno, 下标 − 1]、值 7 字）、`table_header_rest`（core `sub_5AFF6D0` 新根 [5, hash, 0, 4, 7, 1, 页字数 − 7]；内结点只以出处 ②）。实现放新文件 `src/spatial/rules.rs`（Q4），`core_rules.rs` 加变体、填确认集、RULES 改 Confirmed；另加 `#[doc(hidden)] RuleSet::candidates_alone` 供 Property 6 照旧抽取每条规则，确认的实现也进了抽取池。新 Corpus 测试 `tests/spamap_page_headers_real.rs`：1 903 页（53 内结点）页头全为 `[5, 230902453, level, 4, 7, 1, free]`；32 015 条键字全 < 2^31、坐标在 [−621 000, 1 451 700]。坐实集下 `spamap-check` Key0 28 269 / 28 269、基线 640 3 746 / 3 746，`waiting` 0。**未坐实 2 条**：`entries_k`（k = 下标 − 1；`MPBOX2` 路径一条 k = 0，k = 1 只来自 `MPBXBR`，tube 框未读）、`flags`（三位来自 `MAPINS`；`BOXELE` 按 OBST、`MPBOX` 上溯 SITE 等定是否启用；管件来自 `MPBXBR`），二者都要 noun 的 IMAP 定走树方式，放到 C2 读字典之后再坐实。clippy 63、`cargo doc` 25 与基线相同；`core_rules_registry`、`spamap_codec_real`、`spamap_aabb_real`、`spamap_codec_props`、`spamap_reader_real` 全过。
