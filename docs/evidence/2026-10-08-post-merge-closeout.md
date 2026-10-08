# 合并后回归与下游验收（2026-10-08）

基线为 `6f7dd2e`。本次没有重录 Golden、修改 E3D 门脚本或写入源 AMS/TES 项目。

## 结果

- `regression.ps1 -OfflineOnly` 启动了 Cargo 阶段。测试汇总为 835 passed / 20 failed / 35 ignored；Clippy 完成，doc 完成。
- 失败来自测试明确报告的样本漂移：当前 AMS `ams8000_0001` SHA-256 为 `3F8DED2533EE9AFA106DB4873C680F651BA5D719DF3FD3A0A9AE12C29CE0A342`，Golden 记录值为 `3CAB5D7D2B2C40FDF0829B91C555D19D55C530DD49860A4665560E5C553FE023`。未将失败改成 skip，也未更新 Golden。
- `D:\e3dio-fixtures\phase3\ams8000_0001` 缺失；相关单元测试已改为安全跳过，定向测试通过。
- Clippy 维持既有警告范围；`cargo doc` 成功，当前输出为 26 条文档警告。警告未作为本阶段新功能验收项处理。
- 使用当前工作树替换 `e3d-io` 和 `e3d-attlib` 的临时下游副本后，`e3d-model` 主 crate 与 9 个 verify 变体共 10 项 `cargo check --offline` 全部通过。原始下游工作树未修改。

## 结论

下游 `14.5` 的失败归因已更正：`with_derived` 缺失来自 `write-path` 与 `main` 分叉，合并后 10 项均通过。完整离线回归仍受样本漂移阻断，因此不认证完整回归，也不打勾 10.6/14.7；GUI 10.5 继续待办。

证据：完整回归目录为 `D:\e3dio-regression\20261008-012354-b708857a`，下游日志为 `D:\e3dio-downstream-results-20261008`。
