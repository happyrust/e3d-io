# 计划：DRAFT 模块开发下一步（读法 3 第十三轮之后）

状态：**评审通过**（2026-09-28，会话 fable-5-1-2：用户在面板上批「Q1–Q9 全按推荐项定」，Plannotator 门 `{"decision":"approved"}`）→ **P0 已落地**（§7 2026-09-28 条）→ **P1 待开工**。起草同日，起因是用户点「读会话 5a4694a1 + 用 Plannotator 制定 draft 模块下一步方案」。拍板结果：Q1 `e3d_io::picture` 顶层 · Q2 样例路径夹具 · Q3 只编回 · Q4 新 bin `e3d-picture` · Q5 新 crate `vendor/e3d-draft` · Q6 P0→P1→P2→P3→P4 · Q7 SVG + resvg · Q8 `draft31` Python 冻结为 oracle · Q9 本期只做读 + 合成。
日期：2026-09-28
分支 / worktree：`main` @ `D:\work\plant-code\old\vendor\e3d-io`，HEAD `b7c4860`（0.4.0 + OPAR/ODES/APAR/ADES）。工作树里 `src/db_param_env.rs`、`src/record/catalogue_pml.rs` 是别的会话（GENSEC 檐口 ③）的未提交改动，本计划不碰；`docs/plans/2026-09-24-gw-picture-read-plan.md` 仍是未跟踪文件。
依据：Cursor 会话 `5a4694a1-e2dd-4700-9ff3-59ba3f2917ff`（fable-5-1-14，2026-09-24：第十三轮进度盘点 + 颜色表 + GW 移植计划草案，随后转去 gen-model 部署再没回来）；`D:\ida_scratch\draft31\REPORT-*.md` 第一～十三轮；`GW-PICTURE-FORMAT.md`；`docs/plans/2026-09-24-gw-picture-read-plan.md`（下称「GW 计划」）；`docs/evidence/2026-09-22-core-dll-record-draft-data-flow.md`。
影响仓库：`vendor/e3d-io`（新增 `picture` 模块 + `PADD` 模板登记）；**新 crate `vendor/e3d-draft`**（Q5 定）；`D:\ida_scratch\draft31\`（参考实现，冻结为 oracle，Q8 定）。下游 gen-model / e3d-model 不受影响（纯增量）。

---

## 0. 一句话

DRAFT 的逆向已经**解完能解的三层**（PADD 库定义 → GW 图片文件 → 缩略图）并有可复现的硬判据（2 766 个图片文件逐字节回写 0 差异；Plant 79 + Marine 702 张 sheet 离线合成，与 DRAFT 自己缓存的 DPOSCA 位置 12 807 + 32 201 项 0 例外、智能文字重算 462/462 + 91/91 全同；颜色表从 core.dll 读出并被 DRAW 自存缩略图逐像素坐实）。**但一行都没进仓库**：实现是 `D:\ida_scratch\draft31\` 里 ~5 000 行 Python + 一个 739 行的 Rust 探针（对着 e3d-io 的 git-archive 快照编），09-24 起四天没有任何进展，GW 移植计划的 Q1–Q4 也没人拍板。下一步不是再开第十四轮逆向，而是**把已证的东西搬进仓库、立门、定边界**：P0 收口（半天）→ P1/P2 `e3d_io::picture`（GW 计划 S1–S6，4.5 天）→ P3 `e3d-draft` PADD 语义层（3 天）→ P4 合成渲染（2 天）；逆向遗留项（P5）只在验收撞到时按需开轮。写侧（PADD 写回、图片再生成）本期不做，只写清触发条件。

---

## 1. 现状（2026-09-28 实读）

### 1.1 已证（会话 5a4694a1 §1 盘点，本次复核数字出处）

| 层 | 解到什么程度 | 硬判据 | 实现在哪 |
|---|---|---|---|
| PADD 库（Dabacon，`padvir.dat`） | `WORL → DEPT → REGI → DRWG → SHEE → {VIEW(LAYE→VNOT/SLAB/GLAB/LDIM…), NOTE, THUMBN}`；2D 几何 = 点元素图（`PTRF / PTFA → PPOS.XYPS`）；视图只存相机 + 内容规则（`IDLN→IDLI→ADDE`）+ 表达规则引用（`RRSF/HRSF`）；智能文字 `BTEX` 模板 + `EVITCA` 缓存；`DPOSCA` 位置缓存；Marine 加 `TABL / ARC / GRDRLR / PDIM / IMGELE` | Plant 61 库 328 属主、Marine 20 库 720 属主全部列出；DPOSCA 比对 12 807 + 32 201 项 0 例外；文字重算 462/462 + 91/91；OCOD 0/1/4/6 四个原点码由 DPOSCA 坐实 | `draft31\paddprobe\src\main.rs`（739 行 Rust，依赖 e3d-io 快照 `5245654`）+ `anno\annodraw.py`（1 570 行，数据层与画法混在一起） |
| GW 图片文件（Wigwam GTIGS） | 物理层 / 头 / 名字 / 属性表 / 图片表 / 目录回放 / 段 / 组 / 10 种图元逐字段；TEXT attr 混合基数、TEXT2 转义 run、弧 `w = bulge²` | **2 766 文件（Plant 881 + Marine 1 885）、24 873 647 图元解码再编码逐字节 0 差异**；58 629 条弧几何零例外 | `draft31\gwdump.py`（42 KB）、`gw\gwpic.py`（424 行）、`gw\gwrender.py`（237 行） |
| 颜色 / 笔 | 365 项颜色表 = core.dll `fmdevice/FWSCOL`（0x533B140）静态数据 + DruidNet `ColourTableManager` 字典；> 365 = 层号；笔 8001–8045 / 8000 / 8100–8115 | DRAW 自存缩略图图框像素 30/30 = (130,130,130) = int(0.51×255)；9 个可见色主簇夹角 ≤ 1.5° | `gw\draft_colours.py`（221 行）+ `draft_colours.tsv`、`gw\colourverify.py` |
| 缩略图 | `THUMBN → ARCHIV → ATTDAT.ELDATA` = `[4][总字节][UUID×4][载荷]`，载荷按字小端，BinaryFormatter `Bitmap` → PNG 128×128 | 30 个 THUMBN 全解出 | `paddprobe eldata` + `gw\thumbs.py` |
| 整张 sheet 合成 | GW 视图图形（按 VIEW 框裁剪）+ 背景 BACK 从 PADD 重画（含 Marine acp000 背景库，723 个 INSTANCE 全落到 BSRF→BACK 的 NOTE）+ 注释重画（TEXP/STRA/OUTL/RECT/CIRC/ARC/ETRI/TABL/SYMB/SLAB/GLAB/LDIM/GRDRLR/VNOT）→ JSON / SVG / PNG | Plant 79/79 ok；Marine 702 → 700 ok + 2 无图片，0 错误；drawn / ignored 计数表（第十二轮 §5.1）可复跑 | `anno\annodraw.py` + `anno\batch_sheets.py --project plant|marine` |
| 处理链 | draw.exe(.NET 壳) → `PaddAdapter` → `DraftInterfaceImpl` → **Core3D.dll**（DRAFT Fortran：sheetman / updview / add / build / buildanno / annotate / dimact / intext / replib / cpsman）→ core.dll Wigwam（`GSVPI0` 写盘、`PVNO +1`） | `core3d_modules.csv`、sheetman 9 例程伪码；IDB 全在线（`idalib-4000` core、`idalib-49136` Core3D） | `dec\*.c`、`il\`（ILSpy） |

### 1.2 没进仓库的证据

- e3d-io `src/` 无 `picture` / `draft`；`schema/e3d31/manifest.json` 的 `templates` 只登了 `DESI / CATA / DICT`，`binaries/padvir.dat`（985 KB）在但没登 `PADD`；`docs/evidence/` 只有 09-22 的记录草稿数据流一篇与 DRAFT 沾边。
- `git log` 自 `0ae6364`（0.4.0）起没有任何 DRAFT 提交；GW 计划文件未跟踪、Q1–Q4 未拍板（会话最后一条建议「回到 DRAFT 课题：Q1–Q4 按推荐项定，开工 S1」没有回音）。
- `draft31\` 没有第十四轮；09-28 22:41 全目录 mtime 被批量刷过一次（同步），内容与 09-24 相同。

### 1.3 遗留（逆向侧，来自第十二 / 十三轮 §3 / §5）

| 项 | 现状 | 谁会撞到 |
|---|---|---|
| p-point 派生值（`#P1BOR / #P0POSX …` 58 个码） | 只能用 `EVITCA` 缓存，重算不出（Plant 文字 58 解不出） | P3 智能文字 |
| UDA 名表 | 未读 | P3 |
| OCOD 2 / 3 / 5 / 7 / 8 | 无样本（0 / 1 / 4 / 6 已证） | P3 RECT / TABL |
| ADIM / RDIM / PDIM 画法、GRDRLR 刻度长 / 字高 / 标签取向、ARC 方向符号、网格气泡 ADEG = 90 取向 | 近似值，无实证 | P4 |
| 字宽 / 字体、线型实宽（`GTLSWI`） | 不在文件里 | P4（保真度，不是正确性） |
| 设备色 297 / 301 / 304（`FWSQGN` 运行时） | 未取 | P4（背景 / 高亮色） |
| 注释 `PALETTE` 与颜色表不一致（3/4/5/6/7 错、8/16/29/160/296 缺，约 5 000 图元）、`add_gw` 全黑固定线宽、`is_unset` 负 int、无 baseline 自动 diff | 第十二轮 §5.7 审核抓出，未改 | P0（PALETTE 一行改）/ P4 |
| 视图生成链（Core3D `UVIEW / ADREM / RRRGET / HLR`）、Marine `.sdb`、孤儿图片 553 / 1 167 分类 | 未开 | 写侧 / 出图再生成才需要 |

