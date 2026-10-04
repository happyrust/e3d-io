# 里程碑 F：发号下界、Open_Fast_Path 与 Q14

开发基线 `0336432`；取证 F1 `d4f5ab4`、读侧 F2 `6232c21`、优化金样 F3 `718cf77`、字节中性 F4 `aacdcf9`、下界 F5 `6c588d1`、计时/门 F6 `0ea2655`，场景修正 `f008aa4`。所有真写入均在 scratch 项目副本，驱动和许可证可用；原生登录沿用已有 SYSTEM 驱动，CLI USER=e3dio。未启动人工图形验收，不实现 staging 优化或页 1 写侧。

## 发号与 Q14

以新复制的 AMS 会话 646 为精确 base，在任何 TTY 登录或其他批次前运行读侧 `next-word1`：扩展 24384 活最大 26483，索引首号 26484，页 1 NextAvailable 26506，下界严格大于索引界限。CLI 在 SITE `/1RX03-PIPEBJ` 下插入命名 ZONE `/E3DIO-Q14-NAMED`，报告号 `24384/26506`；后续新 TTY 按名称 `Q REF` 同样返回该号。

原始诊断、CLI 操作/报告和查询输出见 [诊断](assets/2026-10-05-allocation-q14/ops/q14-before.json)、[报告](assets/2026-10-05-allocation-q14/ops/q14_named.report.json)、[TTY](assets/2026-10-05-allocation-q14/tty/Q14.log)。只匹配独立 RESULT 标记间唯一的 `Ref =w0/w1` 输出；缺标记、回显、缺结果、错误号、重复结果、驱动失败均不 PASS。离线 1 个合法 + 7 个拒绝场景、PowerShell 语法、JSON 和宏渲染全通过；Q13 离线 41 项通过。

首轮用 EQUI 插入第一 ZONE，导致既有 Q5 的 EQUI 序号改变：该轮 38 PASS / 1 FAIL，未作验收。修正为 SITE 下合法的命名 ZONE 后完整重跑；既有问与期望全部保留。首轮日志和 Q5 原始日志也保留，未覆盖失败证据。

## open 计时

命令：`cargo run --release --example write_scale_probe -- --open-only D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7351_0001 --runs 3`。临时副本，每轮测量前整读热缓存；描述符准备和文件复制不计入 open。首次发号列包括 insert 草稿准备，不声称是独立 Allocator 微基准；未提交，副本与源库运行前后字节相同。

| 次数 | open ms | 首次发号/草稿 ms |
|---|---|---|
| 1 | 0.1798 | 0.1365 |
| 2 | 0.1317 | 0.0208 |
| 3 | 0.1317 | 0.0217 |

中位数 0.1317 ms ≤ 1 s。机器 AMD Ryzen 9 7950X（16 核/32 线程）、约 63 GiB RAM、Windows 11 10.0.26200；源 D: 和临时 C: 均为 WD_BLACK SN770 1TB NVMe。源文件 1,238,308,864 字节，SHA-256 `d7b0cecb020c0872f86e8d03b2eee7b38efdaf6be2035090af2a40c33b4061ac`。[三次原值](assets/2026-10-05-allocation-q14/open-only.json)、[机器/存储](assets/2026-10-05-allocation-q14/machine.json)。只验收第 11 组 open，未验收第 12 组 staging。

## TTY 与库回归

完整 TTY：Q0 + 既有 28 问 + Q13/Q13a–h + Q14 = 39 PASS / 0 FAIL。总耗时 195.2950907 s，Q13 151.8869362 s，Q14 的写入/查询合计 8.6877227 s。Q13 框及 Probe 预判均成立，余量 0；refno 从报告解析。[Probe](assets/2026-10-05-allocation-q14/tty/q13-probes.jsonl)、[报告号](assets/2026-10-05-allocation-q14/tty/q13-refnos.json)。Q13g SAVEWORK 精确会话 667，退出 latest 668 不参与比较；读侧 SPAMAP/Codec 五项各 1/1、waiting=0；Q13h 世界原点每轴误差符合 ≤0.1 mm。[完整日志](assets/2026-10-05-allocation-q14/tty-final.log)。

完整 Cargo 766 passed / 0 failed / 30 ignored；既有样本缺失提前返回仍按 Spec_Ledger 单列，新增必需发号/Corpus 测试不 skip。Clippy/doc 63/25 warning，新增文件无 warning。16 例库副本、16 例历史会话抽样、全 Corpus 最新全量、未确认规则、坏桶/缺桶/重复桶、溢出、删除最大、旧 base、latest 撞号和一次/二次 Moved 均通过。计数器仅提供下界，未写页 1。

金样迁移见 [逐键账本](2026-10-05-allocation-floor-goldens.md)。原 Off 17 条文件 SHA-256 保持 `1b2ac106e9a69a9a5fe677b439ae7c43ef190191dcd8d392b2375a6602e4e80a`；F4 曾完整通过原 17/21 条。F5 仅退役锁定的 16 个动态发号键并两遍验证新下界结果；3 个预铸原语批次和其他未受影响键继续校验。

交替写 5 轮 T4a/T4b/T4c 全 PASS，10 个 EQUI、会话 646–662 共 17 个，64.063 s；原样 FORMAT 0 E1/E2、F0–F5（含子问）全 PASS，10.093 s。三门串行、各自独立 scratch。AMS 原件 34,037,760 字节，SHA-256 `3cab5d7d2b2c40fdf0829b91c555d19d55c530dd49860a4665560e5c553fe023`；TES023 原件 288,768 字节，SHA-256 `cdb87f444680019e4054bc03a0ab514d5bcda2805b719eb42d00a98b51ce3d6b`，前后均不变；FORMAT 0 另验证整个 TES000 源文件夹。

结果和命令见 [回归结果](assets/2026-10-05-allocation-q14/regression-results.json)，逐文件哈希/长度见 [manifest](assets/2026-10-05-allocation-q14/manifest.json)。11.1–11.9 与 10.4 验收完成；10.5 人工图形门、10.6 最终检查点保留待办。第 12/13 组、下游及最终发布检查继续留在任务清单，不以本次自动门代替。
