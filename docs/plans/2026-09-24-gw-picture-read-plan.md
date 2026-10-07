# 计划：e3d-io 读 DRAFT 图片文件（GW / Wigwam GTIGS picture）——把 `gwdump.py` 移植成 Rust 模块

状态：**评审通过**（2026-09-28，会话 fable-5-1-2：用户在面板上批「Q1–Q9 全按推荐项定」并在 Plannotator 门 Approve——Q1 `e3d_io::picture` 顶层、Q2 样例路径夹具 + corpus `#[ignore]`、Q3 只编回不写新图、Q4 新 bin `e3d-picture`；上位计划 `docs/plans/2026-09-28-draft-module-next-steps-plan.md` 把 S1–S3 编为 P1、S4–S6 编为 P2）→ **S1–S6 全部落地**（2026-09-28/29，会话 fable-5-1-8，提交 `7bbc5cf` S1 / `39bc8f8` S2 / `5c6e604` S3 / `6e3c07a` S4 / `c4160d2` S5 / S6 见 §7 末条）：**2 766 文件 roundtrip 0 失败、10 种 kind 计数 = 第八轮、58 629 条弧零例外、365 色 = tsv、Plant 328/328 + Marine 718/720、57 张缩略图**——本计划完成。起草：2026-09-24，会话 fable-5-1-14，用户点选「先给范围与计划」。评审要定的四件事在 §6（Q1–Q4）。
日期：2026-09-24
分支 / worktree：`main` @ `D:\work\plant-code\old\vendor\e3d-io`，HEAD `0ae6364`（0.4.0）。本计划只加新文件，不碰写路径。
依据（全在 `D:\ida_scratch\draft31\`，本计划落地时把格式说明搬进 `docs/evidence/`）：`GW-PICTURE-FORMAT.md`（格式说明，§3.3 颜色条目 2026-09-24 已按二进制改写）、读法 3 第二～八轮报告（core.dll `sub_54DC674` op 6 / op 9 成帧、G-API 逐图元编解码）、第十三轮报告（颜色表、进度盘点）、参考实现 `gwdump.py` v4（937 行，解析 + 回写 + 清单）、`gw\gwpic.py`（几何：变换复合、弧展开）、`gw\draft_colours.py`（颜色表）。
影响仓库：`vendor/e3d-io`（本仓库）新增模块 + 新 bin；下游（gen-model / e3d-model 及 verify 变体）不受影响（纯增量，`cargo check` 复核即可）。

---

## 0. 一句话

DRAFT 每张 sheet 的**视图图形**不在 PADD 库里，在工程 `pic\NN\M<w0>-<w1>-<EXFI>-<PVNO>` 这个 Wigwam 图片文件里；格式已在 12 轮逆向里解到**逐字节**（Plant 881 + Marine 1 885 = 2 766 文件、24 873 647 图元解码再编码零差异），但实现是 `D:\ida_scratch\draft31\gwdump.py` 一份 Python 脚本，e3d-io 里一行都没有。本计划把它做成 `e3d_io::picture`：**读**（文件 → 图片 / 段树 / 组 / 图元，带图纸毫米坐标、笔 / 颜色语义）+ **为验收而写**（每个解出的项能原样编回去，作 2 766 文件 roundtrip 门），外加一个 `e3d-picture` CLI 和从 PADD 库的 SHEE 找到图片文件的那一步。**不做**注释重画（那是 DRAFT 语义层，留在 `anno\annodraw.py`）、不做 SVG / PNG 渲染、不做让 E3D 读回我们新写的图片。

---

## 1. 现状

| 项 | 数字 / 事实 | 出处 |
|---|---|---|
| 格式覆盖 | 物理层（128 B 记录、8 B 字 = 两个大端 4 B 单元）、头 21 字、名字块、属性表、图片表（8 字 / 图，纸面窗口）、目录（7 字 / 条，倒序 DFS 回放）、段（flags / 6 实数变换 / 用户块）、组、**10 种图元**（TEXT 1、TEXT2 10、MARKER 2、LINE 3、CONIC 4、FILL 5、INSTANCE 6、FILLCONIC 7、BOUNDARY 8、BOUNDCONIC 9）逐字段；TEXT `attr` 字混合基数 → `WWTextFormat` 串；TEXT2 转义 run；弧 `w = bulge²` 顺时针扫 | `GW-PICTURE-FORMAT.md` §1–§3；第八轮 §1–§2 |
| 判据 | `gwdump.py --roundtrip`：2 766 文件、24 873 647 图元**逐字节回写 0 差异**，10 种 kind 全部有真数据（TEXT 23 776、MARKER 20 337 800、LINE 4 235 315、CONIC 226 942、FILL 4 518、INSTANCE 5 481、FILLCONIC 148、BOUNDARY 1 960、BOUNDCONIC 176、TEXT2 37 531）；58 629 条弧的中点全在弦的垂直平分线上且 `|M − mid| / 半弦 = √w` 零例外 | 第八轮 §0.2、§5；格式说明 §3.2 |
| 与 PADD 的对应 | 段用户块 `[角色词哈希][w0][w1]…`，角色词 = Dabacon base-27 名词哈希（`SHEE / VIEW / DESI / GEOM / PPTS / PLIN / TEXP / BNTE …`），GEOM 段组 id k = 第 k 个元素（2 350 段零例外）；文件名 `<w0>-<w1>` = 属主 SHEE / OVER 的 refno，`NN = w1 mod 32`，`EXFI` = 抽取库号、`PVNO` = SHEE 的图片版本号，两工程 328 / 328、718 / 718 命中 | 格式说明 §3.1；第一轮 §2.2 |
| 颜色 / 笔 | 颜色号 → 名 + RGB 的 365 项表从 core.dll `fmdevice/FWSCOL` 静态数据 + DruidNet `ColourTableManager` 字典读出，DRAW 自存缩略图图框像素 30/30 = (130,130,130) = int(0.51 × 255)、9 个可见色主簇夹角 ≤ 1.5°；笔 8001–8030 = `8000 + 线型 + 10 × (宽 − 1)`，8031–8045 用户线型，8000 填充，8100–8115 系统笔 | 第十三轮 §2；格式说明 §3.3 |
| e3d-io 现有可复用件 | `RefNo`、`e3d_attlib::db1_dehash`（角色词反哈希）、`DbSet` / `DbElement`（读 PADD 的 SHEE 取 `EXFI` / `PVNO` / `THUMBN`）、`SchemaPack`（`padvir.dat` 模板；`schema/e3d31/binaries/` 已有 `padvir.dat` 985 KB 但 `manifest.json` 的 `templates` 里还没登 `PADD`）、`tests/common::missing` 夹具口径 | `src/lib.rs`、`schema/e3d31/manifest.json`、`tests/common/mod.rs` |
| 不在文件里 | 注释文字（Plant 图片 TEXT = 0 条，DRAW 打开时从 PADD 重画）、INSTANCE 引用的库段（在别的图片里）、线型实宽（`GTLSWI`）、字体、用户 `COLOUR` 命令改过的颜色 | 格式说明 §4 |

---

## 2. 范围

### 2.1 做

| # | 内容 | 对应 `gwdump.py` |
|---|---|---|
| A | **物理层**：`Record`（128 B）/ `Word`（u0, u1 各大端 u32）三种读法 `int() / real() / header()`；`PictureFile::open(path)` 读全文件、按头 [16..21] 定位名字块 / 属性表 / 图片表；不截断的尾巴与记录尾垃圾按格式说明忽略 | `Words`、`GWFile`、`enc_int / enc_real / enc_hdr` |
| B | **逻辑层**：`Header`（版本 15 字符）、`NameList`（`"<w0> <w1> <EXFI> 0 <PVNO>"` → 解析成 `PictureName { owner: RefNo, exfi, pvno }`）、`AttrTable`（原字保留）、`Picture { number, window: [f64; 4], int4, directory }`、目录回放成 `Segment` 树（`rel | levels << 4` 规则：先上溯 `levels` 层，6 = 末子、3 = 前兄弟） | `Picture._replay_directory`、`Node` |
| C | **段 / 组 / 图元**：`Segment { flags, transform: Option<[f64; 6]>, user: UserBlock, groups }`；`UserBlock` 解出角色（`Role::{Sheet, View, Design, Geom, Ppts, Plin, Texp, Other(hash)}` + refno 列表，TEXP 带 8 实数文本框 + 长度前缀串），原字一并保留；`Group { id, prims }`；`Primitive { pen, colour, kind, payload }`，`Payload::{Text(TextPrim), Text2(Text2Prim), Points(Vec<(f64,f64)>), Conic { start, spans: Vec<Span> }, Instance { reference: RefNo, transform }}`；TEXT `attr` → `TextFormat { fonts, dir, hjust, vjust }` + 可打印串；TEXT2 → `Vec<Run>` + 源串重建；TEXT 末字残留 `tail` 原样保留 | `Segment._parse / _decode_user / _decode_texp`、`decode_prim`、`text_attr_format`、`text2_source` |
| D | **编回（验收用）**：每个解出的项 `encode()` 回字，与原字节比对；`roundtrip(path) -> RoundtripReport { seg_hdr, grp_hdr, prim, hdr, pictab, dir, name: (ok, bad) }`；**不是**新图片写入器（不排版、不分配记录、不改 E3D 能读的东西） | `encode_prim`、`Segment.encode_header / encode_group_header`、`roundtrip` |
| E | **几何 / 样式语义**：段变换沿树复合（`x' = a·x + c·y + tx`），`visible()` = 祖先 flags bit0 全为 1；`Span::arc_points(n)` 按顺时针 θ = 4·atan(√w) 展开；`Pen::style(pen) -> (LineType, Width)` 与系统笔；`Colour::rgb(n) -> Option<(u8,u8,u8)>` / `name(n)` = 第十三轮的 365 项表（> 365 = 层号，返回 `None`） | `gwpic.xf_apply / xf_compose / conic_points`、`pen_name`、`draft_colours.py` |
| F | **从 PADD 到文件**：`picture::locate(project_pic_dir, sheet: &DbElement) -> Vec<PathBuf>`——读 SHEE 的 `EXFI` / `PVNO`，按 `NN = w1 mod 32` 拼路径，列出存在的副本；`thumbnail(sheet) -> Option<Vec<u8>>`（`THUMBN → ARCHIV → ATTDAT.ELDATA` → PNG 字节；ELDATA 布局 `[4][总字节][UUID×4][载荷]`，载荷按字小端，前缀 161 B 后是 PNG）。`manifest.json` `templates` 加 `"PADD": "padvir.dat"` | `paddprobe pics / eldata`、`gw\thumbs.py` |
| G | **CLI** `e3d-picture`：`info <file>`（头 / 名字 / 窗口 / 计数）、`list <file> [--page] [--csv]`（逐图元清单，图纸 mm）、`roundtrip <file|dir>`（= gwdump 的 corpus 门）、`locate --db <padd> --pic-dir <dir> --refno w0/w1`、`thumbnail --db … --refno … --out x.png` | `summary / full_list / roundtrip_dir / main` |
| H | **文档**：`docs/evidence/2026-09-24-gw-picture-format.md`（格式说明搬入，所有地址 / 常量指回 core.dll `idalib-4000` 与 Core3D `idalib-49136`）；README 加「DRAFT pictures」一节；CHANGELOG `Unreleased` |

