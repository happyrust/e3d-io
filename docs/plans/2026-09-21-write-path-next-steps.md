# 计划：e3d-io 写路径收口与下一步（阶段 0–3 已落地之后）

状态：评审通过（2026-09-21，用户在面板上批「Q1–Q7 全按推荐项定」；Plannotator 门另行 Approve）→ **S2 已落地**（§7 2026-09-21 条：漂移红测试改沿链找、clippy 0 error、类型 15/17/18 普查——15 非 NAME 无声明、18 语料 0 设值、**17 在 24 个库里设了值且都有以属性名命名的表 → 新增 S4b**）；S1 合入 `main` 待用户看完面板再开
日期：2026-09-21
分支 / worktree：`write-path` @ `D:\work\plant-code\old\vendor\e3d-io-write`，HEAD `eebe21f`（2026-09-21 12:24），自分叉点 `7a51c95` 起 30 个提交；`main` 领先它 2 个提交（`d51c3dd` / `4a76d2a`，池级元素缓存的上限与统计，只碰 `db_element.rs`）
依据：`docs/plans/2026-09-20-dabacon-write-plan.md`（下称「写计划」，其 §7 观察记录是进度的第一手账）、`docs/evidence/2026-09-20-core-dll-dabacon-write-path-algorithms.md`（「算法清单」）、`docs/evidence/2026-09-21-e3d-tty-gate-write-path.md`（「TTY 门」）
影响仓库：`vendor/e3d-io`（本仓库）。S3 只对 gen-model / e3d-model 及其 verify 变体做 `cargo check`，不改它们。

---

## 0. 一句话

写计划的阶段 0–3 全部落地，真 E3D 3.1 在 TTY 里读回了 e3d-io 写的每一样东西（14 问 PASS，10 s 一轮可重跑）。剩下的分三段：**A 收口**——合入 `main`、把文档从「read-only」改过来、修掉 1 条漂移红测试与 2 个 clippy error；**B 真项目第一天就会撞到的四件事**——非 ASCII 文本、按描述子设属性、refno 分配 + 事务门面、写模式哨兵锁，外加读侧对我们自己写出的 19/23 与 hunk 的解码；**C 留待**——SPAMAP / UDA 表 / extract 模型 / 多 extent 等，各写清触发条件，不排期。

---

## 1. 现状（2026-09-21 实测）

### 1.1 数字

| 项 | 数字 | 怎么得到的 |
|---|---|---|
| 提交 | `write-path` 30 个提交（09-20 起，最后一条 09-21 12:24）；`main` 另有 2 个未合入 | `git rev-list --left-right --count main...write-path` = `2 / 30` |
| 工作树 | 22 个文件显示 `M`，但 `git diff --ignore-cr-at-eol --numstat` = **0 行** | 全是 CRLF 假改动：索引 LF、工作树被某编辑器写成 CRLF（`git ls-files --eol` 见 `i/lf w/crlf`）；`.gitattributes` 已定 `* text=auto eol=lf` |
| 单测 | `cargo test --lib` **329 passed / 2 ignored / 0 failed** | 本轮实跑 |
| 写路径集成 | 9 个测试二进制 **36 条全绿**：stage1 6、claim 2、crosspage 2、elements 2、hunks 2、modify_delete 2、naming 3、tables 3、merge 14；合计 1.7 s | 本轮实跑（`ams8000_0001` 副本，会话 640 起） |
| 红 | **1 条**：`tests/session_tail_real.rs::a_merged_chain_bases_sessions_on_numbers_it_no_longer_retains` | 活样本 `ams7000_0001` 09-20 被 E3D 重写，最新会话是 `Saving Meta Data.` 无 base；`main` 上同红；与写路径无关 |
| clippy | `cargo clippy --all-targets`：**2 error**（`src/record/direction_spec.rs:277/278`，`0.7071` ≈ `FRAC_1_SQRT_2`，既有）+ 33 warning | 本轮实跑 |
| 外部门 | TTY 门 **14 问全 PASS**；E3D 在我们的会话 644 之上自己追了 645 / 647，e3d-io 再以 645 为 base 写 646，E3D 照读——两个写者的链互叠 | TTY 门文档 |

### 1.2 已落地（按 core.dll 例程对照）

