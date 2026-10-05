# E3D 空间图形门尝试

日期：2026-10-06。

10.5 要求在 E3D Design 图形界面中打开受控 scratch 项目，用 `ADD` 创建一个由 e3d-io 写入的 BOX，再执行 `LIMITS CE`，确认不出现 `(2,536)`。

本次检查确认了受控项目目录、E3D 2.10 可执行文件和既有 TTY 驱动资产存在。直接启动 `des.exe` 会以 `0xE0434352` 退出；随后改用仓内既有 `gen-model/scripts/e3d/run_ams_gui.bat` 启动链，成功启动 shadow E3D（窗口标题 `AVEVA E3D Design`，SGL 初始化和许可证补丁成功）。当前 CUA 运行时实际没有可用的 Windows 窗口绑定方法（`getState` 不返回原生 app，`getApp` 未暴露），所以不能安全地向窗口发送 `ADD` 或 `LIMITS CE`。已停止 scratch E3D 进程，未留下写锁；没有记录 PASS。该结果是自动化接口阻断，不替代 10.5。

自动 TTY 空间门、SPAMAP 读回、Q15 页 1 桶门和库级测试仍以各自 Evidence_Doc 为准。
