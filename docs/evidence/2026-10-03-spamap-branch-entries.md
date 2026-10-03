# 取证：SPAMAP 管道分支的条目——MPBXBR、条目数与 k、覆盖补充

日期：2026-10-03
任务：write-path-phase-3 7.1（`spamap.entries_k`）、7.2（`spamap.coverage` 补充）
规则：spamap.entries_k, spamap.coverage

> 按 `.kiro/specs/write-path-phase-3/requirements.md` 需求 1 第 7–9 条、需求 4 第 7、14、15 条与需求 6 第 15 条写；交稿前跑
> `powershell -NoProfile -File tools/ida/check_evidence.ps1 docs/evidence/2026-10-03-spamap-branch-entries.md`。
> 接 `docs/evidence/2026-10-03-spamap-encoding.md`（`spamap.entries_k` 当时未坐实）与 `docs/evidence/2026-10-03-spamap-coverage.md`（BRAN 与 `HNGT` noun 当时因 `MPBXBR` 未读而不说）。

## 环境

本轮首条与末条调用对两个 IDB 各跑一次 `ib.ps1 env`。

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |
| Core3D_IDB | `D:\ida_scratch\plant3\Core3D.dll.i64`（IDB 记录的输入文件即门用 E3D 的 `Core3D.dll`，哈希见 spec 术语表 Core3D_IDB） |
| Core3D_IDB SHA-256（首条调用前） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| IDA 版本 | 9.2（两个实例相同） |
| HEAD | `732d2a4ec257e2fc0cf09b87377934d508ab78c1` |
| Core3D_IDB SHA-256（末条调用后） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| Core_IDB SHA-256（末条调用后） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |

交叉引用核对（复现命令第 9–15 条），全部 PASS：Core3D_IDB `0x102FC293` 2 条、`0x102FD8C2` 12 条、`0x1038F5E4` 2 条、`0x102FD2BC` 1 条、`0x102FBD62` 1 条、`0x10AE7598`（`DFIND` 导入）1 条；Core_IDB `0x6453B98`（全局错误码，被大量引用）2 478 条。

## 结论

写条目的只有 `maplib/MAPINS`，它只由 `mapboxlib/MBXGO` 调；`MBXGO` 的调用方只有 `mapboxlib/MPBOX2`（tube 恒为 0）、`mapboxlib/MPBXBR` 与后者调的 `sub_102FD2BC`。`MPBXBR` 只处理 BRAN 与 `HNGT` 为真的 noun（`MPBOX` 与 `maplist/VDEMPR` 都按 noun 交给它）：从尾到头走成员，给每个成员记自身的框（k = 0）与离开它、朝尾方向的那段管子（k = 1），最后给 BRAN 自身只记头管（k = 1）。管子的框由成员的两端点（目录 P 点）与管子规格的管径算出，e3d-io 不读目录。由此 `spamap.entries_k` 坐实为 `rules::entries_by_mpbox`：经 `MPBOX2` 记的元素恰一条 k = 0、用自身的框；BRAN、`HNGT` noun 的元素及其成员为需求 6 第 15 条的 MissingInput，写侧按 D7 回退。`src/core_rules.rs` 改为 Confirmed，Codec_Check 把 MissingInput 记入 `waiting`。坐实集下 Property 5 的条目数与 k 一项（复现命令第 16、17 条）：Corpus 各库最新会话判定的 11 167 个元素全部相符、0 反例，`waiting` 14 350 个全是分支与吊架树上的元素；基线会话 640 判定 986 / 986。`spamap.coverage` 随之补一条：BRAN 与 `HNGT` noun 的成员不看 IMAP、一律为是；它们自身仍不说，理由改为头管要目录几何。

### `spamap.entries_k`

