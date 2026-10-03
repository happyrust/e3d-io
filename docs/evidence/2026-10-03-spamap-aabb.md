# 取证：Built_In_AABB 规则——ORI、复合、BOX / CYLI 外包、尺寸下限、精度

日期：2026-10-03
任务：write-path-phase-3 7.5
规则：aabb.ori_form, aabb.compose, aabb.box, aabb.cyli, aabb.degenerate, aabb.arithmetic

> 按 `.kiro/specs/write-path-phase-3/requirements.md` 需求 1 第 7–9 条、需求 2 第 4 条与需求 6 第 1、16 条写；交稿前跑
> `powershell -NoProfile -File tools/ida/check_evidence.ps1 docs/evidence/2026-10-03-spamap-aabb.md`。
> 取证源按计划 Q1 与 `e67b1c0`：Core3D_IDB 与 Core_IDB 都认。

## 环境

本轮首条与末条调用对两个 IDB 各跑一次 `ib.ps1 env`。

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |
| Core3D_IDB | `D:\ida_scratch\plant3\Core3D.dll.i64`（IDB 记录的输入文件即门用 E3D 的 `Core3D.dll`，哈希见 spec 术语表 Core3D_IDB） |
| Core3D_IDB SHA-256（首条调用前） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| IDA 版本 | 9.2（两个实例相同） |
| HEAD | `d88863ecc611605b36f1abb1a57a8db4c604ef56` |
| Core3D_IDB SHA-256（末条调用后） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| Core_IDB SHA-256（末条调用后） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |

交叉引用核对（复现命令第 17–28 条），全部 PASS：Core3D_IDB `0x103793D4` 70 条、`0x103782B8` 91 条、`0x10378EE0` 10 条、`0x1036A31C` 13 条；Core_IDB `0x526AC64` 18 条、`0x526D1A4` 3 条、`0x526D7E0` 7 条、`0x526C67C` 11 条、`0x55FACE8` 1 条、`0x55FCA18` 1 条、`0x55FB004` 4 条、`0x6453B98`（全局错误码，被大量引用）2 478 条。

## 结论

六条规则的 E3D 做法都已读到。Core3D 的 `desdblib/WSTRAM` 从元素往上到 WORL，逐级把 `desdblib/GETRAM` 给出的位姿放在已累积的变换之前；`GETRAM` 的旋转来自 `desdblib/GETORI` → core 的 `mathlib/GTORI1`，即 R = Rz · Ry · Rx，正余弦由 `mathlib/SINCOS` 给出，0、±90、±180 度为精确值。`dbboxlib/BOXELE` 把小于 0.5 mm 的尺寸一律取 0.5，再交 core 的 `MBXBOX`（BOX）或 `MBXSNO` / `MBXCIR`（CYLI 两端圆求并）。全程 double。候选随之换成与 E3D 同式的写法（`candidates::ori_by_gtori1`、`every_pose_from_the_element_up`、`cylinder_by_end_circles`），Built_In_AABB 在候选集下仍为 1 572 / 1 590（复现命令第 29 条），18 个反例不变，偏差 1.5–273 mm，不是算式或取整差。`aabb.degenerate` 坐实，实现为 `rules::degenerate_below_half`，`src/core_rules.rs` 改为 Confirmed，`src/spatial/aabb.rs` 改为对小于 0.5 mm 的尺寸调它；其余五条按 ① 读出，但适用实例里有这 18 个反例，按「坐实」定义为未坐实，待里程碑 D 的 7.3 按需求 4 第 17 条剔除后再判。

### `aabb.degenerate`

- 状态：坐实
- 例程：`dbboxlib/BOXELE` @ `0x1036A31C`（Core3D_IDB）n = 103–111：box type 2 读 HEIG（@ `0x10B68EF8` = 675926）、DIAM（@ `0x10B68EFC` = 788296），各自小于 0.5 即取 0.5，再调 `MBXSNO`；n = 112–125：box type 3 读 XLEN、YLEN、ZLEN（@ `0x10B68F00` / `0x10B68F04` / `0x10B68F08` = 810996 / 810997 / 810998），同样小于 0.5 取 0.5，再调 `MBXBOX`。负数、0 与 (0, 0.5) 一律为 0.5。
- 实现：`rules::degenerate_below_half`；`aabb.rs` 的阈值常量 `DEGENERATE_BELOW_MM` = 0.5（原先只对不大于 0 的尺寸查这条规则）。
- Corpus 核对：适用实例为 TES023 的两个零尺寸 CYLI `17419/184`、`17419/284`，候选集下 2 / 2 逐字相符（第 29 条，TES023 201 / 201）。Corpus 没有负尺寸与 (0, 0.5) 的 BOX / CYLI，分不出「小于 0.5 取 0.5」与旧候选「0 取 0.5、负取绝对值」，所以这一句只以出处 ① 坐实。

