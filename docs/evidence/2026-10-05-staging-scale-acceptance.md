# 第 12 组 staging 优化验收

优化直接父为 `5548ad0fb6eff70e79cbe09ca8ad75c0b09f898c`。先提交 [探针和剖析基线](2026-10-05-staging-scale-baseline.md)，在干净写侧上运行 `pwsh -NoProfile -ExecutionPolicy Bypass -File tools/golden/record.ps1 -Out tests/golden/staging_scale.json -Test write_scale_golden -Runs 2`：53.33s / 52.87s，两遍通过，5 键保留、0 剔除，`uncommitted_changes_under_src_or_tests=false`。包括旧 32/128/257 和混合 10k/100k 预铸批次。旧 `write_hashes.json`、`off_compat.json`、`allocation_floor.json` 及精确退役关系不改；用单独 `staging_scale.json` 保存此次录制元数据，避免覆盖既有 21 键及录制来源。

## 字节中性改动

- INDEX 尾叶大批次用 BTreeMap 按原输入次序应用编辑，仍在第一个非法操作拒绝；最后按原键序生成同样的分裂和页面。小批次和非严格升序旧叶沿用原算法。新增 proptest 与原顺序算法对照最终条目及第一个错误。
- Transaction 只加速 Last 成员操作：每个 owner 保存临时成员集合，新成员直接追加；已存在成员仍走原放置算法。非 Last 和 Remove 清除该 owner 缓存，保持顺序、重复成员和错误判定。
- 空间 Snapshot 的不可变记录缓存用 Rc；noun/owner 查询不复制大成员向量。EndState 对最终草稿直接查询字段，并预索引草稿 owner→child，保留原 BTreeMap 次序和旧成员关系。
- Snapshot 与 EndState 分别缓存按 refno/hash 查询的属性结果（包括错误），只在一次不可变规划内有效。比较 old/new 时各完整描述符提取一次；保留默认值、草稿 hunk 和解码失败时的存储字回退。已扩展原 hunk 测试检查重复访问及两个版本缓存隔离。
- 重算集合只计算一次，记录项补丁用 refno→首个非 Delete 位置索引，缺少的记录按原补丁次序追加。未改几何规则、codec、侧表规划次序、页布局、合并/重试、发号或页 1 写侧。

## 三轮热缓存复测（release）

一条命令：`cargo run --release --example write_scale_probe -- --source D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --n 10000,100000 --runs 3 --spatial require`；Off 同样运行。准备不计时，每轮复制相同准备库、整读预热。独立三轮的原始 JSON/log 位于 `assets/2026-10-05-staging-scale/optimized-{off,require}.*`。

| policy / n | open / build / stage / commit 中位数 ms | peak RSS bytes | record / index / claim / TYPEX / SPAMAP / other / session |
|---|---|---|---|
| Off / 10k | 0.1652 / 14.3974 / 31.7405 / 0.3695 | 78692352 | 891 / 94 / 2 / 0 / 0 / 0 / 1 |
| Off / 100k | 0.4753 / 115.3752 / 229.6060 / 0.4799 | 213983232 | 8170 / 813 / 2 / 0 / 0 / 0 / 1 |
| Require / 10k | 0.1363 / 17.1046 / 229.2385 / 2.8358 | 79290368 | 891 / 94 / 2 / 0 / 250 / 0 / 1 |
| Require / 100k | 0.2320 / 146.2964 / 1998.8627 / 20.4506 | 317100032 | 8170 / 813 / 2 / 0 / 2274 / 0 / 1 |

stage(100k)/stage(10k)：Off 7.23385、Require 8.71958，均 ≤15；幂指数 0.859 / 0.940。相对于收口前基线，100k stage 提速约 24.73× / 18.43×。Require 10k stage+commit 中位数 231.7402 ms ≤10s。

10k 非空间总页 988 ≤891×1.5；100k 8986 ≤8170×1.5。SPAMAP 容量 L=45、height=3，10k 11000 目标条目追加 250 页 ≤493，100k 101000 条目追加 2274 页 ≤4493。页数与优化前相同；原表未被整表重录。

两策略各 n 各三轮（12 个完整文件）SHA-256/长度与 `verified-{off,require}.json` 逐项完全相同。每轮逐个读回新增/更新 BOX 的 identity 和 owner，逐个读回 1000 个 XLEN=301；完整 owner 列表和次序、100 个删除记录缺失、Require 的全部 SPAMAP 六字坐标/删除缺失均核对。100k owner 成员 101124，覆盖饱和计数。RSS 是进程历史峰值，包含准备及前轮，不声称分阶段峰值；最高 Require 约302 MiB。

机器沿用基线：Ryzen 9 7950X、约63 GiB、Windows 11 26200，源 D:/临时 C: 位于 WD_BLACK SN770 NVMe。源 AMS SHA/长度和 17 条旧 Off 金样文件未变。

## 回归

`cargo test --no-fail-fast -- --nocapture`：768 passed / 0 failed / 30 ignored；相对 F 新增混合规模金样测试和 INDEX 顺序等价 proptest。原样本缺失 skip 清单保持；新必需测试未 skip/ignore。原/新增金样、空间独立参考模型、合并重铸、一次/二次 Moved、拒写和记录项均通过。`cargo clippy --all-targets` / `cargo doc` exit 0，63 /25 warnings，无新 warning。完整输出为 `optimized-{cargo,clippy,doc}.log`。

真 TTY `write_gate.ps1 -Scratch D:\e3dio-scale-gate`：39 PASS /0 FAIL，207.4104754s（Q13 157.3568293s；Q14 9.0608649s）。Q13g 使用 SAVEWORK 精确会话667，退出latest668不用作核对；Q13refno、Probe、框及原始宏/查询/报告完整保存在`tty-logs/`和`tty-ops/`。Q14为24384/26506，仍与页1下界和原生名称查询一致。

`alternate_write_gate.ps1 -Rounds 5 -Scratch D:\e3dio-scale-altgate`：T4a/T4b/T4c全PASS，68.2851095s；10个EQUI、17个会话646..662，日志在`alternate-logs/`。原样 `format0_gate.ps1 -Scratch D:\e3dio-scale-tesgate`：全部11个判分行PASS，10.2091583s；含源TES000全目录未变门。原始日志在`format0-logs/`。

三门串行，只写各自scratch项目，脚本及旧问题期望不改。AMS源34,037,760bytes、SHA256 `3cab5d7d2b2c40fdf0829b91c555d19d55c530dd49860a4665560e5c553fe023`；TES023源288,768bytes、SHA256 `cdb87f444680019e4054bc03a0ab514d5bcda2805b719eb42d00a98b51ce3d6b`，前后相同。金样及源库散列/逐文件比较在`byte-comparison.json`，自动门结果与warning统计在`regression-results.json`，全部原始文件的SHA256/长度在`optimization-manifest.tsv`。基线`manifest.tsv`保持原样。

12.1–12.5均通过并勾选；首优化提交同时保存金样和验收证据，直接父为5548ad0。未进行人工图形验收，10.5/10.6 和第13组页1写侧仍保留待办。

证据目录按既有门证据的方式设置Git `-text`，保留原始BOM/CRLF及尾部空格；同时恢复基线提交中被Git行尾规范化的5个文本证据为仍在本地保留的原始输出字节，内容未重录。提交前逐一核对190个staged blob的SHA256/长度，与原始manifest全部一致；三个旧金样文件HEAD/index完全一致。
