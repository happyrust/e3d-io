# 计划：E3D Studio M1（浏览 + 2.1 → 3.1 原样携带）开发

状态：**评审通过**（2026-10-09 09:28，Plannotator 门 `approved`，无批注 → Q1–Q4 全按推荐项）→ S0 完成；S1 除 S1.6 外完成（S1.6 等 10.5 用完 E3D）；S2 完成；S3 代码与自动验证完成，界面条目待用户看过记 PASS；S4 的 3.1、3.2 完成（§5）。S4 剩余与 S5 的细化计划见 `2026-10-10-e3d-studio-m1-s4-s5-plan.md`（2026-10-10 Plannotator `approved`）
日期：2026-10-09
范围：`vendor/e3d-studio`（新 workspace：`e3d-convert` 已有，`e3d-studio` 应用待建）；`vendor/e3d-io-write`（`write-path` @ `172ab78`）只作依赖，不改源码。
依据：`docs/plans/2026-09-30-e3d-studio-gui-design.md`（下称「设计」）、`docs/adr/0001-e3d21-to-31-verified-carry-over.md`、`docs/adr/0002-gui-and-converter-outside-e3d-io.md`、`docs/plans/2026-10-07-write-path-phase-3-closeout-and-next-plan.md` §P4、`docs/evidence/2026-10-08-post-merge-closeout.md`，以及本次审核（§1.3）。

## 0. 结论

1. **引擎层大半完成，界面层还没开工。** 设计 §6 的 8 个任务：T2 已落地（`3e16423`）；T1 的 `e3d-convert` 库与 CLI 已写成，单元测试 10 / 10 通过，本次冒烟复现了设计 F6 与 §4.5 样例报告的数，但「把 §1.1 的测量写成以语料为输入的测试」和全语料复跑都没做；T3–T8（gpui-kit 应用 `crates/e3d-studio`）为零：没有 crate，`.kiro/specs/e3d-studio-m1/` 只有 `.config.kiro`。按任务量粗估，M1 完成约四分之一。
2. **三件会挡路的事**（§1.2）：`vendor/e3d-studio` 不是 git 仓库，T1 的约 3 700 行代码没有任何版本记录（F1）；gpui-kit 钉的 `gpui-pre =0.3.7` 本机 cargo 缓存里没有，gpui-kit 也从没在本机编过，T3 第一步要联网（F2）；T5 要的「同一元素两版对照」在 `e3d-convert` 里还是 `pub(crate)`（F3）。
3. **建议顺序**：S0 仓库化与基线 → S1 补完 T1 → S2 写 spec → S3 应用骨架（T3）→ S4 浏览（T4）→ S5 属性与会话（T5、T6）→ S6 转换向导（T7）→ S7 M1 验收（T8）。S1 与 S2 可并行；S6 只依赖 S1、S3，可与 S4、S5 并行。
4. M1 全程不改 `e3d-io-write` 的源码，与写路径第三期收口（10.5 图形门、10.6 / 14.7、发版）互不阻塞。交汇只有两处：收口 P3 把 `main` 快进后，`e3d-studio` 的依赖改指 `../e3d-io`；用到真 E3D 的步骤与 10.5 错开。

## 1. 审核

### 1.1 进度

| 任务 | 设计里的内容 | 现状 | 证据 |
|---|---|---|---|
| T2 | e3d-io 读侧小改（设计 §4.7 第 1–3 条） | **完成** | `3e16423`：`project_database_paths` 公开并从 `lib.rs` 导出、`extent_paths`、`DescriptorValue` 的 `Display`、`VERSION`；`tests/send_sync.rs` 断言 `ReadOnlyEngine`、`TemplateProvider`、`SchemaPack`、`AttlibData`、`DbSet` 等为 `Send` |
| T1 | `e3d-convert` 库 + CLI；§1.1 的测量写成语料测试 | **代码完成，验证欠账** | 14 个源文件约 3 700 行；CLI 五个子命令 `schema-diff` / `inventory` / `plan` / `verify` / `run`；单元测试 10 个全过；没有 `tests/`，语料测试为零；没有全语料运行的记录 |
| T3 | 应用骨架 | 未开始 | workspace `members` 只有 `crates/e3d-convert`（`Cargo.toml` 注释写明 T3 时再加） |
| T4–T7 | 浏览、属性、会话、转换向导 | 未开始 | — |
| T8 | 设计 §5 验收；E3D 门回答设计 Q1 | 未开始 | 设计 Q1（3.1 怎么读 2.1 写的 SLADDR）未坐实 |
| spec | `.kiro/specs/e3d-studio-m1/` 写成 requirements / design / tasks | 未开始 | 只有 `.config.kiro`（10-01，`requirements-first`），未跟踪 |

