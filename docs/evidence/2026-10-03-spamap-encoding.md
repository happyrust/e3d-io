# 取证：SPAMAP 编码规则——取整、字、最短轴、桶、Key0、表形与页头

日期：2026-10-03
任务：write-path-phase-3 7.1
规则：spamap.rounding, spamap.word_sign, spamap.short_axis, spamap.zero_extent, spamap.bucket, spamap.key0, spamap.table_shape, spamap.table_header_rest, spamap.entries_k, spamap.flags

> 按 `.kiro/specs/write-path-phase-3/requirements.md` 需求 1 第 7–9 条写；交稿前跑
> `powershell -NoProfile -File tools/ida/check_evidence.ps1 docs/evidence/2026-10-03-spamap-encoding.md`。
> 出处 ① 按 2026-10-03 的 spec 改动（`e67b1c0`）认 E3D_IDB：编码在 Core3D_IDB，表与页在 Core_IDB。

## 环境

本轮首条与末条调用对两个 IDB 各跑一次 `ib.ps1 env`。

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |
| Core3D_IDB | `D:\ida_scratch\plant3\Core3D.dll.i64`（IDB 记录的输入文件即门用 E3D 的 `Core3D.dll`，哈希见 spec 术语表 Core3D_IDB） |
| Core3D_IDB SHA-256（首条调用前） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| IDA 版本 | 9.2（两个实例相同） |
| HEAD | `e67b1c021db9a32e9a8dfbfc0682965fe056f149` |
| Core3D_IDB SHA-256（末条调用后） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| Core_IDB SHA-256（末条调用后） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |

交叉引用核对（复现命令第 15–22 条），全部 PASS：Core3D_IDB `0x1039277C` 2 条、`0x1039210C` 14 条、`0x1038F5E4` 2 条、`0x10AEC574` 1 条；Core_IDB `0x5AB91B0` 11 条、`0x5AFF6D0` 5 条、`0x5B00F10` 6 条、`0x6453B98`（全局错误码，被大量引用）2 478 条。

## 结论

条目由 Core3D.dll 的 FORTRAN 库算出：`mapboxlib/MBXGO` 把框交 `maplib/MAPINS`，后者取整、组 flags 与下标、经 `MAPKEY` / `MAPKY2` 得 Key0，再 `DPUTIT(SPAMAP, 下标, Key0, 7 字)` 交 core.dll；core.dll 的类型 14 表属性 `sub_5AB91B0` 删旧键、插新键并写记录项。坐实的八条落在 `src/spatial/rules.rs`，`src/core_rules.rs` 改为 Confirmed；在坐实集下 Corpus 的 Key0 一项 28 269 / 28 269、基线会话 640 3 746 / 3 746，均无 `waiting`（复现命令第 23、24 条）。

### `spamap.rounding`

- 状态：坐实
- 例程：`maplib/MAPINS` @ `0x1038F5E4`（Core3D_IDB）；行为所在：`pseudocode.n` = 74–79：每轴 min、max 各取 `f_ldnint_val(10 × (坐标 − datum × 10^8))` 存为整数；`f_ldnint_val` 是 libifcoremd 的导入（@ `0x10AEC574`），即 Fortran DNINT，就近取整、半数远离零。datum 在 n = 59–66：每轴取两端中绝对值较大的一个，除以 10^8 mm 向零截断；n = 72：减去 datum 后仍超出 ±10^8 mm 即报错不写。
- 最短轴与桶按取整后的 0.1 mm 整数判定：n = 149 把取整后的六个整数交给 `MAPKEY`（需求 4 第 1 条）。
- 实现：`rules::round_half_away_from_zero`（`f64::round` 即半数远离零）。datum 只在某轴坐标达 10^8 mm 时非 0，由 `spamap.word_sign` 的值域挡在外面，所以实现里 datum 恒为 0。
- Corpus 核对：取整没有能单独核对的项（盘上只有取整后的值）。候选 `round_to_nearest` 与本规则同一函数，Built_In_AABB 在候选集下 1 572 / 1 590 逐字（里程碑 B 计划 §6 B8），18 个反例偏差都在 1.5 mm 以上、无一差 ±1，归 7.5 / 7.3，不是取整造成。

