# 下一阶段：第 11 组发号下界与 Open_Fast_Path

## 基线与目标

以 `0336432` 为开发基线。Q13 自动门及既有回归门已通过；下一阶段完成 11.1–11.9，并接入 Q14，使 10.4 的自动门范围完整。10.5 人工图形门与 10.6 最终检查点仍待办，不实现 staging 性能优化或页 1 桶表写侧。

本阶段必须分开两个行为：Open_Fast_Path 保持当前发号和写出字节；发号下界在后续提交才改变号码。旧 17 条 `off_compat.json` 不用重录来掩盖字节中性优化的错误。新增优化金样单独保存，发号下界确实影响的键按规格办理并逐项记录。

## 当前实现与先决条件

- `Transaction::open_at` 在 `src/write/transaction.rs` 全扫描 base 元素，生成每扩展 `next_word1`；重铸路径又扫描 latest。两处发号需要共用同一 Allocator。
- `IndexCursor` 已有 `seek`、`walk`、页级错误与读取统计，可复用其解码和路径校验实现严格前驱查询。
- `BucketTable::parse` 与 Corpus 桶表测试已存在；不要另写页 1 解码器。
- `alloc.counter_semantics`、`alloc.index_order` 在 `src/core_rules.rs` 均为 Pending。旧 2026-09-22 文档既称计数器为“下一个号”，又描述存盘重建高水位，尚不足以选择本阶段规则；必须补例程与新 TTY 实测，不能依据字段名 `next_word1` 或计数器大于活最大就认定语义。
- 当前源 AMS 库保留至会话 646；规格举例来自更早状态。每个测试从自身页 1 和精确会话索引计算期望，不硬编码 26496 或新号。

## 顺序与提交

### F1：11.1 取证与规则确认

1. 经现有 IB_Wrapper 读取 `db_get_bucket`（0x5aadd90，routine 360）及其实际发号调用路径，确认返回值、计数器更新前后关系。复用既有反编译证据；补齐能够区分“下一个待发”和“已发高水位”的语句。
2. 新增 `tools/e3d/` 计数器探针。每轮从源项目重置独立 scratch，记录登录前、NEW 前、NEW 后及 SAVEWORK 后的页 1、索引最大、`Q REF` 和精确会话；注意登录时可能重建计数器，不能把登录前的磁盘值当作 NEW 时实际使用值。至少运行一个删除后再创建的判别场景，USER 为 e3dio。
3. 核对四库桶表与索引读数，包括页 1 无条目的主扩展。所有值来自本次文件，并记录源哈希、长度、会话与命令。
4. 对全部约定 Corpus 的索引枚举检查严格 `(word0,word1)` 次序，0 反例才确认 `alloc.index_order`。除最新会话外，记录保留会话的检查范围；不能用“排序后没有反例”的结果证明原索引次序。
5. 写 Phase_3_Evidence_Doc，运行 `check_evidence.ps1`；按规则注册方式实现并确认两条规则，补 Ledger。未坐实的规则继续 Pending，不接对应公开行为。

提交：`docs(evidence): 11.1 confirm allocation counter and index order`（包含只读核对、探针和规则实现）。通过条件是例程、TTY 与 Corpus 相互一致；环境失败记日志并保留任务未完成。

### F2：11.3 读侧前驱与诊断

1. 在 `src/index/cursor.rs` 实现 `IndexCursor::last_before(RefNo)`：返回严格小于目标的最大索引键，复用 `read_node`、记录定位和页级错误；不枚举整个索引。
2. 覆盖目标恰在 separator 上、叶首键前驱落在前一子树、目标小于首键、目标大于尾键、缺号、空树、跨扩展和最大 word0。排除 sentinel，错误不得返回 None 冒充不存在。
3. `e3d-descriptor next-word1 --db FILE --extent WORD0 [--session N]` 只读计算并报告精确会话、扫描最大、索引首个可发号、页 1 原值、CounterUse、Bucket_Floor 和最终期望号；计数器未确认或页 1 不可用时明确报告回退原因。所有加一均检查溢出。
4. 用独立的全索引扫描对照 predecessor；检查读取页数随树高增长，覆盖 Corpus 最新及抽样保留会话。诊断 CLI 作为后续 Q14 的期望来源，不引用 `src/write/` 的 Allocator。