- 状态：坐实（分支与吊架树上为 MissingInput）
- 谁写条目：调用方查询（复现命令第 8 条）：`maplib/MAPINS` @ `0x1038F5E4` 只有 `mapboxlib/MBXGO` @ `0x102FD8C2` 的两处调用（@ `0x102FD937`、@ `0x102FD988`）；`MBXGO` 的调用方是 `mapboxlib/MPBOX2` @ `0x102FBD62`（@ `0x102FC264`）、`mapboxlib/MPBXBR` @ `0x102FC293`（十处）与 `sub_102FD2BC` @ `0x102FD2BC`（@ `0x102FD82A`；它唯一的调用方是 `MPBXBR`）。`MBXGO` n = 12–29：第四参 tube 交 `MAPINS` 定下标（`MAPINS` n = 81–84：tube 为真取 2，否则取 1，k = 下标 − 1，见 encoding 文档），第五参为假时改调 `MAPREM` 删该下标的条目，第六参定 flags 是否启用。
- `MPBOX2` 路径：`MPBOX2` n = 93 以 tube = 0（@ `0x10B2FDCC` = 0）调 `MBXGO`，框就是元素自己的框：一条，k = 0。
- 谁进 `MPBXBR`：`MPBOX` n = 98–103 对 noun 为 BRAN（808220）或 `HTCOMP` 为真的元素调它，第三参 @ `0x10B2FD80` = 0；`maplist/VDEMPR` @ `0x102208C7` n = 44–49 对队里的这类项调它，第三参为队列。
- `MPBXBR` 的走法：n = 68–77 先读尾——TDIR（@ `0x10B2FE1C` = 892424）、TPOS（@ `0x10B2FE20` = 916805）、TBOR（@ `0x10B2FE24` = 896744）、LTAI（@ `0x10B2FE28` = 709869）；n = 79–82 `DAFTER(0)` 后 `DFIND(PREV, 1)`（@ `0x10B2FE2C` = 968614 即 PREV，@ `0x10B2FE30` = 1），从最后一个成员往前走。n = 92–96：第三参为 0 时每个成员都记（v18 = −1）；为队列时由 `sub_102FE99B` 按队判定（未读）。n = 116–131：`sub_102FD2BC` 给出成员的框、两端点与 flags（未逐行读）；n = 132–136：框取不到时以 `MBXPOI` 取元素原点作点框。n = 137–138：`MBXGO(框, flags, tube = 0, 有效 = −1)`（@ `0x10B2FE38` = 0、@ `0x10B2FE3C` = −1）——成员的 k = 0 条目，框有效与否都写。
- 成员的 k = 1：n = 144–209：成员算管件（不是 ATTA，或 ATTA 经 `SPKBRK` 判为断点，n = 85–91），两端点都取到，本端到上一个点（起初是尾）的距离平方不小于 1 mm²，`sub_1025B49F` 判连通（未读），再按管子规格（BRAN、TRUNNI、LUG 读 LSTU，@ `0x10B2FE40` = 959889；其余 `HTCOMP` noun 读 LSRO，@ `0x10B2FE44` = 840333）经 `sub_102FD9D7` 取类型、管径与 flags（未读）：类型为 TUBE（631901）时 `sub_102FECE3` 以半径 = 管径 / 2 取圆管框，否则 `sub_102FE6A2` 取框；然后 `MBXGO(管框, flags, tube = −1, 有效 = −1)` 写 k = 1。任一条件不满足时 n = 206–215 以「有效 = 0」删掉该成员的 k = 1。n = 216–232：下一段从本成员的另一端起算。
- BRAN 自身：n = 242–317 走完成员后，当前元素回到 BRAN，读头——HDIR（@ `0x10B2FE48` = 892412）、HBOR（@ `0x10B2FE4C` = 896732）、HPOS（@ `0x10B2FE50` = 916793）、LHEA（@ `0x10B2FE54` = 554997），管子规格 HSTU（@ `0x10B2FE58` = 959885）或 HSRO（@ `0x10B2FE5C` = 840329）——同样的条件下写头管为 BRAN 的 k = 1，否则删掉。BRAN 没有 k = 0。
- 须用到的输入：成员两端点与成员框来自 `sub_102FD2BC`（目录 P 点与几何），管径来自管子规格所指的目录。e3d-io 不读目录，按需求 6 第 15 条为 MissingInput；写侧对这些元素按 D7 回退（`MissingAabb`）。
- 实现：`src/spatial/rules.rs` 的 `entries_by_mpbox`——元素的 noun、或其 owner 的 noun 使 `records_its_members` 为真（BRAN 与 `rules::HNGT`）时返回 MissingInput，否则一条 k = 0、用元素自己的框。`src/spatial/check.rs` 把 MissingInput 与待坐实规则一样记入 `waiting`（设计 §SpatialStage 把二者统一成 `MissingAabb`）。
- Corpus 核对：坐实集 `spamap-check` 的 `entries_k` 项（第 16 条）：`ams8000_0001` 会话 646 986 / 3 628（`waiting` 2 642）、`ams7999_0001` 会话 200 9 549 / 21 257（`waiting` 11 708）、`ams251142_0001` 会话 10 327 / 327、`TES023` 会话 109 305 / 305；判定的 11 167 个全部相符、0 反例。基线会话 640 986 / 3 628（`waiting` 2 642，第 17 条）。Property 5 在坐实集下逐库断言 `agree + waiting = total`（第 18 条）通过，即 Corpus 中带 k = 1 条目的元素全在分支与吊架树上。旧候选「单条 k = 0」缺的恰是这些元素（encoding 文档「反例」）。

