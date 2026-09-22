# 计划：e3d-io 写路径 · 第二期（0.3.0 之后）——收口、消费者门面、规模门、SPAMAP 后果实测

状态：**评审通过**（2026-09-22，用户在面板上批「Q1–Q6 全按推荐项定」；Plannotator 门另行 Approve）→ T0 开工（§7）。接班会话 fable-5-1-12 的评审补充见 §5.1（七条，方案与顺序不变）、T0.2 写者复核见 §7（是 Cursor 本身，不是同步工具）。
日期：2026-09-22
分支 / worktree：`write-path` @ `D:\work\plant-code\old\vendor\e3d-io-write`，HEAD `23af76f`（2026-09-22 16:44）；`main` @ `D:\work\plant-code\old\vendor\e3d-io` 同头（`git rev-list --left-right --count main...write-path` = `0 / 0`）
上一期：`docs/plans/2026-09-21-write-path-next-steps.md`（下称「上期计划」；其 §7 是 S1–S8 的第一手账）、`docs/plans/2026-09-20-dabacon-write-plan.md`（「写计划」）
依据：`docs/evidence/2026-09-20-core-dll-dabacon-write-path-algorithms.md`（「算法清单」）、`docs/evidence/2026-09-21-e3d-tty-gate-write-path.md`（「TTY 门」）、`docs/evidence/2026-09-22-core-dll-refno-allocation.md`、`docs/evidence/2026-09-22-core-dll-text-charset-write.md`
影响仓库：`vendor/e3d-io`（本仓库）。下游（gen-model / e3d-model 及 verify 变体）只 `cargo check`，不改。

---

## 0. 一句话

写路径 A / B 两段全部落地：真 E3D 3.1 读回 e3d-io 写的每一样东西（TTY 门 16 问 PASS），今日实跑 **96 个测试二进制 594 通过 / 0 失败**，clippy 0 error；0.3.0 公开面收敛已在 `main` 工作树改好、**未提交**。但写路径至今**没有一个消费者**（`rg e3d_io::write` 在 gen-model / e3d-model / plant-ui / rs-server 里 0 命中；`plant-model-1` / `plant-model-2` 是空目录），唯一的调用方是夹具 `e3d-write-box`。第二期因此分四段：**T0 收口**（0.3.0 提交、CRLF 根因、下游 `TextList` 臂）；**T1 消费者门面**——`Transaction` 补齐真项目改模型第一天就要的四样（换 owner / 成员重排、按属性名以字符串设值 + E3D 字典口径校验、以 base 会话打开 + 提交时三方合并、批量指令 CLI）；**T2 规模门**——万级元素一个会话的时间 / 页数 / E3D 读回；**T3 SPAMAP 缺失后果实测**——半天定「要不要上几何」；**T4 验证加固**。C 段留待项照旧只写触发。

---

## 1. 现状（2026-09-22 实测）

### 1.1 数字

