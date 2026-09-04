# 教学笔记

给未来的我看的：这个人怎么学得最快，以及别踩什么。

## 偏好

- **要数字，不要形容词。**"覆盖率不高"没有意义，"35734 条里 8042 条有位置 = 22.5%"才有。
  每个结论尽量附一次现场实测。
- **先给结论，再给推导。**开头一句话说清楚，后面再展开。
- **不接受"应该是"。**能跑就跑一遍。手上有实物文件、有样例库、有 CLI，没有理由靠推测。
- 中文行文，标识符、命令、字段名保持英文原样。

## 工作纪律（代替社区评议）

任何新结论，在**第二个独立来源**上复现一次再写进课程或参考卡：

- 磁盘解析 ↔ 运行时内存 dump
- 二进制解析 ↔ E3D TTY 输出
- DESIGN 库 ↔ 其余 19 个 `*vir.dat`

## 环境

| 东西 | 路径 |
|---|---|
| 20 个 schema 文件 + attlib | `schema\e3d31\binaries\`（仓内，已提交进 git；原件在 `E:\reverse\e3d\shadow_e3d31_aps_all`） |
| 引用它们的写法 | `--schema schema\e3d31`，不用再写绝对路径，见 `schema/README.md` |
| 换机器 / 换一套安装 | `$env:E3D31_SCHEMA_DIR` 指到那套安装目录，它比 pack 自带的先被找到 |
| 样例库（小，18 MB） | `D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001` |
| 样例库（大，104 MB） | `D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001` |
| 常用元素 | BRAN `24384/23257`，NXTR `17496/9621` |

样例库文件可能被运行中的 E3D 占用。外部脚本读它要用
`FileShare.ReadWrite` 打开，否则报"正由另一进程使用"。Rust 的 `File::open` 默认就是共享的，不受影响。

## 坑（已踩过，别再踩）

- `--noun WORLD` 查不到。hash 算的是**短名** `WORL`。catalog 里的 `short` 字段才是拿来算 hash 的那个。
- `D:\AVEVA\Everything3D3.1\desvir.dat` 这台机器上没有。schema 文件一律走 `schema\e3d31\binaries\`，
  Rust 侧从 `tests/common` 取 `DESVIR` / `ATTLIB` / `CATVIR` / `DICVIR`，Python 侧从
  `probe_default_chunks` 取 `SCHEMA_BINARIES`；别再往测试里写绝对路径。
- `D:\work\plant-code\pdms-io-fork\test-file\`（`ams1112_0001` + 那份 `attlib.dat`）也不在这台机器上。
  引用它的十几个测试打一行 `SKIP` 就 return，`cargo test` 照样报 ok——**它们现在一个断言都没跑**。
  这一族是样例库不是 schema，pack 补不了。
- `differing_layout_nouns` 的计数会因为算不算空模板而差几个（41 vs 39）。引用数字时要说明口径。
- 写 PowerShell 探针时，函数里 `return $bytes` 会被**展开成 Object[]**——4.8 MB 的
  `byte[]` 变成四百多万个装箱对象，后面每次按 `[byte[]]` 传参又要转换一次，脚本直接卡死。
  写 `return , $bytes`，或者干脆别用函数包装 `ReadAllBytes`。
