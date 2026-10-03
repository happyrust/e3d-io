# 取证：SPAMAP 何时重算——重算表、后代、删除残留

日期：2026-10-03
任务：write-path-phase-3 7.3（里程碑 D 的 D2）
规则：spamap.recompute.<NOUN>, spamap.descendants.move, spamap.descendants.pos, spamap.descendants.ori, spamap.delete_residue

> 按 `.kiro/specs/write-path-phase-3/requirements.md` 需求 1 第 7–9 条写；交稿前跑
> `powershell -NoProfile -File tools/ida/check_evidence.ps1 docs/evidence/2026-10-03-spamap-recompute.md`。

## 环境

本轮 IDA 只经 IB_Wrapper、只读；入口与例程在 D0 读过（`docs/plans/2026-10-03-write-path-phase-3-milestone-d-plan.md` §6），本轮复跑伪代码核行号、复核 xrefs。E3D_Session 由 `tools/e3d/spamap_probe.ps1` 驱动：门用项目副本每个探针从原件重置 `ams8000_0001`，原件只读（散列 SHA256 `3CAB5D7D2B2C40FDF0829B91C555D19D55C530DD49860A4665560E5C553FE023`，最新会话 646），E3D 3.1 Design TTY（shadow 安装 `shadow_e3d31_gen_model_test`），E3D 写的会话 USER 为 Windows 账户。

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |
| Core3D_IDB | `D:\ida_scratch\plant3\Core3D.dll.i64` |
| Core3D_IDB SHA-256（首条调用前） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| IDA 版本 | `9.2` |
| HEAD | `bb7fea6eefc8d351a63364e54c751445e23cc306` |
| Core3D_IDB SHA-256（末条调用后） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| Core_IDB SHA-256（末条调用后） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |

## 结论

### 入口（D0，本轮复核）

SPAMAP 只经 Core3D `maplist/MLSADD` @ `0x10220F82` 入队（11 处调用），出队由 `maplist/VDEMPR` @ `0x102208C7` 对每个队项跑 `mapboxlib/MPBOX` @ `0x102FB475`。本轮复跑的行号：

- 设属性：`descases/VDESPT` @ `0x101F33A9` n 48 `ATAINT` 取该属性的字典字段 `DCHC`，n 49–63 不为 0 才 `MLSADD(元素)`。core.dll `DB_ElementChangesPlugger::PostSetAttribute` @ `0x591E5B0`（D0 笔记 n 20）在 `DB_Attribute::wnoevt` @ `0x58D5290`（n 5：`DB_Attribute` 的 +184 字节）为真时不向任何订阅者发事件。
- 成员次序：`descases/VDESRO` @ `0x101F2989` n 10 `MLSADD(被挪元素, 强制)`；换 owner：`descases/VDESIR` @ `0x101F2755` n 10 同样强制入队被挪元素（先由 `VDESI1` `MPDEL`）。
- 下探：`MPBOX` n 95–120 从队项起逐个成员深度优先走：n 99–101 BRAN / HNGT noun 交 `MPBXBR` 后不再下探，n 104–115 IMAP 7 看 owner，n 116–117 IMAP 1 / 5 记条目（`MPBOX2`），n 118–119 IMAP 为 0 即停止下探，n 120 `DFIND` 进入成员。
- 删除：`descases/VDESDL` @ `0x101F1B24` → `maplist/MPDEL`，只把 owner 入队、把本元素移出队列，不删条目（D0）。

xrefs 8 / 8 PASS（复现命令第 12–19 条）。

### 探针方法

e3d-io 写的元素没有条目，E3D 一旦重算它就登记条目。所以对 e3d-io 先写好的无条目元素做一处改动后 SAVEWORK，「条目出现」= 触发、「仍无条目」= 不触发；框不变的改动（LEVE、OBST、NUMB……）在已有条目的元素上看不出，用这个办法就能看出（计划 §3.1 的盘上不可见问题由此解决）。有条目的情形用探针的准备段：同一 TTY 会话先改各元素的 LEVE（本身就触发重算）并 SAVEWORK，再做被测改动，比对两个 E3D 会话。每个探针还做整表比对，35 个探针都没有改到 watch 之外的条目，E3D 也都没报错。

### `spamap.recompute.<NOUN>`

