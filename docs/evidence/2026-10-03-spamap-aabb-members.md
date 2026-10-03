# 取证：Built_In_AABB 并入 IMAP 3 成员的框——aabb.members、aabb.cone、aabb.pyramid，与 aabb 其余五条坐实

日期：2026-10-03
任务：write-path-phase-3 7.5（里程碑 D 的 D3）
规则：aabb.members, aabb.cone, aabb.pyramid, aabb.ori_form, aabb.compose, aabb.box, aabb.cyli, aabb.arithmetic

> 按 `.kiro/specs/write-path-phase-3/requirements.md` 需求 1 第 7–9 条、需求 2 第 4 条、需求 4 第 17 条与需求 6 第 1、9、16 条写；交稿前跑
> `powershell -NoProfile -File tools/ida/check_evidence.ps1 docs/evidence/2026-10-03-spamap-aabb-members.md`。
> 接 `docs/evidence/2026-10-03-spamap-aabb.md`（C4）：那里读出的 `WSTRAM`、`GETORI`、`GTORI1`、`SINCOS`、`CONCAT`、`MBXBOX`、`MBXSNO`、`MBXCIR` 本文只引用不重述。取证源 Core3D_IDB 与 Core_IDB 都认（`e67b1c0`）。

## 环境

本轮首条与末条调用对两个 IDB 各跑一次 `ib.ps1 env`。

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |
| Core3D_IDB | `D:\ida_scratch\plant3\Core3D.dll.i64` |
| Core3D_IDB SHA-256（首条调用前） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| IDA 版本 | 9.2（两个实例相同） |
| HEAD | `3b71596225c6486676388f86d87c36b0618f761d` |
| Core3D_IDB SHA-256（末条调用后） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| Core_IDB SHA-256（末条调用后） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |

交叉引用核对（复现命令第 20–28 条），全部 PASS：Core3D_IDB `0x102FBD62` 1 条、`0x1036A31C` 13 条、`0x108E7A70` 2 条、`0x103782B8`（`GETRAM`，被大量引用）91 条；Core_IDB `0x55FBAB4` 1 条、`0x55FCA18` 1 条、`0x55FB7B4` 27 条、`0x526C430` 9 条、`0x53A9974` 4 条。

## 结论

C4 留下的 18 个 Built_In_AABB 反例（`ams7999_0001` 会话 200）不是过期条目，而是候选少算了一步：Core3D 的 `mapboxlib/MPBOX2` 对 noun 的 IMAP 为 5 的元素（BOX、CYLI 都是）会把它每个 IMAP 3 成员——负实体 NBOX、NCYL、NCON、NPYR 等——的框并进元素框，候选只算了元素自身。18 个元素各有 1–3 个负实体成员（NBOX 7、NCYL 7、NPYR 18、NCON 1），差值都在成员伸出的那几侧；例如 `24383/78369` 的 NBOX `24383/78370` 在 z −3、ZLEN 105，下沿比 ZLEN 108 的 BOX 低 1.5 mm，正是盘上 minz 比候选低的 15（0.1 mm）。

E3D 本身也这样算（出处 ②，复现命令第 29、30 条，门用副本从原件重置）：给 18 个元素各设 LEVE（DCHC 4，不动框），SAVEWORK 会话 201 里 18 个的 LEVE 都改了、整表没有一条改写——E3D 按最终状态重算出的框就是盘上那个；把 18 个各加大 100 mm（BOX 的 XLEN、CYLI 的 DIAM），18 个条目都被改写，而原先多出的那几侧照旧。

并入成员框（`aabb.members`）、补上 NCON 与 NPYR 的框（`aabb.cone`、`aabb.pyramid`，NBOX、NCYL 与 BOX、CYLI 同 box type）后，坐实集下 Built_In_AABB 在 Corpus 各库最新会话为 1 590 / 1 590（`ams8000_0001` 79 / 79、`ams7999_0001` 1 310 / 1 310、`TES023` 201 / 201，`ams251142_0001` 无 BOX / CYLI，`ams7351_0001` 无表），基线会话 640 为 79 / 79，没有 waiting；E3D 刚重算的会话 201 也是 1 310 / 1 310（复现命令第 31–33 条）。按需求 4 第 17 条一个反例都不剔除，需求 6 第 16 条的放宽判据也不需要。八条规则坐实，`src/core_rules.rs` 改为 Confirmed，实现在 `src/spatial/rules.rs`（原候选移入），`src/spatial/aabb.rs` 并入成员框。