---

## 2. 「DRAFT 模块」是什么（边界）

三档，本期只做前两档：

| 档 | 交付 | 落点 | 本期 |
|---|---|---|---|
| **① 读** | 不开 E3D：打开工程 → 列 DRWG / SHEE → 读 sheet 定义（视图相机、图层、注释、点图、智能文字模板与缓存）→ 找到并解开图片文件（图纸 mm 坐标、笔 / 颜色语义）→ 缩略图 PNG | `e3d_io::picture`（文件格式）+ `e3d-draft::padd`（PADD 语义） | 做（P1–P3） |
| **② 合成** | 离线复现整张 sheet：GW 视图图形 + 背景 + 注释 → JSON / SVG / PNG，以 DRAFT 自己的 `DPOSCA / EVITCA` 缓存与缩略图为真值 | `e3d-draft::{sheet, render}` + CLI | 做（P3–P4） |
| **③ 写 / 生成** | 写回 PADD（`e3d_io::write::Transaction` 对 PADD 库理论上已能用，缺 PADD 模板 + E3D DRAW 读回门）；图片再生成（视图生成链 + Wigwam 写出）；Marine `.sdb` | 另立计划 | **不做**；触发条件：用户要「改图纸后 E3D 打开能看」或「不开 E3D 出新图」 |

