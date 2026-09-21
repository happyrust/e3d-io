# 计划：给 e3d-io 加 DABACON 写入（分三阶段，每阶段在副本库上写、用现有读侧往返验收）

状态：草案，待评审（2026-09-20）；阶段 0/1、阶段 2（用例 A/B/C/D = 增/改/删/命名）、阶段 3（三视图冲突扫描 + 不相交合并 + A3b 属性级 + A3b.4 成员级含 st 2 移动重排与 `kind==2` 整段覆盖档 + INDEX 去重）均已落地并通过 §2 验收，两段 stage/commit 已铺到全部写路径；TYPEX 与类型 16 反向引用表、CLAIM 主元素会话索引随写维护（给模板即开，§7 反向表/CLAIM 条）；单条记录跨页续接已补（阶段 2b，§7 跨页条）；extract flush（第 4 槽）冲突保留提交侧的合并变体已补（§7 extract flush 条）；类型 19 串列表（inline）与 hunk 20–23 外联块（int/real/text/串列表的记录外形态）的写已补（阶段 2c，§7 hunk 条）；`sub_5B20CA0` 对写者所加成员的门（提交侧删了不加；第 4 槽按 OWNREF 比 base↔latest）已补（§7 2026-09-21 A3b.4 收尾条）；**Q8 E3D 人工门已 PASS，且 2026-09-21 起改为无人值守 TTY（`tools/e3d/write_gate.ps1`，10 s 一轮）——BOX、引用成员 GROU、hunk 20/21/22/23（含跨页链）、类型 19 串列表全部由真 E3D 3.1 读回**（§7 末条，`docs/evidence/2026-09-21-e3d-tty-gate-write-path.md`）。§5 的八个决策点按默认值执行；余下见各条「未做」
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

属性号 → 控制块字段语义（算法清单 §6 第一条）、会话页第 4 字 OWNSES 与第 505 字的取值、`DBVRSN` 8 字节写什么、页 1 桶表是否必须更新、3SN 分裂策略。类型 19/23 与 hunk 20–23 的写法已从 `4SCEA`/`sub_5AEC110`/`sub_5AEA500` 读实并落地（§7 hunk 条）。

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
1. **数据页**：类型 7，bump 分配（A2.1）：记录整条写到「当前输出页」剩余空间，不够就开新页；记录位置 = `(pageno, ext=1 | (字偏移 << 13))`。记录跨页 = 块级续接（读侧 `record::block` 的 `continues_at`）：**已补**（阶段 2b，2026-09-20，见 §7 跨页条）——头+implicit 连续放在一页（自身跨不了页，超一页 → `RecordTooLong`），成员块/explicit 块各自按页切成若干段、段头写 `(页, packed)` 续接到下一段，头两槽回填到各块首段落处。
2. **记录编码**：从 `ParsedElement` + 描述子逆向：implicit 区按模板 `encoded_location` 放值，类型宽度用 `StorageShape`；explicit 块 `[hash][type<<26 | words][data]`（读侧按两个 u16 半字解成 `type<<10` 与 `words`）；成员块 kind 2、explicit 块 kind 1，块头 `[kind:u16][words:u16][owner][cont_pg][cont_packed]`。类型 19（串列表）inline 写为 `[n]{[i][字数][字节数][载荷]}*`、头 `19<<26 | 总字数` 无独立计数字；类型 20–23（hunk）在流里只放 7 字目录项 `[hash][type<<26|5][1][页][packed][元素数][字数]`，值本体写成 kind 3 外联块链（块头 7 字 `[3|words][refno×2][cont×2][hash][type]`），**已补**（阶段 2c，见 §7 hunk 条）。
3. **INDEX CoW**（A2.2/A5）：对每个改动键，沿根到叶的路径复制页（新页号从分配器取——**我们是单写者，直接分真实页号，不走 core 的临时页/重编号两步**；结果等价，写出的树与 core 的一样只引用 `≥ old_end+1` 的新页和未改的旧页），叶插/改/删条目并维护 `free_dwords`；满页按 3SN 分裂——分裂点取中位（§5 Q5，读侧不依赖策略）；根变则新根进会话页第 7/8 字。内结点键 = 子树最小键（读侧 `choose_child` 的规则）；哨兵条目 `0x80000001` 的处理照读侧 `leftmost_child`。
4. **用户表**：改了 `NAME` 的元素同步更新 NAME 表（类型 15 唯一索引，A3a.4/§5 表 15 行）：键 = 打包名字（变长键），值 = refno；重名 → 报错（不做抢占）。**读侧游标已补**（`index::named`，2026-09-20，见 §7）：NAME 表页头 `key_dwords = 0xFFFFFFFF`（core 判 `<= 0` 有符号即变长）、`value_dwords = 2`，条目 `[键长][键字×键长][值×2]`，键 = 带 `/` 的名字 4 字符/字 NUL 补齐，哨兵 = **首字** `0x80000001` 的条目（键长随它覆盖掉的旧键，写侧造页用 1 字即可），叶值 = refno、内结点值 = (页, ext)；键序 = 按字**无符号**比较、`0x80000001` 小于一切、前缀短者在前（`sub_5B00F10`/`sub_5B010F0` 实读）；路由 = 最大的 ≤ 目标键。写侧照这个布局造页，用 `NamedTable::iter` / `lookup_name` / `ReadOnlyEngine::find_by_name` 验收。**TYPEX 与反向引用表（CATR/SPRE/ISPE…）已补**（`write::keyed_cow` + `write::reverse`，2026-09-20，见 §7 反向表条）：给了模板（`WriteOptions.templates` / `MergeOptions.templates`）就按 core `sub_5AB91B0`/`sub_5AB3620` 的口径维护——新元素按 noun 模板补 TYPEX 字段与 `[noun, refno, 0]` 键；类型 16 引用属性的非空值记 `[目标, 自身]` 到以属性名命名的表（没有就建、追加进表目录）；改值删旧插新、删元素撤全部键。没给模板照旧原样保留根。**CLAIM 树（`(5,7618377)` 主元素会话索引）也已补**（`write::claim_cow` + `write::claim` + 读侧 `index::claim`，2026-09-20，见 §7 CLAIM 条）：同样给了模板才维护，按 core `sub_5B0C760` 的口径——写到的每个主元素（noun 模板字 4≠0）刷会话号、新主元素记 `(0, 会话)`、删主元素撤条目、非主元素冒到最近的主元素祖先刷号。
5. **提交**：分配顺序 = 数据页 → 树页（自底向上）→ 会话页，与 A5/A6 一致；页 0 同阶段 1。

**不做**：多 extent（ext≠1 一律报错）；带值的键控表（SPAMAP，key 4/val 7）与 UDA 表（UDATAB/UDAFTB/UUIDAR）不维护。

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
| 类型 19/23、hunk 20–23 的写法 | 语料里没有一个设了值的 19 或任何 hunk，口径只来自 core.dll 反编译 | **已落地**（阶段 2c，§7 hunk 条）：布局从 `4SCEA`/`sub_5AD5060`（写）与 `sub_5AB4CF0`/`sub_5AEA500`（读）两侧对读一致；hunk 是否外联由调用方明确选（core 自身按 `DB_Attribute::initialised` 运行态定，阈值 `0x2000` 字记为常量）；余下风险 = 没有 E3D 写的样本可逐字比对，只能自写自读 + 等真引擎认 |
| ~~删除元素不更新 TYPEX/CATR 等用户表~~ **已补**（2026-09-20，§7 反向表条） | 给了模板即按 core 口径维护；没给模板时新元素按类型查不到、引用无反向项 | 调用方给 `WriteOptions.templates`；带值的键控表（SPAMAP）与 UDA 表仍不维护，见下 |
| SPAMAP（空间索引）不维护 | 新/改元素不进空间索引 | 其 7 字值 = 世界系 AABB + 标志，靠几何算而非记录镜像（§7 调研条），写侧无几何引擎、硬写会塞错值比不写更糟；E3D 可重建空间图。留待移植几何后 |
| UDA 表（UDATAB/UDAFTB/UUIDAR）不维护 | 带这类 UDA 的元素其 UDA 表悬空 | 装机语料无带这些表的库可坐实键/值形与验证（ams8000 无此三表），e3d-io 现写的元素不带此类 UDA。留待有带 UDA 表的项目库 |
| 属性 7/10/12/17 等控制块语义未反证 | 页 0 第 14/15 字与属性 17 分支的写法可能与 core 不同 | 不写第 14/15 字；属性 17 的**开关**（extract 模式的建/删 CLAIM 表 `sub_5B13670`）不支持——但属性 17==0 的普通单写者存盘里 `sub_5B0C760` 无条件维护的 `(5,7618377)` 主元素会话索引已补（§7 CLAIM 条）；attr17≠0 的 CLAIM 表搬运（A3a.7）仍不支持 |
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
- 2026-09-20：**阶段 2 用例 A 落地——新增 3 个 BOX + 改 ZONE 成员表，走记录编码 + INDEX 影子分页。** 新模块 `src/write/record.rs`（`RecordDraft`：`ParsedElement`/`read_record` 的逆，`encode`/`from_record_bytes`/`push_member`/`put_real_wide`/`put_real_vec_wide`）、`src/write/index_cow.rs`（`rewrite_index`：批量插入/改值的 CoW），入口 `write::write_element_session(path, request, &[ElementWrite{draft, insert}])`。读侧一行未改（复用 `IndexNode::decode`/`choose_child`/`cmp_refno`）。
  - **坐实一条算法清单没有的事实：记录头第 10 字（存储格式字）= `0x20000000 | (成员数×2) | (explicit 流字数 << 14)`**——低 14 位 = 成员表载荷字数，14..28 位 = explicit 流字数（`Σ(2+data_words)`，不含块头），bit 29 = wide 布局。用 e3d-io 全扫 `ams8000_0001` 的 6 639 个元素**逐一吻合**（探针在 `%TEMP%\s2probe`，未入库）；全部为 wide。读侧不看这个字（它跟着两条槽地址走），但 E3D 看，所以照写。
  - 记录布局：`encode` 把 implicit（含 11 字头）→ 成员块（kind 2）→ explicit 块（kind 1）**连续摆在同一页**，头字 6/7=explicit 槽、8/9=成员槽，`packed = ext | (4字节字偏移 << 13)`（`sub_5AE9DF0` 的写法）；读侧 `read_record` 照槽地址回收，与 core 写的记录同一条路读回。新 BOX 未命名（explicit 槽 0）→ **不动 NAME 表**；起始值从 BOX 模板的 `defaults.implicit`（26 字）铺底，再按 wide 偏移覆盖 POS/XLEN/YLEN/ZLEN。
  - INDEX CoW（`rewrite_index`）：把每条记录的新叶值（插入或改值）批量套到树上，只复制根→受影响叶的路径页、其余子树原样留旧页号，新页号从 `end+1` 顺序取。**插入不改分隔键**——内结点键是子树最小键，`choose_child` 把键送到「≤ 它的最大分隔键」的孩子，故新键不会低于所在叶的现有最小键，路径上只有页号往上传；满页按中位分裂，根裂了就在其上新起一根。单写者直接分真实页号，不走 core 的临时页两步（结果等价）。
  - 分配顺序 = 数据页 → 树页 → 会话页（末页，写新 INDEX 根），再写页 0（LSPGNO），与阶段 1 同的乐观复检（`Moved` 拒写）。`ElementWrite.insert=false` 表示元素已在索引里、记录搬家 → 叶值改指新址（= core 记「改动」的方式，A2/§4.5）。ZONE 更新即用此路：读它的记录 → `from_record_bytes` → `push_member` ×3 → 以 update 交回。
  - 会话页构造扩了一个字段：`NewSession.index_root`——阶段 2 写新根，阶段 1 仍抄旧根（CLAIM 根永远抄旧）。
  - **两处口径**：BOX 未命名，TYPEX/CATR 等反向表本阶段不维护（新 BOX 按类型查不到，§4 已列）；元素路径暂**未暴露** stage/commit 两段句柄（`write_element_session` 一次成型，页 0 仍做提交前复检）。
  验收 `tests/write_stage2_elements.rs`（`ams8000_0001` 副本，取第一个 ZONE、3 个未用 refno = 同 word0 + 高 word1）：`diff_sessions(S, S+1).tally()` = **删 0 / 增 3 / 改 1**；每个 BOX `find_element` 得 noun BOX、owner = ZONE，`extract_element_with_descriptors` 读回的 POS/XLEN/YLEN/ZLEN 与写入**等值**；ZONE 成员表末尾恰好多出这 3 个、顺序如写入、旧成员原样；全库 `scan_elements` 计数 +3、键严格递增、3 个新键 `search_index` 命中；字节前缀除页 0 第 9/10/11 字外不变、新页连续且类型 ∈ {3,5,7}。全套 `cargo test` 283 单测 + 集成全绿（含 `index_cursor_real` 的两条「一处实现」守卫——CoW 复用同一 decoder/router，守卫允许名单加了 `write/index_cow.rs` 并写明理由）；clippy 新文件零告警；cargo doc 无新告警。
