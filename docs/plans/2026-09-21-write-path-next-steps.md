# 计划：e3d-io 写路径收口与下一步（阶段 0–3 已落地之后）

状态：评审通过（2026-09-21，用户在面板上批「Q1–Q7 全按推荐项定」；Plannotator 门另行 Approve）→ **S2 已落地**（§7 2026-09-21 条：漂移红测试改沿链找、clippy 0 error、类型 15/17/18 普查——15 非 NAME 无声明、18 语料 0 设值、**17 在 24 个库里设了值且都有以属性名命名的表 → 新增 S4b**）→ **S4b 已落地**（类型 17 反向表随写维护；语料 3 191 条边正反全对上）→ **S4 已落地**（`RecordDraft::set_attribute` 按描述子设值；三库 7 825 元素 149 558 行往返恒等 0 例外；顺带找到 E3D 自己写的 hunk 样本、块头字字相同）→ **S1 / S3 完**（`main` @ `a6c0a2a` = 0.2.0 + 写路径 + 别的会话的 element_name WIP；下游 7 个 e3d-model crate `cargo check` 0 error，gen-model 仅剩一个与 e3d-io 无关的 e3d-model 接口漂移）→ **S5 已落地**（2026-09-22，§7：`record::text::encode_as` 照 core `sub_54B2CE8` 落地——6 = GBK 表、两字节都 ≥ `0xA0` 才进 `&~ … &`、否则拒（638）；explicit 文本 / 串列表 / hunk / 会话页 / NAME 键全走它；`ams5100` 57 条中文往返 0 差异、E3D 自写的键 `/&~9xB/5:Ih18 &` 逐字节相同、TTY 门 16 问全 PASS 含中文 NAME / DESC / 注释）→ **2026-09-22 进度审核**（§7 当日条：`main` = `write-path` = `5dfd61a`，单测 347 / 写路径 40 条全绿 / clippy 0 error；S1–S5 + S4b 全部落地；剩 S6 / S7 / S8）。**用户拍板余下顺序改为 B：S8 → S6 → S7**（理由见 §3）→ **S8 已落地**（2026-09-22，§7：`ExplicitValue` / `DescriptorValue` 加 `TextList`、类型 19 inline 与槽都按串列表解；`record::hunk::resolve_hunks` + `ReadOnlyEngine::resolve_hunks` 把 20–23 目录项就着页读成值，`extract_element_with_descriptors` 与 `DbElement` 门面先解 hunk 再按描述子解；Bool 门——`data_type 3` 且字 ∉ {0, 1} → `Word`；`ams8000` `DBMVAL` 解出 5 709 int、TTY 门副本 EQUI / FIXING 五行全 `decoded_explicit` 且与 E3D 打出的值一致、stage4 普查 `bool_over_a_word` 185 → 0 并改成硬断言；全套 93 个测试二进制 0 失败、TTY 门 16 问 PASS；下游 gen-model 4 处穷举 match 待补 `TextList` 臂，见 §7）。下一步 S6。
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
| 1 | **非 ASCII 文本** | **已关（S5，§7 2026-09-22 条）**：`record::text::encode_as` 按头 `FORMAT` 编码，会话页 / explicit 文本 / 串列表 / hunk / NAME 键全走它；无转义形态的字符（GBK 码有字节 < `0xA0`、BMP 外、PUA）拒且不写，与 core 的 638 同口径 | 起草时：`session_page.rs:250`、`record.rs:1149` / `1184` 一律 ASCII | 语料 643 库里 451 个 `FORMAT` 1（UTF-8 原字节）、192 个 `FORMAT` 0（GBK 经 `&~ … &` 七位转义；本机 TES000 84 个） |
| 2 | **按描述子设属性** | `RecordDraft` 只到 `set_real_wide(offset)` / `set_explicit_words(hash, type_code, words)` 这一层；wide 偏移、type_code、hash 全由调用方算；`e3d-write-box` 自己从模板 `defaults.implicit` 铺底再 `put_real_wide` | `write::record` 公开面 39 个 fn 里没有一个按属性名 / 描述子设值 | 读侧 `extract_parsed_element_with_descriptors` 的逆函数未写 |
| 3 | **refno 分配 + 事务门面** | 新 refno 由测试 / bin 手工取（`primary_extent_ref` 的 word0 + 该库最高 word1 + 1）；`NamedEdit` 要与记录里的 NAME 项手工配对；owner 成员表要调用方自己 `push_member`；写计划 §6 草图的 `Transaction` 未建 | `tests/common/write_fixture.rs`、`bin` | **分配器已读（S6 前半，`docs/evidence/2026-09-22-core-dll-refno-allocation.md`）**：创建路径 `DB_Element::createAt → ELCREL → ELCREA → BCREAT → routine 597`；号在建块路径铸出、不由上层传入。**页 1（桶表，页类型 1）为每个会长的扩展存 `(word0, 下一个 word1)` 计数器，严格 > 该扩展活最大（删号不回收）**；主扩展只放 WORLD+MNUM 不长、纯字典库不长 → 无计数器。`MNUM` 不是计数器（它的 MARK/MAPCSV 是 noun→数量映射）。忠实做法 = 读页 1 计数器发号 + 页 1 CoW（= A11，记 S6b）；本轮按 max word1 + 1 + 590 兜底 |
| 4 | **写模式哨兵锁**（A9：`LockFileEx` 独占 1 字节 @ `0x7FFFFFF6FFFFFFFF`） | 未做，只有提交前重读页 0 的 `Moved` 复检——能挡「先提交完的写者」，挡不住「正在写的写者」 | `write/mod.rs:20` 模块头明说 | 写计划 Q6 默认值本是两样都做 |
| 5 | **读侧对 19 / 23 / 20–22 的描述子解码** | **已关（S8，§7 2026-09-22 条）**：`ExplicitValue::TextList` / `DescriptorValue::TextList`，19 inline 与槽按串列表解；`resolve_hunks` 把 20–23 目录项就着页读成值，引擎与 `DbElement` 门面自动做；`data_type 3` 且字 ∉ {0, 1} → `Word`（185 例归零）。纯函数 `extract_parsed_element_with_descriptors` 无页，目录项照旧 `RawWords`（写侧原样重发靠它） | 起草时：`AttrValue` / `ExplicitValue` 无变体，19 按 `Characters` 解、目录项 `RawWords`、`UDA.DTYP` 报 `Bool(true)` | 下游 gen-model 4 处穷举 `match` 待补 `TextList` 臂（§7） |
| 6 | 类型 18 集合合并（`sub_5B0F1E0`）、类型 15 非 NAME 唯一表（DRTYPT / DRSTYT / RDFIRI）与 15 的抢占（`sub_5AFFC20`）、类型 17 索引表（MDSYSF / SCAREF / DESREF） | 未建模：18 当不透明字（写者赢）；15 只做 NAME 且撞名一律 561；17 不维护 | 算法清单 §5 类型对表 | **已普查（§7 2026-09-21 条）**：DESI / CATA 模板里 15 只有 NAME；18 只有 `UDASTB`、8.9 M 元素 0 设值 → 两者不做；17 有 21 个属性、语料 2 870 个元素设了值、设值的库**都带**以属性名命名的表 → 进 S4b |
| 7 | SPAMAP | 不维护 | 写计划 §7 调研条：7 字值 = 世界系 AABB + 标志、键含空间格 hash，要几何流水线算 | 有几何 AABB（e3d-model）+ 读出格号算法后另起计划 |
| 8 | UDA 表 UDATAB / UDAFTB / UUIDAR | 不维护 | `ams8000` 表目录无此三表；装机语料无 | 拿到带此表的项目库 |
| 9 | extract 模型（属性 17≠0 的 CLAIM 表搬运 A3a.7、`sub_5B13670`、OWNSES / 第 505 字语义）、页 1 桶表 A11、Central A8 | 不做 | 写计划 §0 | 出现多用户 / extract 需求 |
| 10 | 多 extent（ext≠1 → `MultiExtent` 拒写）、E3D 2.1 | 拒写 | 语料 940 694 条叶值 `flag` 全 1 = 全部单 extent | 出现多 extent 库 |
| 11 | E3D 自造 hunk 的样本（逐字比对） | **已关（§7 S4 条）**：`ams8000_0001` 的 `DBMDAT 24384/26333` `DBMVAL` 是 E3D 写的类型 20 hunk（5 709 字，跨页链），目录项 / 块头 / 续接与本 crate 写法字字相同 | 起草时以为没有，只查过模板类型 19 | — |
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