### `aabb.ori_form`

- 状态：未坐实
- 读到的：`desdblib/GETRAM` @ `0x103782B8`（Core3D_IDB）n = 39–49：IFCOMP 为真的元素以 `desdblib/GETORI` @ `0x10378EE0` 取 ORI 的旋转、POS 为平移；`GETORI` n = 14–25 读 ORI 三个实数交 core `mathlib/GTORI1` @ `0x526AC64`；`GTORI1` n = 14–35：从单位阵起，第一角绕 x（元 4、5、7、8），第二角绕 y（元 0、2、6、8）左乘，第三角绕 z（元 0、1、3、4）左乘，即 R = Rz · Ry · Rx。`mathlib/SINCOS` @ `0x526D1A4` n = 11–37：`LREQ` @ `0x526D7E0`（n = 3：\|a − b\| < max(\|容差\|, 10⁻⁵ · (\|a\| + \|b\|))）以容差 0.0001 判角为 0、\|90\|、\|180\| 时给精确的 0 / ±1 / −1，其余为 sin、cos(角 / 57.29577951308232)。
- 所作假设：候选 `ori_by_gtori1` 照此实现（旧候选 `ori_degrees_x_then_y_then_z` 形式相同、但没有精确特例，留作备选）。
- 须用到它的输入类别：每个 BOX / CYLI 的 Built_In_AABB（owner 链上各级的 ORI）。
- 反例：见下「18 个反例」。

### `aabb.compose`

- 状态：未坐实
- 读到的：`desdblib/WSTRAM` @ `0x103793D4`（Core3D_IDB）n = 46–125：从元素起，到 WORL 停；每级取 `GETRAM` 的位姿 T，`CONCAT(T, 已累积)` 后再上溯 owner，即 T_顶 ∘ (… ∘ (T_owner ∘ T_元素))；n = 123–125 遇某 noun 字段为真即停（未细读）。core `mathlib/CONCAT` @ `0x526C67C` n = 12–23：旋转 Σ_k A(i,k) · B(k,j)，平移 Σ_k A(i,k) · B_t(k) + A_t(i)。
- 所作假设：候选 `every_pose_from_the_element_up` 照此顺序与算式（旧候选自顶向下复合，数学相同，舍入次序不同，留作备选）。
- 须用到它的输入类别与反例：同上。

### `aabb.box`

- 状态：未坐实
- 读到的：`BOXELE` n = 112–125 → core `MBXBOX` @ `0x55FACE8` n = 19–39：三个半长各经旋转（`MVMULT`），每个世界轴取三个分量绝对值之和 s，框为中心 ± s。候选 `box_about_its_centre` 同式。
- 须用到它的输入类别与反例：同上。

### `aabb.cyli`

- 状态：未坐实
- 读到的：`BOXELE` n = 103–111 → core `MBXSNO` @ `0x55FCA18` n = 18–31：两个端圆的圆心 = 元素变换 ∘ 平移 (0, 0, ±HEIG / 2)（`CONCAT`），半径 DIAM / 2，各调 core `MBXCIR` @ `0x55FB004`；`MBXCIR` n = 12–33：每个世界轴 r · √(1 − z_i²)（被开方数为负取 0），第一圆直接赋值，第二圆取并。
- 所作假设：候选 `cylinder_by_end_circles` 照此算式次序（旧候选 `cylinder_about_its_centre` 数学相同，留作备选）。
- 须用到它的输入类别与反例：同上。

### `aabb.arithmetic`

- 状态：未坐实
- 读到的：上列例程全为 double；`SINCOS` 用 double 的 sin / cos；未见单精度。
- 所作假设：候选 `double_precision`。
- 反例：18 个反例无一差 ±1（0.1 mm），都不是精度造成。

### 18 个反例（D 的剔除清单草稿）

都在 `ams7999_0001` 会话 200。差值为「盘上 − 候选算出」，单位 0.1 mm，次序 [Key0, minx, miny, minz, maxx, maxy, maxz, flags]（复现命令第 29 条逐个打印）：

