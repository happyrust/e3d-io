# 2026-09-22 `RecordDraft` 的数据流 ↔ core.dll 的元素块（in-memory record）——逐段对照（ida-bridge + 四库语料）

- 问题（用户原话）：「结合 ida-bridge 分析 draft 的数据是如何处理的」——即 `write::RecordDraft`（implicit 字 / 成员表 / explicit 流 / 待放 hunk）在 e3d-io 里怎么一路变成页上的字节，逐段对照 core.dll 的同一步。
- 手段：ida-bridge 活桥，`core.dll` = `D:\ida_scratch\plant3\core.dll.i64`（E3D 3.1，实例 **`idalib-4000`**）；本文所有地址、常量均本轮 `decompile()` 实读（复现命令见 §7）。语料核对用一个依赖 e3d-io 的临时 cargo 探针（`%TEMP%\e3dio_draft_probe`，未入库）扫了 4 个装机库（§5）。
- 承接（只引用）：`2026-09-20-core-dll-dabacon-write-path-algorithms.md`（下称「算法清单」，A2 / A5 / §5 类型 19）、`2026-09-22-core-dll-refno-allocation.md`（创建路径到 routine 597）、`2026-09-22-core-dll-text-charset-write.md`。
- 只读：未改 IDB，未改生产代码，未跑测试套（探针在 `%TEMP%`）。

---

## 0. 一句话

core 的「draft」= **元素块条目（60 B）指着的三样东西**：临时页上的一条记录（implicit 区，原地改）、堆上的成员表、堆上的属性块（两者同一种 4 字头布局），三者在**元素块释放**（`sub_5AE9460`）和**存盘搬运**（`sub_5AE9960`）时才被 `sub_5AEC110` 切成页上的块链。e3d-io 的 `RecordDraft`（`implicit: Vec<u32>` / `members: Vec<RefNo>` / `explicit_stream: Vec<u8>` / `hunks: Vec<HunkDraft>`）与之一一对应，`place_record` / `place_block` 与 `sub_5AE9DF0` / `sub_5AEC110` 是同一套切块规则。**逐字一致的**：记录头 11 字与格式字、块段头 5 字 / hunk 段头 7 字、explicit 项头 `type<<26|words`、类型 19 与 hunk 目录项、implicit 槽写法（只写 `[count][元素]`、类型 5 位读改写）、成员 / explicit 块顺序、hunk 段最小 9 字。**不一致的**（按影响排）：① 新建元素 core 先从模板 chunk 2 播种默认 explicit 项、TYPEX 是随后普通追加的（四库 34 478 条带流记录 0 例外），e3d-io 不播种且把 TYPEX 强放流首；② `4SCEA` 对 explicit 属性**零值不存、存了的置零即删**（描述子第 6 字 bit 1 例外），e3d-io 照写；③ explicit 项变大时 core 删旧追加到流末，e3d-io 原地替换；④ 存盘时 hunk 落在成员块之后、explicit 块之前，e3d-io 放在记录之前；⑤ 格式字两个 14 位计数 core 在 0x3FFF 饱和、读侧沿链数长，e3d-io 不饱和（≥ 8 191 成员 / ≥ 16 383 explicit 字溢进相邻位域——**T2 规模门里 E3D 把 20 128 成员的 ZONE 读成 3 744 个，就是它：40 256 字 `& 0x3FFF` = 7 488 字 = 3 744**）；⑥ 成员 / explicit 段最少 2 个载荷字（`a6 = 7`），e3d-io 允许 1 个；⑦ core 没有「把 implicit 属性置回未设」的操作（`4RCEDA` 对 implicit 报 539），e3d-io 的 `Unset` 写哨兵 `0x80000001` 是它没有的超集；⑧ compact 记录 core 在首次改动时转 wide，e3d-io 拒。除 ⑤ 外都不影响 E3D 读回（TTY 门已证），影响的是「和 E3D 逐字节相同」；⑤ 是真缺陷，成员块超过 16 383 字后 E3D 只装低 14 位那么多成员。

---

## 1. core 的 draft：元素块条目 + 两张堆表

### 1.1 元素块条目（`dword_6A54024` 表，`+8` = 当前下标，条目 = 表基 + 16 + 60·idx，15 个字）

`4GTE`（`sub_5AEB6B0`）装入、`sub_5AECBE0` 改写、`sub_5AE9460` 释放；`dword_6A5401C` 链着每个 db block（会话视图）自己的一张表，同一元素在别的视图里的条目按 `(refno, ext, sesno)` 四字相等认出并同步指针。