#### S5 非 ASCII 文本（**已落地 2026-09-22**，§7；core 读实见 `docs/evidence/2026-09-22-core-dll-text-charset-write.md`）

- `record::text::decode_as` 的逆：`FORMAT` 1 → UTF-8 原字节 4 字符 / 字打包；`FORMAT` ≠ 1 → 项目字符集（GBK）+ `&~ … &` 七位转义。先从 core 读实写侧转换（`sub_5AD5060` 里按属性 18 的 UTF-8 开关：直接 `sub_54719F0` 打包，或 `sub_5B8F3C0` + `sub_54B2CE8` 转码后 `sub_5471E50` 打包；转码失败 = 错 23 / 638），再落地。会话页 `USER` / `DESC`、explicit 文本（10 / 15）、串列表（19 / 23 每项的 `bytes` 字 = **转换后**字节数）、文本 hunk 22 全走同一编码器；长度按**字节**对模板 cnt 校验（`DESC` 480 字节 = 120 字符 × 4 字节 UTF-8 上限）。
- **验收**：`FORMAT` 1（`ams8000` 副本）写中文 `DESC` 与中文 NAME，读回逐字相等、`find_by_name` 命中；TTY 门 `Q DESC` 打出中文（日志编码先核一次）。`FORMAT` 0 取本机一个 TES000 库副本，写 `反应堆厂房`，盘上字节 = 转义形态，`e3d-descriptor extract` 解回原文。

#### S6 事务门面 + NAME 自动配对 + refno 分配

- `write::Transaction::open(path, WriteOptions)` → `insert(owner, noun, |draft| …)` / `update(refno, |draft| …)` / `delete(refno)` / `rename(refno, name)` / `stage()` → `StagedSession`。NAME 项改动自动生成 `NamedEdit`（今天两处手配，漏一处就是表与记录不一致）；owner 成员表的增删由门面代做；`Insert` 自动 `ensure_typex`。
- refno 分配：先花半天读 core（Q4）——元素创建路径里新 refno 从哪来（候选 `MNUM`）；读不出就用现规则（`primary_extent_ref` word0 + max word1 + 1），会话内连续分配，提交前对 latest 撞号已是 590。**已读（S6 前半，2026-09-22 §7 与 `docs/evidence/2026-09-22-core-dll-refno-allocation.md`）：号在建块路径（`createAt→ELCREL→ELCREA→BCREAT→routine 597`）铸出，页 1 桶表为会长的扩展各存一个「下一个 word1」高水位计数器（删号不回收）。忠实做法要页 1 CoW（A11，记 S6b）；本轮按「owner 扩展 word0 + 该扩展 max word1 + 1」发号（会话内连续），590 兜底。`MNUM` 不是计数器。**
- **验收**：`e3d-write-box` 改用门面重写，同一时钟下写出的 4 个会话与旧版**逐字节相同**（旧版 bin 保留到比对完）；TTY 门照旧 14 PASS。

#### S7 写模式哨兵锁（A9）

- Windows：staging 起对 `0x7FFFFFF6FFFFFFFF` 处 1 字节 `LockFileEx` 独占，`commit` / `rollback` 后 `UnlockFileEx` + `FlushFileBuffers`；被占 → 新错 `WriteError::Locked` **立即返回**（不等待，等待策略是调用方的事）；非 Windows 维持 `Moved` 复检。FFI 手写两个 `extern "system"` 声明，不加 crate（Q5）。
- **验收**：单测两把句柄互斥；TTY 门加第 15 问——E3D 以写模式开着副本时 e3d-io 写 → `Locked` 且文件字节不变，E3D 退出后同一写成功（这一问同时坐实「我们锁的和 E3D 锁的是同一字节」）。

#### S8 读侧跟进 19 / 23 / 20–22，以及 Bool 压掉 word（**已落地 2026-09-22**，§7）

