# 计划：e3d-io 写路径 · 第三期 · 里程碑 E（任务第 8 组：SPAMAP_Writer 接入 Transaction）

状态：**已完成库级验收**（2026-10-04；E1 `a69e318` + 本次 E2–E9 实现提交）。原计划 **评审通过**（2026-10-03 22:47，Plannotator 门 `approved`，无批注 → Q1–Q5 全按推荐项）
日期：2026-10-03
分支 / worktree：`write-path` @ `D:\work\plant-code\old\vendor\e3d-io-write`，HEAD `a69e318`（E1 已完成，未推送）
依据：`.kiro/specs/write-path-phase-3/` 需求 2 第 1、2、5、6 条、需求 6–8、需求 15 第 3、4、7、9 条（2026-10-03 规格修订后的文本），任务 8.1–8.18 与 7.6 的 6.17
上一里程碑：`docs/plans/2026-10-03-write-path-phase-3-milestone-d-plan.md`

---

## 0. 一句话

第 7 组把 SPAMAP 的编码、覆盖、重算、记录项与 Built_In_AABB 全部坐实，取证门槛（7.7）已预核通过。E 把这些接进写侧：泛化键表写时复制以写 7 值字的 SPAMAP 页（8.1），在 staging 里由「上一会话 ⊕ 本批定稿记录」求 Recompute_Set ①–⑦、取 World_AABB、编条目、算编辑集（8.3、8.4），在写任何页之前做完全部拒写判定，把 SPAMAP 页写在其他表页之后、会话页之前（8.6），给 Transaction 加 `with_spatial` / `set_aabb` 与报告（8.7，同提交适配既有测试 8.10），再接合并与重铸（8.8）和 SPAMAP_Record_Item（8.9），最后以库副本模型测试、合并测试与「关闭」逐字节兼容测试收口（8.11–8.17，8.18 检查点）。

## 1. 现状

| 项 | 状态 |
|---|---|
| HEAD | `a69e318`；E1 后基线为 115 个测试二进制 + Doc-tests 702 / 0 / 30，skip 7，clippy 63、`cargo doc` 25 |
| 规则 | 第 7 组 30 条中 SPAMAP / aabb 的 25 条全部 Confirmed（`alloc.*`、`bucket.*` 属第 11、13 组） |
| 读侧 | `src/index/spamap.rs`（SPAMAP_Reader、`read_spamap_tree`、`for_refno`）、`src/spatial/{codec,aabb,rules,check,state}.rs`（`ReadState` 实现 `SpatialState` 含 `members`） |
| 写侧（未改） | `src/write/mod.rs::stage_session_with_base`：先以读侧做 `plan_side_tables`（TYPEX / 反向表 / claim），再开写句柄、`lock_for_write`，依次写数据页 → 索引 CoW → NAME 表 → 键表（`keyed_cow::rewrite_keyed_table`，叶值宽 0）→ claim 树 → 会话页（`build_session_page` 的 `table_overrides` / `table_additions`），sync 后留 page 0 待 commit；`StagedSession::rollback` 截回原长 |
| 公开面 | `Transaction`（open / open_at / set / insert / move_to / reorder / update / rename / delete / stage / commit / commit_reporting / commit_observed）、`TransactionError`（无 `#[non_exhaustive]`）、`TransactionCommit` |
| 下游 | 8.7 的前置搜索已做：`D:\work\plant-code\old\vendor\e3d-model*`（10 个 checkout）与 `D:\work\plant-code\old\gen-model` 中没有 `TransactionError`、`TransactionCommit` 的使用，加 `#[non_exhaustive]` 与新字段不破坏下游 |
| 金样 | `tests/golden/off_compat.json`（1.4，17 条）守「关闭」下逐字节与 0.4.0 相同 |

## 2. 复核与风险

### 2.1 拒写必须在写第一页之前

需求 7 第 12 条与需求 6 第 5、8 条的拒写都要求文件逐字节不变。最终 side-table 与 SPAMAP plan 在取得写锁、读取并固定 latest session 后、追加任何页面之前完成；若锁后发现合并输入或 session 根已变化，返回 `Moved`，由现有重试逻辑重新 stage。记录项补丁先并入元素写集，再生成完整 side-table plan，避免遗漏索引、claim 或反向表更新。

### 2.2 「关闭」与 0.4.0 逐字节相同

