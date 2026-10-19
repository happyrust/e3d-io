# TTY 启动崩溃 0xC0020001：运行时选择漂移定位

- 日期：2026-10-19
- 现象：`regression.ps1` 的 `write`/`alternate`/`format0` 段里 des.exe 每次拉起后 ~8 s 以 `0xC0020001`（`KERNELBASE`）退出，Q1–Q14 全部 `reached command loop: false`；离线六段全绿
- 结论：不是代码回归、不是项目/共享文件损坏——是 `L3_E3D_INSTALL_DIR` 环境漂移，使 TTY 掉回了注释里本就标注「非验证运行时」的 `e3d31-runtime`；其 8/10 版 `E3DNativeFixup.dll` 的 OpenProject 预初始化在 TTY 下必崩

## 因果链

1. `gen-model/scripts/e3d/run_ams_c_entrymacro.bat`：`if not defined L3_E3D_INSTALL_DIR set L3_E3D_INSTALL_DIR=D:\work\plant-code\old\e3d31-runtime`，且头部注释明写「Its isolated APS read-only TTY launch crashed before query.log (0xC0020001); this is NOT a verified runtime」
2. `e3d31-runtime\E3DNativeFixup.dll`（8/10 20:27 构建，含 `direct:OpenProject InitialiseApplication` 字符串）：`Apply("OpenProject")` 无条件调 `TryPreInitialiseCore3DApplication` → `InitialiseApplication(45,false)`。TTY 下没有 GUI 先注册命令集，这次调用**成功**，连带把 `Aveva.Core3D.Graphics.dll` 提前装入 AppDomain
3. `StartEventLoop` 之后 ~750 ms，修复定时器在 UI 线程排队 `import 'DrawListAddin'`（钩子源码注释原文：若原生初始化赢了竞态，后续 PML import 报 "assembly already loaded"）→ attached 会话实测 `(1000,0) Assembly ...Aveva.Core3D.Graphics.dll already loaded` + 托管 NullReference（`未将对象引用设置到对象的实例`×4）→ `abort()` → `0xC0020001`
4. 影子树 `E:\reverse\e3d\shadow_e3d31_gen_model_test\E3DNativeFixup.dll`（8/9 23:36 构建，二进制无 `direct:OpenProject InitialiseApplication` 字符串——预初始化路径不在该构建）：OpenProject 不预初始化，`import 'DrawListAddin'` 干净执行，会话存活
5. 10-17 绿回归（`20261017-233056-21bca7c8`）的 driver log `AVEVA_DESIGN_EXE=E:\reverse\e3d\shadow_e3d31_gen_model_test\` —— 当时 shell 恰好导出了 `L3_E3D_INSTALL_DIR`；`l3_suite` 只传 `L3_E3D_*` 业务变量、不传安装树，靠继承

## 证据点

- `%TEMP%\e3d_native_fixup.log`：10-15→10-17 23:38 的会话零 `InitialiseApplication` 行；10-19 09:33 起每个会话都有 `InitialiseApplication(45,false) begin/OK`——同一日志文件内行为分界线 = 加载的钩子构建不同
- GUI 日志 `E:\reverse\e3d\temp\frida\e3d_native_fixup.log`：GUI 会话 `InitialiseApplication(45,false)` 恒抛 `Invalid duplicate command key AVEVA.View.CommandModelEditor` → 预初始化被当「已完成」跳过 → GUI 不受影响（10-18 图形门正常）
- attached 实测（影子树 + trivial 宏）：控制台横幅 → MAP 警告 → `DIAG-DONE` → 退出码 `0xC0000005`（DLL_PROCESS_DETACH 拆解崩，既有良性 flake，l3_suite/format0 按 L3-ALIVE/L3-DONE 标记判定而非退出码）
- 已排除：许可证签出报错（line 104 噪音，会话照常到横幅 line 109）、`%LOCALAPPDATA%\Aveva\...\3.1` 保存布局（挪开仍崩）、共享 PMLLIB/PMLUI（目录时间戳空写、文件未变）、DFLTS/注册表/scratch 项目 DB（回归用全新副本仍崩）

## 修复（`5ad44b0`）

`regression.ps1` / `write_gate.ps1` / `alternate_write_gate.ps1` / `format0_gate.ps1`：

- 各加 `InstallDir` 参数，默认 `E:\reverse\e3d\shadow_e3d31_gen_model_test`（全部历史绿回归与探针使用的验证 TTY 运行时）
- `if (-not $env:L3_E3D_INSTALL_DIR) { $env:L3_E3D_INSTALL_DIR = $InstallDir }` —— 显式导出仍可覆盖
- 把 `<InstallDir>\des.exe` 存在性并入 TTY 依赖检查

验证：`20261019-095521-ad47c7d6` 全量回归 `RESULT passed`（write TTY 1288.5 s 全判决、alternate 5 轮、format0 11 项），`regression_verify.ps1 -RequireTTY` → `tty_verified=True`。

## 遗留说明（非本仓修复范围）

- `e3d31-runtime` 的 8/10 版钩子对 TTY 致命的根因在 `TryPreInitialiseCore3DApplication`（`E:\reverse\e3d\e3d-license-hook\E3DNativeFixup.cs`）：GUI 场景靠重复命令键异常兜底，TTY 场景应跳过预初始化或容忍 import 竞态。是否修该钩子由逆工程侧决定；本仓门的修法是钉验证运行时。
- `E3D_SKIP_REPAIR_MACRO=1` 不能规避崩溃——致命步是预初始化引起的装配重复装载，发生在修复宏之前。
