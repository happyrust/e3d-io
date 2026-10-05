# 第 12 组规模基线与剖析

写侧基线 e1425db；本次仅添加探针、采样工具和测试，生产写侧未改。数据位于 `assets/2026-10-05-staging-scale/`。所有写入使用临时 AMS8000 副本，源库 34,037,760 bytes，SHA-256 `3cab5d7d2b2c40fdf0829b91c555d19d55c530dd49860a4665560e5c553fe023`。

## 工作负载和计时

每个副本先用原语准备 1100 个 BOX（不计时），随后同 owner 插入 n 个 BOX、更新前 1000 个 BOX 的 XLEN 300→301、删除后 100 个。Transaction 使用 Require 或 Off；Require 给定由 owner 世界变换和八角计算的 World_AABB。固定时间 2026-10-05 00:00；每轮恢复同一准备副本、完整读取预热，再分别计 open/build/stage/commit。stage 包含落盘与 sync，commit 为 page 0 提交；准备、文件散列及读回检查不计时。

完整读回 owner 成员的次序、每个新增和更新元素的 identity/owner、100 个被删元素的 INDEX 缺失；Require 检查每个目标 SPAMAP 条目和删除条目缺失。探针收口增加 XLEN 属性读回及每条空间坐标逐字解码比对。100k 的成员总量 101124，覆盖 14 位计数饱和边界。

`baseline-{off,require}.json` 是首次无剖析器、release 三轮数据（PowerShell 原始 UTF-16 输出）。之后探针增加门槛字段；首次数据保留原样，不回写。RSS 是 K32GetProcessMemoryInfo 的**进程**历史峰值，包含准备和先前轮次，不冒充独立阶段峰值。

| policy / n | open / build / stage / commit 中位数 ms | 进程峰值 bytes | record / index / claim / TYPEX / SPAMAP / other / session |
|---|---|---|---|
| Off / 10k | 0.4058 / 14.6314 / 90.7039 / 0.3730 | 77578240 | 891 / 94 / 2 / 0 / 0 / 0 / 1 |
| Off / 100k | 0.4954 / 114.4453 / 5680.7904 / 0.5153 | 210472960 | 8170 / 813 / 2 / 0 / 0 / 0 / 1 |
| Require / 10k | 0.2084 / 17.3473 / 1140.5584 / 3.0621 | 78495744 | 891 / 94 / 2 / 0 / 250 / 0 / 1 |
| Require / 100k | 0.3043 / 148.1289 / 36437.9006 / 20.0276 | 311959552 | 8170 / 813 / 2 / 0 / 2274 / 0 / 1 |

Off stage 比值 62.6301，Require 31.9474，均**不通过** ≤15；幂指数约 1.797 / 1.504。Require 10k stage+commit 已低于 10s。10k 非 SPAMAP 988 页 ≤ 891×1.5；SPAMAP 250 页低于 2×ceil(11000/45)+height。12.4/12.5 尚未验收。

## 原生采样与结论（12.2）

WPR `-start CPU -filemode` 失败：`0xc5585011 Failed to enable the policy to profile system performance`；未取消或修改其他会话。替代工具 `tools/perf/sample_windows.py` 仅对自己启动的子进程用 SuspendThread/GetThreadContext/StackWalk64/SymFromAddr；finally 配对 ResumeThread。无新 crate/Python 依赖。

本机全局 Cargo rustflags 强制 debuginfo=0 和 /DEBUG:NONE；仅在采样构建进程设置 `CARGO_PROFILE_RELEASE_DEBUG=true`、`RUSTFLAGS=-C debuginfo=2 -C link-arg=/DEBUG:FULL`，target 为 `D:/Rust/target/e3dio-scale-profile`，未改全局配置。首次无 PDB/未设置 PDB 搜索路径的原始尝试也保留，不能用未解析地址归因。有效数据为 `profile-symbol-search.json`（Off）和 `profile-require.json`（Require）；同名 stdout/stderr 为探针完整输出。符号搜索路径显式为 exe 所在目录。