### 2.2 不做（写清楚，免得后面当成漏了）

- 注释 / 背景 / 尺寸 / 标签**重画**（NOTE / TEXP / SLAB / GLAB / LDIM …）——DRAFT 语义层，留在 `D:\ida_scratch\draft31\anno\annodraw.py`；它将来若要吃 Rust 输出，走 `e3d-picture list --csv` 或 JSON。
- SVG / PNG 渲染、字体、字宽。
- INSTANCE 的跨图片解析（只返回 `(w0, w1)` 引用与变换；目标段在 Marine 背景库的图片里，且样本里 180 / 296 个引用指向已丢失的库 22505）。
- 让 E3D 读回我们**新写**的图片（要 TTY 门 + `PVNO` / `CMSVGP` 生命周期，另立计划）。
- 用户 `COLOUR` 命令改过的颜色、`FWSQGN` 设备色 297 / 301 / 304 的运行时值、线型实宽。
- Marine `.sdb`、视图生成链（Core3D `UVIEW / ADREM / RRRGET / HLR`）、孤儿图片分类。

---

## 3. 模块设计

```
src/picture/
  mod.rs        pub use；PictureError（thiserror：Io / ShortFile / BadHeader{rec,expected,found} / BadDirectory{entry,reason} / BadPrimitive{kind,pos} / UnknownKind(u8)）
  word.rs       Word { u0, u1 }：from_be(&[u8;8]) / to_be；as_int / as_real（f64 = u1<<32|u0 位重解）/ as_header -> (kind, payload, len, extra)
  file.rs       PictureFile { bytes, header: Header, name: PictureName, attrs: Vec<Word>, pictures: Vec<Picture> }；open / words_at(rec, n)
  directory.rs  目录条目 + 回放 → Vec<Segment>（树用 parent 索引 + children Vec<usize>，避免 Rc）
  segment.rs    Segment / SegFlags / UserBlock / Role / Group
  prim.rs       Primitive / Payload / TextPrim / TextFormat / Text2Prim / Run / Span；decode(ws, pos) / encode(&self) -> Vec<Word>
  text.rs       attr 字混合基数（180 / 11520 / 737280 / 47185920，dir / hj / vj 三张表）、TEXT2 转义 run ↔ 源串
  geom.rs       Transform（compose / apply）、visible 判定、Span::points(n)
  style.rs      Pen（8001–8045 线型 × 宽、8000、8100–8115）、Colour（365 项表 = draft_colours.py 的 BASE16 / DICT61 / DICT_INDEX_MAP / RGB256 / SPECIAL 数据 + build_table()）
  locate.rs     SHEE → 文件路径；ELDATA → PNG（依赖 db_element，放在这一层）
  roundtrip.rs  RoundtripReport；对一个文件 / 目录跑
src/bin/e3d-picture.rs
tests/picture_small.rs        42 KB 样本（AvevaPlantSample /APS-0001 里 SHEE 2013286677/4350 的 M2013286677-4350-10000-11）：头 'GW6A04    PC 06'、名字 '2013286677 4350 10000 0 11'、窗口 ±210 / ±297、31 段 / 136 组 / 655 图元（MARKER 368 / LINE 259 / CONIC 28）、图框 LINE (0,0)(210,0)(210,297)(0,297)(0,0) 笔 8107 颜色 1、roundtrip 全 ok
tests/picture_big.rs          /APS-0001/1 M2013286677-265-10000-11（14 MB）：4 715 段 / 351 819 图元、GEOM 段组 id = 成员序、DESI 段缩放 = VIEW.VSCA、VIEW 段平移 = VIEW.XYPS（对 PADD 库读的值）——`#[ignore]`（约 3 s，但要 PADD 库）
tests/picture_corpus.rs       Plant 881 + Marine 1 885 文件 roundtrip 0 失败 + 10 种 kind 计数 = 第八轮的数字——`#[ignore]`（Python 版 105 s + 4 min，Rust 预计 < 1 min）
tests/picture_style.rs        颜色表：365 项、1 = grey (130,130,130)、7 blue、59 sandybrown (188,141,0)、101 forestgreen (31,129,0)、160 steelblue (0,141,188)、271 lightgrey (189,189,189)、15 black、273–295 白、366 → None；笔 8001 / 8004 / 8000 / 8107 / 8100
```

- 夹具口径沿用 `tests/common::missing`：GW 文件与 PADD 库都是 AVEVA 样例工程数据，**不入库**，路径常量加到 `tests/common/mod.rs`（`PLANT_PIC_DIR = D:\AVEVA\Projects\E3D3.1\AvevaPlantSample\apspic`、`MARINE_PIC_DIR`、`PLANT_PADD_DIR = …\aps000`），缺则 skip 并打印。
- 不加新依赖：解析手写（`nom` 对 8 B 字流没有优势），PNG 只是字节透传，不解码。
- 大文件（Marine 最大 18 MB）一次读入内存；`words_at` 按 `(rec − 1) × 128 + i × 8` 直接切片，不做流式。
- 树用索引不用 `Rc<RefCell>`：`Segment { parent: Option<usize>, children: Vec<usize>, .. }`，`Picture.segments: Vec<Segment>`，回放时只需要「当前节点」和「上溯 n 层」。
- 公开面：`e3d_io::picture::{PictureFile, Picture, Segment, Group, Primitive, Payload, Role, TextFormat, Transform, Pen, Colour, locate, thumbnail, roundtrip}`；`PictureError` 独立于 `E3dReaderError`（不是 Dabacon 文件）。

---

## 4. 切片与验收

| 步 | 内容 | 验收（每步单独绿） | 估时 |
|---|---|---|---|
| S1 | A + B：物理层、头、名字、属性表、图片表、目录回放（先不解段内容，只切出每段的字范围） | `picture_small`：头 / 名字 / 窗口 / 31 段的 `(rec, nwords, rel, levels)` 与 `gwdump.py --list` 逐条相同；4 715 段的大文件目录回放零异常 | 0.5 d |
| S2 | C：段头 / 用户块 / 组 / 10 种图元解码；`text.rs` | `picture_small` 655 图元逐条与 `gwdump.py --list --raw` 相同（kind / pen / colour / 坐标）；大文件 351 819 条 kind 直方图 = 第八轮；Marine 三个 TEXT2 样本（`lists\marine_*.page.txt`）的 run 列表与 `fmt` 串相同 | 1.5 d |
| S3 | D：编回 + `roundtrip`；`e3d-picture roundtrip` | 小文件全 ok；**`picture_corpus`：2 766 文件 0 失败、kind 计数 = 第八轮的 10 个数字**（这是本计划的门） | 1 d |
| S4 | E：几何与样式 | 58 629 条弧的 `√w` 与中点几何零例外（写成 corpus 测试的一部分）；`picture_style` 表值 = `draft_colours.tsv`；`--page` 输出的图纸 mm 与 `lists\M2013286677-265-10000-11.page.csv` 抽样 1 000 行逐值相同（1e-9） | 0.5 d |
| S5 | F + G：`locate` / `thumbnail` / CLI 其余子命令；manifest 登 `PADD` | Plant 61 个 PADD 库 328 个属主 328 命中、Marine 718 / 720（第一轮的数字）；30 个 THUMBN 全解出 PNG 且字节 = `gw\thumbs\*.png` | 0.5 d |
| S6 | H：evidence 搬入、README、CHANGELOG；`cargo clippy --all-targets` 新模块 0 warning；全套 `cargo test --no-fail-fast` 不比 0.4.0 的 101 / 622 / 0 / 30 少；下游 `cargo check` | — | 0.5 d |

合计约 4.5 个工作日；S1–S3 是串的，S4 / S5 可与 S3 并行。每步一个提交（`feat(picture): …`），只 `git add src/picture tests/picture_* src/bin/e3d-picture.rs docs/… tests/common/mod.rs schema/e3d31/manifest.json src/lib.rs`。

**验收总则**：Rust 版与 `gwdump.py` v4 在同一输入上**逐项相同**（清单行、计数、roundtrip 结论）——Python 版本身已被 2 766 文件逐字节回写证过，所以「与它相同」就是「与 core.dll 写出的字节相同」。任何不同都先查 Python 版是不是错（第八轮就是这样纠出 TEXT2 多 run 的）。

---

## 5. 风险

| 风险 | 处置 |
|---|---|
| Python 版里有「原字兜底」的地方（用户块长度不合 TEXP 布局的 48 个段、TEXT 末字残留）——Rust 若按结构化解就会丢字节 | 与 Python 一样：`UserBlock` / `TextPrim` 都保留 `raw: Vec<Word>`，编回用 raw；结构化视图只是读法 |
| 记录尾垃圾、文件不截断（334 / 881 个文件末尾带上一版残留） | 只信头里的位置，不扫到 EOF；`open` 记 `trailing_bytes` 供 `info` 打印 |
| 双精度用 `f64` 往返：位重解 `(u1 << 32) | u0` 再 `from_bits`，不经十进制 | roundtrip 逐字节比 |
| 版本串 5A17 / 5A18 / 5A19 / 5A1A / 6A02 / 6A04 / 9B2B 全部同构（第八轮已证）——但别的工程可能有新版本 | 只校验 `" PC 06"` 后缀与 15 字符长度，版本前 9 位原样带出，不拒 |
| 与并发会话的工作树冲突 | 只加新文件；`src/lib.rs` 一行 `pub mod picture;`；CHANGELOG / README 在 S6 一次改，改前 `git status` 确认没人正改它 |

---

## 6. 要评审定的（推荐项在前）

- **Q1 模块位置与名字**：(a) **`e3d_io::picture`**（顶层；它是另一种文件，不是 Dabacon 页，和 `page` / `record` 平级）；(b) `e3d_io::draft::picture`（给以后 PADD 语义层留位置，但现在只有一个子模块）。推荐 (a)。
- **Q2 夹具**：(a) **样例工程路径常量 + `missing()` skip，corpus 测试 `#[ignore]`**（与 `TEST_FILE_DB` 同口径；GW 文件是 AVEVA 样例数据不入库）；(b) 把 42 KB 小文件提交进 `tests/fixtures/`（方便 CI，但那是许可数据，与 `binaries/` git-ignore 的口径矛盾）。推荐 (a)。
- **Q3 写侧边界**：(a) **只做编回验收**（每项 `encode()` 回原字），不提供「新建 / 修改图片文件」API；(b) 顺手做记录级写入器（分配记录、回填头）。推荐 (a)——(b) 没有 E3D 读回的门就是盲写，等有需求再立计划。
- **Q4 CLI**：(a) **新 bin `e3d-picture`**（`info / list / roundtrip / locate / thumbnail`，与 `e3d-write` 同风格）；(b) 挂到 `e3d-descriptor picture …` 子命令下。推荐 (a)——`e3d-descriptor` 是描述子 / 属性的 CLI，混进图片文件不顺。