### `spamap.word_sign`

- 状态：坐实
- 值字：`MAPINS` @ `0x1038F5E4` n = 76、79 把取整结果存为 INTEGER*4，即有符号 32 位。
- Key0 的宽度与越界：`maplib/MAPKY2` @ `0x1039210C`（Core3D_IDB）n = 8–13：min 夹到 ±5 × 10^7，Key0 = min + 10^8 × N，N 至多 18，Key0 恒在 [5 × 10^7, 1.85 × 10^9]，有符号字放得下且非负，不会越界。
- 键比较：core.dll `sub_5B00F10` @ `0x5B00F10`（Core_IDB）二分查找时逐字有符号比较，指令 @ `0x5B00F88`（`cmp edx, esi`）、@ `0x5B00F8A`（`jl`）、@ `0x5B00F8C`（`jg`）；随后的线性段 @ `0x5B01048` 至 @ `0x5B0106E` 先单独认哨兵 0x80000001，其余按无符号比（`ja`、`jb`）。SPAMAP 的四个键字都小于 2^31，两种比较次序相同。
- 值域：datum 为 0 时坐标绝对值小于 10^8 mm；`rules::SIGNED_WORDS` 取 0.1 mm 整数 ±999 999 999，超出的元素由 codec 以 OutOfRange 拒绝（e3d-io 不写 datum）。
- Corpus 核对：Key0 项按存储字有符号解码，28 269 / 28 269（第 23 条）；基线与 Corpus 共 32 015 条，键字最大为 `[1700506000, 2013287686, 101285, 1]`，全部小于 2^31，坐标在 [−621 000, 1 451 700]（0.1 mm），都在 datum 为 0 的值域内（第 25 条）。

### `spamap.short_axis`

- 状态：坐实
- 例程：`maplib/MAPKEY` @ `0x1039277C`（Core3D_IDB）；行为所在：`pseudocode.n` = 15–17 算三轴 extent（取整后的 0.1 mm 整数）；n = 18–35：z 不大于 x 且不大于 y 取 z，否则 y 不大于 x 且不大于 z 取 y，否则取 x——并列取后一根（z 先于 y 先于 x）；n = 64–68：所取 extent 达 200 000（20 m）时轴改为 x。
- 实现：`rules::least_axis_last_of_ties`。
- Corpus 核对：Key0 项 28 269 / 28 269，其中最短轴并列的 6 236 条（里程碑 B 计划 §2 的计数）全符；Corpus 没有最短 extent 达 20 m 的条目，「改取 x」一句只以出处 ① 坐实（需求 4 第 18 条）。

### `spamap.zero_extent`

- 状态：坐实
- 例程：`maplib/MAPKEY` @ `0x1039277C`；行为所在：`pseudocode.n` = 36–62 对 extent 为 0 不设特例，与其他小于 600 的 extent 一样落桶 1。
- 实现：`rules::leave_zero_extent_to_the_bucket`（返回「交给桶规则」）。
- Corpus 核对：Key0 项内全符，含 `ams251142_0001` 的 138 条零尺寸条目与 `ams7999_0001` 的点条目。

### `spamap.bucket`

- 状态：坐实
- 例程：`maplib/MAPKEY` @ `0x1039277C`；行为所在：`pseudocode.n` = 36–68：最短 extent 小于 600、2 000、6 000、20 000、200 000（0.1 mm）依次为 level 0–4，界值归上一桶；不小于 200 000 为 level 5，且轴改为 x；桶 = level + 1（`MAPKY2` @ `0x1039210C` n = 13 的 `level + 6 × (轴 − 1) + 1`）。
- 实现：`rules::bucket_by_mapkey`。
- Corpus 核对：Key0 项 28 269 / 28 269；此前只知第四界落在 (19 800, 21 200]，现为 20 000。Corpus 没有桶 6 的条目，此桶只以出处 ① 坐实。

### `spamap.key0`