`rewrite_keyed_table` 泛化为 `rewrite_table::<V>` 后，`V = 0` 必须字节不变：保留原函数为薄包装，现有单测与 `off_compat.json` 守住。公开原语（`stage_element_session[_with]`、`stage_merge_session_with`）签名不变、内部传 `None`（不维护 SPAMAP）。

### 2.3 默认「要求」改变既有行为（D3）

插入 BOX、CYLI 时 Built_In_AABB 自算，可写；插入其他几何 noun（如 PYRA、SCTN）而不给框时，「要求」下整批拒写。既有测试中这类批次按需求 15 第 7 条只许显式改 `.with_spatial(Off)`，逐条记入 Spec_Ledger「测试适配清单」，与 8.7 同一提交落地，保证每个提交上既有测试不因默认值失败。

### 2.4 记录项补丁会把未改动元素的记录带进本批

7.4 坐实 E3D 在登记 / 重算条目的同一会话里把 Key0 写进元素记录的 SPAMAP 属性（类型 14）。Recompute_Set 里因祖先改动而重算、但本批没写过的后代，其条目变了时记录也要重写：plan 产出的补丁在记录分页之前并入写集，这些记录随之进入索引、claim 等侧表的更新。条目不变时（Key0 相同）不补，避免无谓改写。

### 2.5 祖先的重算属性要覆盖非几何 noun

修订后的 Recompute_Set ⑤ 按「任一重算属性」看祖先（EQUI、SUBE、ZONE、SITE、STRU、FRMW 等），而 `rules::RECOMPUTING` 只覆盖 Geometric_Noun 模板。8.4 把静态表扩到所有可能成为祖先的 noun 模板（由 `attlib.dat` 的 DCHC 与 WNOEVT 重推，测试逐属性核对），表外属性按 D7 视为触发（宁多勿少，盘上等价）。

### 2.6 合并与重铸

合并时 EndState 的底、旧表与表目录都取 Merge_Latest；`theirs_changed` 判调用方给值是否失效；重铸映射让条目按写入号编键；因 Merge_Latest 删了祖先而不可达的元素不写条目、列入 `unreachable`；`commit_observed` 遇 `Moved` 整个 stage 重跑一次。这是第 8 组里最复杂的一块，单列 E6，前面各步先在非合并路径上跑通。

## 3. 里程碑 E：步骤

| 步 | 任务 | 产出 | 验收 |
|---|---|---|---|
| E0 | 前置 | 复核 7.7（以 E1 的父提交重跑 SPAMAP Corpus 测试）；记下游搜索结果；Spec_Ledger「取证门槛核查」填父提交 | 门槛全过，否则停下 |
| E1 | 8.1、8.2 | `keyed_cow::rewrite_table::<V>`，`rewrite_keyed_table` 为 `V = 0` 薄包装；`#[doc(hidden)]` 不带规则的建树入口；`tests/write_stage8_spamap_pages.rs`（Property 2、10） | `V = 0` 既有单测与金样 0 失败；proptest 过 |
| E2 | 8.3 | `src/write/spamap/{mod,end_state}.rs`：`EndState`（`previous`、`in_new_index`、`members`、`reachable_from_worl`、`theirs_changed`），实现 `SpatialState` | 单测：叠加读、删除元素的成员、可达性 |
| E3 | 8.4、8.5 | `src/write/spamap/plan.rs`：Recompute_Set ①–⑦；`recompute_row` 扩到祖先 noun（2.5）；可用 World_AABB（给值 → Built_In_AABB → 无）；Codec；旧表全遍历校验；编辑集与净编辑判空；`MissingAabb` / `unused_aabb` / 拒写原因；模块内 proptest（Property 9 第一层） | proptest 对照 ①–⑦ 直译的参考实现 |
| E4 | 8.6 | staging 挂点：crate 私有入口带 `SpatialStage`；取得写锁并固定 latest 后、首次追加前完成 plan；SPAMAP 页在 claim 树之后、会话页之前；`table_overrides` / `table_additions`；建表；`TableDirectoryFull` → `SpatialError::DirectoryFull` | 拒写时文件逐字节不变（测试）；「关闭」金样不变 |
| E5 | 8.7、8.10 | `Transaction::with_spatial`、`set_aabb`（调用处校验 `InvalidAabb` / `NotGeometric`，取最后一次）、`StagedSession::spatial`、`TransactionCommit.spatial`、`TransactionError::Spatial`；`TransactionError`、`SpatialError` 加 `#[non_exhaustive]`；同一提交适配既有测试并记「测试适配清单」 | 每个提交上 Build_Checks 不新增失败 |
| E6 | 8.8 | 合并与重铸：Merge_Latest 为底、`theirs_changed`、重铸映射、`unreachable`、`Moved` 重跑 | 合并单测与 8.12 |
| E7 | 8.9 | SPAMAP_Record_Item 补丁（2.4）：plan 产出 `(refno, RecordItemPatch)`，记录分页前并入写集 | 记录项与 E3D 同形（7.4 的布局） |
| E8 | 8.11–8.17、7.6 的 6.17 | `tests/write_stage8_spamap_{model,merge,refuse,api,record_item,examples}.rs`、`write_stage8_off_compat.rs`（Property 9 第二层、11–18、26，需求 6.17 与 7.8 示例） | 全部 0 失败、未跳过；只用读侧读回（需求 2 第 5 条） |
| E9 | 8.18 | Build_Checks 相对 E1 后基线 702 / 0 / 30 与 clippy 63、doc 25 | 不新增失败、clippy / doc 不增 |

