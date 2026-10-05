# E3D 空间图形门尝试

日期：2026-10-06。

10.5 要求在 E3D Design 图形界面中打开受控 scratch 项目，用 `ADD` 创建一个由 e3d-io 写入的 BOX，再执行 `LIMITS CE`，确认不出现 `(2,536)`。

本次检查确认了受控项目目录、E3D 2.10 可执行文件 `D:\AVEVA\Everything3D2.10\des.exe` 和既有 TTY 驱动资产存在。但本机没有正在运行的 E3D 图形窗口；启动该已安装程序后立即退出，Windows 进程退出码为 `0xE0434352`（CLR 未处理异常）。因此没有进入项目、没有执行 ADD/LIMITS，也没有记录 PASS。该结果是环境阻断，不替代 10.5。

自动 TTY 空间门、SPAMAP 读回、Q15 页 1 桶门和库级测试仍以各自 Evidence_Doc 为准。
