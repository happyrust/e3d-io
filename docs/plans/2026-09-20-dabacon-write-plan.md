# 计划：给 e3d-io 加 DABACON 写入（分三阶段，每阶段在副本库上写、用现有读侧往返验收）

状态：草案，待评审（2026-09-20）；阶段 0 与阶段 1 已落地并通过 §2 验收（见 §7 2026-09-20 第二条），§5 的八个决策点按默认值执行
日期：2026-09-20
分支 / worktree：`write-path` @ `D:\work\plant-code\old\vendor\e3d-io-write`（与 `../e3d-attlib` 同级，path 依赖可解）
依据：`docs/evidence/2026-09-20-core-dll-dabacon-write-path-algorithms.md`（下称「算法清单」，引用其小节号 A1–A12、§2、§4、§5、§6）
影响仓库：`vendor/e3d-io`（本仓库）。gen-model 只在阶段收口后切 path 指向本分支验收，本计划不改它。

---

## 0. 目标

让 e3d-io 能在一份 E3D 3.1 数据库文件上**追加一个新会话**，写出的文件满足两条：

1. **本 crate 自己的读侧原样读得回来**：`DbView::open` / `ReadOnlyEngine::open` 看到的最新会话就是刚写的那一个，`walk_sessions()` 链完整，`diff_sessions(old, new)` 报出的恰好是这次写入的增删改，`find_element` / `scan_elements(Full)` / `extract_element_with_descriptors` 读出写进去的值。
2. **对旧数据只字不改**：写入前的全部页（除页 0 的提交字）逐字节不变，新页全部落在旧末页之后且连续；崩溃在页 0 落盘前 = 文件仍是旧会话（算法清单 §4 五条不变量全部成立）。

阶段顺序照 core.dll 的层次**从下往上**：先把「页级写 + 会话页 + 页 0 提交」做成能往返验证的最小闭环（A4–A7），再上「记录追加 + INDEX CoW」（A2/A5），最后才是「多写三方合并」（A3a/A3b）。每一阶段单独可验、单独可停。

**不做**（本计划范围外）：`SessionPackageCentral` / Spectrum 服务（A8 Central）；页 1 桶表与多用户条目（A11）；extract 间 flush（A12）；CLAIM 表与 claims manager（A3a.6/7）——除非阶段 3 评审时决定要；E3D 2.1 文件格式（宽度表/模板一致，但未验）；覆盖 E3D 正在打开的文件（见 §5 Q6）。

---

## 1. 地基：已经确定的事实与已有的读侧能力

### 1.1 写路径的事实（全部来自算法清单，地址实读自 `idalib-33060`）

| 事实 | 出处 |
|---|---|
| 会话中不改盘上旧页；改动全在临时页（ext=0），SAVEWORK 时从 `old_end+1` 起顺序分真实页号，新页**只追加**、无洞 | A2、A4、A7、§4.1–2 |
| 提交顺序：新页（第一遍）→ 改写位置页（第二遍）→ **页 0 第 10/11 字 `LSPGNO`**；页 0 是唯一提交点 | A6、A7、§4.3 |
| 页 0 存盘时还改：第 9 字 `TMPLVN` >0 取负、属性 10==1 且 12≠1 时第 14/15 字 = 属性 7、第 94/96 字清零 | A6.4 |
| 会话页（类型 3）逐字表：1–2 上一会话页、3 sesno、4 OWNSES、5–6 末页、7–8 INDEX 根、9–10 CLAIM 根、11 类型标志、12–16 DATE、25–29 UDATE、30+ USER、40+ DESC、199/200+ 表目录三元组、503–504 DBVRSN 8 字节、505 写者用户/桶号、506–510 含 BASESN | §2 会话页表、A3a.8 |
| 文件大端；块 I/O 1-based：`offset = (blk−1) × 页字节`；页大小 = 页 0 第 13 字 `PGSIZE`（样本 512 字 = 2048 B） | §2、A9 |
| 写者互斥：`LockFileEx` 于 `0x7FFFFFF6FFFFFFFF` 处 1 字节哨兵（独占）；离开写模式 `FlushFileBuffers` | A9 |
| 元素记录不可变：改一次整条搬到新位置（类型 7 数据页 bump 分配），INDEX 叶值换指针 | A2、§4.5 |
| INDEX 叶值 `(pageno, packed)`，`packed = ext \| (4字节字偏移 << 13)`，ext=1 主文件 | §5 |
| INDEX/CLAIM/用户表都是同一种 B+ 树页（类型 5，7 字头，AoS 条目，`free_dwords` 定条数，内结点键 = 子树最小键，哨兵 `0x80000001`） | `src/index/mod.rs` 模块头（读侧已在 429 库上裁定） |
| 3.1 普通写（DOPENW）在建会话页前做三方合并：base ↔ working 的差异套到 latest 的克隆上；冲突 561/562/590/648/582 → 整次失败不写 | A3a |
| 属性级三方：只有我改过的属性写进 new；两人都改我的赢（extract flush 时别人赢）；清空 = 复位默认；类型 15 唯一键抢占；类型 18 列表按条目合并 | A3b |
| 成员表按增/删/移三种差异逐成员套上，锚 = 前一个共有兄弟；`kind==2` 的 noun 整段覆盖 | A3b.4 |