顺序：E0 → E1 → E2 → E3 → E4 → E5 → E6 / E7 → E8 → E9。E1–E4 期间写侧对外行为不变（新代码只在 crate 私有入口里），E5 起默认「要求」生效。

## 4. 不在 E 里

第 9 组 Write_CLI（`aabb` 与 `--spatial`）、第 10 组门（Q13 组、人工图形会话）、第 11 组发号下界、第 12 组 staging 规模、第 13 组页 1 桶表留到之后的里程碑。

## 5. 待你拍板

批准本计划 = 全按推荐项（A）。要改哪条，在对应行批注。

| Q | 问题 | 推荐（A） | 备选 |
|---|---|---|---|
| Q1 | 提交节奏 | 沿用：每步自动提交，结束统一回报；遇规格没写到的情形停下发卡 | B：每步回报，你回「提交」再提交 |
| Q2 | 可选测试（带 * 的 8.2、8.5、8.14–8.17） | 全做：写侧改动面大，属性测试是主要的安全网 | B：只做必做项加 8.14（拒写） |
| Q3 | 祖先重算属性表（2.5） | 静态表扩到全部 noun 模板中 DCHC 不为 0 且不带 WNOEVT 的属性，测试从 `attlib.dat` 重推核对 | B：写侧运行时读 `attlib.dat` 判定（要在 Transaction 里多带一份字典） |
| Q4 | 里程碑划分 | E 只含第 8 组（加 7.6 的 6.17）；第 9、10 组另起里程碑 F | B：E 一并做第 9 组 Write_CLI |
| Q5 | 记录项补丁（2.4） | 只在条目 Key0 变化时补，与 E3D 的「新旧值相同不重写」一致 | B：Recompute_Set 中凡有条目的元素都补 |

## 6. 执行记录

### E0（2026-10-03 22:5x）

E1 的父提交 `474ea14`（自 D6 的 `d8fecb6` 起只改文档）上重跑 SPAMAP 相关的 9 个测试二进制（`spamap_reader_real`、`spamap_codec_real`、`spamap_aabb_real`、`spamap_coverage_real`、`spamap_record_item_real`、`spamap_recompute_real`、`spamap_page_headers_real`、`spamap_census_real`、`core_rules_registry`），0 失败、无 skip；7.1–7.5 证据与读侧解码见 Spec_Ledger「取证门槛核查」。下游搜索见 §1。门槛满足。

### E1（2026-10-03 23:0x，8.1、8.2）

