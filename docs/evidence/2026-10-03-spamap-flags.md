# 取证：SPAMAP 条目的 Spatial_Flags

日期：2026-10-03
任务：write-path-phase-3 7.1（`spamap.flags`）
规则：spamap.flags

> 按 `.kiro/specs/write-path-phase-3/requirements.md` 需求 1 第 7–9 条与需求 4 第 8、9、11 条写；交稿前跑
> `powershell -NoProfile -File tools/ida/check_evidence.ps1 docs/evidence/2026-10-03-spamap-flags.md`。
> 接 `docs/evidence/2026-10-03-spamap-encoding.md`（当时未坐实）与 `docs/evidence/2026-10-03-spamap-branch-entries.md`（分支树的条目）。

## 环境

本轮首条与末条调用对两个 IDB 各跑一次 `ib.ps1 env`。

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |
| Core3D_IDB | `D:\ida_scratch\plant3\Core3D.dll.i64`（IDB 记录的输入文件即门用 E3D 的 `Core3D.dll`，哈希见 spec 术语表 Core3D_IDB） |
| Core3D_IDB SHA-256（首条调用前） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| IDA 版本 | 9.2（两个实例相同） |
| HEAD | `7e8e98b096a44ca2a18d8fce7e95a980e377b5fd` |
| Core3D_IDB SHA-256（末条调用后） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| Core_IDB SHA-256（末条调用后） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |

交叉引用核对（复现命令第 10–15 条），全部 PASS：Core3D_IDB `0x102FB475` 2 条、`0x1036A31C` 13 条、`0x102FBD62` 1 条、`0x102FD8C2` 12 条、`0x108E7A70` 2 条；Core_IDB `0x6453B98`（全局错误码，被大量引用）2 478 条。

## 结论

flags 是 `maplib/MAPINS` 把三个逻辑量按 1、2、4 相加的结果；三个量由 `mapboxlib/MPBOX2` 交给 `mapboxlib/MBXGO`，后者在 `MPBOX` 判定「不启用」时改用全 0。`MPBOX2` 路径上第 n 个量为真，当且仅当元素（带 WBOX 时 OBST 1–3，否则 OBST 1–2）或 IMAP 5 元素的某个 IMAP 3 成员的 OBST 等于 n。`dbboxlib/BOXELE` 按 box type 分派时，只有 box type 1、18、19、23 会把这三个量交给目录几何或自己置位，其余 box type 不动。`spamap.flags` 坐实，实现为 `rules::flags_by_mpbox`，`src/core_rules.rs` 改为 Confirmed。分支与吊架树上的元素、目录型 box type 的元素与 UDET 不说。坐实集下 Property 5 的 flags 一项（复现命令第 16–18 条）：Corpus 各库最新会话判定的 4 783 条全部相符、0 反例，`waiting` 23 486 条；基线会话 640 判定 402 / 402。至此编码规则（7.1）全部坐实。

### `spamap.flags`

