# core.dll layout取证（ida-bridge 1e192b2）

本次取证使用 `cellebrite-labs/ida-bridge` master 提交
`1e192b2c9c598b4ffaf7f0ec69b14cfbb7d44d69`、IDA 9.2 和 Python 3.12。
bridge 使用新 v5 协议运行；原始 `D:\ida_scratch\plant3` 数据库被已有进程锁定，
因此复制完整 IDA 数据库到 `D:\ida_scratch\ida-bridge-core-readonly` 和
`D:\ida_scratch\ida-bridge-core3d-readonly` 后分析。原始数据库未写入。

副本 SHA-256：Core_IDB `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C`；
Core3D_IDB `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712`。
原始文件锁定和副本启动日志保存在本目录之外的 ida-bridge 日志目录中。

## 查询与结果

所有查询通过 `tools/ida/ib.ps1 --target-idb ... exec --json --sql` 执行，完整结果见本目录的 JSON 文件。

| 目标 | 取证结果 |
|---|---|
| `sub_5AECBE0` (`0x5AECBE0`) | 记录 implicit 区重建时先分配新记录；wide 记录复制 11 字 header/implicit；compact 分支按模板把类型 2/6 展开为 wide；随后设置 `0x20000000` 并重建成员/属性块。 |
| `sub_5AE9960` (`0x5AE9960`) | `sub_5AEC110` 的 kind 2、kind 1、kind 3 调用分别处理成员块、explicit 块和 hunk；调用受旧地址、extent/session 过滤条件控制。kind 3 hunk 目录项回写新页/偏移。 |
| `sub_5AE7C30` (`0x5AE7C30`) | 沿记录链调用 `sub_5AE9960`，对旧叶值和新页地址执行 `DTGET_0`/`DTPUT_0`；错误返回前不会提交最终页。 |
| `sub_5AB91B0` (`0x5AB91B0`) | 属性/成员更新路径把调用方数组和计数传入 `sub_5AFDF10`；失败码通过 core 错误状态传播。 |
| Core3D | 发现 `?ATT_SPAMAP@@3QBVDB_Attribute@@B`、`aMaplibMapkey` 及 `PositionUtils::getWorld*` 符号；现有 SPAMAP 世界 AABB 路径已有独立 Q13/Q14 真实门证据。 |

## 对 Rust 写入器的判定

本次证据确认了成员、explicit、hunk 的 core 写回调用及 compact→wide 分支，但没有证明当前
e3d-io 的页布局顺序会导致 E3D 读侧或现有逐字节 Golden 失败。当前 `place_record` 的 hunk
先写策略与既有真实 TTY、读侧 SPAMAP 和 hunk 回读测试一致；因此本提交不改写入顺序、不改变
公开 API，也不重录 Golden。compact、TextList、跨 extent 和 native byte-for-byte 对齐仍按
Spec_Ledger 的触发条件留待专门需求或反例。

## 可复验命令

```powershell
$env:IDA_BRIDGE_HOME='C:\Users\dpc\.agents\tools\ida-bridge'
pwsh -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\ida-bridge-core-readonly\core.dll.i64 env
pwsh -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\ida-bridge-core-readonly\core.dll.i64 exec --json --sql "SELECT n,line,ea FROM pseudocode WHERE func_ea = 0x5AECBE0 ORDER BY n"
```