**不做**（写清楚，免得当成漏了）：字体 / 字宽保真、线型实宽、INSTANCE 跨图片解析、E3D 读回我们写的任何东西、`.sdb`、孤儿图片分类、视图生成链细审。

---

## 3. 切片

| 步 | 内容 | 验收（每步单独绿） | 估时 |
|---|---|---|---|
| **P0 收口** | (a) Q1–Q9 拍板；GW 计划 + 本计划 `git add docs/plans/<两个文件>` 提交；(b) `manifest.json` `templates` 加 `"PADD": "padvir.dat"` + `tests/padd_open.rs`：用 pack 打开 `aps250133`（`D:\AVEVA\Projects\E3D3.1\AvevaPlantSample\aps000\`），断言 4 923 元素、50 种名词、12 SHEE、`/APS-0001/1` 的 `EXFI = 10000`、`PVNO = 11`（第一轮 §2.1 的数字），缺文件走 `common::missing` skip；(c) `GW-PICTURE-FORMAT.md` 搬进 `docs/evidence/2026-09-28-gw-picture-format.md`；(d) `draft31\anno\annodraw.py` 的注释 `PALETTE` 改走 `gw\draft_colours.py`（第十二轮 §5.7 那约 5 000 个图元），Plant / Marine 两个 batch 的 drawn / ignored / DPOSCA / 文字计数不变；(e) `remember` + `record_decision`（Q5 / Q8 / Q9 是约束别的会话的约定） | (b) 测试绿；(d) 计数与第十二轮 §5.1 逐项相同；`git status` 只多我们的文件 | 0.5 d |
| **P1 `e3d_io::picture` 读 + 编回** | GW 计划 S1–S3：物理层 / 头 / 名字 / 图片表 / 目录回放 → 段 / 组 / 10 种图元 / TEXT attr / TEXT2 run → `encode()` 编回 + `roundtrip` | `picture_small`：31 段 / 655 图元逐条 = `gwdump.py --list --raw`；`picture_big`：351 819 条 kind 直方图 = 第八轮；**`picture_corpus`：2 766 文件 0 失败、10 种 kind 计数 = 第八轮的 10 个数字（本步的门）** | 3 d |
| **P2 `e3d_io::picture` 语义 + 定位 + CLI** | GW 计划 S4–S6：变换复合 / 可见性 / 弧展开、笔表、365 项颜色表；`locate(sheet)` / `thumbnail(sheet)`；`e3d-picture info / list / roundtrip / locate / thumbnail`；README / CHANGELOG | 58 629 条弧零例外；`picture_style` = `draft_colours.tsv`；`--page` 抽样 1 000 行与 `lists\M2013286677-265-10000-11.page.csv` 逐值相同（1e-9）；Plant 328/328、Marine 718/720；30 个缩略图字节 = `gw\thumbs\*.png`；全套测试不少于 0.4.0 的 101 / 622 / 0 / 30；clippy 新模块 0 warning | 1.5 d |
| **P3 `e3d-draft` PADD 语义层** | 新 crate `vendor/e3d-draft`（依赖 e3d-io，与 e3d-model 同构）。模块：`padd`（`Project::open(db_dirs)` 按 `DDNM` 顺着开 PADD / DESI / CATA 库；typed 视图 `Sheet / View / Layer / Note / Texp / Stra / Outl / Rect / Circ / Arc / Etri / Tabl / Symb / Slab / Glab / Ldim / Grdrlr / Vnot / Back`；每个 noun 的字段清单以 `annodraw.py` 的读法为准、以 `padvir.dat` 描述子为形）、`points`（`PTRF / PTFA / PSTF → PPOS / DPPT / VRTX / DPOI` 点图，`OCOD_ORIGIN` 表：0 中心、1 左上、4 左下、6 左中，其余码记 note）、`intext`（`BTEX` 模板求值：`#NAME / #FLNM / #TYPE / 属性 / (C2:) 切片 / <FR X> / <FROM X> 名词框架`；p-point 派生码回落 `EVITCA` 并标 `unresolved`）、`check`（`DPOSCA` 位置、`EVITCA` 文字两张对照表 → `CheckReport`）、`sheet`（合成模型：视图框 + 视图图形引用 + 背景 + 注释项，纯数据，不画）。CLI `e3d-draft sheets --project <dir…>`、`export <sheet> --json`、`check --project …` | `export --json` 对 Plant 79 + Marine 702 张与 `annodraw` 的 JSON **结构归一后逐项相同**（drawn / ignored 计数表 = 第十二轮 §5.1）；`check`：DPOSCA 12 807 + 32 201 项 0 例外、文字 520/462/0/58 + 91/91/0/0（与 Python 版逐字相同）；含 acp000 背景库时 Marine 723 个 INSTANCE 全部解到 BACK 的 NOTE 成员 | 3 d |
| **P4 `e3d-draft` 合成渲染** | `render`：`Canvas`（SVG 直写，PNG 经 resvg 或 tiny-skia，Q7 定）；GW 视图图形按 VIEW 框裁剪、BACK 从 PADD 重画时跳过 GW 的 BACK 段、注释按 P3 的 `sheet` 模型画；颜色全走 `e3d_io::picture::Colour`；`e3d-draft render <sheet> --svg/--png [--bg black|white]`、`batch --project …` | Plant 79/79、Marine 700/702 出图 0 错误；SVG 图元数与 `annodraw` 逐 sheet 相同、坐标 1e-6；30 对缩略图 ↔ 渲染的主簇色相夹角 ≤ 1.5°（沿用 `colourverify` 方法）；第十二轮 §2 的四张目视样张（`Engine_Room_Layout-Equipment_S2`、`Engine_Room-Large_Layout_Drawing_S1`、`DRA_PRJ_TMP_ADP-PGA_22x17_S1`、`DRA_PRJ_TMP_PIPING_36x24_S1`）并排对照存档 | 2 d |
| **P5 逆向遗留（按需）** | §1.3 里 P3 / P4 会撞到的项，撞到再开一轮（每轮一份 `REPORT-…-第 N 轮.md`，证据仍进 `draft31\`，结论进 `docs/evidence/`）：p-point 派生值 → Core3D `intext` 的 `#P` 分支；OCOD 未见码 → `dra_qutil` 原点表；ADIM / RDIM / PDIM → `dimact`；设备色 → attach draw.exe 读 `unk_61ED260` | 每轮结论进对应的 P3 / P4 测试 | 每轮 0.5–1 d |