- 状态：坐实
- 例程：`descases/VDESPT` @ `0x101F33A9`（Core3D_IDB）n 48–63；`descases/VDESRO` @ `0x101F2989` n 10；core.dll `DB_ElementChangesPlugger::PostSetAttribute` @ `0x591E5B0`、`DB_Attribute::wnoevt` @ `0x58D5290` n 5（Core_IDB）
- 规则：写属性 a 让元素重算，当且仅当 a 的字典 `DCHC` 不为 0 且 a 不是 `WNOEVT`；与 noun 无关。改成员次序时被挪的那个元素重算，同 owner 的其余成员不重算。入队之后条目改不改由 `maplib/MAPINS` 定（框与 flags 不变就不重写，`docs/evidence/2026-10-03-spamap-encoding.md`），所以「触发」的行在已有条目的元素上可能盘上不变。
- 每个 Geometric_Noun 的重算表：Geometric_Noun 取 IMAP 1、5、7 的 300 个 noun（`docs/evidence/2026-10-03-spamap-coverage.md`），模板取 `schema/e3d31/catalog/noun_layout.json`，DCHC / WNOEVT 取 `schema/e3d31/binaries/attlib.dat`（`e3d_attlib::AttrDict`）。300 个模板共 722 个属性：176 个触发（DCHC 3 的 POS、ORI、BFORI，DCHC 4 的 173 个），546 个不触发；WNOEVT 全库 7 个属性（DPOSCA、EVITCA、EVITCP、EXTEMP、LIGTCA、LIGTCP、TEMPLI），都不在这些模板里。逐 noun 的表由复现命令第 22 条打印（每 noun 一行：触发的属性 + 成员次序；不触发的属性），实现为 `src/spatial/rules.rs` 的 `RECOMPUTING` 与 `recompute_by_dchc`。BOX、CYLI 两张（属性 hash 在括号里）：

| noun | 触发 | 不触发 |
|---|---|---|
| BOX | POS (545713)、ORI (538503)、LEVE (646041)、OBST (939021)、XLEN (810996)、YLEN (810997)、ZLEN (810998)、成员次序 | NAME (639374)、PURP (860074)、ORRF (663162)、TMRREF (89649875)、REPCOU (309900969) |
| CYLI | POS、ORI、LEVE、OBST、DIAM (788296)、HEIG (675926)、成员次序 | NAME、PURP、ORRF、TMRREF、REPCOU |

- 外部输入（需求 5 第 2 条 ②）：BOX、CYLI 的 World_AABB 只读自身属性与 owner 链的 POS / ORI（`docs/evidence/2026-10-03-spamap-aabb.md` 的 `WSTRAM` / `BOXELE`），owner 链归下面的 `spamap.descendants.*`。按目录取几何的 noun，目录件改动由 `descat/ATTABK` @ `0x101F0DEE` 入队引用它的 ATTA 之 owner（D0），目录库的改动不在设计库会话里，归需求 16 第 5 条，本文不判。
- E3D_Session 核对：命令见第 23–24 条；一致数 / 总数 = 32 / 32。属性行 24 / 24：BOX 的 POS、ORI、LEVE、OBST、YLEN、ZLEN 与自检的 XLEN 7 个条目出现，NAME、PURP、REPCOU 3 个仍无；CYLI 的 DIAM、HEIG、POS、ORI、LEVE、OBST 6 个出现，NAME、PURP 2 个仍无；CONE 的 DTOP、DISH 的 DIAM、PYRA 的 XBOT 3 个出现，三者的 NAME 3 个仍无（ORRF、TMRREF 是引用 / 伪属性，未探）。成员次序 8 / 8：EQUI 下两 BOX 两 CYLI，`REORDER` 第二个 BOX 时只有它出现，`REORDER` 第二个 CYLI 时只有它出现，两次其余 3 个成员都仍无（2 + 6）。

### `spamap.descendants.move`

- 状态：坐实
- 例程：`descases/VDESIR` @ `0x101F2755`（Core3D_IDB）n 10 强制 `MLSADD(被挪元素)`；`mapboxlib/MPBOX` @ `0x102FB475` n 95–120 走其整棵子树
- 规则：被挪元素自身（几何 noun 时）、直接成员、第 2 层及更深的后代都重算；不在 IMAP 0 元素、BRAN、HNGT noun 之下继续（`rules::mpbox_walks_below`）。
- E3D_Session 核对：命令见第 24 条；一致数 / 总数 = 4 / 4。EQUI（内含 BOX、CYLI 与 SUBE，SUBE 内一 BOX，都无条目）`INCLUDE` 到第二个 ZONE：第 1 层 2 个、第 2 层 1 个条目出现，EQUI 外的 BOX 不变；把 EQUI 外的无条目 BOX `INCLUDE` 进 SUBE：它本身出现，SUBE 原有的 BOX 不变。已有条目时同样挪 EQUI，3 个后代条目逐字不变：两个 ZONE 位姿相同，框不变，`MAPINS` 不重写。

### `spamap.descendants.pos`

- 状态：坐实
- 例程：`descases/VDESPT` @ `0x101F33A9` n 48–63（POS 的 DCHC 为 3）；`mapboxlib/MPBOX` @ `0x102FB475` n 95–120
- 规则：元素的 POS 改动时，直接成员与第 2 层及更深的几何后代都重算（下探界限同上）。
- E3D_Session 核对：命令见第 24 条；一致数 / 总数 = 7 / 7。无条目：EQUI 的 POS E 5000 → 5100，第 1 层 2 个、第 2 层 1 个条目出现，EQUI 外的 BOX 不变；SUBE 的 POS 改动，只有它下面的 BOX 出现，同一 EQUI 下 SUBE 之外的 BOX、CYLI 不变。有条目（准备段先给 4 个基本体登记）：EQUI 的 POS 改动，3 个后代条目都改写（x 的 min / max 各 +1000，即 +100 mm），EQUI 外的 BOX 逐字不变。