设计 §7 的待坐实项：Q2 已由 `tests/send_sync.rs` 回答；Q4 落在 `DescriptorValue` 的 `Display`（`verify.rs` 用它渲染值）；Q5 换了做法——`plan` 用 `scan_elements(ScanTier::Header)` 扫记录头来数迁位 noun 的元素，没走 `TYPEX`，`acp7048_0001` 上很快，大库的耗时没量；Q1、Q3、Q6 未动；Q7 属 M3。

### 1.2 发现

- **F1 `vendor/e3d-studio` 不在版本管理下。** `git status` 报 not a git repository；`e3d-convert` 10-01 写成后没有任何提交、没有远端，误删或被覆盖都无从恢复。
- **F2 GUI 依赖不在本机。** gpui-kit（`D:\work\plant-code\old\gpui-kit`，HEAD `201b55a4`，与设计读的版本相同）的 `Cargo.lock` 钉 `gpui-pre 0.3.7`；`D:\Rust\.cargo\registry` 里只有 0.3.5 一套。共享的 `CARGO_TARGET_DIR`（`D:\Rust\target`）里没有 gpui 的编译产物：T3 编不了 `--offline`，首编耗时未知，而且要和别的会话抢这个目录的构建锁。
- **F3 对照解码还没开放给界面。** 设计 §4.2 说属性对照用 `e3d_convert::verify::compare_rows`、与核验共用；实际的逐元素比较是 `verify.rs:414` 的 `pub(crate) fn compare`，公开的只有整库的 `verify_database`。T5 之前要抽出一个公开的逐元素接口，不另抄一份比较逻辑。
- **F4 收口计划 §P4 的「先做 T2」已过时。** T2 在第三期开工前就随 `3e16423` 入库；§P4「T2 在 P3 之后」那条约束因此不再涉及 M1。
- **F5 依赖只能指向 `write-path`。** `3e16423` 不在 `main` 上（`main..write-path` 89 个提交，反向 0），`e3d-studio` 目前只能依赖 `../e3d-io-write`，要等收口 P3 把 `main` 快进。
- **F6 写路径工作树有别的活在进行。** 未跟踪的 `tools/e3d/gui_gate_prepare.ps1`、`gui_gate_launch.bat` 是 10.5 门的准备脚本，不属于本计划。本计划在 `e3d-io-write` 里只新增本文件，不碰别人的未提交改动。

### 1.3 本次审核做了什么

- 读：设计全文、收口计划、`2026-10-08-post-merge-closeout.md`、`e3d-studio/Cargo.toml` 与 `e3d-convert` 的源码目录、`tests/send_sync.rs`。
- `git`：`e3d-io-write` 的 `log` / `status` / `branch -vv`，`log -S` 找 T2 接缝的提交；`e3d-io` 上 `merge-base --is-ancestor 3e16423 main`（否）。
- 跑：`vendor/e3d-studio` 下 `cargo test --offline`：10 passed / 0 failed（含编译 21 s）。
- 冒烟（只读）：
  - `e3d-convert schema-diff --from e3d210 --to e3d31 --db-type DESI`：退出 0，列出 SLADDR 删 `FOOTRO`、其后 13 个属性前移。
  - `e3d-convert verify --from e3d210 --to e3d31 --db D:\AVEVA\Projects\E3D2.1\AvevaCatalogue\acp000\acp7048_0001`：退出 1、判「需确认」；元素 10 034、属性行 217 797、相同 217 787、不一致 10、仅源存值 1、读错 0，8 线程 0.10 s，与设计 §4.5 的样例报告一致；源文件 SHA-256 前后都是 `A38C9071…`。
- 环境：rustc 1.100.0-nightly；`CARGO_HOME=D:\Rust\.cargo`。
- **没有做**：全语料运行、clippy、gpui-kit 编译、任何界面。

## 2. 计划

### S0 仓库化与基线（按 Q1）

- `vendor/e3d-studio` 下 `git init`（`.gitignore` 已有 `/target`）；首个提交原样收录 `Cargo.toml`、`Cargo.lock`、`crates/e3d-convert/`，说明里写明「T1 截至 2026-10-01，未经语料验证」。
- 基线：`cargo test`、`cargo clippy --all-targets`、`cargo doc --no-deps` 的数字记入 §5。
- 验收：`git status` 干净；基线三项有数。

