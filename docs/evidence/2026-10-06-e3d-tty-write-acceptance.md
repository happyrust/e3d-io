# E3D TTY 写入验收（2026-10-06）

本次验收用真实 E3D 3.1 Design TTY 读取 `e3d-write` 写入的每个会话。所有写入均在 scratch 副本 `D:\e3dio-gate` 完成，源库未写入。

## 命令与完整结果

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File tools/e3d/write_gate.ps1
```

退出码为 0，输出 `all PASS — E3D 3.1 read every session e3d-write wrote`。Q0 的六个拒写、dry-run 回滚和字节不变检查，以及既有 Q1–Q12、Q13/Q13a–Q13h 和 Q14 全部通过；没有新增 skip。

| 门 | 结果 | 耗时 |
|---|---:|---:|
| Q0、Q1–Q12、Q13、Q13a–Q13h、Q14 | 全部 PASS | 204.7711665 s |
| Q13 空间组 | 全部 PASS | 158.8434024 s |
| Q14 发号下界 | PASS（24384/26506） | 9.2959684 s |

本次运行同时通过离线检查：`PASS Q13 offline checks=46`、`PASS Q14 offline`、`PASS Q15 offline`。

## 关键 TTY 场景

基础写入使用 e3d-io 会话 647–656；E3D `SAVEWORK` 为会话 658，退出后 latest 为 659；e3d-io 从 base 656 合并并生成会话 660。Q13 的控制 BOX、移动、XLEN 增长、删除以及 CYLI/EQUI 世界坐标场景均由独立 TTY 查询判分：旧位置由 true 变为 false，新位置由 false 变为 true，XLEN 变更使 Probe 从不相交变为相交，删除后条目消失。

Q13g 使用精确保存会话做读侧检查：会话 667 的 SPAMAP 有 3755 个条目、244 页；作者 BOX 条目 `1/1`，Built_In_AABB `agree=1`、`waiting=0`，flags、key0 和 `entries_k` 均 `agree=1`。没有用退出后的 latest 代替保存会话。

Q13h 将 E3D 世界原点与 `world-frame` 输出比较，每轴误差均不超过 0.1 mm。Q14 的期望和实际首个新 refno 均为 `24384/26506`。

## 完整性核对

门脚本内部比较源 AMS 库；运行前后 SHA-256 均为
`3F8DED2533EE9AFA106DB4873C680F651BA5D719DF3FD3A0A9AE12C29CE0A342`，长度均为 `34037760` 字节，未发现 `source database changed`。原始 TTY 日志、操作文件和副本保存在：

```text
D:\e3dio-gate\logs\
D:\e3dio-gate\ops\
D:\e3dio-gate\AvevaMarineSample\
```

本证据覆盖自动 TTY 门（10.1–10.4、Q14/Q15 回归）及写侧兼容性；人工 E3D 图形门 10.5 仍待完成，不能由本次 TTY 结果替代。