| 项 | 数字 | 怎么得到的 |
|---|---|---|
| 提交 | `write-path` = `main` = `23af76f`；今日 8 个提交（S8 → S6 → S7 → S6 尾 → S6b 读侧） | `git log -12`、`git rev-list --left-right --count main...write-path` = `0 / 0` |
| 全套测试 | **96 个测试二进制，594 passed / 0 failed / 30 ignored**；32 s（含增量编译） | `cargo test --no-fail-fast` 本轮实跑，日志 `%TEMP%\e3dio_write_next_test.log` |
| 单测 | `cargo test --lib` **355 passed / 2 ignored** | 同上 |
| 写路径集成 | 13 个测试二进制 **45 条全绿**：stage1 6 · claim 2 · crosspage 2 · elements 2 · hunks 2 · modify_delete 2 · naming 3 · tables 4 · merge 14 · stage4 往返 1 · stage5 文本 2 · stage6 事务 3 · stage7 锁 2 | 同上 |
| clippy | `cargo clippy --all-targets` **0 error**；lib 22 warning，`src/write/**` 0 条，其余全在读侧 / examples / 老测试 | 本轮实跑 |
| 外部门 | TTY 门 **16 问全 PASS**，12 s（refno 由 `Transaction` 发号：BOX `24384/26484`） | 上期计划 §7 S6 尾条 |
| 写工作树 | 10 个文件 `M`，`git diff --ignore-cr-at-eol --numstat` = **0 行**——全是 CRLF 假改动（`git ls-files --eol`：`i/lf w/crlf`） | 本轮实测 |
| `main` 工作树 | **真改动未提交**：`Cargo.toml` 0.2.0 → 0.3.0、`CHANGELOG.md` `Unreleased` → `0.3.0 — 2026-09-22`（+4/−1）、`src/write/mod.rs` +18/−9（`claim` / `claim_cow` / `index_cow` / `keyed_cow` / `named_cow` 收 `pub(crate)`，`LeafEdit` / `KeyedEdit` / `ClaimEdit` / `plan_claim_edits` 等不再 re-export，错误类型与 `NamedEdit` 保留）；`src/meta/bucket.rs` / `tests/bucket_table_real.rs` 只是 CRLF | `git -C ../e3d-io diff --ignore-cr-at-eol --numstat`；`zhimo check_paths` 无锁无任务 |
| CRLF 根因线索 | 两个工作树 **40+ 个文件的 mtime 同为 18:35:31**（含未打开的 `MISSION.md`、`examples/*.rs`）——编辑器不会同一秒改 40 个没打开的文件，这是批量写者（同步工具一类）的手笔 | `Get-ChildItem … | Where LastWriteTime -eq 18:35:31` |
| 消费者 | `e3d_io::write` 下游 **0 处引用**；写路径唯一调用方 = `src/bin/e3d-write-box.rs`（`--group` / `--hunk` / `--text` 三个开关） | `rg` gen-model / e3d-model / plant-ui / rs-server |
| 下游 | e3d-model + 6 个 verify 变体 `cargo check` 0 error；gen-model 4 处 `TextList` 臂**已补但未提交**（其工作树有别人的 WIP），另有与 e3d-io 无关的 `change_is_metadata_only` 漂移 | 上期计划 §7 S6b 条 |

### 1.2 `Transaction` 门面今天能做 / 不能做（`src/write/transaction.rs` 实读）

| 能 | 不能（真项目第一天会撞到） | 证据 |
|---|---|---|
| `open(path, &mut templates)`：开在**最新**会话，扫索引求每扩展 max `word1` | **以 base 会话打开、提交时合并**：`open` 只有 latest；开到提交之间 E3D 存过一次 → `WriteError::Moved`，调用方要从头重做。三方合并原语 `write_merge_session_with(path, request, base_sesno, …)` 在，但门面够不着 | `transaction.rs:127`、`merge.rs:163` |
| `insert(owner, noun, name, build)` / `update(refno, edit)` / `rename` / `delete`；refno `max+1`；owner 成员表随插删；`NAME` 属性 ↔ 表自动配对；TYPEX / 反向表 / claim 随写 | **换 owner（re-parent）/ 成员重排**：`RecordDraft` 有 `owner` 字段与 `move_member_after`，合并侧也认移动（A3b 第 3 步已落地），但门面无 `move_to` / `reorder`；GROU 引用成员也留给调用方 | `record.rs:113/270`；`transaction.rs` 模块头「what it does not」 |
| `build` 闭包里 `draft.set_attribute(template, hash, &DescriptorValue, text_format)` | **按属性名以字符串设值**：调用方要自己算 hash、自己把字符串转成对的 `DescriptorValue` 变体（Real / Int / Word / Bool / RealArray / Ref / Text / TextList）、自己传 `TextFormat` | README「Write path」示例：`db1_hash("DESC")` |
| 模板容量校验（`logical_count`、槽放不放得下、文本**字节**容量） | **E3D 字典口径**：TTY 门坐实的三条——`DESC` 上限 **120 字符**（模板 480 字节）、`UUIDIA` set-once、`RATM` / `UNIPAR` 不是 E3D 认的属性、`USRWEI` 怎么存都读 unset——只写在文档里，代码不拦，写进去 E3D 才报错 | TTY 门文档；README |
| `stage` / `commit` 两段；`TransactionError` 12 个变体 | **批量脚本入口**：只有夹具 bin，没有一个吃指令文件的通用 CLI | `src/bin/` 只有 `e3d-descriptor` / `e3d-write-box` |

字典口径**有数据源**：`schema/e3d31/catalog/noun_attr_fields.json`（live E3D 导出）按属性列 `DbAttributeField`——本轮实读 `DESC` **`SIZE=120`**（正是 TTY 门撞到的 120 字符上限）、`NAME` 500、`UUIDIA` 4、`DESP` 100、`ROKEYS` 20、`POS` 3；`RATM` / `UNIPAR` **不在导出里**（= E3D「does not have attribute」）；`USRWEI` 在（`SIZE=500`、`ITYP=48`）但 E3D 读 unset，`UUIDIA` `CHANGE=True` 但实测 set-once——所以 `SIZE` 与「在不在导出里」可以数据驱动，set-once / 伪属性还要一张小的例外表以 TTY 门为 oracle。

