# 计划：写路径第三期收口与下一步开发

状态：**评审通过**（2026-10-08 00:10，Plannotator 门 `approved`，无批注 → Q1–Q5 全按推荐项）→ P0、P1 执行中
日期：2026-10-07
分支 / worktree：`write-path` @ `D:\work\plant-code\old\vendor\e3d-io-write`，HEAD `0244584`；`main` @ `D:\work\plant-code\old\vendor\e3d-io`，HEAD `cf7224d`；两边 `Cargo.toml` 都是 0.4.0。
依据：`.kiro/specs/write-path-phase-3/tasks.md`（第 10、14 组与 Spec_Ledger）、`docs/evidence/2026-10-07-unified-regression.md`、`docs/evidence/2026-10-06-e3d-spatial-gui-attempt.md`、`docs/evidence/2026-10-06-downstream-and-ledger-closeout.md`、`docs/evidence/2026-09-22-e3d-spamap-consequences.md`、`docs/plans/2026-09-30-e3d-studio-gui-design.md`、`main` 上的 `docs/plans/2026-09-28-draft-module-next-steps-plan.md`，以及本次审核的检查（§1.3）。

## 0. 结论

1. 第三期的实现与自动门都已完成：最近一次完整回归（`1b0bbf9`）Cargo 789 / 0 / 30、TTY 39 问、交替写五轮 + Q15、原样 FORMAT 0 11 行全 PASS，存档核验通过。此后只改了回归脚本的打分与核验（`tools/e3d/regression_{checks,selfcheck,verify}.ps1`）和文档，Rust 源码、测试与 `Cargo.*` 没动。剩下的只有 10.5 人工图形门，10.6、14.7 两个检查点因它挂着。
2. 审核找到三件会影响收口结论的事（§1.2 F1–F3）：`write-path` 与 `main` 已分叉；14.5 记的下游失败其实是分叉造成的，不是下游既有错误；10.5 原先的阻断现在能绕开，但现成的图形启动链改过 `LIMITS CE` 且打开的是源项目，门要加反例并改指 scratch。
3. 建议顺序：P0 文档与工作树小修 → P1 把 `main` 合进 `write-path` 并重跑 14.5 → P2 10.5 图形门（加反例）→ P3 10.6 / 14.7 检查点、发版与推送 → P4 下一期（推荐 E3D Studio M1，先做本仓的 T2 小改，细化另起计划过 Plannotator）。

## 1. 审核

### 1.1 已完成且有证据

| 项 | 状态 | 证据 |
|---|---|---|
| 第 1–9、11–13 组 | 全部打勾 | tasks.md |
| 10.1–10.4（Q13 / Q14 门） | 打勾 | `2026-10-04-q13-spatial-gate.md`、`2026-10-05-allocation-q14-gate.md` |
| 14.1–14.6 | 打勾（14.5 的归因见 F2） | `2026-10-06-downstream-and-ledger-closeout.md` |
| 统一回归 `regression.ps1` | 完整真机一轮 PASS，存档核验通过 | `2026-10-07-unified-regression.md` |
| core.dll 布局复核（ida-bridge 钉版） | 写侧无需改动 | `2026-10-07-core-layout-ida-bridge.md` |
| 10.5 人工图形门 | **未完成** | `2026-10-06-e3d-spatial-gui-attempt.md` |
| 10.6、14.7 检查点 | **挂起**（等 10.5） | tasks.md |

### 1.2 发现