| 能力 | 模块 | core.dll 例程（算法清单） | 验收 |
|---|---|---|---|
| 页写 + 会话页 + 页 0 单点提交；两段 `stage` / `commit`、`rollback`、提交前重读页 0 的 `Moved` 复检 | `write::{pages, session_page, mod}` | A4–A7 | `write_stage1_empty_session` 6 条 |
| 记录编码 / 解码：11 字头、word 10 存储格式字、成员块 / explicit 块、**跨页续接** | `write::record::RecordDraft`、`write::place_record` | A2 `sub_5AE9DF0` | `write_stage2_elements` / `_crosspage` |
| 类型 19 串列表 inline、hunk 20–23 外联 kind 3 块链 | `record::hunk` + `write::record` | `4SCEA` / `sub_5AEC110` / `sub_5AB4CF0` / `sub_5AEA500` | `write_stage2_hunks` 2 条 + TTY Q8c / Q8d |
| INDEX CoW：插 / 改 / 删、中位分裂、根增长；单写者直分真实页号 | `write::index_cow` | A2.2 / A5 | 全部写用例 |
| NAME 表 CoW：变长键、改名、首次命名、重名 = 561 拒写且不动一字节 | `write::named_cow`（读侧 `index::named`） | A3a.4 `sub_5B00F10` / `sub_5B010F0` | `write_stage2_naming` 3 条 |
| TYPEX + 类型 16 反向引用表 CoW：建表、目录追加、删元素撤键 | `write::keyed_cow`、`write::reverse` | `sub_5AB91B0` / `sub_5AFF6D0` / `sub_5AB3620` | `write_stage2_tables` 3 条 |
| CLAIM = 主元素会话索引 `(5,7618377)`：新建 / 刷号 / 撤销，非主元素冒到主元素祖先 | `write::{claim, claim_cow}`、读侧 `index::claim` | `sub_5B0C760` / `sub_5B059C0` | `write_stage2_claim` 2 条 |
| 三方合并：冲突扫描 561 / 562 / 590、属性级三方、成员级锚序（增 / 删 / 移）、`kind==2` 整段覆盖、extract flush 第 4 槽、st 1 加成员门、换 owner 到提交侧 owner 表摘成员、INDEX 去重 | `write::merge`、`record::three_way*`、`member_list_diff`、`merge_members*` | A3a / A3b：`sub_5B10410` `sub_5B0D950` `sub_5B1FF60` `sub_5B20CA0` `sub_5B211C0` `sub_5B1F660` `sub_5B07080` | `write_stage3_merge` 14 条 |
| 夹具与门 | `src/bin/e3d-write-box.rs`、`tools/e3d/write_gate.ps1` + `.mac.template` + `write_gate_mutate.mac` | — | TTY 门 14 问 |

写计划 §5 的八个决策点里，Q1–Q5、Q7、Q8 按默认值落地；**Q6 只做了一半**（见 §1.3 第 4 条）。

### 1.3 未落地（含代码里实测到的限制）

