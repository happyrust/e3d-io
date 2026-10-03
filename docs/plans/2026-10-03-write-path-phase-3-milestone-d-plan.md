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

（开工后按步记录。）