合计 P0–P4 约 **10 个工作日**；P1 → P2 串，P3 可与 P1 并行（P3 的 DPOSCA / 文字门不需要图片），P4 依赖 P2 + P3。每步一个提交（e3d-io `feat(picture): …` / `test(padd): …`；e3d-draft 自己的仓）。共享工作树里只 `git add` 自己的路径，别的会话的 `db_param_env.rs / catalogue_pml.rs` 不进暂存区。

**验收总则**：Rust 版与 `draft31` 的 Python 参考实现在同一输入上**逐项相同**——Python 版自己已被 2 766 文件逐字节回写、DPOSCA 45 008 项、EVITCA 553 条重算证过，所以「与它相同」就是「与 E3D 写出的字节 / 缓存相同」；任何不同先查 Python 是不是错（第八轮 TEXT2、第十二轮 OCOD 6 都是这样纠出来的）。渲染保真（字体 / 线宽）不在门里。

---

## 4. `e3d-draft` 骨架（Q5 按推荐项时）

```
vendor/e3d-draft/
  Cargo.toml            e3d-io = { path = "../e3d-io" }；可选 resvg（png feature）
  src/lib.rs
  src/padd/{mod,project,nouns,attrs}.rs   Project::open；typed 视图；DDNM 跨库
  src/points.rs                            点图 + OCOD 原点
  src/intext/{mod,template,resolver}.rs   BTEX 求值 + 设计名词框架
  src/check.rs                             DPOSCA / EVITCA 对照 → CheckReport
  src/sheet/{mod,view,back,anno}.rs        合成模型（纯数据）
  src/render/{mod,canvas,svg}.rs           P4
  src/bin/e3d-draft.rs                     sheets / export / check / render / batch
  tests/common/mod.rs                      PLANT_PADD_DIR / MARINE_PADD_DIR / ACP000_DIR / *_PIC_DIR + missing()
  tests/padd_plant_small.rs                /APS-0001/1：12 SHEE、VIEW 相机、DPOSCA 全对
  tests/padd_corpus.rs  #[ignore]          79 + 702 张 export/check = Python 版
  tests/render_corpus.rs #[ignore]         P4 门
  docs/evidence/…                          从 draft31 搬入的格式 / 语义结论（指回 Core3D idalib-49136 地址）
```