- 状态：坐实
- 例程：`maplib/MAPKEY` @ `0x1039277C` n = 69–72：所取轴的 min 先求 `% 50 000 000`（截断余数，与 Fortran MOD 相同），再调 `MAPKY2`；`maplib/MAPKY2` @ `0x1039210C` n = 8–13：min 夹到 ±5 × 10^7，Key0 = min + 10^8 × (level + 6 × (轴 − 1) + 1)，轴 x / y / z 为 1 / 2 / 3。
- 实现：`rules::key0_by_mapky2`。
- Corpus 核对：坐实集下 Key0 项 `ams8000_0001` 会话 646 3 746 / 3 746、`ams7999_0001` 会话 200 23 891 / 23 891、`ams251142_0001` 会话 10 327 / 327、`TES023` 会话 109 305 / 305，合计 28 269 / 28 269；基线会话 640 3 746 / 3 746（第 23、24 条）。Corpus 中 min 的绝对值都不到 5 km，取模一句只以出处 ① 坐实。

### `spamap.table_shape`

- 状态：坐实
- 例程：core.dll `sub_5AB91B0` @ `0x5AB91B0`（Core_IDB，类型 14 表属性写）；行为所在：`pseudocode.n` = 261–273：键 = [值（Key0）, refno 第一字, refno 第二字, 下标 − 1]，键宽 = 元素宽 1 + 3 = 4；表名即属性名 hash（`MAPINS` 的 `DPUTIT` 第一参为 230902453，读自 Core3D_IDB @ `0x10B6C8B4`）；值 = 调用方给的 7 字（`MAPINS` @ `0x1038F5E4` n = 149–152：框 6 字 + flags，长度 7）。k 即下标 − 1，下标由 `MAPINS` n = 81–84 定。
- 内结点槽为键 + 子页 `(pgno, extno)`，与反向引用表同一套通用表代码（读侧 `src/index/keyed.rs`）。
- Corpus 核对：基线与 Corpus 共 1 903 页全部按此形状解码、无错（第 25 条；读侧普查同）。

### `spamap.table_header_rest`

- 状态：坐实
- 例程：core.dll `sub_5AFF6D0` @ `0x5AFF6D0`（Core_IDB，建表）；行为所在：`pseudocode.n` = 44–50：新根页 7 个页头字为 [5, 表名 hash, 0, 键宽, 值宽, 定义字 & 7, 页字数 − 7]；类型 2 的定义（n = 24–28）给键宽 = 元素宽 + 3。
- 页头其余字：第 3–5 字为 4、7、1，内结点也一样（值宽仍记叶的 7）；第 6 字为空闲字数，新根为页字数 − 7。
- Corpus 核对：基线与 Corpus 共 1 903 页（53 个内结点）页头全为 `[5, 230902453, level, 4, 7, 1, free]`（第 25 条）。内结点的这三个字只以出处 ② 坐实：分裂结点的例程未读。

### `spamap.entries_k`

- 后续：同日在 `docs/evidence/2026-10-03-spamap-branch-entries.md` 读完 `MPBXBR` 后坐实（分支与吊架树上为 MissingInput）。
- 状态：未坐实
- 读到的：k = `MAPINS` 下标 − 1，下标 n = 81–84 在 tube 为真时取 2，否则取 1；`sub_5AB91B0` n = 269 存 `下标 − 1`。`mapboxlib/MPBOX2` @ `0x102FBD62` n = 93 以常量 tube = 0（Core3D_IDB @ `0x10B2FDCC`）调 `mapboxlib/MBXGO` @ `0x102FD8C2`，经它记的元素只有 k = 0 一条，框就是元素自己的框。k = 1 只来自 `mapboxlib/MPBXBR` @ `0x102FC293`：`mapboxlib/MPBOX` @ `0x102FB475` 在 noun 为 BRAN 或 `HTCOMP` 为真时改走它，且不再向下走。Corpus 的 k = 1 条目只在 BRAN（只有 k = 1）与管件上（ATTA、BEND、CAP、COUP、ELBO、FLAN、FTUB、OLET、PCOM、REDU、TEE、VALV、WELD，普查 JSON）。
- 所作假设：经 `MPBOX2` 记的元素一条 k = 0、用自身的框；`MPBXBR` 下的元素另有 tube 条目。
- 须用到它的输入类别：每个有条目的元素；分不清时要 noun 的 IMAP（走树方式，7.2 读字典）与 `MPBXBR` 的 tube 框（`mapboxlib/MGTUBE`，未读）。
- 反例：候选「单条 k = 0」在会话 640 下 3 507 / 3 628、`ams7999_0001` 下 18 404 / 21 257，差的恰是带 k = 1 条目的元素（里程碑 B 计划 §6 B5）。

