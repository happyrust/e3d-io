# Implementation Plan: write-path-phase-3

## Overview

第三期按「core 取证 → 读侧 → 写侧 → 真 E3D 门」推进：先建基线与 Golden 夹具、IB_Wrapper 与取证工具；再落不在 `src/write/` 下的 SPAMAP_Reader、规则注册表、SPAMAP_Codec、Codec_Check 与 Built_In_AABB；core 取证坐实规则并过需求 2 的门槛后，把 SPAMAP_Writer 接入 Transaction、Write_CLI 与 TTY_Gate。发号下界与 Open_Fast_Path、staging 规模、S6b 页 1 桶表各成一条支线。实现语言为 Rust（取证与门脚本为 PowerShell 5.1 与 IDAPython）。凡依赖 E3D_Core 行为的代码只经 `src/core_rules.rs` 取规则，未坐实时按 D7 回退；任何任务都不得写入未坐实规则的取值。

### 依赖

- 第 1 组先于一切 `src/write/` 改动。
- 第 2 组先于第 7 组、11.1 与 13.1。
- 第 3–6 组可与第 2 组并行。
- 第 7 组依赖第 2、3、5、6 组；7.6 的 6.17 部分另依赖 8.7。
- 第 8 组依赖 7.7 的门槛核查；第 9 组依赖第 8 组；第 10 组依赖第 9 组，10.4 跑门所含的 Q14 另依赖第 11 组（11.7 在 10.2、10.3 之后改同一批门文件）。
- 第 11 组与第 3–10 组并行，11.1 依赖第 2 组。
- 第 12 组：12.4 依赖 12.2 与 12.3；12.5 中 SPAMAP 开启的一项依赖第 8 组。
- 第 13 组依赖第 2 组；13.3–13.5 依赖 13.2 的判定，13.3 另依赖 11.4。
- 第 14 组最后做。

## Tasks

- [x] 1. 基线与测试基础设施（先于任何第三期 `src/write/` 改动）
  - [x] 1.1 在当前 HEAD 跑 Build_Checks 并记录基线
    - 跑 `cargo test --no-fail-fast`、`cargo clippy --all-targets`、`cargo doc`
    - 把 HEAD、测试二进制数、passed / failed / ignored、clippy 与 `cargo doc` 的 warning 数、报 skip 的测试与所缺样本记入 Spec_Ledger「基线数字」
    - _Requirements: 15.1, 15.2, 15.6, 15.8_
  - [x] 1.2 加 `proptest` dev-dependency
    - 查当时 crates.io 上 `proptest` 的最新稳定版，在 `Cargo.toml` 的 `[dev-dependencies]` 以 `=x.y.z` 精确钉死，同步 `Cargo.lock`
    - proptest 的失败种子目录 `proptest-regressions/` 纳入版本管理，不写进 `.gitignore`
    - _Requirements: 2.5, 15.1_
  - [x] 1.3 建 Golden 基础设施
    - `tests/common/golden.rs`：`DbCopy`（`tests/common/write_fixture.rs`）的 `Drop` 对最终文件求 SHA-256；键 = 测试 crate + 线程名 + 线程内序号（`#[track_caller]` 记下的 `DbCopy::of` 调用点只进报错信息，见 design）；金样中有该键即比对，不符则测试失败（线程已在 panic 时跳过）；在 `tests/common/mod.rs` 注册
    - 样本快照：`tools/golden/snapshot.ps1` 把写测试用的样本钉到 `D:\e3dio-fixtures\phase3`（附 `MANIFEST.tsv`），`DbCopy::of` 优先取快照；金样每条另存源样本 SHA-256，源哈希不符单独报「样本漂移」
    - `tools/golden/record.ps1`：连跑两遍写测试，剔除两遍不同的键，打印剔除项与原因，供记入 Spec_Ledger「Golden 剔除键」
    - _Requirements: 2.7, 12.8, 15.3_
  - [x] 1.4 在当前 HEAD 录 `tests/golden/off_compat.json`
    - 用 `tools/golden/record.ps1` 覆盖 `tests/write_stage6_*.rs` 的批次（固定 `SessionRequest.clock`），作为 Property 16 的参照，在任何第三期 `src/write/` 改动之前入库
    - 剔除的键记入 Spec_Ledger「Golden 剔除键」
    - _Requirements: 15.3_

- [x] 2. IB_Wrapper 与取证工具（需求 1）
  - [x] 2.1 实现 `tools/ida/ib.ps1` 主体（PS 5.1）
    - 用法 `ib.ps1 [--target-idb <path>] [--timeout-s <n>] <ida-bridge 命令与参数…>`：两个选项由包装器消费，其余原样转发；`--target-idb` 缺省为 Core_IDB
    - 安装查找与缺项报告（1.1、1.2），无可用安装退出 90
    - 子进程 `PYTHONPATH` 放入所选安装的 `src` 并自检能导入；不改 `.pth`、PATH 等用户配置
    - 拒存盘（1.5）：带 `--save`、`supervisor save`、`--out-idb` 指向任一 E3D_IDB 或 Target_IDB（2026-10-03 C0 后由只认 Core_IDB 扩成）、载荷含 `idb.save` / `save_database` 的调用退出 91，且不向桥发任何请求
    - 连桥（1.4）：桥不可达退出 92
    - 按 Target_IDB 解析实例（1.3、1.10）：无实例打开 Target_IDB 或该文件不存在退出 93，多个实例或被占用退出 94
    - 其余情形沿用被转发命令的退出码（1.11）
    - `exec` 超时（1.12）：时限取 `--timeout-s`（缺省 60 s，0 为不设），在「时限 + 10 s」内退出 95，打印实例号及停止、重开该实例的完整命令，不重发调用、不停实例
    - 包装器自身错误的 stderr 以 `[ib]` 开头
    - 实现时核对并在脚本注释中写明：`list` 输出格式与占用字段、CLI 入口模块、`supervisor stop` 是否存盘
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.10, 1.11, 1.12_
  - [x] 2.2 加 `ib.ps1 env`、`ib.ps1 xrefs` 与 IB_Xref_Helper
    - `env` 打印 Target_IDB（缺省 Core_IDB）的 SHA-256（`Get-FileHash`）、IDA 版本（实例内 `ida_kernwin.get_kernel_version()`）与 `git rev-parse HEAD`
    - `tools/ida/xrefs_to.py`：包装器以 `-c "XREF_TARGET=0x…"` 注入目标、以 `-f` 运行；用 `ida_xref.xrefblk_t` 的 `first_to` / `next_to` 枚举代码与数据引用并排除顺序流（`XREF_FAR` 还是 `XREF_NOFLOW` 按所装 SDK 核对）；取到 `BADADDR` 或满 100 000 条即停；输出按源地址排序的 JSON（`from`、`kind`、`type`、`func`），截断时带 `truncated: true`
    - `tools/ida/queries/xrefs_to.sql` 取 `xrefs` 表中同一 `to_ea`、去掉 flow 类的 `from_ea`；`tools/ida/check_xrefs.ps1 <地址>` 比较两个集合
    - _Requirements: 1.5, 1.6, 1.9_
  - [x] 2.3 建证据模板与机检
    - `tools/ida/EVIDENCE_TEMPLATE.md` 分「环境」「结论」「复现命令」「反例」四节；「结论」逐条写规则名、坐实与否、例程起始地址、`pseudocode.n` 行号或指令地址、Corpus 核对（库、命令、一致数 / 总数），未坐实的另写假设、输入类别与反例
    - `tools/ida/check_evidence.ps1 <doc>`：「复现命令」只以 `tools/ida/ib.ps1`、仓内相对路径脚本与 `cargo` 为入口；不含临时目录、`idalib-\d+` / `idaui-\d+` 与 IDA_Bridge 安装目录；所引 `tools/ida/` 文件受版本管理（`git ls-files --error-unmatch`）；地址十六进制、哈希 64 位、HEAD 40 位；比对「环境」节首尾两次 SHA-256（C0 后按 IDB 分组：含 `Core3D_IDB` 的行一组、其余一组，每组首末相同）
    - _Requirements: 1.7, 1.8, 1.9_
  - [x]* 2.4 写 `tools/ida/selftest.ps1` 离线自检
    - 用假安装目录与不可达的桥地址，覆盖 1.2 的缺项报告、1.4 的桥不可达分支（92）、1.5 的拒存盘（91，且不发请求）
    - **Property 25: 一轮取证不改 E3D_IDB**（离线部分）
    - **Validates: Requirements 1.5**
    - _Requirements: 1.2, 1.4, 1.5_
  - [x]* 2.5 IB_Wrapper 实机冒烟
    - 经 IB_Wrapper 起桥与 idalib（Core_IDB），跑一次 `exec`，并覆盖多实例、被占用、超时三条路径
    - `check_xrefs.ps1` 跑至少 5 个地址，其中一个被大量引用；`env` 首尾两次哈希相同
    - 结果记入 Spec_Ledger「IB_Wrapper 实机冒烟」
    - **Property 24: IB_Xref_Helper 与 SQL 一致**
    - **Validates: Requirements 1.6**
    - **Property 25: 一轮取证不改 E3D_IDB**
    - **Validates: Requirements 1.5**
    - _Requirements: 1.1, 1.3, 1.4, 1.10, 1.11, 1.12_
  - [x] 2.6 Checkpoint - 第 2 组：Ensure all tests pass
    - Ensure all tests pass, ask the user if questions arise.

- [x] 3. SPAMAP_Reader 与 Corpus_Census（需求 3）
  - [x] 3.1 实现 `src/index/spamap.rs`
    - 类型 `SpamapEntry`（4 键字 + 7 值字）、`SpamapRead`、`SpamapTable`（`entries()`、`for_refno()`，首次查询时建 refno → 条目下标索引）
    - 页解码走 `src/index/keyed.rs` 的 `decode::<V>`（由 `src/index/reference.rs::decode` 抽出，反向表 `V = 0` 行为与错误不变），不经 `IndexPageView`（见 design）
    - `read_spamap_tree`：自根显式栈遍历，逐项做需求 3 第 2 条的全部校验；出错返回带页号与出错项的 `SpamapReadError`，不返回截短的列表
    - `read_spamap`：在会话表目录里按名 hash 230902453 找表，找不到返回 `NoTable`；会话由 `DbView::open_at` 选定，取不到即报错，不改读其他会话
    - 不放在 `src/write/`、不调用它；在 `src/index/` 的模块声明中注册
    - _Requirements: 3.1, 3.2, 3.4, 3.9_
  - [x] 3.2 写测试专用建树器 `tests/common/spamap_pages.rs`
    - 由条目序列与页长建内存页树（页类型 5、名 hash 230902453、键宽 4、叶值宽 7），不经 `src/write/`；在 `tests/common/mod.rs` 注册
    - _Requirements: 3.2, 3.4, 3.5_
  - [x]* 3.3 写 SPAMAP_Reader proptest：`tests/spamap_reader_props.rs`
    - 随机条目集合按随机页长建树、读回对照；在建树结果上注入需求 3 第 2 条所列的单项篡改
    - **Property 1: 按 refno 查询等于全表过滤**
    - **Validates: Requirements 3.4, 3.5**
    - **Property 3: 畸形页必报错**
    - **Validates: Requirements 3.2**
  - [x]* 3.4 写 `tests/spamap_reader_real.rs`
    - SPAMAP_Baseline（`ams8000_0001` 原件会话 640，按会话号精确选取）读数：244 页 / 3 746 条 / 3 628 个 refno / k = 0 的 3 625 条 / k = 1 的 121 条；不符时改用仓内受控快照，并记入 Spec_Ledger「SPAMAP_Baseline 读数」
    - Corpus 各库最新会话：查每个出现过的 refno 与至少 1 个未出现的 refno；样本缺失经 `common::missing` 报 skip
    - **Property 1: 按 refno 查询等于全表过滤**（Corpus 部分）
    - **Validates: Requirements 3.4, 3.5**
    - _Requirements: 3.3, 3.5_
  - [x] 3.5 `e3d-descriptor` 加子命令 `spamap` 与 `spamap-census`（`src/bin/e3d-descriptor.rs`）
    - `spamap --db <f> [--session <n>] [--refno <w0/w1>]`：JSON 输出会话号、根页号、页数与条目
    - `spamap-census --input <文件或目录>… [--session <文件>=<n>]…`：输出需求 3 第 7、10、11、12 条各项，另报「按元素的 k 组合」计数；以 `ReadOnlyEngine` / `DbView` 只读打开
    - 输出确定：输入按路径排序、统计一律用 `BTreeMap`、不写时间戳；首部不可解析的文件列为「非库文件」；某库出错时列出路径与错误后继续，全部结束后以非零退出码退出
    - _Requirements: 3.7, 3.8, 3.10, 3.11, 3.12_
  - [x]* 3.6 写普查集成测试 `tests/spamap_census_real.rs`
    - 真跑 bin，固定几种输入组合（库文件、目录、`--session`、非库文件、出错库），比对运行前后的 SHA-256 与两次输出
    - **Property 4: 普查只读且确定**
    - **Validates: Requirements 3.8**
  - [x] 3.7 在 Corpus 上跑普查并写 Phase_3_Evidence_Doc「SPAMAP 普查」
    - 按 `tools/ida/EVIDENCE_TEMPLATE.md` 写 `docs/evidence/YYYY-MM-DD-spamap-census.md`，「复现命令」用仓内 `cargo run --bin e3d-descriptor -- spamap-census …`；经 `tools/ida/check_evidence.ps1` 机检
    - 更正 2026-09-22 取证 §0 第 7 条两组直方图合计不一（N 3 746、flags 3 747，分属会话 640 与 648 的表）的记载
    - _Requirements: 3.7, 3.10, 3.11, 3.12_
  - [x] 3.8 Checkpoint - 第 3 组：Ensure all tests pass
    - Ensure all tests pass, ask the user if questions arise.

