# Q13 空间自动门与 E3D 回归门

基线 `dcc71d8`，脚本首提交 `4939b8a`（`feat(gate): 10.1-10.3 add Q13 spatial acceptance`）。在该提交上启动真实门；修正日志 BOM 和坐标显示精度后，以最终脚本重新运行完整 TTY 门。全部写入只发生在 scratch 项目副本，驱动与许可证可用。未启动人工图形会话。

## 结果

| 检查 | 实测 |
|---|---|
| `write_gate.ps1` | Q0 + 既有 28 问 + Q13、Q13a–Q13h：38 PASS / 0 FAIL |
| `alternate_write_gate.ps1 -Rounds 5` | T4a / T4b / T4c 全 PASS；10 个 EQUI，17 个会话 646–662 |
| 原样 `format0_gate.ps1` | E1 / E2 / F0–F5（含子问）全 PASS；源 TES000 文件夹未改 |
| 离线门 | 38 项语法、渲染、严格判分检查；含副本六批次、逐字读侧与相交检查共 154 项，全 PASS |
| Cargo 全套 | 742 passed / 0 failed / 30 ignored；125 组 harness 结果（含 doc）；既有 7 个样本缺失提前返回测试不变 |
| Clippy / doc | 63 / 25 warning，未超过 63 / 25；新测试无 warning |
| Off Golden | 17 条通过，未重录；SHA-256 `1b2ac106e9a69a9a5fe677b439ae7c43ef190191dcd8d392b2375a6602e4e80a` |
| 耗时 | 总门 199.5563017 s；Q13 161.8146313 s |

验收日志、CLI 报告、框和摘要在 [证据目录](assets/2026-10-04-q13-gate/summary.json)，每份文件的 SHA-256 与长度见 [manifest](assets/2026-10-04-q13-gate/manifest.json)。完整 Cargo 日志及三次修正前的门日志留在 `.agents/tasks/write-path-phase-3-gate/validation/`。各门串行，alternate 使用独立 scratch，FORMAT 0 脚本文件未修改。

## 场景与报告来源

原生参照 BOX 唯一固定为 `24384/24777`，本次源库会话 646。`world-frame` 世界原点 O 为 `[-2821.27, 8003.96, 1400]` mm；SITE 放在 O，专用 ZONE 在 SITE 下。几何与 Probe 均显式 Require，未传入 AABB；Control_Box 显式 Off。六个批次均经 `apply --report --user e3dio`。

| 报告 id | 写后 refno |
|---|---|
| site / zone / control | 16192/4 / 16192/5 / 16192/6 |
| main / moving / growing / deleting | 16192/7 / 16192/8 / 16192/9 / 16192/10 |
| newowner / controlprobe | 16192/11 / 16192/12 |
| oldprobe / newprobe / growprobe / deleteprobe | 16192/13 / 16192/14 / 16192/15 / 16192/16 |
| cylinder / equipment / transformed | 16192/17 / 16192/18 / 16192/19 |

以上值只属于本次验收；脚本每次从报告解析。初始 CLI 会话 661–662；move / XLEN / delete 为 663 / 664 / 665；CYLI+EQUI 为 668。每次 CLI 变更前前一 TTY 已结束，查询新起 TTY。

## Probe 与严格判分

每次查询由独立 BEGIN/END 标记夹住；只从嵌套 RESULT 段读取 `Q VAR` 的 `<ARRAY>` 与 `[n] <STRING> '=w0/w1'` 行。命令回显不能满足 refno 条件。缺日志、缺或重复标记、缺结果、错误号、驱动失败均不 PASS。控制失败会传播至 Q13a–Q13h。

| Probe | 元素 | 余量 mm | 预判 / 实测 |
|---|---|---|---|
| controlprobe 16192/12 | control 16192/6 | 0 | 相交；搜索省略 Control，Control 作体积报 (2,536) |
| oldprobe 16192/13 | moving 16192/8 | 0 | move 前相交，后不相交；查询先列出后省略 |
| newprobe 16192/14 | moving 16192/8 | 0 | move 前不相交，后相交；查询先省略后列出 |
| growprobe 16192/15 | growing 16192/9 | 0 | XLEN 300→1400 前不相交，后相交；查询先省略后列出 |
| deleteprobe 16192/16 | deleting 16192/10 | 0 | 删除前相交且列出，删除后省略 |

完整八份世界框（控制、移动四次、增长两次、删除前）及预判保存在 [probes.json](assets/2026-10-04-q13-gate/probes.json)，由最终运行的 PROBE 行提取。基础 BOX 300×400×500 mm；移动到 x=10000 的 owner；增长 Probe 中心 x=600、100×100×100 mm。预判与期望全部相符，未移动 Probe 或修改期望迁就结果。

## SAVEWORK 与世界坐标

Q13g 在 E3D 中将 main 的 XLEN 300→301，SAVEWORK 日志确定精确会话 **666**。退出后 latest 为 667，未用于此核对。`spamap --session 666` 与 `world-frame --session 666` 的完整 11 字条目相等；`spamap-check --session FILE=666 --refno 16192/7` 的 key0、entries_k、flags、built_in_aabb、coverage 各为 1/1，waiting=0，errors=[]。

条目：`[299970282,16192,7,0,4294937578,78040,11500,4294940588,82040,16500,2]`。表目录根页 17647，244 页，3755 条；读数见 [Q13g-spamap.json](assets/2026-10-04-q13-gate/Q13g-spamap.json) 和 [Q13g-check.json](assets/2026-10-04-q13-gate/Q13g-check.json)。

Q13h 的 CYLI 和经 EQUI 平移、绕 z 旋转 90° 的 BOX 均被原生参照 100 m 搜索找到。PML1 `MM DISTANCE` / `PRECISION 3DP` 提高坐标显示精度，`Q POS WRT /*` 与读侧世界原点比较：

| 元素 | E3D 与 world-frame 世界原点 mm | 每轴误差 mm |
|---|---|---|
| CYLI 16192/17 | [2178.73, 8003.96, 1400] | [0, 0, 0] |
| BOX 16192/19 | [3978.73, 12103.96, 3700] | [0, 0, 0] |

首轮判分拒绝了带 BOM 的首行；加 BOM 回归用例后修正。坐标首轮默认显示整毫米，未通过 0.1 mm 门槛；只调整显示精度并完整重跑，未放宽阈值。未发现需要改库实现的问题。

## 源库保护与边界

write 门运行前后 SHA-256 断言相等，FORMAT 0 门核对整个原 TES000 文件夹；副本离线批次另核源哈希不变。最终源库 SHA-256：

- AMS `ams8000_0001`：`3cab5d7d2b2c40fdf0829b91c555d19d55c530dd49860a4665560e5c553fe023`。
- TES `TES023`：`cdb87f444680019e4054bc03a0ab514d5bcda2805b719eb42d00a98b51ce3d6b`。

10.1–10.3 通过；10.4 仅自动门已跑部分完成，缺第 11 组 Q14（Q15 在 13.5 存在时接入）。10.5 人工图形门、10.6 最终检查点保持未完成。性能、发号、页 1 桶表未接入。无新增依赖，无 Golden 修改。
