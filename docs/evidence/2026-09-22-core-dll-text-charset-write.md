# 2026-09-22 core.dll 写文本时的字符集转换 —— ida-bridge 取证（S5）

- 问题：`docs/plans/2026-09-21-write-path-next-steps.md` S5——写侧缺非 ASCII 文本的编码器。读侧 `record::text` 从语料归纳出「`FORMAT` 1 存 UTF-8 原字节；其它 `FORMAT` 存项目字符集（GBK）并把高位字节包在 `&~ … &` 里、高位清零」，写侧要照 core 的写法落地，先把 core 的转换函数读实。
- 手段：ida-bridge 活桥，`core.dll` = `D:\ida_scratch\plant3\core.dll.i64`（E3D 3.1）。本轮实例 **`idalib-4000`**——原 `idalib-33060` 被本轮一条写错的 xref 脚本（`get_next_dref_to` 在 64 位 IDA 里返回 `BADADDR` 不是 `0xffffffff`，循环不终止）卡死，`supervisor stop` 后用同一 `.i64` 重开；`.i64` 未改。
- 结论一句话：**写侧转换 = `sub_54B2CE8`（源字符集 12 = UTF-8 → 目标 = 项目 `MBCHARSET`，中文站点 = 6）**；6 的表就是 **GBK**（21 792 个码点，与 `encoding_rs::GBK` 全等）；转义只收**两字节都在 `0xA0..=0xFE`** 的码，其余拒绝（错 638）。已照此落地为 `record::text::encode_as`。

---

## 1. 入口：`db_cxx_put_str_vector`（`sub_5AD5060`）里的文本路径

每个串（`std::vector<std::string>` 的一项，24 字节步长；容量 ≥ 16 时数据在堆上）：

| 头 `FORMAT`（`sub_5AF6600(hdr, 18, &v39)`，头整数 18） | 做法 |
|---|---|
| `== 1` | `sub_54719F0`（`CHPACK`）：`sub_54B00A7` 只查**串尾**是否切在一个 UTF-8 多字节序列中间（`sub_54AFE98` 判首/续字节），是则截掉尾巴并打 ` CHPACK valid: {…} Invalid: {…}` 日志；然后 `sub_5B9B920` 按 4 字节/字大端打包。**不转码**——UTF-8 原字节落盘 |
| 其它 | `sub_5B8F3C0` 把字节摊成一字节一 int（上限 `0x4000`）→ `sub_54B2CE8(mode=12, in, n, MBCHARSET=dword_6453EDC, cap=0x4000, subst=-1, out, &outN, &status, &pos)` → `sub_5471E50` 把 int 数组按 4 字节/字大端打包（尾字高位对齐、NUL 补） |

- 项头里的 `bytes` 字（类型 19/23 每项、类型 10/15 的计数字）= **转换后**的字节数 `dword_6423B64`，含转义字节。
- 状态非零的处理：`status == 3` → `dword_6453B98 = 23`（缓冲不够）；其它非零 → **638**（转换失败）；随后不写。
- 会话页 `USER`/`DESC` 走同一转换（算法清单 §2 表 40/41：`sub_5471FC0`（字→一字节一 int）→ `sub_54B2CE8` → `sub_5471E50`）。

## 2. 转换器 `sub_54B2CE8`

签名（`__usercall`，全部指针）：`(a2: 源字符集码, a3: 输入 int 数组, a4: 输入个数, a5: 目标字符集码, a6: 输出容量, a7: 替代字符（-1 = 没有，出错即返）, a8: 输出 int 数组, a9: 输出个数, a10: 状态, a11: 出错位置)`。1 起下标。

### 2.1 字符集码（从 switch 分支与查表函数读出）