### 1.2 读侧已有的验收工具（不用另写读代码）

| 层 | 入口 | 用来验什么 |
|---|---|---|
| 页 | `page::io::PageReader::open` / `physical_page_count`、`PageId::byte_offset`、`PageHeader::parse` | 新页页号连续、页头类型正确、旧页字节不变 |
| 页 0 | `meta::DbDescriptor::parse`（`latest_ses_pgno`、`session_page_no`、`page_size_words`、`header_format`） | 提交指针指向新会话页，其余头字不变 |
| 会话 | `session::SessionPage::parse_as`、`DbView::walk_sessions` / `sessions()`、`SessionChain`、`session::resolve` | 新会话页每一字；链 newest→…→1 完整；`previous` 指旧最新页；`end` = 新末页 |
| 树 | `index::IndexNode::decode`（页级结构校验）、`IndexCursor::walk`、`choose_child`、`IndexDiff` | 每张新树页合法、键有序、每个键可路由；两根 diff = 本次改动 |
| 记录 | `record::read_element_record`、`ParsedElement::parse_as`、`extract_parsed_element_with_descriptors`、`session::ElementDelta` | 新记录可解析、属性值等于写入值、成员/owner 变化如预期 |
| 区间 | `DbView::diff_sessions(old, new)` → `SessionDiff::tally()`、`changed_elements` | Inserted/Deleted/Modified 三态计数与键集恰好等于本次写入 |
| 语料门 | `tests/index_page_decode.rs`（`packed_splits_at_bit_twelve_*`）、`tests/session_l1_contract.rs`、`tests/refno_native_stage0.rs` | 把写出的副本当新样本跑一遍，读侧契约不能红 |

### 1.3 还没定、会影响写法的（见 §5 决策点与 §4 风险）

属性号 → 控制块字段语义（算法清单 §6 第一条）、会话页第 4 字 OWNSES 与第 505 字的取值、`DBVRSN` 8 字节写什么、页 1 桶表是否必须更新、3SN 分裂策略、类型 19/23 与 hunk 20–23 的写法（阶段 2 不碰，见 §2.2「不做」）。

---

## 2. 分阶段

### 阶段 0 · 测试脚手架（半天）

- `tests/common/write_fixture.rs`：把一个样本库拷到 `%TEMP%`（`ams8000_0001` 18 MB/281 会话作主样本，再选一个 <1 MB 的小库作快样本），返回路径；结束删除。样本缺席 → 打印 skip（沿用现有 `common::missing` 口径）。
- `tests/common/bytes.rs`：`assert_prefix_unchanged(before, after, until_page, except_page0_words: &[usize])`——逐页比对，页 0 只允许给定字变化；`assert_appended_contiguous(after, old_end, new_end)`——新页都在 `(old_end, new_end]` 且每页第 0 字是合法页类型。
- 不引入新依赖。