- [x] 4. 规则注册表与空间值类型
  - [x] 4.1 实现规则注册表 `src/core_rules.rs`
    - `RuleId`（`#[non_exhaustive]`）、`Rule`、`RuleStatus`、`RULES`：按 design 规则表逐条登记，全部为 `Unconfirmed`，不预设任何取值
    - `RuleSet::confirmed()`（Transaction 唯一入口）与 `#[doc(hidden)] RuleSet::candidates(&CandidatePick)`；每类规则一个类型化实现槽枚举，初始只有 `Pending`
    - `const` 断言：实现槽不是 `Pending` ⇒ 状态为 Confirmed
    - `recompute_row`（表中没有的属性为 `Unconfirmed`）、`member_order_row`、`descendants_included`（D7、D8 的唯一落点）
    - 在 `src/lib.rs` 注册模块
    - _Requirements: 2.6, 5.2, 5.3, 5.8_
  - [x] 4.2 实现空间值类型
    - `src/spatial/mod.rs`：`WorldAabb`（构造即校验有限值与每轴 min ≤ max）、`SpatialPolicy`（`#[default] Require`，D3 的唯一落点）、`Axis`、`SpatialState` trait
    - `src/spatial/state.rs`：读侧 `ReadState`，按会话读元素记录与 owner 链
    - 在 `src/lib.rs` 注册模块；不放在 `src/write/`
    - _Requirements: 6.5, 6.6, 6.7, 6.10_
  - [x]* 4.3 写 `tests/core_rules_registry.rs`
    - 断言规则名唯一；每条 Confirmed 规则的出处文件存在，且其「结论」节含该规则名；`src/write/` 下的源码不出现 `candidates`
    - _Requirements: 2.6_

- [x] 5. SPAMAP_Codec 与 Codec_Check（需求 4）
  - [x] 5.1 实现 `src/spatial/codec.rs`
    - `SpamapCodec::encode` 按固定次序编码：取整 → 可表示范围 → 最短轴 → 零 extent → 桶 → Key0 → 条目数与 k → Spatial_Flags（经 `SpatialState` 读元素记录）
    - `Encoded`（`Entries { entries, relied_on }`、`NeedsUnconfirmed`、`MissingInput`）、`CodecError::OutOfRange { refno, axis, value_mm, range }`、`key0_of`
    - 用到处于 `Pending` 的规则即收集其规则名、结果为 `NeedsUnconfirmed`，不产出部分条目；Codec 不看 Spatial_Policy
    - _Requirements: 4.2, 4.3, 4.5, 4.6, 4.7, 4.9, 4.12, 4.19_
  - [x] 5.2 实现候选规则 `src/spatial/candidates.rs`
    - 为 Codec 各规则与 `spamap.coverage` 写候选实现，每个候选在注释中写明所作假设
    - 只经 `RuleSet::candidates(&CandidatePick)` 接入（改 `src/core_rules.rs` 的 `candidates()`），`confirmed()` 不可达
    - _Requirements: 2.6_
  - [x] 5.3 实现 Codec_Check：`e3d-descriptor spamap-check`
    - 计数逻辑写成 `src/spatial/` 下的纯函数（如 `src/spatial/check.rs`），子命令与测试共用
    - Key0（只用条目自身 6 个值字）、条目数与 k、Spatial_Flags 三项分开报一致数 / 总数，各列前 100 个反例，每个反例附所依的未坐实规则名
    - 参数 `--input …`、`--rules confirmed|candidates`、`--exclude <文件>`（受版本管理的剔除清单，每条附会话链证据出处），另支持 `--refno`（供 Q13g）；E3D_Authored_Element 按元素最新记录所在会话 USER ≠ `e3dio` 判定
    - _Requirements: 4.10, 4.11, 4.15, 4.16, 4.17_
  - [x]* 5.4 写 Codec proptest：`tests/spamap_codec_props.rs`
    - 随机 `CandidatePick` 决定哪些规则有实现；坐标含各轴边界附近与超出可表示范围、最短轴并列、最短轴 extent 为 0
    - **Property 6: Codec 结果三分**
    - **Validates: Requirements 4.12, 4.19**
  - [x] 5.5 写 Codec Corpus 核对：`tests/spamap_codec_real.rs`
    - Corpus 各库（含 SPAMAP_Baseline）三项计数；proptest 向单条条目注入一项失配，断言只改变对应那一项的计数
    - 规则坐实前用 candidates 跑，结果只作取证数据、不作通过判据；坐实后改由 `confirmed()` 断言一致数等于总数（只按剔除清单剔除）
    - **Property 5: Codec 三项核对分开计数**
    - **Validates: Requirements 4.2, 4.3, 4.7, 4.9, 4.10, 4.11, 4.15, 4.16**

- [x] 6. Built_In_AABB（需求 6 读侧部分）
  - [x] 6.1 实现 `src/spatial/aabb.rs`
    - 复合器 `FrameComposer` 与默认 `ChainComposer`：沿 owner 链上溯到 WORL，按 `aabb.compose` / `aabb.ori_form` 复合 POS / ORI，任一为 `Pending` 返回 `NeedsUnconfirmed`
    - `BuiltInAabb::compute` → `AabbOutcome`；`BUILT_IN_NOUNS` 为 BOX、CYLI（D1 的唯一落点）；BOX 用 XLEN / YLEN / ZLEN，CYLI 用 DIAM / HEIG，方法由 `aabb.box` / `aabb.cyli` / `aabb.degenerate` 定；输出毫米 `f64`，取整只在 Codec 里做
    - 所需属性解码不了时返回 `UndecodableAttr { refno, name, hash }`
    - `aabb.*` 的候选实现加入 `src/spatial/candidates.rs`
    - _Requirements: 6.2, 6.8, 6.14_
  - [x] 6.2 `e3d-descriptor` 加 `world-frame`，并把 Built_In_AABB 一项接入 `spamap-check`
    - `world-frame --db <f> --refno <r>` 打印复合器算出的世界原点与 World_AABB，供 Q13h 与 Probe_Element 预判
    - `spamap-check` 增报 Built_In_AABB 一项（需求 6 第 9 条）的一致数 / 总数与前 100 个反例，供 7.5 的 Corpus 核对
    - _Requirements: 6.9, 10.15_
  - [x] 6.3 写 Built_In_AABB Corpus 核对：`tests/spamap_aabb_real.rs`
    - 经 `ReadState` 算 World_AABB、经 SPAMAP_Codec 编码，与盘上条目逐字比较（条数、4 键字、7 值字）；样本含 BOX `24384/24777`；坐实前用 candidates 跑，结果只作取证数据
    - **Property 8: Built_In_AABB 与盘上逐字相同**
    - **Validates: Requirements 6.9, 6.16**

- [x] 7. core 取证：编码、覆盖、重算、AABB（需求 4、5、6）
  - 7.1–7.5 各产出一份 Phase_3_Evidence_Doc（`docs/evidence/YYYY-MM-DD-*.md`，按模板），经 `tools/ida/check_evidence.ps1` 机检；每轮首尾跑 `ib.ps1 env`，对本轮用到的地址（至少 5 个，含一个被大量引用的）跑 `check_xrefs.ps1`，结果写入证据
  - 坐实的规则在同一提交里于 `src/core_rules.rs` 改为 Confirmed 并补上实现变体（相应改 `src/spatial/` 中用到它的代码）；未坐实的在证据中写明假设、须用到它的输入类别与 Corpus 反例（2.4），注册表保持 `Unconfirmed`
  - [x] 7.1 取证编码规则
    - 规则：`spamap.rounding`、`spamap.word_sign`、`spamap.short_axis`、`spamap.zero_extent`、`spamap.bucket`、`spamap.key0`、`spamap.entries_k`、`spamap.flags`；另取证表形 `spamap.table_shape` 与页头其余字 `spamap.table_header_rest`（供 8.1、8.6 写内结点页与建表）
    - 经 IB_Wrapper 读 E3D_Core 例程（编码在 Core3D_IDB 的 `maplib`，表与记录项在 Core_IDB）；用 `spamap-check --rules candidates` 在 Corpus 上核对，一致数 / 总数写入「结论」；最短轴并列的取轴只认 E3D_IDB 出处
    - _Requirements: 2.4, 4.1, 4.4, 4.5, 4.6, 4.8, 4.13, 4.14, 4.17, 4.18, 7.8_
  - [x] 7.2 取证覆盖判定 `spamap.coverage` 与 Geometric_Noun 清单，并写 Property 7 Corpus 测试
    - 证据写明被检查的每个因素（noun 及 noun 之外的因素）与逐 noun 的三态结论
    - 在 `tests/spamap_codec_real.rs` 加覆盖对账；E3D_Authored_Element 按会话 USER ≠ `e3dio` 判定；坐实前用 candidates 跑、只作取证数据
    - **Property 7: 覆盖对账**
    - **Validates: Requirements 5.7**
    - _Requirements: 2.4, 5.1, 5.10_
  - [x] 7.3 取证重算时机
    - 内容：各 Geometric_Noun 的重算表（「自身」各属性与成员次序）；换 owner、改 POS、改 ORI 时各层后代是否重算；删除后的残留（`spamap.delete_residue`）；无条目元素能否删除并提交
    - 须 E3D_Session 实测的部分写探针脚本 `tools/e3d/spamap_probe.ps1`：副本每轮从原件重置，会话 USER 为 `e3dio`
    - 已做（里程碑 D 的 D1、D2）：`docs/evidence/2026-10-03-spamap-recompute.md`，五条规则坐实；探针 `tools/e3d/spamap_probes/d2/` 35 个
    - 结论写入 `spamap.recompute.<NOUN>` 与 `spamap.descendants.{move,pos,ori}`（D7、D8）
    - _Requirements: 2.4, 5.2, 5.3, 5.4, 5.8, 5.9_
  - [x] 7.4 取证 SPAMAP_Record_Item
    - 逐 Geometric_Noun 坐实 E3D_Core 是否写该项；若写，写明类型码、字数、数据字布局、位置及其与条目键字（Key0、k）的对应关系；逐 noun 给出 Corpus 中带与不带该项的 E3D_Authored_Element 个数
    - 结论写入 `spamap.record_item.<NOUN>`
    - _Requirements: 2.4, 5.5, 5.11_
  - [x] 7.5 取证 AABB 规则
    - 规则：`aabb.ori_form`、`aabb.compose`、`aabb.box`、`aabb.cyli`、`aabb.degenerate`、`aabb.arithmetic`；Corpus 核对用 `spamap-check` 的 Built_In_AABB 一项与 `world-frame`
    - 坐实的变体补进 `src/spatial/aabb.rs`；不能逐字复现时，按需求 6 第 16 条把改用的判据记入 Spec_Ledger「规则坐实状态总表」`aabb.arithmetic` 行
    - 已做（里程碑 C 的 C4、D 的 D3）：`docs/evidence/2026-10-03-spamap-aabb.md`、`docs/evidence/2026-10-03-spamap-aabb-members.md`；C4 留下的 18 个反例由新增的 `aabb.members`（IMAP 5 元素并入 IMAP 3 成员的框）解释，另增 `aabb.cone`、`aabb.pyramid`，九条全部坐实，Built_In_AABB 1 590 / 1 590 逐字，未剔除、未放宽
    - _Requirements: 2.4, 6.1, 6.16_
  - [x]* 7.6 写示例测试 4.20 与 6.17
    - 4.20：在 `tests/spamap_codec_real.rs` 中以 BOX `24384/26484` 在 E3D 为其写出条目那一会话的状态为输入（经 `ReadState`），断言 SPAMAP_Codec 恰得需求 4 第 20 条的那一条条目
    - 4.20 已做（里程碑 D 的 D4）：探针 `tools/e3d/spamap_probes/selfcheck_e3dio_box_xlen.pml` 让 E3D 改该 BOX 的 `XLEN`，得到的会话 648 钉为 `D:\e3dio-fixtures\phase3\spamap-4-20\ams8000_0001`（输入记于 `docs/evidence/2026-10-03-spamap-aabb-members.md`）；`requirement_4_20s_box_encodes_to_exactly_its_entry` 在坐实集下恰得该条目，与盘上所存相同。6.17 已做（E2–E9，`fd9ed67`）：`write_stage8_spamap_examples::requirement_6_17_box_registers_its_exact_world_bounds` 只用读侧验证六个坐标字
    - 6.17：在 `tests/write_stage8_spamap_examples.rs` 中经 Transaction 插入需求 6 第 17 条的 BOX（不给 World_AABB）并提交，只用读侧读回，断言条目值前 6 字即该条的 World_AABB；此部分依赖 8.7
    - 所需规则未坐实时，测试断言 D7 回退（视同无可用 World_AABB），并把该示例作为待办记入 Spec_Ledger「规则坐实状态总表」备注
    - _Requirements: 4.20, 6.17_
  - [x] 7.7 取证门槛核查
    - 已做（里程碑 D 的 D5）：预核结果见 Spec_Ledger「取证门槛核查」SPAMAP 维护一行；8.1 落地时按它的父提交复核（期间改了 SPAMAP 读侧或证据就重跑表中的测试）
    - 在 SPAMAP 维护的首个写侧提交（8.1）之前，确认其父提交中已有 7.1–7.5 的 Evidence_Doc（各含例程地址与 Corpus 核对）、第 3 组的读侧解码，以及以 Corpus 为输入、0 失败且未跳过的测试（3.4，及坐实后由 `confirmed()` 断言的 5.5、6.3、7.2）
    - 结果与第 7 组各规则的坐实状态填入 Spec_Ledger「取证门槛核查」与「规则坐实状态总表」
    - _Requirements: 2.1, 2.2_
  - [x] 7.8 Checkpoint - 第 7 组：Ensure all tests pass
    - Ensure all tests pass, ask the user if questions arise.
    - 已做（里程碑 D 的 D6，`d8fecb6`，其后只改文档）：114 个测试二进制 + Doc-tests，697 / 0 / 30，skip 仍是基线那 7 个；clippy 63、`cargo doc` 25 同基线