### `aabb.members`

- 状态：坐实
- 例程：Core3D `mapboxlib/MPBOX2` @ `0x102FBD62`（Core3D_IDB）。n = 33–36：读不到 WBOX（@ `0x10B2FDB4` = 1014845）时以 `dbboxlib/BOXELE` 算元素自身的框；n = 37：自身框无误且元素 noun 的 IMAP（第二参）为 5 才进成员循环；n = 41–75：`DFIND(NEXT, 1)`（@ `0x10B2FDBC` = 942746，@ `0x10B2FDC0` = 1）逐个成员，`DGETI(TYPE)`（@ `0x10B2FDC4` = 642215）取 noun，`UDSOFT` / `ATNINT` 取 IMAP，n = 50 为 3 才算；n = 52–58：core `nounlib/IFCOMP` @ `0x53A9974`（n = 10：`ATNINT(noun, 600459)`，即 noun 字段 FRMC，@ `0x5D67844` = 600459）为 0、或 noun 为 STRU（958465）/ PANE（640105）时，`desdblib/GETRAM` @ `0x103782B8` 取成员自身位姿，n = 56 `CONCAT(元素变换, 成员位姿)` 后交 `BOXELE`；否则 n = 61 以元素变换直接交 `BOXELE`；n = 63–71：成员框无误时每轴取较小的 min、较大的 max 并进元素框；n = 93 交 `MBXGO`。`GETRAM` n = 26–37：FRMC 不大于 0 的 noun 以 `getattlib/GTDORI` @ `0x51E22EB`（n = 14–20：`GATOR1` 取到 ORI 即交 `GTORI1`，取不到为单位阵）取旋转、`getattlib/GATPS1` @ `0x51E14E2`（n = 7 转 `GATPOS`）取 POS。`attlib.dat` 中 NBOX、NCYL、NCON、NPYR 的 FRMC 都是 0（复现命令第 34 条的测试 `the_member_nouns_have_a_frmc_of_0`），所以成员框在「元素的世界系接成员自身 POS / ORI」里算。
- 实现：`rules::imap_3_into_imap_5`；`aabb.rs` 以 `SpatialState::members` 列成员、`rules::soft_imap` 取 IMAP（UDET 为缺输入），成员系为 `WorldFrame::then`（即 `CONCAT`），按轴比较取并。成员 noun 不在 `BUILT_IN_MEMBER_NOUNS`（NBOX、NCYL、NCON、NPYR）时该元素没有内建框，给 `AabbOutcome::MissingInput`（需求 6 第 15 条：写侧按 D7 要调用方给框）。这条规则只对有成员的元素查。
- Corpus 核对：Corpus 各库最新会话中 E3D 写、有条目的 1 590 个 BOX / CYLI 里，45 个有 IMAP 3 成员（成员 NBOX 7、NCYL 34、NCON 1、NPYR 18），并入后 45 / 45 逐字相符；不并入时 `ams7999_0001` 为 1 292 / 1 310，恰差这 18 个（复现命令第 31 条的「members not taken in」行）。

### `aabb.cone`

- 状态：坐实
- 例程：`BOXELE` @ `0x1036A31C`（Core3D_IDB）n = 137–148：box type 5（`DB_Model::getBoxType` @ `0x108E7A70` n = 157–161：CONE 与 NCON 为 5）读 HEIG（@ `0x10B68EF8` = 675926）、DTOP（@ `0x10B68F10` = 857848）、DBOT（@ `0x10B68F14` = 936094）；HEIG 小于 0.5 取 0.5，DTOP、DBOT 小于 0 取 0；`MBXSNO(变换, HEIG, 0, 0, DTOP, DBOT, 框)`。core `boxlib/MBXSNO` @ `0x55FCA18`（Core_IDB）n = 18–31：第一个端圆圆心为 `CONCAT(变换, 平移 (XOFF, YOFF, HEIG) / 2)`、半径为第一个直径的一半，第二个端圆圆心取平移的负值、半径为第二个直径的一半，各交 `boxlib/MBXCIR` @ `0x55FB004`（n = 12–34：每个世界轴 r · √(1 − z_i²)，第一圆赋值、第二圆取并）。即 DTOP 在 +HEIG / 2 一端，DBOT 在另一端。
- 实现：`rules::cone_by_end_circles`，与 `cylinder_by_end_circles` 共用 `MBXSNO` 的写法；HEIG 的下限经 `aabb.degenerate`。
- Corpus 核对：适用实例为 `ams7999_0001` 的 CYLI `24383/77136` 的 NCON `24383/77138`（HEIG 21、DTOP 60、DBOT 32），该 CYLI 逐字相符。

