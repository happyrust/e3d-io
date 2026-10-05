# S6b 页 1 桶表取证

日期：2026-10-05
任务：write-path-phase-3 13.1–13.2
规则：bucket.collision, bucket.persist

## 环境

本轮使用受控 AMS 副本；源库只读保护，三次运行前后 SHA-256 与长度相同。所有 E3D 操作通过 `l3_suite.exe --check-driver` 的独立 TTY 完成，e3d-io 批次显式使用 `--spatial off --user e3dio`。

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |
| IDA 版本 | `9.2` |
| HEAD | `1198fb4afb0aa04b1f01aa5a231a207e1d3aef76` |
| Core_IDB SHA-256（末条调用后） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |

运行证据位于 `docs/evidence/assets/2026-10-05-s6b/validated/{live,deleted,gap}`；每例包含 result、pages、manifest、宏、驱动日志、CLI 报告和输出。原始未验证运行保留在同一资产目录的上层 `probe/`。

## 结论

### bucket.collision

- 状态：坐实；“已发”包含 e3d-io 发出后又删除的 RefNo。
- Live：e3d-io 发出 `24384/26506`、`26507`、`26508` 后均保留；E3D NEW 得到 `24384/26509`，不是存活重复。
- Deleted：三号均在下一 e3d-io 会话删除；E3D NEW 得到 `24384/26506`，与已发历史编号重复。
- Gap：`26506`、`26507` 删除，`26508` 保留；E3D NEW 得到 `26506`，证明算法不是简单从活最大值以上发号。
- 三例 before_e3d 与 before_new 的最新会话和 page 1 counter 相同，分别为 e3d-io 的最后会话和 `24384:26506`；每例 counter 低于本批最高已发 `26508`。
- 反例：Live 只证明存活编号未重复，不能推翻 Deleted/Gap 的历史编号复用。

### bucket.persist

- 状态：坐实为原地页 1 更新；不是由 page 0 指向的新 CoW 页。
- 静态取证：`DB_DB::updateBucketList` @ `0x59144e0` 在 `pseudocode.txt` 行 87–157 遍历 `DB_IndexTableIterator` 并更新桶条目；`2GNE` @ `0x5af7480` 在行 47–83 读取 type 1 page、写入第三字并调用 `sub_5AF1A00`；`2UP1UOR` @ `0x5afd040` 在行 65–80 校验 page 0 的扩展并在行 101–145 更新/释放桶。
- 动态取证：三例均在 NEW 后、SAVEWORK 前看到 page 1 第三个字由 `0` 变为 `4`，而最新会话号及 page 0 words 6、12、96 未变；SAVEWORK 后计数器从 `26506` 推进为 `26510`（Live）或 `26507`（Deleted/Gap），退出后第三字恢复 `0`。这说明 claim 和计数器更新直接改变物理 page 1，未经过 page 0 唯一提交点。
- 静态与动态结果一致：page 1 写入本身不生成会话，退出释放清除暂存 claim；SAVEWORK 才追加会话。
- 反例：page 0 整体 SHA-256 会在 E3D 其他元数据追加后变化，因此仅比较整个 page 0 哈希不足；本结论使用 NEW 前后会话号及固定 words 的逐项比较，并由静态 page-1 写例程交叉确认。

## 复现命令

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
powershell -NoProfile -File tools/ida/ib.ps1 --timeout-s 300 exec --json --sql "SELECT func_ea, n, line FROM pseudocode WHERE func_ea IN (0x59144e0, 0x5904cd0, 0x5af7480, 0x5afd040, 0x5ae5770) ORDER BY func_ea, n"
powershell -NoProfile -ExecutionPolicy Bypass -File tools/e3d/s6b_selfcheck.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File tools/e3d/s6b_probe.ps1 -Case All -Assets docs/evidence/assets/2026-10-05-s6b/validated
cargo test --test write_stage10_s6b_gate -- --nocapture
powershell -NoProfile -File tools/ida/check_evidence.ps1 docs/evidence/2026-10-05-s6b-bucket-evidence.md
powershell -NoProfile -File tools/ida/ib.ps1 env
```

## 反例

- Live 的 `24384/26509` 是 fresh，不能作为“所有 E3D NEW 都从活最大加一”的证明。
- Deleted 和 Gap 的 `24384/26506` 是 historical reuse；由于该编号已删除，不能把它误报为存活元素重复。
- 若 NEW 前任一快照的最新会话或 Bucket_Counter 改变，该场景按需求 13.11 作无效，不参与分支判定；本轮三例均未发生。
- 页 1 claim 的第三字是短暂运行态，不应被写侧读回测试当作最终条目值。