Q1–Q4 全按推荐项定的话，S1 当天可以开工。

---

## 7. 记录

- 2026-09-24：草案（fable-5-1-14）。写在第十三轮进度盘点之后：PADD 三层存储与处理链已解完、GW 格式 2 766 文件逐字节回写零差异、颜色表已从二进制读出，但实现全在 `D:\ida_scratch\draft31\`，e3d-io 没有 PADD / GW 代码。
- 2026-09-28：评审通过（fable-5-1-2，Q1–Q4 全按推荐项）。格式说明已搬入 `docs/evidence/2026-09-28-gw-picture-format.md`；`manifest.json` 已登 `PADD`（§2.1 F 的那一行提前做了，`tests/padd_open.rs`）。S1 待开工。
- 2026-09-28/29：**S1–S3 落地**（fable-5-1-8；三个提交 `7bbc5cf` S1、`39bc8f8` S2、S3 = 本条所在的提交）。
  - S1 `src/picture/{mod,word,file,directory}.rs`：`Word{u0,u1}` 三种读法与构造互逆；`PictureFile::open` 只信头 [16..21]；`PictureName` 解成 RefNo / EXFI / PVNO；目录按 op 9 回放成索引树（`walk()` 前序 = gwdump 打印顺序）。`picture_small` 2 条：小样本 31 条 `(rec, rel, nwords)` 与树逐条 = gwdump；`picture_big`：4 715 段回放零异常。
  - S2 `segment.rs / prim.rs / text.rs / geom.rs / style.rs / listing.rs`：段头 / 变换 / 用户块（原字保留，GEOM / PPTS 成员表，TEXP 文本框）、组头与裸图元、10 种图元（TEXT 末字残留 `tail`、TEXT2 run 表与 Raw 兜底）、`TextFormat` 混合基数、TEXT2 源串重建、`Transform` 复合、笔名。**清单同形**：`listing.rs` 把 gwdump 的 `--list` / `--csv` / `--page` / 摘要一字不差地复现，小样本 655 行 raw + 655 行 page、大样本 351 819 行 page、Marine `M2013287597-16864-10000-1`（TEXT 57 / INSTANCE 7）67 行 + `M2013287597-1792-10000-1`（TEXT2 78 全部解成 run、7 个多 run 源串 `SHOP&/TEST` / `Block &&&/Unit No.` / `&USEC.(Right)`…）95 行——全部与 `draft31\lists\*.txt` 逐行相同。大样本 kind 直方图 MARKER 300 211 / LINE 48 915 / CONIC 2 465 / FILL 228 = 第八轮；GEOM 组 id 全在成员表。
  - S3 `roundtrip.rs` + `src/bin/e3d-picture.rs`（`info / list [--page] [--csv] [-o] / roundtrip <file|dir> [-v]`）+ `tests/picture_corpus.rs`：头 / 名字 / 图片表 / 目录 / 段头 / 组头 / 图元逐字节编回比对。**门：Plant 881 文件 seg 191 374 / grp 2 204 157 / prim 8 640 993、Marine 1 885 文件 1 365 582 / 2 581 787 / 16 232 654，全部 0 差异；10 种 kind 计数与第八轮 §0.2 逐个相同**；`e3d-picture roundtrip <dir>` 的报告文本与 `lists\corpus-roundtrip-*.txt` 用 `fc` 比无差异。耗时 Plant 8.4 s + Marine 10.9 s（Python 105 s + 4 min）。
  - 与 Python 的两处「更严」：名字块按「字符 + 空格补齐」编回（gwdump 只重编 u1）；TEXT 末字之后若还有整字、TEXT2 解不开时的 Raw 都原字编回（gwdump 会丢）。语料上两者都没触发差异。
  - 验证：`cargo test --release --lib picture::` 30/0；`picture_small` 6/0、`picture_big` 1/0 + ignored 1/0、`picture_marine` 2/0、`picture_corpus` 3/0 + ignored 3/0；`cargo clippy --release --all-targets` 新模块与新 bin 0 warning；rustfmt 过。共享 target 目录被别的会话锁住，本会话用 `CARGO_TARGET_DIR=D:\Rust\target-draft`。
  - 下一步 S4（P2）：`Span::points(n)` 弧展开 + 58 629 条弧 `√w` 几何零例外、365 项颜色表进 `style.rs`（`draft_colours.tsv` 生成）、`picture_style` 测试。
- 2026-09-29：**S4–S6 落地（P2）**（fable-5-1-8；提交 `6e3c07a` S4、`c4160d2` S5、S6 = 本条所在的提交）。
  - S4 `geom.rs` / `style.rs` / `colour_data.rs`：`Span::points(start, n)` 按第九轮画法（归一化框架里从 S′ 经 M′ 顺时针扫 θ = 4·atan(√w)），`Span::check` / `ArcStats` 复现 `verify_conic.py` 的两条判据。**门**：Plant 58 629 条弧 M 在垂直平分线上且 |M−mid|/半弦 = √w 零例外（= 第二轮）；Marine 第一次跑：447 819 条，w > 0 的全部一致、M 全在平分线上，但 **w = 0 的直段 317 939 条里 161 946 条 M 不在弦上**（偏 ~1% 半弦；`verify_conic.py` 也记 mismatched 161 946）——M 对直段无意义，两边都画直线，出图不受影响；DRAW 对 w = 0 究竟画不画 M 留作 P5 课题。颜色表：`BASE16 / DICT61 / DICT_INDEX_MAP / RGB256` 静态数据由 `draft_colours.py` 生成，`Colour::of(n)` 按 `Init` + `FWSCOL` 顺序重放，365 行与 `draft_colours.tsv` 逐行（名字 / 双精度 / 8 位）相同。`--page` 清单在 S2 已与 351 819 行 oracle 全同，超过 S4 要求的抽样 1 000 行。
  - S5 `locate.rs`：`PictureLocation`（`pic\NN\M<w0>-<w1>-<EXFI>-<PVNO>`，`copies()` 列别的 EXFI 副本）、`locate()`（PVNO ≤ 0 → None）、`Thumbnail::from_eldata`（总字节按整字补齐——aps250133 的 2013286677/1470 报 3 294 字节装在 824 字里）、`thumbnail()` 按成员表走 THUMBN → ARCHIV → ATTDAT、`PaddProject::open` 开整目录的 PADD 库。**门**：Plant 61 库 SHEE 301 + OVER 27 = 328 属主 328 命中，子目录规则零反例；Marine 20 库 SHEE 702 + OVER 18 = 720 / 718，缺的 2 张是 `/MARKS` 那个库里的 `/MARKS` 与一个无名 SHEE（= 第一轮）。**新发现**：Plant 的 LALB 24 里 2 个、SYLB 69 里 2 个带 PVNO 5 / EXFI 35（外抽取库的图，本工程无文件）——第一轮「库 PVNO ≤ 0」对 BACK / SHTMPL / ISOLB 成立、对 LALB / SYLB 有 4 个例外。缩略图：`/APS-0001/1` 898 字 → 3 564 字节载荷 → 3 402 字节 PNG = 第一轮 `thumb_APS-0001_1.png`。**Python 错了一处**：`thumbs.py` 靠 `paddprobe dump` 的文本树配对 SHEE 与 ATTDAT，而那棵树每层只印 4 个成员（"… +N more"），/APS-1000/1 的 THUMBN 被截掉后配到了后面别的元素的 ATTDAT（194 字节空白 PNG）；按成员表走的 `thumbnail()` 给 2 574 字节真图，aps250133 与 aps250501 两份库的同名 sheet 两两相同。
  - S6：CLI `e3d-picture locate / thumbnail`；README「DRAFT pictures」一节 + API 表；CHANGELOG 一条写全；**`thumbs.py` 修 bug（Q8 允许）**：按缩进配对（THUMBN / ARCHIV / ATTDAT 各深一级，回到 sheet 层即放弃）、遍历 `paddprobe sheets` 列出的全部 SHEE 而不只是树里可见的、树里看不到 THUMBN 的退到 `e3d-picture thumbnail`（tsv 第 8 列 `how` 标 `paddprobe` / `e3d-picture`）、同名 sheet 第二份写 `<name>.<db>.png` 不再覆盖。重跑：57 张（8 个库的全部带 THUMBN 的 SHEE；28 张 Python 独立取得、29 张退到 Rust），`gw\thumbs\APS-1000_1.png` 重写为 2 574 字节真图；`tests/picture_locate.rs` 对 57 张逐字节比、同名两库两两相同。
  - 全套：`cargo test --release --no-fail-fast` **108 个测试二进制，687 过 / 1 败 / 35 ignore**（0.4.0：101 / 622 / 0 / 30；新增 7 个二进制、+65 过、+5 ignore 是全量 corpus 门）。那 1 败是 `tests/hunk_resolve_real.rs`：它钉的是 `ams8000_0001` 里 E3D 自己写的 hunk 链起始页 16456，而这个库 2026-09-28 18:41 被别的会话（写门 / E3D SAVEWORK）重存过、链挪到了 16600——与 picture 无关，是夹具漂移，留给写侧会话处理。下游 `cargo check --release`：e3d-model 0 error（1 m 38 s），e3d-io-write 与 7 个 e3d-model 变体 0 error，gen-model 0 error（2 m 57 s）。clippy `--all-targets` 新模块 / 新 bin 0 warning。
  - 本计划（S1–S6）到此**全部落地**；剩下的都在 §2.2「不做」里。后续 DRAFT 语义层与渲染按上位计划 P3 / P4 在新 crate `vendor/e3d-draft` 做。