- [x] 8. SPAMAP_Writer 接入 Transaction（需求 5 第 6 条、需求 6–8）
  - [x] 8.1 把 `src/write/keyed_cow.rs` 泛化为 `rewrite_table::<V>`
    - `V = 0` 时写出字节不变，由既有单测与 1.4 的金样守住；`V = 7` 供 SPAMAP，页头其余字按 `spamap.table_header_rest`
    - 另加 `#[doc(hidden)]`、不带规则的建树入口，供 Property 2 调用
    - _Requirements: 7.7, 7.10, 15.3_
  - [x]* 8.2 写页树 proptest：`tests/write_stage8_spamap_pages.rs`
    - 随机条目序列与合法页长（含逼出多层树的小页长）经建树入口编码后由 `read_spamap_tree` 读回；Corpus 各库的表读出、重编、再读回；编辑集合在内存页上应用一次与两次，对照按 534 / 529 直译的 `BTreeMap` 模型
    - **Property 2: 条目序列往返**
    - **Validates: Requirements 3.1, 3.6**
    - **Property 10: 编辑集合幂等**
    - **Validates: Requirements 7.10**
  - [x] 8.3 实现 `src/write/spamap/end_state.rs`
    - `EndState` = 上一会话 ⊕ 定稿记录，即本批结束状态（合并时为 Merged_State）；提供 `previous`、`in_new_index`、`members`、`reachable_from_worl`、`theirs_changed`，并实现 `SpatialState`
    - 建 `src/write/spamap/mod.rs`，在 `src/write/mod.rs` 注册
    - _Requirements: 7.13, 8.1, 8.6, 8.8_
  - [x] 8.4 实现 `SpatialStage::plan`：`src/write/spamap/plan.rs`
    - Recompute_Set 按定义 ①–⑦ 求，只按本批结束状态；后代是否纳入只经 `core_rules::descendants_included`；⑤ 的祖先重算属性按 `recompute_row`（扩到祖先 noun），成员次序按 `member_order_row`，⑦ 取 IMAP 3 元素的第一个 IMAP 不为 3 的祖先
    - 逐元素判存在、从 WORL 可达、属于 Geometric_Noun；可用 World_AABB 依次取调用方值 → Built_In_AABB → 无；经 SPAMAP_Codec 得 Target_Entries
    - 仅 Recompute_Set 非空时读上一会话表（全遍历并校验）得旧条目；编辑集 = 删 旧(RS) + 插 Target(RS)，并判定净编辑是否为空
    - 汇总 `MissingAabb`（refno、`given_as`、`类别:规则名`）：「要求」下拒写整批，「跳过」下列入 `skipped`；`UndecodableAttr`、`OutOfRange`、`MalformedTable` 在「要求」「跳过」下都拒写；给在 Recompute_Set 之外元素上的值列入 `unused_aabb`
    - _Requirements: 6.2, 6.3, 6.4, 6.5, 6.6, 6.7, 6.8, 6.13, 6.14, 6.15, 7.1, 7.2, 7.3, 7.4, 7.5, 7.11, 7.12, 7.13, 7.14_
  - [x] 8.5 写 plan 模块内 proptest（`src/write/spamap/plan.rs` 的 `#[cfg(test)]`）
    - 合成的 `SpatialState` 加替身编码器，对照按 Recompute_Set 定义 ①–⑦ 直译的参考实现；生成器含成员次序、祖先非几何重算属性、IMAP 3 成员三类改动
    - **Property 9: Recompute_Set 模型**（① 模块内层）
    - **Validates: Requirements 6.2, 6.3, 6.4, 6.13, 7.1, 7.2, 7.3, 7.4, 7.5, 7.9, 7.11, 8.2, 8.8**
  - [x] 8.6 实现 staging 挂点
    - 原语增加 crate 私有入口，带 `SpatialStage`；公开原语 `stage_element_session[_with]`、`stage_merge_session_with` 签名不变，内部传 `None`（不维护 SPAMAP，写出 0.4.0 的字节）
    - 挂点在记录、成员、索引、NAME、claim、TYPEX、反向表编辑定稿之后（合并时在三方合并之后）、写任何页之前；所有拒写都发生在追加第一页之前；若发现挂点之前已写页，改接现有 rollback 截断路径，并以测试断言文件逐字节不变
    - SPAMAP 页排在其他表页之后、会话页之前；经 `table_overrides` / `table_additions` 写表目录；上一会话无表且 Target_Entries 非空时建表；`WriteError::TableDirectoryFull` 映射为 `SpatialError::DirectoryFull`
    - _Requirements: 7.6, 7.7, 7.8, 7.12_
  - [x] 8.7 扩展 Transaction 公开 API
    - 动手前在 Downstream_Crates 中搜 `TransactionError` 与 `TransactionCommit` 的穷尽 `match` 与结构体字面量；搜到即停下，把清单交用户确认后再改（需用户）
    - 新增 `Transaction::with_spatial`、`set_aabb`（在调用处校验：非有限值或 min > max 报 `InvalidAabb`，已坐实为非 Geometric_Noun 报 `NotGeometric`，出错时草稿与此前接受的值不变；多次给值取最后一次）、`StagedSession::spatial`、`TransactionCommit.spatial`、`TransactionError::Spatial`；`SpatialError` 为 `#[non_exhaustive]`，并给 `TransactionError` 加 `#[non_exhaustive]`
    - `SpatialStage` 只存 Spatial_Policy 与按调用方 refno 记下的 World_AABB，派生量每次 stage 内重算
    - 与 8.10 同一提交落地，使每个提交上的既有测试不因 D3 默认而失败
    - _Requirements: 6.3, 6.4, 6.10, 6.11, 6.12, 15.4, 15.9_
  - [x] 8.8 实现合并与重铸下的 SPAMAP
    - 合并时 EndState 的底、旧 SPAMAP 表与表目录都取 Merge_Latest；合并模块把 base 与 Merge_Latest 的比较结果经 `EndState::theirs_changed` 交给 plan，判定调用方给值是否失效（祖先链按需惰性比较）
    - 重铸映射（给定号 → 写入号）传给 plan：条目按新号编键，原号下占用者的条目保留；因 Merge_Latest 删了祖先而不可达的元素不写条目，列入 `SpatialReport.unreachable`（附被删祖先）
    - `commit_observed` 遇 `Moved` 时整个 stage 重跑一次，第二次 `Moved` 照旧上抛；SPAMAP 条目差异不算撞车
    - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5, 8.6, 8.7, 8.8, 8.9_
  - [x] 8.9 写入 SPAMAP_Record_Item（仅当 7.4 坐实 E3D 写该项）
    - plan 另产出记录项补丁，在记录分页之前并入写集（可能带入本批未写的后代记录）；未坐实或坐实为「不写」时本任务不做
    - _Requirements: 5.6_
  - [x] 8.10 适配既有测试
    - 因 D3 默认「要求」而失败的既有测试（如 `tests/write_stage6_*.rs` 中插入 BOX / EQUI / GROU 的批次）按需求 15 第 7 条只许显式指定 Spatial_Policy（如 `.with_spatial(Off)`），不改其他期望
    - 所改测试逐条记入 Spec_Ledger「测试适配清单」
    - _Requirements: 15.7_
  - [x] 8.11 写库副本模型测试：`tests/write_stage8_spamap_model.rs`
    - 库副本 proptest（16 例，`DbCopy::of`），只用读侧读回新会话表（需求 2 第 5 条）；dry run 先断言文件 SHA-256 与长度不变，再提交并比对报告；Property 13 在 `commit_observed` 的 observe 回调里读表
    - **Property 9: Recompute_Set 模型**（② 库副本层）
    - **Validates: Requirements 6.2, 6.3, 6.4, 6.13, 7.1, 7.2, 7.3, 7.4, 7.5, 7.9, 7.11, 8.2, 8.8**
    - **Property 11: 多次写入等价于直达结束状态**
    - **Validates: Requirements 7.13**
    - **Property 12: 净编辑为空不动表**
    - **Validates: Requirements 6.7, 7.6, 8.1**
    - **Property 13: 写时复制与唯一提交点**
    - **Validates: Requirements 7.7**
    - **Property 15: dry run 与提交一致**
    - **Validates: Requirements 8.5, 9.5**
  - [x] 8.12 写合并测试：`tests/write_stage8_spamap_merge.rs`
    - 库副本 proptest（16 例），两个写者，其一经 `commit_observed` 在 stage 与 page 0 之间提交以触发 `Moved`；只用读侧读回
    - **Property 14: 合并下的保留、重铸与不可达**
    - **Validates: Requirements 8.3, 8.4, 8.7, 8.9**
  - [x] 8.13 写「关闭」兼容测试：`tests/write_stage8_off_compat.rs`
    - `tests/write_stage6_*.rs` 的批次以 `with_spatial(Off)`、固定 `SessionRequest.clock` 写出，逐字节对照 1.4 录下的 `tests/golden/off_compat.json`；发号下界落地后只比不含新发 refno 的副本，或以预铸 refno 走原语路径
    - **Property 16: 「关闭」下与 0.4.0 逐字节相同**
    - **Validates: Requirements 15.3**
  - [x] 8.14 写拒写测试：`tests/write_stage8_spamap_refuse.rs`
    - 库副本 proptest，每批注入至多一类 A / B 原因，断言各策略下的拒写与 `skipped`，拒写后文件 SHA-256 与长度不变；另加表目录已满的示例测试
    - **Property 17: 缺值与空间拒写**
    - **Validates: Requirements 2.6, 6.5, 6.6, 6.8, 6.14, 6.15, 7.12, 8.6**
    - _Requirements: 7.12_
  - [x] 8.15 写 `set_aabb` proptest：`tests/write_stage8_spamap_api.rs`
    - **Property 18: `set_aabb` 校验与取最后一次**
    - **Validates: Requirements 6.10, 6.11, 6.12**
  - [x] 8.16 写 SPAMAP_Record_Item 测试：`tests/write_stage8_spamap_record_item.rs`（仅当 8.9 存在）
    - Corpus 遍历加库副本 proptest
    - **Property 26: SPAMAP_Record_Item 逐字相同（仅当需求 5 第 5 条坐实 E3D_Core 写该项）**
    - **Validates: Requirements 5.6**
  - [x] 8.17 写建表示例测试（需求 7 第 8 条）
    - 在 `tests/write_stage8_spamap_examples.rs` 中以无 SPAMAP 表的库副本为底提交几何元素，只用读侧断言新会话表目录新增名 hash 230902453 的表项且条目可读回；`spamap.table_header_rest` 未坐实时断言按 D7 视同无可用 World_AABB
    - _Requirements: 7.8_
  - [x] 8.18 Checkpoint - 第 8 组：Ensure all tests pass
    - Ensure all tests pass, ask the user if questions arise.
    - 已验收（2026-10-04，E2–E9，`fd9ed67`）：`cargo test --no-fail-fast -- --nocapture` 为 722 / 0 / 30（122 个测试二进制 + Doc-tests）；既有 skip 7 个（8 条提示，其中成员覆盖测试同时缺新、旧样本），新增 E 测试无 skip。`cargo clippy --all-targets` 63、`cargo doc` 25，与 E1 基线相同。17 条 Off 金样通过，未更新金样。CLI 显式 Off，完整空间 CLI 与真 E3D 门留到 F。

- [x] 9. Write_CLI（需求 9）
  - [x] 9.1 扩展 `src/bin/e3d-write.rs`
    - `aabb`（世界系毫米 `[minx, miny, minz, maxx, maxy, maxz]`）只能用于 `insert` / `set` / `move`：建批阶段校验恰 6 个有限 JSON 数、每轴 min ≤ max；允许只带 `aabb` 的 `set`（只调 `set_aabb`）；带在其他指令或非 Geometric_Noun 上即拒；拒绝时写 `op #n` 与原因、退出码 1、不写任何字节；同一元素多条带 `aabb` 时取序号最大的一条
    - `--spatial require|skip|off`：缺省 `require`；非法值走 `Failure::Usage`（退出码 2），列出合法取值，在打开库之前退出
    - 记录 `touched` 与来源指令；`--report` 增加 `spatial` 节（refno 用写入号，重铸时保留 `given`）；dry run 从 `StagedSession::spatial()` 取同一份报告后 rollback；stage 阶段空间拒写退出码 1，逐个列出 refno、原因与来源指令
    - `--no-dictionary` 保持 0.4.0 行为
    - 已完成基础 CLI 空间接入：`--spatial require|skip|off`、insert/set/move 的 `aabb` 六值校验、Transaction `set_aabb`、dry-run/commit 空间报告；既有 CLI 测试显式 `--spatial off`，新增 2 项 CLI 验证。9.2/9.3 已在本次 CLI 收口提交验收；补齐解析 min≤max、空间拒写来源、重铸后全部操作引用。
    - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5, 9.6, 9.7, 9.8, 9.9, 9.10, 9.11, 9.12, 9.13_
  - [x]* 9.2 写 CLI 解析 proptest（`src/bin/e3d-write.rs` 内 `#[cfg(test)]`）
    - 已验收：6 项解析测试，其中 3 组 proptest（每组 256 例）；默认/合法/缺值/非法策略、帮助、AABB 数值/长度/轴序、允许操作、非有限 JSON。抽出 parse_args_from，生产参数仍由同一解析函数处理。
    - **Property 19: CLI `aabb` 与 `--spatial` 校验**
    - **Validates: Requirements 9.2, 9.7, 9.8, 9.12**
  - [x]* 9.3 写真跑 bin 测试：`tests/write_stage8_spamap_cli.rs`
    - 已验收：11 项（其中一组 16 例 proptest）；必需受控副本缺失即失败，不 skip。只用读侧对照完整条目差集；dry-run/commit 报告相同；拒写核 SHA-256/长度/会话号；覆盖三策略、set/move、多次给框、unused、祖先删除、重铸保留占用者与 given、祖先触发缺框来源。既有 CLI 7 项不改期望。
    - 核对退出码与 `--db` 文件哈希；`--report` 的 `spatial` 节与读侧读回的差集比对；示例 9.1、9.3、9.6、9.11、9.13
    - **Property 15: dry run 与提交一致**
    - **Validates: Requirements 8.5, 9.5**
    - **Property 19: CLI `aabb` 与 `--spatial` 校验**
    - **Validates: Requirements 9.2, 9.7, 9.8, 9.12**
    - **Property 20: CLI 报告等于读回的差集**
    - **Validates: Requirements 9.4, 9.9, 9.10**
    - _Requirements: 9.1, 9.3, 9.6, 9.11, 9.13_

