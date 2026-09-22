# 2026-09-22 core.dll 新元素 refno 从哪来 —— ida-bridge + 语料取证（S6 前半）

- 问题：`write::Transaction` 给新元素分配 refno 时，`word1`（序号）从哪来——core 有没有一个「下一个号」计数器，还是每次扫最大值 +1？
- 手段：ida-bridge 活桥（`core.dll` = `D:\ida_scratch\plant3\core.dll.i64`，E3D 3.1，实例 `idalib-4000`；包装见 `2026-09-20-core-dll-dabacon-write-path-algorithms.md` §7），加 4 个装机库页 1 实读。

## 结论（一句话）

**core 在页 1（桶表 / 扩展表，页类型 1）为每个「会长」的扩展各存一个「下一个 `word1`」计数器，它严格大于该扩展当前活元素的最大 `word1`（删掉的号不回收）。** 主扩展只放 `WORLD` + `MNUM` 两个引导元素、不长，所以页 1 里没有它的计数器；纯字典库（全在主扩展、建一次不长）页 1 里一个计数器都没有。新元素的 `word1` 由这个计数器发、发后自增；`word0` 是它落进的那个扩展的前缀。

对写侧的意义：忠实做法是 = 读页 1 目标扩展的计数器、发号、自增、把页 1 也 CoW 写回。但**页 1 维护（写计划 §1.3 #9 的 A11）尚未落地**，本轮 `Transaction` 先按「该扩展当前最大 `word1` + 1」发号（会话内连续），提交前撞 latest 已是 590——与写计划 §4 风险行「读不出分配器就 max+1 + 590 兜底」同口径。差别只是：计数器发的号在活元素之上留了空档（回收不了删除号），max+1 会填进最早的空档；两者都不会撞活元素，590 兜住并发写者。**页 1 计数器维护记为 S6b（下）。**

## 创建路径（IDA）

`NEW <noun>` 的 C++ 侧到 FORTRAN 侧：

```
DB_Element::createAt (0x592ace0)          // v16 = 新 noun hash, v15 = owner noun hash
  └─ ELCREL (0x5195263)                    // dbmodlib/ELCREL；LGCREA 许可检查后转 ELCREA
       └─ ELCREA (0x5195312)               // 权限 / 类型 / 父子合法性；末尾 BCREAT
            └─ BCREAT (get_name_ea BCREAT) // dbmodlib/BCREAT：在 owner 下建块
                 └─ sub_5AAFCE0 → sub_5ABD470(597, pos, noun)   // FORTRAN routine 597 = 建记录块
```

- refno **不是** `createAt` 传进去的：`createAt` 只传 owner noun hash（`v15`）、新 noun hash（`v16`）与一个位置字（`v17 = a4`），创建成功后用 `DB_Element::setFortranReturnCE(a2)` 从全局「当前元素 CE」读回新 refno。号在 `BCREAT` → routine 597 的建块路径里铸出。
- `DGETNO`（= `sub_5AC3EA0(89, …)`，routine 89）只是「取当前 db 号」，不是计数器；`BCREAT` 用它拿 db 号后 `sub_59EFD20` / `GATIN1` / `GATRF1` 走 owner 成员链、`sub_5AB0050` / `sub_5AAF8D0` 落块，错 601（桶满）时 `sub_5A06250`=CHKUSR 复核用户后重试。routine 597 的号铸出点在 FORTRAN 侧（本轮未展开到字节级——数据侧已足以定论，见下）。

## 页 1 = 桶表 / 扩展表（语料实读，4 库）

页 1 首字类型 = 1（桶页；页 0 描述子=0、会话=3、索引=5）。头三字 `[1][0xFFFFFFFF][N]`，其后是 `(扩展 word0, 下一个 word1)` 对——**只列会长的扩展**：

| 库 | 元素数 | 每扩展 max word1 | 页 1（非零头 + 对） | 计数器 vs max |
|---|---|---|---|---|
| `ams8000_0001`（DESI） | 6 639 | 16192→1, **24384→26483**, 32576→31 | `[0]=1 [1]=0xFFFFFFFF [2]=8` · `(24384, 26496)` `(32576, 98)` | 26496 > 26483（+13）；98 > 31 |
| `ams251142_0001`（DESI） | 948 | 2013279494→1, **2013287686→946** | `[2]=5` · `(2013287686, 947)` | 947 > 946（+1） |
| `TES023`（DESI） | 320 | **9227→299, 17419→315** | `[2]=5` · `(17419, 331)` | 331 > 315（+16）；9227 无对 |
| `ams5100_0001`（DICT） | 238 | 13292→238 | `[2]=2`，**无对** | 无计数器 |

读法：
- **计数器严格 > 活最大**（余量 +1..+16 = 删过的号），所以它是「历来发过的号 + 1」的高水位，不是「活最大 + 1」——删除号不回收，正好解释了为什么 core 要存它而不是每次扫。
- **主扩展不在表里**：`ams8000` 主扩展 16192 只有 `MNUM 16192/1` + `WORLD 16192/0`（max 1），设计库的用户元素全落在次扩展（24384 / 32576）。`TES023` 主扩展 9227 有 128 个元素、max 299，但页 1 也只列了 17419——说明它早期在主扩展建过、后来切到次扩展 17419 长，主扩展停长后计数器就不再维护在表里（或另存）。
- **纯字典 `ams5100`** 全 238 个都在主扩展 13292、页 1 无对——建一次不长的库根本没有计数器。
- 头字 `[2]` = 2 / 5 / 8 随库不同，像是「桶 / 表项游标」而非计数器（与对数不成简单比例），本轮不深究。

## 决策与后续

- **S6（本轮）**：`Transaction` 按「新 refno = (owner 所在扩展的 word0, 该扩展当前 max word1 + 1)」，会话内连续发号；不写页 1；590 兜底。理由：页 1 CoW（A11）未建，且写计划 §4 已把 max+1 列为 sanctioned fallback。
- **S6b（记入计划 §2 C 留待 / §7 触发）**：页 1 桶表 CoW，发号改为读该扩展计数器、自增、把页 1 随会话一并 CoW 写回——触发 = 需要与 E3D 交替写同一扩展且不容许号段有空档，或要把「删除号不回收」这条对齐到字节。届时还需把 routine 597 展到字节级，坐实计数器的读改写点。

## 复现

```powershell
# 创建路径
& $env:TEMP\ib.ps1 exec idalib-4000 --sql "SELECT decompile(0x592ace0)"   # DB_Element::createAt
& $env:TEMP\ib.ps1 exec idalib-4000 --sql "SELECT decompile(0x5195263), decompile(0x5195312)"  # ELCREL / ELCREA
& $env:TEMP\ib.ps1 exec idalib-4000 -c "import idc; _r=hex(idc.get_name_ea_simple('BCREAT'))"   # BCREAT 地址后 decompile
# 页 1（任意库前 4 KB，big-endian 逐字）——见 e3dio_probe（未入库）
```
