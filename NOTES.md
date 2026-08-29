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
| 20 个 schema 文件 | `E:\reverse\e3d\shadow_e3d31_aps_all\*vir.dat` |
| attlib | `E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat` |
| 样例库（小，18 MB） | `D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001` |
| 样例库（大，104 MB） | `D:\work\plant-code\pdms-io-fork\test-file\ams1112_0001` |
| 常用元素 | BRAN `24384/23257`，NXTR `17496/9621` |

样例库文件可能被运行中的 E3D 占用。外部脚本读它要用
`FileShare.ReadWrite` 打开，否则报"正由另一进程使用"。Rust 的 `File::open` 默认就是共享的，不受影响。

## 坑（已踩过，别再踩）

- `--noun WORLD` 查不到。hash 算的是**短名** `WORL`。catalog 里的 `short` 字段才是拿来算 hash 的那个。
- `D:\AVEVA\Everything3D3.1\desvir.dat` 不存在，测试里那个常量是旧路径。实际用 `E:\reverse\...`。
- `differing_layout_nouns` 的计数会因为算不算空模板而差几个（41 vs 39）。引用数字时要说明口径。
- 写 PowerShell 探针时，函数里 `return $bytes` 会被**展开成 Object[]**——4.8 MB 的
  `byte[]` 变成四百多万个装箱对象，后面每次按 `[byte[]]` 传参又要转换一次，脚本直接卡死。
  写 `return , $bytes`，或者干脆别用函数包装 `ReadAllBytes`。