- 夹具口径与 e3d-io 一样：AVEVA 样例数据不入库，路径常量 + `missing()` skip，全量门 `#[ignore]`。
- 不复制 e3d-io 的 Dabacon 读法：所有元素与属性都经 `DbSet / DbElement` 与 `padvir.dat` 描述子读；本 crate 只加 DRAFT 语义。
- Python 参考实现冻结（Q8）：`annodraw.py` 只修 bug、不加功能；P3 / P4 逐项对齐后退役为 oracle。

---

## 5. 风险

| 风险 | 处置 |
|---|---|
| `annodraw.py` 1 570 行数据层与画法纠在一起，搬 P3 时容易把画法假设当成数据事实 | P3 先只搬「读 + check」，`export --json` 与 Python 对齐为门；画法全部留到 P4，且 P4 的门是「与 Python SVG 相同」而不是「好看」 |
| PADD 语义里有近似（GRDRLR 2 mm / 2.5 mm、OCOD 未见码、ADEG 取向） | 原样搬并在 `CheckReport.notes` 里点名；不在 P3 / P4 里「顺手修正」，修正走 P5 开轮拿证据 |
| 共享工作树并发（e3d-io 里别的会话未提交的 2 个文件；`draft31` 里曾有两个会话同时跑 batch 把 `batch.tsv` 截断） | e3d-io 只加新文件 + `lib.rs` 一行 + manifest 一行；跑 `batch_sheets.py` 前 `Get-CimInstance Win32_Process \| ? CommandLine -match batch_sheets`；e3d-draft 是新仓，没有并发 |
| `paddprobe` 依赖 e3d-io 快照 `5245654`，与 0.4.0 公开面有漂移（`TextList`、hunk 自动解） | 不修 paddprobe；P0 的 `padd_open` 测试直接用当前 e3d-io，P3 起 paddprobe 退役 |
| IDA bridge / idalib 实例不在线时 P5 开不了轮 | P5 是按需项，不阻塞 P1–P4；开轮前 `ib list` 确认 `idalib-4000 / 49136` |
| 估时按单人串行；P3 若与 P1 并行由两个会话做，`e3d_io::picture::Colour` 是两边共用件 | 颜色表先落 P0（`draft_colours.tsv` 直接生成 Rust 数据文件进 `picture/style.rs`），两边都引它 |

