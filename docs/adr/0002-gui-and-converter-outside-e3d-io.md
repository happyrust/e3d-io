# GUI 与转换引擎放在 e3d-io 之外的独立 workspace

浏览与转换界面（gpui-kit）和转换引擎放进新的 `vendor/e3d-studio` workspace：`e3d-convert` 是无 UI 的库加 CLI，`e3d-studio` 是 gpui 应用，二者都按路径依赖 e3d-io 与 e3d-attlib。e3d-io 保持无 UI、依赖精简、MISSION 不改（它把「E3D 2.10 及其他版本的常量差异」列在范围外），只补通用的只读接口；跨发布版本的知识（schema 差异分类、核验口径、转换路线）住在 `e3d-convert`。

## Considered Options

- **放进 e3d-io**（新模块加 `e3d-descriptor` 子命令）：要改 MISSION 的范围；e3d-io 有七个下游 crate 按路径依赖它，界面类依赖不能进它的 manifest。
- **只做 GUI crate，引擎写在界面里**：没有 CLI 和无头测试，核验结果在 CI 里复现不了。

## Consequences

- `e3d-studio` 的 workspace 要自己带上 gpui-kit 的构建设置：MSVC 的 `/STACK:8000000` 和 dev profile 的 opt-level 覆盖，这两样不随依赖继承。
- e3d-attlib 必须和 e3d-io 走同一个路径，否则依赖图里会有两个 `AttlibData` 类型（README「Building」）。