| # | 项 | 现状 | 证据 | 阻塞 / 触发 |
|---|---|---|---|---|
| 1 | **非 ASCII 文本** | 会话页 `USER` / `DESC`、explicit 文本、串列表、文本 hunk 一律 ASCII，否则 `WriteError::TextNotAscii` / `RecordEncodeError` | `session_page.rs:250`、`record.rs:1149` / `1184` | 语料 643 库里 451 个 `FORMAT` 1（UTF-8 原字节）、192 个 `FORMAT` 0（GBK 经 `&~ … &` 七位转义；本机 TES000 84 个）；读侧 `record::text` 两种都会解，**缺逆向编码器**。中文项目第一天就撞 |
| 2 | **按描述子设属性** | `RecordDraft` 只到 `set_real_wide(offset)` / `set_explicit_words(hash, type_code, words)` 这一层；wide 偏移、type_code、hash 全由调用方算；`e3d-write-box` 自己从模板 `defaults.implicit` 铺底再 `put_real_wide` | `write::record` 公开面 39 个 fn 里没有一个按属性名 / 描述子设值 | 读侧 `extract_parsed_element_with_descriptors` 的逆函数未写 |
| 3 | **refno 分配 + 事务门面** | 新 refno 由测试 / bin 手工取（`primary_extent_ref` 的 word0 + 该库最高 word1 + 1）；`NamedEdit` 要与记录里的 NAME 项手工配对；owner 成员表要调用方自己 `push_member`；写计划 §6 草图的 `Transaction` 未建 | `tests/common/write_fixture.rs`、`bin` | core 的分配器未读（候选：`MNUM`——CLAIM 普查里唯一「primary 却不在树里」的 noun，像个计数元素） |
| 4 | **写模式哨兵锁**（A9：`LockFileEx` 独占 1 字节 @ `0x7FFFFFF6FFFFFFFF`） | 未做，只有提交前重读页 0 的 `Moved` 复检——能挡「先提交完的写者」，挡不住「正在写的写者」 | `write/mod.rs:20` 模块头明说 | 写计划 Q6 默认值本是两样都做 |
| 5 | **读侧对 19 / 23 / 20–22 的描述子解码** | `AttrValue` / `ExplicitValue` 无变体：19 仍按 `Characters` 解（已知错，只因语料无设值的 19 才休眠）、hunk 目录项以 `RawWords` 到达 | `descriptor.rs` 注释、写计划 §7 hunk 条 | 现在我们**自己会写出**这些形态（TTY 门的 EQUI / FIXING），下游读回即错 |
| 6 | 类型 18 集合合并（`sub_5B0F1E0`）、类型 15 非 NAME 唯一表（DRTYPT / DRSTYT / RDFIRI）与 15 的抢占（`sub_5AFFC20`）、类型 17 索引表（MDSYSF / SCAREF / DESREF） | 未建模：18 当不透明字（写者赢）；15 只做 NAME 且撞名一律 561；17 不维护 | 算法清单 §5 类型对表 | **已普查（§7 2026-09-21 条）**：DESI / CATA 模板里 15 只有 NAME；18 只有 `UDASTB`、8.9 M 元素 0 设值 → 两者不做；17 有 21 个属性、语料 2 870 个元素设了值、设值的库**都带**以属性名命名的表 → 进 S4b |
| 7 | SPAMAP | 不维护 | 写计划 §7 调研条：7 字值 = 世界系 AABB + 标志、键含空间格 hash，要几何流水线算 | 有几何 AABB（e3d-model）+ 读出格号算法后另起计划 |
| 8 | UDA 表 UDATAB / UDAFTB / UUIDAR | 不维护 | `ams8000` 表目录无此三表；装机语料无 | 拿到带此表的项目库 |
| 9 | extract 模型（属性 17≠0 的 CLAIM 表搬运 A3a.7、`sub_5B13670`、OWNSES / 第 505 字语义）、页 1 桶表 A11、Central A8 | 不做 | 写计划 §0 | 出现多用户 / extract 需求 |
| 10 | 多 extent（ext≠1 → `MultiExtent` 拒写）、E3D 2.1 | 拒写 | 语料 940 694 条叶值 `flag` 全 1 = 全部单 extent | 出现多 extent 库 |
| 11 | E3D 自造 hunk 的样本（逐字比对） | 无 | E3D 只在 ≥ `0x2000` 字且 hunk 登记表 armed 时外联；普通模板属性 cnt 到不了 | 找到能让 E3D 造 hunk 的属性 / 宏再补 |
| 12 | 部分存盘（`sub_5B211C0` 第 ④ 步 `sub_5B1CC10`） | 无对应场景 | — | 不做 |

---

## 2. 切片

### A · 收口（先做，合计约 1 天）

#### S1 合入 `main` 与文档

- `write-path` 先 `git merge main`（2 个提交，只碰 `db_element.rs`，预期无冲突）→ 全套 `cargo test --no-fail-fast` → `main` fast-forward 到 `write-path`。worktree `e3d-io-write` 之后留作写路径工作区还是撤，见 Q1。
- 工作树 CRLF 假改动：`git checkout -- .`（索引已是 LF），完后 `git ls-files --eol` 全部 `w/lf`。
- 文档：README 的「Standalone read-only」「Not yet: writeback」改掉，加「Write path」一节——入口（`write_element_session[_with]` / `stage_element_session` / `write_merge_session[_with]`）、两段提交、`WriteOptions.templates` 给不给的差别、TTY 门怎么跑、三条调用方口径（模板 cnt 对文本是**字节**容量、E3D 字典 `DESC` 上限 120 字符、`UUIDIA` 一次性）；`Cargo.toml` description 去掉 read-only；MISSION 加「写回」一节；CHANGELOG `Unreleased` 里写路径 8 条归到一个「Write path」小标题下，是否切 `0.2.0` 见 Q2。
- **验收**：`main` 上 `cargo test` 与 `write-path` 同数（329 单测 + 87 个测试二进制）；README 里每条命令跑得通；`git status` 干净。