| 字 | 内容 | 出处 |
|---|---|---|
| 0 | noun 模板指针（`sub_5AF6AB0` 取） | `4GTE` `v6[0] = v48` |
| 1 | 记录所在页的缓冲指针 | `v6[1] = v45` |
| 2 | **成员表**（堆，见 1.2；0 = 未装入） | `sub_5AEA430` 装 |
| 3 | **属性块 = explicit 流**（堆，见 1.2；0 = 未装入） | `sub_5AEA360` 装 |
| 4–5 | refno `(word0, word1)` | `4GTE` 校验记录 1/2 字相等，否则 10 / 663 |
| 6–7 | INDEX 叶值 `(pageno, packed)` | `sub_5AECBE0` 改后 `3CTE` |
| 8 | dbno（= `word0 & 0x1FFF \| (word0 >> 13) & 0x3E000`——**refno 的 word0 就是带 extent 位的 db 号**） | `4GTE` 开头 |
| 9–10 | (ext, sesno)＝该条目属于哪个视图（属性 0 / 1） | 同 |
| 11 | 页帧号 pfno | |
| 12 | 记录在页内的**4 字节字**偏移 | |
| 13 | implicit 长度 = **`(u16) record[0]`**（core 只取低 16 位；写侧写的是整字，所以高 16 位实际恒 0） | `v42 = *(unsigned __int16 *)(v45 + 4*v19)` |
| 14 | 0 | |

### 1.2 两张堆表同一布局（成员表 / 属性块 / 装入内存的 hunk 都是它）

```
[0] 改动标志：0 未改、1 改过、2 = 本会话刚写过 hunk（4SCEA 末尾 / 4RCEDA 删 hunk 项时置）
[1] 共享计数（几个视图条目指着它；减到 0 才写回 + free）
[2] 载荷字数
[3] 容量（字）
[4..] 载荷：成员表 = refno 对；属性块 = { [hash][type<<26 | words][data×words] }*
```

- 首分配：成员表 `2n + 14` 字（`sub_5B21E00`），属性块 `words + 4`（空块 68）；扩容：成员表 realloc 到 `max(cap, 2n) + 20`，属性块 `5·(words + used + 7) / 4`（`4SCEA`）；扩容后把 `dword_6A5401C` 链上同一元素的条目指针一并改掉。
- e3d-io 对应：`members: Vec<RefNo>`（对 = 两字）、`explicit_stream: Vec<u8>`（同一 `[hash][hdr][data]` 流，只是按字节存）；没有改动标志 / 共享计数——一次 staging 用完即弃，不需要。

### 1.3 记录（implicit 区）住在页上，不在堆上

- `sub_5AECBE0()`（`4SCEA` / `4RCEDA` / `sub_5AB91B0` 三条写路的第一句）：`sub_5AF0360(pfno)` 判记录所在页能否原地改（有刷盘上下文：本库本 ext 且 `pageno ≥ 首新页`；无：ext=0 临时页且 `pageno ≥ dword_6453EBC` 水位）——能就直接改；不能就 **`sub_5AE9DF0(len, len, 1, …)` 在当前输出页里给整段 implicit 重分配**（长度 = 字 13，或 compact→wide 后的 `模板宽字数 + 11`），逐字拷过去，然后：`new[6..9] = 0`（两块地址清零）、`new[10] |= 0x20000000` 再 `&= 0xF0000000`（**只留高 4 位标志，两个 14 位计数清零**）；把成员表 / 属性块装进堆（没装的话）并置改动标志 1；`sub_5AEB030` 登记 refno→位置缓存；`3CTE(INDEX, refno, 新叶值)`。
- compact 记录（字 10 bit 29 清）走另一支：按模板逐属性把 f32 展成 f64（类型 2 / 6）、位属性只拷 bit 0 那一份、其它按 compact 字数拷；属性块经 `sub_5AE96D0` 同样展宽（类型 2 项头改 `0x08000002`，类型 6 改 `(2n−1) | 0x18000000`）。**e3d-io 拒 compact（`AttributeWriteError::CompactLayout`）**，语料 6 639 元素全 wide，无需求。
- e3d-io 对应：`RecordDraft::from_record_bytes` 把整条记录（`read_record` 拼好的）拆成三段——等价于「装入」；`encode` / `place_record` 重算 0–10 字——等价于 `sub_5AECBE0` 的清零 + `sub_5AEC110` 的回填。

### 1.4 什么时候写回页

| 时机 | core | e3d-io |
|---|---|---|
| implicit 字改动 | 立刻改临时页上的记录（`4SCEA` implicit 分支直接 `record[offset] = …`） | 改 `implicit[offset]` |
| explicit 项改动 | 只改堆上的属性块 | 只改 `explicit_stream` |
| hunk 值设定 | **立刻** `sub_5AEC110(kind 3)` 写到当前输出页，目录项当场拿到地址 | 存成 `HunkDraft`，目录项地址 `(0,0)`，`place_record` 时先放 |
| 成员表 / 属性块落页 | **元素块条目释放时**（`sub_5AE9460`：共享计数减到 0 且改动标志非 0 → `sub_5AEC110(kind 2 / 1, …, a6 = 7；shared 模式 21)`）——写到**当时的**当前输出页，未必与记录同页 | `place_record` 里紧跟记录 |
| 存盘 | A5.1 `sub_5AE7C30` 对每个 ext=0 的叶 `sub_5AE9960` **整条重搬**到真实页：implicit → 成员块 → 仍在临时页上的 hunk（`a9[1] = INT_MAX` 使 ext≠0 的旧 hunk 跳过不搬）→ explicit 块 | 同一次 staging 直接落真实页 |