### `aabb.pyramid`

- 状态：坐实
- 例程：`BOXELE` n = 163–174：box type 7（`getBoxType` n = 169–173：PYRA 与 NPYR 为 7）读 XTOP（@ `0x10B68F20` = 857868）、YTOP（@ `0x10B68F24` = 857869）、XBOT（@ `0x10B68F28` = 936114）、YBOT（@ `0x10B68F2C` = 936115）、HEIG、XOFF（@ `0x10B68F18` = 654342）、YOFF（@ `0x10B68F1C` = 654343），只有 HEIG 小于 0.5 取 0.5；`MBXPYR(变换, XTOP, YTOP, XBOT, YBOT, HEIG, XOFF, YOFF, 框)`。core `boxlib/MBXPYR` @ `0x55FBAB4`（Core_IDB）n = 49–88：各量乘 0.5，顶面四角 z = HEIG / 2、x = XOFF / 2 ± XTOP / 2、y = YOFF / 2 ± YTOP / 2，底面四角 z = −HEIG / 2、x = −XOFF / 2 ± XBOT / 2、y = −YOFF / 2 ± YBOT / 2，依次交 `boxlib/MBXPOI` @ `0x55FB7B4`（n = 22–50：`TRAVEC` 变到世界系，第一点赋值、其余取并）；`mathlib/TRAVEC` @ `0x526C430` n = 8–10：`MVMULT` 后加平移；`mathlib/MVMULT` @ `0x526B724` n = 9–15：每行从 0 起按列依次累加。
- 实现：`rules::pyramid_by_corners`：角点顺序与算式照 `MBXPYR`，点变换用 `WorldFrame::point`（与 `TRAVEC` 同次序）。
- Corpus 核对：适用实例为 `ams7999_0001` 9 个 BOX 下的 18 个 NPYR，9 个 BOX 全部逐字相符（含 Key0 换桶的 `24383/76685`、`76696`、`76707`）。

### `aabb.ori_form`

- 状态：坐实
- 例程：C4 文档同节——`GETORI` @ `0x10378EE0` → core `mathlib/GTORI1` @ `0x526AC64`（R = Rz · Ry · Rx）与 `mathlib/SINCOS` @ `0x526D1A4`（0、±90、±180 度精确）；本轮补读 FRMC 为 0 的 noun 所走的 `GTDORI` @ `0x51E22EB`，同样交 `GTORI1`。
- 实现：`rules::ori_by_gtori1`、`rules::sincos`。
- Corpus 核对：Built_In_AABB 1 590 / 1 590；备选「y、x、z 顺序」在 `ams7999_0001` 为 1 138 / 1 310。

### `aabb.compose`

- 状态：坐实
- 例程：C4 文档同节——`desdblib/WSTRAM` @ `0x103793D4` 从元素往上逐级 `CONCAT(T_级, 已累积)`，core `mathlib/CONCAT` @ `0x526C67C`；成员一级为 `MPBOX2` n = 55–56 的 `CONCAT(元素变换, 成员位姿)`（见 `aabb.members`）。
- 实现：`rules::every_pose_from_the_element_up`；成员系 `WorldFrame::then`。
- Corpus 核对：1 590 / 1 590。

### `aabb.box`

