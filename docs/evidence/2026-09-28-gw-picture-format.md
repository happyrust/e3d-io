# 2026-09-28 E3D DRAFT 图片文件（GW / Wigwam GTIGS picture）格式说明——从 `D:\ida_scratch\draft31\GW-PICTURE-FORMAT.md` 搬入

- 来源：读法 3 第二～十三轮（2026-09-23 / 24，会话 fable-5-1-12 / -14 / -17 等）对 core.dll Wigwam 写出路 `GSVPI0` → `sub_54DC674`（op 6 写 / op 9 读）的逆向 + 两个样例工程全部图片文件的核对。IDA 实例 `idalib-4000`（core.dll `D:\ida_scratch\plant3\core.dll.i64`）、`idalib-49136`（Core3D.dll）。
- 判据：`gwdump.py --roundtrip`——Plant 881 + Marine 1 885 = 2 766 个文件、24 873 647 个图元解码再编码**逐字节 0 差异**；58 629 条弧的中点几何零例外；颜色表由 DRAW 自存缩略图逐像素坐实（第十三轮）。
- 参考实现（冻结为 oracle，DRAFT 计划 Q8）：`D:\ida_scratch\draft31\gwdump.py`（解析 / 回写 / 清单）、`gw\gwpic.py`（几何）、`gw\gwrender.py`（渲染）、`gw\draft_colours.py`（颜色表）。Rust 移植见 `docs/plans/2026-09-24-gw-picture-read-plan.md`（`e3d_io::picture`）与 `docs/plans/2026-09-28-draft-module-next-steps-plan.md` P1 / P2。
- 本文正文与 `draft31` 里的原件 2026-09-28 逐字相同（原件保留在原处，之后以本文为准）。

---

# E3D DRAFT 图片文件（GW / Wigwam GTIGS picture）格式说明

对象：`<proj>pic\NN\M<w0>-<w1>-<EXFI>-<PVNO>`（Plant `apspic\`、Marine `amspic\`），由 core.dll Wigwam 内核 `GSVPI0` → 图片调度器 `sub_54DC674` op 6 写出、op 9 读回。
依据：core.dll（`D:\ida_scratch\plant3\core.dll.i64`）伪码 + AvevaPlantSample 881 个、AvevaMarineSample 1 885 个文件全量核对（版本 5A17 / 5A18 / 5A19 / 5A1A / 6A02 / 6A04 / 9B2B 全部同构）。读取实现：`gw\gwpic.py`，渲染：`gw\gwrender.py`。

## 1. 物理层：记录与字

- 文件 = 连续 **128 字节记录**（`HWDBIR` 记录长 32 个 4 字节单元），所有「位置」都是 **1 起的记录号**：字节偏移 = (rec − 1) × 128。
- 内容单位是 H-list 的 **8 字节字** = 两个 4 字节单元 u0、u1，写盘时**每个 4 字节单元各自大端**，单元顺序不变（`FHDBWN`）。
  - 整数字（`HPUTI1`）：u0 = 0，u1 = 值（有符号 32 位）。
  - 实数字（`HPUTRA`）：8 字节就是一个 IEEE double，u0 = 低 4 字节、u1 = 高 4 字节（即 `struct.unpack('<d', pack('<II', u0, u1))`）。
  - 头字（元素头 / 组头 / 图元头）：u0 = 种类与标志，u1 = 长度（低 16 位）| 附加值 << 16。
- 一个块占若干整记录，最后一条记录的尾巴是 **未清零的堆垃圾**（`malloc` 后只 `memcpy` 有效部分），读时忽略。
- 文件**不截断**：重存后如果内容变短，旧内容留在尾部（334/881 个 Plant 文件末尾带上一版残留），只信头里的位置。

## 2. 逻辑层：五种块

| 块 | 位置 | 内容 |
|---|---|---|
| 头 | 记录 1（21 字） | 字 1–15：版本串 15 个字符，一字一符：`G W 6 A 0 4 ␠ ␠ ␠ ␠ P C ␠ 0 6`（前 9 位 = `GVRSN` 版本，后 6 位常量 `" PC 06"`）；字 16 = 名字块记录号、17 = 名字块字数；18 / 19 = 属性表记录号 / 字数；20 / 21 = 图片表记录号 / 字数 |
| 名字块 | 头[16]，头[17] 字 | 一字一符的文件标识串 `"<w0> <w1> <EXFI> 0 <PVNO>"`（32 字符） |
| 属性表 | 头[18]，头[19] 字 | Wigwam 全局表 `dword_72A0AC0` 每条 13 字记录抽出第 i+1, i+3, i+6, i+7 四字；实测要么 0 字（334 个文件），要么恒等的 132 字（33 条，id ±1 / ±11..±16 / 21..26 / 31..36 / 41..46 / 63），与图形内容无关 |
| 图片表 | 头[20]，头[21] 字 | 每图 8 字：`[8, 图片号, 目录记录号, int, real×4]`；real×4 = 图纸窗口 `(-W, -H, W, H)`（A0 = ±1189, ±841；Marine 36″×24″ = ±914.4, ±609.6 mm）；int Plant 为 0、Marine 为 1（含义未定）。实测每文件恰 1 张图 |
| 目录 | 图片表[3] | 字 1 = 目录自身字数；从字 2 起每 7 字一条：`[7, ref_w0, ref_w1, 数据记录号, rel | levels<<4, 元素类型, 字数]` |
| 段数据 | 目录条目[4] | 元素（段）的全部字（§3） |

### 2.1 目录 → 段树

目录顺序是存盘时的深度优先**倒序遍历**（先最后一个子，再前一个兄弟，再上溯），回放规则（core.dll op 9）：

```
cur = 图片根
for each entry:
    for _ in range(rel >> 4): cur = cur.parent      # 先上溯 levels 层
    if rel & 0xF == 6: 新节点作为 cur 的最后一个子
    if rel & 0xF == 3: 新节点作为 cur 的前一个兄弟（同父）
    cur = 新节点