- 2026-09-20：**阶段 2 用例 B/C 落地——改属性、删元素。** `index_cow` 的 `LeafEdit` 由 `insert: bool` 换成 `EditKind{Insert, Update, Delete}`；`ElementWrite` 换成枚举 `{Insert(draft), Update(draft), Delete(refno)}`；`RecordDraft` 加 `remove_member`、`set_real_wide`、`set_explicit_text_ascii`（后者把 explicit 流按 `[hash][type<<16|字数][data]` 拆项、只换目标项的文本、其余原样重排；仅 ASCII，UTF-8/escaped 两种 FORMAT 都存自身字节）。删除只从 INDEX 摘键、不写记录——旧记录字节原样留盘（§4.5 页不复用），`read_element_direct(旧址)` 仍读得回；摘键不下调分隔键（分隔键只会变得偏小，`choose_child` 路由不误——被删键落回本叶查空 = `None`，活键不会错投，已论证）。
  - **用例 B**（`tests/write_stage2_modify_delete.rs`，5 个 SUBS）：每个改 `LENG`（implicit real，wide@29，+1000）与首个非 NAME 的 explicit 文本（→ `E3DIO-EDIT-i`）。`diff.tally()` = **删 0 / 增 0 / 改 5**；每个元素的 `ElementDelta`（`of_as` + `with_attribute_rows`）：owner/成员不变、`explicit_changed` **恰好一条** = 那个文本 hash→新值、`implicit_words_changed` 全落在 `{29,30}`、`attribute_rows` 的 `LENG` 行 after = `Real(旧+1000)`；重开 `extract` 读回 `LENG` 等值；元素计数不变、键序不变。
  - **用例 C**（同文件，删 2）：扫出一个 noun=BRAN、且有 ≥2 个 0 成员子元素的 owner，取前两个（且确认在 owner 成员表里）→ `Delete`×2 + owner `Update`（`remove_member`×2）。`diff.tally()` = **删 2 / 增 0 / 改 1**；两个被删键 `find_element` = `None`、`read_element_direct(旧 page/offset)` 仍读回旧记录（refno 对上）；owner 成员表少掉这 2 个、其余原样；元素计数 −2、键序不变；字节前缀除页 0 提交字外不变、新页连续类型 ∈ {3,5,7}。
  - 全套 `cargo test` **284 单测 + 集成全绿**（`index_cow` 加了删除单测：删中间键留两端、删不存在的键报 `DeleteMissing`）；clippy 新文件零告警；cargo doc 仍原有 24 条。
- 2026-09-20：**阶段 2 用例 D 落地——写 NAME、维护 NAME 表（变长键 CoW）。** 新模块 `src/write/named_cow.rs`（`rewrite_named_table`：对 `NAME` 形状的表做插入/删除的 CoW），`ElementWrite` 之外加 `NamedEdit{key, refno, kind: Insert|Delete}`；`write_element_session` 多收一个 `&[NamedEdit]`，元素记录改完后再对 NAME 表跑一遍 CoW，把新根写进会话页表目录。
  - NAME 表页与 `index::named` 同形（页类型 5、`key_dwords=0xFFFFFFFF`、`value_dwords=2`、条目 `[键长][键字][值 2]`、哨兵按首字 `0x80000001`、`compare_keys` 序），CoW 复用 `index::named` 的 `compare_keys`/`encode_name`/常量；与定长的 `index_cow` 唯一的差别是「满页」按**字预算**（`page_words−7`）而非固定条数，分裂按字预算切。插入不改分隔键、删除留旧分隔键（同 `index_cow` 的论证）。哨兵一律写成 1 字（读回时后面几字本就不参与比较）。
  - **改名 = 删旧键 + 插新键**（refno 不变）；**重名即 core 的 561**——插一个已存在的键报 `NamedCowError::Duplicate{existing}`，`write_element_session` 冒成 `WriteError::NameTable(..)`，且**发生在写盘之前**（所有 CoW 都在内存里攒页、最后才落盘），所以拒写时文件一个字节都没动。会话页构造扩了 `NewSession.table_overrides`——按 name hash 在表目录里改那一条的根页/extent（其余表原样）；只改名不动元素时 `edits` 为空则 INDEX 根原样、不复制树。
  - 验收 `tests/write_stage2_naming.rs`：**改名 3 个**已命名元素（NAME 属性 → `/E3DIO-RENAMED-i`）——`diff.tally()` = 删0/增0/改3；每个 `find_by_name(新名)=refno`、`find_by_name(旧名)=None`、记录里 NAME 读回新名；`NamedTable::iter` 的 `(名, refno)` 映射与 `scan_elements(Named)` 逐一相等、名字总数不变；字节前缀除页 0 提交字外不变、新页连续类型 ∈ {3,5,7}。**重名拒写**：让 X 去占 Y 的名（`elements` 空、`name_edits` 仅一条 Insert）→ `Err(NameTable(Duplicate{existing==Y}))`，副本字节与原始**全等**。
  - `named_cow` 单测：小树上插入按序落位、删除移除、重名报 `Duplicate`、叶溢出分裂后仍有序并长出新根。全套 `cargo test` **287 单测 + 集成全绿**；clippy 新文件零告警；cargo doc 仍原有 24 条。仍未做：给未命名元素**新增** NAME（`set_explicit_text_ascii` 只换已有项，尚不能往 explicit 流里加项）；TYPEX/CATR 反向表；跨页记录；阶段 3 多写合并。
- 2026-09-20：**阶段 3 起步——三视图冲突扫描 + 不相交合并写。** 新模块 `src/write/merge.rs`：`write_merge_session(path, request, base_sesno, elements, name_edits)`——先按 core 的 `DOPENW` 三视图对齐（**base** = 写者打开时的会话、**latest** = 已提交的最新会话、**working** = base + 写者改动）跑冲突扫描，干净才把改动写在 latest 之上、会话页 `BASESN` = base（A3a.8）。
  - 冲突扫描（`scan_conflicts`）：`IndexDiff(base_root, latest_root)` 得「别人（已提交）改了什么」，写者改动按 `ElementWrite` 分成插/改/删；**562** = 我改 ∩ 别人删、**590** = 我删 ∩ 别人改（含我新建的 refno latest 已有）、**561** = 我起的名 latest 的 NAME 表已归别的元素（`NamedTable::open(latest).lookup`）。三视图 = 两个只读 `DbView`（base 用 `open_at(base_sesno)`、latest 用 `open`）+ 写者的 working 改动。任一冲突 → `Err(WriteError::MergeConflicts(Vec<MergeConflict>))`，**扫描在写盘前**、拒写时一个字节不动。
  - **本阶段只做「不相交」的合并写**：写者改动落在 latest 之上（复用 `write_session_with_base`，`BASESN` = base）——两边改动不相交时，结果即 A ∪ B。**两边改同一元素**（都是 modify）暂**拒写**（`code::OVERLAPPING_CHANGE` = 0，非 core 码），因为按属性/成员逐一调和（A3b：我赢 / extract flush 别人赢 / 列表按条目 / 成员按锚）还没做——宁可拒，不静默错并。
  - 验收 `tests/write_stage3_merge.rs`（构造 C0=会话 S → A 写 S+1 → B 以 S 为 base 合并写 S+2）：**不相交并**——A 在 zone[0] 下加 BOX_A + 改 subs[0] 的 LENG；B（drafts 从 `open_at(S)` 读）在 zone[1] 下加 BOX_B + 改 subs[1] 的 LENG；合并后 `diff(S, S+2)` = 增 2 改 4（两 zone 两 subs）、`diff(S+1, S+2)` = 增 1 改 2（只 B）、`BASESN`=S、A 的 BOX_A 与 LENG=1000 仍在、B 的 LENG=2000、B 的写只在 A 的 S+1 之后追加（前缀不变、类型∈{3,5,7}）。**四种拒写**：A 删叶 e / B 改 e → 562；A 改 e / B 删 e → 590；B 用已存在的名 → 561；A 改 e / B 也改 e → OVERLAPPING（本 crate 限制）；每种都 `Err(MergeConflicts)` 且副本字节与 A 写完后**全等**。
  - `write_element_session` 抽出 `write_session_with_base(.., base_sesno: Option<u32>)` 共用核心（普通写 base=前一会话，合并写 base=合并基）。全套 `cargo test` **287 单测 + 集成（含 5 条阶段 3）全绿**；clippy 新文件零告警；cargo doc 仍原有 24 条。仍未做（阶段 3 余下）：A3b 的属性级 / 成员级三方调和（两边改同一元素）、INDEX 去重（改回原值不写副本）、CLAIM。