提交：`feat(read): 11.3 add index predecessor and next-word1 diagnostics`。

### F3：11.2 优化金样基线

在首个 Open_Fast_Path 写侧提交的直接父提交录 `tests/golden/write_hashes.json`：固定时钟与预铸 refno，沿用现有两遍录制工具，剔除不稳定键并写 Ledger。金样准备提交紧接 F4；中间发生任何其他提交就重新核对父提交并重录新增优化金样。保留 17 条 Off 金样的原内容和哈希。

提交：`test(golden): 11.2 record allocator fast-path baseline`。

### F4：11.4 第一部分，字节中性 Open_Fast_Path

1. 新建 `src/write/alloc.rs`，抽取分配逻辑及 AllocationBasis；本提交仍只用索引 max+1，不启用 Bucket_Floor。
2. 索引次序已确认时，open 不扫索引叶；某扩展首次发号用 predecessor 求最大 word1，缓存下一待发号。最大 word0 使用无溢出的末端查询；没有该扩展时保留原行为。
3. 次序未确认时保留 open 时全扫描，不将扫描延后到首次发号。用独立扫描模型对照两路径。
4. 接入 open、open_at(base) 和重铸；重铸使用已经固定的 Merge_Latest 快照，不另开 latest；保持 Moved 重试从原输入重建，不累计分配状态。
5. 运行金样对照与全套测试，写出字节必须完全相同；有差异先修实现，不修改金样。

提交：`perf(write): 11.4 add byte-neutral lazy allocation seeks`。

### F5：11.4 第二部分及 11.5、11.8，发号下界

1. 核查首个下界写侧提交的父提交、Evidence_Doc 和读侧 Corpus 门槛，写入 Ledger。
2. 语义已确认且桶表可解析时，首个待发号取 `max(base_index_max+1, Bucket_Floor)`。未确认记 Unconfirmed；无法解析记 Unavailable，均回退索引规则。无桶表条目的扩展照样走 seek。
3. open_at 用 base 索引及打开时读取的页 1；重铸用固定 Merge_Latest 索引及提交时读取的页 1。分配本身不写页 1。
4. 添加 `write_stage9_alloc_real.rs`：Corpus 最新会话全量与保留会话抽样，对照读侧 CLI；添加 `write_stage9_alloc.rs`：16 例库副本 proptest及规格 11.5/11.6 示例。
5. 覆盖缺桶条目、坏页 1、未确认规则、计数器低于/等于/高于索引界限、u32 溢出、多次发号、删除后插入、旧 base、latest 撞号重铸及重试。
6. 既有测试只适配确实变化的期望 refno，不改其他语义；与下界实现在同一提交落地。新增优化金样中受行为变化影响的键按规格退役或重录，逐项记录；无关金样继续原样比对。

提交：`feat(write): 11.4-11.5 enforce allocation bucket floors`。

### F6：11.6、11.7，计时与 Q14

1. 复用或最小补齐 Scale_Probe 的 `--open-only`，只测本阶段所需 open 和首次发号。release、热文件缓存、ams7351_0001 连续三次取中位数；记录机器、存储、三次原值和中位数。open 中位数 ≤1 s。不给 staging 优化提前扩范围。
2. 增加 `q14_named.json`，在扩展 24384 创建带名字的元素。CLI 报告解析 refno；调用只读 next-word1，在插入前按相同副本/会话计算期望，再用新 TTY 对名字 `Q REF` 比较。场景必须有 Bucket_Floor > 活最大+1，不能选一个无法区分旧、新发号的状态来判 PASS。
3. Q14 独立标记、只判查询输出；不以命令回显满足期望。保留现有 Q0、28 问与 Q13 原问和期望。
4. 串行跑完整 TTY（含 Q13、Q14）、alternate 5 轮、原样 FORMAT 0 门，分别保留日志；源库运行前后哈希和长度一致。