---

## 2. 逐段对照

### 2.1 记录头 11 字 + 格式字（`RecordDraft::encode` / `place_record` ↔ `4CEIL` `sub_5AB2D40`、`sub_5AECBE0`、`sub_5AEC110`）

| 字 | core 写法 | e3d-io | 一致？ |
|---|---|---|---|
| 0 | implicit 字数（整字写、读时 `(u16)`） | `implicit.len()` 整字 | 一致（高 16 位恒 0） |
| 1–2 | refno | 同 | 一致 |
| 3 | noun hash | 同 | 一致 |
| 4–5 | owner；**新建时先写 0**（`4CEIL` 第 148–149 行），随后 `sub_5AB6990(位置字, &refno)` 挂进 owner 成员表时回填（回填点在 `sub_5AB6990` 内，本轮未展开——推断） | 直接写 `draft.owner` | 结果一致 |
| 6–7 | explicit 块首段 `(page, ext \| off<<13)`；无块 = 0,0 | 同（`pack_offset`） | 一致 |
| 8–9 | 成员块首段地址；无 = 0,0 | 同 | 一致 |
| 10 | `0x20000000 \| min(成员字, 0x3FFF) \| (min(explicit 字, 0x3FFF) << 14)`——**两个计数各 14 位、在 0x3FFF 饱和**（`sub_5AEC110` 第 116–122 / 508–512 行）；`sub_5AECBE0` 清零时保留高 4 位（bit 28–31） | `wide_format_word` = `0x20000000 \| member_words \| explicit_words << 14`，**不饱和** | 计数 < 0x3FFF 时一致；差异 ⑤ |

- 读法（`sub_5AEA500` 第 66–80 / 137–181 行）：成员块字数 = `record[10] & 0x3FFF`、explicit 块字数 = `(record[10] >> 14) & 0x3FFF`，**装入时就拷这么多字**；计数 == 0x3FFF 才沿块链累加每段 `(u16)seg[0] − 段头字数`，并把**最后一段末尾的 0 字剔掉**（`while (!*v26) …`）。e3d-io 的读侧 `record::block` 不看字 10、只沿地址走，所以 e3d-io **读**不受影响；**写**超过 0x3FFF 字（8 191 个成员 / 16 383 个 explicit 字）时 `member_words` 溢进 explicit 位域、explicit 溢进 bit 28+，**E3D 就只装低 14 位那么多**。
- **这正是 T2 规模门看到的「E3D 成员枚举封顶 3 744」**（计划 §7 T2 条）：ZONE 20 128 成员 = 40 256 字，`40 256 & 0x3FFF = 7 488` 字 = **3 744 个成员**，与 `Q MEM` / `!!CE.Members.Size()` 停下的数（124 原成员 + 3 620 BOX）逐位相等；溢出的 2 位又给 explicit 计数加了 2（ZONE 的 explicit 块末尾多读 2 字，撞到 `hash == 0` 停）。元素逐个 `=refno` 可达是因为 INDEX 与记录本身没坏；`--spread` 400/ZONE = 800 字没过阈值所以全数读回。不是 E3D 的上限，是 e3d-io 写坏了字 10。

### 2.2 implicit 槽（`write::attr::set_attribute` Implicit / Bit 分支 ↔ `4SCEA` `sub_5AB7B90` 第 220–268 行）

core：描述子（`4GAD` `sub_5AB5920` 取，条目字：`[2]` 类型、`[3]` 逻辑个数、`[5]`/`[8]` compact / wide 编码位置、`[6]` 标志）wide 位置 `& 0xFFFFF` 非零 → implicit：
1. `offset ≥ (u16) record[0]` → **623**「attribute offset is %d」；
2. 类型 5 → `word = (word & ~(1 << (loc >> 20))) | (*buf << bit)` 读改写一位，返回；
3. 「带计数」（`v124`：逻辑个数 == 0 或 > 1，或类型 14 / 17 / 18 / 19；UDA 无描述子时 = 类型 ∉ {2,3,4,5,12}）→ 先写 `record[offset] = *len`（元素个数）再 `offset++`；
4. 拷 `a3` 个**数据字**（调用方给的是字数不是元素数）；**槽里没用到的字不动**，也不做槽容量 / 越界检查。
5. 元素个数 > 逻辑个数 → **23**「Given size is %d, required size is %d」——只对 implicit 槽检查（第 197–216 行在 implicit 判定之内）。