- `extract_parsed_element_with_descriptors`：类型 19 / 23 → `AttrValue::TextList(Vec<String>)`（Q3）；20 / 21 / 22 目录项 → 经 `record::hunk::read_hunk` 解成 `IntArray` / `RealArray` / `Text`，状态 `Decoded`；`e3d-descriptor extract` 输出随之。（落地时的两处口径修正：变体加在 `ExplicitValue` / `DescriptorValue` 上——`AttrValue` 是不带字符串的旧摘要枚举，`Text` 在它那里是个 word，串列表映成 `None`；hunk 的解析放在**有页的一层**——纯函数没有页，所以是 `ReadOnlyEngine::extract_element_with_descriptors` / `DbElement` 先 `resolve_hunks` 再解，状态是 explicit 值一贯的 `DecodedExplicit`。）
- 一字 word 的 `Bool` 判定加一道门：attlib 说 `data_type 3` 但字不是 0 / 1 → 报 `Word { raw }` 而不是 `Bool(true)`（S4 普查：`ams5100` 的 185 个 `UDA.DTYP` 盘上是 `TEXT` / `REAL` / `INTE` 的 WORD，E3D 打 `Dtype TEXT`）。要先查 `uda_catalog` 等读 `DTYP` 的地方有没有依赖现在的 `Bool`。
- **验收**：对 TTY 门写出的副本 `extract` EQUI / FIXING，`DESC` / `DESP` / `UUIDIA` / `ROKEYS` / `ROVALS` 五行 `Decoded` 且值与 TTY 日志逐字相等；`ams8000` 的 `DBMDAT.DBMVAL` 解出 5 709 个 int；`tests/write_stage2_hunks.rs` 加同样断言；S4 的往返普查里 `ams5100` 的 185 个「Bool 压 word」归零。

### C · 留待（不排期，只写触发）

| 项 | 触发条件 |
|---|---|
| SPAMAP | 有几何 AABB（e3d-model 已算世界系几何）**且**从 core 读出空间格 hash 的算法 → 另起计划 |
| UDA 表 UDATAB / UDAFTB / UUIDAR | 拿到一个带这三张表的项目库 |
| 类型 15 非 NAME | **不做**：DESI / CATA 模板里没有一个（DRTYPT / DRSTYT / RDFIRI 是别的库类型的） |
| 类型 18 `UDASTB` 集合合并 | **不做**：DESI 868 / CATA 146 个 noun 声明，语料 2.09 M + 6.85 M 元素 **0** 设值；触发 = 语料里出现设了值的 `UDASTB` |
| 类型 17 索引表 | **已落地（S4b，§7）**。留待：A3b 对数组属性逐槽（元素 k）三方与 17 写入撞 23 → 576 的冲突——触发 = 出现两边并发改同一数组的场景 |
| **S6b 页 1 桶表 CoW（refno 计数器随写维护）** | 需要与 E3D 交替写同一扩展且号段不容空档，或要把「删除号不回收」对齐到字节；届时把 routine 597 展到字节级、页 1 随会话 CoW 写回（现按 max+1，`docs/evidence/2026-09-22-core-dll-refno-allocation.md`） |
| extract 模型、Central、多 extent、E3D 2.1、部分存盘 | 需求出现再议 |
| E3D 自造 hunk 样本 | **已有**：S4 普查撞到 `ams8000` `DBMDAT.DBMVAL`（类型 20，5 709 字，跨页），块头 / 目录项 / 续接与本 crate 字字相同（§7） |

---

## 3. 顺序与依赖

S1 → S2 → S3（收口，1 天）→ **S4**（2–3 天）→ S4b（半天）→ **S5**（1–2 天，含 core 读实）→ ~~**S6**（1–2 天）→ S8（半天）→ S7（半天）~~ **S8（半天–1 天）→ S6（1–2 天）→ S7（半天）**（2026-09-22 审核后用户改定：S8 最小且独立，清掉最后一个已知读侧错值——我们自己写出的 hunk / 串列表下游读回即错、`DTYP` 恒 `true`；S6 门面的验收以读侧解码为 oracle，读侧先对、门面往返才可信）。

- S4 是 S6 的前置（门面靠它设值）；S4b 与 S4 互不依赖（只动 `reverse`），可先做；S5 与 S4 互不依赖，可并行；S7 / S8 独立。
- 估时按写计划 §7 的实际节奏（09-20 一天落地阶段 2 四个用例 + NAME 读侧）给，是上限。

---

## 4. 风险