### 1.3 上期留下的口子

| # | 项 | 现状 | 本期 |
|---|---|---|---|
| 1 | 0.3.0 公开面收敛 | `main` 工作树改好未提交（§1.1） | T0.1 |
| 2 | CRLF 假改动反复出现（S1 还原过、进度审核还原过、今又 10 个） | 根因指向 18:35:31 的批量写者 | T0.2 |
| 3 | gen-model 4 处 `TextList` 臂 | 已补未提交（gen-model 归属者的工作树） | T0.3 |
| 4 | S6b 页 1 桶表**写侧** | 读侧 + 布局 + 证据已落；`max+1` ≈ E3D 收敛值，写侧收益边际 | C 留待，触发不变 |
| 5 | SPAMAP | 不维护；写的几何元素不进空间索引。TTY 门只证 `Q ATT`，**没证 3D 视图 / 空间查询 / E3D 存盘会不会自己补** | T3 先实测 |
| 6 | UDA 表 / extract / Central / 多 extent / compact / E3D 2.1 / 部分存盘 / 类型 17 逐槽三方（576） | 无语料或无需求 | C 留待 |
| 7 | 性能 | 只有「45 条用例 1.7 s」「2000 成员跨页 8+ 页」；**无万级数字** | T2 |
| 8 | TTY 门盲区 | `FORMAT 0` 项目没上门；Q17（活 E3D 写模式 → `Locked`）未自动化；E3D ↔ e3d-io 交替写只做过 1 轮 | T4 |
| 9 | `tests/` 里引用 `ams1112_0001` 的十几个测试一个断言都没跑（本机无此样本，`SKIP` 即 return） | NOTES.md 已记 | T4 |

---

## 2. 切片

### T0 · 收口 0.3.0（半天；多数是别人 WIP 的验收，本计划只定验收）

- **T0.1 0.3.0 提交**：在 `main` 工作树 `cargo build` → `cargo test --no-fail-fast`（须 = §1.1 的 96 / 594 / 0）→ S3 口径下游 `cargo check`（e3d-model + 6 变体 + gen-model，只可能少 `pub` 项编译错，不可能静默错值）→ 提交 `chore(release): 0.3.0` → `write-path` fast-forward → `Cargo.lock` 随版本。归属：改这份 WIP 的会话（Q6）。
- **T0.2 CRLF**：写工作树 10 个假改动 `git checkout -- .`；**找到 18:35:31 的写者**（候选：异地同步 / `dpc-sync` 一类批量拷贝；判据 = 它写过的文件 mtime 同秒、内容 CRLF）并让它保字节，或在源机 `git config core.autocrlf false`；仓内加 `.editorconfig`（`[*] end_of_line = lf`）兜编辑器一档。**验收**：连续两个工作日开工时 `git status` 无 CRLF 假改动。
- **T0.3 下游**：gen-model 4 处 `TextList` 臂随其工作树归属者提交；不由本仓改。
- **T0.4 文档**：README「Not yet」句与上期 §1.3 表对齐（S6b 读侧已落、写侧仍留；`Transaction` 已在）；上期计划状态行加「第二期见本文」。

### T1 · 消费者门面（3 天上限；按「真项目第一天会撞到」排序）

#### T1a `Transaction::move_to` / `reorder` / GROU 引用成员（半天–1 天）

- `move_to(refno, new_owner, MemberPosition::{Last, After(RefNo), First})`：改记录头 `owner`、旧 owner 表摘（`remove_member`）、新 owner 表插（`push_member` / `move_member_after`），两侧 owner 都进批（在批里就改 pending draft，不在就从 base 读来当 update——与今天 insert 的 owner 处理同一条路）；claim 两侧主元素都冒会话；类型 16 / 17 表不动（引用没变）；`reorder(owner, refno, after)` 只动一张成员表。GROU 一类 `member_list_kind == WholeList` 的 owner：`add_reference_member` / `remove_reference_member` 改的是引用列表，不改被引用元素的 owner。
- 合并侧已有依据：A3b 第 3 步 `sub_5B211C0` 换 owner 到提交侧 owner 表摘成员（上期 §7 2026-09-21 条）、`sub_5B20CA0` 加成员门——门面只是把「写者一侧」的三处改动收成一步。
- **验收**：`tests/write_stage6_transaction.rs` 加 2 条：ams8000 副本把一个 BOX 从 ZONE A 挪到 ZONE B 尾 → BOX owner = B、A 表少它、B 表末尾多它、`diff` 恰改 3、claim 两侧 SITE 会话冒；`reorder` 把成员挪到首位 → 只 1 个 modify。TTY 门加 Q17 `Q OWNER` + Q18 `Q MEMBERS`（顺序逐项）。