- 2026-09-20：**阶段 3 推进——属性级三方调和（A3b）。** 新 `record::three_way(base, latest, working)`：把写者的改动**变基到已提交记录上**——从 latest 起，只把写者真正改过的（base→working 有别的）implicit 字 / explicit 属性（按 hash）/ 成员套上去；写者没碰的沿用 latest。所以「两边改同一元素**不同属性**」两个改动都在；改**同一属性**则写者赢（普通 SAVEWORK 的 A3b 口径，extract flush 才别人赢、本 crate 不做）。owner 写者改过则用写者的。成员暂用集合增删（latest + 写者增 − 写者删），A3b.4 的锚序还没做。
  - `write_merge_session` 冲突扫描通过后，对写者的每个 `Update` 读 base（`open_at`）与 latest（`open`）的记录、`three_way` 变基后再写；**「两边改同一元素」不再拒写**（`code::OVERLAPPING_CHANGE` 去掉），562/590/561 照旧。不相交时 base==latest、变基结果即 working，行为不变。
  - 验收：`record` 单测 `three_way_composes_disjoint_changes_and_prefers_the_writer_on_a_clash`（不同 implicit 字与 explicit 属性各自合上、同属性写者赢、只 latest 改过的属性保留）；集成 `two_changes_to_one_element_merge_attribute_by_attribute`（A 改 subs[0] 的字 11、B 以 base 改字 12 → 合并后两字都在、`diff(S,S+2)` 只这一元素改一次）。原「两边改同一元素拒写」用例改成合并用例。全套 `cargo test` **288 单测 + 集成全绿**；clippy 新文件零告警；cargo doc 仍原有 24 条。仍未做：成员级锚序调和（A3b.4）、INDEX 去重、CLAIM。
- 2026-09-20：**阶段 3 A3b.4——成员级三方调和（锚序）。** `three_way` 的成员合并从「集合增删」升级为 `sub_5B20CA0` 的锚序：从 latest 的成员表起，先去掉写者删的（base 有 working 无），再把写者加的（base 无 working 有）**按 working 顺序**插到「working 里排在它前面、且合并表里还在的最近一个成员（前一个共有兄弟）」之后（找不到就插最前）；两边都没动的成员保持 latest 的序。所以两边向同一 owner 加不同成员两个都在、各自锚在自己的前兄弟后。移动（st 2 同成员换位）暂不重排。
  - 验收：`record` 单测 `three_way_anchors_added_members_and_honours_removals`（base `a b c`、latest committed 在 a 后插 X、writer 在 b 后插 Y 且删 c → 合并 `a X b Y`）；集成 `two_writers_add_members_to_one_owner`（A 给 ZONE 加 BOX_A、B 以 base 给同一 ZONE 加 BOX_B → 合并后 ZONE 成员含**两者**、两 BOX 都入索引、`diff(S,S+2)` = 增 2 改 1）。全套 `cargo test` **289 单测 + 集成全绿**；clippy 新文件零告警；cargo doc 仍原有 24 条。仍未做：成员移动（st 2）重排、INDEX 去重、CLAIM、给未命名元素新增 NAME、TYPEX/CATR、跨页记录。
- 2026-09-20：**阶段 3 INDEX 去重（A3a `sub_5B07080`）。** 合并变基后，对写者的每个 `Update` 比 `merged` 与 latest 的记录：内容相同（`RecordDraft::content_eq`——noun/owner/成员/implicit 头以后/explicit 按字段，忽略头字与流内 padding）就**丢弃这条 Update**——不写记录副本、索引叶原样指向盘上已有记录。所以两个写者各自把某属性改成同一个值（收敛）、或改了又改回，合并后那个元素不产生新页。
  - 验收：`record` 单测 `content_eq_normalises_header_and_padding`（同内容经 encode→from_record_bytes 往返仍相等、改一个属性值即不等）；集成 `a_merge_that_converges_to_the_committed_value_writes_no_record`（A 把 subs[0] 字 11 改成 V、B 以 base 也改成同一个 V → 合并后 `merged==latest` 被去重：本次会话 `appended_pages` 只有 1 页（会话页本身、无记录/树副本）、`diff(S+1,S+2)` = 0/0/0、元素仍读 V 且停在 A 给的旧址）。全套 `cargo test` **290 单测 + 集成（7 条阶段 3）全绿**；clippy 新文件零告警；cargo doc 仍原有 24 条。仍未做：成员移动（st 2）重排、CLAIM、给未命名元素新增 NAME、TYPEX/CATR、跨页记录。
- 2026-09-20：**用例 D 补齐——给未命名元素新增 NAME。** `RecordDraft::add_explicit_text_ascii(hash, type_code, text)`：往 explicit 流里**插新项**（已有则替换），文本数据 `[字数][4 字符/字打包]`（与 `set_explicit_text_ascii` 共用 `ascii_text_data`）。命名一个未命名元素 = 给记录加 NAME 项（`type_code` 0x3C00 = FORMAT 1 文本，NAME 的形态）+ NAME 表 CoW 插入（复用 `write_element_session` 的 name-edits 路径），无新合并逻辑。
  - 验收 `tests/write_stage2_naming.rs::naming_a_previously_unnamed_element_adds_it_to_the_table`：取一个 `scan_elements(Named)` 里 name 为 None 的元素（确认无 NAME 属性），加 `/E3DIO-FIRST-NAME` + 表插入 → `find_by_name(新名)=refno`、记录里 NAME 读回新名、`NamedTable::iter` 与 `scan_elements(Named)` 映射逐一相等、字节前缀除页 0 提交字外不变、新页连续类型 ∈ {3,5,7}。全套 `cargo test` **290 单测 + 集成全绿**；clippy 新文件零告警；cargo doc 仍原有 24 条。用例 D 至此改名 + 新命名两半都齐。仍未做：成员移动（st 2）重排、CLAIM、TYPEX/CATR、跨页记录。
- 2026-09-20：**元素写路径补两段 stage/commit（对齐阶段 1）。** 把三处写路径的核心统一成 `stage_session_with_base(..) -> StagedSession`——写全部新页（数据/树/名字表/会话页）+ sync 后停下、页 0 备好但**不写**；`StagedSession::commit` 才做「重读页 0 → `Moved` 复检 → 写页 0 → sync」，`rollback` 截回 staging 前的长度。阶段 1 的 `Staged` 并入 `StagedSession`（`previous`/`staged`/`page_size`/`commit`/`rollback` 方法名不变，加 `appended_pages()`）；`stage_empty_session`/`write_empty_session`、`write_element_session`、`write_merge_session` 全走它。新增 `pub stage_element_session` 暴露元素写的两段句柄。
  - 验收 `tests/write_stage2_elements.rs::a_staged_element_write_reads_as_the_old_session_until_commit`：stage 一个 BOX 插入 + owner 更新（`appended_pages().len() > 1`，确有记录/树页而非只会话页）→ **中间态**（新页已写、页 0 未写）读侧仍是旧会话、BOX 未入索引、元素计数不变；`commit` 后会话 +1、BOX 入索引；另起一份 stage 后 `rollback` 字节全等原始。阶段 1 的 6 项、阶段 2/3 全部用例照旧全绿（`Staged`→`StagedSession` 方法名兼容）。全套 `cargo test` **290 单测 + 集成全绿**；clippy 新文件零告警；cargo doc 仍原有 24 条。
- 2026-09-20：**E3D 人工门（Q8）——未在本机自动跑，需人操作**。目的：外部验证——AVEVA 自己的读侧（E3D 3.1 Design）能否认下 e3d-io 写的会话。**为什么没自动跑**：E3D 打开的是**项目**（一组 DB + 项目环境变量 + 登录 + license），不是单个文件；本机此刻有 `des.exe`（`E:\reverse\e3d\shadow_e3d31_aps_all`）在跑，真装在 `D:\AVEVA\Everything3D3.1`；把我写的副本喂给它要么新建一个一次性项目、要么覆盖样本项目 `AvevaMarineSample` 的 `ams8000_0001`——前者要交互式项目管理、后者会改动真样本项目（正是读侧测试刻意回避的）；且要占 license、可能扰动在跑的 E3D 实例。这一步交互性+破坏性都在用户侧，不适合自动做。
  - **人工配方**（用户在自己的 E3D 里跑，建议用一次性项目副本、别碰生产）：① 拷一份 `AvevaMarineSample` 项目到新名字（或至少备份它的 `ams000\ams8000_0001`）；② 用 e3d-io 的写路径往那份 `ams8000_0001` 追加一个会话（`write_element_session` 加一个 BOX + owner 成员，或 `write_empty_session` 追加空会话——可写个几行的 bin 调库，或把 `tests/write_stage2_elements.rs` 的用例指向该文件的副本）；③ E3D 3.1 Design 打开那个一次性项目、进 DESIGN 模块，`Q SESSIONS`（或 `Q DB CURRENT SESSIONS`）与在新 BOX 上 `Q ATT`。
  - **PASS 判据**：`Q SESSIONS` 列出新会话，用户名 = 我写的 `USER`（如 `e3dio`）、日期为写入时刻、`DBVRSN`/引擎标签能认（我写 `e3dio001`，E3D 可能标为未知引擎但仍应能读）；新 BOX 的 `Q ATT` 给出写进去的 POS/XLEN/YLEN/ZLEN，owner 指向那个 ZONE。若 E3D 拒开、报 INDEX/一致性错、或某字校验失败——把库名、会话号、报错原文记回本节，据此回头核对会话页字段（§4 风险表里 OWNSES/第 505 字/页 1 桶表几项本就是推断/未维护）。
  - 已备好夹具：`src/bin/e3d-write-box.rs`（`cargo run --bin e3d-write-box -- <db>` 往库里追一个带 BOX 的会话、打印会话号/BOX refno/几何/开项目要跑的两条 Q），加一次性脚本 `%TEMP%\e3dio_gate\prepare.ps1`（E3D 没在跑才动手 → 备份 `ams8000_0001` → 跑 bin → 打印步骤）与 `restore.ps1`（还原备份）。bin 已在 `%TEMP%` 的 `ams8000_0001` 副本上验过能写出（会话 641、BOX 24384/31483 挂在 ZONE 24384/47 下），真样本未动。
  - **现状：PASS（用户已在 E3D 3.1 Design 里跑过）。** 真引擎打开带 e3d-io 追加会话的 `ams8000_0001`：`Q SESSIONS` 列出了那个新会话（用户 `e3dio`），新 BOX 的 `Q ATT` 给出写进去的 POS/XLEN/YLEN/ZLEN、owner 指向那个 ZONE——**AVEVA 自己的读侧认下了 e3d-io 写的会话与元素**。这是对整条写路径（阶段 1–3 写出的文件）最硬的外部验证：不止本 crate 读侧能往返，商用 E3D 也能读。Q8 人工门通过。