**验收**：脚手架自身有单测（对同一文件比对 = 全等；人为改一字 = 报出页号与字号）。

### 阶段 1 · 页级写 + 会话页 + 页 0 提交（A4–A7；目标：写出一个「空会话」）

**范围**：新增 `page::io::PageWriter`（或在 `PageReader` 旁加写句柄）、`session::write::SessionPageBuilder`、`write::Commit`（阶段性的门面）。

做什么：
1. **打开写句柄**：`OpenOptions::read(true).write(true)`；Windows 上（feature `win-lock`，默认关）按 A9 对 `0x7FFFFFF6FFFFFFFF` 处 1 字节 `LockFileEx` 独占，提交后解锁 + `FlushFileBuffers`。非 Windows 用文件内容层面的乐观检查：提交前重读页 0，`LSPGNO` 变了就放弃（§5 Q6）。
2. **页分配器**：`next = old_session.end.page + 1`，只增不回退；写页 = 大端整页、`offset = (pageno) × page_bytes`（页号 0-based，即 core 的 `blk−1`），**缓存在内存直到 commit**。
3. **会话页构造**（按 §2 表逐字填）：类型 3；第 1/2 字 = 旧页 0 的 `LSPGNO`（链上一会话）；第 3 字 sesno = 旧最新会话 sesno + 1；第 4 字 OWNSES 抄旧会话页第 4 字（样本全为 `0xFFFFFFFF`，§5 Q2）；第 5/6 字 = 本次末页（= 会话页自身页号，空会话时）；第 7/8 INDEX 根、第 9/10 CLAIM 根 = 抄旧会话页（空会话不动树）；第 11 字 = 1；第 12–16 字 DATE、第 25–29 字 UDATE 按 `SessionDate` 的逆运算写本机时钟；第 30+ USER、第 40+ DESC 按现有 `parse_as` 的逆运算（长度字 + 4 字节/字打包，FORMAT≠1 时经现有 `text` 模块的逆向；阶段 1 只允许 ASCII，非 ASCII 报错）；第 199/200+ 表目录 = 抄旧会话页 `tables`（顺序保持：用户表、CLAIM、INDEX）；第 503/504 DBVRSN = 8 字节自有标识（§5 Q3）；第 505 = 抄旧会话页；第 506–510 = 抄旧会话页但第 508 字 `BASESN` = 旧 sesno。
4. **刷盘顺序**：新页按页号升序写 → 页 0 改第 10/11 字（新会话页 pageno、ext=1），第 9 字 >0 取负（第 14/15 字不动，因为我们不维护属性 7；§5 Q4）→ 写页 0 → `sync_all()`。**第 94/96 字不动**：算法清单 A6.4 说 core 存盘时清零，但 E3D 刚存过的 `ams8000_0001` 页 0 第 94 字是 2，说明那两字的读法还没坐实，宁可不碰。
5. 崩溃注入点：在「新页已写、页 0 未写」处提供一个测试钩子，验证此时读侧仍看到旧会话。

**不做**：任何树页、数据页；页 1；CLAIM。

**验收**（`tests/write_stage1_empty_session.rs`）：
- 在 `ams8000_0001` 副本上写空会话（comment `"e3d-io stage1"`）→ `DbView::open(copy)`：`session().sesno == 282`、`session().previous == Some(旧 LSPGNO 页)`、`session().end.page == 新会话页号`、`index_root`/`claim`/`tables` 与会话 281 相同、`comment == "e3d-io stage1"`、`user` 为写入值、`date`/`utc_date` 解出来与写入时刻同秒；`descriptor().latest_ses_pgno == 新页号`。
- `walk_sessions()` 得到 282 条，newest→oldest 严格递减、尾为 1；`sessions()` 同。
- `diff_sessions(281, 282).tally()` 全零；`scan_elements(Full)` 元素数与 281 相同；`find_element(任意 10 个已知 refno)` 的 `(page, offset)` 与之前一致。
- 字节：`assert_prefix_unchanged(旧, 新, until=旧 end, except_page0_words=[9,10,11])`；新文件只多一页；`physical_page_count` +1。
- 崩溃钩子：页 0 未写的中间态文件，`DbView::open` 看到的仍是会话 281，`walk_sessions()` 仍 281 条。
- 把副本喂给现有语料门（`index_page_decode` 三个契约测试改成可指定文件）不红。
- **人工门（可选，非阻塞）**：用 E3D 3.1 Design 以 `DOPENR` 打开副本，`q sessions` 能看到 282 且能 `q att` 元素——记录结果到本计划 §7。