e3d-io：`check_slot`（槽容量 + 记录边界，比 core 严）、`write_bit` 同一位运算、`Encoded::Implicit([count][元素])` 同形、`descriptor_is_array` = `count == 0 || count > 1 || 14/17/19`（core 多一个 18；18 只有 UDASTB，语料 0 设值）。`check_count` 对 explicit 数组也拦——core 对 explicit 数组不拦逻辑个数（ROKEYS 之类靶的就是这个）。**一致，e3d-io 更严。**

### 2.3 explicit 项（`upsert_explicit` / `set_explicit_words[_checked]` / `remove_explicit` ↔ `4SCEA` 第 271–939 行、`4RCEDA` `sub_5AB6E90`）

core 在堆上的属性块里按 hash 线性找（从字 4 起，步进 `(hdr & 0x3FFFFFF) + 2`）：

| 情形 | core | e3d-io | 一致？ |
|---|---|---|---|
| 没找到、值是「零」（数组 `len == 0`；标量 `*buf == 0`，类型 2 看 `double == 0.0`）且描述子字 6 bit 1 清 | **不存，返回 1** | 追加一项 | 差异 ② |
| 没找到、非零 | 追加 `[hash][type<<26 \| words(+1)][count?][data]`——数组多一个计数字；类型 19 直接拷 `[n]{[i][words][bytes][载荷]}` 不加计数字 | `upsert_explicit` 追加同形 | 一致 |
| 找到、新值是零（`sub_5AB2A30`）且字 6 bit 1 清 | **`4RCEDA` 删项** | 写零 | 差异 ② |
| 找到、同字数 | 原地覆盖（数组连计数字） | 原地 | 一致 |
| 找到、更短、inline 且非 19 | **原地缩小**：后面的项整体前移 `旧字数 − (新数据字数 + 计数字)`，`[2]` 减同数，项头改 `(type<<26) \| (新数据字数 + 计数字)` | 原地（重建流） | 一致（顺序不变） |
| 找到、更长 / 类型 19 / hunk 项 | **`4RCEDA` 删旧项 → 追加到流末** | 原地替换 | 差异 ③（顺序） |
| 删项（`4RCEDA`） | 后续项前移、`[2] −= words + 2`；hunk 项还登记回收 `sub_5B14640`；implicit 属性 → **539**；没有该项 → 18 | `remove_explicit` 重建流 | 一致；implicit 见 ⑦ |
| 项头位宽 | `type << 26 \| words`，words 26 位 | `(type_code << 16) \| len`，`type_code = type << 10`，len ≤ `0xFFFF`（`MAX_INLINE_DATA_WORDS`） | 位模式相同；e3d-io 限 65 535 字（core 有 hunk 开关时 ≥ 0x2000 即外联，不会碰到） |

- **描述子字 6 bit 1（e3d-io `TemplateAttr::compact.flags & 2`）**：`4SCEA` 的 `Block = (desc[6] >> 1) & 1`——置位的属性零值照存；`4CEIL` 播种时跳过它（§2.7）。语料：`ams8000` 全部 7 032 个「值为 0/false 的单字 explicit 项」里 7 028 个是 chunk 2 播种（ISOH / LOOS / NCUTS …），另 4 个是 BRAN 的 `HDSHAP` / `TDSHAP` = 0（描述子字 6 = 0x2，正是 bit 1 置位的属性）；`ams7999` 27 880 个全是播种；`ams251142` / `TES023` 0 个。**没有一个例外**——e3d-io 若用 `set_attribute(Int(0))` / `Bool(false)` / `RefNo(0,0)` 写 explicit 属性，会写出 core 从不写的项（E3D 读回为 0，功能等价）。
- e3d-io 的 `TemplateAttr` 把描述子字 6 叫 `compact.flags`、字 9 叫 `wide.flags`；core 在 `4SCEA` 与 `4CEIL` 里**不分布局都读字 6**，字 9 在这两处未见使用（RREF 的字 9 = 0xFFFFFFFF）。命名待商，本文按「字 6 = 属性标志」写。

### 2.4 hunk（`set_*_hunk` / `HunkDraft` / `explicit_stream_words_with_hunks` ↔ `4SCEA` 第 665–938 行、`sub_5AEC110(kind 3)`）