#### S2 测试与 lint 卫生

- `session_tail_real::a_merged_chain_bases_sessions_on_numbers_it_no_longer_retains`：不再假定「最新会话」就是合并链上的那个——沿链找第一个「`base_sesno` 不在保留链里」的会话来断言；找不到打 SKIP 并说明原因（Q6）。这条 09-20 起已经红过三次，每次 E3D 开一次样本项目就漂一次。
- `direction_spec.rs:277/278` 两个 clippy error 改用 `FRAC_1_SQRT_2`；33 个 warning 里落在写路径文件上的清零，其余既有的不动。
- 顺手做 §1.3 第 6 条的普查（一个 `%TEMP%` 探针，复用 `tools/probe_type_widths.py` 的模板遍历）：`desvir` / `catvir` 里类型 15（非 NAME）/ 17 / 18 的属性名单，以及 `D:\AVEVA\Projects\E3D3.1` 全部 DESI / CATA 库里设过值的元素数。结果记进 §7。
- **验收**：`cargo clippy --all-targets` 0 error；`cargo test --no-fail-fast` 0 红；§7 有普查数字。

#### S3 下游编译门

- 不改 gen-model：在 `old/gen-model`、`vendor/e3d-model` 与 5 个 verify 变体 worktree 里 `cargo check`（它们 path 指 `../vendor/e3d-io`，S1 合入后自动吃到）。读侧承诺「一行不改」——但 `record::page_of` 改了可见性、`session::at` / `meta::descriptor::at` 变 `pub(crate)`，这一步把承诺变成数字。
- **验收**：7 个 crate `cargo check` 通过；gen-model 现有走 e3d-io 的测试（`e3d_collector` 那族）抽 1 个跑通。

### B · 真项目第一天就会撞到的四件事（按价值排序）

#### S4 按描述子设属性（`extract_parsed_element_with_descriptors` 的逆）

- 新 `RecordDraft::set_attribute(&NounTemplate, hash, &AttrValue)`（或独立 `write::attr` 模块）：按描述子 `encoded_location & 0xFFFFF` 判 implicit 槽还是 explicit 项；类型 2 / 6 real（wide / compact 按模板 flag bit 29，**先只做 wide、compact 报错**——`ams8000` 6 639 个元素全 wide，compact 无语料可验）、3 / 7 int 与 word、4 / 8 ref、5 位、9 bool 位集、10 文本、16 ref（并入现有 `reverse::set_reference`）、19 串列表；`unset` = implicit 写 `UNSET_SENTINELS`（`0x80000000` / `1`）、explicit 删项；元素数 > 描述子 `logical_count` → 错；是否外联成 hunk 仍由调用方显式选（core 自身按运行态定，写计划 §7 hunk 条）。
- **验收（最硬的一条）：往返恒等普查**——`ams8000_0001` 6 639 个元素逐个 `from_record_bytes` → 对每个已设属性 `set_attribute(读侧解出的值)` → `content_eq` 与原记录相等，**0 例外**（读侧的解码就是 oracle）；再对 BOX / ZONE / EQUI 各用 `set_attribute` 写一个新元素，TTY 门 `Q ATT` 全列无错、值逐字相等。

#### S4b 类型 17 反向引用表（半天；S2 普查后新增）