- 状态：坐实
- 例程：`BOXELE` n = 112–125（`getBoxType` n = 136–137、148–149：BOX 与 NBOX 为 3）→ core `boxlib/MBXBOX` @ `0x55FACE8`（C4 文档同节）。
- 实现：`rules::box_by_mbxbox`（BOX 自身与 NBOX 成员）。
- Corpus 核对：BOX 384 / 384（`ams8000_0001` 45、`ams7999_0001` 275、`TES023` 64），NBOX 成员 7 个全在这 18 个反例里。

### `aabb.cyli`

- 状态：坐实
- 例程：`BOXELE` n = 103–111（`getBoxType` n = 130–134：CYLI 与 NCYL 为 2）→ `MBXSNO(变换, HEIG, 0, 0, DIAM, DIAM, 框)` → `MBXCIR`（见 `aabb.cone`）。
- 实现：`rules::cylinder_by_end_circles`（CYLI 自身与 NCYL 成员）。
- Corpus 核对：CYLI 1 206 / 1 206（34、1 035、137）；NCYL 成员 34 个。

### `aabb.arithmetic`

- 状态：坐实
- 例程：Core3D `dbboxlib` 与 core `boxlib`、`mathlib` 的例程全为 double（C4 文档同节；本轮 `MBXPYR`、`MBXPOI`、`TRAVEC`、`MVMULT` 亦然）。
- 实现：`rules::double_precision`。
- Corpus 核对：1 590 / 1 590 逐字相符，需求 6 第 16 条的放宽判据不需要。

## 需求 4 第 20 条的输入

`tools/e3d/spamap_probes/selfcheck_e3dio_box_xlen.pml`（复现命令第 35 条）在门用副本的 `ams8000_0001`（原件 SHA-256 `3CAB5D7D…E023`）上：e3d-io 会话 647 按 `tools/e3d/write_gate_ops/10_box.json` 插 BOX `24384/26484`，E3D 会话 648 把它的 XLEN 由 300 改为 301 并 SAVEWORK，给它登记了条目。该库钉在 `D:\e3dio-fixtures\phase3\spamap-4-20\ams8000_0001`（SHA-256 `F2BB27BDCCD9C2EDDCA0B928F5D0BBF64C01521CE1F6DC8BC9E62858D27A33B2`，旁有 `MANIFEST.tsv`）。会话 648 中：

| 项 | 值 |
|---|---|
| BOX `24384/26484` | POS (1000, 2000, 3000)，ORI (0, 0, 0)，XLEN 301、YLEN 400、ZLEN 500，OBST 2，LEVE (0, 10)，没有成员 |
| owner 链 | ZONE `24384/47`、SITE `24384/46`，POS 与 ORI 都是 0；WORL `16192/0` |
| World_AABB | x [849.5, 1150.5]、y [1800, 2200]、z [2750, 3250] mm |
| 条目（E3D 所存，坐实集所得） | 键 `[300008495, 24384, 26484, 0]`，值 `[8495, 18000, 27500, 11505, 22000, 32500, 2]`（flags 2：SITE 启用、OBST 2）；记录项 SPAMAP `[300008495]` |

`tests/spamap_codec_real.rs` 的 `requirement_4_20s_box_encodes_to_exactly_its_entry`（复现命令第 36 条）先核夹具的 SHA-256，再经 `ReadState` 读会话 648：坐实集下 Built_In_AABB 与 codec 恰得这一条，与盘上所存相同。