| refno | noun | 差值 | 形态 |
|---|---|---|---|
| `24383/78369`、`24383/78372`、`24383/78299`、`24383/78302` | BOX | [0, 0, 0, −15, 0, 0, 0, 0] | 只 minz 低 1.5 mm |
| `24383/76689`、`24383/76692` | BOX | [−1291, −1291, 0, 0, 0, 2575, 0, 0] | 成对，x / y 大偏差 |
| `24383/76700`、`24383/76703` | BOX | [−908, −908, −2733, 0, 0, 0, 0, 0] | 成对 |
| `24383/76711`、`24383/76714` | BOX | [−907, 0, −907, 0, 2733, 0, 0, 0] | 成对 |
| `24383/76685` | BOX | [99999900, −832, 0, −100, 0, 417, 100, 0] | Key0 换桶 |
| `24383/76696` | BOX | [99999900, −883, −293, −100, 0, 0, 100, 0] | Key0 换桶 |
| `24383/76707` | BOX | [99999900, 0, −883, −100, 293, 0, 100, 0] | Key0 换桶 |
| `24383/77889` | BOX | [−220, −220, 0, −1140, 0, 1119, 0, 0] | |
| `24383/71552`、`24383/71555` | CYLI | [0, −22, 0, 0, 21, 0, 0, 0] / [0, −21, 0, 0, 21, 0, 0, 0] | x 两侧各宽 2.1 mm（绕 Z −35°） |
| `24383/77136` | CYLI | [−600013543, 0, 0, −330, 0, 0, 0, 0] | 换轴换桶 |
| `24383/77023` | CYLI | [−280, 0, 0, −280, 0, 0, 0, 0] | 只 minz |

里程碑 B 的 B8 已把多数判为「元素或祖先改过、条目没有重算」（`76689`–`76714` 在 EQUI `24383/76459` 下）。剔除须 D 的 7.3 坐实 `spamap.descendants.*` 与 `spamap.recompute.*` 后，按需求 4 第 17 条逐个对照会话链：条目最后一次写在哪个会话、此后元素或祖先改过什么。本文未逐个走会话链。

> 更正（2026-10-03，里程碑 D 的 D3）：这 18 个不是过期条目，B8 的判断不成立。每个元素都有 IMAP 3 成员（负实体 NBOX、NCYL、NCON、NPYR），Core3D 的 `MPBOX2` 把成员的框并进元素框，本文的候选没有算；补上 `aabb.members`、`aabb.cone`、`aabb.pyramid` 后 18 个全部逐字相符，没有剔除任何元素，上面五条随之坐实。见 `docs/evidence/2026-10-03-spamap-aabb-members.md`。

## 复现命令

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x103793D4
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x103782B8
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x10378EE0
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1036A31C 98 126
powershell -NoProfile -File tools/ida/pseudo.ps1 0x526AC64
powershell -NoProfile -File tools/ida/pseudo.ps1 0x526D1A4
powershell -NoProfile -File tools/ida/pseudo.ps1 0x526D7E0
powershell -NoProfile -File tools/ida/pseudo.ps1 0x526C67C
powershell -NoProfile -File tools/ida/pseudo.ps1 0x55FACE8
powershell -NoProfile -File tools/ida/pseudo.ps1 0x55FCA18
powershell -NoProfile -File tools/ida/pseudo.ps1 0x55FB004
powershell -NoProfile -File tools/ida/sql.ps1 "SELECT printf('0x%X', a) AS ea, read_i32(a) AS lo, read_i32(a + 4) AS hi FROM (SELECT 0x5D2DB18 AS a UNION ALL SELECT 0x5D2DB20 UNION ALL SELECT 0x5D2DB28 UNION ALL SELECT 0x5D2DB30)"
powershell -NoProfile -File tools/ida/sql.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 "SELECT printf('0x%X', a) AS ea, read_i32(a) AS v FROM (SELECT 0x10B68EF8 AS a UNION ALL SELECT 0x10B68EFC UNION ALL SELECT 0x10B68F00 UNION ALL SELECT 0x10B68F04 UNION ALL SELECT 0x10B68F08)"
powershell -NoProfile -File tools/ida/sql.ps1 "SELECT name, start_ea FROM funcs WHERE name IN ('GTORI1', 'LREQ', 'CONCAT', 'MBXBOX', 'MBXSNO', 'MBXCIR')"
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x103793D4 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x103782B8 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x10378EE0 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x1036A31C --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x526AC64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x526D1A4
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x526D7E0
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x526C67C
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x55FACE8
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x55FCA18
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x55FB004
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x6453B98
cargo test --release --test spamap_aabb_real -- --nocapture --test-threads 1
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/ib.ps1 env
```

第 14 条的两个 32 位字拼成 double 即 0、0.0001、90、180（`SINCOS` 的比较常量）；第 29 条逐库打印候选集下的 Built_In_AABB 一致数 / 总数、备选的一致数与每个反例的差值。

## 反例

`aabb.degenerate`：无（适用实例 2 / 2）。

其余五条：上表 18 个（`ams7999_0001`），全部待 D 剔除；候选集下 Built_In_AABB 合计 1 572 / 1 590（`ams8000_0001` 79 / 79、`ams7999_0001` 1 292 / 1 310、`TES023` 201 / 201）。