### 阶段 2 · 记录追加 + INDEX CoW + 用户表（A2/A5；目标：真正写元素）

**范围**：`record::write::RecordEncoder`（11 字头 + implicit 区 + kind 1/2 块，含跨页续接 `(page, packed)`）、`index::write::{Cow, Leaf/Node 编辑, 分裂}`、`write::Transaction`（暂存增删改，commit 时统一编号 + 落盘）。

做什么：
1. **数据页**：类型 7，bump 分配（A2.1）：记录整条写到「当前输出页」剩余空间，不够就开新页；记录位置 = `(pageno, ext=1 | (字偏移 << 13))`。记录跨页只发生在块级续接（读侧 `record::block` 已定义 `continues_at`），阶段 2 先禁止跨页（单条记录必须整页放得下，否则报错），续接留阶段 2b。
2. **记录编码**：从 `ParsedElement` + 描述子逆向：implicit 区按模板 `encoded_location` 放值，类型宽度用 `StorageShape`（19/23、20–23 hunk 不支持 → 报错）；explicit 块 `[hash][type<<10 | words][data]`；成员块 kind 2、explicit 块 kind 1，块头 `[kind:u16][words:u16][owner][cont_pg][cont_packed]`。
3. **INDEX CoW**（A2.2/A5）：对每个改动键，沿根到叶的路径复制页（新页号从分配器取——**我们是单写者，直接分真实页号，不走 core 的临时页/重编号两步**；结果等价，写出的树与 core 的一样只引用 `≥ old_end+1` 的新页和未改的旧页），叶插/改/删条目并维护 `free_dwords`；满页按 3SN 分裂——分裂点取中位（§5 Q5，读侧不依赖策略）；根变则新根进会话页第 7/8 字。内结点键 = 子树最小键（读侧 `choose_child` 的规则）；哨兵条目 `0x80000001` 的处理照读侧 `leftmost_child`。
4. **用户表**：改了 `NAME` 的元素同步更新 NAME 表（类型 15 唯一索引，A3a.4/§5 表 15 行）：键 = 打包名字（变长键），值 = refno；重名 → 报错（不做抢占）。**读侧游标已补**（`index::named`，2026-09-20，见 §7）：NAME 表页头 `key_dwords = 0xFFFFFFFF`（core 判 `<= 0` 有符号即变长）、`value_dwords = 2`，条目 `[键长][键字×键长][值×2]`，键 = 带 `/` 的名字 4 字符/字 NUL 补齐，哨兵 = **首字** `0x80000001` 的条目（键长随它覆盖掉的旧键，写侧造页用 1 字即可），叶值 = refno、内结点值 = (页, ext)；键序 = 按字**无符号**比较、`0x80000001` 小于一切、前缀短者在前（`sub_5B00F10`/`sub_5B010F0` 实读）；路由 = 最大的 ≤ 目标键。写侧照这个布局造页，用 `NamedTable::iter` / `lookup_name` / `ReadOnlyEngine::find_by_name` 验收。其它用户表（TYPEX、CATR…）阶段 2 不维护，**但**要在会话页表目录里原样保留根。CLAIM 不动。
5. **提交**：分配顺序 = 数据页 → 树页（自底向上）→ 会话页，与 A5/A6 一致；页 0 同阶段 1。

