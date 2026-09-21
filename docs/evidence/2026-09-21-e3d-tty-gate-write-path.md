# E3D 3.1 读回 e3d-io 写的会话——TTY 无人值守人工门（Q8 / Q8b / Q8c / Q8d）

日期：2026-09-21。方法：`tools/e3d/write_gate.ps1`（本仓库），底下是 gen-model 的 `l3_suite --check-driver`（`des.exe` TTY 会话，登录 `SYSTEM/XXXXXX`，MDB `/ALL`，项目 AMS 经 `--project-dir` 指向样本项目的**副本**）。真样本项目未动。

## 结论

**全部 PASS。** 真 E3D 3.1 Design（`E:\reverse\e3d\shadow_e3d31_gen_model_test\des.exe`）在 TTY 里读下了 e3d-io 追加的四个会话里的每一件东西：

| 问 | 会话 | 元素 | E3D 打出来的 |
|---|---|---|---|
| Q8 BOX | 641 | BOX `24384/31483` | `Position E 1000mm N 2000mm U 3000mm` / `Xlength 300mm` / `Ylength 400mm` / `Zlength 500mm` / `Owner /1RX03-MSUP` |
| Q8b 引用成员 | 642 | GROU `24384/31485` | `Members 1 ZONE 1 /1RX03-MSUP 2 BOX 1 =24384/31483`；GPWL `24384/31484` owner `/*`（WORL），成员 GROU；BOX 的 owner 仍是 ZONE |
| Q8c hunk 22（文本） | 643 | EQUI `24384/31486`、`24384/31487` | `Description e3d-io hunk gate EQUI n: DESC stored out of the record (type 22) @065 … @115`（120 字符逐字） |
| Q8c hunk 21（real 数组） | 643 | 同上 | `Desparam 1001.3 1002.3 … 1100.3`（100 个，EQUI 2 的 `2001.3 … 2100.3` 的链**跨两页**） |
| Q8c hunk 20（int 数组） | 643 | 同上 | `Uuidia 11 12 13 14` / `Uuidia 21 22 23 24` |
| Q8d 类型 19 inline | 644 | FIXING `24384/31488` | `Rokeys 1 K1 2 K2` |
| Q8d hunk 23（串列表） | 644 | 同上 | `Rovals 1 V1 2 V2` |
| 成员表 | 641–644 | ZONE `24384/47` | 128 个成员，末四个 `BOX 1 =24384/31483` / `EQUI 1 =24384/31486` / `EQUI 2 =24384/31487` / `FIXING 1 =24384/31488` |
| 会话页 | 644 | — | `Q SESSION` → `Last saved session number 644 / Session 644: Date: 10:30 21 Sep 2026 User: e3dio Description: e3d-io write-path gate: FIXING …` |

`Q ATT` 走遍 EQUI 的 108 个属性、FIXING 的 40 个属性都没报错（hunk 目录项夹在 explicit 流里不碍事）。

另一个更硬的旁证（第一轮，`RATM` 版）：E3D 的 TTY 会话退出时自己在我们的 644 之上**追加了会话 645**（`Saving Meta Data.`，user dpc，reason 15，`previous_page` = 我们 644 的会话页、INDEX 根是对我们索引页的 CoW）；随后 e3d-io 又以 645 为 base 写了 646，E3D 再读 646 一切照旧。即两个写者的会话链可以互相叠着走。

## 三个「不是 hunk 的问题」

1. **`RATM` / `UNIPAR`**：模板里有（类型 7、cnt 500/100、无 implicit 槽），E3D 却答 `(2,201) Element … does not have attribute RATM`——不在 E3D 字典的可查属性表里，`Q ATT` 也不列。换成 `UUIDIA`（类型 7、cnt 4）就打出来了。
2. **`USRWEI`**：inline、hunk 5 个、hunk 500 个三种存法 E3D 一律 `Usrweight unset`，PML `!!ce.usrwei` 报 `(2,425) EXPRLIB inconsistent operation stack`——属性本身在 E3D 侧另有来源，与存储形态无关。
3. **`DESC` 480 字符**：模板 `logical_count 480 / storage_words 121`，但 E3D `Q DESC` 报 `(2,170) Attribute DESC too long. Are you using the schema file it was created with?`；65 与 120 字符都正常。480 是 UTF-8 库里给 120 字符留的**字节**容量，E3D 字典的 DESC 上限是 120 字符。写侧若按模板 cnt 顶格写文本，E3D 会拒读——这是调用方要知道的一条口径（模板 cnt 对文本 = 字节容量，不是字符上限）。

## 怎么跑

```powershell
# 在 vendor/e3d-io-write 下；需要 gen-model 的 l3_suite（默认 D:\Rust\target\debug\l3_suite.exe）
powershell -ExecutionPolicy Bypass -File tools\e3d\write_gate.ps1
```