- **F1 分叉。** 合并基 `c61b097`（2026-09-24）。`write-path` 比 `main` 多 84 个提交，其中 69 个已在 `github/write-path`、15 个未推送；`main` 比 `write-path` 多 11 个（picture S1–S6、catalogue-pml 两处（含 `DbElementParamEnv::with_derived`）、`DbElement::get_number`、param-env 的 OPAR / ODES / APAR / ADES、PADD 模板登记），也都没推送到 `github/main`。两边都改过的文件只有 4 个：`CHANGELOG.md`、`README.md`、`src/lib.rs`、`tests/common/mod.rs`。
- **F2 14.5 的下游失败归因有误。** closeout 把 `e3d-model` 在 `src/catalogue_point.rs:466` 调不到 `DbElementParamEnv::with_derived` 记成下游既有错误。这个函数在 `main` 的 `3031902` 里（`src/db_param_env.rs:190`），`write-path` 上没有：失败来自 `write-path` 落后 `main`，合并后应当消失。「未由写入改动引入」本身没错，但「与 e3d-io 无关」不对；14.5 要在合并后重跑才算真过。
- **F3 10.5 的阻断与门的有效性。** 10-06 那次 E3D 其实起来了（`gen-model\scripts\e3d\run_ams_gui.bat` 启动链，窗口标题 `AVEVA E3D Design`），卡在 CUA 拿不到窗口绑定。本机现在有 cua-driver（Windows UIA、键入、截图），可以驱动 Design。但这条启动链有两处要处理：
  - `PROJECTS_ROOT` 写死为 `D:\AVEVA\Projects\E3D3.1`，打开的是**源** AMS 项目（也是 aios-database 的数据源）。10.5 要的是受控 scratch 副本，e3d-io 的 BOX 只能写进副本。
  - 它用 `Repair-E3dLimitsCe.ps1` 给 `designviewLimits.pmlcmd` 打 5 处 AIOS 补丁：放行 G3D 视图、空 drawlist 时补 CE、G3D 命令路由到最新 drawlist、limits 作用于该 drawlist 的视图、读 limits 前物化新加的图形。`LIMITS CE` 的命令路径被改过，只做正例时，「不报 (2,536)」说明不了是 SPAMAP 条目起的作用。(2,536) 是元素在 SPAMAP 里没有条目时 E3D 取体积报的错（`2026-09-22-e3d-spamap-consequences.md`；Q13 里 `--spatial off` 写的 Control_Box 作体积就报它），所以门里要加反例。
- **F4 README 自相矛盾。** 「Status」节的 *Not yet* 仍说写侧不做 page-1 bucket-table maintenance（S6b），与下文「Page 1 bucket table」及已落地的 13.3–13.4 矛盾。
- **F5 未推送。** 见 F1：两条分支共 26 个提交只在本机。
- **F6 工作树里的未跟踪物。** `.kiro/skills/`（7 个第三方通用 skill 与 `THIRD_PARTY_NOTICES.md`，9-29 至 9-30 拷入，与本 crate 无关）；`.kiro/specs/e3d-studio-m1/`（只有 `.config.kiro`，10-01 起的空 spec）。
- **F7 警告基线。** clippy 63、`cargo doc` 25 条整期按基线持平，没增也没减。不影响本期验收，记为债。

### 1.3 本次审核做了什么

只读检查：`git status`、`log`、`branch -vv`、`merge-base`、`diff --name-only`（两条分支之间、与远端之间）；两条分支上 `git grep "fn with_derived"`；读 tasks.md 第 10、14 组与 Spec_Ledger、上列证据与 README；读 `run_ams_gui.bat` 与 `Repair-E3dLimitsCe.ps1`（`gen-model` 与 `.room-query-build\gen-model` 两份启动脚本 SHA-256 相同）。

**没有重跑** cargo 与 E3D 门。§0 第 1 条的数字取自 `1b0bbf9` 的存档；它对 HEAD 仍成立，依据是 `git diff --stat 1b0bbf9 HEAD -- src tests examples Cargo.toml Cargo.lock` 为空。

## 2. 计划

### P0 文档与工作树小修（不需用户，可立即做）

- P0.1 改 README「Status」的 *Not yet*：删去写侧 S6b 一项，指向「Page 1 bucket table」与 ADR 0003（F4）。
- P0.2 按 Q4 处理 `.kiro/skills/` 与 `.kiro/specs/e3d-studio-m1/`（F6）。
- 验收：`git diff --stat` 只含上述路径。

### P1 把 `main` 合进 `write-path`（按 Q1）