- 类型 17 = ref 数组 + 以属性 hash 命名的索引表（算法清单 §5：写 `sub_5AB91B0` case 17、删元素 `sub_5AB3620` 对 16 / 17 同样「每对非零 ref 出 `[t, 自身]` 键」）。今天 `write::reverse::table_keys` 只从 TYPEX 与类型 16 出键，写一个带 HCREF / TCREF / MDSYSF / CONNRE 的元素不会进表、删它不会撤键。
- 落地：`table_keys` 对模板里每个类型 17 属性（implicit 槽者按 wide 偏移读 `[count][ref×count]`，explicit 者按 hash 取数组）把每个非空 ref 出一键 `[t.0, t.1, 自身.0, 自身.1]`，其余（建表、目录追加、差集删插）复用 `keyed_cow`；`set_reference` 旁加 `set_reference_array` / `reference_array`。
- **验收**：① 语料不变量普查（`%TEMP%` 探针，同类型 16 那次的口径）：`ams251142_0001`（HCREF 1 151 / TCREF 857 设值元素）、`ams251085_0001`（MDSYSF）、`aps250179_0001`（ROUTND）里每个设了值的类型 17 元素、其每个非空 ref 在同名表里都有 `[t, 自身]` 边，反向抽 300 边都映回一个值——0 例外才算口径坐实；② 集成用例：`ams251142_0001` 副本上给一个 CWBRAN 改 HCREF（删旧插新）、再删一个带 TCREF 的元素 → 表随之变、其它表连根页号都不动（`write_stage2_tables` 的口径）；③ 3 个「设了 CONNRE 却无表」的库（§7）在普查里核清是不是全空引用。

#### S5 非 ASCII 文本

- `record::text::decode_as` 的逆：`FORMAT` 1 → UTF-8 原字节 4 字符 / 字打包；`FORMAT` ≠ 1 → 项目字符集（GBK）+ `&~ … &` 七位转义。先从 core 读实写侧转换（`sub_5AD5060` 里按属性 18 的 UTF-8 开关：直接 `sub_54719F0` 打包，或 `sub_5B8F3C0` + `sub_54B2CE8` 转码后 `sub_5471E50` 打包；转码失败 = 错 23 / 638），再落地。会话页 `USER` / `DESC`、explicit 文本（10 / 15）、串列表（19 / 23 每项的 `bytes` 字 = **转换后**字节数）、文本 hunk 22 全走同一编码器；长度按**字节**对模板 cnt 校验（`DESC` 480 字节 = 120 字符 × 4 字节 UTF-8 上限）。
- **验收**：`FORMAT` 1（`ams8000` 副本）写中文 `DESC` 与中文 NAME，读回逐字相等、`find_by_name` 命中；TTY 门 `Q DESC` 打出中文（日志编码先核一次）。`FORMAT` 0 取本机一个 TES000 库副本，写 `反应堆厂房`，盘上字节 = 转义形态，`e3d-descriptor extract` 解回原文。

#### S6 事务门面 + NAME 自动配对 + refno 分配

- `write::Transaction::open(path, WriteOptions)` → `insert(owner, noun, |draft| …)` / `update(refno, |draft| …)` / `delete(refno)` / `rename(refno, name)` / `stage()` → `StagedSession`。NAME 项改动自动生成 `NamedEdit`（今天两处手配，漏一处就是表与记录不一致）；owner 成员表的增删由门面代做；`Insert` 自动 `ensure_typex`。
- refno 分配：先花半天读 core（Q4）——元素创建路径里新 refno 从哪来（候选 `MNUM`）；读不出就用现规则（`primary_extent_ref` word0 + max word1 + 1），会话内连续分配，提交前对 latest 撞号已是 590。
- **验收**：`e3d-write-box` 改用门面重写，同一时钟下写出的 4 个会话与旧版**逐字节相同**（旧版 bin 保留到比对完）；TTY 门照旧 14 PASS。

#### S7 写模式哨兵锁（A9）

- Windows：staging 起对 `0x7FFFFFF6FFFFFFFF` 处 1 字节 `LockFileEx` 独占，`commit` / `rollback` 后 `UnlockFileEx` + `FlushFileBuffers`；被占 → 新错 `WriteError::Locked` **立即返回**（不等待，等待策略是调用方的事）；非 Windows 维持 `Moved` 复检。FFI 手写两个 `extern "system"` 声明，不加 crate（Q5）。
- **验收**：单测两把句柄互斥；TTY 门加第 15 问——E3D 以写模式开着副本时 e3d-io 写 → `Locked` 且文件字节不变，E3D 退出后同一写成功（这一问同时坐实「我们锁的和 E3D 锁的是同一字节」）。

#### S8 读侧跟进 19 / 23 / 20–22