**不做**：删除元素时不回收成员表以外的引用（TYPEX/CATR 表不更新，标在 §4 风险）；类型 19/23 与 hunk 20–23 的写；跨页记录；多 extent（ext≠1 一律报错）。

**验收**（`tests/write_stage2_elements.rs`，在 `ams8000_0001` 副本上）：
- 用例 A（新增）：在一个已知 ZONE 下新建 3 个 BOX（refno 取该库 `primary_extent_ref` 下未用号；成员挂进 ZONE）→ `diff_sessions(281, 282).tally()` = Inserted 3 / Modified 1（ZONE 成员表变）/ Deleted 0；`find_element(新 refno)` 解析出 noun `BOX`、owner = ZONE、写入的 XLEN/YLEN/ZLEN/POS 经 `extract_element_with_descriptors` 等值；ZONE 的 `ParsedElement.members` 多出 3 个且顺序如写入。
- 用例 B（修改）：改 5 个已有元素的一个 real 属性和一个 text 属性 → Modified 5，`ElementDelta::with_attribute_rows` 恰好列出这两行；其余属性行 0 变化；`relocated_only == false`。
- 用例 C（删除）：删 2 个叶元素 → Deleted 2 + owner Modified；被删记录旧地址仍可 `read_element_direct` 读到（页不复用，§4.5）。
- 用例 D（重命名）：改 NAME → `find_by_name(新名)` 命中该元素、`find_by_name(旧名)` 为 `None`；`NamedTable::iter` 与 `scan_elements(Named)` 的名字集合仍逐一相等（`tests/name_table_real.rs` 的口径）；重名写入被拒。
- 树：对新会话 `index_root` 走 `IndexCursor::walk`，键严格递增、无重复；对每张新页 `IndexNode::decode` 通过（`FreeSpaceNotEntryAligned` 等一个都不能出）；对全部键 `search_index` 命中且 `(page, offset)` 与 `scan_elements` 一致；未改动元素的 `(page, offset)` 与会话 281 逐一相同（共享旧页）。
- 字节：`assert_prefix_unchanged` 同阶段 1；新页连续；每张新页第 0 字 ∈ {3, 5, 7}。
- 语料门与阶段 1 同；再加 `tests/session_diff_real.rs` 里的 `changed_elements` 走一遍。
- 人工门（可选）：E3D 3.1 打开副本，`q att` 新 BOX、`q sessions`；若 E3D 拒开或报 INDEX 错，把现象与页号记进 §7。

### 阶段 3 · 多写三方合并（A3a/A3b；目标：base ≠ latest 时也能存）

**范围**：`write::merge`：`sub_5B10410` 的冲突扫描、`sub_5B0C090` 的元素合并、`sub_5B0D080` 用户表合并、`sub_5B07080` INDEX 去重、`sub_5B0ED40`/`sub_5B0D950` 属性级三方、`sub_5B1FF60`/`sub_5B20CA0` 成员表逐成员；不做 CLAIM。

做什么（口径全按算法清单 A3a/A3b，这里只列实现上的映射）：
1. 三个视图 = 三个 `DbView`：base（`open_at(base_sesno)`）、latest（`open`）、working（阶段 2 的 `Transaction` 暂存 = base + 我的改动）；new = 从 latest 的树根出发的 CoW 事务。
2. **冲突扫描** = `IndexDiff(base, latest)` 与 `IndexDiff(base, working)` 两次两根 diff（现有）：别人删 ∩ 我改 → 562；我删 ∩ 别人改 → 590；我新建的 refno 在 latest 已有 → 590/撞号；NAME 表：我新起的名在 latest 已归别的元素 → 561。任一条 → 返回带全部冲突项的 `Err(MergeConflicts)`，**不写任何页**。
3. **元素合并**：对 `IndexDiff(base, working)` 的每个键：Deleted → new 里删；Inserted → 把 working 的记录复制进 new（+ 挂 owner 成员表）；Modified → 若 latest 里还在：属性级三方（下）+ owner 变更 + 成员表逐成员；latest 里已删且未触发 562 → 丢弃。
4. **属性级三方**：对 `ElementDelta(base, working)` 列出的每个变化属性：working == latest → 跳过；否则 new 取 working 的值（两人都改我的赢）；清空 → 复位默认（写 `UNSET_SENTINELS` / 删 explicit 项）；类型 15 唯一键撞 → 抢占（删占用者的键再写）；类型 18 按条目集合合并。
5. **成员表**：实现 `sub_5B1FF60` 的有序差异迭代器（增/删/移 + 前一个共有兄弟锚）；`kind==2` 的 noun 整段覆盖（kind 来自模板字 3——若拿不到，阶段 3 一律走逐成员，标记推断）。
6. **INDEX 去重**：合并后对 `IndexDiff(latest, new)` 的 Modified 键，记录字节与 latest 相同 → 叶值指回 latest 的地址（不写副本）。
7. 会话页第 508 字 `BASESN` = base sesno（不是 latest）。