| 风险 | 影响 | 处置 |
|---|---|---|
| S1 合入后 7 个下游 crate 一起吃到 `write` 与 `record::hunk` | 读侧 API 未改，但可见性有变 | S3 `cargo check` 兜；出问题只可能是编译错，不是静默错值 |
| S4 的 compact 布局（flag bit 29 清零、f32）无语料 | 写 compact 元素会错 | 先只支持 wide、compact 返回错误；有 compact 语料再开 |
| S5 的 GBK 转义规则读侧是从语料归纳的 | core 的 `sub_54B2CE8` 可能有读侧没见过的分支（不可映射字符） | **已读实**：分支确有——GBK 码两字节都 ≥ `0xA0` 才写，否则 638（`–` U+2013 这类 14 134 个 GBK 码写不出）；表里没有的 Latin-1 字符按单个原字节写；写侧照此落地，不可映射 → `TextEncodeError::Unmappable` 不写 |
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
- 2026-09-21：**S4b 落地——类型 17 反向引用表随写维护。**
  - **不变量普查先行**（`%TEMP%\e3dio_census\src\bin\type17_invariant.rs`，未入库；同一 609 库）：对每个设了值的类型 17 字段取全部非空项，到同名表里找边 `[t, 自身]`；再反向把表里每条边映回字段。结果 **8 个属性 3 191 条期望边 → 3 191 条全在表里；表里 3 191 条边 → 3 191 条全映回值；0 缺 0 多**（COCORE 564、HCREF 1 164、TCREF 871、ROUTND 4、CONNRE 119、STRREF 12、MDSYSF 301、DESLNK 116）。数组里 **0 个 unset 标记**；`CONNRE` 165 项里 46 个空 `(0,0)`，三个「无表却有值」的库其 CONNRE 项**全部是空引用**（非空 0 个）——无键即无表，坐实。explicit 字段 2 754 个 type_code **全部 `0x4400`**（= 17<<10）、布局 `[count][pair×count]`；**4 个字段（aps250514 ×2、aps250517 ×2）存了 21 字却只 count 1**——槽按容量 10 对预留、按 count 读，写侧因此按 count 不按长度取项。DESLNK 116 个全部走 implicit 槽（CYMLNK 偏移非 0）。12 s。
  - 落地：`write::reverse` 加 `REFERENCE_ARRAY_STORAGE_TYPE = 17`、`reference_array(draft, template, hash) -> Option<Vec<RefNo>>`（implicit：偏移处 `[count][pairs]`，count 按槽容量 `(storage_words−1)/2` 截；explicit：`explicit_words` 的 data 按 count 取；空对与 unset 标记跳过）、`set_reference_array(draft, template, hash, &[RefNo]) -> bool`（implicit 写 count + 对、槽余零填、超容量拒且不动；explicit 空表删项、否则 `[n][pairs]` 以 `REFERENCE_ARRAY_TYPE_CODE = 17<<10` 写，与语料 2 754 个字段一致）；`table_keys` 对模板里每个类型 17 属性按项出键 `[t, 自身]`。`write::record` 加常量 `REFERENCE_ARRAY_TYPE_CODE`。建表 / 目录追加 / 差集删插 / 删元素撤键全部复用 `keyed_cow` 与 `stage_session_with_base` 现有路径，一行未改。
  - 验收：单测 `reverse::a_reference_array_files_one_key_per_non_null_entry`（explicit 与 implicit 两档、每项一键、空对与 unset 不出键、count 之外的对不读、超容量拒写不动、空表清项 / 清槽、类型 16 属性拒）；集成 `tests/write_stage2_tables.rs::a_reference_array_moves_one_edge_per_entry_and_a_deleted_referrer_withdraws_them_all`（`ams251142_0001` 副本，表 SPRE 128 / HCREF 42 / TCREF 34 / TYPEX 362 / CONNRE 10 / POSREF 282 条键）：取首个同时设了 HCREF 与 TCREF 的引用者 `CWBRAN 2013287686/36`，先核它出的每个键都在表里；S+1 把 HCREF 首项 A 换成 B → HCREF 表恰好 −`[A,R]` +`[B,R]`、**其余表连根页号都没动**、记录读回 type_code `0x4400`、TCREF 原样、diff 改 1；S+2 删 R（并出 owner 表）→ 每张表恰好少掉引用者为 R 的键（HCREF 边 + TCREF 边 + TYPEX 键 = 3）、R 出索引、diff 删 1 改 1；两轮字节前缀不变、新页连续。全套：`cargo test --lib` **330** 通过、写路径 **37** 条全绿、clippy 写路径文件 0 warning、cargo doc 仍 24 条。
  - 仍未做（类型 17 相关，记入 §2 C）：A3b 的 `sub_5B0D950` 对数组属性按**元素 k 逐槽**三读一写（17 的元素数 = `(size−1)/2`），两边改同一数组的**不同槽**时 core 两者都留；本 crate 的 `three_way` 按**整个属性**比，同一数组两边都改则写者整段赢。`sub_5AB8DC0` 写 17 时回 23 → 冲突 576 也未建模（`keyed_cow` 插已有键幂等）。语料里没有并发改同一数组的样本可坐实，先记下。
- 2026-09-21：**S4 落地——按描述子设属性 `RecordDraft::set_attribute(&template, hash, &DescriptorValue, TextFormat)`（新模块 `write::attr`），验收 = 三个样本库 7 825 个元素 149 558 行往返恒等 0 例外。**
  - 口径 = `extract_parsed_element_with_descriptors` 的逆，按描述子 `wide.location.word_offset()` 判 implicit 槽 / explicit 项，explicit 的 type_code 一律 `存储类型 << 10`（与读侧 `explicit::decode_value` 的分派一致，语料 0 例外）：2/6 real = 两字 Fortran 序（低半在前）、标量直写 / 数组 `[n][2n]`；3/7/18 word = 一字，`Int`/`Bool`(0/1)/`Word{raw}`/角度属性的 `Real`→有符号百分度（`is_angle_attr`）；4/8/16/17 ref = 两字，数组 `[n][2n]`；5 = 槽字里 `location>>20` 那一位读改写（不动同字其它位），explicit 形 `[0|1]`；9 = `[n][ceil(n/32)]` 位集；10/15 文本 = `[字节数][4 字节/字 NUL 补齐]`，FORMAT 1 存 UTF-8 原字节、其它 FORMAT 暂只收 ASCII（S5）；14 = `[n][n×stride]`，stride 取 compact 列 `(storage_words−1)/logical_count`（`sub_5AB5600`）；19 拒（走串列表 setter）。**只写值需要的字**：implicit 槽里 count+元素之后的字原样保留（`4SCEA` 也不清），所以盘上读回再写回逐字相同。`RawWords` 的含义按落点分：implicit = 元素（读侧去掉了 count），explicit 且读侧原样透传的类型（14/17/18/19…）= 整个字段 data 原样。**读回的 hunk 目录项**（type_code 20–23 的 5 字 `RawWords`）原样重发、地址不动——与 `sub_5AE9960` 重写记录不搬 hunk 一致；新 hunk 值走 `set_*_hunk`。`Unset` = implicit 写 `0x80000001`（5 清位）、explicit 删项。compact 记录（字 10 bit 29 清）拒；数量 > `logical_count`（≠0）拒；槽放不下 / 出记录拒；拒写不动 draft。
  - **往返普查**（`tests/write_stage4_set_attribute.rs`）：对每个元素 `from_record_bytes` 成 draft，把读侧每行 `Decoded`/`DecodedExplicit` 的值 `set_attribute` 回去，`content_eq` 必须等于原记录，不等则按描述子逐槽/逐项报第一处差异。**`ams8000_0001`（FORMAT 1）6 639 元素 140 112 行**（按类型/落点：2 implicit 11 447 + explicit 7 259、3 25 834 + 7 448、4 10 678、5 19 373 + 9 275、6 13 965 + 2 228、7 793 + 2 234、10 637 + 1 067、14 5 253 + 5 941、15 1 834、16 14 238 + 608）**0 差异**；**`ams251142_0001`** 948 元素 6 510 行（含 17 explicit 87）0 差异；**`ams5100_0001`（DICT、FORMAT 0）** 238 元素 2 936 行，其中两类按预期跳过：**57 个中文文本**（`&~…&` 转义库、写侧还没有编码器 → S5 的现成 oracle）与 **185 个 `UDA.DTYP`**——读侧按 attlib 的 `data_type 3` 把一字 word 解成 `Bool(raw != 0)`，而盘上是 `640481`/`942752`/`536961` 这类 WORD（E3D 打 `Dtype TEXT`），值到写侧时已经丢了原字，**这是读侧的错**（记 S8）；其余 2 751 行 0 差异。类型 8/9/18 三个样本里没有设过值的，靠单测（`write::attr` 3 条：implicit 各形、explicit 各形 + Unset、各种拒写不动 draft + UTF-8 文本按字节）。全套 `cargo test --lib` **333**、写路径 **38** 条全绿；clippy 写路径 0 warning；cargo doc 24 条。
  - **顺带把 §1.3 第 11 条关掉了——语料里有一个 E3D 自己写的 hunk。**（下接）