- 2026-09-20：**阶段 3 A3b.4 补齐——成员移动（st 2）重排。** 成员三方从「latest + 写者增 − 写者删」升级为 `sub_5B1FF60` 的完整有序差异迭代器 + `sub_5B20CA0` 的逐 st 落位：`record::member_list_diff(base, working)` 双光标同步走两表，相等同进；base 项不在 working → `Removed`（st 3）；working 项不在 base → `Added`（st 1）；两边都有但位置不同 = 移动，按 core 的**距离**判谁动了——`|base 项在 working 里的位置 − base 光标|` 与 `|working 项在 base 里的位置 − working 光标|`，前者大则 base 项进「待落位」等 working 光标到它时再出 `Moved`（st 2），否则 working 项当场出 `Moved` 并进「已处理」（base 光标到它时跳过）；一边耗尽后 base 余项 = 删（已处理的跳过）、working 余项 = 待落位的移、否则增。所以 `[p q r s]→[q r s p]` 是 p 一次移动而不是三次。`record::merge_members(base, latest, working)` 从 latest 表起按序回放：删 → 摘掉；增 → 已在则幂等、否则插到锚后；**移 → 从 latest 表里现在的位置摘出、再插到锚后**——锚一律是「working 里它前面、且合并表里还在的最近一个成员」，找不到就最前。两边都动同一成员 → 落写者放的地方（A3b 普通 SAVEWORK 口径）；写者把 X 挪到 Y 后而别人删了 Y → 锚退到前一个共有兄弟；写者挪的成员被别人从表里摘了 → **不复活**（推断：`sub_5B21C10` 先在 new 表里找该成员再挪段）。`RecordDraft::move_member_after(member, Some(anchor)|None)` 给调用方一个 REORDER 口径的编辑。
  - 验收：`record` 单测 `member_list_diff_classifies_adds_removes_and_moves`（`[p q r s]→[q r s p]` 一次 Moved(p,3)、`→[r p q s]` 一次 Moved(r,0)、`[p q r]→[r q p]` 两次 Moved(r,0)/(p,2) q 不动、`[p q r]→[p x r]` = Removed(q)+Added(x,1)、相同表为空）、`three_way_reorders_a_moved_member_after_its_anchor`（base `a b c d`、latest 在 a 后插 X、writer 把 d 挪到 a 后 → `a d X b c`）、`three_way_lets_the_writer_place_a_member_both_sides_moved`（latest 把 d 挪最前、writer 把 d 挪 b 后 → `a b d c`）、`three_way_move_falls_back_to_the_previous_shared_sibling`（writer 把 a 挪 c 后、latest 删 c → `b a d`；writer 把 c 挪最前、latest 删 c → `a b`）、`move_member_after_reorders_in_place`；集成 `tests/write_stage3_merge.rs::a_moved_member_lands_after_its_anchor_when_merged`：取一个 ≥3 成员的 ZONE（实跑命中 `24384/47`、126 个成员），A 给它追加 BOX 写 S+1，B 以 S 为 base 把首成员 `24384/48` 挪到末成员 `24384/2471` 后合并写 S+2 → 合并后成员表**恰等于** `base[1..] + [首成员] + [A 的 BOX]`（写者的移动落在锚后、A 的 BOX 保持提交时的位置）、BOX 仍在索引、`diff(S,S+2)` = 增 1 改 1。全套 `cargo test` **295 单测 + 集成全绿**（阶段 3 现 8 条），唯 `tests/session_tail_real.rs::a_merged_chain_bases_sessions_on_numbers_it_no_longer_retains` 红——`ams7000_0001` 今天 18:28 被 E3D（Q8 人工门那次开项目）重写、最新会话是 Saving Meta Data 无 base，活样本漂移，main 上同样红，与本提交无关、未动；clippy 新改文件零告警；cargo doc 仍原有 24 条。A3b.4 至此增/删/移三种差异都落地。仍未做：CLAIM（属性 17）、TYPEX/CATR 反向表、跨页记录、`kind==2` 整段覆盖档（模板字 3 尚未取到，一律走逐成员）。
- 2026-09-20：**阶段 3 A3b.4 收尾——`kind==2` 整段覆盖档，模板字 3 坐实。** 算法清单里「`kind` = 该 noun 模板字 3」原是推断；这次把 20 个 `schema/e3d31/binaries/*vir.dat` 的每个 noun 模板 payload 头 14 字做了分布统计：**字 3 只取 0/1/2 三个值**——`desvir.dat` 994 个可解 noun 里 0×347（全是不带成员的叶 noun：TP、WELD、NBOX、TEXT、POIN…）、1×646（拥有子元素的 noun：ZONE、SITE、PIPE、BRAN、EQUI…）、**2×1 = `GROU`**；`catvir.dat` 同样只有 `GROU` 为 2；`sysvir.dat` 为 2 的是 **`MDB`、`USLI`**；dic/pad 没有 2。为 2 的恰是成员表装「别处拥有的元素的引用」的 noun（组、MDB 的库列表、用户列表）——与 core 把这一档成员表当成一个属性整段覆盖的做法互证，推断升级为语料坐实。落地：
  - `record::template::MemberListKind { Unknown, None, Owned, Referenced, Other(n) }`，`NounTemplate.member_list_kind` 从 payload 字 3 取（`new`/`from_attlib_noun_map` 得 `Unknown`）。
  - `write::record::MemberMerge { MemberByMember, WholeList }` + `three_way_with(base, latest, working, mode)`；`three_way` = `MemberByMember`。`WholeList` = `sub_5B1F660`：写者动过成员表（base≠working）→ 整表换成写者的（别人在 latest 的增删丢，同「同一属性写者赢」）；没动 → 留 latest 的。
  - `write::merge::MergeOptions { templates: Option<&mut TemplateProvider> }` + `write_merge_session_with(.., options)`；`write_merge_session` = 默认选项（无模板 → 一律逐成员）。给了模板时按 latest 记录的 noun 查 `member_list_kind`，`Referenced` → `WholeList`，其它（含模板里没有该 noun）→ 逐成员；模板读取失败 → 新变体 `WriteError::Template`。
  - 验收：`record` 单测 `three_way_whole_list_takes_the_writers_list_when_it_changed`（base `a b`、latest 加 c、writer 加 d：WholeList → `a b d`、MemberByMember → `a b d c`；writer 没动 → `a b c`）；`template` 单测 `member_list_kind_comes_from_header_word_3`（0/1/2/7 → None/Owned/Referenced/Other(7)，`new`/`default` → Unknown）；集成 `tests/write_stage3_merge.rs::a_reference_list_noun_merges_its_members_whole`：先断言 desvir 里 GROU=Referenced、ZONE/BRAN=Owned、WELD=None；语料里没有 GROU（E3D3.1 全部 <200 MB 的库扫过 0 个），所以在 ams8000 副本上自己建 WORL→GPWL→GROU（GROU 引用两个 SUBS）写 S，A 给组加 zone0 写 S+1，B 以 S 为 base 给组加 zone1 合并写 S+2——**带模板**合并后成员 = `[s0 s1 z1]`（A 的 z0 丢，整段覆盖）、**不带模板**同一构造 = `[s0 s1 z1 z0]`（逐成员）；被引用元素仍各归其 owner、组仍归 GPWL、`diff(S,S+2)` = 改 1。全套 `cargo test` **299 单测 + 集成全绿**（阶段 3 现 9 条；`session_tail_real` 的 ams7000 活样本漂移一条照旧红、与本提交无关）；clippy 新改文件零告警；cargo doc 仍原有 24 条。§2 阶段 3 第 5 条至此全部落地。仍未做：CLAIM（属性 17）、TYPEX/CATR 反向表、跨页记录、extract flush（第 4 槽）那一档「别人动过就不覆盖」的变体。