`src/write/keyed_cow.rs` 按叶值宽 `V` 泛化：`Slot<V>`、`Node<V>`、`decode_table_page::<V>`（叶值宽须为 `V`）、`write_table_page::<V>`（每页头字 4 写 `V`，叶槽键后跟值）、`rewrite_leaf` 带值（重复插入留旧值 529、删不存在的键无事 534）。新入口 `rewrite_table::<V>(root, name_hash, new_page_flags, page_words, edits, read_page, alloc, page_limit)` 与 `TableEdit<V>` / `TableEditKind<V>`；凭空新建的页（新表首叶、分裂长出的根）头字 5 取 `new_page_flags`（SPAMAP 为 1），复制的页沿用原值。`rewrite_keyed_table` 成为 `V = 0`、`new_page_flags = 0` 的薄包装。根分裂改为循环长高：原实现只在分裂的根上加一层，新根放不下全部子页时会在写页时下溢；正常情形（一层放得下）字节不变。`#[doc(hidden)] build_table::<V>` 是不带规则的建树入口。单测加两条（7 字值表的插删与多层分裂、每页头字 4 = 7 / 5 = 1 与新叶的 free 字）；`tests/write_stage8_spamap_pages.rs`：Property 2（随机条目与页长建树后由 SPAMAP_Reader 读回相同）、Property 10（编辑集合应用一次与两次相同，且等于按 534 / 529 直译的 `BTreeMap` 模型）、Corpus 四张表读出重编读回相同（`ams7999_0001` 23 891 条重编为 539 页）。Build_Checks：115 个测试二进制 + Doc-tests 702 / 0 / 30（`V = 0` 的金样比对都过，字节不变），skip 仍 7 个；clippy 63、`cargo doc` 25。

### E2–E9（2026-10-04，8.3–8.18、7.6 的 6.17）

在 E1 稳定基线上修正未提交草稿。先隔离空间入口并恢复既有 staging、改名 recompute_set、CLI 显式 Off，消除起点的五个失败；独立参考模型通过后才接入公开行为。最终将 E2–E9 合为一个实现提交，以满足默认 Require 和旧测试 Off 适配（8.7、8.10）必须同提交的约束；E1 保留。

- E2：精确上一会话 Snapshot 叠加定稿记录，复用描述符、默认值和 hunk 解码；尚未落盘载荷优先；解码失败拒写。真实成员次序与删除后代枚举分开，按最终 move 调整；owner 链检测环与无效祖先。
- E3：结束状态求 ①–⑦ 并记录原因；属性改回原值不触发。祖先 DCHC/WNOEVT 静态表由 attlib 重推测试；表外按 D7。纯内存 64 例独立参考模型；仅非空重算集校验旧表，完整 11 字差集，先删再插以覆盖同键值变化，净空不分配 SPAMAP 页。
- E4/E7：取得写锁后固定 latest，定稿→空间计划→Key0-by-k 类型 14 补丁→全部侧表→页面生成→追加；补丁后代参与侧表；SPAMAP 页在其他表后、会话页前；目录满为空间错误，page 0 唯一提交点。
- E5：公开 with_spatial/set_aabb/报告及空间错误映射；默认 Require；非法给值保留此前合法值，合法取最后一次；给框不触发重算。公开原语签名不变，不维护 SPAMAP。旧测试只显式 Off，逐项见 Spec_Ledger；CLI 显式 Off，空间参数与 JSON 留到 F。
- E6：冲突、重铸、三方合并、空间和侧表共用 latest；Merge_Latest 为底，base→latest 单独判断给框失效，重铸保留占用者；删除祖先报告实际阻断祖先；Moved 最多一次重试且每次恢复原始输入，不积累补丁或报告。
- E8：库副本模型及合并各 16 例；API、拒写、记录项属性测试；读侧验证建表、同键变值、多次编辑等价、未改后代、IMAP 3、祖先删除、两次 Moved；6.17 坐标逐字；拒写与 rollback 校验完整字节（强于只核 SHA-256/长度）。保留三份回归种子，金样未重录。k≥1 的布局验证沿用 Corpus；分支/吊架的目录输入仍按已坐实规则拒写，未扩大几何求解范围。
- E9：最终 cargo test --no-fail-fast -- --nocapture：722 通过 / 0 失败 / 30 ignored，122 个测试二进制 + Doc-tests；新增 E 测试无 skip；既有 skip 7 个（8 条提示，成员覆盖测试缺新旧两份样本），逐项在 Ledger。Clippy 63、cargo doc 25，均与基线相同。17 条 Off 金样通过。

日志保存在 `.agents/tasks/write-path-phase-3-milestone-e/validation/`。任务 8.3–8.18 和 7.6 经本次验收后打勾。后续仍需 F 的完整空间 CLI 与真 E3D 门；性能、发号与页 1 桶表不在本次范围。
