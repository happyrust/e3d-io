# 计划：e3d-io 写路径 · 第三期 · 里程碑 D（任务第 7 组后半：重算取证、AABB 剔除后坐实、4.20 示例、门槛核查）

状态：**评审通过**（2026-10-03 19:04，Plannotator 门 `approved`，无批注 → Q1–Q5 全按推荐项）
日期：2026-10-03
分支 / worktree：`write-path` @ `D:\work\plant-code\old\vendor\e3d-io-write`，HEAD `f5fefe8`（里程碑 C 收尾，领先 `github/write-path` 35 个提交，未推送）
依据：`.kiro/specs/write-path-phase-3/` 需求 2、需求 4 第 17、20 条、需求 5 第 2–4、8、9 条、需求 6 第 16 条，任务 7.3、7.5（续）、7.6、7.7、7.8；`docs/plans/2026-10-03-write-path-phase-3-milestone-c-plan.md` §6
上一里程碑：`docs/plans/2026-10-03-write-path-phase-3-milestone-c-plan.md`

---

## 0. 一句话

C 坐实了编码、覆盖、记录项与 `aabb.degenerate`。剩下的规则都取决于「E3D 何时重算条目」：D 先从 Core3D 读出重算的入队路径（谁调 `maplist/MLSADD`、在什么改动下），再用真 E3D 探针（新增 `tools/e3d/spamap_probe.ps1`，副本每轮重置，会话 USER 为 `e3dio`）坐实 7.3 的重算表、后代与删除规则；据此按需求 4 第 17 条剔除 `ams7999_0001` 的 18 个 Built_In_AABB 反例，重判 `aabb` 其余五条（7.5）；跑出 4.20 所要的那一会话，写示例测试（7.6）；最后做 8.1 之前的门槛核查（7.7）与第 7 组检查点（7.8）。

## 1. 现状

| 项 | 状态 |
|---|---|
| HEAD | `f5fefe8`；113 个测试二进制 + Doc-tests 690 / 0 / 30，clippy 63、`cargo doc` 25（同基线） |
| 已坐实 | `spamap.{rounding, word_sign, short_axis, zero_extent, bucket, key0, table_shape, table_header_rest, entries_k, flags, coverage, record_item}`、`aabb.degenerate`（任务 7.1、7.2、7.4 打勾） |
| 未坐实 | `aabb.{ori_form, compose, box, cyli, arithmetic}`（出处 ① 已读、候选与 E3D 同式，18 个反例待剔除）；`spamap.recompute.<NOUN>`、`spamap.descendants.{move, pos, ori}`、`spamap.delete_residue`（无候选） |
| 工具 | `tools/e3d/write_gate.ps1`：复制 AMS 样例工程到临时目录、每轮从原件重置 `ams8000_0001`，渲染宏后经 gen-model 的 `l3_suite --check-driver` 跑 des.exe TTY（`write_gate_e3d_saves.mac.template` 会 `SAVEWORK`）；IDA 桥常驻 Core_IDB 与 Core3D_IDB；`e3d-descriptor spamap` / `diff` / `spamap-check` |
| 证据 | `docs/evidence/2026-10-03-spamap-{encoding, coverage, branch-entries, flags, record-item, aabb}.md` |

## 2. 已知线索（C 读到的，待 D 坐实）

| 线索 | 出处 | 对 D 的意义 |
|---|---|---|
| `maplist/MLSADD` @ `0x10220F82` 按 IMAP 入队：IMAP 0 不入，IMAP 3 上溯到第一个非 3 的祖先，IMAP 7 且 owner 非 2 改由 owner 入队；共 11 处调用 | coverage 证据 | 7.3 的入口：读这 11 个调用方，定「哪种改动让哪个元素入队」 |
| `maplist/VDEMPR` @ `0x102208C7` 出队：标志 1 的项跑 `MPBOX`；BRAN / HNGT 的项以队列为第三参跑 `MPBXBR`，由 `sub_102FE99B` 判哪些成员重记 | coverage、branch-entries 证据 | 分支树的增量重算规则在 `sub_102FE99B`（未读） |
| `mapboxlib/MPBOX` 记下一个元素后继续下探成员，成员的框按累积变换重算 | coverage 证据 | 推论：owner 入队即重算其下全部 Geometric_Noun 后代；要坐实「改 owner 的 POS / ORI 是否让 owner 入队」 |
| `maplib/MAPINS`：旧条目六值都在 ±1 内、flags 与 datum 不变、旧 Key0 与旧框相符则不重写 | encoding 证据 | 盘上分不出「触发了但值未变」与「未触发」（§3.1） |
| B8：18 个反例里 EQUI `24383/76459` 下成对的大偏差、`78369` 等只差 minz 1.5 mm | C 计划 §6、aabb 证据 | 像「祖先改了、后代没重算」与「改尺寸后没重算」；剔除要逐个走会话链（§3.3） |