- 2026-09-20：**E3D 人工门第二问（Q8b）——真引擎认不认「不带 owner 的引用成员」：夹具已备、待用户跑。** `e3d-write-box <db> [desvir.dat] --group`：在原来那一个 BOX 会话之后**再追一个会话**——WORL 下新建 GPWL（挂进 WORL 成员表末尾）、GPWL 下新建 GROU，GROU 的成员表放两个**引用**（那个 ZONE 与新 BOX，各自 owner 不变），即 `MemberListKind::Referenced` 那一档 noun 的成员表形态；bin 先核 desvir 里 GROU 的模板字 3 确为 2 再写。分两个会话是为了 E3D 若拒开能分清是 BOX 还是 GROU 那一步。已在 `%TEMP%` 的 ams8000 副本上验过：会话 641（BOX 24384/31483）+ 642（GPWL 24384/31484 owner WORL 16192/0、GROU 24384/31485 owner GPWL，成员 24384/47 与 24384/31483），读侧回读 6 642 元素、GROU 两个成员各归其原 owner（ZONE→SITE 24384/46、BOX→ZONE）、会话链 640→641→642 base 逐级对；真样本未动。`%TEMP%\e3dio_gate\prepare.ps1` 已改为带 `--group`，PASS 判据加一条：E3D 能导航到 GROU（`=24384/31485`）、`Q MEM` 列出 ZONE 与 BOX、BOX 的 `Q OWNER` 仍是 ZONE。**现状：PASS（2026-09-21 TTY 无人值守跑通，见下 Q8c/Q8d 条与 `docs/evidence/2026-09-21-e3d-tty-gate-write-path.md`）**：真 E3D 3.1 对 `=24384/31485` 打出 `Type GROU / Owner GPWLD 1 of WORLD /* / Members 1 ZONE 1 /1RX03-MSUP 2 BOX 1 =24384/31483`，GPWL owner `/*` 成员 GROU，BOX owner 仍 `/1RX03-MSUP`——不带 owner 的引用成员表真引擎认。
- 2026-09-20：**反向表——TYPEX 与类型 16 反向引用表的维护落地，口径由反编译 + 语料两头坐实。** 之前 §2 阶段 2 第 4 条与 §4 都记「TYPEX/CATR 不维护」。这次先把 core 的写法读实：
  - **谁维护**：类型 16（单 ref）属性的写入口 `sub_5AB8D20(hash, 16, 2, buf, &2)` 取属性描述子（`sub_5AB5920`），描述子类型 == 16 才走 **`sub_5AB91B0`**（类型 14/15/16/18 共用的「键控表属性」写器）；类型 4 → 普通 `4SCEA`，不进表。`sub_5AB91B0` 的 case 16：新值 `(0,0)` → `4RCETA` 删条目即返回；否则新键 = `[目标.w0, 目标.w1, 自身.w0, 自身.w1]`（4 字），先 `4GCEA` 读旧值——有且非空、且与新值不同 → `3CTE(hash, 旧键, 4, …, op 0)` 删旧边（534/523 可容忍），再 `3CTE(hash, 新键, 4, …, op 1)` 插新边（529 可容忍）；插入报 523（表不存在）→ **`sub_5AFF6D0(dbno, hash)` 建表**（查模板类型 11 的表定义：定义类型 3 → 键宽 4、值宽取定义；分一页、`sub_5AFCB40` 登记进目录、页头 `[5, hash, 0, 键宽, 值宽, 标志, 页字数−7]`）再插；最后 `4SCEA` 把值写进记录。case 14（键控表属性）：键 = `[值的 stride 个字][自身.w0][自身.w1][k−1]`。
  - **删元素** `sub_5AB3620`：先摘 owner 成员表，再**按 noun 模板的属性表**逐属性（只看类型 14/15/16/17/18）`4GCEA` 读值：16/17 → 每对非零 ref 出 `3CTE(hash, [t, 自身], 4, …, op 0)` 删；14 → 每个首字非 `0x80000000/1` 的元素出 `3CTE(hash, [值…, 自身, k], stride+3, op 0)`；15 → 表里映到自身才删；然后 `3CTE(INDEX, 自身, 2, op 0)` 摘键、必要时删 CLAIM。
  - **TYPEX 的键形**：core 里 `13396799`（= `db1_hash("TYPEX")`）只出现在 `DB_Element::checkForUnknownAtts` 与 RCF 输出两处——TYPEX 没有专用代码，就是模板里一个类型 14、1 元素 1 字的属性；`ams8000_0001` 实读：TYPEX 表 5941 键**全部** = `[noun, refno.w0, refno.w1, 0]` 且对应索引里的元素（`s2probe tables`）；5941 个元素的 explicit 流首项就是 `TYPEX`（type_code `0x3800`，data `[1, noun]`）；没有键的 698 个元素恰是模板不列 TYPEX 的 noun（BOX 45、CYLI 34、EXTR 74、LOOP 74、VERT 435、TEXT 24、PAVE 7、WORL/DBMDAT/DBMETA/MNUM/PLOO 各 1）。
  - **类型 16 的不变量**：3000 个元素抽样，类型 16 的 SPRE 466 / LSTU 447 / HSTU 62 个非空值**全部**在表里有边（0 缺、0 无表）；类型 4 的 CREF 622 / JOIS 498 / JOIE 124 / PSPE 79 / HREF 39 / TREF 35 **全部无表**；反向抽 HSTU/SPRE/LSTU 各 300 边 + CATR 34 + ISPE 23，引用者的该属性值 == 键的目标，0 例外。所以：**边 `[t, e]` ∈ 表(attr) ⟺ e.attr == t ≠ nulref，attr 存储类型 16**；表按需创建（ams8000 只有 HSTU/SPRE/LSTU/CATR/ISPE 五张，目录顺序 = 创建顺序，排在 NAME/TYPEX/SPAMAP 之后）；所有这些表与 TYPEX 页头字 5（标志）= 0，SPAMAP = 1。
  - 落地：`write::keyed_cow::rewrite_keyed_table(root: Option<PageId>, hash, …)`——4 字键、无值的 B+ 树 CoW（叶步长 4、内结点 6，路由 = 最大的 ≤ 目标分隔键、哨兵/最左兜底，与读侧 `index::reference::route` 同规则；插入不改分隔键、删除留旧分隔键；满页中位分裂、根裂长新根；`root=None` = 建表，页头 `[5, hash, 0, 4, 0, 0, free]`；复制旧页时字 5 原样带过）；插已有键、删不存在的键**幂等**（= core 容忍 529/534）。`write::reverse`：`table_keys(draft, template)` 从记录的 TYPEX explicit 项（type 14，每元素一键 `[值, 自身, k]`）与模板里全部类型 16 属性（implicit 者按 wide 列偏移读两字、explicit 者按 hash 取两字；`(0,0)` 与未设标记不出键）算出「这条记录贡献的全部键」；`ensure_typex(draft, template)` 给新元素补 `[1, noun]`（插在 explicit 流**最前**，与 core 一致）；`set_reference` / `reference_value` 给调用方按模板设/读类型 16 值；`TableEdits::transition(before, after)` = 差集 → 删/插。`stage_session_with_base` 多一个 `templates: Option<&mut TemplateProvider>`：有模板时先用读侧取每个 Update/Delete 在最新会话里的旧记录，Insert 先 `ensure_typex`，逐元素算 before/after 键集做差，NAME 表之后逐表 CoW，已有表改目录根、新表**追加进目录**（`NewSession.table_additions`，目录满 → `WriteError::TableDirectoryFull`，目录以 DAB 标签为界可放 101 张）；只删不插且表不存在 → 跳过（core 的 523 容忍）。公开面：`WriteOptions { templates }` + `write_element_session_with` / `stage_element_session_with`；`MergeOptions.templates` 现在也一路传到 staging，合并写同样维护。`write_element_session`（无模板）行为不变 = 原样保留根。`e3d-write-box` 两个会话都带模板写（BOX 无 TYPEX 属性、不出键；GPWL/GROU 会得到 TYPEX 字段与键）。`RecordDraft` 新增 `explicit_words` / `set_explicit_words` / `set_explicit_words_first` / `set_explicit_refno` / `remove_explicit`。
  - 验收：`keyed_cow` 单测 6 条（插入落序且未动子树保留页号、删除 + 两种幂等、`None` 建表页头逐字 == `sub_5AFF6D0`、叶溢出分裂长新根、字 5 带过、别表页拒绝——都把写出的页写进临时文件再经读侧 **`ReferenceTable::at_root` 走回来**比对）；`reverse` 单测 4 条（TYPEX 常量、键只来自 TYPEX 与类型 16、无 TYPEX 模板不出键、差集）；集成 `tests/write_stage2_tables.rs` 3 条（ams8000 副本）：**新 ZONE**（ISPE implicit@30 + SPCREF explicit，SPCREF 表原本不存在）→ 目录 8→9 张、SPCREF 排末、根页头 `[5, hash, 0, 4, 0, 0]`、TYPEX/ISPE 多出恰好那一键、SPCREF = 那一边、HSTU/SPRE/LSTU/CATR 根页号**原样**、`referrers_of` 答对、记录 explicit 流首项 TYPEX `[1, ZONE]`、diff 增 1 改 1；**改 SPRE**（取表里第一条边 `[A,R]`，R 的 SPRE 指向 B）→ SPRE 少 `[A,R]` 多 `[B,R]`、其它表连根页号都没动，再**删 R** → 每张表恰好少掉引用者为 R 的键（SPRE 边 + TYPEX 键）、R 出索引；**BOX 带模板写**无 TYPEX 字段/键且所有表根原样，**ZONE 不带模板写**所有表根原样、无 TYPEX 字段。`write_stage3_merge::a_merge_keeps_the_reverse_reference_tables_in_step`：A 写 BOX、B 以 S 为 base 带模板合并改 R 的 SPRE → 表随合并走。全套 `cargo test` **309 单测 + 集成全绿**（84 个测试二进制 ok；`session_tail_real` 的 ams7000 活样本漂移一条照旧红、与本提交无关）；clippy 改动文件零告警；cargo doc 仍原有 24 条。**仍未做**：SPAMAP（key 4/val 7）与 UDA 表；跨页记录；extract flush 变体。
- 2026-09-20：**CLAIM 树——`(5,7618377)` 主元素会话索引随写维护，口径由反编译 + 语料两头坐实。** `ENHAS2` 把 `7618377` 解成 `CLAIM`，读侧一直把会话页 +0x24 的这棵树叫 `claim`；但 R1 证据（`docs/evidence/2026-08-30-e3d-io-data-table-tree-7618377.md`）早已定：单写者非 extract 库里它其实是**主元素→会话**索引——refno 键、值 `(owner, 会话)`，键的正是 SITE/ZONE/EQUI/PIPE/BRAN/STRU/SUBS/SCTN… 这些可导航层级 noun，底下的组件/图元不在内，429/429 文件按 noun 干净划分。这次把 core 的维护读实并落地：
  - **谁是主元素**：`sub_5B059C0(1)` 读元素块 `*(*(CE+16)+16)` = noun 模板 payload **字 4**，非零即「自身是主元素」（返回自身 refno），否则沿 owner 链（`sub_5AB6900`）往上找第一个字 4≠0 的祖先。所以 primary = 模板字 4≠0。`ams8000_0001` 实读（`%TEMP%\s2probe claim`）：CLAIM 树 3129 条，**owner 字全 0**、会话字 1..=640；「字 4≠0」对「在树里」的判定 primary&in 3129 / primary&out **1（仅 MNUM，我们不写）** / nonprimary&in **0** / nonprimary&out 3509——近乎完美的谓词。
  - **怎么维护**（`sub_5B0C760(0)`，主存盘流程第 6 步、**不看属性 17 无条件跑**）：latest↔new 的 INDEX 差异，逐元素找主元素 P：st 2（新增）且 P 是自身 → cm+8 加 claim `(*dword_6453DF0, newSesno)`（529 已有→cm+48 改）；st 3（删）且 P 是自身 → cm+36 删；否则（改了，或非主元素冒到 P）→ cm+44 读 P 的 claim、有（≠534）且变过 → cm+48 把会话字盖成 newSesno。`*dword_6453DF0`（写者用户/桶号）在这些单写者存盘里 = 0，故值恒为 `(0, newSesno)`。属性 17≠0 时另有 CLAIM 表搬运（第 7 步 A3a.7，仍不支持）。
  - 落地：读侧 `index::claim::ClaimTable`（refno 键、`(owner, session)` 值的 2/2 树读游标 + 点查，页形同 refno 索引、值语义不同故自解页）；`record::template::NounTemplate.is_primary`（payload 字 4≠0，`new`/`from_attlib` 得 false）；写侧 `write::claim_cow::rewrite_claim_table`（refno 键、`(owner,sesno)` 值的 CoW，键序复用 index 的 `cmp_refno`——「一处实现」守卫加了 `write/claim_cow.rs`；`Set` 建/改、`Refresh` 只改已有会话字保 owner、`Remove`）；`write::claim::plan_claim_edits`（把写的元素按 owner 链冒到主元素，产出 Create/Refresh/Remove，优先级 Remove>Create>Refresh，owner 链先查本批再查文件、有跳数上限防环）。`stage_session_with_base` 有模板时算好 claim 编辑、NAME/反向表之后 CoW 这棵树、会话页 +0x24 写新根（`NewSession.claim_root`）；无模板照旧抄旧根。合并写也走同一路（`MergeOptions.templates` 一路传到 staging）。
  - 验收：`claim` 单测 6 条（新主元素 Create、子元素冒到 ZONE、多跳冒祖、删主元素 Remove 压过 Refresh、改主元素 Refresh、无主元素祖先不产出）、`claim_cow` 4 条（Set 落序/原地改、Remove + 缺键幂等、未动子树保页号 + 分裂长新根、别名树页拒绝——写出页经 `ClaimTable::at_root` 读回）；集成 `tests/write_stage2_claim.rs` 2 条（ams8000 副本）：新 ZONE 得 `(0,S+1)`、其 BOX 无键、SITE 会话冒到 S+1、别的 claim 一字不动、树 +1，再删 ZONE+BOX → ZONE claim 撤、树回原大小、SITE 再冒到 S+2；改 SUBS 只冒它自己；**无模板写 claim 根逐字不变**。全套 `cargo test` **319 单测 + 集成全绿**（85 个测试二进制 ok；`session_tail_real` ams7000 漂移一条照旧红、与本提交无关）；clippy 改动文件零告警；cargo doc 仍 24 条。**仍未做**：属性 17≠0 的 extract/CLAIM 表搬运（A3a.7）与建删 CLAIM 表开关（`sub_5B13670`）；SPAMAP 与 UDA 表；跨页记录；extract flush 变体。