### S1 补完 T1

- **S1.1 语料测试** `crates/e3d-convert/tests/corpus.rs`。语料根由环境变量给（默认 `D:\AVEVA\Projects\E3D2.1`、`D:\AVEVA\Projects\E3D3.1`），不在就打印原因并跳过（沿用 e3d-io 的安全跳过）；全语料那几条慢，标 `#[ignore]`，验收时 `--ignored` 跑。
  - 只用内置 schema、总是跑：F3、F4 的 noun 分类计数（DESI 745 → 1 113；685 个元素 noun 中 327 相同 / 329 只增 / 27 扩容（32 个属性）/ SLADDR 迁位 13 / CONVEY 删 `RLST`；CATA 165 → 191、4 个扩容；DICT 81 → 95、33 个扩容）。
  - `acp7048_0001`：§1.3 冒烟那组数。
  - 全语料（`#[ignore]`）：F5 的总数（536 个库、15 352 620 个元素、属性行 265 173 400、相同 265 167 053、默认值变化 6 295、不一致 52、仅源存值 22、读错 0）与 F6 的分布（不一致只在 SLADDR，共 8 个库）。
- **S1.2 全语料 CLI 复跑**：对 E3D2.1 下 6 个项目 `e3d-convert verify --project … --json`，总数应与 F5 一致；记用时（设计单线程 143.9 s）。
- **S1.3 落位验证**（设计 §5 第 4、5 条的 CLI 部分）：AVEVA 3.1 样例里与 2.1 逐字节相同的 317 个库 `run` 到 scratch 输出目录（`acp7048_0001` 加 `--accept`），输出 SHA-256 与 3.1 树里的文件相同；源文件 SHA-256 前后不变。
- **S1.4 取消**（设计 §5 第 6 条的库层部分）：库层测试对 `ams7351_0001`（1.2 GB）跑 `run`，核验阶段由另一线程置取消标志，1 s 内返回已取消，暂存目录清空、目标位置无残留（`#[ignore]`，要语料）。
- **S1.5 公开逐元素对照**（F3）：把 `verify.rs` 的比较抽成公开接口（入参一个已解析元素与一对发布版本，出参归好类的属性行），`verify_database` 改走它；S1.1 的数不变即说明行为没变。
- **S1.6 设计 Q1**（按 Q3）：在 AvevaCatalogue 的 scratch 副本里开 TTY，对 `=15240/1968` 查 `Q HRSTAR`、`Q FRAD`、`Q POSTEX`，复用 `tools/e3d` 的驱动；与写路径 10.5 错开使用 E3D。
- 产出：`docs/evidence/2026-10-xx-e3d-convert-corpus.md`（落点按 Q1）：命令、数字、用时、报告存档路径。
- 验收：S1.1 常规测试与 `--ignored` 全过；S1.2–S1.4 的数与设计一致。不一致时按真实结果记录并回报用户，不改判据。

### S2 写 spec（按 Q2）

- 把设计 §3–§6 与本计划写成 `.kiro/specs/e3d-studio-m1/` 的 `requirements.md` / `design.md` / `tasks.md`，格式沿用 `write-path-phase-3`：需求带编号与验收条款，tasks 逐条勾选并带 Spec_Ledger。
- 设计 §5 的 7 条验收原样进需求；D1–D14 不改。
- 验收：tasks 覆盖 S3–S7 每一条；需求编号与设计 §5 一一对应。

### S3 应用骨架（T3）

- 前置：联网一次，在 `vendor/e3d-studio` 下 `cargo fetch` 拉 `gpui-pre 0.3.7` 及其依赖（F2）；先编 gpui-kit 的 `examples/hello_world`，量首编时间、确认工具链（gpui-kit 是 edition 2024）。之后的编译一律 `--offline`。不能联网就回报用户。
- 新建 `crates/e3d-studio`（bin）并加进 `members`；`.cargo/config.toml` 写 MSVC `link-arg=/STACK:8000000`；核对现有 `[profile.dev.package."*"] opt-level = 3` 已覆盖 gpui-kit 要求的 gpui-pre / taffy / rustybuzz。
- 窗口：`gpui_kit::application()` → `init` → `open_window`（包 `Root`）；`TitleBar`、`AppMenuBar`、工具栏、`StatusBar`；`DockArea` 放设计 §3.1 的各面板（先是空壳），布局存到应用数据目录、重启恢复。
- 设置页：schema 根目录、各版本 binaries 位置、页缓存大小、主题、语言；rust-i18n 的 `zh-CN` / `en`。
- 设计 Q3：最小例子坐实 `prompt_for_paths` / `prompt_for_new_path` / `reveal_path` 的签名与外部文件拖入（`ExternalPaths` + `on_drop`）。设计 Q6：中文字形回退截图，不够就把 `font_family` 设成 Microsoft YaHei UI。
- 验收：`cargo build -p e3d-studio --offline` 成功；应用能起，中文正常，布局重启后还在，打开文件对话框与拖入都拿得到路径（截图与 UIA 读回，按 Q4）；`e3d-convert` 的测试不受影响。