#### T1b 按属性名以字符串设值 + 字典口径（1 天）

- `Transaction::set(&mut draft, template, "DESC", "反应堆厂房")`（或 `RecordDraft::set_attribute_str(template, attlib, name, &str, text_format)`）：名字 → `db1_hash`；`attlib` `data_type` + 描述子存储类型 → 目标 `DescriptorValue` 变体：2/6 → `Real` / `RealArray`（空白或逗号分隔）；3/7 → `Int` / `Bool`（`true|false|1|0`）/ `Word`（大写 4–6 字母词，查 `word_name` 反表）；4/8/16/17 → `Ref` / `RefArray`（`=24384/26484` 或 `/NAME` 经 `find_by_name`）；10/15 → `Text`；19 → `TextList`（`|` 分隔或 JSON 数组）；`unset` → `Unset`。解析失败 → `TransactionError::Parse { name, input, expected }`，不动 draft。
- **字典口径**：`SchemaPack` 加载 `noun_attr_fields.json` 成 `DictionaryLimits`——① 属性不在导出里 → `NotInDictionary`（`RATM` / `UNIPAR`）；② 文本按 **`SIZE` 字符**上限与模板字节容量**双卡**（`DESC` 120 字符 / 480 字节）；③ 数组元素数按 `SIZE`；④ 例外表 `SET_ONCE = [UUIDIA]`、`PSEUDO = [USRWEI]`（TTY 门 oracle，表驱动、可加）。默认拦，`WriteOptions.dictionary: Option<&DictionaryLimits>` 为 `None` 时行为与今天一致。
- **验收**：单测覆盖每种存储类型的字符串 → 值 → `set_attribute` → 读侧解回相等；`ams8000` 副本上 `set("DESC", 121 个字符)` 拒、120 个过；`set("RATM", …)` 拒 `NotInDictionary`；TTY 门 Q19：用字符串路径写的 EQUI `DESC` / `POS` / `ROKEYS` 与 E3D 打出的逐字相等。

#### T1c 以 base 打开 + 提交时合并（半天–1 天）

- `Transaction::open_at(path, SessionSelector, &mut templates)`；`commit` 时若 latest.sesno ≠ base → 改走 `write_merge_session_with(base_sesno, …, MergeOptions { templates, extract_flush: false })`；冲突（561 / 562 / 590）→ `TransactionError::Conflicts(Vec<MergeConflict>)`，不写一字节。**同属性 / 同 owner 撞车**按 Q2 定：推荐门面默认 `OnClash::Refuse`（回报撞车清单，让调用方选），`OnClash::WriterWins` 可选 = core 普通 `SAVEWORK` 口径。`Moved`（stage 与 commit 之间又动了）→ 门面自动重开 latest 重做**一次**，再 `Moved` 才上抛。
- **验收**：`tests/write_stage6_transaction.rs` 加 3 条（复用 stage3 的 A / B 构造）：B 以 S 开 tx、A 先提交 S+1 不相交改动 → B commit 成 S+2 = A∪B；A / B 同改一属性 → `Refuse` 拒且字节不变、`WriterWins` 留 B；A 删了 B 要改的元素 → 562 拒。TTY 门 Q20：E3D 在我们 open 之后先存一次，我们再 commit，E3D 两边都读到。

#### T1d 批量指令 CLI `e3d-write`（半天–1 天）

- 新 bin `src/bin/e3d-write.rs`：`e3d-write apply --db <file> --schema schema\e3d31 --ops ops.json [--base N] [--on-clash refuse|writer-wins] [--dry-run] [--user U --comment C] --report report.json`。`ops.json` = 数组，每条 `{op: insert|update|rename|delete|move|reorder, owner, noun, name, refno, attrs: {"DESC": "…", "POS": "1000 2000 3000"}, after}`，全部走 T1a / T1b / T1c 的门面，一个会话提交；`--dry-run` 只解析 + 字典校验 + 冲突扫描不写；`report.json` = `CommitReport` + 每条指令铸出的 refno + 拒写原因。
- 它是消费者接入前的可脚本入口（Q1 无论选谁都要），也**接替 `e3d-write-box` 当 TTY 门的夹具**（box 保留到新门跑绿）。
- **验收**：`tools/e3d/write_gate.ps1` 改由 `e3d-write` + 仓内 `tools/e3d/write_gate_ops.json` 驱动，原 16 问 + T1a/b/c 新增 4 问全 PASS；`--dry-run` 对同一文件字节不变；错指令（属性名打错、refno 不存在、owner 已删）各回一条可读错误且文件不变。

