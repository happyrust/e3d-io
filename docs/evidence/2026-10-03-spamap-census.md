# 取证：SPAMAP 普查（Corpus 五库与 SPAMAP_Baseline）

日期：2026-10-03
任务：write-path-phase-3 3.7
规则：`spamap.key0`、`spamap.bucket`、`spamap.flags`、`spamap.entries_k`、`spamap.coverage`（本文只出普查数据，不坐实任何规则；坐实归任务 7.1、7.2）

> 只读普查，未在 Core_IDB 上取证；「环境」节照模板记录，便于与同期取证对齐。普查命令与口径见 `src/bin/e3d-descriptor.rs` 的 `spamap-census`，输出随附在 `docs/evidence/assets/`。

## 环境

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |
| IDA 版本 | `9.2` |
| HEAD | `f917440f78af02c192a9bfca27d5ae6fb3bad426` |
| Core_IDB SHA-256（末条调用后） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |

普查输出（会话号逐库钉死，重跑逐字节相同；本轮重跑一次核过）：

| 文件 | 字节 | SHA-256 |
|---|---|---|
| `docs/evidence/assets/2026-10-03-spamap-census-corpus.json` | 64 549 | `4C0B30B290A30346484CDE48EA6C5F5B9F5986218FC3D9CB15A9354FCE039F3A` |
| `docs/evidence/assets/2026-10-03-spamap-census-baseline-640.json` | 16 325 | `8FCC6BCB7FAF0824E101A9452E100FC6D497CDD031EA50C6A10D2A00E5D6E5D0` |

## 结论

### 各库读数

| 库 | 会话 | SPAMAP 根页 / 页数 / 条目 | 索引元素 | 悬空条目 | Key0 不符 |
|---|---|---|---|---|---|
| `ams8000_0001` | 640（SPAMAP_Baseline） | 16 309 / 244 / 3 746 | 6 639 | 0 | 0 |
| `ams8000_0001` | 646（普查时最新） | 16 586 / 244 / 3 746 | 6 639 | 0 | 0 |
| `ams7999_0001` | 200 | 28 989 / 1 387 / 23 891 | 34 709 | 0 | 0 |
| `ams7351_0001` | 116 | 无 SPAMAP 表 | 3 345 855 | — | — |
| `ams251142_0001` | 10 | 112 / 15 / 327 | 948 | 0 | 0 |
| `TES023` | 109 | 23 / 13 / 305 | 320 | 0 | 0 |

- `ams8000_0001` 会话 641–646（2026-09-28 E3D 追加）没有动 SPAMAP：646 的三个直方图与 640 逐格相同，只是根页换了。
- `ams7351_0001` 无 SPAMAP 表：它是目录 / 数据库，索引里是 DATA 2 507 075、DTSE 744 948、CATE 89 845、SECT 2 169、TEXT 1 802 个元素，没有几何图元。
- 五库六个会话没有悬空条目：每条条目的 refno 都在所读会话的 refno 索引里。

### `spamap.key0`：普查口径下全部读得通（不坐实）

普查口径：`key0 = N × 10^8 + min`，`min` 为值字里某一轴的有符号 0.1 mm 最小坐标，`N = 6 × 轴（x 0 / y 1 / z 2）+ 桶（1–5）`，口径原文随输出印出。四个有表的库最新会话共 28 269 条、SPAMAP_Baseline 3 746 条，**Key0 不符 0 条**：每条 Key0 都能按这一分解唯一读出轴与桶。这根轴是不是最短轴、并列时取哪根、桶按什么分，不在本口径之内，仍归任务 7.1（`spamap.short_axis`、`spamap.bucket`）。

### 表级直方图（`totals`）

| 库 / 会话 | (轴, 桶) | flags | k |
|---|---|---|---|
| `ams8000_0001` 640 | x1 185 · x2 250 · x3 48 · x4 3 · y1 335 · y2 261 · y3 38 · y4 3 · z1 1 251 · z2 1 321 · z3 43 · z4 7 · z5 1 | 0: 1 083 · 2: 747 · 3: 1 907 · 6: 8 · 7: 1 | 0: 3 625 · 1: 121 |
| `ams7999_0001` 200 | x1 1 104 · x2 1 381 · x3 326 · x4 23 · y1 1 749 · y2 2 163 · y3 467 · y4 41 · y5 1 · z1 9 688 · z2 5 572 · z3 1 357 · z4 19 | 0: 5 608 · 2: 12 476 · 3: 3 854 · 6: 1 856 · 7: 97 | 0: 21 038 · 1: 2 853 |
| `ams251142_0001` 10 | x2 2 · x3 23 · y2 13 · y3 64 · z1 138 · z2 37 · z3 50 | 0: 138 · 1: 61 · 3: 128 | 0: 327 |
| `TES023` 109 | x1 4 · x2 2 · x3 6 · x4 22 · x5 26 · y1 28 · y2 6 · y3 26 · y4 14 · y5 21 · z1 9 · z2 57 · z3 30 · z4 36 · z5 18 | 2: 305 | 0: 305 |

给 7.1 / 7.2 的线索（只是观察）：flags 只见 0、1、2、3、6、7 六个值，1 只在 `ams251142_0001`（电缆桥架库）出现，`TES023` 全为 2；k = 1 的条目只在 `ams8000_0001` 与 `ams7999_0001`；`TES023` 五个桶在三根轴上都出现，最适合拟合桶界。逐 noun 的元素条目数（0 / 1 / 2 / 3+）、(轴, 桶)、flags、k 与按元素的 k 组合见两份 JSON 的 `nouns`；例如会话 640 的 BOX 45 个、CYLI 34 个都各 1 条，EQUI 77 个全为 0 条，与 2026-09-22 取证一致。

### 更正 `docs/evidence/2026-09-22-e3d-spamap-consequences.md` §0 第 7 条

- 该条的 N 直方图合计 3 746、flags 直方图合计 3 747，两者不是同一张表。本普查在原件会话 640 的同一张表上读：(轴, 桶) 与 flags 都是 3 746 条（上表第一行）。
- 该条的 flags 直方图正好是会话 640 再加一条 flags 2：门用副本会话 648 里 E3D 给 e3d-io 写的 BOX 补的那条（值字末位 2，见该文 §0 第 5 条）。即它取自副本会话 648 的表。
- 该条的 N 直方图与会话 640 有两格不同（x3 49 对 48、z1 1 250 对 1 251），合计却仍是 3 746，与 640、648 两张表都对不上。副本每轮从原件重置，会话 648 已无从复核；以本普查的会话 640 读数为准。

## 复现命令

在仓库根执行；会话号逐库钉死，原件日后再被 E3D 追加会话也照样逐字节重现。

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
cargo run --release --quiet --bin e3d-descriptor -- spamap-census --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7351_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001 --input D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001=646 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001=200 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7351_0001=116 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001=10 --session D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023=109 > docs/evidence/assets/2026-10-03-spamap-census-corpus.json
cargo run --release --quiet --bin e3d-descriptor -- spamap-census --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001=640 > docs/evidence/assets/2026-10-03-spamap-census-baseline-640.json
powershell -NoProfile -File tools/ida/ib.ps1 env
```

## 反例

无：五库六个会话的悬空条目与 Key0 不符均为 0。
