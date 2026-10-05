# 第 13 组续：S6b 页 1 原地写（13.3–13.5）

## 摘要与审阅边界

规划基线：`24cc0e8`（与 github `write-path` 同步）。2026-10-05 22:22 用户确认接受「page 0 不再是唯一提交点」（需求 13 第 4 条），要求另立 13.3–13.5 方案。本文件只定设计、需求与设计修订、实施步骤；定稿前不改 `src/`、不跑真实门、不勾任务、不提交。2026-10-05 23:18 用户审阅定稿，无修改；同时折入证据文档补采回合的两个结论（SAVEWORK 写序、claim 生命周期，见「已定事项」与「证据边界」）。

范围：13.3（`src/write/bucket.rs` 与 commit 接入）、13.4（Property 27 测试）、13.5（Q15），连带修订需求 13 第 6 条、design S6b 一节与 Property 27，新建 ADR 0003（已起草，随本方案一起审）。不做：第 14 组文档（14.2 README 页 1 一条、14.3 CHANGELOG 按第 14 组任务写）、10.5/10.6；不增加依赖；不改无关目录。

## 已定事项

| 事项 | 结论 | 来源 |
|---|---|---|
| 落盘方式 | 原地改写物理页 1；E3D 发号只读物理页 1，新页 1 由 page 0 指向对 E3D 无效 | 13.2 Evidence（`b8faf7a`）；tasks「页 1 原地写的崩溃一致性影响」 |
| 写序 | 先写页 1 并 sync，再写 page 0 并 sync；崩溃最多空掉一段号 | 同上，七种情形表；证据文档补采回合：E3D SAVEWORK 逻辑序相同，两页同一次刷盘 |
| 第三字 | 原样保留，只抬计数器 | 同上；用户随「接受」一并确认 |
| 计数器含义 | `alloc.counter_semantics = NextAvailable` 已坐实，Bucket_Floor 即计数器本身 | `src/write/alloc.rs`、core_rules |

## 设计

### 页 1 写在 commit 里

现状：`StagedSession::commit`（`src/write/mod.rs:389`）先重读 page 0，`LSPGNO` 不符即 `Moved`、什么都不写，再写 page 0、sync。`rollback`（`:416`）只把文件截回暂存前的长度；页 1 在原长度以内，若在暂存阶段写，rollback 恢复不了。

方案：页 1 在 `commit` 里写，位于 `Moved` 检查之后、page 0 之前。

```text
commit:
  1. 重读 page 0；LSPGNO 不符 → Moved（页 1 未动）
  2. 暂存带有页 1 新字节且与原字节不同 → write_page(1) → sync
  3. write_page(0) → sync
```

- 拒写、rollback、dry run（`e3d-write --dry-run` 即暂存加 rollback）、`Moved` 及其重试都不碰页 1，失败的写仍逐字节不变；暂存后、commit 前按崩溃读文件，已有页也不变。
- 只有 commit 内「页 1 已写、page 0 未写成」（I/O 失败或崩溃）时，计数器已抬、会话未提交：空掉一段号，读侧仍见上一会话。
- 改页 1 的提交多一次 sync。这次 sync 不能省：没有它，操作系统可以先把 page 0 落盘，等于退回「先 page 0 后页 1」那种有复用窗口的顺序。E3D 自己不在两页之间刷盘（证据文档补采回合），它靠离开写模式时的一次刷盘；e3d-io 每次提交重开文件，这次 sync 是 e3d-io 的加固。
- 因此 tasks 崩溃一致性表「失败的写不再逐字节不变」一行收窄为上面第二条（见修订）。

### 页 1 新字节

- 来源：暂存时在哨兵锁内读到的物理页 1。锁从 `stage_batch` 的 `lock_for_write` 一直持有到 commit，锁内读到的就是权威值；没有 `LockFileEx` 的平台仍由 `LSPGNO` 复核兜底。commit 不重读页 1，因而不增加新的失败方式，`WriteError`（未标 `#[non_exhaustive]`）不加变体。
- 逐条只改计数器字：`new[E] = max(原计数器[E], required[E])`。条目顺序、条目数、第 2 字、第三字和页内其余字节原样保留。页 1 不变就不写，也不多 sync。
- 只处理页 1 已列出的扩展；主扩展与未长过的次扩展不新增条目（需求 13 第 3 条只抬表中已有的扩展；E3D 何时新增条目未取证）。

### required 的算法

`required[E] = 1 + max(pinned 索引中扩展 E 的最大 word1, 本批最终插入扩展 E 的每个 word1)`

- pinned 索引：直接提交时是 base；Merge_Commit 时是 Merge_Latest（`stage_batch` 在锁内 `read_base` 所得）。物理页 1 没有历史版本，锁内读到的就是 Merge_Latest 对应的页 1，满足需求 13 第 10 条。
- 本批最终插入：取重铸之后的 refno；被重铸替换掉的原 refno 不计入（需求 13 第 10 条）。原 refno 之所以被重铸，是因为它已经在 Merge_Latest 的索引里，会经由第一项计入，两者并不冲突。
- 索引最大值沿用 Open_Fast_Path 的 `last_before`，每个扩展 O(树高)；页 1 通常只列 0–2 个扩展。本批没动过的扩展也要算：旧版 e3d-io 写过、计数器没跟上的库（历史欠账），在新版第一次提交时一并修好。
- 删除不降低 required：取删除前的 pinned 索引，偏保守，也覆盖本批刚删掉的号。