- 阈值（`4SCEA` 第 671–676 行）：`words ≥ 0x2000`，**描述子逻辑个数 == 0（无上限数组）时降到 1024**，`a7 & 1` 且 `≥ 1024` 亦可；还须 `DB_Attribute::initialised(&dword_6A540A8)`（运行态开关）或已是 hunk。升级表 6→21、7→20、10→22、19→23（`hunk_type_of` 一致）。`DBMDAT.DBMVAL` 5 709 字 < 0x2000 却是 hunk = 走的 1024 档。e3d-io 不自动升级（调用方选 `set_*_hunk`，`set_explicit_words_checked` 拒 > 0xFFFF）——已在计划里定为「调用方显式选」。
- 目录项：`[hash][type<<26 \| 5][1][page][packed][count][words]`——第 3 字 `1` = `(int)(4 × 0.25)`、项头 `5 = 1/0.25 + 1`（`xmmword_6453E50 = 0.25`，类型 11 的元素/字比）；`count` = 元素个数、`words` = 载荷字数。`HunkRef::to_words` 同形。**一致。**
- 段：7 字头 `[3<<16 \| words][refno0][refno1][cont0][cont1][hash][type]`，最小 `a6 = 9`（2 个载荷字）。`place_block(3, [hash, type], HUNK_MIN_WORDS − HUNK_HEADER_WORDS = 2, …)` **一致。**
- 时机与位置：core 设值当场写到当前输出页（记录之后、其它块之前的任意位置），存盘搬运时落在 **成员块之后、explicit 块之前**（`sub_5AE9960` 第 68–110 / 118–197 行顺序）；e3d-io `place_record` 把待放 hunk **放在记录之前**（因为目录项地址要先算出来）。差异 ④，读侧无感（两边都沿地址走）。
- 已存盘的 hunk：目录项原样重发、地址不动（e3d-io `set_attribute` 的 `RawWords` 直通、`from_record_bytes` 不解 hunk）= `sub_5AE9960` 的 `a9` 跳过规则。**一致**（TTY 门第二轮已证）。

### 2.5 成员表（`push_member` / `remove_member` / `move_member_after` ↔ `sub_5B21E00` / `sub_5B219B0` / `sub_5B21F80` / `sub_5B21C10`）

堆表 `[4..]` 每成员两字；`sub_5B21E00(n)` 保证还能放 n 个。合并时的插 / 删 / 移三例程已在算法清单 A3b 展开并落地为 `member_list_diff` / `merge_members_with`，本轮不重复。落页：`sub_5AEC110(table, record, 2, 0, pfno, 7, 0)`，段头 `[2<<16 \| words][refno0][refno1][cont0][cont1]`，`record[8..9]` = 首段地址，字 10 低 14 位 = `min(2n, 0x3FFF)`。e3d-io `place_block(2, &[], 1, member_payload, …)` **同形**；空表两边都写 `(0,0)` + 计数 0。

### 2.6 类型 14 / 15 / 16 / 17（`write::reverse` ↔ `sub_5AB91B0` → `4SCEA`）

`sub_5AB91B0(hash, k, type, buf, &len, aux, auxLen, elemWidth)`：也以 `sub_5AECBE0()` 开头；`4GCEA` 读旧值，组旧键 / 新键（值 + 自身 refno），`3CTE(表 = hash, …, 0)` 删旧键、`3CTE(…, 1)` 插新键（523 → `sub_5AFF6D0` 建表），类型 16 值 `(0,0)` 走 `4RCETA` 删；**最后 `4SCEA(hash, type, words, buf, &len, 0, 0)` 写项**（第 417 行）——所以 TYPEX / NAME / SPRE 在流里的位置服从 §2.3 的追加规则，**没有「放流首」的特例**。e3d-io `reverse::set_reference` / `set_reference_array` / `ensure_typex` + `keyed_cow`：表侧一致；项的位置见 §2.7。

### 2.7 新建元素（`Transaction::insert` ↔ `4CEIL` `sub_5AB2D40`，routine 597 `DCRTNB` 的实现体）

core：
1. 模板 `sub_5AF6AB0(…, noun, 2, &tpl)`；**默认记录** `sub_5AF6AB0(…, noun, 1, &impl_words, &impl_n, &expl_stream, &expl_n)`（= `*vir.dat` chunk 1 / chunk 2，e3d-io `NounDefaults::{implicit, explicit}`）。
2. refno 给定则查重（569）、否则 `sub_5AEA9C0` 发号（refno 证据文档）。
3. `sub_5AE9DF0(impl_n + 11, …)` 分记录：`[0] = impl_n + 11`，`[1..2]` refno，`[3]` noun，`[4..9] = 0`，**`[10] = (expl_n << 14) | 0x20000000`**，`[11..]` = chunk 1 逐字。
4. **属性块播种**（第 166–327 行）：遍历模板属性，取「explicit-only（wide 位置 == 0）或 UDA」且**字 6 bit 1 清**的那些，在 chunk 2 里按 hash 找到就把 `[hash][hdr][data]` 拷进堆表（**chunk 2 顺序**）；若没有任何属性带 bit 1，则整段 chunk 2 原样拷。模板头字 0 bit 0x40 置位的 noun 还把其余 explicit-only 属性全部以零数据补齐并当场落页（`a6 = 21`）——哪些 noun 有此位本轮未查。
5. `3CTE(INDEX, …, 插)`；主元素（模板头字 4）且写模式且属性 17 → claims manager `+8` 登 claim；`sub_5AB6990(位置字, &refno)` 挂到 owner 成员表并回填 owner；`4GTE` 到新元素，把播种好的堆表挂到条目字 3。
6. TYPEX 不在这里：由 `DB_Element::createAt` 之后经 `sub_5AB91B0` → `4SCEA` **追加**。