- [ ] 10. 门（需求 10）
  - [x] 10.1 写 Gate_Ops（`tools/e3d/write_gate_ops/`）
    - 新增 `q13_control.json`（`--spatial off`，写 Control_Box）、`q13_boxes.json`（`require`，写 ZONE 及其下 BOX、Q13_Box 与作 Probe_Element 的 BOX）、`q13_move.json`、`q13_xlen.json`、`q13_delete.json`、`q13_cyli_equi.json`（CYLI、带平移与旋转的 EQUI 及其下 BOX）
    - 既有 Gate_Ops 中插入几何 noun 的批次按需求 15 第 7 条显式指定 `--spatial`，逐条记入 Spec_Ledger「测试适配清单」
    - _Requirements: 10.16, 15.7_
  - [x] 10.2 扩展 E3D 宏模板
    - `tools/e3d/write_gate.mac.template` 加 Q13 段：只用 PML1（`VAR !r EVALUATE (REF) FOR ALL BOX WITHIN VOLUME =w0/w1 <余量>`、`Q VAR !r`，Q13h 另加 `Q POS WRT /*`），每问以 `$P [Q13x-BEGIN]` / `$P [Q13x-END]` 夹住；Q13 排在 Q13a 之前，先以包含 Control_Box 的 Probe_Element、再以 Control_Box 本身为体积各搜一次；不用 PML2 对象与 `COLLECT`，不对图元设 `DESC`
    - `tools/e3d/write_gate_e3d_saves.mac.template` 加 Q13g 所需的「E3D 改 XLEN 并 SAVEWORK」
    - _Requirements: 10.1, 10.8, 10.13_
  - [x] 10.3 扩展 `tools/e3d/write_gate.ps1`
    - 在 Q12e 之后加 Q13 组：e3d-io 写入全部经 `e3d-write apply --report` 与 Gate_Ops（会话 USER 为 `e3dio`），所问 refno 取自 `--report`，不写死
    - 按标记切日志段，判「列出 / 省略 / 无 `(2,536)`」，每问一行 `PASS <问号>` 或 `FAIL <问号>`；Q13 FAIL 时 Q13a–Q13h 一律 FAIL
    - Q13g 借助 `e3d-descriptor spamap` 与 `spamap-check --refno`；Q13h 借助 `world-frame`（各轴之差 ≤ 0.1 mm）
    - 打印 Probe 预判的 `PROBE` 行与耗时的 `TIME` 行
    - _Requirements: 10.2, 10.3, 10.4, 10.5, 10.6, 10.7, 10.8, 10.9, 10.12, 10.13, 10.14, 10.15, 10.16, 10.17_
  - [x] 10.4 跑门并记录
    - 2026-10-05 F7：完整 TTY 39/0（含 Q14），alternate 5 轮、原样 FORMAT 0 全 PASS，源 AMS/TES 哈希/长度不变；证据 `docs/evidence/2026-10-05-allocation-q14-gate.md`。10.5/10.6 仍待人工图形门。
    - 2026-10-04 部分完成：Q0 + 既有 28 问 + Q13/Q13a–h、alternate 5 轮、原样 FORMAT 0 全 PASS。总 199.5563017 s / Q13 161.8146313 s。Q14 等第 11 组，不标整体完成；证据 `docs/evidence/2026-10-04-q13-spatial-gate.md`。
    - 在 SPAMAP 维护记为完成的提交上跑 E3D_Gate_Set：TTY_Gate（含 Q13 组与 Q14）、`alternate_write_gate.ps1` 5 轮（13.5 存在时含 Q15）、`format0_gate.ps1`（只跑不改），全数 PASS
    - 把 `PROBE` 预判、总耗时与 Q13 组耗时抄入 Spec_Ledger「Probe 预判与门耗时」；在「取证门槛核查」填各写侧能力的门问与记为完成的提交
    - _Requirements: 2.3, 10.10, 10.15, 10.17_
  - [~] 10.5 （需用户）人工 E3D 图形会话一次
    - 用户在 E3D 图形界面 `ADD` 一个 e3d-io 写的 BOX，确认 `LIMITS CE` 不报 (2,536)
    - 截图放 `docs/evidence/assets/`，记录写入 `docs/evidence/YYYY-MM-DD-e3d-spatial-gui-session.md`
    - _Requirements: 10.11_
  - [~] 10.6 Checkpoint - 第 10 组：Ensure all tests pass
    - Ensure all tests pass, ask the user if questions arise.

- [x] 11. 发号下界与 Open_Fast_Path（需求 11；与第 3–10 组并行）
  - [x] 11.1 取证 Counter_Semantics 与索引键序
    - 经 IB_Wrapper 读 `db_get_bucket` 等例程；Counter_Semantics 另以一次 E3D_Session 实测（探针脚本放 `tools/e3d/`，副本每轮从原件重置，会话 USER 为 `e3dio`）
    - 写 Phase_3_Evidence_Doc，经 `check_evidence.ps1` 机检；结论写入 `src/core_rules.rs` 的 `alloc.counter_semantics`、`alloc.index_order`
    - _Requirements: 2.4, 11.7, 11.10_
    - F1 验证：`docs/evidence/2026-10-05-allocation-counter-index.md`；机检 PASS，四库桶表、五库 15 会话次序门通过，0 反例。环境适配：原生 e3dio 登录失败，沿用既有 SYSTEM 登录，SAVEWORK User=dpc；日志与 CLI User=e3dio 分列，不改源项目帐号。两次 NEW / 中间删除及源哈希验证通过。
  - [x] 11.2 录 Open_Fast_Path 的 Golden_Hashes
    - 在 Open_Fast_Path 首个写侧提交（11.4）的父提交上，用 `tools/golden/record.ps1` 录 `tests/golden/write_hashes.json`（固定时钟与预铸 refno）并入库；与 11.4 之间有别的提交时重录
    - 剔除的键记入 Spec_Ledger「Golden 剔除键」
    - _Requirements: 2.7_
  - [x] 11.3 读侧：`IndexCursor::last_before(RefNo)` 与 `e3d-descriptor next-word1`
    - `last_before` 放在读侧索引游标所在模块，不在 `src/write/`
    - `next-word1 --db <f> --extent <w0>` 在读侧算出「全索引 max + 1」与 Bucket_Floor，作为 Q14 与需求 11 第 5 条的期望值
    - _Requirements: 11.4, 11.5, 11.13_
    - F2：前驱/CLI 定向 18 通过、0 失败、新增无 skip，既有 1 ignored；五库 15 会话独立扫描一致、冷读至多 4 页。完整 Cargo 751/0/30，随后桶表坏计数保护定向 5/0；Clippy 63、doc 25，Off 金样 SHA-256 未变。诊断先 seek 后 scan，缺桶与坏页 1 明确回退。
  - [x] 11.4 实现 `src/write/alloc.rs`
    - F5：父 `aacdcf9`，NextAvailable 下界启用；规则回退、坏页/重复桶、历史 base/固定 latest/重试覆盖。完整 765/0/30 + 补重复桶 1/0，Clippy/doc 63/25；门槛 F1/F2 已通过；不维护页 1。
    - F4 字节中性部分通过：Allocator 接 open/open_at/固定 latest 重铸；仍 IndexOnly max+1，Pending 次序保持 open 时扫描。直接父提交 `718cf77` 上重录新金样，21 条 entries 与 F3 完全一致；完整 Cargo 755/0/30，Clippy/doc 63/25，旧 Off 17 条未变。页 1 下界留 F5。
    - `Allocator`：惰性 seek（`alloc.index_order` 未坐实时不启用 Open_Fast_Path、`open` 全扫描），以 `max_word1_scan` 为朴素对照，记下 `AllocationBasis`（页 1 解析失败记 `CounterUse::Unavailable`；Counter_Semantics 未坐实记 `Unconfirmed`，发号 = base 索引 max + 1）
    - 发号逻辑从 `src/write/transaction.rs` 抽出，接入 `open`、`open_at(base)` 与重铸（重铸取提交时读到的页 1）；`RefnoExhausted` 照旧
    - 先单独提交 Open_Fast_Path（字节中性，对照 11.2 的金样），再另一提交落发号下界；后者动手前按需求 2 第 1、2 条核对父提交，结果记入 Spec_Ledger「取证门槛核查」，并在该提交重录或删去受影响的金样、记入「Golden 剔除键」
    - _Requirements: 11.2, 11.3, 11.4, 11.6, 11.8, 11.9, 11.11, 11.12_
  - [x] 11.5 写发号测试
    - F5：必需 Corpus 最新全量、16 例保留会话 + 读侧 CLI、16 例库副本；定向 16 项通过，无新增 skip。删除最大及 Moved 一/二次单列。
    - `tests/write_stage9_alloc_real.rs`：每库最新会话全量，加 proptest 抽样保留会话号，期望值与 `e3d-descriptor next-word1` 一致
    - `tests/write_stage9_alloc.rs`：库副本 proptest（16 例），只用读侧核对新会话索引；另写示例 11.5、11.6
    - 核对 11.4 中 Open_Fast_Path 提交上 `cargo test --no-fail-fast` 的金样比对 0 失败且未跳过
    - **Property 21: Open_Fast_Path 对照全扫描**
    - **Validates: Requirements 11.4, 11.8, 11.9**
    - **Property 22: 发号下界**
    - **Validates: Requirements 11.2, 11.3, 11.11, 11.12**
    - **Property 23: 字节中性能力逐字节相同**（Open_Fast_Path 部分）
    - **Validates: Requirements 2.7, 12.8**
    - _Requirements: 11.5, 11.6_
  - [x] 11.6 测 `open` 耗时
    - F6：release 热缓存 3 次 0.1798/0.1317/0.1317 ms，中位 0.1317 ms；首次发号/草稿另报，源/副本不变；机器和存储见本次证据。
    - 用 Scale_Probe `--open-only` 在 `ams7351_0001` 上测 `Transaction::open`（release、热缓存、连续三次取中位数），另报首次发号耗时；连同机器与存储记入 Spec_Ledger「`open` 计时」
    - _Requirements: 11.1_
  - [x] 11.7 加门问 Q14
    - F6 `0ea2655`，场景修正 `f008aa4`：SITE 下命名 ZONE 避免改变旧 Q5 的 EQUI 序号；精确会话 646，读侧 26484 < 桶下界 26506，CLI/E3D 均 24384/26506；完整 39 问重跑通过。
    - 新增 `tools/e3d/write_gate_ops/q14_named.json`（在扩展 24384 写一个带名字的元素）；在 `write_gate.ps1` 与 `write_gate.mac.template` 加 Q14：用 `next-word1` 算期望 word1，与 E3D `Q REF` 比对
    - _Requirements: 11.13_
  - [x] 11.8 适配因发号下界失败的既有测试
    - 既有语义断言无需更改。16 个动态发号金样键两遍验证后以锁旧哈希的精确退役清单迁移；原 Off 文件 SHA-256 未改，未受影响及预铸批次继续校验。明细 `docs/evidence/2026-10-05-allocation-floor-goldens.md`。
    - 按需求 15 第 7 条只改期望 refno，逐条记入 Spec_Ledger「测试适配清单」；与 11.4 的发号下界提交同一提交落地
    - _Requirements: 15.7_
  - [x] 11.9 Checkpoint - 第 11 组：Ensure all tests pass
    - F7：766/0/30，Clippy/doc 63/25；39 问真门、alternate 5 轮及原样 FORMAT 0 全 PASS；新必需测试不 skip；原 17 条文件未改，动态发号键按 15.3 精确迁移；12/13/人工图形门未纳入。
    - Ensure all tests pass, ask the user if questions arise.