- 2026-09-20：**跨页记录——单条记录的成员/explicit 块按页切段续接（阶段 2b）。** 之前写路径要求整条记录（头+implicit+两块）放得下一页，否则 `RecordTooLong`。读侧早就按块级续接读（`record::block::continues_at`、`record::read_record` 的 `gather` 跟着 `(页, packed)` 链把各段拼成连续记录）；语料里 158526 个块状续接，每个都恰好填到页边界、落到同元素的下一块。这次把写侧补齐：
  - **头+implicit 不跨页**：索引叶指向记录头、读侧一次性把 implicit 区从这一页读出，所以头（11 字）+ implicit 必须整段落在一页；超一页 → `WriteError::RecordTooLong`（现在只对这一段成立，不再是整条记录）。放不下当前 bump 页就翻新页。
  - **成员块、explicit 块各自切段**（`write::place_record` / `place_block`）：从记录头之后按 bump 位置起，一段填到页边界（`容量 = 页字数 − used − 5`，5 = 块头 `[kind|words][owner.0][owner.1][cont.0][cont.1]` 字数），payload 还有剩就本段头写续接地址 `(下一页, packed=pack_offset(1))`、翻页在字 1 续下一段；每段自带块头与 owner，段序即 payload 顺序。头两槽（6/7 = explicit、8/9 = 成员）回填到「各块首段」落处——成员块先放、explicit 块紧随其后，故成员块跨页后 explicit 落在续接页、其地址回填进**已翻走**的记录头页（记录头页留到整条记录布完再推入 appended，故仍可回填）。续接页也是类型 7 数据页，读侧 `page_of` 认。段续接页从 `next` 顺序取，仍在树页之前（A5 顺序不变）。
  - 验收 `tests/write_stage2_crosspage.rs`（ams8000 副本）：给首个 ZONE 追加 2000 个成员（成员块几页长）→ 写成功、appended ≥ 8 页、读回成员表**恰等于**原成员 + 2000 且顺序对（写侧若没切段、读侧会 `BlockOverflowsPage` 报错，故整表读通即证续接链自洽）、diff 改 1、字节前缀不变新页连续；再建一个 1500 成员 + 一条 `DESC` explicit 的新元素 → 成员块跨页后 explicit 落续接页、两块都读回原值（专验「成员块把记录头页挤走后 explicit 槽仍能回填」这条路径）。全套 `cargo test` 阶段 1/2/3 写用例全绿；clippy 改动文件零告警。**仍未做**：类型 19/23 与 hunk 20–23 的写；SPAMAP 与 UDA 表（见下条调研）；extract flush 变体；CLAIM 表搬运（A3a.7）。
- 2026-09-20：**SPAMAP / UDA 表调研——结论：现在不宜写侧维护（会写错或无从验证），不硬做。** 先把两者读实（`%TEMP%\s2probe spamap`）：
  - **SPAMAP（`230902453`，key 4 / val 7，flags 1）不是记录属性的镜像，是几何算出来的空间索引**。实读键 = `[空间格 hash, refno.w0=24384, refno.w1, k]`（即 `sub_5AB91B0` 类型 14 的 `[值, 自身, k]`，值 stride=1 = 空间格号），**7 字值 = 世界系有向包围盒 + 标志**：如 `[-197549, -59672, 24500, -197164, -58645, 25000, 2]`（前 6 字按 i32 是 min/max XYZ 毫米，末字是标志 0/2/3）。模板里 BRAN/SCTN/BOX/PANE 等几何 noun 列 SPAMAP（类型 14）。**格号与包围盒都要靠完整几何+定位流水线（POS、朝向、owner 变换、图元尺寸→世界系 AABB）算**，不是从记录里抄——写侧没有几何引擎，硬写会往空间索引里塞错值，比不写更糟（E3D 可自行重建空间图）。所以新/改元素**不产 SPAMAP 条目**（可接受：元素照样可导航/可查，只是暂不进空间索引）；真要做需先移植几何/AABB，属另一大块，明确留待。
  - **UDA 表（UDATAB/UDAFTB/UUIDAR）**：模板里 ZONE/EQUI/…/PANE 都列 UDATAB/UDAFTB（类型 14、cnt 1000 = 变长数组哨兵），但 **ams8000 的会话表目录里根本没有这三张表**（本项目无这类 UDA；UUIDAR 连模板都没 noun 列）。装机语料里没有带这些表的库可坐实键/值形与写侧口径，也无从验证；而 e3d-io 目前写的元素（BOX/ZONE/GROU…）不带此类 UDA。所以 UDA 表**暂不维护**——不是不能做，是没有能对照坐实的语料，硬写无从验证。两者都记 §4 风险，等「有几何引擎」（SPAMAP）/「有带 UDA 表的项目库」（UDA）再补。
- 2026-09-20：**extract flush（第 4 槽）变体——合并冲突时保留提交侧（latest），不是写者。** 普通 `SAVEWORK`（`a6=0`）里同一属性两人都改是「写者赢」（A3b.2，`sub_5B0D950` 写 working 值、不比 base↔latest）。第 4 槽 = `DXSWK2`（extract flush）多一步：`sub_5B0D950`/`sub_5B1F660`/`sub_5B211C0` 都先比 base↔latest，**提交侧也改过就跳过写者的改动**——把 extract 刷回 master 时不能盖掉 master 期间新增的东西，故冲突时 master（latest）赢。落地：
  - `write::record::MergeMode { PlainSaveWork, ExtractFlush }` + `three_way_with_mode(base, latest, working, member_merge, mode)`；`three_way`/`three_way_with` = `PlainSaveWork`（行为不变）。ExtractFlush 下：implicit 字，写者改了且**提交侧也改了**（`latest[i]≠base[i]`）→ 跳过留 latest；explicit 属性，写者改了且 latest 那条 ≠ base → 跳过；owner，写者移了且 latest 也移了 → 留 latest（`sub_5B211C0`）；`WholeList` 成员表，写者改了且 latest 也改了 → 留 latest（`sub_5B1F660` 第 4 槽先比 base↔latest）。不相交的改动照旧各自合上；提交侧没动的地方写者照旧赢（无冲突）。
  - `write::merge::MergeOptions` 加 `extract_flush: bool`（默认 false = 普通 SAVEWORK），`rebase_updates` 据此选 `MergeMode` 传进 `three_way_with_mode`。逐成员合并（`MemberByMember`）的第 4 槽里 `sub_5B20CA0` 那条「按成员比 base↔latest 的 OWNREF、别人换了它 owner 就不加」的细则本条未建模，**已由 2026-09-21 的「st 1 加成员门」条补齐**（见下）。
  - 装机语料无 extract（每库 EXTID 1），故与所有合并用例一样靠构造的三视图验证：`record` 单测 `extract_flush_keeps_the_committed_value_on_a_clash`（同改 implicit 字 12 + explicit 0xAA → 留 latest，不相交的字 13 照合）、`extract_flush_keeps_the_committed_owner_and_whole_list`（两边都移 owner + 都改引用表 → 留 latest；普通 SAVEWORK 同视图写者赢；提交侧没动则写者赢）；集成 `tests/write_stage3_merge.rs::an_extract_flush_merge_keeps_the_committed_value_where_plain_keeps_the_writers`（ams8000 副本，A 提交 S+1、B 以 S 为 base 同改一字合并 S+2：`extract_flush:false` 留 B、`true` 留 A）。全套 `cargo test` 全绿（唯 `session_tail_real` ams7000 漂移照旧红、无关）；clippy 改动文件零告警。**仍未做**：类型 19/23 与 hunk 20–23 的写；SPAMAP（需几何）/ UDA（需语料）；CLAIM 表搬运（A3a.7）；`MemberByMember` 第 4 槽的 OWNREF 细则。