```

元素类型实测只有 3（段）。`ref_w0/w1` 只在段头标志 0x20（「有 DB 引用」）置位时非零（Marine 极少数段），Plant 全为 0——段与 DB 元素的对应关系在段的用户块里（§3.1）。

## 3. 段（元素类型 3）

```
字 1        头：u0 = 0x70000000 | flags，u1 = hdrlen（低 16 位，= 头部占用字数）
字 2..7     flags & 0x08：6 个 real 变换 (a, b, c, d, tx, ty)，x' = a·x + c·y + tx，y' = b·x + d·y + ty，沿树向下复合
(+4 字)     flags & 0x10：4 字（实测未出现）
(+1+n 字)   flags & 0x04：用户块 [n+1][n 字]
字 hdrlen+1 起：组 × N，直到元素末尾
```

flags 位（`GQUSGA`）：bit0 可见、bit1 可拾取、bit3 有变换、bit2 有用户块、bit5 有 DB 引用（目录里 ref 非零）、0x80 / 0x100 / 0x200 其它段属性。**渲染只画 bit0 = 1 的段**（Plant 里 0x6 的段全是拾取点 MARKER，DRAW 里不可见）。

### 3.1 用户块 = 段与 DB 的对应

`[角色词哈希][refno w0][refno w1][...]`，角色词是 Dabacon base-27 名词哈希（`e3d-attlib::db1_dehash`）：

| 角色 | 后续字 | 含义 |
|---|---|---|
| `SHEE` / `BACK` / `SAID` | w0, w1 (, 0) | 图纸 / 背景 / 图纸注释容器；refno = SHEE |
| `VIEW`（带平移变换 = XYPS） / `VAID` | w0, w1 | 视图 / 视图注释容器；refno = VIEW |
| `DESI`（带缩放变换 = VSCA） | 0, 0 | 设计内容容器，子段坐标是 3D 模型 mm |
| `FRMW` / `BRAN` / `EQUI` / `HPLATE` … 设计名词 | w0, w1 | 一个设计元素的段 |
| `GEOM` | 0, 0, (名词, w0, w1) × n | 几何段；**组 id k = 第 k 个元素**（2 350 段零例外） |
| `PPTS` | 1, 0, (w0, w1, 名词, 0) × n | p-point 拾取点段（不可见 MARKER），组 id 同上 |
| `PLIN` | 0, 0, 1, `MEML`, w0, w1 | p-line 段 |
| `BNTE` / `TEXT` / `VNOT` / `STRA` / `LAYE` / `GRPH` … | w0, w1 | Marine 背景注释 / 2D 注释段 |
| `TEXP` | w0, w1, **real×8**（文本框 4 角点 (x,y)），`[1][k][k][k][k][0][nw]`，nw 字长度前缀串（首字节 = 长度，4 字符/字） | Marine 2D 文字注释段；用户块里有 real 字，故用户块须按原字回写（不能当纯 int） |

### 3.2 组与图元

组头：u0 = 组 id | 15 << 27，u1 = 组字数（含头）。组内是连续图元，图元头：**u0 = 笔号 | kind << 27，u1 = 字数（含头）| 颜色号 << 16**。

| kind | 名 | 数据（头后 D = 字数 − 1 个字） | 来源 |
|---|---|---|---|
| 1 | TEXT | `real×4` 盒 (x0, y0, x1, y1)，`real×6` (x, y, upx·H, upy·H, CSP, LSP)，`[attr][n][⌈n/4⌉ 字：每字 u1 装 4 个字符，大端]`；末字第 n 字符之后的字节是写入方残留（GW5A1x 为 0，GW6A0x 为上一个 4 字符块的旧内容），读时忽略、回写时原样保留 | `GOPTXS` 路径 +1 |
| 10 | TEXT2 | 同上 10 个 real，然后 `[attr][srclen]` + 若干**段（run）**直到图元末尾：`[n][font][style][newline][z][⌈n/2⌉ 字：u0、u1 各一字符码，n 为奇数时末 u1 = 0]`。写侧 `sub_54F1B00`（`GOPTXS` → `sub_54D3968` 模式 −1）按转义符 E（Wigwam 文字属性 26，样本全为 `&`）把源串切段：`style` 位 B=1 / I=2 / U=4 / X=8（`EB Eb` / `EI Ei` / `EU E:`·`Eu E;` / `EX Ex` 开关，`ER` 全关），`newline` = 本段之前的 `E/` 个数，`z` = 上下标栈（`E>` 上标 4z+1、`E<` 下标 4z+2、`E=` 弹出、`E*` 清零），`font` = 当前字体号（`EO` = f1、`EN` = f2、`E<d>c` = 单字符 c 用 f(d+1)），`EE` = 字面 E；`srclen` = 源串长度（含转义）+ 4·`E/` 个数 + 6·`ER` 个数（读侧 `sub_54F39D9` 不用它）。读侧从 run 重建源串：`'SHOP&/TEST'`、`'Block &&&/Unit No.'`、`'&B&IP'`、`'&USEC.(Right)'`（`gwdump.py text2_source()`）。实测内容多为 `?drawno?` 一类待替换关键字 | `GOPTXS` 路径 −1 |
| 2 | MARKER | `real×D`，D/2 个点 | `GOPPMK` |
| 3 | LINE | `real×D`，D/2 个点的折线 | `GOPPLN` |
| 4 | CONIC | `x0 y0` + (D−2)/5 段 `[mx, my, w, ex, ey]` | `GOPCCL / GOPELL / GOPPCC / GOPPCN` |
| 5 | FILL | 同 3，闭合填充 | `GOPF..` |
| 6 | INSTANCE | `[w0][w1]` + `real×6` 变换：引用别的图片里 DB 引用 = (w0, w1) 的段（Marine 背景库） | op 2/10 |
| 7 | FILLCONIC | 同 4，闭合填充 | `GOPFCN` |
| 8 | BOUNDARY | 同 3（边界折线） | `GOPBLN` |
| 9 | BOUNDCONIC | 同 4（边界圆锥链） | `GOPBCN` |
| 14 | 段头 | — | — |
| 15 | 组头 | — | — |

CONIC 段 `[mx, my, w, ex, ey]`：从当前点 S 到 E 的圆弧，**(mx, my) 是弧中点**（在弦的垂直平分线上），`w = bulge²`，bulge = tan(θ/4) = |M − 弦中点| / 半弦。58 629 条弧全数核对：M 在垂直平分线上且 |M−mid|/半弦 = √w，零例外（w = 1 半圆、0.1716 = 90° 弯头、5.83 = 270°）。椭圆（`GOPELL`）也是 w = 1、M 取共轭半径端点，按仿射框架同样公式画。全圆 = 两段 w = 1 的半圆。

**画法（第九轮修正）**：在归一化框架 S′=(−1,0)、E′=(1,0)、M′=(0,√w) 里，圆心 (0, m)，m = (w − 1)/(2√w)，弧**始终从 S′ 经 M′ 顺时针扫到 E′**，扫过的角 θ = 4·atan(√w)（w=1 → 180°，0.1716 → 90°，5.83 → 270°），再映回 c + x′·u + (y′/√w)·v（u = 半弦向量，v = 弦中点→M）。旧 `gw\gwpic.py` 的 unwrap 写法按逆时针扫，非半圆的弧都被画成 270°–630° 的大圈（平面图里的"大圆"、北向箭头实心盘就是这个）；`gw\gwpic.conic_points` 已改，`anno\annodraw.bulge_points`（VRTX.BULG，bulge 归到"到达该顶点"的那段弧，正 = 弧凸向弦方向右侧）同样走这条路。

文字 6 个 real：(x, y) 锚点，(upx, upy) 字高向量（长度 = 字高 mm，方向 = 字的上方向），后两个 = CSP / LSP（字距、行距，实测为 0）；`GQUTXS` 读法：HGETRA(1,2) → 位置，HGETRA(3,2) → 归一化成 H + 单位上向量，HGETRA(5,2) → CSP、LSP。盒 4 个 real 以字高为单位、相对锚点：`x0 = −x1` 居中，`x0 = 0` 左对齐，`x1 = 0` 右对齐；`y1 = 0` 顶对齐。

`attr` 字（`GQUTXS` 0x54C09F8 混合基数解码，表在 core.dll .rdata 0xF86AE0 `"TEXT  MARKERLINEARFILL  "` 之后）：`attr = (f4−32)·47185920 + f3·737280 + f2·11520 + f1·180 + dir·36 + hj·6 + vj`（负数先加 0x5A000000），dir 1..4 = RIGHT / LEFT / DOWN / UP，hj 1..5 = LBODY / RBODY / CENTRE / LKERN / RKERN，vj 1..5 = BASE / TBODY / BBODY / HHEIGHT / HBODY，f1..f4 = FONT 号（非零者依次写出）；拼成 `WWTextFormat::fromString` 的串，如 `-1509947401 → "FONT    11,RIGHT,CENTRE,HBODY"`（盒 x0 = −x1 且 y0 = −y1，与 CENTRE / HBODY 吻合）。`gwdump.py text_attr_format()`。

### 3.3 笔号与颜色

笔号 = Wigwam 图元表示号，DRAFT 标准编号在 Core3D `linestyle/DFSLPN`（`sub_10099DAB`）、`deviceadc/PRNSPE`（`sub_1010E48B` / `sub_1010E7C2`）里定义：

- 8001–8030 = `8000 + 线型 + 10 × (宽度 − 1)`，线型 1..10 = SOLID / DASH / DOT / CHAIN / LDASH / DDASH / DDOT / FDOT / DCHAIN / TCHAIN，宽度 1..3 = THIN / MEDIUM / THICK（实宽 RWIDTH 来自 `GTLSWI`）。故 8001 = 细实线、8004 = 细点划线（管道中心线）、8005 = 细长虚线。
- 8031–8045 = `8030 + k + 5 × (宽度 − 1)`，k = 1 SOLID，k = 2..5 用户线型 LPA 8032..8035。
- 8000 = 填充笔；8100–8107 系统笔（8101 / 8105 THICK SOLID，8102 / 8103 / 8104 / 8106 / 8107 THIN SOLID，8107 = 图框）；8100 / 8115 = 系统标记笔（拾取点）。
- 颜色号 = DRAFT 颜色表索引（1、101、160、59、109…），颜色表本身不在文件里。**表在二进制里（第十三轮，`gw\draft_colours.py`）**：`udNet.dll`（shadow 安装里就有，第十轮说「缺失」有误）的 `CTINIT / CTCNAM / CTCRGB / CTQRGB / CTQNAM / CT_get_actual_rgb` 是原生 thunk，转进 `DruidNet.dll` 的托管类 `Aveva.Core.Presentation.Druid.ColourTableManager`（61 个命名色的 RGB 字典写在它的静态构造函数里，`il\DruidNet\…\ColourTableManager.cs`）；core.dll `PrepareEnvironment`（0x5352920）`CTINIT(maxcols = 365, extcols = 10000)` 后调 `fmdevice/FWSCOL`（0x533B140，`dec\FWSCOL.c`）按静态数据 `aGrey`（0x5FFBD40）填表：1–16 = grey red orange yellow green cyan blue violet brown white pink mauve turquoise indigo black magenta；17–272 = 256 个 RGB 双精度三元组（`aGrey+7392`）；其中 `dword_5FFF220` 指定的下标再套上字典名（59 = sandybrown (0.741,0.553,0)、101 = forestgreen (0.122,0.506,0)、109 = green (0,0.741,0)、160 = steelblue (0,0.553,0.741)、271 = lightgrey (0.745)…）；305–365 = 61 个字典名；296–304 为系统色（303 gold、302/298 lightyellow、300 midnight、299 white、296 (0.341,0.506,0.682)，297/301/304 由 `fmdevice/FWSQGN` 在运行时按设备设定）；273–295 未设（白）。8 位值按 `(int)(r × 255)` 截断。Core3D `WWPrimitive::getColour`：颜色号 > 365 表示「层号 + 365」而非颜色，返回 −4 = 用层颜色。**校验**（`gw\colourverify.py`，30 对缩略图 / 图片）：DRAW 自存缩略图里图框（笔 8107、颜色 1）像素恒为 (130,130,130) = int(0.51 × 255)；9 个可见颜色号（1 / 7 / 12 / 59 / 101 / 109 / 152 / 160 / 271）主簇色相与表值夹角 0.0–1.5°。第十轮按色相估的值全部与表一致，只有两处解释要改：颜色 1 不是「前景色」而是 grey (0.51)；颜色 15 是 black（黑屏上不可见，第十轮记的「蓝」是相邻网格线）。学科对应不变：160 结构（FRMW/HANDRA/SBFR/CWALL…）、101 管道 BRAN、109 电缆 CWBRAN、59 设备 EQUI/SUBE、7 参考网格 REFGRD、15 体积 VOLM、271 楼板 CFLOOR 填充。

## 4. 不在文件里的东西

- 注释文字（NOTE / TEXP / SLAB / 尺寸）在 Plant 图片里**一条都没有**（8.6 M 图元里 TEXT = 0）：DRAW 打开时从 PADD 库重画（`update anno`）。Marine 图片里有 TEXT（背景标题栏、关键字占位）。
- INSTANCE 引用的库段（背景 / 符号）在别的图片里，按 (w0, w1) 经各图片的引用表解析，离线画要另外从库 DB 生成。
- 线型实宽、字体。颜色表也不在文件里，但已从 core.dll / DruidNet 读出（§3.3、`gw\draft_colours.py`）。