- [ ] 12. staging 规模（需求 12）
  - [x] 12.1 写 Scale_Probe：`examples/write_scale_probe/`（`main.rs`、`batch.rs`、`scale.rs`）
    - 一条命令 `cargo run --release --example write_scale_probe -- --source <ams8000_0001> --n 10000,100000 --runs 3 [--spatial require|off] [--open-only <库>]`；`--spatial` 在 8.7 落地后接入（见 12.5）
    - 只在临时副本上运行，原件运行前后比对 SHA-256；先整读一遍副本求得热缓存
    - 批次按第二期 T2「单会话 A」的形状构造；分阶段报 open / build / stage / commit 的墙钟中位数与峰值 RSS（`K32GetProcessMemoryInfo`，不加依赖）
    - 追加页按页头分为记录 / 索引 / claim / TYPEX / SPAMAP / 其他 / 会话页七类计数（D6：SPAMAP 单列）
    - 提交后用读侧逐个核对 owner 的成员；输出 JSON 与拟合的幂指数
    - _Requirements: 12.1, 12.5, 12.9, 12.10_
  - [x] 12.2 基线测量与剖析
    - 以 `CARGO_PROFILE_RELEASE_DEBUG=true` 构建带符号的 release，用 WPR / WPA 或同类剖析器跑 n = 10 000 与 100 000
    - 在 Spec_Ledger「剖析结论」写明所用工具、基线数字与超线性的来源；写明之前不改 staging 代码
    - _Requirements: 12.2_
    - e1425db 写侧基线：10k/100k 各三热轮 Off/Require，全量成员/属性/空间条目读回；WPR 被环境策略拒绝，改用原生子进程线程符号采样。源码无生产写侧改动；原始数据与采样局限见 `docs/evidence/2026-10-05-staging-scale-baseline.md`。收口探针 stage 比值 Off 61.7168、Require 32.3028 均失败，保留 12.4/12.5；金样混合测试已准备但 12.3 未录制。
  - [~] 12.3 录 staging 优化的 Golden_Hashes
    - 在 staging 优化首个写侧提交（12.4）的父提交上，用 `tools/golden/record.ps1` 录 `tests/golden/write_hashes.json`，含 Scale_Probe 批次（固定时钟、refno 按源文件预铸；Transaction 无显式 refno 入口时走原语路径）；与 12.4 之间有别的提交时重录
    - `tests/write_scale_golden.rs` 以 `#[path]` 复用 `batch.rs` 比对；剔除的键记入 Spec_Ledger「Golden 剔除键」
    - **Property 23: 字节中性能力逐字节相同**
    - **Validates: Requirements 2.7, 12.8**
    - _Requirements: 12.11_
  - [~] 12.4 按剖析结论优化 staging
    - 只改 12.2 指明的热点，保持字节中性：`cargo test --no-fail-fast` 下金样比对 0 失败且未跳过
    - **Property 23: 字节中性能力逐字节相同**
    - **Validates: Requirements 2.7, 12.8**
    - _Requirements: 12.3, 12.4, 12.8_
  - [~] 12.5 复测并记录
    - 在 Scale_Probe 接上 `--spatial`（依赖第 8 组），复测需求 12 第 3、4 条（含 SPAMAP 开启时）与第 6、7 条（D6：SPAMAP 页单列、上界按半满叶计），连同机器与存储记入 Spec_Ledger「规模复测」
    - _Requirements: 12.3, 12.4, 12.6, 12.7, 12.9_

- [ ] 13. S6b 页 1 桶表（需求 13，条件分支）
  - [~] 13.1 取证页 1 桶表
    - 经 IB_Wrapper 读 `DB_DB::updateBucketList`、`DB_DB::getRefBuckets`、`db_get_bucket`、`db_add_bucket`，坐实 `bucket.persist`
    - 写 `tools/e3d/s6b_probe.ps1` 驱动一次 E3D_Session：比对 `NEW` 前后的页 1 字节与会话链，分测「e3d-io 已发且仍存在」与「已发后删去」的 word1，坐实 `bucket.collision`
    - 写 Phase_3_Evidence_Doc，经 `check_evidence.ps1` 机检；结论写入 `src/core_rules.rs`
    - _Requirements: 2.4, 13.1, 13.2, 13.11, 13.12_
  - [~] 13.2 按取证结果判定分支
    - 不撞号 → 在 Spec_Ledger 把 S6b 写侧记为留待并附证据与触发条件，13.3–13.5 不做
    - 撞号且 E3D 原地重写页 1 → 停下，把「page 0 不再是唯一提交点」对崩溃一致性的影响写进 Spec_Ledger，取得用户确认后才做 13.3–13.5（需用户）
    - 撞号且页 1 写时复制、由 page 0 指向 → 做 13.3–13.5
    - 判定结果填入「规则坐实状态总表」`bucket.*` 行
    - _Requirements: 13.4, 13.5_
  - [~] 13.3 （条件）实现 `src/write/bucket.rs`
    - 以上一会话（合并时为 Merge_Latest）的页 1 为起点，把每个扩展的 Bucket_Counter 抬到使 Bucket_Floor 大于新会话索引与本批发出的每个 word1（重铸后的号计入、原号不计）；保留全部条目，计数器只升不降；落盘方式与 E3D_Core 相同
    - 动手前按需求 2 第 1、2 条核对父提交，结果记入 Spec_Ledger「取证门槛核查」；对金样的影响按 11.4 的做法处理
    - _Requirements: 13.3, 13.6, 13.9, 13.10_
  - [~] 13.4 （条件）写页 1 测试：`tests/write_stage10_bucket.rs`
    - 库副本 proptest，只用读侧（`BucketTable::parse`）读回
    - **Property 27: 页 1 桶表写侧（仅当需求 13 第 3 条成立）**
    - **Validates: Requirements 13.3, 13.6, 13.9, 13.10**
  - [~] 13.5 （条件）加门问 Q15
    - 放在 `tools/e3d/alternate_write_gate.ps1`，须在 `c61b097` 上 FAIL；`alternate_write_gate.ps1` 5 轮不回归
    - _Requirements: 13.7, 13.8_

- [ ] 14. 文档、回归与收尾（需求 14、15、16）
  - [~] 14.1 更新 `MISSION.md`
    - 按 D1 推荐默认改范围表述：SPAMAP 纳入写侧，几何求解只做 SPAMAP 所需的 BOX / CYLI 包围盒；「写回」节的门问数改为该提交上 TTY_Gate 的实际门问数
    - _Requirements: 14.1, 14.2, 14.7_
  - [~] 14.2 更新 `README.md`
    - Write path 节列出 SPAMAP（13.3 成立时另列页 1 桶表及其落盘方式）；`e3d-write apply` 旗标清单写 `--spatial require|skip|off`（默认 `require`）、`aabb` 的形状与可带它的指令；门描述改为实际门问数并列出新增问号；以仓内相对路径引用本期每份 Phase_3_Evidence_Doc
    - _Requirements: 14.3, 14.5, 14.6, 14.8, 14.9_
  - [~] 14.3 更新 `CHANGELOG.md` 的 `Unreleased`
    - 写明公开面变化（`with_spatial`、`set_aabb`、`StagedSession::spatial`、`TransactionCommit.spatial`、`TransactionError::Spatial` 与 `#[non_exhaustive]`）及迁移方法（含下游结构体字面量）、D3 的默认拒写、发号下界
    - _Requirements: 14.4, 15.9_
  - [~] 14.4 跑 Build_Checks 并与基线对照
    - `cargo test --no-fail-fast`、`cargo clippy --all-targets`、`cargo doc` 全绿，与 1.1 的基线对照；列出 skip 清单（skip 不计为通过），记入 Spec_Ledger「skip 清单」
    - _Requirements: 15.1, 15.2, 15.6, 15.8_
  - [~] 14.5 下游 `cargo check`
    - 合并到 `main` 后做，或用依赖覆盖（path 指向本工作树）做，覆盖全部 Downstream_Crates；与 e3d-io 无关的既有错误只记录；结果记入 Spec_Ledger「下游检查」
    - _Requirements: 15.4, 15.5_
  - [~] 14.6 Spec_Ledger 收尾
    - 不做项与触发条件定稿；实测数字、规则坐实状态与决策记录定稿；「D# 变更」写明有无改选
    - _Requirements: 16.1, 16.2, 16.3, 16.4, 16.5, 16.6, 16.7, 16.8, 16.9_
  - [~] 14.7 Checkpoint - 最终：Ensure all tests pass
    - Ensure all tests pass, ask the user if questions arise.

## Notes

- 标 `*` 的子任务是可选测试，可为更快的 MVP 跳过。不可跳过的测试：需求 2 第 5 条的写侧读回（8.11 的 Property 9②、8.12 的 Property 14、11.5 的 Property 22、条件成立时 13.4 的 Property 27）、逐字节对比（8.13 的 Property 16，11.5、12.3、12.4 的 Property 23）与 Corpus 核对（5.5、7.2、6.3 的 Property 5、7、8）。
- 每个性质测试以注释 `// Feature: write-path-phase-3, Property N: <标题>` 标注；同一性质分层或分文件实现时每处都带同一标注。样本缺失时经 `common::missing` 报 skip，skip 不计为通过。
- 规则坐实前，测试另以 `RuleSet::candidates()` 跑同一组生成器，并断言 Transaction 视同无可用 World_AABB；坐实后期望条目改由 `confirmed()` 算。
- 「（需用户）」标出须由人来做或确认的步骤：8.7 的下游冲突确认、10.5 的人工图形会话、13.2 的页 1 原地重写确认；改选任一 D# 也须用户决定，改选时只动 design「D# 对照表」所列落点。
- 每个 Checkpoint 做增量验收；Spec_Ledger 的记录写在下一节。

## Spec_Ledger

### 决策

D1–D8 全部按推荐默认，「细化时采用的默认」照用；用户未改任何决策。

| D# | 采用 | 落点 |
|---|---|---|
| D1 | 混合：BOX、CYLI 由 e3d-io 自算，其余几何 noun 由调用方给 World_AABB | `BUILT_IN_NOUNS`（`src/spatial/aabb.rs`）、`aabb.*` 规则、MISSION、README |
| D2 | 编码与表维护对全部 Geometric_Noun 通用 | `spamap.coverage`（`src/core_rules.rs`） |
| D3 | 默认「要求」：缺 World_AABB 拒写整批；「跳过」「关闭」须显式选择 | `SpatialPolicy` 的 `#[default]`、`with_spatial` 缺省、CLI `--spatial` 缺省 |
| D4 | `open` ≤ 1 s 与发号下界、staging 近线性纳入；S6b 按取证条件纳入 | `src/write/alloc.rs`、Scale_Probe 与 staging 优化、`src/write/bucket.rs`（条件） |
| D5 | Q13 组加进 `write_gate.ps1`，另两门只须不回归，人工图形会话一次 | `write_gate.ps1` 与两份模板、`write_gate_ops/q13_*.json` |
| D6 | 「≤ 1.5 × 记录页」不含 SPAMAP 页，SPAMAP 页单列、上界按半满叶计 | Scale_Probe 的页分类与阈值常量 |
| D7 | 未坐实规则在默认构建中不执行，按 D7 回退表处理 | `RuleSet::confirmed()` / `Pending` 槽、const 断言、`tests/core_rules_registry.rs` |
| D8 | 与 E3D_Core 一致，只重算坐实会重算的后代 | `core_rules::descendants_included` |

规格修订（2026-10-03，里程碑 D 收尾，`docs/plans/2026-10-03-write-path-phase-3-spec-revision-d.md`，Plannotator 22:35 `approved`、无批注 → Q1–Q4 全按推荐项）：Built_In_AABB 并入 IMAP 3 成员的框，成员清单 NBOX、NCYL、NCON、NPYR（需求 6 第 1、2、15 条，术语表）；重算属性与 noun 无关，Recompute_Set ②④ 加成员次序、⑤ 改为祖先任一重算属性、加 ⑦（IMAP 3 元素的改动记在第一个 IMAP 不为 3 的祖先上），需求 5 第 8 条、需求 7 第 4、6 条，新增需求 7 第 14 条。D1 不变（`BUILT_IN_NOUNS` 仍为 BOX、CYLI，成员清单另列）。

### 本期不做与触发条件

每项触发条件按需求 16 第 9 条写成「谁 · 在哪里 · 看到什么」。

| 条款 | 不做项 | 触发条件 |
|---|---|---|
| 16.1 | UDA 表 `UDATAB` / `UDAFTB` / `UUIDAR` | Corpus_Census · 新增 Corpus 项目库的普查输出 · 会话表目录列出其中任一张表的名 hash |
| 16.1 | 类型 17 逐槽三方合并与冲突 576 | 调用方或某条测试 · 一次 Merge_Commit 的提交报告或测试输出 · 本批与 Merge_Latest 改了同一元素的同一个类型 17 属性 |
| 16.1 | extract 模型 / Central / 多 extent 文件 / compact 记录 / E3D 2.1 / 部分存盘 | 调用方 · 需求 · 对其中一项提出明确需求；或 Corpus_Census · Corpus · 出现需要它们的文件（现状：940 694 条叶值 flag 全为 1 即全单 extent，`ams8000_0001` 的 6 639 个元素全 wide） |
| 16.1 | `AttrValue` 加 `TextList`（第二期定为不做） | 调用方 · 需求 · 要求写文本列表属性 |
| 16.1 | 类型 18 `UDASTB` | Corpus_Census 或读侧 · Corpus · 出现设了值的 `UDASTB` |
| 16.2 | `docs/evidence/2026-09-22-core-dll-record-draft-data-flow.md` 差异 ②（explicit 零值不存）、③（项变大删旧追加流末）、④（hunk 位置）、⑥（段最少 2 载荷字）、⑦（implicit 无置未设）、⑧（compact 转 wide）只记不改；⑦ 下本 crate 对 `Unset` 写哨兵，是 E3D_Core 行为的超集 | 调用方 · 需求 · 要求该结构与 E3D 逐字节相同；或某道门 · 门日志 · 因该差异输出 `FAIL <问号>` |
| 16.3 | 上列差异引起的失败不就地修 | 某道门或某条测试 · 门日志或 `cargo test` 输出 · 因上列任一差异失败 → 先取得用户对范围变化的确认（需用户），再补一条需求纳入本期并实现 |
| 16.4 | Built_In_AABB 之外的包围盒求解 | 用户 · Spec_Ledger「决策」 · D1 改选备选 B |
| 16.5 | 外部输入（如 SPRE、CATR 所指元素的属性）改动后重算引用它的元素的条目；重算属性只取「自身」一类 | Phase_3_Evidence_Doc · 「结论」节 · 坐实 E3D_Core 在该改动所在会话重算这类条目；或某道门 · 门日志 · 因此 FAIL |
| 16.6 | `Transaction::delete` 删除整棵子树（现只删元素本身） | 调用方 · 需求 · 要求随元素删除其后代 |
| 16.7 | D8 备选：owner 或祖先位姿改变时照样重算后代条目 | 用户 · Spec_Ledger「决策」 · 改选 D8 备选 |
| 16.8 | HTTP 层、E3D 2.1 写、改动下游源码 | 用户 · 需求 · 提出明确需求 |
| 16.8 | T4 Q17 活 E3D 持锁实测门（已由 `tests/write_stage7_lock.rs` 与 IDA 取证覆盖） | 一次 E3D_Session · 会话日志 · 持锁行为与该测试或取证结论不符 |
| 13.5 | S6b 页 1 写侧（仅当 13.2 判定不撞号时列入） | 由 13.2 填写证据与触发条件 |