e3d-io：`vec![0; 11] + defaults.implicit`、字 10 只置 wide 位（编码时重算）、explicit 流**空**、`plan_side_tables` 里 `ensure_typex` 用 `set_explicit_words_first` **放到流首**。头与 implicit 一致；explicit 流的差异见 §5 数字：四库 34 478 条带 explicit 流的记录，**播种前缀（chunk 2 顺序、bit 1 过滤）100% 成立、0 例外**；TYPEX 在 AMS 两库 100% 紧跟播种前缀（前缀为空时才是流首），在 E3D 3.1 自建的 `TES023` 里则排在 NAME / FUNC / DUNIO **之后**（`EQUI: [NAME TYPEX]`、`ZONE: [DUNIO NAME FUNC TYPEX]`）——即 **TYPEX 就是「第几个被设的属性」，从来不是「强放流首」**。`RecordDraft::set_explicit_words_first` 文档里那句「every TYPEX-carrying record of ams8000_0001 opens its stream with it」不成立：`ams8000` 5 941 条带 TYPEX 的记录里 2 690 条流首是 TYPEX，其余 3 251 条流首是 ISOH（BRAN / PIPE / ELBO / BEND / ATTA / FTUB 的播种项）。

### 2.8 Unset

- core **没有**「把 implicit 属性置回未设」：`4RCEDA` 对有 implicit 槽的属性直接 **539**；DB4 层（0x5AA0000–0x5B40000）把 `0x80000001` 写进内存的只有 `4RCETA`（类型 14 槽复位）、`sub_5AB91B0`（类型 14 空槽）、`sub_5B02530`（表页哨兵键）——没有一处写进 implicit 槽。implicit 槽里的哨兵只能来自 chunk 1（默认记录本身就是哨兵）。
- e3d-io `DescriptorValue::Unset`：implicit → 槽首字写 `0x80000001`（类型 5 清位）、explicit → 删项。explicit 侧与 `4RCEDA` 一致；implicit 侧是 core 做不到的事，读侧（`sub_5AB4CF0` 两个哨兵都当未设 → 模板默认）能正确解释，但 E3D 自己的库里不会出现「被改回哨兵」的槽。

### 2.9 落页切块（`place_record` / `place_block` ↔ `sub_5AE9DF0` / `sub_5AEC110`）

| 规则 | core | e3d-io | 一致？ |
|---|---|---|---|
| 新数据页 | `sub_5AEE230` 取页、`buf[0] = 7`、已用 1 字 | `new_data_page`：字 0 = 7，used = 1 | 一致 |
| 分配器 | `sub_5AE9DF0(want, min, …)`：页剩余 `< min` 才换页；给 `min(剩余, want)` 字 | `place_block`：剩余 `< 段头 + min_payload` 换页；`take = min(remaining, capacity)` | 一致 |
| implicit 区 | `want == min == len`（`sub_5AECBE0` / `sub_5AE9960` / `4CEIL`）→ 整段不跨页 | 整段不跨页，放不下 → `RecordTooLong` | 一致 |
| 段头 | `[kind<<16 \| 段总字][refno0][refno1][cont0][cont1]`；hunk 再 `[hash][type]`；续接地址在**分到下一段后回填**上一段 3/4 字 | 同形；续接页号**先取**再写头 | 一致（结果同） |
| 段最小 | 成员 / explicit `a6 = 7`（2 载荷字；shared 模式 21）、hunk 9 | 成员 / explicit `5 + 1 = 6`、hunk 9 | 差异 ⑥：页剩 6 字时 e3d-io 开一个 1 载荷字的段（成员对会被劈成两段），core 换页 |
| 块顺序 | implicit → 成员块 → （存盘时）临时页 hunk → explicit 块 | hunk → implicit → 成员块 → explicit 块 | 差异 ④ |
| 段内多余字 | 段是紧的（分配器只给需要的）；原地改写路才 `memset 0` | 紧的 | 一致 |
| 块类型校验（读） | `(hdr >> 16) & 0xF != kind` → 551 | `BlockKind::from_raw(u16)` 精确匹配 1 / 2 | 一致 |

---

## 3. 差异清单（按影响排序，⑤ 之外都不影响 E3D 读回；每条给最小改法，本轮未改代码）