**验收**（`tests/write_stage3_merge.rs`）：
- 构造并发：C0 = `ams8000_0001` 副本（会话 S）；写者 A 在 C0 上用阶段 2 写出 S+1（改元素集 X、新增 Y、删 Z）；写者 B 以 S 为 base 暂存改动（改 X'、新增 Y'、删 Z'，与 A 不相交）→ 合并写出 S+2。
  - `diff_sessions(S, S+2)` 的键集 = A ∪ B；`diff_sessions(S+1, S+2)` 的键集 = B；A 的每个值仍在，B 的每个值都在。
  - 同一元素 A 改属性 p、B 改属性 q → S+2 两者都有；同一属性两人都改 → B 的值；B 清空 → 默认值。
  - 成员表：A 在 ZONE 里加 m1、B 加 m2 并把 m0 挪到末尾 → S+2 的成员序 = 按锚规则推出的期望序列（用例里写死）。
  - 冲突：A 删 e、B 改 e → 拒绝，报 562 类冲突项 e，文件字节与 C1 全等；A 改 e、B 删 e → 590；B 新起名 = A 已用名 → 561。
  - 去重：B 改了 e 又改回原值 → S+2 里 e 的地址 = S+1 的地址（没写副本）。
- 字节/链/语料门同前两阶段；`session().tail.base_sesno == S`（B 的 base）。

---

## 3. 每次写入都必须成立的不变量（三阶段共用断言）

1. 旧页零改动：写前文件的 `[0, old_end]` 页中，除页 0 的第 9/10/11/94/96 字外逐字节相同。
2. 新页连续：新页号 ∈ `(old_end, new_end]`，无洞；`new_end` = 新会话页第 5 字 = 新会话页自身或其后的最大数据/树页（core 里会话页是最后一页——我们也把会话页分配在最后）。
3. 页 0 是唯一提交点：中间态文件（新页齐、页 0 未写）读出的仍是旧会话。
4. 新会话页链回旧会话页；`walk_sessions()` 数量 +1，sesno 严格递减到 1。
5. 元素记录不可变：任何旧 `(page, offset)` 的字节不变；Modified 的键地址一定不同（除去重命中）。
6. 每张新树页过 `IndexNode::decode`；键有序；每个键可路由。
7. 大端；页大小取自页 0 `PGSIZE`，不写死 2048。

---

## 4. 风险与未定