### 待填记录表

内容留空，执行时填写。

#### 基线数字（1.1）

| 项 | 值 |
|---|---|
| HEAD | `3e16423`（= `c61b097` + 启动时未提交改动；测量后由编排者单独提交，构建内容与测量时相同）；测于 2026-10-01，`CARGO_TARGET_DIR=target-phase3` |
| 测试二进制数 | 101 个（`Running` 行：lib 单测 1 + bin 单测 2 + `tests/*.rs` 98），另 Doc-tests 1 个 |
| passed / failed / ignored | 626 / 1 / 30。唯一失败 `tests/hunk_resolve_real.rs::e3ds_own_int_array_hunk_resolves_to_its_5709_ints`（第 50 行，期望页 16456、实得 16600）为既有失败（样本漂移）：`ams8000_0001` 原件于 2026-09-28 被 E3D 追加会话 641–646，DBMVAL 链起页随之改变；在干净的 `c61b097` 上同样失败。验收口径：相对基线不新增失败 |
| clippy warning 数 | 63（去重；cargo 各目标汇总 85，其中 lib test 报 22 个重复），0 error |
| `cargo doc` warning 数 | 25（全在 lib doc：private / broken intra-doc link），0 error |
| 报 skip 的测试与所缺样本 | 共 7 个测试：`element_version_census::how_often_is_the_answered_copy_not_the_richest`、`sampled_elements_resolve::every_sampled_element_resolves_to_itself_in_the_shape_recorded` 缺 `evidence/20260826-tty-sampling/sample.tsv`；`live_tree_sample_resolves::{both_storage_layouts_are_covered, every_sampled_element_resolves_to_itself_in_the_shape_recorded}` 缺 `evidence/2026-08-30-tty-sampling/sample.tsv`；`live_tree_sample_resolves::the_sample_covers_every_bucket_the_old_list_did_that_a_live_tree_can_witness` 两份 `sample.tsv` 都缺；`refno_dbno_corpus::every_installed_corpus_key_decodes_to_its_descriptor_dbno`、`world_refno_corpus::every_installed_database_keeps_its_world_where_the_descriptor_says` 缺环境变量 `E3D_CORPUS_ROOTS`。无测试经 `common::missing` 报装机样本缺失。详见 `docs/evidence/2026-10-01-write-path-phase-3-baseline.md` |

#### IB_Wrapper 实机冒烟（2.5）

| 路径 | 命令 | 退出码与结果 | 日期 |
|---|---|---|---|
| 起桥与 idalib（Core_IDB），`exec` 一次 | 桥（server PID 13404）与 `idalib-4000`（2026-09-22 起开着 Core_IDB）是本机常驻环境，未另起；`ib.ps1 exec --sql "SELECT name, start_ea FROM funcs WHERE start_ea = 0x5a1d090"` | 0。未给实例号，按 `meta.idb_path` 解析到 `idalib-4000`（同桥另有 `plant210\core.dll.i64` 的实例，未误中），返回 `?startSearch@DB_RefTableIterator@@…` | 2026-10-03 |
| 多实例 | 实机构造不出：IDA 打开 IDB 即锁其文件，同一 Core_IDB 起不了第二个实例。改由 `tools/ida/selftest.ps1` 以替身 ida_bridge 的固定 `list`（两实例同指目标，大小写与斜杠不同）覆盖 | 94，逐个列出 `idalib-21` / `idalib-22`、PID、占用 none | 2026-10-03 |
| 被占用 | `ib.ps1 exec --stateful --session-id ib-smoke -c …` 之后：不带 session 调用；`--session-id someone-else` 调用；持有者再调；`ib.ps1 reset --session-id ib-smoke --release` | 0 / 94（`idalib-4000  pid 4000  held by session ib-smoke`）/ 94 / 0 / 0；释放后 `list` 无占用 | 2026-10-03 |
| 超时 | `ib.ps1 --timeout-s 3 exec -c "import time; time.sleep(12); _result_ = 'woke'"` | 95，3.85 s 返回（时限 + 10 s 内），印实例号与停 / 重开命令；实例未停，sleep 结束后照常应答 | 2026-10-03 |
| `check_xrefs.ps1`（≥ 5 个地址，含一个被大量引用的） | `0x586526b`（`@__security_check_cookie@4`，10 516 引用）、`0x5864da3`（`??2@YAPAXI@Z`，10 319）、`0x5a1d090`、`0x5ab91b0`、`0x5aecbe0`、`0x5ae9960` | 6 / 6 PASS：助手与 SQL `xrefs`（去 flow）的源地址集合逐一相同。SQL 侧按 ida-bridge 单次结果 10 000 行上限分页 | 2026-10-03 |
| `env` 首尾 SHA-256 | `ib.ps1 env`，本轮首末各一次 | 首尾同为 `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C`；IDA 9.2；HEAD `ebad935` | 2026-10-03 |

#### SPAMAP_Baseline 读数（3.4）

| 项 | 需求 3 第 3 条 | 实读 |
|---|---|---|
| 页数 | 244 | 244 |
| 叶条目 | 3 746 | 3 746 |
| 不同 refno | 3 628 | 3 628 |
| k = 0 | 3 625 | 3 625 |
| k = 1 | 121 | 121 |
| 改用受控快照（路径与原因） | — | 不需要：原件按 `DbView::open_at(…, 640)` 读，五项全符（2026-10-03，`tests/spamap_reader_real.rs`），坐实「640 的表即 09-22 所测那张」 |

各库最新会话（同一测试，2026-10-03）：`ams8000_0001` 会话 646，244 页 / 3 746 条；`ams7999_0001` 会话 200，1 387 页 / 23 891 条 / 21 257 个 refno；`ams7351_0001` 会话 116 **无 SPAMAP 表**；`ams251142_0001` 会话 10，15 页 / 327 条；`TES023` 会话 109，13 页 / 305 条。按 refno 查询与按 refno 稳定排序后的分组逐条相同。

#### 规则坐实状态总表（第 7 组、11.1、13.1）

| 规则名 | 取证任务 | 状态 | Evidence_Doc | 提交 | 备注 |
|---|---|---|---|---|---|
| `spamap.table_shape` | 7.1 | 坐实 | `docs/evidence/2026-10-03-spamap-encoding.md` | C1 | core `sub_5AB91B0` 键 [Key0, refno, refno, 下标 − 1]、值 7 字；Corpus 1 903 页按此解码（`tests/spamap_page_headers_real.rs`） |
| `spamap.table_header_rest` | 7.1 | 坐实 | `docs/evidence/2026-10-03-spamap-encoding.md` | C1 | core `sub_5AFF6D0` 新根页头 [5, hash, 0, 4, 7, 1, 页字数 − 7]；1 903 页（53 内结点）第 3–5 字全为 4、7、1，内结点只以出处 ② |
| `spamap.word_sign` | 7.1 | 坐实 | `docs/evidence/2026-10-03-spamap-encoding.md` | C1 | 有符号 32 位；Key0 ∈ [5×10^7, 1.85×10^9]（Core3D `MAPKY2`）；core `sub_5B00F10` 有符号比键，SPAMAP 键字全 < 2^31；值域 ±999 999 999（datum 为 0） |
| `spamap.rounding` | 7.1 | 坐实 | `docs/evidence/2026-10-03-spamap-encoding.md` | C1 | Core3D `MAPINS`：`f_ldnint_val`（Fortran DNINT，半数远离零）于 10 × (坐标 − datum × 10^8)；最短轴与桶按取整后判定 |
| `spamap.key0` | 7.1 | 坐实 | `docs/evidence/2026-10-03-spamap-encoding.md` | C1 | Core3D `MAPKEY` / `MAPKY2`：min % 5×10^7 再夹 ±5×10^7，+ 10^8 × (6 × 轴 + 桶)；坐实集 Key0 28 269 / 28 269、基线 3 746 / 3 746 |
| `spamap.short_axis` | 7.1 | 坐实 | `docs/evidence/2026-10-03-spamap-encoding.md` | C1 | Core3D `MAPKEY`：并列取后一根（z、y、x），最短 extent ≥ 20 m 改取 x（只以出处 ①） |
| `spamap.zero_extent` | 7.1 | 坐实 | `docs/evidence/2026-10-03-spamap-encoding.md` | C1 | Core3D `MAPKEY` 无特例，extent 0 落桶 1 |
| `spamap.bucket` | 7.1 | 坐实 | `docs/evidence/2026-10-03-spamap-encoding.md` | C1 | Core3D `MAPKEY`：600 / 2 000 / 6 000 / 20 000 / 200 000，界值归上一桶；≥ 200 000 为桶 6（只以出处 ①） |
| `spamap.entries_k` | 7.1 | 坐实 | `docs/evidence/2026-10-03-spamap-branch-entries.md` | C2 | `MBXGO` 只由 `MPBOX2`（tube 0：一条 k = 0、自身框）与 `MPBXBR` 调；`MPBXBR` 只走 BRAN 与 HNGT noun，从尾到头给成员记 k = 0 与离开它的管子 k = 1、给 BRAN 记头管 k = 1，管框要目录 P 点与管径 → 分支 / 吊架树为 MissingInput（需求 6.15，Codec_Check 记 waiting）。坐实集 Corpus 判定 11 167 / 11 167、基线 986 / 986，waiting 14 350 / 2 642 |
| `spamap.flags` | 7.1 | 坐实 | `docs/evidence/2026-10-03-spamap-flags.md` | C2 | `MAPINS` 三个逻辑量按 1 / 2 / 4；`MPBOX` 上溯（DDEP 1、SITE / BLOCK / SSOWLD / HBLWLD 启用，WORL 不启用），不启用全 0；WBOX 时 OBST 1–3，否则 `BOXELE` OBST 1–2，IMAP 5 并入 IMAP 3 成员；box type 1 / 18 / 19 / 23 与分支树不说。坐实集 Corpus 判定 4 783 / 4 783、基线 402 / 402 |
| `spamap.coverage` | 7.2 | 坐实 | `docs/evidence/2026-10-03-spamap-coverage.md` | C2 | 按元素三态：Core3D `MPBOX` / `MLSADD` 按 noun 的 IMAP（字典字段 847458）1、5 记，7 看 owner 的 IMAP 是否为 2，box type 1（`getBoxType`，SCTN / GENSEC / WLJOIN 除外）须 SPRE 非 0（`GTSTDT` → `MBXSTL` 失败 → `MAPREM`）；BRAN 与 HNGT 四个（`MPBXBR` 未读）、UDET、模板无 SPAMAP 的八个未坐实。Property 7 坐实集 40 556 / 40 556 判定无反例，`waiting` 2 060（BRAN 1 819、HANG 241） |
| `spamap.recompute.<NOUN>` | 7.3 | 坐实 | `docs/evidence/2026-10-03-spamap-recompute.md` | D2 | 与 noun 无关：Core3D `VDESPT` 对字典 `DCHC` 非 0 的属性入队，core `PostSetAttribute` 不为 `WNOEVT` 属性发事件；`VDESRO` 入队被挪的成员。300 个 Geometric_Noun 模板 722 个属性中 176 个触发（`rules::RECOMPUTING`，`tests/spamap_recompute_real.rs` 逐 noun 打印并与 `attlib.dat` 核对）。E3D_Session 以 e3d-io 写的无条目元素区分触发与否：32 / 32（BOX 10、CYLI 8、CONE / DISH / PYRA 6、成员次序 8）。B8 那几处反例按此本应重算，归 D3 走会话链；E3 扩展全部祖先 noun 的静态 DCHC/WNOEVT 表，`ancestor_trigger_table_matches_the_dictionary` 从 attlib 重推验证通过，规则取证不变 |
| `spamap.descendants.{move,pos,ori}` | 7.3 | 坐实 | `docs/evidence/2026-10-03-spamap-recompute.md` | D2 | 换 owner（`VDESIR`）与 POS / ORI 改动（`VDESPT`，DCHC 3）入队该元素，`MPBOX` 走整棵子树，各层都重算，不在 IMAP 0、BRAN、HNGT 之下继续（`rules::mpbox_walks_below`）。E3D_Session：move 4 / 4、POS 7 / 7、ORI 6 / 6（第 1、2 层，有无条目两种）。另见证据「规格外的发现」：owner 的 NUMB 等其他重算属性、容器 REORDER 同样整树重算——已由 2026-10-03 规格修订收进 Recompute_Set ②④⑤ 与需求 7 第 4 条（Plannotator 通过） |
| `spamap.delete_residue` | 7.3 | 坐实 | `docs/evidence/2026-10-03-spamap-recompute.md` | D2 | 不留残留：删除那一会话中被删元素与其各层后代的条目都已删去（出处 ②，4 / 4；Core3D 删除路径只动队列，删键之处未读）。需求 5 第 9 条：删无条目 BOX、删其 owner 都能 `SAVEWORK`，无错误号 |
| `spamap.record_item.<NOUN>` | 7.4 | 坐实 | `docs/evidence/2026-10-03-spamap-record-item.md` | C3 | 各 noun 相同：core `sub_5AB91B0` 类型 14 写条目时把 Key0 写进记录 `SPAMAP` 属性第 k 槽，新项整列先填 0x80000001；位置按模板（BOX implicit word 35、存储 2 字）。Corpus 25 517 / 25 517、基线 3 628 / 3 628 带且逐槽相符，46 个 noun 无例外（`tests/spamap_record_item_real.rs`） |
| `aabb.ori_form` | 7.5 | 坐实 | `docs/evidence/2026-10-03-spamap-aabb-members.md` | D3 | ① core `GTORI1`：R = Rz·Ry·Rx，`SINCOS` 在 0 / ±90 / ±180（`LREQ` 容差 0.0001）给精确值，其余 sin / cos(角 / 57.29577951308232)（C4 读）；FRMC 为 0 的 noun 经 `GTDORI` 同到 `GTORI1`；`rules::ori_by_gtori1`；Built_In_AABB 1 590 / 1 590 |
| `aabb.compose` | 7.5 | 坐实 | `docs/evidence/2026-10-03-spamap-aabb-members.md` | D3 | ① Core3D `WSTRAM`：从元素往上 `CONCAT(T_级, 已累积)` 到 WORL（C4 读）；成员一级为 `MPBOX2` 的 `CONCAT(元素变换, 成员位姿)`；`rules::every_pose_from_the_element_up`，成员系 `WorldFrame::then` |
| `aabb.box` | 7.5 | 坐实 | `docs/evidence/2026-10-03-spamap-aabb-members.md` | D3 | ① `BOXELE` box type 3（BOX、NBOX）→ core `MBXBOX`：中心 ± Σ\|R·半长\|；`rules::box_by_mbxbox`；BOX 384 / 384 |
| `aabb.cyli` | 7.5 | 坐实 | `docs/evidence/2026-10-03-spamap-aabb-members.md` | D3 | ① box type 2（CYLI、NCYL）→ `MBXSNO`(HEIG, 0, 0, DIAM, DIAM) → 两端圆 `MBXCIR` r·√(1 − z_i²) 求并；`rules::cylinder_by_end_circles`；CYLI 1 206 / 1 206（`71552`、`71555` 的 2.1 mm 来自 NBOX 成员） |
| `aabb.cone` | 7.5 | 坐实 | `docs/evidence/2026-10-03-spamap-aabb-members.md` | D3 | ① box type 5（CONE、NCON）：HEIG < 0.5 取 0.5、DTOP / DBOT < 0 取 0，`MBXSNO`(HEIG, 0, 0, DTOP, DBOT)，DTOP 在 +HEIG / 2 一端；`rules::cone_by_end_circles`；适用 1 个 NCON（`24383/77138`） |
| `aabb.pyramid` | 7.5 | 坐实 | `docs/evidence/2026-10-03-spamap-aabb-members.md` | D3 | ① box type 7（PYRA、NPYR）：只 HEIG < 0.5 取 0.5，core `MBXPYR` 八角（顶 XOFF / 2 ± XTOP / 2、底 −XOFF / 2 ± XBOT / 2，y 同理）经 `MBXPOI`（`TRAVEC`）取框；`rules::pyramid_by_corners`；适用 18 个 NPYR（9 个 BOX）全对 |
| `aabb.members` | 7.5 | 坐实 | `docs/evidence/2026-10-03-spamap-aabb-members.md` | D3 | ① Core3D `MPBOX2` n 37–79：IMAP 5 的元素对每个 IMAP 3 成员（FRMC 为 0 → `GETRAM` + `CONCAT`）跑 `BOXELE`，按轴并入元素框；② `tools/e3d/spamap_probes/d3/`：改 LEVE 后 18 个条目不变，各加大 100 mm 后改写、成员伸出的几侧照旧。`rules::imap_3_into_imap_5`；成员只算 NBOX / NCYL / NCON / NPYR，其余 noun → MissingInput（D7）。C4 的 18 个反例全由此解释，未剔除 |
| `aabb.degenerate` | 7.5 | 坐实 | `docs/evidence/2026-10-03-spamap-aabb.md` | C4 | ① Core3D `BOXELE`：HEIG / DIAM、XLEN / YLEN / ZLEN 小于 0.5 一律取 0.5（含负、0）；`aabb.rs` 改为对 < 0.5 的尺寸查此规则；TES023 两个零尺寸 CYLI 2 / 2；Corpus 分不出与旧候选，只以 ① 坐实。成员同：NBOX、NCYL 各尺寸与 NCON、NPYR 的 HEIG（D3） |
| `aabb.arithmetic` | 7.5 | 坐实 | `docs/evidence/2026-10-03-spamap-aabb-members.md` | D3 | ① 上列例程全 double（含 `MBXPYR`、`MBXPOI`、`TRAVEC`、`MVMULT`）；`rules::double_precision`；1 590 / 1 590 逐字，需求 6 第 16 条的放宽判据不需要 |
| `alloc.counter_semantics` | 11.1 | 坐实 | `docs/evidence/2026-10-05-allocation-counter-index.md` | F1 | NextAvailable：页 1 直接返回、缓存随后 +1；SYSTEM NEW 26506、删除后 NEW 26507；源库不变，原生 User=dpc 分列 |
| `alloc.index_order` | 11.1 | 坐实 | `docs/evidence/2026-10-05-allocation-counter-index.md` | F1 | 固定宽有符号词典序，native 普通两字非负；五库 15 个最新/保留会话原始次序 0 反例 |
| `bucket.collision` | 13.1 | | | | |
| `bucket.persist` | 13.1 | | | | |