采样间隔请求 10ms，实际还包含枚举、展开和符号解析时间；这是热点定位，不能将 inclusive 样本数当成精确阶段占比。等待线程剔除，递归同名帧每栈只计一次；16 帧上限可能截断外层调用。有效 Off stage 栈 197 条：INDEX rewrite_node 122，resolve_members 69。Require stage 栈 1396 条：SpatialStage::prepare 1206；所有栈中 Snapshot::value 533、EndState::record 390、flags_by_mpbox 752（inclusive，不能相加）。Off 有 4 次 suspend 失败，Require 无失败；子进程均 exit 0。保留失败计数，不计为有效栈。

与采样对应的源码解释：

- `index_cow::rewrite_leaf` 对每次编辑线性寻找键，并为插入再扫描位置；一个尾叶接收 n 条新记录后再分裂，中间向量随 n 增长，形成二次扫描。
- `Transaction::place_member` 的 Last 路径逐次 contains 扫描不断增长的 owner 成员列表。
- `Snapshot::value` 每次属性访问重新编码、解析及完整描述符提取；`EndState::record` / Snapshot 缓存命中返回 RecordDraft 克隆，owner 的大成员向量反复复制。空间计划重复构造重算集合；记录项补丁逐条在线性 elements 中找位置。

这些是 12.4 的候选范围；本次未应用优化，也未改页面布局、规则或错误语义。先固定直接父提交并两遍录制预铸混合金样，才能修改上述写侧。

## 验证与下一提交

旧 32/128/257 批次入口和金样保留。新增 `mixed_pre_minted_scale_batches_read_complete_member_order` 使用同样准备、固定时间、预铸编号和底层原语，10k/100k 均实际读回，2/0/0（含旧测试），50.72s。探针单元检查 1/0/0；32 Require 三轮属性/空间坐标验证通过，文件 SHA 三轮一致。尚未录制混合金样；12.3 不勾选。

`cargo test --no-fail-fast -- --nocapture` 全套 exit 0，新增混合测试实际运行（50.69s），既有 30 ignored 和样本缺失 skip 沿用；`cargo clippy --all-targets` / `cargo doc` exit 0，warning 63 / 25，与 F 基线相同。后两者完整日志已保存。`off_compat.json` SHA-256 仍为 `1b2ac106e9a69a9a5fe677b439ae7c43ef190191dcd8d392b2375a6602e4e80a`，17 条未修改。

收口探针在独立三轮 `verified-off.json` 上核对 XLEN 并完整读取 10k/100k owner 成员；stage 中位数 91.9999 / 5677.9427 ms，比值 61.7168，页数门通过、性能比例门失败，按设计 exit 1。原始首次基线不覆盖。

`verified-require.json` 同样全量核对每条 SPAMAP 坐标，10k/100k 三轮均完成；stage 中位数 1140.3622 / 36836.9332 ms，比值 32.3028。10k stage+commit 1142.9923 ms，时间门和页数门通过、比例门失败，exit 1；源库散列再次相同。`open-only.json` 是原入口兼容自检，三轮 open 门通过，副本及源库字节不变。12.1/12.2 已完成，12.3–12.5 仍待办。

建议下一提交 `test(perf): 12.1-12.2 add mixed scale probe and profiling baseline` 固定本次工具、测试及取证数据；在该提交两遍录制混合金样作为优化直接父基线，保留所有已有文件与退役元数据，然后推进 12.4。仓库 tasks.md 的提交约定要求用户确认提交时机；本次尚未提交。

机器沿用 F 基线：Ryzen 9 7950X、约 63 GiB、Windows 11 26200，源 D: 和临时 C: 为 WD_BLACK SN770 NVMe。10.5/10.6 和页 1 写侧未纳入本次。
