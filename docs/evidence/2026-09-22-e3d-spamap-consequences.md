# 取证：e3d-io 写的几何元素没有 SPAMAP 条目——E3D 看不看得见（T3）

日期：2026-09-22
计划：`docs/plans/2026-09-22-write-path-phase-2-plan.md` T3 / Q3
环境：真 E3D 3.1 Design TTY（gen-model `l3_suite --check-driver`，项目副本 `D:\e3dio-gate\AvevaMarineSample`，`ams8000_0001` 每轮从原件重置）；SPAMAP 表用一支 `%TEMP%` 探针按页走读（页头 7 字 `[5][hash 230902453][level][key 4][value 7][…][free]`，叶条目 4 键字 + 7 值字，内结点 4 键字 + `(pgno, ext)`），未入库。

## 0. 结论

1. **E3D 给每一个几何图元都维护 SPAMAP 条目**：`ams8000_0001` 3 746 条 / 3 628 个元素，BOX 45/45、CYLI 34/34、FTUB 1 559/1 559、BEND 628/628、SCTN 528/528、ATTA / PNOD / ELBO / EXTR / ELCONN / FITT / STRT / PANE / REDU / VALV / WELD 全部 100 %；EQUI / ZONE / SITE / GROU / FIXING 一条都没有（不是几何）。e3d-io 写的 BOX 是全库**唯一**没有条目的 BOX（46 个里的第 46 个）。
2. **E3D 的空间搜索只看 SPAMAP**：`EVALUATE … FOR ALL BOX WITHIN VOLUME <elem> [clearance]` 以原生 BOX 为中心、100 m 余量列出全 MDB 12 190 个 BOX，**我们的 BOX 不在里面**；以我们的 ZONE 为体积列 911 行，**它自己的子元素不在里面**；层级遍历 `FOR ALL BOX FOR CE` 才找得到它。
3. **E3D 算不出我们元素的体积**：`WITHIN VOLUME =24384/26484 100` → `(2,536) Error getting volume for BOX 1 of ZONE /1RX03-MSUP`——元素的世界系包围盒是从 SPAMAP 读的，没有条目就报错。凡是要「这个元素占多大地方」的功能（体积选择、clash、limits / 自动取景一类）对我们写的元素都会走到这一步。
4. **E3D 不会顺手补**：E3D 改一个无关元素并 `SAVEWORK`（会话 643）后，SPAMAP 仍是 244 页 / 3 746 条，我们的 BOX 仍 0 条。
5. **E3D 自己改过这个元素才补**：E3D 把我们 BOX 的 `XLEN` 300 → 301 并 `SAVEWORK`（会话 648）后，SPAMAP 3 747 条、BOX 46/46，我们的 BOX 得到条目 `key [300008495, 24384, 26484, 0]`、值 `[8495, 18000, 27500, 11505, 22000, 32500, 2]`；同一会话 `WITHIN VOLUME =24384/26484 100` 返回 `=24384/26484`。
6. **值的单位是 0.1 mm，不是 mm**（修正写计划 §7 2026-09-20 调研条的说法）：我们的 BOX 在 E 1000 N 2000 U 3000、301 × 400 × 500 → 世界系 AABB x ∈ [849.5, 1150.5] mm，盘上 `8495 … 11505`；原生 BOX `24384/24777`（W 2821 N 8004 U 1400，312 × 590 × 550，绕 Z 转 5°、Z 朝下）中心 x = −2821.3 mm ↔ `(−30024 − 26402) / 2 = −28213`，半宽 181.1 mm ↔ 1 811。
7. **键的第一个字不是「空间格 hash」，是从 AABB 算的**：`key0 = N × 10^8 + AABB 最短轴的 min 坐标`（0.1 mm）。E3D 存盘后的全表 **3 747 条全部成立**（`key0 − min[最短轴]` 恰为 10^8 的整数倍 3 747 / 3 747，0 例外；538 条最短轴并列时取其一亦成立）。N 按轴分段：x 轴 1–4（185 / 250 / 49 / 3）、y 轴 7–10（335 / 261 / 38 / 3）、z 轴 13–17（1 250 / 1 321 / 43 / 7 / 1），即 `N = 6 × 轴序（x 0 / y 1 / z 2）+ 桶（1–5）`。例：我们的 BOX（301 × 400 × 500，最短 x，minx 8495）→ `3 × 10^8 + 8495`；原生 BOX `24384/24777`（世界系 362 × 615 × 550，最短 x）→ `3 × 10^8 − 30024`；样本 `24384/2869`（11 712 × 8 305 × 7 370，最短 z）→ `16 × 10^8 + 20129`。**桶（1–5）按什么分未定**——候选是最短轴 extent 的量级（3 010 / 3 622 → 3，73 700 → 4）或坐标区间；flags（0 / 2 / 3 / 6 / 7，直方图 1 083 / 748 / 1 907 / 8 / 1）语义未定。两者都能从这 3 747 条离线拟合，或读 core。**（2026-10-03 更正：本条两组直方图不是同一张表——flags 一组合计 3 747，取自门用副本会话 648；N 一组合计 3 746，却与原件会话 640 有两格不同（x3 49 对 48、z1 1 250 对 1 251）。原件会话 640 同一张表上两组都是 3 746 条，以 `docs/evidence/2026-10-03-spamap-census.md` 为准。）**