- 2026-09-21：**S1 做到「差最后一步」——`main` 已并进 `write-path`（`8d50edf`，CHANGELOG 两边各自的新条目都留）、0.2.0 收口已提交（`57760f8`：README 加「Write path」一节 + 写侧 API 表、`Cargo.toml` 0.2.0 与新 description、MISSION 纳入写回、CHANGELOG `Unreleased` 归档为 `0.2.0 — 2026-09-21` 并分 Write path / Read path 两个小标题、工作树 22 个 CRLF 假改动全部还原为 LF、`Cargo.lock` 随版本号）；全套 `cargo test --no-fail-fast` 90 个测试二进制 0 失败。`write-path` 现在包含 `main` 的一切（`git rev-list --left-right --count main...write-path` = `0 / 34`），`main` 可以 fast-forward。**没做的一步：`main` 的 fast-forward。** 原因：`main` 检出在 `D:\work\plant-code\old\vendor\e3d-io`，那个工作树里有**别的会话未提交的活**——`src/element_name.rs`（+587/−69）、`src/db_element.rs`、`src/lib.rs`、`CHANGELOG.md` 改动 + 未跟踪的 `src/element_name/`、`tests/element_name_real.rs`（mtime 09-20 15:19 与 09-21 15:35），`zhimo check_paths` 无锁无任务。`git merge --ff-only` 要动 `CHANGELOG.md` / `src/lib.rs`，会被那些本地改动挡住；`update-ref` 硬挪分支会让那个工作树的 HEAD 与索引脱节，等于替别人撤活。所以停在这里等用户定：那份 element_name 工作先提交 / 先 stash，或由用户指定谁来收。S3 先用 cargo 的 `paths` 覆盖（`cargo check --config 'paths=["…/e3d-io-write"]'`）对下游试编译，不改任何下游文件。
  - **用户拍板：那份活直接提交到 `main`，再合 `write-path`。** 先在 `main` 工作树核了它能编译、`element_name` 单测 9 条 + `element_name_real` 6 条 + `db_element_facade` 3 条全绿，`git add -A` 提交为 `3b1878d`（`wip(element_name)`，提交消息写明来历）；`git merge write-path` 无冲突（`a6c0a2a`，CHANGELOG 里 WIP 的 `ElementName` 条目落在 0.2.0 的「Read path」下）；`write-path` 随后 fast-forward 到同一提交，两个工作树同头（`main...write-path` = `0 / 0`）。**S1 至此完成**（`main` 上全套测试与 clippy 结果见下条 S3）。
  - S3 第一轮（`paths` 覆盖到合并前的 `write-path`）：e3d-model 的 6 个变体 `cargo check` **全部 0 error**；gen-model 6 个 error——4 个是 `e3d_io::ElementName` / `display_name` / `member_names` 找不到（gen-model 已经在用那份当时还没提交的 element_name WIP，覆盖到 write-path 自然没有；合并后自消），另 2 个是 gen-model ↔ e3d-model 的 `change_is_metadata_only` 返回类型不合（`window_root_plan.rs:628` ↔ `e3d-model/src/increment.rs:1093`，e3d-model 工作树里有 9 个别人未提交的改动）——**与 e3d-io 无关**。第二轮对合并后的真 `main` 重跑，结果见下。
  - **S3 第二轮（下游 path 依赖直指合并后的 `main` `a6c0a2a`，不带覆盖、不改下游一个文件）**：`e3d-model` + `e3d-model-adr079` / `-audit-verify` / `-f29-verify` / `-fix-verify` / `-head-tmp` / `-mintube` **7 个 crate `cargo check` 全部 0 error**；`gen-model` 只剩 **1 个 error**，`window_root_plan.rs:628` 期望 `change_is_metadata_only` 返回 `Option<NoModelTier>` 而 `e3d-model/src/increment.rs:1093` 返回 `bool`——gen-model ↔ e3d-model 之间的接口漂移（e3d-model 工作树里有 9 个别人未提交的改动），第一轮同样有、不含任何 e3d-io 符号；第一轮 gen-model 报的 4 个 `ElementName` 错在合并后全消。**结论：写路径合入 `main` 没有弄坏任何下游的编译。** 合并头 `a6c0a2a` 在 `main` 工作树全套 `cargo test --no-fail-fast`：**91 个测试二进制 0 失败**（90 + WIP 带来的 `element_name_real`），`cargo clippy --all-targets` 0 error。S1 / S3 完。 往返普查在 `DBMDAT 24384/26333`（`ams8000_0001`）撞到 `DBMVAL`：模板说类型 7、盘上 explicit 项 type_code `0x5000`（20<<10）、data 5 字 `[1, 16456, 0x18001, 5709, 5709]` = **类型 20 int 数组 hunk，5 709 字**（`DBMVAL` 描述子 cnt 0 → 阈值 1024，超过即外联）。用读侧对着 core 的写法逐字核：目录项 `[1][页][packed = ext | 字<<13][元素数][字数]` ✓；首块在页 16456 字 12，块头 7 字 `[3<<16 | 500][24384][26333][16457][0x2001][173692327][20]` = kind 3、本块 500 字恰填到页尾（12+500=512）、续接 `(16457, ext 1 字 1)`、hash、type ✓——与 `write::place_block` 写的 kind 3 块头**字字相同**；`read_hunk` 沿链读回恰 5 709 字。之前「语料无 hunk」的结论是只查了模板类型 19；这一条把 hunk 块头 / 目录项 / 续接三样都从 E3D 自己的产物上坐实了。旁证一条：该记录头在同页字 1、hunk 在字 12——E3D 这一次是记录 implicit 区先落、hunk 紧随其后、explicit 块另页；我们是 hunk 先放。两种顺序读侧都认（`Q ATT` 门已过）。