| 码 | 编码 | 编码函数（Unicode→码） | 解码函数（码→Unicode） | 备注 |
|---|---|---|---|---|
| 1 | Latin-1 | 直接 ≤ 255 | 直接 | `dword_6453EDC` 的默认值（`sub_5AE4A90`） |
| 2 / 3 | 单字节表 | `sub_54B95A1` / `sub_54B9674` | `sub_54B8C39` / `sub_54B8CAB` | |
| 4 / 24 | 双字节，双字节部分再经 `sub_54B5925`（写）/ `sub_54B5A01`（读）变换 | `sub_54B8E74` | `sub_54B87C8` | 形态是 SJIS→JIS 七位（日文） |
| 5 / 25 | 双字节 EUC 型 | `sub_54B91F0` | `sub_54B89E6` | 读侧固定按两字节取、高位补回 |
| **6 / 26** | **GBK**（见 §3） | **`sub_54B9032`** | **`sub_54B88D7`** | 读侧运行外单字节走 `sub_54B8E01`（CP1252）兜底——与 `record::text` 模块文档里描述的 core 行为完全一致 → **中文站点 `CI-CHI` = 6** |
| 7 / 27 | 双字节，运行内**不清高位** | `sub_54B93AE` | `sub_54B8AF5` | 形态是 Big5 |
| 8 / 9 / 10 / 11 | 4 / 6 / 5 / 7 的表，**不转义**的原始双字节 | 同上 | 同上 | |
| 12 | UTF-8 | 内联（1–4 字节） | `sub_54B7297` | 解码只认 ≤ 4 字节；5/6 字节头 → 状态 1 |
| 13 / −13 | UTF-16 LE / BE | 内联 | 内联 | 代理对合成 |
| 14 / −14 | UTF-32 LE / BE | 内联 | 内联 | |
| 15 | 码点原样 | | | |
| 20 / 21 / 22 | 单字节表 | `sub_54B9832` / `sub_54B99F0` / `sub_54B9BAE` | `sub_54B8D1D` / `sub_54B8D8F` / `sub_54B8E01` | 22 = CP1252（`dword_608A800`），即 4/6 读侧的兜底 |
| 23 | `\U+XXXX` 转义 | 内联 | | `\` 写成 `\\` |

4–7 是带 `&~ … &` 转义的（`v19`/`v24` 标志）；24–27 是同一张表的「纯双字节」形态：不开不关运行、ASCII 只有空格能写（写成 `!!`，27 写 `A1 40`），别的 ASCII → 状态 2。`MBCHARSET` 由 `sub_5AE76F0` 设（唯一写点，被 DB API 入口 `sub_5AD9DB0` 调），数字由上层传入——`CI-CHI` 这个名字到 6 的映射不在 core.dll 里，6 = 中文由上表的行为 + §3 的表内容 + 语料反推。

### 2.2 源 = 12（UTF-8）→ 目标 = 6 的逐字符规则（写侧要复现的全部）

1. 取一个码点（`sub_54B7297`）。解码出错（续字节不对、5/6 字节头）→ 状态 1；`U+D800..DFFF`、`U+FFFE`、`U+FFFF`、≥ `0x110000` → 状态 1（`LABEL_176` 的有效性检查）。
2. **ASCII（≤ 0x7F）**：若运行开着（`v14`）先写 `0x20 0x26`（` &`）关掉；写该字节。控制字符不拦。
3. **非 ASCII**：`sub_54B9032(cp)` 查 GBK：
   - 查不到且 `0x80 ≤ cp ≤ 0xFF` → `v74 = cp`，当**单字节原样写**（关运行后写一个高位字节；读侧 CP1252 兜底能读回）。
   - 查不到其它 → 状态 2。
   - 单字节结果（表里只有 `€` → `0x80`）→ 关运行后原样写。
   - 双字节 `[lead][trail]`：**要求 `lead ≥ 0xA0 && trail ≥ 0xA0 && lead ≠ 0xFF && trail ≠ 0xFF`**，否则状态 2（`v183`/`v184`/`v185`/`v186` 分支，什么都不写）。通过则：运行没开先写 `&~` 并置 `v14`；写 `lead − 0x80`、`trail − 0x80`。
4. 输入耗尽：运行开着 → 补 ` &`（容量不够 → 状态 3）。
5. 状态 2 时若 `a7 ≠ -1` 且在 `0x20..=0x7F` 就用替代字符（走 ASCII 路径，先关运行）；`sub_5AD5060` 传的是 −1 → 直接返回，整串不写。

状态码：0 成功；1 输入非法；2 不可映射；3 输出溢出；4 输入在多字节单元中间截断；5 未知字符集；6 UTF-8 里出现代理对（软标记，但调用方一律按非零 = 638 处理）。

### 2.3 读侧（源 = 6 → 目标 = 12）对照

`case 6`：运行内（`v13`）取两字节 `trail + ((lead + 128) << 8) + 128` 查 `sub_54B88D7`；运行外单字节先查 `sub_54B88D7`（GBK 单字节只有 `0x80`），查不到且 `128..=255` → `sub_54B8E01`（CP1252）。`sub_54B88D7` 的表按**尾字节 `0x40..=0xFE`（191 列）**索引——是 GBK/CP936 的形状，不是 GB2312 的 94 列。`record::text::escaped` 对运行外的高位串先按 GBK 再按 CP1252 读（`ams1112_0001` 那 78 条无转义的 GBK），比 core 多认一种形态，已在模块文档里记明。

## 3. 表 `sub_54B9032` 到底是什么

两级索引：`dword_60BA400[cp/64 + 192]`（`0x80..0x7FF`）/ `dword_60BA400[cp/4096 + 224]`（`0x800..0xFFFF`）给页号（负数），`dword_60A2200[…]` 取叶；`≥ 0x10000` 一律 `-99999`（**只有 BMP**）。用 ida-bridge 在 IDA 内跑一遍全 BMP（`%TEMP%\dump_cs6.py`，回执 `cs6_table.json`）：**21 792 个码点有映射**。

| 对照 | 结果 |
|---|---|
| Python `gbk` / `cp936` codec | 21 791 同、0 异；core 多 1 个：`U+20AC €` → **`0x80`** 单字节 |
| Python `gb2312` | 7 443 同、1 异、core 多 14 348 → 不是 GB2312 |
| Python `gb18030`（两字节部分） | 21 791 同、1 异（€）、codec 多 2 148（GB18030 增补 + 自定义区）→ 也不是 GB18030 |
| **`encoding_rs::GBK`（`%TEMP%\e3dio_census\src\bin\gbk_vs_core.rs`）** | **21 792 全同、0 异、0 core 独有**；encoding_rs 多 2 165 个码点：1 377 个能过 §2.2 的字节规则（**1 364 个 PUA `U+E000..=U+E864`** + **13 个非 PUA**：竖排标点 `U+FE10..=U+FE19` → `A6D9..A6DF`/`A6EC`/`A6ED`/`A6F3`、`ḿ U+1E3F` → `A8BC`、`ǹ U+01F9` → `A8BF`、`龻 U+9FBB` → `FEA0`），788 个过不了规则本来就拒 |

所以写侧 = `encoding_rs::GBK` + 拒 PUA 区间 + 拒那 13 个码（`record::text::NOT_IN_CORE_GBK`）+ §2.2 字节规则，输出与 core 逐字节一致。

**能写进 `&~ … &` 的双字节码 = 7 657 个**（7 443 个 GB2312 + 214 个两字节都 ≥ `0xA1` 的 GBK 扩展如 `·`→`A1A4`、`ŉ`→`A8BD`）；GBK 里另外 **14 134** 个码（尾字节 `0x40..0x7E`/`0x80..0x9F` 或首字节 `0x81..0x9F`，如 `–` U+2013 → `A843`、`—` U+2014 → `A1AA`✓ 能写而 `‐` U+2010 → `A95C` 不能）core **写不出、报 638**——中文项目里常见的英文破折号 `–` 就在其中。

## 4. 落地与验证（2026-09-22）

- `record::text::encode_as(text, TextFormat)` → `Result<Vec<u8>, TextEncodeError>`（`Nul` / `Unmappable` / `FormatUnknown`）+ `pack_words`；`TextFormat::Detect` 只放 ASCII。单测：`ams1112_0001` 的 8 条转义串逐字节写回并解回、运行嵌在明文中间（`-3.320m~±0.00m`）、`範板`（尾字节 `0xA0` 存成空格）、`/A-反应-板` 开两次运行、`–`/`😀`/`U+FFFE`/`U+E000`/`U+FE10`/`ḿ`/`ǹ`/`龻` 拒、`ÿ` 原样 `0xFF`、`€` → `0x80`、`café` → `caf&~(& &`、NUL 拒。
- 接入：`RecordDraft::set_explicit_text` / `add_explicit_text` / `set_explicit_text_list` / `set_text_hunk` / `set_text_list_hunk`（带 `TextFormat`；`_ascii` 版保留为只收 ASCII 的包装）、`write::attr::text_words`（`set_attribute` 的 10/15）、会话页 `USER`/`DESC`（按**存储字节**对槽校验，控制字符仍拒）、`index::named::encode_name_as` + `NamedKey::text_as`、`NamedTable::lookup_name_as`、`ReadOnlyEngine::find_by_name` 按头 `FORMAT` 编键。
- **往返**：`tests/write_stage4_set_attribute.rs` 里 `ams5100_0001`（`FORMAT` 0）的 **57 条中文文本**不再跳过，写回后 `content_eq` 逐字相等（2 993 行 0 差异）。
- **E3D 自己写的转义键**：`TES023`（`D:\AVEVA\Projects\E3D3.1\TEST\TES000`，DESI，`FORMAT` 0）的 SITE `/锅炉岛设备` 在 NAME 表里是 `/&~9xB/5:Ih18 &`；`encode_name_as("/锅炉岛设备", Escaped)` 给出同一串，`find_by_name("/锅炉岛设备")` 命中。`tests/write_stage5_text.rs` 在其副本上把一个 ZONE 改名 `/反应堆厂房-E3DIO`、`DESC` 设 `反应堆厂房 -3.320m~±0.00m`、会话注释中文：盘上 `DESC` = `[34][&~74S&6Q3'7? & -3.320m~&~!@ &0.00m]`，键 = `/&~74S&6Q3'7? &-E3DIO`，读侧三处都解回原文；`a–b` 被 `set_attribute` 与 `write_element_session`（注释）拒、文件字节不变。`ams8000_0001`（`FORMAT` 1）同一套走 UTF-8 原字节（含 `–`）。
- **TTY 门**：`e3d-write-box --text` 加第五个会话（BOX 中文 NAME + NAME 表键、ZONE 中文 DESC、中文注释），`tools/e3d/write_gate.ps1` 加 Q8a/Q8b、Q7 改看中文注释；**16 问全 PASS**，E3D 3.1 打出 `Name /E3DIO-盒子`、`Description 反应堆厂房 -3.320m~±0.00m`、`Description: e3d-io 写路径门：中文 NAME / DESC / 会话注释`（ALPHA LOG 是 UTF-8，脚本按「先 UTF-8 严格解、失败再 ANSI」读）。AMS 项目是 `FORMAT` 1；`FORMAT` 0 没上 TTY 门（TES000 项目的 MDB/用户未接入驱动），其口径靠上一条 E3D 自写的键坐实。

## 5. 复现命令（本机）

```powershell
# %TEMP%\ib.ps1 见 2026-09-20-core-dll-dabacon-write-path-algorithms.md §7；实例现为 idalib-4000
& $env:TEMP\ib.ps1 exec idalib-4000 --sql "SELECT decompile(0x5AD5060)"                        # db_cxx_put_str_vector：FORMAT 分支
& $env:TEMP\ib.ps1 exec idalib-4000 --sql "SELECT decompile(0x5B8F3C0), decompile(0x5471E50), decompile(0x54719F0), decompile(0x5471FC0), decompile(0x54B00A7)"   # 摊开 / 打包 / CHPACK
& $env:TEMP\ib.ps1 exec idalib-4000 --timeout-s 120 --sql "SELECT decompile(0x54B2CE8)"       # 转换器（1 412 行）
& $env:TEMP\ib.ps1 exec idalib-4000 --sql "SELECT decompile(0x54B9032), decompile(0x54B88D7), decompile(0x54B7297), decompile(0x54B8E01)"   # 6 的两张表、UTF-8 解码、CP1252
& $env:TEMP\ib.ps1 exec idalib-4000 --sql "SELECT decompile(0x5AE76F0), decompile(0x5AD9DB0)"  # MBCHARSET 的写点与 API 入口
& $env:TEMP\ib.ps1 exec idalib-4000 -c "import idautils, idc, ida_funcs`n_result_=[(hex(x), ida_funcs.get_func_name(x), idc.GetDisasm(x)) for x in idautils.DataRefsTo(0x6453EDC)]"   # 用 DataRefsTo，别手写 get_next_dref_to 循环
& $env:TEMP\ib.ps1 exec idalib-4000 --timeout-s 600 -c (Get-Content $env:TEMP\dump_cs6.py -Raw)   # 全 BMP 跑 sub_54B9032 → cs6_table.json
```