→ **Q3 拍板依据**：写侧不维护 SPAMAP 的元素，在 E3D 里「导航得到、`Q ATT` 全对、空间上不存在」，且 E3D 不会自愈。按计划 §2 T3 的规则，**SPAMAP 升为第三期第一件**，另起计划；这份取证把它的成本从「读 core 的空间格 hash」降到「世界系 AABB（e3d-model 已有）+ 一个可从语料拟合的键」。

## 1. 逐步记录

| # | 做了什么 | 结果 |
|---|---|---|
| 0 | `tools/e3d/write_gate.ps1` 重跑（本机首次，robocopy 3.2 GB） | 16 问 PASS，12.6 s；副本会话 641–645（e3d-io）+ 646（E3D 开库自追） |
| 1 | 探针：SPAMAP 表全量 + 目标 refno | 244 页 / 3 746 条 / 3 628 元素；k（第 4 键字）0：3 625、1：121（同一元素两条 = 跨桶）；我们的 BOX / EQUI × 2 / FIXING 各 0 条；ZONE `24384/47` 128 个成员 0 条（都是 EQUI） |
| 2 | 探针：按 noun 统计 | 见 §0 第 1 条；三条原生 BOX 的 owner 链都是 `BOX ← EQUI ← ZONE ← SITE ← WORL` |
| 3 | TTY：`Q POS WRT /*` | 我们的 BOX E 1000 N 2000 U 3000、`Y is N and Z is U`；ZONE `24384/47` 在原点无旋转；原生 BOX `24384/24777` W 2821 N 8004 U 1400、`Y is N 5 W and Z is D`、310 × 590 × 550 |
| 4 | TTY：`VAR !a COLLECT ALL BOX …`、`!!collectallfor(…)`、`object ARRAY()` / `.size()` / `do values` | **E3D 进程 0xC0000005**——TTY 驱动环境下任何 PML2 对象 / `COLLECT` 都崩，与库内容无关（pristine 库同崩）。可用的是 PML1：`VAR !n EVALUATE NAME FOR ALL BOX [FOR CE \| WITHIN VOLUME <elem> [n]]` + `Q VAR !n`。`WITHIN VOLUME <pos> TO <pos>` 在此语法错 `(47,15)` |
| 5 | TTY（只写了 BOX 的副本，会话 641）：`WITHIN VOLUME =24384/26484 100` / `… 100000` | `(2,536) Error getting volume for BOX 1 of ZONE /1RX03-MSUP` |
| 6 | 同上：`WITHIN VOLUME =24384/24777 100000`（原生 BOX，100 m） | 12 190 个 BOX，**无 26484** |
| 7 | 同上：`WITHIN VOLUME =24384/47`（我们的 ZONE） | 911 行，**无 26484** |
| 8 | 同上：`=24384/47` → `EVALUATE NAME FOR ALL BOX FOR CE` | `=24384/26484`（层级遍历看得见） |
| 9 | TTY 有状态（`--target-db-file … --aios-project AMS`）：E3D 给无关 ZONE `24384/24775` 设 `DESC` 并 `SAVEWORK` | 会话 643 提交；SPAMAP **不变**（244 页 / 3 746 条），我们的 BOX 仍 0 条 |
| 10 | TTY 有状态：E3D 给 BOX 设 `DESC`（我们的、原生的各试一次） | 两次都 0xC0000005——TTY 环境对 BOX 设 `DESC` 就崩，与是谁写的无关；给原生 EQUI 设 `DESC` + `SAVEWORK` 正常（会话 645） |
| 11 | TTY 有状态：E3D 给我们的 BOX `XLEN 301`、原生 BOX `XLEN 312`，`SAVEWORK` | 会话 648；SPAMAP 3 747 条、BOX 46/46；我们的 BOX 得条目（§0 第 5 条）；原生 BOX 条目重算（`299969986 → 299969976`，AABB 随之）；`WITHIN VOLUME =24384/26484 100` 现在返回它自己 |
| 12 | 探针：`key0 − 坐标` 拟合 | §0 第 7 条 |

## 2. 没做 / 留待

- **3D 视图能否画出无 SPAMAP 的元素**：TTY 不出图，本轮未验；从 §0 第 3 条推断，画（走几何）大概率不受影响、「取景到元素 / 体积选择 / clash」受影响。需一次 E3D 图形会话人工确认（`ADD =24384/26484` → 看得见否；`LIMITS CE` / 自动取景 → 报不报 (2,536)）。
- **粗桶边界与 flags 语义**：3 746 条语料在手，可离线拟合；写侧要落 SPAMAP 时再定，或读 core 的 `sub_5AB91B0` 类型 14 路径上游算键的例程。
- TTY 驱动的两条坑记 `tools/e3d/` 用法：只用 PML1（`VAR … EVALUATE … ` + `Q VAR`），不要 PML2 对象；别对图元设它没有的属性（`DESC` on BOX）。
