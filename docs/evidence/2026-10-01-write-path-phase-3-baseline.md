# 写路径第三期 · 基线快照（任务 1.1）

测于 2026-10-01，分支 `write-path`。测量时工作树为 `c61b097` 加启动时未提交的改动，测量后单独提交为 `3e16423`，构建内容相同；测量期间未改动 `src/write/` 及任何源码。所有 cargo 命令都设了 `CARGO_TARGET_DIR=target-phase3`（仓库根下）。

原始日志（`cargo test --no-fail-fast -- --show-output` 的 `test.log`、`clippy.log`、`doc.log`）与解析脚本只留在本机 `.agents/tasks/write-path-phase-3-milestone-a/`，不入库。

## 数字

| 项 | 值 |
|---|---|
| HEAD | `3e16423`（= `c61b097` + 启动时未提交改动） |
| 测试二进制数 | 101（lib 单测 1 + bin 单测 2 + `tests/*.rs` 98），另 Doc-tests 1 |
| passed / failed / ignored | 626 / 1 / 30（`cargo test` 退出码 101） |
| clippy | 63 个 warning（去重；cargo 各目标汇总 85，lib test 的 22 个是重复），0 error，退出码 0 |
| `cargo doc` | 25 个 warning（全在 lib doc，private / broken intra-doc link），0 error |

对照 `c61b097` 提交说明的 100 / 622 / 0 / 30：多出的测试二进制与通过数来自启动时未提交改动新增的 `tests/send_sync.rs`、`tests/value_text_round_trip.rs` 等。

## 既有失败（样本漂移）

- `tests/hunk_resolve_real.rs::e3ds_own_int_array_hunk_resolves_to_its_5709_ints`，第 50 行：期望页 16456，实得 16600。
- 在干净的 `c61b097` 临时 worktree 上复跑，同样失败，与未提交改动无关。
- 原因：`ams8000_0001` 原件在 2026-09-28 被 E3D 追加了会话 641–646（文件最后写入 2026-09-30 17:05:57），最新会话里 DBMDAT 24384/26333 的 DBMVAL 链起页从 16456（会话 640 所写）变为 16600。
- 处置（2026-10-03）：测试改用 `ReadOnlyEngine::open_at(AMS8000, 640)`，此后全套为 627 / 0 / 30。

## skip 清单（skip 不计为通过）

| 测试 | 所缺样本 |
|---|---|
| `element_version_census::how_often_is_the_answered_copy_not_the_richest` | `evidence/20260826-tty-sampling/sample.tsv` |
| `sampled_elements_resolve::every_sampled_element_resolves_to_itself_in_the_shape_recorded` | `evidence/20260826-tty-sampling/sample.tsv` |
| `live_tree_sample_resolves::both_storage_layouts_are_covered` | `evidence/2026-08-30-tty-sampling/sample.tsv` |
| `live_tree_sample_resolves::every_sampled_element_resolves_to_itself_in_the_shape_recorded` | `evidence/2026-08-30-tty-sampling/sample.tsv` |
| `live_tree_sample_resolves::the_sample_covers_every_bucket_the_old_list_did_that_a_live_tree_can_witness` | 上述两份 `sample.tsv` |
| `refno_dbno_corpus::every_installed_corpus_key_decodes_to_its_descriptor_dbno` | 环境变量 `E3D_CORPUS_ROOTS` 未设 |
| `world_refno_corpus::every_installed_database_keeps_its_world_where_the_descriptor_says` | 环境变量 `E3D_CORPUS_ROOTS` 未设 |

没有测试经 `common::missing` 报装机样本缺失。30 个 ignored 都是 `#[ignore]` 的全 Corpus 扫描或 scratch 探针。

## 外部依赖

| 依赖 | 路径 | 存在 |
|---|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` | 是 |
| IDA_Bridge | `C:\Users\dpc\.agents-old\tools\ida-bridge`（含 `src\ida_bridge` 与 `.venv`） | 是（只查了目录，未起桥） |
| Corpus 根 | `D:\AVEVA\Projects\E3D3.1` | 是 |
| `ams8000_0001` | `…\AvevaMarineSample\ams000\ams8000_0001`，34 037 760 字节，SHA-256 `3CAB5D7D2B2C40FDF0829B91C555D19D55C530DD49860A4665560E5C553FE023` | 是 |
| `ams7999_0001` | `…\AvevaMarineSample\ams000\ams7999_0001`，59 656 192 字节 | 是 |
| `ams7351_0001` | `…\AvevaMarineSample\ams000\ams7351_0001`，1 238 308 864 字节 | 是 |
| `ams251142_0001` | `…\AvevaMarineSample\ams000\ams251142_0001`，305 152 字节 | 是 |
| `TES023` | `D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023`（另有 `TEST\TESMAC\TES023.mac`） | 是 |
| SPAMAP_Baseline（`ams8000_0001` 会话 640） | 同上 | 原件在；会话 640 之后被追加了 641–646（见「既有失败」），本步未读会话链核实 |
| plant-io 夹具 | `D:\work\plant-code\plant-io\test-file\ams1112_0001`、`attlib.dat` | 是 |

## 给后续步骤

- 此后 `git diff` 以 `3e16423` 为基。
- 用户会用 E3D 在 AvevaMarineSample 原件上做别的工作，金样哈希取决于样本内容。1.4 录金样时，把所用每个样本文件的 SHA-256 与长度写进 Spec_Ledger「Golden 剔除键」表的备注（另见 `docs/plans/2026-10-03-write-path-phase-3-milestone-a-plan.md` §2.2）。