- 2026-09-22：**S5 落地——非 ASCII 文本按头 `FORMAT` 编码，`record::text::encode_as`。**
  - **先读 core**（`docs/evidence/2026-09-22-core-dll-text-charset-write.md`；ida-bridge 实例换成 `idalib-4000`——`idalib-33060` 被本轮一条写错的 xref 循环卡死，`supervisor stop` 后同一 `.i64` 重开）。`db_cxx_put_str_vector`（`sub_5AD5060`）读头整数 18（`FORMAT`）：`== 1` → `sub_54719F0`（CHPACK，只查串尾是否切断 UTF-8 序列）原字节打包；其它 → `sub_5B8F3C0` 摊字节 → **`sub_54B2CE8`**（源 12 = UTF-8 → 目标 `MBCHARSET` `dword_6453EDC`）→ `sub_5471E50` 打包；状态 3 → 23，其它非零 → **638**，不写。转换器是一个通用多字符集转换器（1 Latin-1、2/3/20–22 单字节表、4/24 SJIS→JIS、5/25 EUC、**6/26 GBK**、7/27 Big5、8–11 无转义双字节、12 UTF-8、13/14 UTF-16/32、23 `\U+` 转义），4–7 带 `&~ … &` 转义、24–27 同表不转义。**中文站点 = 6**：读侧「运行外高位字节走 CP1252 兜底」恰是 `case 6` 的行为，6 的表 `sub_54B88D7` 按尾字节 `0x40..=0xFE` 191 列索引（GBK 形状），`MBCHARSET` 的名字→数字映射不在 core.dll（`sub_5AE76F0` 唯一写点，数字由 `sub_5AD9DB0` 的调用方传）。**规则**：ASCII 原样（运行开着先补 ` &`）；非 ASCII 查 `sub_54B9032`——查不到且码点 `0x80..=0xFF` 按**单个原字节**写、查不到其它 → 状态 2；单字节结果（只有 `€`→`0x80`）原样写；双字节**要两字节都 ≥ `0xA0` 且 ≠ `0xFF`** 才写（运行没开先写 `&~`，两字节各减 `0x80`），否则状态 2；收尾补 ` &`；`U+FFFE`/`U+FFFF`/代理/≥ `0x110000` 状态 1。**表**：在 IDA 内跑全 BMP，`sub_54B9032` 有 **21 792** 个映射，与 Python `gbk`/`cp936` 全等（多 `€`→`0x80`），不是 GB2312（多 14 348）也不是 GB18030（少 2 148）；**与 `encoding_rs::GBK` 21 792 全等、0 差异**，encoding_rs 另多 2 165 个（1 364 个 PUA `U+E000..=U+E864` + 13 个非 PUA 能过字节规则：`U+FE10..=U+FE19`、`ḿ`、`ǹ`、`龻`，其余 788 个本来就过不了）。能进 `&~ … &` 的双字节码 **7 657** 个（GB2312 7 443 + 214）；GBK 另 **14 134** 个码（如 `–` U+2013 → `A843`）core 写不出、报 638。
  - **落地**：`record::text::encode_as(text, TextFormat) -> Result<Vec<u8>, TextEncodeError>`（`Nul` / `Unmappable` / `FormatUnknown`——`Detect` 只放 ASCII）= `encoding_rs::GBK` + 拒 PUA + 拒 `NOT_IN_CORE_GBK` 13 码 + 字节规则 + Latin-1 原字节；`pack_words` 收拢四处各写一遍的 4 字节/字打包。接入：`RecordDraft::set_explicit_text` / `add_explicit_text` / `set_explicit_text_list` / `set_text_hunk` / `set_text_list_hunk`（带 `TextFormat`；`_ascii` 五个保留为只收 ASCII 的包装，行为不变）、`text_list_data`；`write::attr::text_words`（`set_attribute` 的 10/15，`AttributeWriteError::TextNotAscii` → `Text { hash, source }`）；会话页 `USER` / `DESC` 按**存储字节**对槽校验、控制字符仍拒（`WriteError::TextNotAscii` → `Text { field, source }` + `TextControl`）；`index::named::encode_name_as(name, format)` + `NamedKey::text_as`、`NamedTable::lookup_name_as`，**`ReadOnlyEngine::find_by_name` 改按头 `FORMAT` 编键**（之前在 `FORMAT` 0 库上找不到中文名——读侧顺手关掉的一个坑）。`e3d-write-box --text` 加第五个会话（BOX 中文 NAME + 表键、ZONE 中文 DESC、中文注释）。
  - **验收**：① 单测 `record::text` 4 条新增（`ams1112_0001` 8 条转义串逐字节写回并解回、运行嵌在明文中间、`範板` 尾字节 `0xA0` 存成空格、两次开关运行、11 种拒、`ÿ`→`0xFF`、`€`→`0x80`、`café`→`caf&~(& &`、NUL 拒、`pack_words`），`write::attr` / `write::session_page` 各补 Escaped 用例（`反应堆` → `[10][&~74S&6Q &]`；USER 槽 36 字节：9 个汉字转义 22 字节进、18 个 40 字节拒 `TextTooLong{len:40,max:36}`）。② **往返**：`ams5100_0001`（DICT、`FORMAT` 0）的 **57 条中文文本**不再跳过，写回 `content_eq` 逐字相等（2 993 行 0 差异；`bool_over_a_word` 185 照旧记 S8）。③ **E3D 自写的转义键**：`TES023`（TES000，DESI，`FORMAT` 0，288 KB，新增 fixture）的 SITE `/锅炉岛设备` 表键 `/&~9xB/5:Ih18 &` = `encode_name_as` 输出、`find_by_name` 命中；`tests/write_stage5_text.rs` 2 条：TES023 副本上 ZONE 改名 `/反应堆厂房-E3DIO` + DESC `反应堆厂房 -3.320m~±0.00m` + 中文注释 → 盘上 `DESC` = `[34][&~74S&6Q3'7? & -3.320m~&~!@ &0.00m]`、键 `/&~74S&6Q3'7? &-E3DIO`、`extract` / 表 / 会话页三处解回原文、旧名消、E3D 的键仍在、diff 改 1、字节前缀不变新页连续；`a–b` 被 `set_attribute`（`Unmappable{ch:'–'}`）与会话注释（`WriteError::Text{field:"DESC"}`）拒且文件字节不变；`ams8000` 副本同一套走 UTF-8（含 `–`）。④ **TTY 门**（`tools/e3d/write_gate.ps1`，AMS 项目 `FORMAT` 1）：加 Q8a `Q NAME` → `Name /E3DIO-盒子`、Q8b `Q DESC` → `Description 反应堆厂房 -3.320m~±0.00m`，Q7 `Q SESSION` 只打最后一个会话、改看中文注释；Q2/Q5 的 BOX 现在按名字列出（`BOX 1 /E3DIO-盒子`）；日志读法改成「UTF-8 严格解、失败再 ANSI」——E3D 的 ALPHA LOG 实测是 UTF-8；**16 问全 PASS**，9 s。`FORMAT` 0 没上 TTY 门（TES000 的 MDB / 用户没接进驱动），口径靠 ③ 的 E3D 自写键。全套：`cargo test --lib` **347**、写路径 **40** 条（含 stage5 2 条）+ `element_name_real` / `db_element_facade` / `session_tail_real` 全绿；clippy 改动文件 0 warning、全仓 0 error；cargo doc 24 条不变。
  - 留待：`record::text::escaped`（读侧）对运行外的高位字节串按 GBK 读，core 逐字节走 CP1252——写侧照 core 把表外的 Latin-1 字符（`Ø`、`ÿ`…）写成单个原字节后，两个相邻这样的字节若恰好组成合法 GBK 对，本 crate 读回会错、E3D 不会；语料里无此形态，记下不改。