- `extract_parsed_element_with_descriptors`：类型 19 / 23 → `AttrValue::TextList(Vec<String>)`（Q3）；20 / 21 / 22 目录项 → 经 `record::hunk::read_hunk` 解成 `IntArray` / `RealArray` / `Text`，状态 `Decoded`；`e3d-descriptor extract` 输出随之。
- **验收**：对 TTY 门写出的副本 `extract` EQUI / FIXING，`DESC` / `DESP` / `UUIDIA` / `ROKEYS` / `ROVALS` 五行 `Decoded` 且值与 TTY 日志逐字相等；`tests/write_stage2_hunks.rs` 加同样断言。

### C · 留待（不排期，只写触发）

| 项 | 触发条件 |
|---|---|
| SPAMAP | 有几何 AABB（e3d-model 已算世界系几何）**且**从 core 读出空间格 hash 的算法 → 另起计划 |
| UDA 表 UDATAB / UDAFTB / UUIDAR | 拿到一个带这三张表的项目库 |
| 类型 15 非 NAME | **不做**：DESI / CATA 模板里没有一个（DRTYPT / DRSTYT / RDFIRI 是别的库类型的） |
| 类型 18 `UDASTB` 集合合并 | **不做**：DESI 868 / CATA 146 个 noun 声明，语料 2.09 M + 6.85 M 元素 **0** 设值；触发 = 语料里出现设了值的 `UDASTB` |
| 类型 17 索引表 | 已进 S4b |
| extract 模型、页 1 桶表、Central、多 extent、E3D 2.1、部分存盘 | 需求出现再议 |
| E3D 自造 hunk 样本 | 不专门做 |

---

## 3. 顺序与依赖

S1 → S2 → S3（收口，1 天）→ **S4**（2–3 天）→ S4b（半天）→ **S5**（1–2 天，含 core 读实）→ **S6**（1–2 天）→ S8（半天）→ S7（半天）。

- S4 是 S6 的前置（门面靠它设值）；S4b 与 S4 互不依赖（只动 `reverse`），可先做；S5 与 S4 互不依赖，可并行；S7 / S8 独立。
- 估时按写计划 §7 的实际节奏（09-20 一天落地阶段 2 四个用例 + NAME 读侧）给，是上限。

---

## 4. 风险

| 风险 | 影响 | 处置 |
|---|---|---|
| S1 合入后 7 个下游 crate 一起吃到 `write` 与 `record::hunk` | 读侧 API 未改，但可见性有变 | S3 `cargo check` 兜；出问题只可能是编译错，不是静默错值 |
| S4 的 compact 布局（flag bit 29 清零、f32）无语料 | 写 compact 元素会错 | 先只支持 wide、compact 返回错误；有 compact 语料再开 |
| S5 的 GBK 转义规则读侧是从语料归纳的 | core 的 `sub_54B2CE8` 可能有读侧没见过的分支（不可映射字符） | 写侧以 core 反编译为准；不可映射 → 报错不写 |
| S6 若读不出 core 的 refno 分配器 | max+1 在多写者下可能撞号 | 590 已兜底，代价是用户重试；记 §7 |
| S7 锁偏移超过文件长度 | Windows 允许锁文件末尾之外的区间，但要坐实与 E3D 是同一字节 | TTY 门第 15 问 |
| 活样本继续漂移（E3D 每开一次样本项目就写） | 依赖样本状态的断言反复红 | S2 把断言改成沿链找；新测试一律沿链找或在副本上构造 |

---

## 5. 决策（2026-09-21 评审：Q1–Q7 全部按推荐项定）

- **Q1 合入方式——已定**：把 `write-path` 合入 `main`，worktree 保留为写路径工作区；**时点由用户定**（看完面板再开 S1）。
- **Q2 版本——已定**：切 `0.2.0`，CHANGELOG `Unreleased` 归档为该版。
- **Q3——已定**：`AttrValue` 加 `TextList(Vec<String>)` 变体，靠编译器点出下游穷举 match。
- **Q4 refno 分配——已定**：先花半天读 core 的分配器，读不出再落 max+1。
- **Q5 锁的 FFI——已定**：手写两个 `extern "system"` 声明，不加 crate。
- **Q6 `session_tail_real` 漂移——已定并落地（S2）**：沿链找「base 被折掉」的那个会话，找不到 SKIP。
- **Q7 第一个真实写场景——已定**：**改已有元素属性 + 新建元素挂进 owner**（TTY 门已覆盖的两种）；S4 先做这两种路径用到的类型，S6 门面先开 `update` / `insert`。

---

## 6. 不做