提交：`test(gate): 11.6-11.7 measure open and validate Q14 allocation`。

### F7：11.9 验收与账本

顺序运行定向发号测试、完整 Cargo、Clippy、doc 和真实门；最终全部 0 失败，新增必需测试无 skip，既有 30 ignored 与样本缺失单列。Clippy/doc 不高于 63/25。更新任务、计划、规则表、取证门槛、金样变动、测试适配、open 计时及门耗时；通过后才打勾。

提交：`docs(acceptance): close milestone F allocation and Q14 gates`。Q14 与本阶段完整门均通过后，10.4 可完成；10.5 与 10.6 保持待办，不将本次自动验收替代人工图形门。

## 本次计划交付状态

已核对任务、需求、设计及现有源码。此文件是第 11 组开发计划，尚未实现 Allocator、修改发号、确认 Pending 规则或执行本阶段 TTY 探针；第 11 组任务未提前打勾。推荐首先执行 F1 与 F2，再录 F3 金样并进入写侧改动。

## F1 执行记录（2026-10-05）

完成桶返回/缓存推进与固定宽索引比较取证，两条规则坐实；原生 TTY 两次 NEW 与中间删除验证 NextAvailable，源库 SHA-256/长度一致。原生 e3dio 登录失败，沿用已有 SYSTEM 驱动，日志 Q USER=SYSTEM、SAVEWORK User=dpc，如实记录差异。独立 Corpus 次序门 1 通过、0 失败、0 ignored，覆盖五库最新/抽样保留 15 会话。读侧前驱/CLI 草稿一并验证 22 项通过（加独立次序门共 23），既有 1 ignored 单列。写侧发号尚未变更。F2 的完整读侧提交随后完成；F3–F7 未验收。

## F2 执行记录（2026-10-05）

F1 提交 `d4f5ab4`。完成 `last_before` / `last`、独立扫描与随机模型、Corpus 最新/历史对照及 `next-word1`；18 项定向通过，0 失败，既有 1 ignored。完整 Cargo 751 通过 / 0 失败 / 30 ignored，既有样本缺失提前返回保留。之后补桶表异常项数校验，先检查页容量再分配，保留原零 extent 停止语义；桶表单元测试 5/0。Clippy 新增闭包 warning 已修，回到 63；doc 25。Off 17 条金样未改。F3 录新优化金样之前不会修改写侧发号。

## F3 执行记录（2026-10-05）

F2 提交 `6232c21`。新增固定时钟、从源索引预铸 refno 的原语批次金样测试，32/128/257 BOX 三副本读回成员关系；两遍通过。新 `write_hashes.json` 保留 21 条，剔除 7 个时间不稳定键（明细见录制日志/Golden 表）；原 Off 17 条 SHA-256 仍 `1b2ac106e9a69a9a5fe677b439ae7c43ef190191dcd8d392b2375a6602e4e80a`。F4 直接以本准备提交为父提交，改写侧前再录一次，断言 21 条内容完全一致，只更新录制提交元数据。

## F4 执行记录（2026-10-05）

直接父提交 F3 `718cf77`，写侧改动前两遍重录的新金样 21 条 entries 完全相同；录制元数据记录该父提交。Allocator 惰性缓存每扩展 max+1，首号经共享 IndexCursor 前驱，open/open_at 不扫描叶或元素；Pending 次序保持 open 时全扫描与原记录校验。重铸接收固定 latest 读引擎，复用 Allocator，不重新查询 latest。新增 `allocation_basis()` 记录根/会话/首号，CounterUse 此阶段为 IndexOnly。定向 25/0；完整 Cargo 755/0/30；Clippy/doc 63/25。旧 Off 17 条哈希不变，新 21 条比对通过。11.4 页 1 下界尚未启用，F5/F6/F7 待办。