脚本：把 `AvevaMarineSample` 复制到 `D:\e3dio-gate\`（去掉 amspic/amsdrg，3.2 GB，robocopy 3 s）→ 每次从原库重置副本里的 `ams8000_0001` → `cargo run --bin e3d-write-box -- <副本库> --group --hunk` → 把 `tools/e3d/write_gate.mac.template` 里的 refno 与日志目录填成实跑值 → `l3_suite --check-driver <宏> --project-dir <副本> --output …`（`L3_ALLOW_EXISTING_E3D_SESSION=1`：本机常年挂着别的 des.exe，driver 只等自己起的那一个）→ 逐问 grep 日志判 PASS/FAIL。全程约 10 s。

宏的写法（踩过的坑，来自 gen-model `scripts/e3d/*.mac` 的注释）：`ALPHA LOG … OVER` 到 `ALPHA LOG END` 之间任一语句未处理的报错会让整段日志**不落盘**，所以一问一段日志、每个导航与每个可能报错的 `Q` 后面都跟 `handle any`（`$P … $!!error.text` 把错误码打进日志——`RATM` 那次就是这样拿到 `(2,201)` 的）。宏不含 `QUIT`/`SAVEWORK`（driver 的 wrapper 负责 `$M` 与 `QUIT`；含 `SAVEWORK` 的宏 driver 要求 `--target-db-file`）。

## 日志原文（`D:\e3dio-gate\logs\`，2026-09-21 10:30 那一轮）

```text
----- 10_box.log -----
BOX 1 of ZONE /1RX03-MSUP
Ref =24384/31483
Type BOX
Owner /1RX03-MSUP
Position E 1000mm N 2000mm U 3000mm
Xlength 300mm
Ylength 400mm
Zlength 500mm
----- 20_grou.log -----
GROUP 1 of GPWLD 1 of WORLD /*
Ref =24384/31485
Type GROU
Owner GPWLD 1 of WORLD /*
Members
 1 ZONE 1 /1RX03-MSUP
 2 BOX 1 =24384/31483
GPWLD 1 of WORLD /*
Ref =24384/31484
Type GPWL
Owner /*
Members
 1 GROU 1 =24384/31485
----- 31_equi1_desc.log -----
Ref =24384/31486
Description e3d-io hunk gate EQUI 1: DESC stored out of the record (type 22) @065 @070 @075 @080 @085 @090 @095 @100 @105 @110 @115
----- 33_equi1_uuidia.log -----
Ref =24384/31486
Uuidia 11 12 13 14
----- 37_equi2_desp.log -----
Ref =24384/31487
Desparam 2001.3 2002.3 2003.3 2004.3 2005.3
2006.3 2007.3 2008.3 2009.3 2010.3
…
2096.3 2097.3 2098.3 2099.3 2100.3
----- 41_fixing_rokeys.log -----
Ref =24384/31488
Rokeys
 1 K1
 2 K2
----- 42_fixing_rovals.log -----
Ref =24384/31488
Rovals
 1 V1
 2 V2
----- 50_zone.log (tail) -----
125 BOX 1 =24384/31483
126 EQUI 1 =24384/31486
127 EQUI 2 =24384/31487
128 FIXING 1 =24384/31488
----- 70_sessions.log -----
Dbname MASTER/DESIN
Db MASTER/DESIN, number 8000
Last saved session number 644
Session 644:
 Date: 10:30 21 Sep 2026 User: e3dio Description: e3d-io write-path gate: FIXING with ROKEYS inline (type 19) and ROVALS as a hunk (type 23)
```

第一轮（`RATM` 版、480 字符 DESC 版）里出的三条报错原文：

```text
GATE-33-RATM-ERROR (2,201)   Element EQUIPMENT 1 of ZONE /1RX03-MSUP does not have attribute RATM
GATE-84-UNIPAR-ERROR (2,201)   Element EQUIPMENT 6 of ZONE /1RX03-MSUP does not have attribute UNIPAR
GATE-31-DESC-ERROR (2,170)   Attribute DESC too long. Are you using the schema file it was created with?
GATE-85-A-PML-ERROR (2,425)   EXPRLIB inconsistent operation stack (element EQUIPMENT 2 of ZONE /1RX03-MSUP). object id 1111, object type 2222, evaluation type ---
```

E3D 自己追加的会话（第一轮之后，`e3d-descriptor sessions --db` 读出）：

```text
sesno 645  user dpc   reason 15  comment "Saving Meta Data."  previous_page 16525 (= 我们 644 的会话页)  index_root 16542
sesno 644  user e3dio reason 0   comment "e3d-io write-path gate: FIXING …"  base_sesno 643
```