---

## 6. 要评审定的（推荐项在前）

GW 计划的四条照搬，再加五条：

- **Q1 `picture` 模块位置**：(a) **`e3d_io::picture` 顶层**；(b) `e3d_io::draft::picture`。推荐 (a)。
- **Q2 夹具**：(a) **样例工程路径常量 + `missing()` skip，corpus `#[ignore]`**；(b) 提交 42 KB 小文件进 `tests/fixtures/`。推荐 (a)。
- **Q3 写侧边界**：(a) **只做编回验收**；(b) 顺手做新图片写入器。推荐 (a)。
- **Q4 CLI**：(a) **新 bin `e3d-picture`**；(b) 挂 `e3d-descriptor picture …`。推荐 (a)。
- **Q5 PADD 语义层 + 合成的落点**：(a) **新 crate `vendor/e3d-draft`**（依赖 e3d-io；与 e3d-model 同构——e3d-io 只管文件与格式，DRAFT 语义与渲染在上层；e3d-io MISSION 明写「下游渲染」不在范围）；(b) 全进 e3d-io `e3d_io::draft`。推荐 (a)。
- **Q6 顺序**：(a) **P0 → P1 → P2 → P3 → P4**（自底向上，每层硬门；P1 的门最硬最便宜，P3 可并行）；(b) P0 → P3 → P1 / P2 → P4（先搬 annodraw 数据层）。推荐 (a)。
- **Q7 PNG 光栅**：(a) **P4 只出 SVG + 经 `resvg` 出 PNG（feature 可关）**；(b) 自写光栅（tiny-skia 直画）。推荐 (a)——门是结构相同，不是像素相同。
- **Q8 参考实现地位**：(a) **`draft31` Python 冻结为 oracle（只修 bug）**，Rust 逐项对齐后退役；(b) 继续在 Python 上加功能再同步。推荐 (a)。
- **Q9 本期目标口径**：(a) **只做「不开 E3D 读出并复现图纸」（① 读 + ② 合成）**，③ 写 / 生成另立计划、写清触发条件；(b) 本期就把 PADD 写回（`Transaction` + `padvir` 模板 + DRAW 读回门）纳入。推荐 (a)。