#### 取证门槛核查（7.7；另见 11.4、13.3、10.4）

| 写侧能力 | 首个写侧提交 | 父提交 | Evidence_Doc | 读侧解码与 Corpus 测试 | 门问 | 记为完成的提交 |
|---|---|---|---|---|---|---|
| SPAMAP 维护 | 8.1（里程碑 E 的 E1） | `474ea14`：E0 在其上重跑 SPAMAP 相关 9 个测试二进制，0 失败、无 skip（预核于 `d8fecb6` / `6811730`，其后只改文档） | 7.1 `docs/evidence/2026-10-03-spamap-encoding.md`、`…-spamap-branch-entries.md`、`…-spamap-flags.md`；7.2 `…-spamap-coverage.md`；7.3 `…-spamap-recompute.md`；7.4 `…-spamap-record-item.md`；7.5 `…-spamap-aabb.md`、`…-spamap-aabb-members.md`——结论都含例程地址与 Corpus 核对，`check_evidence` 全 PASS；第 7 组规则全部坐实（规则坐实状态总表） | 读侧解码：`src/index/spamap.rs`（SPAMAP_Reader）与 `src/spatial/`（codec、aabb、check、state），不在 `src/write/` 下、代码不引用 `src/write/`。以 Corpus 为输入的测试在 `d8fecb6` 的 Build_Checks 中 0 失败、未跳过：`spamap_reader_real`（3.4）、`spamap_codec_real`（5.5、7.2，含 4.20）、`spamap_aabb_real`（6.3）、`spamap_coverage_real`、`spamap_record_item_real`、`spamap_recompute_real`、`spamap_page_headers_real`、`spamap_census_real`；坐实集下各项 agree + waiting = total | Q13/Q13a–h 全 PASS；docs/evidence/2026-10-04-q13-spatial-gate.md | `fd9ed67` 库实现；`4939b8a` 门首提交 + 本次修正/验收提交（10.1–10.3）；10.4 整体待 Q14 |
| 发号下界 | F5 `6c588d1` | `aacdcf9` | F1 `d4f5ab4` 的 `2026-10-05-allocation-counter-index.md` | F2 `6232c21` 的 Corpus/CLI；F5/F7 全套 766/0/30、无新增 skip；固定 base/latest、16 例历史/副本模型 | Q14 24384/26506；完整 39 问、alternate 5 轮、FORMAT 0 全 PASS | F5 库实现；F6 `0ea2655` + `f008aa4` 门修正；本 F7 验收提交 |
| Open_Fast_Path（字节中性，按需求 2 第 7 条） | F4 | `718cf77`；在此准备提交上两遍重录，21 条 entries 与 F3 完全一致 | `docs/evidence/2026-10-05-allocation-counter-index.md#alloc.index_order`（F1 `d4f5ab4`） | F2 `6232c21` 的 `last_before` / 必需 Corpus；F4 独立 eager/lazy/scan 模型，完整 Cargo 755/0/30，金样全部通过，无新增 skip | 本优化仍 max+1；发号行为改变与 Q14 留 F5/F6 | F4（字节中性部分） |
| staging 优化（字节中性，按需求 2 第 7 条） | | | | | | |
| 页 1 桶表写侧（条件） | | | | | | |

#### Golden 剔除键（1.3、11.2、12.3）

| 金样文件 | 录制提交 | 剔除的键 | 原因 | 重录 / 删去记录 |
|---|---|---|---|---|
| `tests/golden/off_compat.json`（1.4；`write_stage6_{transaction,merge,set_by_name,spread,cli}`，留 17 条） | `d040f18` | `write_stage6_cli::` 下 5 条：`a_base_behind_the_newest_session_merges_and_reports_the_reminting#0`、`a_dry_run_makes_every_check_and_leaves_the_file_byte_identical#0`、`a_name_bound_earlier_in_the_file_resolves_before_the_commit#0`、`a_refused_instruction_names_itself_and_writes_nothing#0`、`an_instruction_file_is_applied_as_one_session_through_the_facade#0` | 两遍不同：`e3d-write` 提交用 `clock: None`（`src/bin/e3d-write.rs:711`），会话页带当时时间；五条都有一次真提交（dry run 与拒写两条也先做了一次） | — |
| 同上 | `d040f18` | `write_stage6_spread::spread_inserts_read_back_across_many_owners#0` | 两遍不同：`SessionClock::now`（`tests/write_stage6_spread.rs:91`） | — |
| `tests/golden/write_hashes.json`（F3；21 条，含固定时钟/预铸 32、128、257 BOX 批次） | F2 `6232c21` 上两遍，F3 准备提交紧接 F4 | 原 Off 录制的六个时间不稳定键，另 `write_stage6_cli::spatial_cli_writes_given_aabb_and_reports_readable_spatial_diff#0` | CLI / spread 使用当前时间，两遍末文件不同 | 原 Off 17 条未改；F4 前在直接父提交再录，只核对条目完全一致并更新录制元数据 |
| 原 Off 17 / 优化 21 的动态发号键 | F5 `6c588d1`，父 `aacdcf9` | 16 个键各在两个原文件精确退役；32 条锁旧 SHA 的 retirement；见 `docs/evidence/2026-10-05-allocation-floor-goldens.md` 逐项表 | 需求 11 有意改变新 refno；需求 15.3 不把动态发号当字节兼容输入 | 两遍新下界结果一致，`allocation_floor.json` 校验新 16 条；原文件均未改；3 预铸批次/rename/拒写金样继续执行。F4 已完整通过未退役的原 17/21 |

样本（备注）：17 条全部取自快照 `D:\e3dio-fixtures\phase3\ams8000_0001`，34 037 760 字节，SHA-256 `3CAB5D7D2B2C40FDF0829B91C555D19D55C530DD49860A4665560E5C553FE023`，即装机原件 2026-09-30 17:05:57 那一版（会话至 646）。

#### 测试适配清单（8.10、11.8）