- 在 `write-path` 上 `git merge main`。不 rebase：84 个提交里 69 个已在 `github/write-path`。
- 冲突只可能落在 F1 的 4 个文件，两边内容并存、不改任何一边的既有语义：`CHANGELOG.md` 的 Unreleased（main 的 Fixed 与 picture 条目、write-path 的 Added 等）；`README.md` 的 Status 与 API 表；`src/lib.rs` 的 `mod` 声明；`tests/common/mod.rs` 的 helper。
- 验收：
  - `cargo test --no-fail-fast`、`cargo clippy --all-targets`、`cargo doc`：失败 0；通过数 ≥ 789 加 main 新增的测试数；skip 清单与警告数和两边基线对照，增量逐条说明来自哪一边。
  - `pwsh -NoProfile -File tools/e3d/regression.ps1 -OfflineOnly` PASS。
  - 重跑 14.5（path override，`e3d-model` 加 9 个 verify 变体，`cargo check --offline`）：预期全部通过，然后更正 closeout 证据与 tasks 14.5 的归因（F2）。若 `e3d-model` 仍失败，按真实原因记录，不改下游源码（需求 16.8）。
- 提交：合并提交只 `git add` 冲突解决涉及的明确路径。

### P2 10.5 人工图形门（按 Q2）

在需求 10 第 11 条的正例之上加一个反例。

- 备库：照 `write_gate.ps1` 的做法从源 AMS 拷一份 scratch 项目（evars 同样改指副本）；用 `e3d-write apply` 依次跑 `write_gate_ops/q13_control.json`（`--spatial off`，写 Control_Box）与 `q13_boxes.json`（`require`，写 ZONE 与其下 BOX）；refno 从 `--report` 取。门前后对源 AMS 目录做哈希清点（沿用 `regression.ps1` 的检查）。
- 启动：把 `run_ams_gui.bat` 复制成门专用的一份放进 `tools/e3d/`，只把 `PROJECTS_ROOT` 改指 scratch，其余启动链不动。证据里记下启动链、shadow 安装路径、`designviewLimits.pmlcmd` 打补丁后的 SHA-256 与补丁 marker 清单。
- 执行与判据（同一 Design 会话）：
  1. 正例：`ADD =<Q13_Box>`，3D 视图里看得见；`LIMITS CE` 取景到它，命令窗口不出 (2,536)。
  2. 反例：`ADD =<Control_Box>`，`LIMITS CE` 应报 (2,536)。反例不报，说明这条图形路径不读 SPAMAP 或被补丁遮掉了，门没有判别力：不记 PASS，回报用户。
  3. 截图至少三张（正例视图、正例命令输出、反例命令输出）放 `docs/evidence/assets/2026-10-xx-e3d-spatial-gui/`，命令窗口原文抄进证据。
- 收尾：只 `QUIT`，不 `SAVEWORK`；确认没有残留进程与写锁。
- 产出：`docs/evidence/2026-10-xx-e3d-spatial-gui-session.md`；用户确认后 tasks 10.5 打勾。
- 失败：正例报 (2,536) 时按需求 16.3 不就地修，先回报用户定范围。

### P3 检查点、发版与推送（按 Q3）

- 10.6、14.7：在 P1、P2 之后的 HEAD 上跑一次完整 `regression.ps1`（真 TTY）与 `regression_verify.ps1 -RequireTTY`；数字记入 Spec_Ledger，tasks 10.6、14.7 与第 10、14 组打勾。
- 发版：CHANGELOG Unreleased 落为版本号，`Cargo.toml` 同步。Unreleased 里有破坏性改动：`TransactionError` 加了 `#[non_exhaustive]`；CLI `--spatial` 默认 `require`，既有调用要显式 `--spatial off`。
- `main` 快进：在 `main` 的 worktree 里 `git merge --ff-only write-path`（该工作树当前干净）。
- 推送 `write-path` 与 `main` 到 `github`：等用户在 Q3 点头，代理不自行推送。

### P4 下一期（按 Q5）