## 3. 复核与风险

### 3.1 盘上看不见「触发但值相同」

`MAPINS` 对 ±1 内的框不重写，所以不改框的属性（NAME、DESC 等）改了之后，「触发重算」与「不触发」在盘上一字不差。写侧只需与 E3D 逐字节相同：这类行在重算表记「不改条目（盘上等价）」，依据写 ①（入队路径）或探针（条目未变）。需求 5 第 2 条要的是「是否重算」，这一点需要你拍板是否接受（Q2 一并）。

### 3.2 真 E3D 探针的成本

每个探针要起一次 des.exe TTY 会话（几十秒），副本每轮从原件重置；探针只改副本，原件与快照不动。会话 USER 为 `e3dio`，以便 E3D_Authored_Element 的判定照旧。

### 3.3 反例剔除要走会话链

需求 4 第 17 条只许剔除「E3D 未对其做重算」的元素。逐个反例要定：条目最后一次写在哪个会话（SPAMAP 表页的会话归属）、此后该元素或祖先改过什么（`e3d-descriptor diff`），再看按 D2 坐实的规则 E3D 是否本应重算。

### 3.4 需求 5 第 9 条要在 E3D 里删除并 SAVEWORK

只在副本上做；被测元素用 0.4.0 写出的 BOX（门用副本里有），删它本身与删它的 owner 各一次。

## 4. 里程碑 D：步骤

每份 Evidence_Doc 按 `tools/ida/EVIDENCE_TEMPLATE.md` 写、过 `check_evidence.ps1`；探针产物留本机 `.agents/tasks/write-path-phase-3-milestone-d/`，不入库。

| 步 | 任务 | 产出 | 验收 |
|---|---|---|---|
| D0 | 7.3 前置 | IDA 读 `MLSADD` 的 11 个调用方、`MPDEL`（删除）、`VDEMPR` 与 `sub_102FE99B`：属性改、换 owner、改 POS / ORI、删除、新建各自何时入队、入谁；写进 7.3 证据的「入口」节 | 每类改动有一个入队例程地址，或写明追不到及原因 |
| D1 | 探针工具 | `tools/e3d/spamap_probe.ps1`：复用 `write_gate.ps1` 的复制、重置与 driver；一个探针 = 一个渲染出的 E3D 宏（改一处 → `SAVEWORK`），再以 `e3d-descriptor spamap` / `diff` 读回该会话的条目变化；带自检 | 复现已知行：改原生 BOX `24384/24777` 的 XLEN 后其条目被重写（需求 5 第 2 条） |
| D2 | 7.3 | `docs/evidence/<日期>-spamap-recompute.md`：BOX、CYLI 的重算表（模板各属性与成员次序）；换 owner、改 POS、改 ORI 时直接成员与更深后代是否重算；删除后的残留；无条目元素删除能否 `SAVEWORK`（需求 5 第 9 条）；其余 Geometric_Noun 按 D0 的入队规则归类（Q2） | `spamap.recompute.<NOUN>`、`spamap.descendants.*`、`spamap.delete_residue` 有结论或写明未坐实；坐实的改 Confirmed |
| D3 | 7.5 续 | 18 个反例逐个走会话链，按需求 4 第 17 条判能否剔除（Q3）；全部剔除则 `aabb` 五条以 ① + 剔除后 0 反例坐实（实现入 `rules.rs`，`core_rules` 改 Confirmed），否则写明哪些仍是反例 | 每个反例有剔除依据，或保留并说明 |
| D4 | 7.6 | 探针改 BOX `24384/26484` 的 XLEN 得到 E3D 为它写出条目的那一会话；`tests/spamap_codec_real.rs` 加 4.20 示例（`aabb` 坐实则断言恰得该条目，否则断言 D7 回退并在 Ledger 记待办） | 测试 0 失败且未跳过 |
| D5 | 7.7 | Spec_Ledger「取证门槛核查」：7.1–7.5 证据各含例程地址与 Corpus 核对、第 3 组读侧解码、以 Corpus 为输入的测试（3.4、5.5、6.3、7.2）在父提交上 0 失败且未跳过（需求 2 第 1、2 条） | 逐项打勾，或写明缺口 |
| D6 | 7.8 检查点 | Build_Checks 对照 690 / 0 / 30；clippy 63、`cargo doc` 25 | 不新增失败，clippy / doc 不增 |