### 规则门控（D7）

`bucket.collision = HistoricalReuse`、`bucket.persist = InPlace`、`alloc.counter_semantics = NextAvailable` 三条都坐实才写页 1；任一未坐实就走 D7「不写页 1」（design 现行默认）。打开时页 1 解析失败（`AllocationBasis.counter = Unavailable`）同样不写，现有报告已能说明原因。

### 模块落点

- 新建 `src/write/bucket.rs`（`pub(crate)`）：纯函数 `raise(page1, required) -> Option<Vec<u8>>`（页 1 不变返回 `None`），外加 required 的计算；不依赖 commit 细节，便于 proptest。
- `StagedSession` 加一个私有字段携带页 1 新字节，只由 `Transaction::stage_batch`（直接与合并两条路径）在锁内填入。自己铸号的低层暂存 API（`stage_element_session`、`write_merge_session*` 等）不填、不维护页 1：需求 13 第 3 条只约束 Transaction，低层调用方的字节级场景与金样保持不变；这一点写进这些 API 的文档。
- 公开 API 不变，不加报告字段：13.4 与 Q15 都按需求只用读侧 `BucketTable::parse` 读回。
- 同步改两处模块头：`src/write/mod.rs` 的「已有页从不改写，page 0 是唯一提交字」改为页 1 例外及写序；`src/meta/bucket.rs` 的「`updateBucketList` 在存盘时重建」与「写侧尚不维护」两句按 13.2 证据更正（重建只由 ADMIN 重配置器调用；SAVEWORK 原地推进计数器）。

## 需求与设计修订（随第一个提交）

1. 需求 13 第 6 条：「每条第 3 字为 0」改为「每条第 3 字与写之前盘上的值相同」，附理由：按 A9，e3d-io 拿到独占锁时盘上的非零第三字只可能是崩溃的 E3D 会话残留，清零等于替 E3D 回收 claim。
2. design：Property 27 的第三字一句同步修改；S6b 一节把「撞号，且 E3D 原地重写页 1：先停下」改为已确认，并补上本方案「设计」一节的要点。
3. tasks：崩溃一致性表「失败的写不再逐字节不变」一行按上文收窄；节标题的「需用户确认」改为确认日期；「取证门槛核查」的页 1 行填首个写侧提交与父提交；13.3 描述补「只由 Transaction 维护」。
4. ADR：`docs/adr/0003-page-1-in-place-bucket-write.md`。
5. GLOSSARY：不新增词条（Bucket_Table、Bucket_Counter、Bucket_Floor 已有）；审阅中出现新概念再补。

## 实施步骤

### 13.3 `src/write/bucket.rs` 与 commit 接入

1. 动手前核对取证门槛 G1、G2（需求 2 第 1、2 条）：首个写侧提交的父提交须已有 Evidence_Doc（`docs/evidence/2026-10-05-s6b-bucket-evidence.md`）、不在 `src/write/` 下的读侧（`src/meta/bucket.rs`）与以 Corpus 为输入的测试（`tests/bucket_table_real.rs`），结果填进 Spec_Ledger「取证门槛核查」。
2. 实现 `bucket.rs`、`StagedSession` 字段与 commit 写序；Transaction 的直接提交与 Merge_Commit（含重铸）两条路径都在锁内填页 1 新字节。
3. 单测：只升不降；条目、第三字（含非零）、其余字节保留；页 1 不变返回 `None`；未列出的扩展不新增条目；规则未坐实时不写。

### 13.4 `tests/write_stage10_bucket.rs`（Property 27）

- 库副本 proptest（沿用 `DbCopy::of`），四个 Corpus 库：`ams8000_0001`、`ams251142_0001`、`TES023`、`ams5100_0001`（最后一个页 1 无条目，断言页 1 永不改）。批次覆盖：插入列出的扩展；只插主扩展；只改名、改值、删除；Merge_Commit 含重铸；历史欠账（先用不维护页 1 的低层路径让计数器落后，再经 Transaction 提交）。
- 只用 `BucketTable::parse` 读回并断言：第 2 字 = 2 + 3n；条目集合与顺序不变；第三字与提交前相同（含人为置非零的副本）；计数器不低于上一会话；Bucket_Floor 大于新索引与本批每个 word1（重铸后的号计入）。
- 失败路径：rollback、dry run、`Moved` 重试、拒写之后页 1 逐字节不变；`commit_observed` 的回调里（暂存后、commit 前）页 1 未变。
- 「页 1 已写、page 0 未写」窗口：在副本上按 `raise` 的结果手工写页 1、不写 page 0，断言读侧见上一会话、再次经 Transaction 提交正常且计数器不回退。

### 13.5 Q15（`tools/e3d/alternate_write_gate.ps1`）