| 风险 | 影响 | 处置 |
|---|---|---|
| 会话页里几个字的语义靠推断（第 4 字 OWNSES、第 505 字用户/桶号、第 11 字标志、第 506–510 字） | E3D 打开副本时可能校验 | 阶段 1 一律「抄旧会话页 + 改已知字」；人工门记录 E3D 反应 |
| 页 1 桶表不更新（A11 `2UP1UOR`） | 多用户场景下 E3D 可能认为该用户没登记 | 单用户副本验证；写进 §7 观察项 |
| 3SN 分裂策略未读 | 树形状与 core 不同 | 读侧不依赖；只保证不变量 6 |
| 类型 19/23、hunk 20–23 的写法 | 含这些属性的元素无法整条重写 | 阶段 2 遇到即报错；语料里无设值的 19，20–23 只在 explicit 外联块 |
| 删除元素不更新 TYPEX/CATR 等用户表 | 反向引用表悬空 | 阶段 2 记 §4；阶段 3 的 `sub_5B0D080` 口径补齐 |
| 属性 7/10/12/17 等控制块语义未反证 | 页 0 第 14/15 字与属性 17 分支的写法可能与 core 不同 | 不写第 14/15 字；属性 17（claim）不支持 |
| 与运行中的 E3D 并发打开同一文件 | 无哨兵锁时可能互相覆盖 | `win-lock` feature + 提交前重读页 0 的乐观检查；文档明说「先 FHSWIT READ」 |
| E3D 2.1 文件 | 未验 | 页 0 `template_version` / DBVRSN 判断，非 3.1 直接拒 |

---

## 5. 决策点（评审时定；括号里是不回复时的默认值）

- **Q1 目标格式**：只做 E3D 3.1 单 extent（默认：是；ext≠1、多 extent 文件拒写）。
- **Q2 OWNSES / 第 505 字**：抄旧会话页（默认）还是写常量 `0xFFFFFFFF` / 0？
- **Q3 DBVRSN 8 字节**：写自有标识便于事后识别 e3d-io 写的会话（默认：`e3dio` + 3 位版本，如 `e3dio001`），还是抄上一会话的 `2_35_23o`？
- **Q4 页 0 第 14/15 字**：core 在属性 10==1 且 12≠1 时写属性 7；我们不维护属性 7 → 不动（默认）。
- **Q5 3SN 分裂点**：中位（默认）。
- **Q6 锁**：Windows 下实现哨兵字节 `LockFileEx`（feature 默认关，测试里开）+ 提交前重读页 0 乐观检查（默认都做）。
- **Q7 CLAIM**：阶段 3 是否连 claims 一起做（默认：不做；属性 17≠0 的库拒写）。
- **Q8 人工门**：每阶段结束是否用 E3D 3.1 打开副本做一次人工检查（默认：做，结果记 §7，不阻塞下一阶段）。

---

## 6. 模块与 API 草图（评审后再定名）

```
src/write/mod.rs          pub struct Transaction  { open(path) -> Result; put_element(..); delete_element(..); set_attribute(..); commit(user, comment) -> Result<CommitReport> }
src/write/commit.rs       页分配器、刷盘顺序、页 0 提交、崩溃钩子（cfg(test)）
src/write/session_page.rs SessionPageBuilder：按 §2 表逐字填，逆 `SessionPage::parse_as`
src/write/index_cow.rs    Cow 路径复制、叶插/改/删、分裂、根登记；逆 `IndexNode::decode`
src/write/record.rs       RecordEncoder：逆 `ParsedElement::parse_as` + 描述子放值；kind 1/2 块
src/write/merge.rs        阶段 3：冲突扫描 / 元素合并 / 属性三方 / 成员表迭代器 / INDEX 去重
src/page/io.rs            PageWriter（读写句柄、整页大端写、sync）；PageReader 不变
```

读侧一行不改；写侧每个模块都以「对应读侧的逆函数」为设计约束，并用读侧做单测的断言方。

---

## 7. 观察记录（随阶段推进填写）