见 §2 C。另：不在本计划里改 gen-model（S3 只 check）；不做性能基准——写路径每次 CoW 只复制根到叶的路径页，`ams8000` 上 36 条用例合计 1.7 s，等真场景给出元素数与库大小再定。

---

## 7. 观察记录（随切片推进填写）

- 2026-09-21：计划起草；§1 数字全部来自当日实跑。
- 2026-09-21：**评审通过，S2 落地。**
  - `tests/session_tail_real.rs::a_merged_chain_bases_sessions_on_numbers_it_no_longer_retains` 改成沿链找第一对 `Stacking::OverAGap`（`BASESN` 比后面保留的那个会话新）的相邻会话，断言它「坐在紧邻前驱上、而那个前驱已不在链里」；找不到打 SKIP 并列出保留链与 stacking 普查。实跑：`ams7000_0001` 今天保留 16 个会话（`274..262, 238, 237, 1`），命中 **262 基于被折掉的 261**，通过；原写法假定「最新会话」是那一个，E3D 每开一次样本项目就在链顶追一个 `Saving Meta Data.`（无 base），所以 09-20 起红了三次。
  - clippy：`direction_spec.rs:277/278` 的 `0.7071` 改 `FRAC_1_SQRT_2`（**0 error**）；写路径文件的 6 个 warning 清零——`index/claim.rs` 两处 `% != 0` → `is_multiple_of`、`write/record.rs` 三处 `chunks_exact(4)` 收成一个 `stream_words`（`as_chunks::<4>`，余数照旧丢）、一处 doc 里以 `+ 3)` 起行被当成列表项的换行。其余 65 个 warning 全在既有读侧 / examples / 老测试里，未动。全套：`cargo test --lib` 329 通过、写路径 36 条 + `session_tail_real` 4 条全绿。
  - **类型 15 / 17 / 18 普查**（`%TEMP%\e3dio_census`，未入库；`TemplateFile` 枚举 `desvir` / `catvir` 全部 noun → 挑类型 15（非 NAME）/ 17 / 18 的描述子 → `scan_elements(Full)` 扫 `D:\AVEVA\Projects\E3D3.1` 下全部 < 200 MB 主文件：**DESI 507 库 2 157 179 元素、CATA 102 库 6 845 638 元素**，11 个按大小跳过、0 个打不开、0 个元素读错，19 s）。「设了值」= implicit 槽（wide 偏移）里有非 0 非 `0x80000000/1` 的字，或 explicit 流里有该 hash 的项。
    - **类型 15 非 NAME：DESI / CATA 模板里一个都没有**（DRTYPT / DRSTYT / RDFIRI 是别的库类型）。→ 不做。
    - **类型 18：只有 `UDASTB`**（cnt 1000、只能 explicit），DESI 868 / CATA 146 个 noun 声明，**8.9 M 元素里 0 个设值**。→ 不做，记触发。
    - **类型 17：DESI 20 个 + CATA 1 个属性**，全部 explicit-only，除 `CONNRE`（HNODE 有 implicit 槽）与 `DESLNK`（CYMLNK 有）。设了值的：`HCREF` **1 151** 元素 / 24 库、`TCREF` 857 / 22（CTWALL / CWBRAN / HANDRA / RBRAN）、`MDSYSF` 291 / 24（29 个 noun：AHU / BATT / BRAN / CABLE …）、`CONNRE` 165 / 19、`DESLNK` 116 / 2、`STRREF` 6 / 1、`ROUTND` 2 / 2；CATA `COCORE` 282 / 3（`acp7046_0001`）；其余 13 个属性 0 设值。**设了值的库里，表目录都有以该属性名命名的表**（HCREF 24/24、TCREF 22/22、MDSYSF 24/24、COCORE 3/3、DESLNK 2/2、STRREF 1/1、ROUTND 2/2；`CONNRE` 16/19——无表的 3 个库 `ams251158` / `aps250176` / `aps250520` 只在 RNODE 上各有 1 / 4 / 4 个 explicit 项，疑为全空引用、无键即无表，S4b 普查时核）。例：`ams251142_0001` 表目录 = `NAME TYPEX TCREF SPAMAP POSREF SPRE HCREF CONNRE`。→ 与算法清单 §5「17 = ref 数组 + 以属性 hash 命名的索引表」互证，**新增 S4b**。