### T2 · 规模门（1 天；与 T1 无依赖，可并行）

- 探针（`%TEMP%`，不入库；数字进 §7）：ams8000 副本上**一个会话** insert 10 000 个 BOX 到 1 个 ZONE（成员块跨 ~20 页）+ 1 000 个 update + 100 个 delete；量：耗时、`appended_pages` 拆成记录页 / 索引 CoW 页 / 表页 / claim 页、峰值 RSS、文件增长；再 10 个会话各 1 000 → 看会话链膨胀（CoW 永不复用页）。E3D 读回：TTY `Q MEMBERS` 计数 + 抽 20 个 `Q ATT`；E3D 打开该库的耗时前后对比。
- 已知的可疑点先列出、有数字再定：索引 CoW 每条 insert 复制根到叶一条路径——批内同叶多次 CoW 是否只写一次（`rewrite_index` 是按批还是按条）；成员表 10 000 项的编码 / 三方 diff 是 O(n) 还是 O(n²)（`member_list_diff` 双游标应是 O(n)）；`Transaction::open` 扫全索引求 max word1 的开销随库大小（ams8000 6 639 元素几毫秒，百万元素库要量）。
- **验收**：§7 有一张数字表 + 阈值提案（Q4）；超出预期的项各开一条优化，不在本计划内做。

### T3 · SPAMAP 缺失后果实测（半天；放 T0 之后第一件，最便宜、信息量最大）

- 问题：e3d-io 写的 BOX / EQUI 不进 SPAMAP。已证：`Q ATT` 全对、导航可达。**未证**：① E3D 3D 视图能否画出它（`ADD CE` / `AUTO CE`）；② 依赖空间图的功能能否命中（体积选择 / clash / `WITHIN`）；③ E3D 自己 `SAVEWORK` 后会不会补上 SPAMAP 键（09-20 调研只看了值的形状——世界系 AABB + 标志、键含空间格 hash）。
- 做法：TTY 门副本（`D:\e3dio-gate`）上：(1) TTY `ADD CE` + 一条体积选择宏对我们的 BOX；(2) E3D 改一个无关属性 `SAVEWORK` → 用读侧的键控表读法（09-20 探针 `%TEMP%\s2probe spamap`）看 SPAMAP 表里有没有 BOX 的键；(3) 一次人工 3D 视图确认（截图进 evidence）。
- **产出与决策（Q3）**：若 ③ 会补且 ① 能画 → SPAMAP 留待关掉、写清「E3D 下次存盘自补」；若 ① 不画或 ② 不命中且 ③ 不补 → **SPAMAP 升为第三期第一件**（几何 AABB 来自 e3d-model，空间格 hash 从 core 读实），另起计划。

### T4 · 验证加固（半天–1 天）

- TTY 门 `FORMAT 0`：把 TES000 的 MDB / 用户接进 `l3_suite --check-driver`，`TES023` 副本上跑中文 NAME / DESC 三问（今天口径靠 E3D 自写的键 `/&~9xB/5:Ih18 &` 逐字节相同，缺一次 E3D 亲口读回）。
- Q17 自动化：门里起一个写模式 E3D（`DOPENW` 宏）再让 `e3d-write` 写 → 期望 `Locked` 且字节不变；E3D 退出后同一写成功。
- 交替写 5 轮：E3D 写 → e3d-io 写（以 E3D 会话为 base）→ E3D 写 → …，每轮两边都 `Q SESSION` / `Q ATT`，会话链 `BASESN` 逐段正确。
- `ams1112_0001` 族：换到本机存在的样本（`D:\AVEVA\Projects\E3D3.1` 9 个项目目录）或删，让「SKIP 即 ok」的十几个测试重新有断言。

### C · 留待（不排期，只写触发；较上期只改一处）