Q1–Q9 全按推荐项定的话，P0 当天做完、P1 当天开工。

---

## 7. 记录

- 2026-09-28：草案（fable-5-1-2）。起因：读会话 5a4694a1 发现 09-24 第十三轮之后 DRAFT 侧零进展、GW 计划 Q1–Q4 悬而未决、全部实现仍在 `draft31\` 未入库。
- 2026-09-28（同日）：评审通过（面板「Q1–Q9 全按推荐项定」+ Plannotator `approved`），**P0 落地**：
  - (a) 两份计划状态行改「评审通过」，与 manifest / 测试 / 证据 / CHANGELOG 一起提交（本条所在的提交）。
  - (b) `manifest.json` `templates` 加 `"PADD": "padvir.dat"`；`tests/padd_open.rs` 2 条绿：`SchemaPack::template("PADD")` 解到 pack 里的文件；`aps250133_0001` 经 `DbSet::add_db(db_type PADD)` 打开 → dbno 250133，`ScanTier::Header` 普查 **4 923 元素 / 50 名词 / 12 SHEE**，`find_named("/APS-0001/1")` 唯一，属主链 DRWG `/APS-0001` → REGI → DEPT，VIEW 里有 `VSCA = 0.01`——全部与第一轮 `paddprobe` 的数字相同。**一个发现**：`padvir.dat` 把 `EXFI / PVNO` 的数据类型定成 word（`get_integer` 报 `TypeMismatch{found: Word}`），要读 `DescriptorValue::Word{raw}` 的 `raw`（paddprobe 的 `word_attr` 一直如此）；P3 的 typed 视图要照这个形。`cargo test --release --lib schema::` 14 过；clippy 对新测试 0 warning。
  - (c) `GW-PICTURE-FORMAT.md` 搬入 `docs/evidence/2026-09-28-gw-picture-format.md`（正文逐字相同，前加来源 / 判据 / 参考实现头）。
  - (d) `draft31\anno\annodraw.py`：8 项手写 `PALETTE` 改为 `colour_hex()` 走 `gw\draft_colours.rgb255`（1 与 10 在白纸上仍画黑，近白色 `min(rgb) ≥ 240` 画黑，> 365 回黑）。回归：Plant 79/79 ok、DPOSCA 12 807 / 0、文字 520 / 462 / 0 / 58、GW 图元 353 220（59 s）；Marine 702 → 700 ok + 2 no-gw、DPOSCA 32 201 / 0、文字 91 / 91 / 0 / 0、GW 765 421（219 s）；两份 `batch.tsv` 除耗时列外**逐格相同**（改前副本在 `%TEMP%\r14-before\`）。SVG 里注释描边色从只有 `#000000` 变成表值：Plant `#bc0000`(2 red) 1 145、`#0000ff`(7 blue) 10、`#683400`(9 brown) 9；Marine `#bc0000` 1 454、`#00bc00`(5 green) 883、`#ffff00`(4 yellow) 18。
  - (e) 共享记忆 `mem-13`（PADD 登记、EXFI/PVNO 是 word、Plannotator 用法）；共识 `d-11`（Q5 / Q8 / Q9 等约束其他会话的定案）。
  - 下一步：P1 = GW 计划 S1（`src/picture/{mod,word,file,directory}.rs` + `tests/picture_small.rs`）。