### S4 浏览（T4）

- `db/protocol.rs`：请求 `Open` / `Children` / `Attributes` / `Find` / `Reopen` / `Poll`，都带代号 `gen`。`db/worker.rs`：每个打开的库一个 `std::thread`，独占 `ReadOnlyEngine` 与各版本的 `TemplateProvider`，`AttlibData` 用 `Arc` 共享；应答走 smol channel，前台 `cx.spawn` 接收，过期的 `gen` 丢弃（设计 D10、§4.3）。
- 头信息面板（`header_diagnostics` 等）；源版本必选、按项目根记住、不猜（D5）。
- 懒加载虚拟树：扁平可见行加 `uniform_list`，展开时才读子记录，超大成员表只取可见行的名字（D11）。
- 搜索 `/NAME` 与 `=w0/w1`，没命中说明原因；面包屑 owner 链；前进、后退。
- 验收：不开界面的单元测试覆盖协议、过期应答丢弃、搜索输入解析；`ams7351_0001` 打开到出头信息 < 1 s；展开 8 200 个成员的 ZONE < 150 ms；展开时 UI 线程单帧 < 16 ms（用 gpui-kit 的 `fps` crate 或帧计时日志量）。

### S5 属性与会话（T5、T6）

- 属性表：`DataTable` 列出名字 / 值 / 状态 / 存储；过滤；双击看长值；复制。对照模式走 S1.5 的公开接口，多一列目标版本的值与差异标记，带「只看差异」开关。
- 会话：底部会话列表；「按此会话查看」走 `open_at`，标题栏标会话号；每 5 s 重读页 0，有新会话时状态栏提示，F5 切到最新并保持选中。
- 验收：选中到出属性表（含对照）< 10 ms；`acp7048_0001` 的 `=15240/1968` 对照出 `HRSTAR` 1200 / 0、`FRAD` 1200 / 50、`POSTEX` 1000 / 100 并标「不一致」，与 CLI `verify` 的发现一致；新会话提示在 scratch 副本上验：浏览中用 `e3d-write apply` 给副本追加一个会话，5 s 内出提示。

### S6 转换向导与任务（T7）

- 四步向导（选库 → 计划 → 执行 → 报告），全部调 `e3d_convert`；勾选列自绘（`DataTable` 只能单选）；任务线程各开各的 engine；进度每秒至多 10 次；取消走 `AtomicBool`。
- 报告用 `TextView::markdown` 显示；「需确认」的库可「接受并携带」，二次确认，报告记下接受人与时间；可导出 JSON / Markdown、在资源管理器里显示输出。
- 验收：同一输入下 GUI 与 CLI 的报告除时间字段外逐字段相同（设计 §5 第 2 条；脚本去掉时间字段后 diff）；GUI 里取消 1 s 内停下、暂存清空、目标无残留（§5 第 6 条）；一个库失败不影响其它库。

### S7 M1 验收（T8）

- 设计 §5 第 1–7 条逐条跑，证据写 `docs/evidence/2026-10-xx-e3d-studio-m1-acceptance.md`。第 7 条 E3D 3.1 门走 CLI、复用 `tools/e3d` 的驱动：把若干转换输出放进 scratch 项目副本，TTY 读回抽样属性，与 e3d-io 按 `e3d31` 解出的值一致。
- 用户确认后 spec 的 tasks 打勾，M1 收口。

### 依赖

S0 先做。S1、S2 可并行。S3 要 S2 写完和一次联网。S4 在 S3 之后；S5 要 S4 与 S1.5；S6 要 S1、S3，可与 S4、S5 并行。S7 在全部之后。