- 2026-09-21：**类型 19 串列表与 hunk 20–23 外联块的写（阶段 2c）。** 语料里没有一个设了值的 19、也没有任何 hunk（算法清单 §5 类型对表），口径全部从 core.dll 两侧对读：写侧 `4SCEA`（`sub_5AB7B90`）+ `db_cxx_put_str_vector`（`sub_5AD5060`）+ 块写器 `sub_5AEC110`，读侧 `sub_5AB4CF0`（explicit 读）+ `sub_5AEA500`（块读）+ 记录搬运 `sub_5AE9960`，本轮新反编译了后三者（`%TEMP%\e3dio_decomp`）。
  - **类型 19 inline**：值 = `[n]{[i][字数][字节数][载荷×字数]}*`（`sub_5AD5060` 按串 `push(i)`、`push(字数)`、`push(字节数)` 再追加 4 字符/字打包文本），4SCEA `v114 == 19` 分支把整个 buf 原样写进 explicit 块，头 `19<<26 | 总字数`、**无独立计数字**（别的数组型是 `[type<<26 | 字数+1][计数][数据]`）；读侧 `case 19` 从 entry[2] 起 memcpy `字数` 字原样返回。`sub_5AD5060` 报给 4SCEA 的「长度」`*a5 = 1 + Σ(4·字数_i + 3)`（不是项数），空 vector → `4SCEA(hash, 19, 0, …, len=0)` → 4SCEA 视为未设，**删掉已有项、不写**。
  - **hunk 20–23**：4SCEA 升级表 6→21、7→20、10→22、19→23，条件 `(字数 ≥ 0x2000（描述子 cnt=0 或 flags&1 时 1024）) && (DB_Attribute::initialised(hunk 登记表) || 原来就是 hunk)`——即是否外联取决于 core 运行态，不是纯尺寸；写侧因此让调用方明确选（`set_*_hunk`），阈值记为 `hunk::HUNK_THRESHOLD_WORDS`。外联时流里放 **7 字目录项** `[hash][type<<26 | 5][1][页][packed][元素数][字数]`：`5` 与 `1` 来自把目录体当 1 个类型 11 元素（`1.0/dbl_6453DF8[11]` = 4 字），`(页, packed)` 是 `sub_5AEC110` 返回的首段地址（`packed = ext | 字偏移<<13`，与索引叶值同编码），元素数 = `*a5`、字数 = a3。值本体经 `sub_5AEC110(a3 == 3)` 写成 **kind 3 块链**：块头 **7 字** `[3<<16 | 本块总字数][refno.w0][refno.w1][cont 页][cont packed][hash][type]`（比成员/explicit 块头多 hash 与 type 两字），载荷紧随，填到页边界就在 cont 写下一段地址、下一页字 1 续，最小段 = 头 + 2 字（`sub_5AE9DF0` 的 `a6 = 9`）；4SCEA 在当前输出页**先写 hunk 再写记录**。读侧 `sub_5AB4CF0` 20–23 分支：`*len = entry[5]`，字数 = type 23 取 entry[6]、其它 = `ceil(entry[5] / dbl_6453DF8[type])`，`sub_5AEA500(kind 3)` 校验 `(hdr>>16)&0xF == 3`、每块取 `本块字数 − 7` 载荷字、还欠就沿 cont 走。发现一处 core 自身的冗余：4SCEA 给 `sub_5AEC110` 的长度 `a7[2] = a3`，而该函数对 kind 3 写 `a7[2] + 4` 字——多写 4 个调用方缓冲区之后的字；读侧只读 entry[6] 字、`sub_5AE9960` 搬运时按 entry[6] 读回再以 `len − 4` 传入写恰好 len 字，所以存盘后（临时页搬到真实页）hunk 恰为 len 字。本实现直接写恰好 len 字。记录 word 10 的 explicit 字数照常 = Σ(2 + data_words)，每个目录项贡献 7。
  - **落地**：读侧新模块 `record::hunk`——`HunkRef`（目录项，`from_explicit`/`from_words`/`to_words`/`address`）、`hunk_block_at`（kind 3 块头，`block::block_at` 仍不认 kind 3 → 记录走到 hunk 处自然停）、`read_hunk`（沿链取恰好 `words` 字，`HunkError`）、`decode_text_list`（19/23 → `Vec<String>`）、`decode_hunk_value`（20/21/22 → `IntArray`/`RealArray`/`Text`）、`hunk_type_of`/`inline_type_of`/`payload_words`/`HUNK_THRESHOLD_WORDS`；`record::page_of` 改 `pub(crate)` 供其复用页头校验。写侧 `RecordDraft` 加 `hunks: Vec<HunkDraft>`（待放置的 hunk），`set_explicit_text_list_ascii`（19 inline，空表 = 删项）、`set_hunk_words`/`set_int_array_hunk`/`set_real_array_hunk`/`set_text_hunk_ascii`/`set_text_list_hunk_ascii`（目录项先写 `(0,0)` 地址 + 登记 `HunkDraft`）、`pending_hunks`/`hunk_ref`/`explicit_stream_words_with_hunks`（放置后回填地址）、`text_list_data_ascii`；`remove_explicit`/任何同 hash 的 upsert 顺带丢掉待放置的 hunk；`content_eq` 也比 `hunks`；`three_way_with_mode` 把 pending hunk 随「采用了谁的字段」带过去（extract flush 冲突留 latest 时写者的 hunk 一起丢）；inline 项加 `MAX_INLINE_DATA_WORDS`（0xFFFF，读侧按 u16 半字解长度）超限 → `ExplicitTooLong`。`write::place_record` **先放 hunk**（`place_block` 通用化：`header_extra=[hash,type]`、`min_payload=2`）、把首段地址回填进 explicit 流、再放记录头/成员/explicit；从盘上读回的目录项保留原地址原样重发（页不改写，地址恒有效）→ 改别的属性不会搬 hunk。`ExplicitValue` **未加变体**（engine/uda_catalog/schema/db_element/e3d-descriptor 多处穷举 match），19 与目录项照旧以 `RawWords` 到达，由 `HunkRef::from_explicit` / `decode_text_list` 解。
  - **验收**：`record::hunk` 单测 5 条（目录项解/构、块头解与拒绝、串列表解码含空表与越界、20/21/22 载荷解码、类型表互逆）；`write::record` 单测 4 条（`text_list_data_ascii` 布局与 `1+Σ(4w+3)` 计数并经 `decode_text_list` 走回、inline 19 头无计数字与空表删项、hunk 目录项/待放置/地址回填/覆盖与删除丢 hunk/读回不再 pending、三方合并携带 hunk 与 extract flush 冲突丢弃）；集成 `tests/write_stage2_hunks.rs` 2 条（ams8000 副本）：新 GROU 带 inline 19（4 串含空串）+ 23（600 串、载荷 4801 字跨页）+ 20（7 int 含极值）+ 21（6 real）+ 22（51 字符文本）+ 一条普通 int → 读回流里恰 6 项、目录项各 5 字首字 1、每个 hunk 经 `read_hunk` 读回解码等值、23 的元素数 = `1+Σ(4w+3)`、word 10 = `wide_format_word(0, 3 + (2+19 字) + 4×7)`、每个 hunk 落在本会话新页且**页号 ≤ 记录页**、块头 kind 3 且 hash/type 对、别 refno 与 `block_at` 都不认它、600 串的首段填满整页且有 cont、其它三个单段恰等字数、diff 增 1、字节前缀不变新页连续；再重写只换 int hunk + 一个普通 int → 其它三个目录项**逐字不变**（仍指旧 hunk 且可读）、新 int hunk 落新页、旧 int hunk 仍可读、diff 改 1；合并用例：A 提交 S+1 给 ZONE 加 inline int，B 以 S 为 base 给同一 ZONE 设 real hunk 合并 S+2 → 两者都在、hunk 已放置可读、diff S..S+2 改 1。全套 `cargo test --no-fail-fast`：330 单测 + 集成 87 个测试二进制 ok，唯 `session_tail_real` ams7000 漂移一条照旧红（无关）；clippy 改动文件零告警（`direction_spec.rs` 的 `FRAC_1_SQRT_2` 两条 clippy error 为既有、非本轮）；cargo doc 仍 24 条。**仍未做**：SPAMAP（需几何）/ UDA（需语料）；CLAIM 表搬运（A3a.7）；`MemberByMember` 第 4 槽的 OWNREF 细则；hunk 的 E3D 人工门（真引擎读 e3d-io 写的 hunk）未跑——配方可沿 §7 Q8 的 `e3d-write-box` 路子再加一个带 hunk 的会话。
- 2026-09-21：**E3D 人工门第三/四问（Q8c/Q8d）——真引擎认不认 e3d-io 写的 hunk 与类型 19 串列表：PASS（TTY 无人值守跑通，全部 14 问绿；见末条与 `docs/evidence/2026-09-21-e3d-tty-gate-write-path.md`）。** 夹具 `e3d-write-box <db> [desvir.dat] [--group] [--hunk]`，`--hunk` 在前面的会话之后**再追两个会话**（分开是为了 E3D 若拒开能分清是 hunk 还是串列表）。下面两段是首版夹具的配方（RATM / 单个 EQUI），实跑后按 E3D 的反应改成两个 EQUI + `UUIDIA`、DESC 120 字符——改动与原因见末条：
  - 会话 A：首个 ZONE 下新建 **EQUI**（挂进 ZONE 成员表末尾），`DESC`（type 10 → **hunk 22**，65 字符文本）、`DESP`（type 6 → **hunk 21**，12 个 real）、`RATM`（type 7 → **hunk 20**，1..=500 共 500 个 int，链**跨两页**）三个属性都写成外联块。挑这三个是因为它们在 EQUI 模板里**没有 implicit 槽**（wide word offset 0）——`4GCEA`（`sub_5AB5BC0`）只在描述子无 implicit 槽时才走 explicit 读 `sub_5AB4CF0`，hunk 目录项只可能在那儿被读到；数量都压在描述子声明数以内（480/100/500）。bin 写前先核这三点（`explicit_only`），不满足就拒写。
  - 会话 B：首个 ZONE 下新建 **FIXING**（DESI 模板里带类型 19 的 noun 只有 FIXING/PFIT/HRPANE/HRGATE/HRPOST/HRTERM，属性 `ROKEYS`/`ROVALS`，cnt 20），`ROKEYS = ["K1","K2"]` **inline 19**、`ROVALS = ["V1","V2"]` **hunk 23**；两项各 9 字、`sub_5AD5060` 口径的元素数 15 < 20。FIXING 挂 ZONE 下是否合 E3D 层级规则未核（读侧不管；E3D `Q ATT` 只读数据），若 E3D 只在这一会话上报错即归为层级/串列表问题、不影响 hunk 结论。
  - 已在 `%TEMP%` 的 ams8000 副本上验过（`--group --hunk`）：会话 641 BOX 24384/31483、642 GPWL/GROU 31484/31485、**643 EQUI 24384/31486**（DESC hunk 页 16499 字 1、DESP 字 25、RATM 字 56 起 449 字填满整页 → cont 页 16500 字 1 再 51 字；EQUI 记录本身在页 16500 字节 236，即 hunk 在记录之前；word 10 explicit 字数 25 = TYPEX 4 + 3×7）、**644 FIXING 24384/31487**（ROKEYS inline 9 字、ROVALS hunk 页 16513 字 1 共 9 字，元素数 15）；读侧 `record::hunk::read_hunk` 逐个读回解码等值（DESC 原文、12 个 real、1..=500、`["V1","V2"]`），ZONE 成员 124 → 127 且末三个 = BOX/EQUI/FIXING，会话链 640→…→644；真样本未动，探针改动只在 `%TEMP%\s2probe`（新 `hunks` 子命令）。`%TEMP%\e3dio_gate\prepare.ps1` 已改为 `--group --hunk` 并加第 5/6 步与 PASS 判据：E3D 上 `=24384/31486` 后 `Q DESC`/`Q DESP`/`Q RATM` 打出上述值 = **hunk PASS**；`=24384/31487` 后 `Q ROKEYS`/`Q ROVALS` 打出两个两项列表 = **串列表 PASS**。**现状：已由下条 TTY 跑通，不必再手工跑**。