| 项 | 触发条件 |
|---|---|
| SPAMAP | **改为按 T3 结果定**（见上） |
| S6b 页 1 桶表写侧 | 需与 E3D 交替写同一扩展且号段不容空档，或要把「删号不回收 + 保留量」对齐到字节（`updateBucketList` 存盘重算，`max+1` ≈ 收敛值） |
| UDA 表 UDATAB / UDAFTB / UUIDAR | 拿到一个带这三张表的项目库 |
| 类型 17 逐槽三方 + 576 | 出现两边并发改同一数组的场景 |
| extract 模型 / Central / 多 extent / compact 记录 / E3D 2.1 / 部分存盘 | 需求出现再议（语料 940 694 条叶值 `flag` 全 1 = 全单 extent；ams8000 6 639 元素全 wide） |
| `AttrValue` 加 `TextList` | 不做（旧摘要枚举） |

---

## 3. 顺序与依赖

**T0**（半天）→ **T3**（半天；结果只影响第三期，不阻塞 T1）→ **T1a → T1b → T1c → T1d**（3 天上限）→ **T2**（1 天，可与 T1 并行）→ **T4**（半天–1 天）。

- T1d 依赖 T1a / T1b / T1c（CLI 只是门面的壳）；T1a / T1b / T1c 互不依赖，可并行。
- T2 只依赖现有门面，出的数字若指向索引 CoW 或成员表，优化排第三期。
- T3 独立；它的答案决定第三期第一件是 SPAMAP 还是继续门面。
- 估时按上期 §7 的实际节奏（S4 一天、S5 一天、S8 + S6 + S7 一天）给，是上限。

---

## 4. 风险

| 风险 | 影响 | 处置 |
|---|---|---|
| T1c 的 `OnClash::Refuse` 与 core 普通 `SAVEWORK`（写者赢）**有意不同** | 用 e3d-io 的人以为和 E3D 一样 | 文档写清、`WriterWins` 一开就是 core 口径；默认拒是因为 UI 消费者静默覆盖别人的改动比重试贵 |
| T1b 字典口径的例外表靠 TTY 观察 | 漏一条 E3D 才报 | 表驱动可加；`--dry-run` + TTY 门每加一条就加一问 |
| T2 可能暴露 CoW 页 ∝ N · 树高 | 万级 insert 一个会话追加几千页 | 数字先出，优化第三期；E3D 自己也是 CoW，膨胀由 `MERGE CHANGES` 收 |
| 18:35:31 的写者找不到 | CRLF 假改动继续每天出现 | `.editorconfig` + 开工 `git checkout -- .`（现状），不阻塞 |
| `main` 工作树 0.3.0 WIP 无人认领 | `main` 与 `write-path` 分叉、下游吃不到收敛 | Q6 由用户指定；本计划不动那份 WIP |
| T3 需要一次真 E3D 图形会话（TTY 驱动不画图） | 自动化不了 | 人工一次 + 截图进 evidence，够定决策 |
| 活样本继续漂移（E3D 每开一次样本项目就写） | 依赖样本状态的断言红 | 上期 S2 口径：沿链找或在副本上构造 |

---

## 5. 决策（2026-09-22 评审：Q1–Q6 全部按推荐项定）

- **Q1 首个消费者形态——已定**：本仓先做 Rust 门面 + `e3d-write` CLI（T1），HTTP / UI 接线由消费者仓做。（备选未取：直接在 gen-model 加 HTTP 写接口；`plant-model-2` 起新仓。）理由：`e3d_io::write` 今天 0 个下游引用，先把门面做到「不看源码能用」，接线谁做都一样。
- **Q2 提交时库已前进、且同属性撞车——已定**：门面默认 `OnClash::Refuse`（回报冲突清单），`OnClash::WriterWins` 可选 = core 普通 `SAVEWORK` 口径。
- **Q3 SPAMAP——已定**：按 T3 结果定；T3 之前不排几何。
- **Q4 规模门阈值——已定口径**：T2 出数字后定；提案 1 万 insert 单会话 ≤ 10 s、追加页 ≤ 1.5 × 记录页、`Transaction::open` 在最大装机库 ≤ 1 s。
- **Q5 CRLF 根因处置——已定**：找 18:35:31 的写者并让它保字节；`.editorconfig` 同时加上兜编辑器一档。
- **Q6 `main` 工作树 0.3.0 WIP——已定**：本会话验收并提交（T0.1），再 fast-forward `write-path`。

### 5.1 评审补充（接班会话 fable-5-1-12，2026-09-22；每条都核到代码行，方案与 §3 顺序不变）