- 场景：复用 `s6b_probe.ps1` 的 Deleted 例与 `s6b_checks.ps1` 的判分。新 scratch 副本 → 用 `next-word1` 读扩展 24384 的全索引最大值 M 与 Bucket_Floor C（前提 M < C，否则输出无法判定）→ e3d-io 一次插入 K = C − M 个元素（保证在 `c61b097` 的「索引最大值 + 1」发号下也覆盖 C）→ 下一个 e3d-io 会话全部删除 → E3D `NEW` 一个该扩展的元素并 SAVEWORK → 判定：E3D 所得 word1 等于 e3d-io 提交后页 1 的 Bucket_Floor，且不在 e3d-io 已发集合内。
- 预期：本方案下 e3d-io 发 C..C+K−1，页 1 抬到 C+K，E3D 得 C+K → PASS；`c61b097` 上 e3d-io 发 M+1..C，页 1 不动，E3D 得 C，与已删号相同 → FAIL。
- `c61b097` 上实跑一次（需求 13 第 8 条）：另起 worktree，用独立 `CARGO_TARGET_DIR` 构建 `c61b097` 的 `e3d-write`；脚本加「只跑 Q15」与「指定 e3d-write 路径」两个参数；旧 CLI 没有 `--spatial`，段内按二进制能力省掉该旗标。Q14 的证据里没有 `c61b097` 实跑记录，Q15 的 FAIL 日志并入证据。
- Q15 放在 5 轮之后、用独立副本，不改 T4a–c 的期望；5 轮不回归（需求 13 第 7 条）。K 很大时沿用第 12 组的 staging 规模能力并记录耗时；超过门超时就改选 C − M 小的扩展并写明。

## 金样与测试适配

- 末文件 SHA 只会在两类批次里变：插入页 1 列出的扩展的批次，以及库本身有历史欠账的批次。按 11.4（F5）的做法：`off_compat`、`write_hashes`、`allocation_floor`、`staging_scale` 原文件一字不改，受影响的键精确退役并锁旧 SHA，新值录进新文件（暂名 `tests/golden/bucket_write.json`，两遍录制一致），逐键记入 Spec_Ledger「Golden 剔除键」。按需求 15.3，页 1 字节由动态发号决定，不作字节兼容输入。
- 只走低层 API、或不插入列出扩展的键预期不变；实施时先全量跑一遍金样，按实际变化清点，不按预估。
- 测试适配预计只涉及断言「提交后除 page 0 外已有页不变」的测试（如有）；rollback、dry run、`Moved`、暂存后按崩溃读这几类按本设计不用改。逐条记入「测试适配清单」。

## 验证与提交

1. 完整 Cargo 零失败，30 ignored 与 skip 7 分列不变；clippy/doc 不超过 63/25。
2. 真 TTY 39 问（含 Q14）全 PASS。Q14 在扩展 24384 插入，提交后页 1 会抬；Q14 只比对 e3d-io 的首个 word1，不受影响，但之后 E3D `NEW` 得到的号会变高，要逐问核对有没有依赖 E3D 具体号的期望。
3. alternate 5 轮加 Q15 PASS；`c61b097` 上 Q15 FAIL 的日志入证据；原样 FORMAT0 PASS；源 AMS/TES 的哈希与长度不变。改页 1 的提交多一次 sync，在 `staging_scale` 与门耗时里记录前后对比。
4. 提交拆分：
   - 第一提交 `docs(spec): 13.3-13.5 accept in-place page-1 write`：ADR 0003、需求 13 第 6 条、design 与 Property 27 修订、tasks 崩溃一致性一节与门槛核查、本方案。
   - 第二提交 `feat(write): 13.3-13.4 maintain page-1 bucket counters in place`：`bucket.rs`、commit 接入、两处模块头更正、`write_stage10_bucket.rs`、金样退役与新文件、测试适配清单。父提交即第一提交，满足 G1、G2。
   - 第三提交 `test(gate): 13.5 add Q15 bucket floor gate`：Q15、`c61b097` FAIL 证据、回归归档、tasks 13.3–13.5 勾选。
   - 推送：做完回报时再问。

## 证据边界与触发条件

- 证据文档补采回合已坐实：SAVEWORK 先抬计数器、后写会话与 page 0，两页同一次刷盘；claim 在 NEW 时写入并落盘、SAVEWORK 期间保持、退出时清零。崩溃残留的 claim 很可能在下次登录时由 EXPUNGE 回收（强旁证，登录到 EXPUNGE 的调用链未追）。仍未解：SAVEWORK 之后 page 0 HCCNT 不变，与静态代码不一致。本方案不依赖这几点（e3d-io 只在拿到独占锁时写，抬计数器不动 HCCNT，与 E3D SAVEWORK 之后的观测一致）。
- 页 1 未列出的扩展：本方案不新增条目。若日后出现 e3d-io 插入未列出的次扩展、随后 E3D 在该扩展撞号，作为新的触发条件另行取证。
- 自己铸号的低层 API 不维护页 1，调用方自负发号一致性。