## 复现命令

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FBD62
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1036A31C
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x108E7A70
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x103782B8
powershell -NoProfile -File tools/ida/sql.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 "SELECT printf('0x%X', a) AS ea, read_i32(a) AS v FROM (SELECT 0x10B68EE8 AS a UNION ALL SELECT 0x10B68EF8 UNION ALL SELECT 0x10B68EFC UNION ALL SELECT 0x10B68F00 UNION ALL SELECT 0x10B68F04 UNION ALL SELECT 0x10B68F08 UNION ALL SELECT 0x10B68F10 UNION ALL SELECT 0x10B68F14 UNION ALL SELECT 0x10B68F18 UNION ALL SELECT 0x10B68F1C UNION ALL SELECT 0x10B68F20 UNION ALL SELECT 0x10B68F24 UNION ALL SELECT 0x10B68F28 UNION ALL SELECT 0x10B68F2C UNION ALL SELECT 0x10B2FDB4 UNION ALL SELECT 0x10B2FDBC UNION ALL SELECT 0x10B2FDC0 UNION ALL SELECT 0x10B2FDC4)"
powershell -NoProfile -File tools/ida/sql.ps1 "SELECT name, printf('0x%X', start_ea) AS ea FROM funcs WHERE name IN ('IFCOMP', 'MBXPYR', 'MBXPOI', 'MBXSNO', 'MBXCIR', 'MBXBOX', 'TRAVEC', 'MVMULT', 'GTDORI', 'GATPS1')"
powershell -NoProfile -File tools/ida/pseudo.ps1 0x53A9974
powershell -NoProfile -File tools/ida/sql.ps1 "SELECT printf('0x%X', a) AS ea, read_i32(a) AS v FROM (SELECT 0x5D67844 AS a)"
powershell -NoProfile -File tools/ida/pseudo.ps1 0x51E22EB
powershell -NoProfile -File tools/ida/pseudo.ps1 0x51E14E2
powershell -NoProfile -File tools/ida/pseudo.ps1 0x55FBAB4
powershell -NoProfile -File tools/ida/pseudo.ps1 0x55FB7B4
powershell -NoProfile -File tools/ida/pseudo.ps1 0x526C430
powershell -NoProfile -File tools/ida/pseudo.ps1 0x526B724
powershell -NoProfile -File tools/ida/pseudo.ps1 0x55FCA18
powershell -NoProfile -File tools/ida/pseudo.ps1 0x55FB004
powershell -NoProfile -File tools/ida/pseudo.ps1 0x55FACE8
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FBD62 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x1036A31C --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x108E7A70 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x103782B8 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x55FBAB4
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x55FCA18
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x55FB7B4
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x526C430
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x53A9974
powershell -NoProfile -File tools/e3d/spamap_probe.ps1 tools/e3d/spamap_probes/d3/recompute_counterexamples.pml -DbRelative ams000\ams7999_0001
powershell -NoProfile -File tools/e3d/spamap_probe.ps1 tools/e3d/spamap_probes/d3/recompute_counterexamples_grow.pml -DbRelative ams000\ams7999_0001
cargo test --release --test spamap_aabb_real -- --nocapture --test-threads 1
cargo run --release --bin e3d-descriptor -- spamap-check --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001 --schema schema/e3d31 --rules confirmed
cargo run --release --bin e3d-descriptor -- spamap-check --input D:\e3dio-gate\AvevaMarineSample\ams000\ams7999_0001 --session D:\e3dio-gate\AvevaMarineSample\ams000\ams7999_0001=201 --schema schema/e3d31 --rules confirmed
cargo test --release --test spamap_aabb_real the_member_nouns_have_a_frmc_of_0
powershell -NoProfile -File tools/e3d/spamap_probe.ps1 tools/e3d/spamap_probes/selfcheck_e3dio_box_xlen.pml
cargo test --release --test spamap_codec_real requirement_4_20s_box_encodes_to_exactly_its_entry
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/ib.ps1 env
```

第 7 条读出 BOXTYP、HEIG、DIAM、XLEN、YLEN、ZLEN、DTOP、DBOT、XOFF、YOFF、XTOP、YTOP、XBOT、YBOT、WBOX、NEXT、1、TYPE 的名字 hash（base-27 可直接拆回名字）。第 29 条（LEVE）与第 30 条（加大 100 mm）各自从原件重置门用副本的 `ams7999_0001` 后跑一次 E3D（会话 201），结果写在副本旁的 `probe\<探针名>\result.json`；第 29 条判 18 个都是 same、整表无改写，第 30 条判 18 个都是 rewritten、整表只改写这 18 个，两条都以 PASS 退出（探针文件的 `-- expect:` 即此）。第 33 条须紧跟第 30 条，读的是它写出的会话 201。第 31 条逐库打印坐实集下的一致数，与把单条规则换成备选（不并入成员、ORI 换序、CYLI 用自身外包盒、尺寸取绝对值）时的一致数。

## 反例

无。C4 的 18 个（`ams7999_0001` 会话 200）都由 `aabb.members` 解释，补上后逐字相符，没有按需求 4 第 17 条剔除的元素。