### `spamap.descendants.ori`

- 状态：坐实
- 例程：同 `spamap.descendants.pos`（ORI 的 DCHC 为 3）
- 规则：元素的 ORI 改动时，直接成员与第 2 层及更深的几何后代都重算（下探界限同上）。
- E3D_Session 核对：命令见第 24 条；一致数 / 总数 = 6 / 6。EQUI 的 ORI 改为 `Y is N 10 W and Z is U`：无条目时第 1 层 2 个、第 2 层 1 个条目出现，有条目时这 3 个都改写；两次 EQUI 外的 BOX 都不变。

### `spamap.delete_residue`

- 状态：坐实（出处 ②）
- 规则：删除元素的那一会话里，被删元素自身及其各层后代的条目都已从表中删去，不留残留。Core3D 的删除路径不删条目（`VDESDL` → `MPDEL`，见入口），删键的那段代码本轮未读，按定义以 E3D_Session 核对坐实。
- E3D_Session 核对：命令见第 24 条；一致数 / 总数 = 4 / 4。准备段先登记条目：删 BOX，它的条目消失；删 EQUI，第 1 层 2 个、第 2 层 1 个条目消失，EQUI 外的 BOX 不变。

### 需求 5 第 9 条：删除没有条目的元素

E3D 删除 e3d-io 写的无条目 BOX 后 `SAVEWORK` 不报错（会话 648）；删除 owner——内含无条目 BOX、CYLI 与 SUBE→BOX 的 EQUI——后 `SAVEWORK` 同样不报错（会话 648）。两次都没有错误号。

### 规格外的发现

1. **owner 的其他重算属性同样让整棵子树重算。** EQUI / SUBE / ZONE 模板里 DCHC 为 4 的还有 NUMB、DESP、ISPE、STMF、SPRE。探针：改 EQUI 的 NUMB，第 1 层 2 个、第 2 层 1 个无条目后代的条目都出现；改 EQUI 的 DESC（DCHC 0）则都不出现。按 ①，`REORDER` 一个容器也会入队容器本身、再走它的子树（未探）。需求 7 第 4 条与 Recompute_Set 的 ④⑤ 只收换 owner 与 POS / ORI 改动下的后代。后代已有条目时这一差别在盘上看不出（框不变，`MAPINS` 不重写）；但后代没有条目时（如 0.4.0 写的 BOX），E3D 改祖先的 NUMB 会给它补上条目。是否扩 Recompute_Set，待定。
2. **E3D 不在 ZONE 下给 BOX 排序或挪入。** `REORDER` / `INCLUDE` 经 `DINSER` 插入，会检查层级：BOX 在 ZONE 下时报 `Database error 4 from DINSER - Element type not a legal member of current element`，随后 TTY 退出。e3d-io 的门把 BOX 直接写在 ZONE 下，E3D 照读、照改属性，只是不能在那里排序或挪入；本文的排序与挪入探针都放在 EQUI / SUBE 下。
3. **本机 E3D 不再响应 wrapper 的 QUIT**（D 计划 §6 D1），探针在 L3-DONE 后 20 s 停掉 des.exe。所有比对都用 SAVEWORK 写出的会话，不受影响。

## 复现命令

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x101F33A9
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x10220F82
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FB475
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x101F2989
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x101F2755
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x101F1B24
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102208C7
powershell -NoProfile -File tools/ida/pseudo.ps1 0x591E5B0
powershell -NoProfile -File tools/ida/pseudo.ps1 0x58D5290
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x101F33A9 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x10220F82 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FB475 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x101F2989 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x101F2755 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x101F1B24 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x591E5B0
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x58D5290
cargo run --manifest-path ../e3d-attlib/Cargo.toml --release --example wnoevt -- schema/e3d31/binaries/attlib.dat
cargo test --release --lib core_rules
cargo test --release --test spamap_recompute_real -- --nocapture
powershell -NoProfile -ExecutionPolicy Bypass -File tools/e3d/spamap_probe.ps1 -SelfCheck
powershell -NoProfile -ExecutionPolicy Bypass -File tools/e3d/spamap_probe.ps1 tools/e3d/spamap_probes/d2
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/ib.ps1 env
```

第 20 条列出 WNOEVT 的 7 个属性；第 21 条测 `recompute_row` / `member_order_row` / `descendants_included` / `mpbox_walks_below`；第 22 条打印每个 Geometric_Noun 的重算表并与字典逐属性核对。第 23–24 条要本机 E3D 3.1 与 gen-model 的 `l3_suite`：每个探针约 27 s，逐个打印各 watch refno 的判定（rewritten / same / appeared / vanished / none）与整表比对，结果写门用副本下的 `probe\<探针名>\result.json`；`spamap_probes/d2` 下 35 个探针的 `-- expect:` 都满足。

## 反例

无。