### `spamap.coverage`（补充）

- 成员：`MPBXBR` 第三参为 0 时 n = 92–96、137–138 对每个成员都写 k = 0，不读成员的 IMAP；所以 BRAN 与 `HNGT` noun 的成员为是（设计模板里没有 SPAMAP 的八个 noun 仍不说，见 coverage 文档的 `BTSELE` 一条）。`rules::coverage_by_imap` 在读 IMAP 之前先判 owner。
- BRAN 与 `HNGT` noun 自身：只在头管成立时有条目（n = 242–317：首成员的一端到 HPOS 不小于 1 mm、连通、头管规格取到），要目录几何，仍不说。Corpus 中 BRAN 有条目的 `ams8000_0001` 3 / 563、`ams7999_0001` 219 / 1 256，与「只有部分分支有头管」相符。
- Corpus 核对：Property 7 与 coverage 文档同数（第 18 条；成员原本都按 IMAP 1 判是、全部相符）。

### `spamap.flags`（读到的，仍未坐实）

- 分支与吊架树：成员 k = 0 的 flags 来自 `sub_102FD2BC`，k = 1 的来自管子规格（`sub_102FD9D7`），都要目录，坐实后也是 MissingInput。`MPBOX2` 路径的 flags（`BOXELE` 按 OBST 置位、IMAP 5 并入 IMAP 3 成员的 OBST、`MPBOX` 上溯定是否启用）留到下一步。

## 复现命令

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FC293
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FD8C2
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102208C7 34 50
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FB475 96 103
powershell -NoProfile -File tools/ida/sql.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 "SELECT printf('0x%X', a) AS ea, read_i32(a) AS v FROM (SELECT 0x10B2FD80 AS a UNION ALL SELECT 0x10B2FE1C UNION ALL SELECT 0x10B2FE20 UNION ALL SELECT 0x10B2FE24 UNION ALL SELECT 0x10B2FE28 UNION ALL SELECT 0x10B2FE2C UNION ALL SELECT 0x10B2FE30 UNION ALL SELECT 0x10B2FE38 UNION ALL SELECT 0x10B2FE3C UNION ALL SELECT 0x10B2FE40 UNION ALL SELECT 0x10B2FE44 UNION ALL SELECT 0x10B2FE48 UNION ALL SELECT 0x10B2FE4C UNION ALL SELECT 0x10B2FE50 UNION ALL SELECT 0x10B2FE54 UNION ALL SELECT 0x10B2FE58 UNION ALL SELECT 0x10B2FE5C UNION ALL SELECT 0x10B2FDCC)"
powershell -NoProfile -File tools/ida/sql.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 "SELECT printf('0x%X', x.to_ea) AS target, printf('0x%X', x.from_ea) AS site, f.name AS caller FROM xrefs x JOIN funcs f ON x.from_ea >= f.start_ea AND x.from_ea < f.end_ea WHERE x.to_ea IN (0x102FD8C2, 0x1038F5E4, 0x102FC293, 0x102FD2BC) AND (x.type_name IS NULL OR x.type_name <> 'flow') ORDER BY x.to_ea, x.from_ea"
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FC293 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FD8C2 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x1038F5E4 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FD2BC --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FBD62 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x10AE7598 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x6453B98
cargo run --release --quiet --bin e3d-descriptor -- spamap-check --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001 --input D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001=646 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001=200 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001=10 --session D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023=109 --schema schema/e3d31 --rules confirmed
cargo run --release --quiet --bin e3d-descriptor -- spamap-check --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001=640 --schema schema/e3d31 --rules confirmed
cargo test --release --test spamap_codec_real -- --nocapture --test-threads 1
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/ib.ps1 env
```

第 16、17 条输出 JSON，各库 `entries_k` 的 `agree` / `total` / `waiting` 即上文数字，`waiting` 的 miss 理由为 `missing input: …`；第 18 条跑 Property 5（坐实集下逐库 `agree + waiting = total`）与 Property 7。

## 反例

`spamap.entries_k` 判定的元素：无（Corpus 11 167 / 11 167，基线会话 640 986 / 986）。`spamap.coverage`：无，与 coverage 文档同数。

MissingInput 的 14 350 个元素（Corpus）与 2 642 个（基线）记 `waiting`，不是反例。