推荐 E3D Studio M1。设计 `docs/plans/2026-09-30-e3d-studio-gui-design.md` 已按推荐默认定（ADR 0001、0002），是本工作树里最新的设计，spec 目录也已建好。

- 先做本仓的 T2（设计 §4.7 第 1–3 条），都是读侧、只加不改，不碰写侧：
  - `schema::project_database_paths` 公开，或另加一个公开的项目库枚举；
  - `ReadOnlyEngine: Send` 的编译期断言测试（同时回答设计 §7 Q2）；
  - 值的 TTY 口径文本渲染公开一个入口（先按设计 §7 Q4 查现状）。
- T1、T3–T8 落在新 workspace `vendor/e3d-studio`，另起计划：把 `.kiro/specs/e3d-studio-m1/` 写成 requirements / design / tasks，过 Plannotator 后再开工。
- 不选的两条：DRAFT P3（新 crate `vendor/e3d-draft`；main 上的计划已批，P0–P2 已落地）可与 M1 并行，但不在本计划；写路径第四期：本期不做项都挂着触发条件，目前一条也没触发，不主动开。

### 依赖

P0 随时可做。P1、P2 都在 P3 之前。P2 不依赖 P1，但放在 P1 之后，这样 10.5 的证据与发版提交是同一套代码。P4 的 T2 在 P3 之后，新接口不混进这次收口。

## 3. 待定问题（每题第一项为推荐）

- **Q1 合并方式。** A（推荐）在 `write-path` 上 merge `main`，之后 `main` 快进；B 把 `write-path` rebase 到 `main`（改写 69 个已推送的提交）；C 暂不合并，先做 P2（14.5 维持现在的归因，发版延后）。
- **Q2 10.5 谁来操作。** A（推荐）代理用 cua-driver 驱动 Design，截图和命令窗口原文交用户确认后才记 PASS；B 用户按 P2 清单手动做，代理只备库、写证据。两种都先把启动链改指 scratch。
- **Q3 发版与推送。** A（推荐）P3 完成后发 0.5.0（Unreleased 有破坏性改动），`main` 快进，用户确认后推送两条分支；B 只推送不发版，版本留 0.4.0；C 暂不推送。
- **Q4 `.kiro` 未跟踪物。** A（推荐）`.kiro/skills/` 不入库，写进 `.git/info/exclude`（只影响本机）；`.kiro/specs/e3d-studio-m1/` 留到 P4 写实后随 spec 一起提交。B 都提交。C 都写进 `.gitignore`。
- **Q5 下一期方向。** A（推荐）E3D Studio M1，先做本仓 T2；B DRAFT P3（`vendor/e3d-draft`）；C 写路径第四期。

## 4. 不做

- 不清 clippy / doc 警告（F7），不改 Golden，不改下游源码，不改 D1–D8。
- 不在源 AMS / TES 项目上做任何写入或图形会话。

## 5. 执行记录

- 2026-10-08：合并后运行 `regression.ps1 -OfflineOnly`。Cargo 汇总 835 passed / 20 failed / 35 ignored；失败均来自当前 AMS 样本 `3F8DED...` 与 Golden 记录样本 `3CAB5D...` 的漂移（以及一个缺失的可选 pinned fixture），没有重录 Golden。Clippy 完成且维持基线范围，doc 完成且产生 26 条文档警告；后续离线阶段因测试失败按入口规则未运行。统一回归证据保留于 `D:\e3dio-regression\20261008-012354-b708857a`。
- 2026-10-08：将缺失 pinned fixture 的单元测试改为安全跳过；定向测试通过。以当前工作树替换 `e3d-io` 和 `e3d-attlib` 路径后，`e3d-model` 主 crate 与 9 个 verify 变体全部 `cargo check --offline` 通过。下游结果保留于 `D:\e3dio-downstream-results-20261008`。
- P1 状态：下游复验完成；完整回归受样本漂移阻断，不能将 10.6/14.7 标为通过。GUI 10.5 仍待办。