| # | 差异 | 影响 | 最小改法 |
|---|---|---|---|
| ① | 新建不播种 chunk 2、TYPEX 强放流首 | 新元素的 explicit 流与 E3D 自建的字节不同、少了 ISOH / RLOC / HREL / DELDSG / AEXCES / LEXCES / LOOS 这类零值默认项（E3D 读回时按模板默认解释，值相同） | `Transaction::insert` 起草时：按 `template.defaults.explicit` 顺序，取 `attrs` 中 `wide.location.word_offset() == 0 && compact.flags & 2 == 0` 的项 `set_explicit_words(hash, type << 10, words)`；`ensure_typex` 改用普通追加（`set_explicit_words`）并在 build 闭包之后调（= `TES023` 观察到的 E3D 3.1 顺序） |
| ② | explicit 零值照写 / 置零不删 | 写出 core 从不写的项；`content_eq` 认为「变了」而 core 认为「没变」（合并去重口径不同） | `write::attr::encode` 之后：落 explicit 且值为零（标量首字 0 / 类型 2 为 0.0 / 数组 0 元素）且 `compact.flags & 2 == 0` → 改走 `remove_explicit`；文档写清 |
| ③ | explicit 项变大原地替换 | 流内顺序与 E3D 不同 | `upsert_explicit`：`data.len() > 旧 len` 时 `retain` 掉旧项再 `push` |
| ④ | hunk 放记录之前 | 页布局顺序与 E3D 不同 | `place_record`：先放头 + implicit（地址先记）、成员块，再放 hunk，最后 explicit（explicit 本来就在 hunk 之后组词） |
| ⑤ | 格式字计数不饱和 | **已在 T2 撞到**：≥ 8 191 成员或 ≥ 16 383 explicit 字时字 10 位域互相污染，E3D 只装低 14 位那么多成员（20 128 成员的 ZONE 被读成 3 744）；这是本文唯一一条影响 E3D 读回的差异 | `wide_format_word` 各 `min(_, 0x3FFF)`；饱和时确保最后一段无尾随 0 字（core 会剔）；用 T2 的 20 000 成员 ZONE 在真 des.exe 上复验 `Members.Size()` = 20 128 |
| ⑥ | 段最小载荷 1 vs 2 | 边界布局不同 | `place_block(1/2, …, min_payload = 2, …)` |
| ⑦ | implicit `Unset` 写哨兵 | E3D 库里不会出现的字样，但 E3D 读得懂 | 保留；文档注明 core 无此操作，或改为写回 chunk 1 默认字 |
| ⑧ | compact 记录拒 | 语料无 compact，无需求 | 留待 |
| ⑨ | explicit 数组也拦逻辑个数 | 比 core 严；ROKEYS 之类已单独放行 | 留待 |

---

## 4. `4SCEA` 入参与 e3d-io 的对应（供 T1b「按属性名以字符串设值」用）

`4SCEA(hash, type, data_words, buf, &count, append_desc, flags)`：`type` = 存储类型（UDA 用 4SCEA 自带的类型表判「带计数」）、`data_words` = **字数**、`*count` = 元素个数（文本 = 字节数、类型 19 = `1 + Σ(4·words_i + 3)`、hunk = 元素数）；`sub_5AB41D0(type, desc[2])` 校给定类型与描述子类型相容（6↔21 等，未展开）；返回 0 时错误码在 `dword_6453B98`（23 大小、623 偏移、106 内存、630 记录页不可写、551 块类型）。e3d-io `set_attribute(template, hash, &DescriptorValue, TextFormat)` 把「类型 + 字数 + 元素数」三件事都从描述子和值推出来，少一处调用方出错的地方；T1b 的字符串入口在这之上，不需要再碰 4SCEA 口径。

---

## 5. 语料核对（探针 `%TEMP%\e3dio_draft_probe`，`scan_elements(Full)`，desvir.dat = `schema\e3d31\binaries`）

| 库 | 元素 | 带 explicit 流 | 播种前缀成立 / 违反 | 带 TYPEX | TYPEX 在流首 | TYPEX 紧跟播种前缀 | 零值单字 explicit 项（播种 / 非播种） |
|---|---|---|---|---|---|---|---|
| `ams8000_0001` | 6 639 | 5 969 | **5 969 / 0** | 5 941 | 2 690 | **5 941** | 7 032（7 028 / 4 = BRAN HDSHAP·TDSHAP，字 6 bit 1） |
| `ams7999_0001` | 34 709 | 28 054 | **28 054 / 0** | 27 441 | 14 178 | **27 441** | 27 880（27 880 / 0） |
| `ams251142_0001` | 948 | 395 | **395 / 0** | 362 | 362 | 362 | 0 |
| `TES023`（E3D 3.1 自建） | 320 | 60 | **60 / 0** | 17 | **0** | 0（在 NAME / FUNC 之后） | 0 |