与写路径第三期收口：M1 不改 `e3d-io-write` 的源码；收口 P3 把 `main` 快进后，`e3d-studio` 的 `e3d-io` 依赖改指 `../e3d-io` 并重跑 S1.1（F5）。M1 中途若真要 e3d-io 新的读侧接口，只加不改，等 P3 之后单独提交。用到真 E3D 的 S1.6、S7 与 10.5 错开。

## 3. 待定问题（每题第一项为推荐）

- **Q1 版本管理与文档落点。** A（推荐）`vendor/e3d-studio` 建独立的本地 git 仓库，暂不建远端；M1 的 spec 与证据放在这个仓库（`.kiro/specs/e3d-studio-m1/`、`docs/evidence/`），设计与本计划留在 `e3d-io-write` 作依据。这会改动收口计划已批 Q4-A 的后半句（空 spec 目录「写实后随 spec 在 `e3d-io-write` 提交」）：空目录改为移到 `e3d-studio`。B 独立仓库，但 spec 与证据仍放 `e3d-io-write`。C 暂不建仓。
- **Q2 spec 要不要再过一次门。** A（推荐）本计划过门后据它写 spec，spec 只在改动 D1–D14 或设计 §5 验收时再过 Plannotator；B spec 写完一律再过一次。
- **Q3 设计 Q1（3.1 怎么读 2.1 写的 SLADDR）何时坐实。** A（推荐）提到 S1 做：几分钟的 TTY 三问，结果决定报告里怎么解释 SLADDR 的「需确认」；B 留到 S7。
- **Q4 界面验收谁来操作。** A（推荐）代理用 cua-driver 驱动应用，截图与 UIA 读回存证据，S3、S7 两处交用户看过再记 PASS；B 用户按清单手动操作，代理只写证据。

## 4. 不做

- M2、M3：会话差异、记录检查器、引用我的、schema 差异视图、SLADDR 按 3.1 布局重写、3.1 → 2.1、项目级转换。
- 不改 `e3d-io-write` 的源码与测试，不改 D1–D14，不碰写路径第三期收口的 10.5、10.6 / 14.7 与发版。
- 不写源项目：E3D2.1 / 3.1 的源库只读打开，转换输出与 E3D 会话都在 scratch 副本上。
- 不推送、不建远端。

## 5. 执行记录

- 2026-10-09 S0 完成。`vendor/e3d-studio` 下 `git init -b main`，`d70f6f7` 原样收录 workspace 与 `crates/e3d-convert`（18 个文件）；`c5705b4` 把 `.kiro/specs/e3d-studio-m1/.config.kiro` 从 `e3d-io-write` 移入（Q1-A，specId 不变）。基线：`cargo test --offline` 10 passed / 0 failed / 0 ignored；`cargo clippy --all-targets` 3 条（`parallel.rs:36` 可写成 `while let`，`report.rs:280` 与 `main.rs:600` 手写了 `is_multiple_of`）；`cargo doc --no-deps` 0 条。构建产物在共享的 `D:\Rust\target`。
- 2026-10-09 S1 除 S1.6 外完成，证据 `e3d-studio/docs/evidence/2026-10-09-e3d-convert-corpus.md`。`e3d-studio` `fa729de`：S1.5 公开逐元素对照 `ElementComparer`，`verify_database` 改走同一段解码；`a9458cd`：语料测试 `crates/e3d-convert/tests/corpus.rs` 与证据。测试：常规 3 / 3（单元 11 / 11）；`--ignored` 三条全过——全语料 536 个库与 F5、F6、F8 一致（40.6 s），取消 `ams7351_0001` 174 ms 停下且无残留，317 个库落位后 SHA-256 与 3.1 树相同（34.6 s）。S1.2 CLI 六个项目合计与 F5 逐项一致，8 线程 72.7 s。
  - 与设计不符一处：§5 第 4 条只写 `acp7048_0001` 先按 D2 接受，实跑 317 个里还有 7 份 `ams1112_0001*` 人工副本被拦（两树逐字节相同、F6 已点名它们带 SLADDR）。测试改为 8 个逐个接受后比 SHA-256；2026-10-09 用户确认按此口径（A：8 个接受，317 不变）。
  - S1.6 未做：10.5 门的 E3D Design（`shadow_e3d31_aps_all\des.exe`）自 10-08 16:36 起一直开着，按本计划与 10.5 错开。