- 2026-09-22：**进度审核（S5 之后、S8 之前）。**
  - **数字**（当日实跑）：HEAD `5dfd61a`，`git rev-list --left-right --count main...write-path` = `0 / 0`（两个工作树同头；另 4 个 detached worktree `merge-0.1.30` / `_wt-f39` / `_wt-f42` / `p1p2` 停在旧提交）；`cargo test --lib` **347 passed / 2 ignored / 0 failed**（1.14 s）；写路径 11 个测试二进制 **40 条全绿**（stage1 6 · claim 2 · crosspage 2 · elements 2 · hunks 2 · modify_delete 2 · naming 3 · tables 4 · merge 14 · stage4 往返 1 · stage5 文本 2）；`cargo clippy --all-targets` 0 error，余下 warning 全在读侧 / examples / 老测试。工作树 5 个文件 `M`（`attr.rs`、stage4 / stage5 测试、两份 docs）**全是 CRLF 假改动**（`git diff --ignore-cr-at-eol` 0 行）——S1 还原过一次，编辑器又写回 CRLF；本轮 `git checkout -- .` 还原，`git ls-files --eol` 五个全 `i/lf w/lf`。
  - **切片**：S1 / S2 / S3 / S4 / S4b / S5 全部落地且各有验收数字（见上各条）；**S6 / S7 / S8 未开**。
  - **代码里坐实的三点**：① S6 的缺口是真实的调用方负担——`e3d-write-box.rs:121-132` 新 refno = `max_word1 + 5000` 手取、`:561` `NamedEdit` 与记录里的 NAME 项手工配对、`:160 / :348 / :409 / :484` owner 成员表手工 `push_member`，三处一致性全靠调用方，下游（plant-model-2 / gen-model）接入前必须先收成门面。② S8 的「Bool 压 word」不只是显示错：`record/descriptor.rs:603` `Some(3) => Bool(raw != 0)`，而 `uda_catalog.rs:430` 拿 `DTYP` 当 `default_is_defined` 用——盘上 `DTYP` 是 `TEXT` / `REAL` / `INTE` 这类 WORD，所以任何设了 `DTYP` 的 UDA 该字段恒 `true`，是读侧语义错，S8 改判定会波及此处与其夹具（`:571`），改前先查下游谁读 `default_is_defined`。③ `plant-model-2` 工作区目前是空目录——若它是写路径的第一个消费者，S6 门面就是它的入口。
  - **方案**：S8 → S6 → S7（用户拍板 B 顺序，§3 已改）；S6 一起把公开面收敛（`Transaction` / `RecordDraft::set_attribute` / `DescriptorValue` / `StagedSession` / `WriteError` 对外，`index_cow` / `keyed_cow` / `claim_cow` / `named_cow` 收 `pub(crate)` 或 `#[doc(hidden)]`），切 0.3.0，S3 同口径再跑一遍下游 `cargo check`；C 段留待不动。