- 「播种前缀」= 该 noun chunk 2 中、属性 explicit-only 且描述子字 6 bit 1 清的项，按 chunk 2 顺序；记录里出现的那些必须恰好构成流的前缀（老 schema 建的记录可以缺项，如 EQUI 的 CACHID 73/77 缺——CACHID 字 6 = 0x3，本就跳过；4 条有的是后来单独设的、排在流末）。
- 每 noun 的播种序列（desvir.dat 实读）：PIPE `[ISOH DELDSG SLOREF]`；BRAN `[ISOH DELDSG SLOREF AEXCES LEXCES LOOS]`（chunk 2 还有 CACHID / HDSHAP / TDSHAP / LCHKDA，bit 1 置位不播）；ELBO / VALV `[ISOH RLOC HREL DELDSG AEXCES LEXCES LOOS]`；FLAN `[… NOMDIR]`；ATTA `[… ANGL …]`；WELD `[… WELDTY]`；BOX / CYLI / ZONE / SITE / NOZZ / GROU / EQUI / SCTN / STRU / FIXING **空**（EQUI·SCTN·STRU 的 CACHID、FIXING 的 BANG 都被 bit 1 挡掉）。与 `ams8000` 各 noun 的流首逐字相同（BRAN 24384/2500 `[ISOH DELDSG SLOREF AEXCES LEXCES LOOS TYPEX NAME …]`、ELBO 24384/2868 `[ISOH RLOC HREL DELDSG AEXCES LEXCES LOOS TYPEX DESP UNIPAR]`、PIPE 24384/2499 `[ISOH DELDSG SLOREF TYPEX NAME …]`）。
- TTY 门写的 BOX 与 E3D 逐字一致的原因也在此：BOX 播种序列为空、模板无 TYPEX，差异 ①③④ 都碰不到。

---

## 6. 未展开 / 推断

- `sub_5AB41D0`（类型相容表）、`sub_5AB2A30`（找到项时的「零值」判定，按未找到分支的内联版本推断为同一规则）、`DB_Attribute::initialised(&dword_6A540A8)` 的运行态含义、`4SCEA` 的 `a6` 追加模式（`a6[10] = 1/2/3`，类型 19/23 增量追加）与 `sub_5AEC110` 的原地改链分支（只对本会话已在临时页上的链生效，e3d-io 单次 staging 用不到）。
- 模板头字 0 bit 0x40（新建时把全部 explicit-only 属性以零数据补齐并当场落页）——哪些 noun 有，未查；e3d-io `NounTemplate` 不暴露头字 0。
- 描述子字 6 / 字 9 的完整位义（本文只用 bit 1）；e3d-io 的 `compact.flags` / `wide.flags` 命名是否恰当待商。
- `sub_5AECBE0` compact→wide 转换里位属性「只拷 bit 0 那份」的推断（`(loc & 0xFFF00000) == 0` 才拷）。
- `sub_5AEB030` 登记的 `dword_6A540CC` 缓存（32 B/桶，按 `word1 % dword_6A540C8`）只是 refno→位置的快取，不影响格式。

---

## 7. 复现命令（本机）

> `%TEMP%\ib.ps1` 里写的 `.agents-old` 路径已不存在，本轮用 `%TEMP%\ib2.ps1`（同内容，路径改 `C:\Users\dpc\.agents\tools\ida-bridge`）；`%TEMP%\dec.ps1 <addr>…` 把 `decompile()` 结果落成 `%TEMP%\core_draft\<addr>.c` 纯文本。

```powershell
& $env:TEMP\ib2.ps1 list                                                                       # idalib-4000 = plant3\core.dll.i64
& $env:TEMP\dec.ps1 0x5AECBE0 0x5AE9DF0 0x5AE9960 0x5AEC110 0x5AEA500 0x5AEA430 0x5AEA360        # 重分配 / 分配器 / 存盘搬运 / 切块写 / 块装入 / 成员表装入 / 属性块装入
& $env:TEMP\dec.ps1 0x5AEB6B0 0x5AE9460 0x5AEB030 0x5AF0360 0x5AE96D0 0x5B21E00                  # 4GTE / 元素块释放（写回点）/ 位置缓存 / 页可写判定 / compact→wide 属性块 / 成员表扩容
& $env:TEMP\dec.ps1 0x5AB7B90 0x5AB6E90 0x5AB91B0 0x5AB2D40 0x5AE4510 0x5ABD470                  # 4SCEA / 4RCEDA / 类型 14–17 写 / 4CEIL 新建 / 5ECOK 前置 / routine 597 包装
& $env:TEMP\ib2.ps1 exec idalib-4000 -c "import ida_bytes, idc; _result_={i:(idc.get_strlit_contents(ida_bytes.get_dword(0x6003900+4*i),-1,0) or b'').decode('latin1') for i in (293,294,351,352,597,598)}"   # DADJBE / DFNDUB / DCRTNB=db_create_element_no_bucket_alloc
& $env:TEMP\ib2.ps1 exec idalib-4000 --sql "SELECT DISTINCT name_at(from_func) FROM xrefs WHERE to_ea = 0x5AB7B90 AND is_code = 1"   # 4SCEA 的 21 个调用者（含 sub_5AB91B0、sub_5AD5060）
& $env:TEMP\ib2.ps1 exec idalib-4000 -f $env:TEMP\find_unset_imm.py                              # 0x5AA0000–0x5B40000 里以 0x80000001 为立即数的指令（哨兵写点）
# 语料：%TEMP%\e3dio_draft_probe（cargo 项目，path 依赖 e3d-io / e3d-attlib，CARGO_TARGET_DIR=D:\Rust\target）
#   main.rs 三版：chunk 2 与描述子字 6 表；四库播种前缀 / TYPEX 位置 / 零值项统计；TES023 带 TYPEX 记录的流
```