### `spamap.flags`

- 状态：未坐实
- 读到的：`MAPINS` n = 52–58：flags = 三个逻辑量按 1、2、4 相加；`MBXGO` @ `0x102FD8C2` n = 12–24：第六参为假时三量清零；`MPBOX` @ `0x102FB475` n = 42–62 定这一参：沿 owner 链上溯，遇 SITE、BLOCK、SSOWLD、HBLWLD，或 DDEP = 1 且 noun 不是 TPWL、APPLDW、TRNWLD、FEMWLD，即为真，到 WORL 为假；`dbboxlib/BOXELE` @ `0x1036A31C` n = 85–88：OBST 为 1 或 2 时置第 OBST 位；`MPBOX2` n = 84–86：元素带 WBOX 时 OBST 为 1–3 都置位。管件的 0 / 2 / 3 / 6 / 7 来自 `MPBXBR`，未读。
- 所作假设：BOX、CYLI 走 `BOXELE`，flags = OBST（1 或 2）或 0。
- 须用到它的输入类别：每个条目。
- 反例：候选 `flags_from_obst` 在 BOX / CYLI 上 1 590 / 1 590，其余 noun 不覆盖（里程碑 B 计划 §6 B3）。

## 复现命令

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1039277C
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1039210C
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1038F5E4 40 160
powershell -NoProfile -File tools/ida/sql.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 "SELECT name, module, printf('0x%X', address) AS ea FROM imports WHERE name = 'f_ldnint_val'"
powershell -NoProfile -File tools/ida/sql.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 "SELECT printf('0x%X', a) AS ea, read_i32(a) AS v FROM (SELECT 0x10B6C8B4 AS a UNION ALL SELECT 0x10B2FDCC)"
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FB475
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FBD62
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FD8C2
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1036A31C 80 90
powershell -NoProfile -File tools/ida/pseudo.ps1 0x5AB91B0 261 273
powershell -NoProfile -File tools/ida/pseudo.ps1 0x5AFF6D0
powershell -NoProfile -File tools/ida/sql.ps1 "SELECT printf('0x%X', address) AS ea, disasm FROM instructions WHERE address BETWEEN 0x5B00F85 AND 0x5B00F8C OR address BETWEEN 0x5B01048 AND 0x5B0106E ORDER BY address"
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x1039277C --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x1039210C --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x1038F5E4 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x10AEC574 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x5AB91B0
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x5AFF6D0
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x5B00F10
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x6453B98
cargo run --release --quiet --bin e3d-descriptor -- spamap-check --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001 --input D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001=646 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001=200 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001=10 --session D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023=109 --schema schema/e3d31 --rules confirmed
cargo run --release --quiet --bin e3d-descriptor -- spamap-check --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001=640 --schema schema/e3d31 --rules confirmed
cargo test --release --test spamap_page_headers_real -- --nocapture --test-threads 1
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/ib.ps1 env
```

第 23、24 条输出 JSON，各库 `key0` 的 `agree` / `total` / `waiting` 即上文数字；第 25 条打印页数、内结点数、键字最大值与坐标范围。

## 反例

坐实的八条：无（Key0 项 28 269 / 28 269 与 3 746 / 3 746，页头 1 903 / 1 903，键字与坐标 32 015 / 32 015）。

未坐实的两条（候选数据，非本文规则的反例）：`spamap.entries_k` 的候选「单条 k = 0」缺带 k = 1 条目的元素，会话 640 121 个、`ams7999_0001` 2 853 个；`spamap.flags` 的候选只覆盖 BOX / CYLI，管件 flags 0 / 2 / 3 / 6 / 7 记 `waiting`。