顺序：D0 → D1 → D2 → D3 / D4（D3 用到 D2 的结论）→ D5 → D6。

## 5. 待你拍板

批准本计划 = 全按推荐项（A）。要改哪条，在对应行批注。

| Q | 问题 | 推荐（A） | 备选 |
|---|---|---|---|
| Q1 | 7.3 的取证源 | IDA 读入队路径（出处 ①）+ 真 E3D 探针验证（出处 ②），两者都做 | B：只用探针，把 E3D 当黑盒；C：只读 IDA，不驱动 E3D |
| Q2 | 重算表覆盖面 | BOX、CYLI（Built_In_AABB）逐属性做全表；其余 Geometric_Noun 按 D0 读出的入队规则归类、每类抽样探针；不改框的属性记「不改条目（盘上等价）」（§3.1） | B：Corpus 出现的每个 Geometric_Noun 逐属性探针（工作量大）；C：只做 BOX、CYLI，其余记未坐实 |
| Q3 | 18 个反例的剔除标准 | 逐个走会话链：能指出「条目最后写入后，元素或祖先改过影响框的属性，且按 D2 坐实的规则 E3D 本就不重算」才剔除 | B：沿用 B8 的启发式归类；C：不剔除，`aabb` 五条以未坐实进 8.x（写侧对 BOX、CYLI 按 D7 回退，要调用方给框） |
| Q4 | 探针的运行方式 | 复用 `write_gate.ps1` 的副本与 `l3_suite` driver，探针宏逐条渲染，USER `e3dio`，结果留本机 `.agents/` | B：手动在 E3D 里操作并记录 TTY 输出 |
| Q5 | 提交节奏 | 沿用：每步自动提交，结束统一回报 | B：每步回报，你回「提交」再提交 |

## 6. 执行记录

### D0（2026-10-03 19:1x，IDA 只读；笔记 `.agents/tasks/write-path-phase-3-milestone-d/d0/`）

本轮首条调用前 `ib.ps1 env`：Core_IDB `18D5C155…992C`、Core3D_IDB `03129C29…A712`，IDA 9.2，HEAD `daba8e3`。地址未注明的在 Core3D_IDB。

**入口只有一个**：SPAMAP 队列全局 `dword_10FC1780` 只被 `maplist/MPCNIT`（初始化）、`VDEMPR`（出队）、`MPDEL`、`MLSADD` 与调试转储 `DUMPML` 引用，入队只经 `maplist/MLSADD` @ `0x10220F82`（11 处调用，`check_xrefs` PASS）。11 处全部落在 Core3D 的 `DESDRA_SCPlugs`（core `DB_ElementChangesPlugger` 的订阅者）转来的 `descases` 例程，另有 `descat/ATTABK` 与 `mapupdate/MUPLDE`：