1. **T1c 的 refno 发号必须扫 latest，不能扫 base。** `Transaction::open` 今天在所开会话的索引上求每扩展 max `word1`（`transaction.rs:132`）；`open_at(base)` 若沿用同一扫描，base 之后别人建的元素号会被再发一次，直到提交才撞 590。`open_at` 要另开一个 latest 引擎求 `next_word1`（或取 base / latest 之大者），验收加一条：A 在 S+1 建 BOX，B 以 S 开 tx 再 insert → 号不撞、merge 成功。
2. **T1c 的 `Moved` 自动重试基本是理论项。** S7 起 staging 全程持哨兵锁，`Moved` 只可能来自不认锁的写者；重试一次可以留，但不为它写门。重试 = 同一 `write_merge_session_with(base_sesno, …)` 再调一次——草稿已带 base 字节，不需要重放构建闭包。
3. **T1a 的「claim 两侧都冒」不用另做。** 换 owner 时旧 owner（摘成员）与新 owner（插成员）都作为 Update 进批，`plan_claim_edits` 对每个 `Touched` 各爬一条 owner 链，两侧主元素自然都 Refresh；T1a 只需保证两侧 owner 都进批。`RecordDraft.owner` 是 `pub`（`write/record.rs:113`），`move_to` 直接改；`remove_member` / `push_member` / `move_member_after` 都在（`:254` / `:260` / `:270`）。
4. **T1b 的 WORD 反表要一份。** 读侧 `word_name(raw)` = `lookup_system_name(raw)` ∪ `db1_dehash(raw)`（`record/descriptor.rs:698`）；字符串 → 字：db1 可哈希的词直接 `db1_hash`，系统名那张表要反向映射（表在 e3d-attlib 侧）。`noun_attr_fields.json` 在 `schema/e3d31/catalog/`，`SchemaPack.catalog`（`schema.rs:640`）已有目录位可挂 `DictionaryLimits`。
5. **T2 的「按批还是按条」已有答案。** `index_cow::rewrite_index` 收整批 `&[LeafEdit]`（`index_cow.rs:104`），模块头明说「共享页只复制一次」（`:13`，`by_child` 分桶下钻 `:239`）；剩下要量的是 CoW 页数 ∝ 触及的叶数 × 树高，以及 `Transaction::open` 的全索引扫描随库大小的开销。
6. **T3 (2) 读 SPAMAP 要一把小工具。** `index::reference::ReferenceTable` 只认叶 `value_dwords == 0`（`reference.rs:326`；`value_dwords: 7` 在它的测试里是拒收用例 `:764`），SPAMAP 是 key 4 / value 7 的有值表；用 `index::IndexPageView`（通用页解析，带 `key_dwords` / `value_dwords`）在 `%TEMP%` 写探针走叶即可，09-20 那只若已不在就重写。
7. **T4 第 17 问值半天不值一天。** `tests/write_stage7_lock.rs` 已是它的自动化等价（持锁则 `Locked` 且字节不变）；门里再起写模式 E3D 只是把「锁字节与 E3D 同一」从 IDA 证据变成实测。

另：T0.2 的写者复核见 §7（是 Cursor 本身，不是同步工具）。

---

## 6. 不做

见 §2 C。另：不做 HTTP 层；不做 E3D 2.1 写（MISSION 明示）；不做性能优化直到 T2 有数字；不改 gen-model / e3d-model 一行（T0.3 归其归属者）；不动 `main` 工作树的 WIP（Q6 之前）。

---

## 7. 观察记录（随切片推进填写）