| 测试或 Gate_Ops | 原因（D3 默认 / 发号下界） | 改动（显式 Spatial_Policy / 期望 refno） | 提交 |
|---|---|---|---|
| `write_gate` refused 六批次 | CLI 默认 Require；保留旧拒写原因 | 显式 Off，原期望不改 | 4939b8a |
| `10_box`（含 dry-run） | CLI 默认 Require；保留旧几何读回门 | 显式 Off，原期望不改 | 4939b8a |
| `20_group` | 同上，保留引用成员门 | 显式 Off，原期望不改 | 4939b8a |
| `30_hunk_equis` | 同上，保留 hunk 门 | 显式 Off，原期望不改 | 4939b8a |
| `40_hunk_fixing` | 同上，保留字符串列表门 | 显式 Off，原期望不改 | 4939b8a |
| `50_moves` | 同上，保留原 BOX 创建门 | 显式 Off，原期望不改 | 4939b8a |
| `51_moves_apply` | 同上，保留 move/reorder/reference 门 | 显式 Off，原期望不改 | 4939b8a |
| `60_big` | 同上，保留 8200 BOX 成员计数门 | 显式 Off，原期望不改 | 4939b8a |
| `70_strings` | 同上，保留按名设值门 | 显式 Off，原期望不改 | 4939b8a |
| `80_text` | 同上，保留中文字节兼容门 | 显式 Off，原期望不改 | 4939b8a |
| `90_merge` | 同上，保留原 E3D 三方合并门 | 显式 Off，原期望不改 | 4939b8a |
| `alternate_write_gate` 探针 / 五轮 IO EQUI | 同上，保留原交替写门 | 显式 Off，原期望不改 | 本次修正/验收提交 |
| `write_stage6_transaction::a_facade_insert_equals_the_primitive_path_byte_for_byte` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_transaction::a_facade_rename_moves_the_name_and_rewrites_the_record` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_transaction::a_facade_delete_drops_the_key_and_the_owner_membership` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_transaction::move_to_reparents_the_box_and_rewrites_exactly_both_owners` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_transaction::reorder_moves_a_member_within_its_owner_and_rewrites_only_the_owner` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_transaction::reference_members_are_edited_on_the_group_and_nowhere_else` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_transaction::a_move_within_the_inserting_transaction_touches_only_the_final_owner` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_merge::a_transaction_opened_at_the_base_merges_onto_what_was_committed_meanwhile` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_merge::a_same_attribute_clash_is_refused_by_default_and_taken_under_writer_wins` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_merge::a_change_over_a_delete_a_delete_over_a_change_and_a_taken_name_are_refused` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_merge::a_writer_that_commits_between_staging_and_page_0_is_merged_over_once` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_set_by_name::every_shape_set_by_name_reads_back_as_the_value_it_names` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_set_by_name::the_dictionary_holds_a_set_to_what_e3d_allows` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_set_by_name::a_name_set_twice_in_one_transaction_files_the_last_one_only` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_set_by_name::what_cannot_be_read_is_refused_and_writes_nothing` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_set_by_name::a_value_set_by_name_as_a_hunk_is_stored_out_of_the_record_and_reads_back` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |
| `write_stage6_spread::spread_inserts_read_back_across_many_owners` | E 默认 Require；保留原测试及逐字节兼容语义 | 所有 Transaction 显式 `SpatialPolicy::Off`；期望不改 | E2–E9（`fd9ed67`） |

CLI：`src/bin/e3d-write.rs` 显式 Off 作为 E→F 过渡，不新增空间参数或报告；既有五个 CLI 测试期望不变。

#### Write_CLI 收口验收（9.2、9.3）

2026-10-04；提交以 `test(cli): 9.2-9.3 validate spatial parsing and real runs` 标识（本次原子提交）。9.1 基线为 `426484d`。

| 检查 | 结果 |
|---|---|
| 既有 CLI 定向检查 | 7 / 0 / 0，所有期望保留，调用显式 Off |
| 新增解析 / 真跑 | 6 / 11 项，全通过，无 skip；解析 3×256 例、库副本随机 16 例 |
| 全套测试 | 741 / 0 / 30（123 个测试二进制 + Doc-tests） |
| 既有 skip | 7 个提前返回测试 / 8 条提示，见下方清单；新增无 skip |
| Clippy / doc | 62 / 25 warning，不高于 63 / 25 基线 |
| Golden | 17 条全部通过；off_compat.json SHA-256 `1b2ac106e9a69a9a5fe677b439ae7c43ef190191dcd8d392b2375a6602e4e80a` 与 HEAD 一致，未重录 |

解析阶段拒绝 min>max；AABB 指令错误报告序号，其他既有文件解析错误语义保留。空间拒写沿固定 base 的 owner 链关联来源（包括祖先编辑触发的后代）；提交重铸修正操作行的 refno/owner/group/member/位置引用，given 原号保留。报告 JSON 结构沿用 9.1，未新增依赖。

日志：`.agents/tasks/write-path-phase-3-cli/validation/{tests,clippy,doc}.log`。第 10 组 Gate_Ops、TTY/E3D 与人工图形门均未在此次执行或记为通过，性能、发号、页 1 桶表留待后续。

#### Probe 预判与门耗时（10.4）

2026-10-04：基线 `dcc71d8`，脚本首提交 `4939b8a`；真实门发现 BOM 与默认整毫米显示的判分问题，修正后完整重跑。10.1–10.3 经验收打勾；10.4 部分完成，Q14 待第 11 组，10.5、10.6 待办。详见 `docs/evidence/2026-10-04-q13-spatial-gate.md` 及同名 assets 的报告、框、SAVEWORK、manifest。

| 本次自动验收 | 结果 |
|---|---|
| TTY | 38 PASS / 0 FAIL：Q0 + 既有 28 问 + Q13/Q13a–h |
| alternate / FORMAT 0 | 5 轮 T4a–c PASS；FORMAT 0 原样 E1/E2/F0–F5 全 PASS |
| 离线 | 38 项纯检查；库副本六批次、读侧 11 字与 Probe 共 154 项，无 skip |
| Cargo / Clippy / doc | 742 / 0 / 30；63 / 25 warning。既有 7 个提前返回测试不变，17 条 Golden SHA-256 不变 |
| Q13g | SAVEWORK 精确会话 666，退出 latest 667 不用于核对；五项各 1/1、waiting=0；完整 11 字相等 |
| Q13h | CYLI、变换 BOX 每轴误差 [0,0,0] mm；E3D 显示设 `PRECISION 3DP`，阈值保持 0.1 mm |

测试适配补记（仅显式策略，原问与期望不改）：`write_gate.ps1` 的 `refused/*`、10_box dry-run、10_box、20_group、30_hunk_equis、40_hunk_fixing、50_moves、51_moves_apply、60_big、70_strings、80_text、90_merge 全部显式 Off；`alternate_write_gate.ps1` 的探针 dry-run 与五轮 IO EQUI 批次显式 Off。`format0_gate.ps1` 未改。Q13_control 为 Off，另五批次 Require，未提供调用方 AABB。

写侧门槛补记：E 的插入 / 空间维护由 Q13a–c，move / XLEN / delete 由 Q13d–f，E3D 保存兼容由 Q13g，CYLI 与祖先旋转 / 平移由 Q13h 坐实。库 SPAMAP 实现仍为 `fd9ed67`；本次未扩大规则、依赖、性能或发号范围。

| 门问 | Probe_Element | 被测 BOX | 余量 | 预判（相交 / 不相交） |
|---|---|---|---|---|
| Q13 | 16192/12 | 16192/6 | 0 mm | 相交，搜索省略 Control；Control 作体积 (2,536) |
| Q13d 旧位 | 16192/13 | 16192/8 | 0 mm | move 前相交、后不相交 |
| Q13d 新位 | 16192/14 | 16192/8 | 0 mm | move 前不相交、后相交 |
| Q13e | 16192/15 | 16192/9 | 0 mm | XLEN 前不相交、后相交 |
| Q13f | 16192/16 | 16192/10 | 0 mm | 删除前相交、后省略 |

| 运行提交 | 总耗时 | Q13 组耗时 |
|---|---|---|
| 4939b8a + 本次门判分修正（第二提交） | 199.5563017 s | 161.8146313 s |

#### `open` 计时（11.6）

F7 自动门补记：`f008aa4` 上完整 TTY 39 PASS / 0 FAIL，195.2950907 s；Q13 151.8869362 s、Q14 8.6877227 s。Q14 在副本会话 646 由读侧诊断 26484 < 26506，CLI 与 E3D 都为 24384/26506。Q13 refno 与 Probe 和上一表相同，预判均成立；SAVEWORK 精确会话 667（不用退出 latest 668）。alternate 五轮 64.063 s，FORMAT 0 10.093 s，全 PASS；源 AMS/TES 哈希/长度不变。完整 Cargo 766/0/30，Clippy/doc 63/25，新必需测试无 skip；原始日志/报告/manifest 在 `docs/evidence/assets/2026-10-05-allocation-q14/`。首次 Q14 EQUI 干扰 Q5 序号的失败日志保留；改为 SITE 下命名 ZONE，原问/期望不改。

F7 文档同步：README/CHANGELOG 明确发号下界及报告取号迁移；补此前未同步的 SPAMAP 默认策略/CLI AABB 和 MISSION/README 门问数。第 14 组最终文档/下游收尾仍待后续；本次仅关闭第 11 组和自动门 10.4。

| 库 | 三次耗时 | 中位数 | 首次发号耗时 | 机器与存储 |
|---|---|---|---|---|
| `ams7351_0001`，release，热临时副本 | 0.1798 / 0.1317 / 0.1317 ms | 0.1317 ms ≤ 1 s | 0.1365 / 0.0208 / 0.0217 ms，含 insert 草稿，不提交 | Ryzen 9 7950X、约 63 GiB、Win11 26200，源 D:/临时 C: 均 WD_BLACK SN770 NVMe；源/副本字节不变；`docs/evidence/2026-10-05-allocation-q14-gate.md` |

#### 剖析结论（12.2）

| 剖析工具 | n | 各阶段中位数 | 超线性来源 |
|---|---|---|---|
| 原生 x64 线程采样（WPR 因 0xc5585011 无法启用）；release + PDB | 10k / 100k，各 3 热轮 | Off open/build/stage/commit：0.4058/14.6314/90.7039/0.3730 → 0.4954/114.4453/5680.7904/0.5153 ms；Require：0.2084/17.3473/1140.5584/3.0621 → 0.3043/148.1289/36437.9006/20.0276 ms | Off stage 栈 INDEX rewrite_node 122/197、成员解析 69/197；Require Snapshot::value 533、EndState::record 390（inclusive）。尾叶逐次扫描、成员 contains、owner 克隆及重复属性提取；生产写侧尚未改。见 docs/evidence/2026-10-05-staging-scale-baseline.md 与原始 profile JSON；比值 62.63/31.95 失败，12.4/12.5 未验收 |

#### 规模复测（12.5）

| 条款 | `--spatial` | n | Staging_Time 中位数 | 峰值 RSS | 追加页七类计数 | 幂指数 | 机器与存储 |
|---|---|---|---|---|---|---|---|
| | | | | | | | |

#### skip 清单（14.4）

E 验收沿用既有 7 个提前返回测试；另有 Rust harness 的 30 个 ignored。新增必需测试无 skip。

| 测试 | 所缺样本 |
|---|---|
| `element_version_census::how_often_is_the_answered_copy_not_the_richest` | evidence/20260826-tty-sampling/sample.tsv |
| `live_tree_sample_resolves::every_sampled_element_resolves_to_itself_in_the_shape_recorded` | evidence/2026-08-30-tty-sampling/sample.tsv |
| `live_tree_sample_resolves::the_sample_covers_every_bucket_the_old_list_did_that_a_live_tree_can_witness` | 新、旧 sample.tsv（两条提示，一个测试） |
| `live_tree_sample_resolves::both_storage_layouts_are_covered` | evidence/2026-08-30-tty-sampling/sample.tsv |
| `sampled_elements_resolve::every_sampled_element_resolves_to_itself_in_the_shape_recorded` | evidence/20260826-tty-sampling/sample.tsv |
| `refno_dbno_corpus::every_installed_corpus_key_decodes_to_its_descriptor_dbno` | E3D_CORPUS_ROOTS 未配置 |
| `world_refno_corpus::every_installed_database_keeps_its_world_where_the_descriptor_says` | E3D_CORPUS_ROOTS 未配置 |

#### 下游检查（14.5）

| crate | 方式（合并后 / 依赖覆盖） | 结果 | 与 e3d-io 无关的既有错误 |
|---|---|---|---|
| | | | |

#### D# 变更

| D# | 改选为 | 用户确认日期 | 按 design「D# 对照表」改动的落点 |
|---|---|---|---|
| | | | |

### 提交约定

- 每个写侧能力的提交说明首行写该任务号（如 `feat(write): 8.1 …`），以便按需求 2 识别首个写侧提交。
- 8.7 与 8.10 同一提交；11.4 的发号下界与 11.8 同一提交。
- 何时提交由用户确认。

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "1.2", "1.3", "2.1", "2.3", "3.1", "4.1", "12.1"] },
    { "id": 1, "tasks": ["1.4", "2.2", "2.4", "3.2", "3.5", "4.2", "4.3"] },
    { "id": 2, "tasks": ["2.5", "3.3", "3.6", "5.1"] },
    { "id": 3, "tasks": ["2.6", "3.4", "3.7", "5.2"] },
    { "id": 4, "tasks": ["3.8", "5.3", "5.4", "11.1", "12.2"] },
    { "id": 5, "tasks": ["5.5", "6.1", "11.3", "12.3", "13.1"] },
    { "id": 6, "tasks": ["6.2", "6.3", "12.4", "13.2"] },
    { "id": 7, "tasks": ["7.1", "11.2"] },
    { "id": 8, "tasks": ["7.2", "11.4"] },
    { "id": 9, "tasks": ["7.3", "11.5", "11.8"] },
    { "id": 10, "tasks": ["7.4", "13.3"] },
    { "id": 11, "tasks": ["7.5", "13.4", "13.5"] },
    { "id": 12, "tasks": ["7.7"] },
    { "id": 13, "tasks": ["8.1", "8.3", "11.6"] },
    { "id": 14, "tasks": ["8.2", "8.4"] },
    { "id": 15, "tasks": ["8.5", "8.6"] },
    { "id": 16, "tasks": ["8.7"] },
    { "id": 17, "tasks": ["8.8", "8.10", "8.13", "8.15"] },
    { "id": 18, "tasks": ["7.6", "8.9", "8.11", "8.12", "8.14"] },
    { "id": 19, "tasks": ["7.8", "8.16", "8.17"] },
    { "id": 20, "tasks": ["8.18", "9.1", "12.5"] },
    { "id": 21, "tasks": ["9.2", "9.3"] },
    { "id": 22, "tasks": ["10.1", "10.2"] },
    { "id": 23, "tasks": ["10.3"] },
    { "id": 24, "tasks": ["11.7"] },
    { "id": 25, "tasks": ["10.4", "11.9"] },
    { "id": 26, "tasks": ["10.5"] },
    { "id": 27, "tasks": ["10.6", "14.1", "14.2", "14.3"] },
    { "id": 28, "tasks": ["14.4"] },
    { "id": 29, "tasks": ["14.5"] },
    { "id": 30, "tasks": ["14.6"] },
    { "id": 31, "tasks": ["14.7"] }
  ]
}
```