- 2026-09-21：**E3D 人工门改成无人值守 TTY，Q8 / Q8b / Q8c / Q8d 全部 PASS。** 用 gen-model 现成的 `l3_suite --check-driver`（`des.exe` TTY 会话 + `AVEVA_DESIGN_ENTRYMACRO` 宏，登录 `SYSTEM/XXXXXX`、MDB `/ALL`）读回 e3d-io 写的会话，不再依赖人在 GUI 里点。为了不碰真样本项目，把 `AvevaMarineSample` 复制到 `D:\e3dio-gate\`（去 amspic/amsdrg，3.2 GB，robocopy 3 s），`--project-dir` 指向副本（evars 用 `%projects_dir%` 参数化，驱动会绑到副本的父目录）；本机常年挂着 3 个别的 des.exe，driver 加 `L3_ALLOW_EXISTING_E3D_SESSION=1` 只等自己起的那一个。落地 `tools/e3d/write_gate.ps1` + `tools/e3d/write_gate.mac.template`：重置副本库 → `e3d-write-box --group --hunk` → 按实跑 refno 渲染宏 → 跑 driver → 逐问 grep 判 PASS/FAIL；全程 10 s，可反复跑。宏写法照 gen-model 的教训：一问一段 `ALPHA LOG`（段内未处理的报错会让整段日志不落盘），每个导航与每个 `Q` 后跟 `handle any` 把 `$!!error.text` 打进日志。
  - **结果**（`docs/evidence/2026-09-21-e3d-tty-gate-write-path.md` 有日志原文）：BOX `Position E 1000mm N 2000mm U 3000mm / Xlength 300mm …`；GROU `Members 1 ZONE 1 /1RX03-MSUP 2 BOX 1 =24384/31483`；EQUI×2 的 `Description …（120 字符逐字）`、`Desparam 1001.3 … 1100.3`（100 个，EQUI 2 那条链跨两页）、`Uuidia 11 12 13 14`；FIXING `Rokeys 1 K1 2 K2`（inline 19）、`Rovals 1 V1 2 V2`（hunk 23）；ZONE 128 成员末四个恰为新四件；`Q SESSION` → `Last saved session number 644 … User: e3dio Description: e3d-io write-path gate: …`；`Q ATT` 走遍 EQUI 108 / FIXING 40 个属性无错。旁证：第一轮后 E3D 退出时自己在我们的 644 上追加了会话 645（`Saving Meta Data.`，reason 15，previous_page = 我们的会话页、INDEX 根是对我们索引页的 CoW），e3d-io 再以 645 为 base 写 646，E3D 照读——两个写者的链互相叠着走。
  - **夹具据 E3D 的反应改了三处**（都不是 hunk 问题）：① `RATM`/`UNIPAR` 模板里有、E3D 答 `(2,201) does not have attribute`（不在字典可查表里，`Q ATT` 也不列）→ int 数组 hunk 改用 `UUIDIA`（cnt 4）；② `USRWEI` inline / hunk 5 / hunk 500 三种存法一律 `Usrweight unset`、PML `!!ce.usrwei` 报 `(2,425) EXPRLIB inconsistent operation stack`——属性另有来源，弃用；③ DESC 写满模板 cnt 480 → `(2,170) Attribute DESC too long`，65/120 字符正常：**模板 cnt 对文本是 UTF-8 字节容量，E3D 字典的 DESC 上限 120 字符**（调用方口径，记进 bin 注释）。为保住「链跨页」这一问，改写两个 EQUI（DESC 120 字符 + DESP 100 real + UUIDIA 4）：第二个的 hunk 从第一个记录结束处起放，DESP（200 字）必跨页；bin 写后读回各链页数并打印（本轮 EQUI 2 DESP 2 页）。
  - **第二轮（同日）：E3D 自己改 hunk 属性并 SAVEWORK，看它怎么对待我们的 hunk。** 宏含一个 `SAVEWORK`（driver stateful 模式 `--target-db-file … --aios-project AMS`，`outcome=Completed`）：EQUI 1 改 DESC/DESP（`UUIDIA 5 6 7 8` 被 E3D 拒 `(41,160) already set`——UUID 一次性属性，E3D 语义）、EQUI 2 只设 FUNC、FIXING 只设 DESC → E3D 写出 646（base 645）。读侧对照 645 快照：**E3D 重写记录时不搬已存盘的 hunk**——EQUI 2 三个目录项、EQUI 1 的 UUIDIA 目录项、FIXING 的 ROVALS 目录项**逐字不变**，仍指我们页 16499/16500/16513 的块（= `sub_5AE9960` 的 `ext != 0 → 跳过复制`，与 e3d-io「盘上读回的目录项原样重发」一致）；被改掉的 DESC/DESP 旧目录项被删、新值 inline **追加在流末**（还多了一项 `UNIPAR = [NONE,NONE,NONE]`，E3D 设 DESP 的单位副作用）；E3D 写的记录头/块头/流首 TYPEX 与我们的约定逐字一致（`w7 = ext | 字<<13`、块头 `[1<<16|words][refno×2][0][0]`、目录项 `type<<26|5` 原样保留）。`diff_sessions(645,646)` = 改 3。E3D 本轮值都小，没自己造 hunk（阈值 ≥ 0x2000 字，普通模板属性 cnt 到不了）——E3D 写出的 hunk 块与我们的逐字比对仍缺样本。细节与日志见 `docs/evidence/2026-09-21-e3d-tty-gate-write-path.md` 第二轮。
  - 未做：`Q ATT` 不列 UUIDIA/ROKEYS/ROVALS（非字典可见属性），只能 `Q <名>` 单查；E3D 自造 hunk 的样本。
- 2026-09-21：**A3b.4 收尾——`sub_5B20CA0` 对「我加的成员」（st 1）的门，含第 4 槽的 OWNREF 细则。** 本轮新反编译 `sub_5B20CA0`/`sub_5B219B0`（`%TEMP%\e3dio_decomp`），把 st 1 分支读实：落位前先看**成员元素本身**——普通存盘（`a13 == 0`）：切到 base `4GTE(成员)`、再切到 latest `4GTE(成员)`，「base 有（错误码 ≠ 10）且 latest 没有（10 = 元素不在索引）」→ 不加、直接下一 st；第 4 槽（`a13 ≠ 0`）**改比 OWNREF**：分别在 base / latest 上 `4GTE` 后 `sub_5AB6020(89647224 = OWNREF, buf, 2)` 读记录头 4/5 字，元素不在索引时记 `(0,0)`，两侧不等 → 不加。两条门都**不看 working**；过门才 `sub_5B219B0`（已在 new 表则幂等，否则前一个共有兄弟锚后插）。st 2（移）/ st 3（删）没有门。意义：第 4 槽下 `sub_5B211C0` 让提交侧改过的 owner 站住，这条门让成员表跟 owner 一致——写者把 M 挪进 O 而 master 期间把 M 挪去了 Q，则 M 留在 Q、O 表不收 M；普通存盘那条则挡住「把别人已删的元素塞回成员表」（写者若同时改了 M 会先被 562 拒掉，能走到这里的是只改 owner 表不改成员记录的写法——引用表的形态）。
  - 落地：`write::record::MemberStanding { base_owner: Option<RefNo>, latest_owner: Option<RefNo> }`（`None` = 该会话索引里没有）+ `blocks_addition(mode)`（Plain：`base.is_some() && latest.is_none()`；ExtractFlush：`base.unwrap_or((0,0)) != latest.unwrap_or((0,0))`）；`merge_members_with(base, latest, working, blocked)` 在 `Added` 落位前问一次 `blocked(member)`，`merge_members` = 不挡；`three_way_with_standing(.., standing: FnMut(RefNo) -> Option<MemberStanding>)` 只在 `MemberByMember` 档问、`None` = 不知道即放行，`three_way_with_mode` = 全 `None`（行为不变）；`write::merge::rebase_updates` 对每个 `Update` 先 `member_list_diff(base 表, working 表)` 取出 `Added` 的成员，用已开着的 base/latest 两个 `ReadOnlyEngine` `find_element` + `header().owner` 查好装进 map 再传入（读失败即本次写的错误，不吞成「不知道」）；`WholeList` 档不查。公开面加 `write::MemberStanding`。
  - 验收：`write::record` 单测 `an_added_member_is_gated_on_where_it_stands_in_base_and_latest`（门的八种取值；经 `merge_members_with` 两种模式各挡掉哪个、锚序照旧、移动不问门；经 `three_way_with_standing` 随模式变、`WholeList` 无门、`three_way_with_mode` 不挡）；集成 `tests/write_stage3_merge.rs` 2 条（ams8000 副本）：**A 删叶子 M（出 P 表 + 出索引）写 S+1，B 以 S 为 base 把 M 追加进另一个 ZONE O 的表**（不碰 M）→ 普通与第 4 槽两轮都：O 表 = 提交侧原表、M 仍不在索引、去重后**只追加会话页 1 页**、`diff` 0/0/0；**A 把 M 从 P 挪到 Q（M owner + P 表 − M + Q 表 + M）写 S+1，B 以 S 为 base 把同一 M 从 P 挪到 O** → `extract_flush:true`：M owner = Q、O 表不收 M、P 表两边都摘了、三条记录都与 latest 相等 → 只追加会话页、`diff(S+1,S+2)` 0/0/0；`false`：M owner = O、O 表末尾多 M、`diff` 改 2（M 与 O）。全套 `cargo test --no-fail-fast`：**331 单测 + 87 个测试二进制 ok**，唯 `session_tail_real` ams7000 漂移一条照旧红（无关）；clippy 改动文件零告警（`FRAC_1_SQRT_2` 两条既有）；cargo doc 仍 24 条。
  - 顺带发现、**未做**：普通存盘里 `sub_5B211C0` 换 owner 时还会到 **latest 的 owner** 上 `4FA` 定位并摘掉该成员项（A3b 第 3 步）——本 crate 只按写者给的三条记录合并，写者没交 Q 的记录时 Q 表里的 M 不会被摘（上面 `false` 那一轮 Q 表仍含 M，用例未断言 Q）。要补需在 `rebase_updates` 里对「owner 变了的 Update」额外读 latest owner 的记录、摘成员、作为附加 Update 交下去。另仍未做：SPAMAP（需几何）/ UDA（需语料）；CLAIM 表搬运（A3a.7）；E3D 自造 hunk 的样本。