- 2026-09-20：计划起草；worktree `write-path` 建于 `7a51c95`，算法清单入库为 `bb315af`。
- 2026-09-20：**阶段 0 + 1 落地。** 新模块 `src/write/{mod,pages,session_page}.rs`（`PageFile` 读写句柄、`build_session_page` = `SessionPage::parse_as` 的逆、`stage_empty_session` → `Staged::commit` 两步提交、`rollback`、提交前重读页 0 的乐观检查 `WriteError::Moved`）；读侧只把 `session::at` / `meta::descriptor::at` 两个偏移表放成 `pub(crate)` 并加了 `PRISES` 常量，其余一行未动。脚手架 `tests/common/write_fixture.rs`（`DbCopy`、`assert_prefix_unchanged`、`assert_appended_contiguous`）。
  验收 `tests/write_stage1_empty_session.rs` 6 项全绿：`ams8000_0001`（**当天已是会话 640、16 476 页**，非计划里写的 281）→ 641，`ams5100_0001` 27 → 28；`walk_sessions` +1 且旧链原样、`diff_sessions(old, new)` 三态全零、前 10 个元素地址不变、旧页字节只有页 0 第 10 字变（第 9 字本来就是 −1、第 11 字仍是 1）、新文件恰多一页且类型 3；中间态（新页已写、页 0 未写）读出的仍是旧会话；`rollback` 后字节全等；页 0 被别人先改 → `Moved` 拒写。全套 `cargo test` 273 + 集成全部通过；clippy 对新文件零告警。
  改动了两处计划口径：页 0 第 94/96 字不再清零（E3D 刚写的样本第 94 字 = 2）；会话页 `DBVRSN` 写 `e3dio001`（Q3 默认）。人工门（E3D 打开副本）未做。
- 2026-09-20：**阶段 2 前置落地——NAME 表只读游标 `index::named`。** 只读，读侧其它模块未动；`ReadOnlyEngine::name_table()` / `find_by_name(name)` 是入口，`NamedTable::{lookup, lookup_name, iter}` 是游标。
  从样本实读的布局：页类型 5、页头第 3 字 `key_dwords = 0xFFFFFFFF`（算法清单 A3a.2 写的「key 宽 = 0」不准确——文件里是 −1，core 在 `sub_5B00F10`/`sub_5B010F0` 第一行按有符号 `<= 0` 判变长）、第 4 字 `value_dwords = 2`；条目 `[键长][键字×键长][值 2 字]`；键 = 带 `/` 的名字 4 字节/字 NUL 补齐；叶值 = refno、内结点值 = `(页, ext)`（样本全是 ext 1）。**哨兵按首字认**：`ams8000_0001` 根页 16306 的哨兵是 1 字 `0x80000001`，但 `TES6804_0001` 第 49875 页（level 1）的哨兵是 **7 字**——首字 `0x80000001`、后 6 字还是被它覆盖前那条名字的尾巴（`.81A/45&~!cHHQ9MdM7 &:50`）；core 的比较把 `0x80000001` 当成小于一切的字，所以后面几字从不参与比较。第一版按「键长 1 且等于哨兵字」认哨兵，全语料走查在这一页报 `KeyOutOfOrder`，改成按首字认后 1 002 张表全部通过。
  键序按 `3STS`（`sub_5B02B80`）的两个比较例程实读：`sub_5B00F10`（内结点路由，线扫、留最后一个 ≤ 目标的条目）与 `sub_5B010F0`（叶定位）——变长分支逐字**无符号**比较（`unsigned int v23 > v24`），`0x80000001` 两侧特判为最小，`min(两键长)` 字全等时短键在前、等长即相等；定长分支（refno 表）用的是二分。UTF-8 名字里字首字节 ≥ 0x80 的字因此排在 ASCII 之后（有符号比较会反过来），`compare_keys` 照此实现并有单测。
  验收 `tests/name_table_real.rs`：`ams8000_0001` 1 834 个名字、`ams5100_0001` 221 个——`NamedTable::iter` 严格递增、无重名；与 `scan_elements(Named)` 收到的元素 NAME（补 `/`）**集合逐一相等**、refno 逐一相等；每 1/400 取样点查 `lookup_name` 命中且读页数 ≤ 树高；不存在的名与大小写不同的名 → `None`；`find_by_name` 一调即得。全语料门（`--ignored`）：`D:\AVEVA\Projects\E3D3.1` 下 1 002 张 NAME 表、13 565 455 个名字全部解码、按无符号序全部严格递增（87 s）；其中 168 对相邻键的首个不同字高位不同——有符号比较会把它们颠倒——它们在树里的实际顺序就是无符号的，与 core 代码读出的口径互证。
