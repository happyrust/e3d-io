# 统一读写回归与 E3D TTY 归档验收

日期：2026-10-07。开发基线 `8663556`，统一入口实现提交 `1b0bbf9`。没有改变写入公开 API、既有门判分、FORMAT0 脚本或 Golden。

## 完整运行

```powershell
pwsh -NoProfile -File tools/e3d/regression.ps1
```

运行目录 `D:\e3dio-regression\20261007-133329-46342331`，退出码 0，`status=passed`、`tty_verified=true`。阶段按顺序串行运行，三个门各用独立项目副本。Git 工作树仅有既有无关未跟踪目录。

| 阶段 | 结果 | 耗时 |
|---|---|---:|
| Cargo | 789 passed / 0 failed / 30 ignored；134 个结果汇总（含 Doc-tests） | 128.698 s |
| Clippy / doc | 63 / 25 warning，按消息与源码位置去重 | 0.402 / 0.414 s |
| Q13/Q14/Q15 离线 | 全 PASS | 0.586 / 0.479 / 0.495 s |
| 真 TTY | 39 PASS / 0 FAIL | 门内部 213.278 s；进程 213.557 s |
| alternate | 五轮 T4a–c 和 Q15 全 PASS | 84.208 s |
| 原样 FORMAT0 | 11 项 PASS | 10.410 s |

既有样本缺失提示 8 条（7 个提前返回测试）另列于 `results.json` 和 `summary.md`，不作为样本已验收的证据；没有新增 required 测试跳过。Q13 用时 161.437 s，Q14 用时 9.113 s，首号 `24384/26506`。Q13g 按 SAVEWORK 精确会话 667 核对。Q15 发出后删除 `24384/26506–26528`，页 1 保持 26529，E3D NEW 取得 `24384/26529`。

AMS 的 `ams000`、TES 的 `TES000` 全部文件集合、长度和 SHA-256 前后一致；Golden 目录全部文件亦一致。17 条 Off 金样文件 `off_compat.json` SHA-256 保持 `1B2AC106E9A69A9A5FE677B439AE7C43EF190191DCD8D392B2375A6602E4E80A`。

完整证据见 [results.json](assets/2026-10-07-unified-regression/full/results.json)、[summary.md](assets/2026-10-07-unified-regression/full/summary.md) 与 [manifest.json](assets/2026-10-07-unified-regression/full/manifest.json)。304 个证据文件（约 7.5 MB）从运行目录逐项复制后重新验证哈希；整份项目数据库未归档。原始字节编码保留，默认登录口令已脱敏。

## 离线与失败路径

`regression_selfcheck.ps1` 36 项通过，由 `tests/write_regression_gate.rs` 接入 Cargo。覆盖测试汇总、消息和源码位置去重、63/25 超限、缺问/重复/矛盾/回显/非零退出、空或缺原始日志、源/Golden 变化、manifest 篡改，以及实际调用入口的失败注入。失败注入两次产生独立目录，每次结果和 manifest 均保存，后续 8 个阶段保持 `not_run`。

两次独立完整离线运行分别为 `20261007-133056-4549a88e` 和 `20261007-134113-0a4cd73f`，均只运行 Cargo 和离线检查，`status=offline_passed`、`tty_verified=false`；manifest 重新计算通过，目录互不覆盖。归档分别位于 `assets/2026-10-07-unified-regression/offline-1/` 与 `offline-2/`。

早期开发运行曾发现结果容器类型和 warning/skip 统计口径问题，已在首提交前修正；旧开发运行不作为本次验收数字来源。

## 边界

源项目与输出路径重叠时拒绝执行；缺环境、文件占用、非零退出、判分或完整性失败均不得认证 TTY。失败阶段停止后续执行并保存证据，不自动结束用户 E3D 进程。

人工图形 10.5、依赖它的 10.6 和最终 14.7 保持待办；本次自动回归不替代图形截图验收。未修改 `.kiro/skills/` 或 `.kiro/specs/e3d-studio-m1/`。