- 2026-09-22：**S8 落地——读侧跟进 19 / 23 / 20–22 与 Bool 门。**
  - **串列表（19 / 23）**：`ExplicitValue::TextList(Vec<String>)`、`DescriptorValue::TextList(Vec<String>)` 两个新变体（不是计划里写的 `AttrValue`——那是不带字符串的旧摘要枚举，`Text` 在它那里是 word，串列表映 `None`）。`explicit::decode_value` 对 `0x4C00` 用 `hunk::decode_text_list` 拆项、拆不开留 `RawWords`；`descriptor::StorageShape` 把 19 从 `Characters` 里分出来成 `TextList`（`Characters` 只剩 10 / 15），槽内按同一 buffer 解、首字 0 = 空表、槽里表尾之后的字不读、不成形报 `Unsupported` 而不是当文本读。`decode_hunk_value` 的 23 也解成 `TextList`。
  - **hunk（20–23）解在有页的一层**：纯函数 `extract_parsed_element_with_descriptors` 没有页，目录项照旧五字 `RawWords`——写侧 `set_attribute` 靠它原样重发目录项、stage4 往返普查靠它——所以新加 `record::hunk::resolve_hunks(pages, ext, &mut parsed) -> usize`：对流里每个 `HunkRef::from_explicit` 成立的项 `read_hunk` + `decode_hunk_value` 换掉 `value`，`type_code`（`20..=23 << 10`）与 `data_words`（5）**不动**，调用方仍能看出值曾在记录外。`ReadOnlyEngine::resolve_hunks(&mut parsed)` 用引擎的页与索引根的 ext；`extract_element_with_descriptors` 先解再按描述子解（读不动 → 新 `ResolveError::Hunk`）；`DbElement::parsed_uncounted` 解析完立刻解，池级缓存里存的是值不是地址。状态用 explicit 值一贯的 `DecodedExplicit`。
  - **Bool 门**：`descriptor.rs` `StorageShape::Word` 标量 `Some(3) if raw <= 1 => Bool(raw == 1)`，否则 `Word { raw, text: word_name(raw) }`。`uda_catalog::UdaDefinition::default_is_defined` 改为「`DTYP` 存了非零字」（`set_word`，兼容 `Bool` / `Int` / `Word`），新增 `default_type: Option<String>`（`TEXT` / `REAL` / …），`e3d-descriptor uda` 多打 `default_type`；夹具里的 `DTYP: Bool(true)` 改成 `Word { raw: 768_519, text: "REAL" }`。下游无人读 `default_is_defined`（`rg` 全 `old/` 只有本 crate）。
  - **写侧顺带**：`set_attribute` 对 19 收 `TextList`（`text_list_data` 出 buffer，implicit 写槽、explicit `19 << 10` 无计数字），空表 = `Unset` 路径（`4SCEA` 长度 0 即 unset）；`AttributeWriteError::StringList` 删掉，改走 `TypeMismatch { shape: "a string list" }`。`tty::compare_tty_value` 加 `TextList` 臂：E3D 一行一项 ` 1 K1` / ` 2 K2`，attlist 拼成 `1 K1 2 K2`，按 `"{i} {item}"` 连接后忽略大小写与空白比。
  - **验收**：① 单测 `cargo test --lib` **350**（+3：explicit 类型 19 解表 / 不成形留 raw；descriptor Bool 门四档 `TEXT` / 1 / 0 / unset；descriptor 19 槽三档 两项 / 空 / 不成形），`hunk` 的 23 用例改成 `TextList`。② **`ams8000` E3D 自写的 hunk**：新 `tests/hunk_resolve_real.rs`——`DBMDAT 24384/26333` 的 `DBMVAL` 目录项 `(20, 5 709, 5 709)` 起于页 16456，`resolve_hunks` 返 1、值 `IntArray` 5 709 个、其余项逐字不变、再解返 0；描述子路径 `DecodedExplicit`、5 709 元素（到达形态是 `RawWords`：DBMVAL 在 attlib 无定义，类型 7 数组无定义时读侧一贯不定 int / word）。③ `tests/write_stage2_hunks.rs`：ROKEYS 断言由 `RawWords` 改 `TextList`；写完 `resolve_hunks` 返 4（再解 0）、ROVALS / IPARAM / PARA / DESC 四值与所写相等且 `type_code` 仍是 hunk 码、`data_words` 仍 5；`extract_element_with_descriptors` 对 GROU 模板声明的那 1 个（DESC）报 `DecodedExplicit` 且值相等。④ **stage4 往返普查**：`bool_over_a_word` **185 → 0** 并改成 `assert_eq!(…, 0)` 硬断言；`ams5100` 2 993 行全部写回相等（其中 185 个 `DTYP` 现在以 `Word { raw }` 写回原字，类型 3 implicit 共 1 484 行）；`ams8000` 140 112 行、`ams251142` 6 510 行照旧 0 差异。⑤ **TTY 门副本**（今晨 10:59 那份 `D:\e3dio-gate`）：`e3d-descriptor extract` EQUI 2 `24384/31487` → `DESC` `decoded_explicit` = E3D 打的 120 字符 `Description …`（我们多一个尾空格，E3D 打印时裁掉）、`DESP` = 100 个 `2001.25 …`（E3D 打 `2001.3`，一位小数）、`UUIDIA` = `[21,22,23,24]`；FIXING `24384/31488` → `ROKEYS` = `["K1","K2"]`、`ROVALS` = `["V1","V2"]`（E3D 打 ` 1 K1` / ` 2 K2`）。重跑 `tools/e3d/write_gate.ps1`：**16 问全 PASS**，12 s。⑥ 全套 `cargo test --no-fail-fast` **93 个测试二进制 0 失败**；clippy 全仓 0 error，改动文件 0 新 warning（lib 24 → 22，顺手清了 `explicit.rs` 一处 `chunks_exact` 与 `engine.rs` 一处 `format!`）；cargo doc 24 条不变。
  - **下游（S3 口径，`--config paths` 指到本工作树）**：`e3d-model` `cargo check` **0 error**；`gen-model` 除既有的 `change_is_metadata_only` 漂移外，**4 处穷举 `match` 少 `TextList` 臂**（Q3 预期的那种）——`src/data_interface/direct_attmap.rs:271`（`natural`，建议 `D::TextList(v) => NamedAttrValue::StringType(v.join(" "))` 或按 gen-model 的列表类型）、`:423`（`shape_of`，`format!("TextList[{}]", v.len())`）、`src/data_interface/element_attributes.rs:903`（`descriptor_json`，`("text-list", json!(v), v.join(" "), true)`）、`:959`（`explicit_json`，同上）。gen-model 工作树里有别人未提交的改动（含 `element_attributes.rs`），**本轮不改 gen-model**，等其归属者补臂；`main` 未合入前下游不受影响。
  - 留待：`AttrValue`（旧摘要枚举）不加 `TextList`；hunk 解析不进 `session::delta`（它比两版记录的目录项，地址不同 = 值可能不同，够用）。