| 改动 | 插件 → 例程 | 入队 |
|---|---|---|
| 设属性 | `PostSetAttribute` @ `0x10409A60` n 41 → `descases/VDESPT` @ `0x101F33A9` | n 48–64：`ATAINT(属性, DCHC)`（字段字 @ `0x10B200DC` = 596407，调用 @ `0x101F343B`）不为 0 才 `MLSADD(元素)`（@ `0x101F3528`）。core `ATAINT` @ `0x58DE220` n 12–19 = `DB_Attribute::findAttribute` + `getField`，`DB_Attribute::getField` @ `0x58D1780` n 73–77：596407 取 `DB_Attribute` 第 23 字。core 一侧：`DB_Element::postSetAttribute` @ `0x59453B0` n 62–67 对每次设属性发事件，`DB_ElementChangesPlugger::PostSetAttribute` @ `0x591E5B0` n 20 属性带 `WNOEVT` 时不发给任何订阅者。`PostSetRefAttribute`（`VDESPF` @ `0x101F2A27`）与 `PostSetName`（`VDESNA` @ `0x101F3BF2`）不入队 |
| 新建 | `PostCreateElement` @ `0x10409710` n 34 → `VDESCR` @ `0x101F1290` | n 20 `MLSADD(新元素)` |
| 换 owner | `PreIncludeElement` @ `0x10409EB0` n 20 → `VDESI1` @ `0x101F23F4` n 20 `MPDEL`；`PostIncludeElement` @ `0x104097C0` n 59 → `VDESIR` @ `0x101F2755` | n 10 `MLSADD(元素)`，强制标志 1（@ `0x10B20024`） |
| 成员次序 | `PreReorderElement` @ `0x10409F10` n 14 → `VDESR1` @ `0x101F27F2` n 15 `MPDEL`；`PostReorderElement` @ `0x10409920` n 59 → `VDESRO` @ `0x101F2989` | n 10 `MLSADD(元素)`，强制标志 1（@ `0x10B20068`） |
| 删除 | `PreDeleteElement` @ `0x10409DD0` n 49 → `VDESDL` @ `0x101F1B24` n 72 → `maplist/MPDEL` @ `0x10220D21` | n 27–30：IMAP 3 / 7 时 `MLSADD(元素)`（即入 owner）；n 31–48：管件 / 吊架件入其 BRAN / HANG；n 53 把本元素移出队列；n 62 `mapboxlib/MAPARF` 只给某元素的计数属性加 1。**本元素的条目不在 Core3D 删** |
| 复制 | `PostCopyAllElement` → `VDESAC` @ `0x101F1562` n 11；`PostCopyAllElementMembers` → `VDESMC` @ `0x101F14B6` n 9 | `MLSADD(副本)` |
| 目录 / 批量 | `descat/ATTABK` @ `0x101F0DEE` n 53、85（目录件改动后入引用它的 ATTA 之 owner）；`mapupdate/MUPLDE` @ `0x102F9375` n 40（按 20 张表逐项入队后 `MPRCSS`） | 外部输入，归需求 16.5 |

插件在元素所在库类型为 7（DRAFT）时改走另一组例程（如 `PostSetAttribute` n 23–31），设计库走上表。`MLSADD` n 18–21 跳过 GROU / GPWL / WORL，n 23–25 IMAP 0 不入，n 30–42 IMAP 3 上溯到第一个非 3 的祖先，n 43–57 IMAP 7 且 owner 不为 2 时入 owner，n 58 入队。出队：`maplist/VDEMPR` @ `0x102208C7` n 34–40 对队项跑 `mapboxlib/MPBOX`；`VDEMPR` 只由 `DDES_WriteMngr::clearPending` @ `0x1052F6E0` n 6 调（虚表 `??_7DDES_WriteMngr` @ `0x10B305B8` 第 8 槽，core 一侧调用时机未追，探针以「改动所在会话」验证）。`MPBOX` @ `0x102FB475` n 95–166 从队项起深度优先走整棵子树，IMAP 1 / 5（及 7）记、n 118–119 遇 IMAP 0 不下探。

**推论（D2 要坐实的形状）**
- 重算表「自身」与 noun 无关：属性 a 改动入队 ⇔ `¬WNOEVT(a) ∧ DCHC(a) ≠ 0`；成员次序改动入队。字典值已在仓内（`schema/e3d31/catalog/noun_attr_fields.json`，或 `e3d_attlib::dict::AttrDict::dchc`）：BOX 的 POS / ORI 为 3，LEVE / OBST / XLEN / YLEN / ZLEN 为 4，NAME / PURP / ORRF / TMRREF / REPCOU 为 0；CYLI 的 DIAM / HEIG 为 4。入队后条目是否变由 `MAPINS` 定（框与 flags 不变则盘上不变，§3.1）。
- 后代：owner 改 POS / ORI（DCHC 3）→ owner 入队（EQUI、SUBE、ZONE、SITE、STRU、FRMW 等都是 IMAP 2）→ `MPBOX` 重算其下全部层（IMAP 0 结点以下除外）；换 owner 同理（`VDESIR` 入被挪元素 → 整棵子树）。
- 删除残留：Core3D 不删被删元素的条目，要看 core 的元素删除路径或探针（需求 5 第 4 条）。
- 对 D3 的影响：E3D 正常改 POS / ORI / 尺寸都会重算，B8 的「祖先改了、后代没重算」只能出自不经 `DESDRA_SCPlugs` 的改动（非设计模块、库级操作或旧版本）；按 Q3，指不出这类改动的反例不能剔除。

`sub_102FE99B`（BRAN / HNGT 增量重记哪些成员）未读：分支 / 吊架树在 C2 已判 MissingInput，写侧按 D7 回退，不影响 D2。xrefs：`0x10220F82` 11、`0x101F33A9` 1、`0x10220D21` 3、`0x10FC1780` 17（Core3D_IDB）、`0x591E5B0` 2、`0x58DE220` 178（Core_IDB），6 / 6 PASS。