- 2026-09-22：计划起草；§1 数字全部来自当日实跑（`cargo test --no-fail-fast` 96 / 594 / 0 / 30 ignored，32 s；`cargo clippy --all-targets` 0 error、lib 22 warning 无一在 `src/write/`）。`noun_attr_fields.json` 实读：`DESC SIZE=120`、`NAME 500`、`UUIDIA 4`、`DESP 100`、`ROKEYS 20`、`POS 3`；`RATM` / `UNIPAR` 不在导出；`USRWEI` 在（`ITYP=48`）。
- 2026-09-22：**评审通过（Q1–Q6 全按推荐项），T0 收口。**
  - **T0.1 0.3.0 已由做 WIP 的会话提交**：`main` `ae9cdfd`（20:21:05，`chore(release): 0.3.0`），`write-path` 同分钟 fast-forward 到同头。本会话在 `main` 工作树**独立重跑**验收：`cargo test --no-fail-fast` **96 个测试二进制 594 passed / 0 failed / 30 ignored**，`--lib` 355 / 2 ignored（137 s 含全量重编译）；`cargo clippy --all-targets` 0 error、lib 22 warning（与 `write-path` 基线同）。被收 `pub(crate)` 的项在本仓 `tests/` / `examples/` / `src/bin/` 里 `rg` 0 命中，故只可能是下游编译错、不会静默错值；下游 7 crate `cargo check` 0 error 由提交消息记账，本会话未重跑。
  - **T0.2 CRLF**：写工作树 10 个假改动 `git checkout -- .` 还原，`git ls-files --eol` 回到 `w/lf`；`main` 工作树 `src/meta/bucket.rs` / `tests/bucket_table_real.rs` 同样只是 CRLF、同样还原。**写者画像**（本轮实测）：`D:\work\plant-code\old` 下 **193 个文件** mtime 落在 18:35:00–18:36:00，跨 **10+ 个互不相关的仓**（`plant3d-web\src` 49、`vendor\e3d-io-write` 28、`gen-model-model-cache\src` 20、`plant3d-web-e3dlook\src` 10、`e3dlook-shots\2026-09-21` 7 张截图 …），`.git` 目录里无一文件在那一分钟被动、两个工作树 reflog 在 18:35 无记录——**不是 git、不是编辑器（连 PNG 都写了），是一个排除 `.git` 的整目录同步 / 拷贝工具，来源侧工作树是 CRLF**。此刻没有同步类进程在跑、无计划任务指向 `D:\work`；`vendor\dpc-sync` 只是 chunk 归档库（无 EOL 逻辑、无已编译二进制），不是它。git 侧另有一条线索：系统级 `E:/Program Files/Git/etc/gitconfig` `core.autocrlf=true`，用户级 `~/.gitconfig` `false`——本机被后者压住，来源机若只有系统级那条，其工作树天然 CRLF。**工具名要用户指认**（它不在本机可见处）；仓内已加 `.editorconfig`（`end_of_line = lf`）兜编辑器一档。
  - **T0.2 复核（接班会话 fable-5-1-12，20:45）：写者是 Cursor 本身，不是同步工具——上一条的「整目录同步」推断不成立。** 判据（本机实测）：① `D:\work\plant-code\old` 下那 190 个文件（排除 `.git` / `target` / `node_modules`）的 mtime **全部落在 18:35:31.401–18:35:31.964 这 0.56 s 里**（另 1 个 18:35:32），不是「一分钟内」；② Cursor 主进程 `Cursor.exe`（PID 90036）`StartTime` = **18:35:22**，各窗口 renderer 18:35:23–:34 陆续起，主进程在 **18:35:31.785 / .795 / .804** 连起三个 `--utility-sub-type=node.mojom.NodeService` 子进程——与文件写入窗口重合到毫秒级；③ 本机无重启（`LastBootUpTime` 09-03）、System 日志 18:30–18:40 无电源 / 恢复事件、当前无任何同步类进程；④ `e3dlook-shots\2026-09-21` 里被写的 7 个是 `.pmlmac` / `.py` / `.ps1` 脚本，**不是截图**，所以「连 PNG 都写了」不成立，「写者不是编辑器」的推理随之失效；⑤ 190 个文件跨 12 个仓，正是各会话在 Cursor tab 里打开过的那批（含 `MISSION.md` / `examples/*.rs`）。结论：Cursor 18:35:22 重启，启动第 9 秒把恢复的编辑器 / 会话状态里的文件内容整批写回盘（具体是哪一子系统——热退出恢复还是 agent checkpoint 回放——从外面看不出，也不必），其中缓冲仍是 CRLF 的 12 个带回了 CRLF。处置（Q5「让它保字节」）：Cursor 用户设置 `%APPDATA%\Cursor\User\settings.json` **加 `"files.eol": "\n"`**（此前未设 = `auto`，Windows 上新缓冲默认 CRLF；已加）；`.editorconfig` 照旧兜一档；**不必再找同步工具**。验收口径不变：两个工作日开工 `git status` 无 CRLF 假改动；若再现，先看 mtime 是否又与 `Get-Process Cursor` 的 `StartTime` 同秒。
  - **T0.3** gen-model 4 处 `TextList` 臂：归其工作树归属者，本仓不动。
  - **T0.4** 上期计划状态行末尾指向本文；README 「Not yet」（第 23 行）与「Not written」（第 230 行）已与 §1.3 表一致（0.3.0 提交顺手改过），未再动。
  - **下一步：T3**（SPAMAP 缺失后果实测），随后 T1a。