- 状态：坐实（分支与吊架树、目录型 box type、UDET 不说）
- 位：`maplib/MAPINS` @ `0x1038F5E4` n = 52–58：flags = 第一量 1 + 第二量 2 + 第三量 4。
- 是否启用：`mapboxlib/MPBOX` @ `0x102FB475` n = 31–62 从元素自身起沿 owner 上溯：DDEP（字段字 @ `0x10B2FD68` = 850126）为 1 且 noun 不是 TPWL、APPLDW、TRNWLD、FEMWLD（784856、332930359、64767782、64766688）即启用；noun 为 SITE、BLOCK、SSOWLD、HBLWLD（644698、6447602、64768537、64765880）即启用；到 WORL（781187）或取不到 owner 即不启用。`mapboxlib/MBXGO` @ `0x102FD8C2` n = 12–24：不启用时以全 0 调 `MAPINS`。
- 三个量：`mapboxlib/MPBOX2` @ `0x102FBD62` n = 30 先清零；元素带 WBOX（字段字 @ `0x10B2FDB4` = 1014845）时 n = 84–86 以 OBST（@ `0x10B2FDB8` = 939021）1–3 置第 OBST 个量；否则 n = 36 调 `dbboxlib/BOXELE` @ `0x1036A31C`，后者 n = 85–88 以 OBST（@ `0x10B68EEC` = 939021）1–2 置第 OBST 个量，再按 box type 分派。元素 IMAP 为 5 且自身框取到时，`MPBOX2` n = 37–79 对每个 IMAP 3 成员以同一组量调 `BOXELE`，成员的 OBST 也并进来。
- box type：`DB_Model::getBoxType` @ `0x108E7A70` n = 84–311 给 noun 定 box type（1：SCTN 等 15 个；2–17、20–22：CYLI、BOX、DISH、CONE、SNOU、PYRA、CTOR、EXTR、SLCY、POHE 等基本体与点；18 为 `IPCOMP`、19 为 `IHCOMP`、24 按库引用判定；25 为 XGEOM）。`BOXELE` 的 switch 中只有 case 1（n = 100–101 调 `MBXSTL`）、case 18 与 19（n = 275–290 调 `sub_1036CC5C`）、case 23（n = 337–385，n = 356 调 `sub_1036CC5C`，n = 367 直接置第一个量）把这组量往下交或自己置位，case 2–17 与 20–22 不碰。
- 分支与吊架树：`MPBXBR` 的成员 flags 来自 `sub_102FD2BC`、管子的来自管子规格（见 branch-entries 文档），要目录，不说。
- 实现：`src/spatial/rules.rs` 的 `flags_by_mpbox`——分支树、目录型 box type（`FLAGS_BY_OBST_ALONE` 之外的 noun）、UDET、成员列不出时不说；不启用为 0；带 WBOX 时为 OBST 1–3 的位；否则为 OBST 1–2 的位，IMAP 5 时并上 IMAP 3 成员的 OBST 1–2 的位（成员属目录型 box type 时不说）。`SpatialState` 加默认方法 `members`，`ReadState` 以一次头部扫描建 owner → 成员表。`ENABLE_FLAGS`、`DDEP_DOES_NOT_ENABLE` 的 hash 由单测钉为上列八个立即数（第 19 条）。
- Corpus 核对：坐实集 `spamap-check` 的 `flags` 项（第 16 条）：`ams8000_0001` 会话 646 402 / 3 746（`waiting` 3 344）、`ams7999_0001` 会话 200 4 080 / 23 891（`waiting` 19 811）、`ams251142_0001` 会话 10 0 / 327（`waiting` 327，全是分支树与目录型）、`TES023` 会话 109 301 / 305（`waiting` 4）；判定的 4 783 条 0 反例，逐 noun 全对：BOX、CYLI、CONE、DISH、PYRA、SNOU、CTOR、RTOR、SLCY、EXTR、PANE、PNOD、DATU。基线会话 640 402 / 3 746（第 17 条）。Property 5 坐实集断言通过，planted-mismatch 测试照过（flags 不随 k 变，第 18 条）。旧候选 `flags_from_obst` 只判 BOX / CYLI 1 590 条。

## 复现命令

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FB475 31 62
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FD8C2
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FBD62
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1036A31C
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1038F5E4 50 60
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x108E7A70 84 311
powershell -NoProfile -File tools/ida/sql.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 "SELECT printf('0x%X', a) AS ea, read_i32(a) AS v FROM (SELECT 0x10B2FD68 AS a UNION ALL SELECT 0x10B68EEC UNION ALL SELECT 0x10B2FDB8 UNION ALL SELECT 0x10B2FDB4)"
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FB475 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x1036A31C --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FBD62 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FD8C2 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x108E7A70 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x6453B98
cargo run --release --quiet --bin e3d-descriptor -- spamap-check --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001 --input D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001=646 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001=200 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001=10 --session D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023=109 --schema schema/e3d31 --rules confirmed
cargo run --release --quiet --bin e3d-descriptor -- spamap-check --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001=640 --schema schema/e3d31 --rules confirmed
cargo test --release --test spamap_codec_real -- --nocapture --test-threads 1
cargo test --release --lib spatial::rules
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/ib.ps1 env
```

第 16、17 条输出 JSON，各库 `flags` 的 `agree` / `total` / `waiting` 与 `by_noun` 即上文数字；第 18 条跑 Property 5（坐实集下逐库 `agree + waiting = total`）与 planted-mismatch；第 19 条含钉住八个 noun hash 的单测。

## 反例

判定的 4 783 条（Corpus）与 402 条（基线会话 640）：无。

`waiting` 的 23 486 条（Corpus）与 3 344 条（基线）是分支与吊架树、目录型 box type（管件 18、吊架件 19、SCTN 等 1）与无 SPAMAP 规则可说的元素，不是反例。
