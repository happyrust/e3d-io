# 10.5 E3D 3.1 图形会话（空间/LIMITS 门）

- 日期：2026-10-18（UTC 会话 17:56–18:16，本机 01:56–02:16 +08:00）
- Scratch 项目：`D:\e3dio-gui-gate\AvevaMarineSample`（源 `D:\AVEVA\Projects\E3D3.1\AvevaMarineSample` 未动，源 SHA 未变）
- Scratch DB：`ams000\ams8000_0001`，本会话结束时 SHA-256 `2BD2D96F…`（E3D 登录又自加内务会话，未发 SAVEWORK）
- 会话：真 E3D 3.1 Design（影子安装 `E:\reverse\e3d\shadow_e3d31_aps_all\des.exe`，PID 46228），`gui_gate_launch.bat` 起，`LIMITS_CE`/`designViewLimits` 命令对象注册 PASS、runtime health PASS、`3D View(1) - Drawlist(3)` 在线
- 驱动方式：收口计划 Q2 方案 A——CLR 注入 `E3DBootstrap.QueueThreadedDirectMacroRawFromHost` 在 UI 线程上跑命令/宏，等价于人在命令行窗口键入；命令原文、逐步返回、E3D 错误码全部落盘

## 执行与逐条结果（文件原文见 assets/2026-10-18-spatial-gui/）

正例 `/E3DIO-Q13-MAIN` = `=16192/5`（e3d-io 会话 652 `q13_boxes --spatial require` 写入、含 1 条 SPAMAP）：

```
GATE105-POSITIVE
NAV OK ce==16192/5 type=BOX
ADD OK
VOLUME OK VOLUME          <- !!volume(!!ce) 读 SPAMAP 体积成功
VIEW OK type=G3D          <- 真实 3D 视图对象
LIMITS OK                 <- !!gphViews.limits(view, !!ce) 成功
EXEC OK                   <- !!designViewLimits.execute(['CE']) 成功
GATE105-POSITIVE-END
```

零 (2,536)、零 (69,9)、零异常——e3d-io 写的 BOX 通过真 E3D 3.1 GUI 的 Limits-CE 全路径。

负例 `/E3DIO-Q13-CONTROL` = `=16192/4`（会话 651 `q13_control --spatial off` 写入、无 SPAMAP）：

```
GATE105-NEGATIVE
NAV OK ce==16192/4 type=BOX
ADD OK
VOLUME ERR (69,9) BOX /E3DIO-Q13-CONTROL is not in the spatial map
VIEW OK type=G3D
LIMITS OK                 <- gphViews.limits 容忍缺 SPAMAP 的对象
EXEC OK
GATE105-NEGATIVE-END
```

## 与设计/预期的偏差（如实记录）

1. **`LIMITS CE` 字面命令在本环境不存在**：命令行直发报 `(47,62) Incomplete command line`（TTY 同）。3.1 GUI 里 Limits-CE 是注册的**命令对象** `!!designViewLimits`（key `AVEVA.DesignView.CommandViewLimits`，`PMLLIB\common\commands\designviewLimits.pmlcmd`，功能区「Limits CE」按的就是它）。本次以 `!!designViewLimits.execute(!args)`（args[0]='CE'）直接调用同一实现——包括其内部 `drawlist.add(!!ce)` + `view.refresh()` + `!!gphViews.limits(view, ce)`。
2. **缺 SPAMAP 的报错号按 API 层不同**：`WITHIN VOLUME` 扫描（TTY 探针 2026-10-17 记录）报 `(2,536) Error getting volume`；`!!volume(ce)` 直接读报 `(69,9) ... is not in the spatial map`。两者是同一「不在空间表」条件在不同入口的错误码。
3. **`ADD` 的语义**：命令行 `ADD =ref` / `ADD /name` 是「加入当前视图/drawlist」手势（本次 OK）；`designViewLimits` 的 CE 分支自己也做 `drawlist.add(!!ce)`。
4. **PML 细节**：`ARRAY.append` 落在下标 1，`execute` 读 `args[0]`——须显式 `!a[0]='CE'`；`!!ce.name()` 在宏上下文触发 `(47,62)`（改用 `!!ce.string()`）。

## 附带发现（图形路径的负例后果）

把无 SPAMAP 的 CONTROL 拉入 drawlist 后，图形刷新路径在材料化该成员时命中 `(69,9)` 且未被兜住，**E3D 主窗口随后拆解**（进程存活、UI 销毁；本会话与上一会话 68320 同此结局）。即对照件在 GUI 侧同样真实地坏，只是错误号是 (69,9)——与 TTY 扫描的 (2,536) 属同一根因（缺 SPAMAP 条目）。

## 截图

`assets/2026-10-18-spatial-gui/gui-105-window-composite.png`：会话现场合成截图——AVEVA E3D Design 主窗口 + Explorer 树（`SITE E3DIO-Q13-SITE` → `ZONE E3DIO-Q13-ZONE` → `BOX E3DIO-Q13-*` ×4 + `EQUI` 可见）+ Command Window + 3D 视图框架。3D 视口画面为 GPU/SGL 表面，PrintWindow 截不出像素（启动器既有记录同样提示该限制），视口内容以 `3D View(1) - Drawlist(N)` 标题与文件轨迹为准。

## 结论（待用户确认后打 PASS）

任务字面「`ADD` 一个 e3d-io 写的 BOX，`LIMITS CE` 不报 (2,536)」：ADD 成功，Limits-CE 注册命令路径全程无 (2,536)——按收口计划交用户确认截图与命令原文后记 PASS；10.6/14.7 随之一并收口。