- 2026-10-09 S2 完成，`e3d-studio` `313a5c8`：`.kiro/specs/e3d-studio-m1/` 的 `requirements.md`（11 条需求；需求 9 的 7 条与设计 §5 逐条对应，第 4 条按勘误写 Held_8）、`design.md`（落点、线程、组件、7 条性质、D# 落点表）、`tasks.md`（6 组覆盖 S1、S3–S7，S1.1–S1.5 打勾，带 Spec_Ledger）。D1–D14 未改。spec 含设计 §5 第 4 条的勘误，用户定不再过 Plannotator（聊天里的确认即过门）。
- 2026-10-09 S3（spec 任务 2.1–2.7）完成，证据 `e3d-studio/docs/evidence/2026-10-09-e3d-studio-s3.md` 与截图 01–08。`e3d-studio` `7f218af`（2.1–2.2：gpui-kit 工作树 `cargo fetch` 42.6 s 拉到 `gpui-pre 0.3.7`，`hello_world` 首编 98.3 s；`crates/e3d-studio` 建成，`.cargo/config.toml` /STACK，首编 431 个 crate 164 s）、`9232ea1`（2.3 + 2.6：主窗口、Dock 空壳面板、布局存盘与恢复、全部快捷键）、`1d86e62`（2.4：设置页与 `settings.json`，主题 / 语言即时生效；Q6 结论不设 `font_family`）、`35db70f`（2.5 + 2.7：文件对话框与拖入，Q3 签名坐实；设置与布局往返测试）。F2 的「联网一次」只在 gpui-kit 工作树 fetch 加本仓补 1 个 crate，之后全部 `--offline`。检查点：`cargo test -p e3d-convert` 11 / 0 + 3 / 0（3 ignored）不变；`cargo test -p e3d-studio` 8 / 0；clippy 仍是 S0 基线 3 条、e3d-studio 0 条；doc 0 条。界面条目（应用能起、中文正常、布局与设置重启后还在、坏文件回默认进日志页、快捷键送达、对话框弹出）有截图；对话框选中后路径进状态栏、拖入、菜单弹出、语言下拉四项 cua-driver 驱动不到（原因见证据 §2.5），待用户手验后 2.8 记 PASS。
- 2026-10-09 S4（spec 任务 3.1–3.2）完成。`e3d-studio` `438cc04`：`db/protocol.rs`（请求应答、`GenGate`、搜索输入解析）、`db/worker.rs`（每库一线程，`SchemaCache` 共用 `Arc<AttlibData>`，对照走 `ElementComparer`）、`db/library.rs`（前台 `Library` 实体）；选中、拖入、命令行给出的库真正打开，状态栏显示库类型、会话号、页数；e3d-convert 只加 `ReleaseSchema::with_attlib`、`attlib_shared`。`72708fb`：spec 3.1 按实现改写（打开与轮询不是请求，只有子行、属性、搜索带 Gen；用户确认），design 协议表同步，3.1、3.2 打勾，面板显示退出原因移到 3.3。`b504e70`（P0）：库线程 panic 时 `Library` 报 `Exited`，`LibraryState` 装箱消掉 clippy，删去过时提示。检查点：`cargo test -p e3d-studio` 14 / 0；`cargo test -p e3d-convert` 11 / 0 + 3 / 0（3 ignored）不变；clippy 回到 S0 基线 3 条、e3d-studio 0 条；doc 0 条。S4 剩余按用户定的顺序：面板取数骨架（多库列表）→ 3.3 → 3.6 → 3.7–3.8 → 3.4–3.5 → 3.10–3.11；S6 可另开会话并行。
- 2026-10-09 夜 S4 续：`b4eec47`（多库列表 `OpenLibraries` / `OpenSet`，「库」页签）、`f91f3e8`（3.3 头信息面板与横幅，打勾）、`9579f9c`（3.6 前半 `TreeModel` 与 Property 2）。细节与之后的步在 `2026-10-10-e3d-studio-m1-s4-s5-plan.md` §1.1、§5。
- 2026-10-10 S4 3.6 完成（打勾），`e3d-studio` `e9c4f91`：`db/browse.rs` 的 `Browse`（每库一个：树模型、在途请求 `Loader`、可见区）、`panels/tree.rs` 用 `uniform_list` 画树、`ElementTree` 上下文的键盘；`cargo test -p e3d-studio` 28 / 0；clippy e3d-studio 0、e3d-convert 3（基线）；doc 0。证据 `e3d-studio/docs/evidence/2026-10-09-e3d-studio-s4.md` 2026-10-10 节与截图 03–05。记录见 `2026-10-10-e3d-studio-m1-s4-s5-plan.md` §5。
