# Requirements Document

e3d-io 写路径第三期——SPAMAP 写侧维护 · 性能 · 页 1 桶表

## Introduction

第二期（T0–T4）落地后，e3d-io 写的元素真 E3D 3.1 全部读得回，但几何元素在 E3D 里「导航得到、`Q ATT` 全对、空间上不存在」：E3D 给每个几何图元维护 SPAMAP 条目，空间搜索只看 SPAMAP；e3d-io 不写条目，所以 `WITHIN VOLUME` 看不见 e3d-io 写的元素，以该元素为体积会报 `(2,536) Error getting volume`，E3D 存盘也不自愈（`docs/evidence/2026-09-22-e3d-spamap-consequences.md`）。第三期以 SPAMAP 写侧维护为主线，其次是第二期 T2 留下的两项性能旗标，再其次是 S6b 页 1 桶表写侧。C 段其余项只在触发条件满足时纳入。

方法论沿用 MISSION：唯一权威是 `core.dll` 与语料字节。每个写侧能力先经 ida-bridge 读 core 并落取证文档，再做读侧（读侧是写侧的验收方），再做写侧，最后过真 E3D 3.1 TTY 门。本期同时把 ida-bridge 的调用固定进仓，不再依赖 `%TEMP%` 下的包装和会漂移的安装路径、实例号。

基线：分支 `write-path` @ `c61b097`（0.4.0 之后）；`tools/e3d/write_gate.ps1` Q0 + 28 问、`alternate_write_gate.ps1` 5 轮、`format0_gate.ps1` 全 PASS。

**优先级**：① ida-bridge 仓内约定与取证先行（需求 1–2）→ ② SPAMAP 读侧与语料普查（需求 3）→ ③ core 取证：键 / 桶 / 条数 / flags / 覆盖 / 重算时机 / AABB 推导（需求 4–6）→ ④ SPAMAP 写侧、合并、CLI（需求 7–9）→ ⑤ 真 E3D 门（需求 10）→ ⑥ `Transaction::open` ≤ 1 s（需求 11）→ ⑦ staging 近线性（需求 12）→ ⑧ S6b 按取证结果条件纳入（需求 13）。

**SPAMAP 已知 / 待取证**

| 项 | 状态 | 出处 |
|---|---|---|
| 表形：页类型 5，名 hash 230902453，键 4 字，叶值 7 字；内结点 4 键字 + `(pgno, ext)` | 已知 | SPAMAP 取证 §0 |
| 值 = 世界系 AABB `[minx, miny, minz, maxx, maxy, maxz]`（0.1 mm 整数）+ flags | 已知 | 同上 §0 第 5、6 条 |
| 键 = `[key0, refno.word0, refno.word1, k]`，`key0 = N × 10^8 + 最短轴 min`，`N = 6 × 轴序 + 桶`；`ams8000_0001` 全表成立 | 已知（语料拟合） | 同上 §0 第 7 条 |
| 桶（1–5）的划分规则；最短轴并列时取哪根；最短轴 extent 为 0 时怎么编 | 待取证 | — |
| flags（语料值 0 / 2 / 3 / 6 / 7）逐位语义 | 待取证 | — |
| 一个元素何时有 2 条、k 的含义（基线 3 746 条中 k = 1 的 121 条，而有 2 条的元素 118 个——至少 3 个元素只有 k = 1 条目） | 待取证 | 同上 §1 第 1 步 |
| 毫米 → 0.1 mm 的取整规则（min / max 各向哪取整） | 待取证 | — |
| core 判定哪些 noun 维护条目（语料：BOX / CYLI / FTUB / BEND / SCTN / ATTA / PNOD / ELBO / EXTR / ELCONN / FITT / STRT / PANE / REDU / VALV / WELD 100 %；EQUI / ZONE / SITE / GROU / FIXING 0） | 待取证 | 同上 §0 第 1 条 |
| 哪些改动让 core 重算条目（`XLEN` 会）；owner 变 / owner 位姿变时后代是否重算；删除时后代怎么办 | 待取证 | 同上 §0 第 4、5 条 |
| 几何元素的记录 explicit 流里是否也带 SPAMAP 项（如 TYPEX 的 `[1, noun]`） | 待取证 | — |
| 世界系 AABB 怎么从 POS / ORI / 尺寸与 owner 链推出（ORI 存储形、复合顺序；CYLI 是解析解、外包盒还是离散面） | 待取证 | — |
| SPAMAP 键按有符号还是无符号比较；键字、值字是否有符号 | 待取证 | — |
| 3D 视图画不画无条目元素；`LIMITS CE` 报不报 (2,536) | 未验（TTY 不出图） | 同上 §2 |

2026-09-22 取证 §0 第 7 条的两组直方图合计不一（N 3 746、flags 3 747，分属会话 640 与 648 的表），首次普查后在新 Evidence_Doc 中更正。

**待拍板决策**（需求按推荐默认写，相关验收标准标「D#」；改选备选时按所选项改写这些条目）

| # | 问题 | 推荐默认 | 备选 |
|---|---|---|---|
| D1 | 世界系 AABB 从哪来；MISSION 范围怎么改 | 混合：BOX、CYLI 由 e3d-io 从元素属性与 owner 链自算（闭式，可对语料与 E3D 逐字核对）；其余几何 noun 由调用方提供（下游 e3d-model 实现）。MISSION 的 Out of scope 改为「几何求解只做 SPAMAP 所需的 BOX / CYLI 包围盒；目录几何、模型生成、渲染照旧不做」 | A：纯调用方提供，e3d-io 零几何，MISSION 只删「不维护 SPAMAP」半句。B：e3d-io 自算全部设备原语（CONE / SNOU / PYRA / DISH / CTOR / RTOR / SLCY / EXTR …），MISSION 范围扩大 |
| D2 | SPAMAP 覆盖哪些 noun | 键 / 值编码与表维护对全部 Geometric_Noun 通用；内置 AABB 先做 BOX、CYLI；其余几何 noun 靠调用方给 AABB | 只做 BOX / CYLI，其余几何 noun 照旧不写条目 |
| D3 | 几何元素拿不到 AABB 时 | 默认拒写整批（一字不写，逐个点名）；显式「跳过」保留其余条目并报告缺的元素；显式「关闭」= 0.4.0 行为。对插入非内置几何 noun 的调用方是破坏性变化，记 CHANGELOG | 默认「跳过」+ 报告 |
| D4 | 性能两项与 S6b 是否纳入 | `open` ≤ 1 s 纳入，并加「不低于页 1 计数器」的发号下界；staging 近线性纳入（先剖析后改）；S6b 写侧按需求 13 的取证结果条件纳入（已有旁证指向「不需要」，见需求 13） | 性能两项推到第四期 |
| D5 | 验收门口径 | `write_gate.ps1` 新增 Q13a–Q13h（需求 10）；另两个门只须不回归；人工图形会话一次 | 另起 `spatial_gate.ps1` |
| D6 | 含 SPAMAP 后的 Q4 追加页口径 | 「≤ 1.5 × 记录页」不含 SPAMAP 页；SPAMAP 页单列，上界按半满叶计（需求 12） | SPAMAP 页计入，阈值待规模门出数后再定 |
| D7 | 依赖「未坐实」规则的写侧逻辑在发布版本中怎么办 | 发布提交的默认构建不执行它——须用到该规则的元素视同无可用 World_AABB，交给 Spatial_Policy（默认「要求」即拒写整批）；其余结构回到 `c61b097` 行为（发号 = 全索引 max + 1、不写页 1）；重算表中「未坐实」的行按「触发」处理（多算只会按当前状态重算，不会算错） | 放在非默认 cargo feature 后；或发布提交根本不含该代码 |
| D8 | 若需求 5 第 3 / 8 条坐实 E3D_Core 在换 owner / 祖先改位姿时**不**重算后代条目 | 与 E3D_Core 一致（不多不少，e3d-io 写的与 E3D 写的逐字可比） | 照样重算后代（空间搜索始终按当前位置，但与 E3D 对同一改动写出的表不逐字相同） |

**细化时采用的默认**（每条可被用户推翻）

- 需求 1：09-20 / 09-22 的旧取证文档不迁到 IB_Wrapper；改内存 IDB 的 SQL 写放行（不改 SHA-256）；IB_Wrapper 只在自身进程环境里指定所选安装的 `src`（已核实 `ida-bridge-src.pth` 仍指向不存在的 `C:\Users\dpc\.agents\tools\ida-bridge\src`），不改 `.pth`、PATH 等用户配置；Phase_3_Evidence_Doc 另记 IDA 版本号。
- 需求 2：顺序按偏序——取证与读侧都早于首个写侧提交，不强制取证早于读侧；门问可放在 E3D_Gate_Set 任一脚本；发号下界改变 refno，不算字节中性，要有自己的门问。
- 需求 3：SPAMAP_Baseline 钉在原件会话 640（已核实原件保留会话 1–646，640 在链上；「640 的表即 09-22 所测那张」为推断，读数不符时改用仓内受控快照）；普查另报「按元素的 k 组合」计数。
- 需求 5 / 7：「改了」一律按本批结束状态与上一会话的值比较，不按「调用过 set」算；「跳过」下 Recompute_Set 中无 World_AABB 的元素删去旧条目、不留过期位置，也不写 SPAMAP_Record_Item；`Transaction::delete` 只删元素本身、不删后代（已核实 `src/write/transaction.rs`），本期不改。
- 需求 8：重铸后不以原 refno 登记，原号下属于占用者的条目保留；合并中调用方给的 World_AABB 在 Merge_Latest 改了该元素或其祖先位姿时失效；Merge_Latest 删了祖先使本批元素不可达时不拒写，只不写条目并报告。
- 需求 4：Key0 核对（第 10 条）只以条目自身的 6 个值字为输入，k = 1 条目亦然；条目数与 k 另立第 15 条，与 Key0、Spatial_Flags 分项报告；最短轴并列的取轴因语料不区分，只认 Core_IDB 出处；Spatial_Flags、条目数 / k 与需求 6 第 9 条的反例，只在会话链证明条目过期（此后的改动经需求 5 坐实不触发重算）时剔除。
- 需求 6：第 5 条改以 Recompute_Set 判定；调用方每元素只给一个 World_AABB，多次给出取最后一次，只作用于本批；给在不属于 Recompute_Set 的元素上不拒写，只报告为「未使用的 World_AABB」；非法值、给在非 Geometric_Noun 上在调用处即拒；自算所需属性无法解码时「跳过」下也拒写（与需求 7 第 12 条同一立场）。
- 需求 9：`aabb` 可用于 `insert` / `set` / `move`；报告按 refno 汇总并注明来源指令，「登记 / 删去」按新旧表的 11 字差集计；`--spatial` 取值非法按命令行参数错误处理。
- 需求 10：判别力对照记为 Q13，排在 Q13a 之前，Q13 FAIL 时 Q13a–Q13h 一律 FAIL；Q13d / Q13e 以 Probe_Element 为体积；Q13h 比元素原点。
- 需求 11：发号下界的门问记为 TTY_Gate Q14；Counter_Semantics 未坐实时默认构建按 `c61b097` 发号；`open_at(base)` 以 base 会话的索引与打开时读到的页 1 计下界。
- 需求 12：Staging_Time 只计 stage 阶段；Golden_Hashes 在优化前的父提交上录下并入库。
- 需求 13：区分写与不写页 1 的门问须在 `c61b097` 上 FAIL；合并时以 Merge_Latest 所对应的页 1 为起点。
- 需求 15：skip 不计为通过；因发号下界或 D3 默认而失败的既有测试只许改期望 refno 或显式指定 Spatial_Policy，逐条记入 Spec_Ledger。

## Glossary

- **E3D_Core**：E3D 3.1 的 `core.dll`，本期一切写侧行为的权威。
- **Core_IDB**：`D:\ida_scratch\plant3\core.dll.i64`（34 308 个函数，MSVC 符号 + FORTRAN 例程名）。
- **IDA_Bridge**：本机 ida-bridge 安装（当前 `C:\Users\dpc\.agents-old\tools\ida-bridge`，不在 PATH）及其 idalib 实例。
- **IB_Wrapper**：本期新建、受版本管理的仓内 ida-bridge 调用入口。
- **Target_IDB**：一次 IB_Wrapper 调用所针对的 IDB，取调用方给出的 IDB 路径，未给时为 Core_IDB。
- **IB_Xref_Helper**：`tools/ida/` 下受版本管理、经 IB_Wrapper 在已连接 IDA 实例内执行的交叉引用枚举助手。
- **Evidence_Doc**：`docs/evidence/YYYY-MM-DD-*.md` 取证文档，带「复现命令」节。
- **Phase_3_Evidence_Doc**：基线 `c61b097` 之后新增的 Evidence_Doc；此前入库的不受需求 1 第 7–9 条约束。
- **坐实 / 未坐实**：「坐实」须有下列出处之一：① Core_IDB 中实现该行为的函数地址，及反编译行号或反汇编地址（需求 1 第 8 条）；② 一条仓内受版本管理的命令在 Corpus（含门用项目副本中由 E3D_Session 写出的字节）里该行为适用的全部实例上复现该行为、0 反例，Evidence_Doc 在「复现命令」节给出该命令、在「结论」节给出「一致数 / 总数」。没有上述出处，或按 ① 读出的规则在 Corpus 上有反例，即为「未坐实」。
- **Phase_3_Workflow**：本期的开发流程：core 取证 → 读侧 → 写侧 → 真 E3D 门。
- **Spec_Ledger**：本 spec 目录下的文档（requirements / design / tasks）；本期的决策、实测数字与留待项记于此。
- **写侧能力**：本期改动 `src/write/` 的四项能力——SPAMAP 维护（需求 5 第 6 条、需求 6 第 2–8、10–15、17 条、需求 7–9）、发号下界（需求 11）、staging 优化（需求 12）、页 1 桶表写侧（需求 13 第 3、6、7、9、10 条；仅当需求 13 第 3 条的条件成立时存在）。
- **首个写侧提交**：对一个写侧能力，`c61b097` 的后代提交中最早的一个：它改动 `src/write/` 下的文件，且提交说明首行含该能力在 Spec_Ledger tasks 文档中的任务号。
- **字节中性**：写侧能力的一种性质：对 `c61b097` 时 `tests/write_stage*.rs` 中的每个写入批次，该能力改动前后写出的文件逐字节相同。
- **Corpus**：本机 `D:\AVEVA\Projects\E3D3.1` 下的装机库，至少含 `ams8000_0001`、`ams7999_0001`、`ams7351_0001`、`ams251142_0001`，以及 `TES023` 与门用的项目副本。
- **E3D_Authored_Element**：Corpus 中最新版本记录所在会话由 E3D 写出的元素；e3d-io 写出的会话以会话 USER 为 `e3dio` 认定并排除——门与测试夹具写的会话固定用这个 USER。
- **SPAMAP**：E3D 的空间映射表，即会话页表目录中以属性 hash 230902453 命名的 B+ 树（页类型 5，键 4 字，叶值 7 字，内结点 4 键字 + `(pgno, ext)`）。
- **SPAMAP_Entry**：SPAMAP 的一条叶条目：键 `[Key0, refno.word0, refno.word1, k]`，值 `[minx, miny, minz, maxx, maxy, maxz, Spatial_Flags]`。
- **哨兵槽**：B+ 树页中 4 个键字均为 `0x80000001` 的槽，不是 SPAMAP_Entry，不计入条目数。
- **SPAMAP_Record_Item**：元素记录中属性 hash 为 230902453 的值，位于 implicit 区或 explicit 流。
- **World_AABB**：元素在世界系（WORL）下的轴对齐包围盒；接口上以毫米给出，编进 SPAMAP_Entry 时为 0.1 mm 整数。
- **Key0**：键首字，`N × 10^8 + 最短轴 min`（0.1 mm），`N = 6 × 轴序（x 0 / y 1 / z 2）+ 桶`。
- **桶**：N 中 1–5 的分段。
- **(轴序, 桶)（普查口径）**：对一条 SPAMAP_Entry，把 Key0 与值的前 6 字都按有符号 32 位整数读；若某轴（x / y / z，轴序 0 / 1 / 2）与某个 b ∈ {1, …, 5} 使 `Key0 − 该轴 min = (6 × 轴序 + b) × 10^8`，则 (轴序, 桶) 为 (该轴轴序, b)；多根轴满足时取轴序最小者；都不满足记为「Key0 不符」。
- **Spatial_Flags**：SPAMAP_Entry 值的第 7 字。
- **Geometric_Noun**：E3D_Core 为其元素维护 SPAMAP_Entry 的 noun 集合。
- **SPAMAP_Recompute**：E3D_Core 删去一个元素的全部旧 SPAMAP_Entry、并按改动后状态登记新条目。以 E3D_Session 实测时，用 SPAMAP_Reader 比较改动前的最新会话与改动后 `SAVEWORK` 所得会话中该元素的条目：条目改变、或原无条目的元素出现条目，判为发生；改动使 World_AABB 改变而条目逐字不变、或原无条目的元素仍无条目，判为未发生；其余观测不作结论。
- **重算属性**：需求 5 第 2 条重算表「自身」一类中取值「触发」的属性；取值「未坐实」的行按 D7 处理。
- **Built_In_AABB**：e3d-io 自行计算 World_AABB 的 noun 集合（D1 推荐默认：BOX、CYLI）。
- **Spatial_Policy**：Transaction 对「Geometric_Noun 元素无 World_AABB」的处理：「要求」（默认，拒写整批）、「跳过」（该元素不写条目并报告）、「关闭」（整批不维护 SPAMAP，= 0.4.0 行为）。
- **SPAMAP_Reader**：读侧的 SPAMAP 表读取器。
- **SPAMAP_Codec**：由元素与 World_AABB 得出 SPAMAP_Entry 集合（键、值、条目数、k）的编码。
- **SPAMAP_Writer**：写侧在一个会话中维护 SPAMAP 表的部分。
- **Corpus_Census**：对 Corpus 做 SPAMAP 普查的命令。
- **SPAMAP_Baseline**：`ams8000_0001` 装机原件——不是门用项目副本——会话 640 的 SPAMAP 表，即 2026-09-22 SPAMAP 取证所用门副本的底座会话；按会话号精确选取，不随原件此后追加的会话变化；原件会话链上没有会话 640 时读取报错，不改读其他会话。
- **Transaction**：`e3d_io::write::Transaction` 门面。
- **Merge_Commit**：stage 时文件最新会话不是 base 会话、因而把本批经三方合并写在该最新会话之上的一次提交；成功时 `TransactionCommit::merged_onto` 非空；dry run 的 stage 按同一判定。
- **Merge_Latest**：Merge_Commit 该次 stage 所基于的文件最新会话；因 `WriteError::Moved` 重新 stage 时为重新 stage 那一刻的最新会话；成功提交时等于 `TransactionCommit::merged_onto`。
- **Merged_State**：Merge_Commit 写入新会话的元素状态：Merge_Latest 的元素记录与成员树，叠加本批经三方合并后的改动（两边改不同属性时两边的改动都在；同属性撞车在 `OnClash::WriterWins` 下取本批的值），即提交后读侧在新会话上读出的状态；它只由元素记录的合并决定，SPAMAP 条目在 base 与 Merge_Latest 之间的差异不构成冲突或撞车。
- **上一会话**：新会话直接写在其上的会话：非合并提交时即 Transaction 的 base 会话；合并提交时为 Merge_Latest。
- **本批结束状态**：本批全部写入按调用顺序应用于上一会话之后——合并提交时即 Merged_State——各元素的存在与否、属性值与 owner。
- **从 WORL 可达**：在本批结束状态下由该元素沿 owner 链逐级上溯，每一级的 refno 都在新会话索引中，且止于 WORL。
- **Recompute_Set**：一个批次中 SPAMAP_Writer 须重定条目的元素集合，在本批结束状态上求，插入元素按最终写入的 refno 计。Spatial_Policy 为「关闭」时为空集；否则为下列元素之并：① 本批插入的每个 Geometric_Noun 元素；② 上一会话已有、至少一个重算属性在本批结束状态的值与上一会话不同的每个 Geometric_Noun 元素；③ 上一会话已有、本批结束状态的 owner 与上一会话不同的每个 Geometric_Noun 元素；④ 上一会话已有、owner 在本批结束状态与上一会话不同的元素，其本批结束状态下的 Geometric_Noun 后代中需求 5 第 3 条坐实 E3D_Core 会随之重算的每一个；⑤ 上一会话已有、POS 或 ORI 在本批结束状态与上一会话不同的元素，其本批结束状态下的 Geometric_Noun 后代中需求 5 第 8 条坐实 E3D_Core 会随之重算的每一个（④⑤ 含合并时 Merge_Latest 此间挂到其下的元素，不含此间删去或移走的）；⑥ 本批删除的每个元素（不论 noun，含 Transaction 随之删除的后代），以及本批结束状态下 owner 链经过这些元素的每个元素。
- **Target_Entries**：Recompute_Set 中一个元素在新会话 SPAMAP 表中应有的条目：该元素在本批结束状态存在、从 WORL 可达、noun 属于 Geometric_Noun 且有 World_AABB（需求 6）时，为 SPAMAP_Codec 由其本批结束状态与该 World_AABB 算出的全部条目（条数与 k 按需求 4）；否则为空。
- **Write_CLI**：`e3d-write apply` 批量指令 CLI。
- **TTY_Gate**：`tools/e3d/write_gate.ps1`（真 E3D 3.1 Design TTY，现 Q0 + 28 问）。
- **E3D_Gate_Set / 门问**：E3D_Gate_Set 指 TTY_Gate、`tools/e3d/alternate_write_gate.ps1`、`tools/e3d/format0_gate.ps1` 三个门脚本（D5 改选备选时加上 `spatial_gate.ps1`）；门问指其中一项以 `PASS <问号>` 或 `FAIL <问号>` 输出结果的自动检查；人工 E3D 图形会话不算门问。
- **E3D_Session**：由 TTY_Gate 或人工驱动的真 E3D 3.1 Design 会话。
- **Scale_Probe**：规模测量命令（第二期 T2 口径）。
- **Bucket_Table**：页 1 桶表（`meta::bucket::BucketTable::parse(page1)`；页形 `[1][0xFFFFFFFF][2+3n]` 后接 n 条 `(extent_word0, Bucket_Counter, 0)`），每个会长的次扩展一条，主扩展与纯字典库无条目。
- **MISSION**：仓内 `MISSION.md`。
- **Release_Docs**：仓内 `README.md` 与 `CHANGELOG.md`。
- **发布提交 / 默认构建**：发布提交指提交说明首行为 `chore(release): x.y.z`、并把 `CHANGELOG.md` 的 `Unreleased` 节归档为 `x.y.z` 版本节的提交；默认构建指在该提交上以 `Cargo.toml` 默认 feature 集、不加 `--features` 构建的 crate。
- **Build_Checks**：`cargo test --no-fail-fast`、`cargo clippy --all-targets`、`cargo doc`。
- **Downstream_Crates**：`e3d-model` 及其 verify 变体、`gen-model`。
- **Q4**：第二期定的规模阈值：1 万 insert 单会话 ≤ 10 s，追加页 ≤ 1.5 × 记录页，`Transaction::open` 在最大装机库 ≤ 1 s。
- **提交报告**：Transaction 成功提交时返回的 `TransactionCommit`；dry run 时为 stage 所得的同类结果。Write_CLI 的 `--report` 由它写出。
- **可用 World_AABB**：Recompute_Set 中一个元素在本批所用的 World_AABB：调用方在本批为它给出、且未按需求 8 第 6 条失效的值（多次给出取最后一次，需求 6 第 4、11 条）；没有这样的值而 noun 属于 Built_In_AABB 时，为按需求 6 第 2 条算出的值；否则没有。需求 6 第 14、15 条所述的元素视为没有。Target_Entries 及需求 6–8 所说的「有 / 无 World_AABB」均按此认定。
- **Codec_Check**：Evidence_Doc「复现命令」节中以 Corpus 为输入、逐条核对 SPAMAP_Codec 的仓内受版本管理命令（需求 4 第 10、11、15、16 条，需求 6 第 9 条）。
- **Bucket_Counter**：Bucket_Table 中一个扩展条目的第 2 字（`next_word1`）。
- **Counter_Semantics**：Bucket_Counter 的含义，二者取一：「下一个待发」（E3D_Core 为该扩展发出的下一个 word1 等于 Bucket_Counter）或「已发高水位」（等于 Bucket_Counter + 1）；由需求 11 第 7 条坐实。
- **Bucket_Floor**：Bucket_Table 列出的一个扩展按 Counter_Semantics 由其 Bucket_Counter 得出的最小可发 word1：「下一个待发」时为 Bucket_Counter，「已发高水位」时为 Bucket_Counter + 1。
- **首个待发 word1**：Transaction 在一个扩展上为本批第一个插入元素发出的 word1。
- **Open_Fast_Path**：`Transaction::open` 不遍历全部索引键即得出各扩展首个待发 word1 的实现；其朴素对照为全索引扫描。
- **热文件缓存**：计时前已把被测文件完整读过一遍、其间没有其他大文件读写的状态；计时一律用 release 构建、连续三次取中位数。
- **Staging_Time**：Scale_Probe 一次运行中 stage 阶段的墙钟时间（第二期 T2 的 stage 列口径，不含 open、build、commit）。
- **Golden_Hashes**：staging 优化的首个写侧提交之前、在其父提交上录下的既有写路径测试批次与 Scale_Probe 批次输出文件的 SHA-256 集合（固定 `SessionRequest.clock` 与预铸 refno）。
- **Gate_Ops**：`tools/e3d/write_gate_ops/*.json`，TTY_Gate 为每个 e3d-io 会话各备一份的 `e3d-write apply` 指令文件。
- **Probe_Element**：TTY_Gate 空间搜索用作体积的元素：世界位置已知（原生元素，或 Gate_Ops 以 Spatial_Policy「要求」写入的 BOX），在本次门运行中不再改动。
- **Control_Box**：Q13 中由 e3d-io 以 Spatial_Policy「关闭」写入、没有 SPAMAP_Entry 的 BOX。
- **Q13_Box**：Q13a–Q13h 中由 e3d-io 以 Spatial_Policy「要求」写入的 BOX（不含 Control_Box）。

## Requirements

### Requirement 1: ida-bridge 仓内调用约定

**User Story:** 作为接班的开发会话，我想用仓内一个固定入口驱动 ida-bridge 读 `core.dll`，以便取证文档里的复现命令在安装目录改名、`%TEMP%` 被清、实例号变化之后照样能跑。

#### Acceptance Criteria

1. WHEN 调用方运行 IB_Wrapper（`tools/ida/` 下受版本管理的脚本），THE IB_Wrapper SHALL 用「环境变量 `IDA_BRIDGE_HOME` → `C:\Users\dpc\.agents-old\tools\ida-bridge` → `C:\Users\dpc\.agents\tools\ida-bridge`」中第一处同时含 `src\ida_bridge` 目录与 `.venv\Scripts\python.exe` 的 IDA_Bridge 安装，执行调用方给出的 ida-bridge 命令；PATH 中没有 `ida-bridge`、venv 内 editable 安装所记的路径已失效时亦然。
2. IF 查找顺序中没有一处同时含 `src\ida_bridge` 目录与 `.venv\Scripts\python.exe`，THEN THE IB_Wrapper SHALL 以非零退出码退出，并按查找顺序逐条打印查过的位置及该处缺少的项（`IDA_BRIDGE_HOME` 未设置时注明「未设置」）。
3. WHEN 调用方经 IB_Wrapper 发起 `exec`、`reset` 或 `supervisor stop` 而未给实例号，且恰有一个已连接实例打开 Target_IDB、该实例未被另一个 stateful session 占用（定义见第 10 条），THE IB_Wrapper SHALL 把该调用转发给这个实例，使调用方无需写死 `idalib-<pid>`；「打开 Target_IDB」指实例所报的 IDB 路径与 Target_IDB 的绝对全路径相同，比较时不区分大小写，`/` 与 `\` 视为相同。
4. IF IDA_Bridge 桥服务不可达（连不上所配的桥地址），或桥服务可达但没有已连接实例打开 Target_IDB，THEN THE IB_Wrapper SHALL 以非零退出码退出，指明属于哪一种情形（前者附所试的桥地址），并按执行顺序打印让 Target_IDB 上线的完整 IB_Wrapper 命令（前者先列起桥服务的命令；两种情形都列在 Target_IDB 上启动 idalib 实例的命令）。这些命令按序原样执行后，重发同一调用即按第 3 条被转发。Target_IDB 文件不存在时，另指明该文件不存在，且不打印启动 idalib 实例的命令。
5. IF 经 IB_Wrapper 的调用带 `--save`、是 `supervisor save`、其 `--out-idb` 按第 3 条的比较规则指向 Core_IDB，或其 `-c` 文本 / `-f` 文件内容含 `idb.save` 或 `save_database`，THEN THE IB_Wrapper SHALL 以非零退出码拒绝该调用，打印被拒原因，且不向 IDA_Bridge 发出任何请求，使只经 IB_Wrapper 的一轮取证（从首条调用之前到末条调用结束之后）前后 Core_IDB 的 SHA-256 相同。
6. WHEN 调用方经 IB_Wrapper 对一个目标地址运行 IB_Xref_Helper，THE IB_Xref_Helper SHALL 在 60 s 内返回引用该地址的全部代码引用（不含顺序流）与数据引用，每条含引用源地址与所在函数名（不在函数内时为空）。枚举在取到 64 位 `BADADDR`（`0xFFFFFFFFFFFFFFFF`）或已取满 100 000 条时终止，后一种情形在结果中标明已截断。未截断时，所返回的引用源地址集合与 ida-bridge `xrefs` 表中同一 `to_ea`、去掉 `flow` 类后的 `from_ea` 集合相同。
7. THE Phase_3_Evidence_Doc SHALL 在「复现命令」节中只以 IB_Wrapper、按仓库根相对路径调用的仓内脚本与 `cargo` 命令为命令入口，所用 SQL / IDAPython 文件都位于 `tools/ida/` 下且已受版本管理（内联的 `--sql` / `-c` 文本可直接写在命令里，Corpus 与 Core_IDB 路径可作命令参数）；该节不出现临时目录路径（`%TEMP%`、`$env:TEMP` 或其展开值）、IDA 实例号（`idalib-<数字>`、`idaui-<数字>`）与 IDA_Bridge 安装目录路径。
8. THE Phase_3_Evidence_Doc SHALL 为其陈述的每一条 E3D_Core 行为给出所在函数的起始地址，以及该行为在反编译输出中的行号（ida-bridge `pseudocode` 表的 `n`）或其指令地址；地址均以十六进制书写。
9. THE Phase_3_Evidence_Doc SHALL 记录取证时 Core_IDB 的 SHA-256（64 位十六进制）、IDA 版本号，以及仓库 HEAD 的完整提交号（40 位十六进制）。
10. IF 调用方经 IB_Wrapper 发起 `exec`、`reset` 或 `supervisor stop` 而未给实例号，且打开 Target_IDB 的已连接实例多于一个，或唯一的那个被另一个 stateful session 占用（其占用 session id 非空且不等于调用方所给的 `--session-id`；调用方未给时，任何非空占用都算），THEN THE IB_Wrapper SHALL 以非零退出码退出，不向任何实例转发该调用，并逐个打印这些实例的实例号、PID 与占用 session id（无占用时注明「无」）。
11. WHEN IB_Wrapper 转发的 ida-bridge 命令结束，THE IB_Wrapper SHALL 以该命令的退出码退出。
12. IF 被转发的 `exec` 在时限内未返回（时限取调用方所给的 `--timeout-s`，未给时为 60 s；`--timeout-s 0` 表示不设时限，本条不适用），THEN THE IB_Wrapper SHALL 在调用开始后「时限 + 10 s」之内以非零退出码退出，打印超时实例的实例号以及停止、重开该实例的完整 IB_Wrapper 命令，且不重发该调用、不停止该实例。

### Requirement 2: 取证先行的切片顺序

**User Story:** 作为仓库维护者，我想让本期每个写侧能力都按「core 取证 → 读侧 → 写侧 → 真 E3D 门」的顺序落地，以便写出的每个字都能指回 `core.dll` 或语料字节。

#### Acceptance Criteria

1. WHEN 一个写侧能力的首个写侧提交落地，THE Phase_3_Workflow SHALL 使该提交的父提交（合并提交取第一父提交）中已有一份 Evidence_Doc，其「结论」节至少写明一个实现该能力的 E3D_Core 例程（函数地址，按需求 1 第 8 条）和一项 Corpus 核对（所用库、「复现命令」节中的对应命令、一致数 / 总数）；`c61b097` 之前已有的 Evidence_Doc（如 `docs/evidence/2026-09-22-core-dll-refno-allocation.md`）按需求 1 第 7–9 条补齐后计入。
2. WHEN 一个写侧能力的首个写侧提交落地，THE Phase_3_Workflow SHALL 使同一父提交中已有该能力所写每种盘上结构的读侧解码，该解码不在 `src/write/` 下、不调用 `src/write/` 中的代码，且在该父提交上至少有一条以 Corpus 库为输入、对其解码结果作断言的 `cargo test` 测试 0 失败且未跳过（`c61b097` 已有的读侧解码，如 Bucket_Table，计入）。
3. WHEN Spec_Ledger 把一个写侧能力记为完成，THE E3D_Gate_Set SHALL 含至少一个 Spec_Ledger 为该能力列出、且 `c61b097` 的门脚本中没有的门问；该门问让 E3D_Session 读取该能力在本次门运行中写出的数据、以 E3D_Session 的回答判定结果，并在该记录所在的提交上运行时输出 `PASS <问号>`（D5）。
4. IF 一个写侧能力所依赖的 E3D_Core 行为未「坐实」，THEN THE Evidence_Doc SHALL 把该行为标为「未坐实」，并写明所作假设、须用到该行为的输入类别（如「最短轴 extent 为 0 的 BOX」）与 Corpus 上的反例（库名与个数；若有）。
5. WHEN Spec_Ledger 把一个写侧能力记为完成，THE Phase_3_Workflow SHALL 使该记录所在的提交中，该能力所写每种盘上结构都有至少一条 0 失败且未跳过的 `cargo test` 测试：在 Corpus 库副本上经 Transaction 提交，只用第 2 条的读侧解码读回该结构，并断言读回值等于该能力所属需求规定的期望（如需求 7 第 9 条、需求 11 第 5 条、需求 13 第 6 条）；读回值只取自对提交后文件的解码，不取自 Transaction 或 `src/write/` 的返回值。
6. IF 一个发布提交上仍有写侧能力所依赖的「未坐实」行为，THEN THE Transaction SHALL 在该提交的默认构建中不执行依赖该行为的写侧逻辑：须用到该行为才能确定其 SPAMAP_Entry 的元素视同无可用 World_AABB，按 Spatial_Policy 处理（默认「要求」即拒绝整批、逐个列出 refno、文件一字不改，D3）；依赖该行为的其他结构按 `c61b097` 的行为写出（如发号取全索引扫描所得最大 word1 + 1、不写页 1）。
7. IF 一个写侧能力是字节中性的，THEN THE Phase_3_Workflow SHALL 以「其逐字节对比测试在 `cargo test --no-fail-fast` 下 0 失败且未跳过」与「E3D_Gate_Set 在该能力记为完成的提交上全数 PASS」作为该能力的验收，不对其适用第 1、2、3、5 条。

### Requirement 3: SPAMAP 读侧与语料普查

**User Story:** 作为写侧开发者，我想用本 crate 的读侧完整读出任意库的 SPAMAP 表并做全量普查，以便写侧有逐字的验收方、规则拟合有全量语料。

#### Acceptance Criteria

1. WHEN 调用方读取一个表目录列出 SPAMAP 的会话（最新会话，或按会话号指定的会话）的 SPAMAP 表，THE SPAMAP_Reader SHALL 从该会话表目录所指的根出发，按键序（4 个键字逐字按无符号 32 位比较的升序）返回表中每条叶条目。每条是盘上原值的 4 个键字与 7 个值字，不含哨兵槽。
2. IF 自 SPAMAP 根可达的任一页出现下列任一情形：页读不出；页类型不是 5；名 hash 不是 230902453；键宽不是 4；层号为 0 的页值宽不是 7；空闲字数超过页头 7 字之后的字数；已用字数（页头之后的字数减空闲字数）不是条目步长（叶 11 字、内结点 6 字）的整数倍；子页层号不等于父页层号减 1；内结点页没有子页；同一页第二次被到达；相邻两条非哨兵叶条目（含跨页相邻）的键不是严格升序，THEN THE SPAMAP_Reader SHALL 以一个错误结束该次读取，错误指明页号与出错项（所声明的值，或所违反的关系），调用方得到的是该错误而不是截短的条目列表。
3. WHEN SPAMAP_Reader 读 SPAMAP_Baseline，THE SPAMAP_Reader SHALL 读出 244 页（自根可达的内结点页与叶页合计）、3 746 条叶条目，其中不同 refno 3 628 个、k = 0 的 3 625 条、k = 1 的 121 条。
4. WHEN 调用方以一个 refno 查询 SPAMAP 表，THE SPAMAP_Reader SHALL 按键序返回键中 refno（第 2、3 键字）等于该 refno 的全部叶条目（可为空列表；空列表不表示读取失败）。
5. FOR ALL Corpus 各库最新会话中的 SPAMAP 表，对表中出现的每个 refno 及至少 1 个未在表中出现的 refno，按 refno 查询的结果 SHALL 与全表遍历（第 1 条）按该 refno 过滤并保持键序的结果逐条相同（模型对照性质）。
6. FOR ALL Corpus 各库最新会话中的 SPAMAP 表，把 SPAMAP_Reader 读出的条目序列由 SPAMAP_Writer 按该库页长编码成页，再由 SPAMAP_Reader 读回，SHALL 无错误地得到与原序列逐条 11 字相同、顺序相同的序列（往返性质）。本性质只约束「条目序列 → 页 → 条目序列」，不要求重编的页与盘上页逐字节相同。
7. THE Corpus_Census SHALL 为每个输入库输出路径、所读会话号、SPAMAP 根页号、页数（自根可达）与条目数；所读会话的表目录无 SPAMAP 时，后三项标为「无 SPAMAP 表」，该库元素全部计入 0 条。
8. THE Corpus_Census SHALL 由仓内受版本管理的一条命令运行，并满足：输入为库文件或目录（目录中首部库描述不可解析的文件列为「非库文件」并跳过，不算错误）；可为单个库指定会话号，缺省读最新会话；以只读方式打开输入库，运行前后各库文件逐字节不变；所读会话号相同的两次运行输出逐字节相同，使 Evidence_Doc 引用的普查输出能由其「复现命令」逐字节重现。
9. IF 所选会话的表目录没有 SPAMAP，THEN THE SPAMAP_Reader SHALL 返回指明该会话号的「无 SPAMAP 表」结果，且该结果与「表存在、叶条目为 0」的结果可区分。
10. IF 一条 SPAMAP_Entry 键中的 refno 不在所读会话的 refno 索引中（元素已删或从未存在），THEN THE Corpus_Census SHALL 把该条目计入该库的「悬空」一栏、不计入任何 noun，并输出该库悬空条目总数与按键序的前 100 条（键字与值字）。
11. IF 一个输入库无法打开、调用方为它给出的会话号不在其会话链上，或读取其 SPAMAP 表或元素记录时出错，THEN THE Corpus_Census SHALL 在输出中列出该库路径与错误、不输出该库的统计，继续普查其余输入库，并在全部输入处理完后以非零退出码结束。
12. THE Corpus_Census SHALL（承第 7 条）另逐 noun 输出：该 noun 在所读会话 refno 索引中的元素按条目数 0 / 1 / 2 / ≥ 3 分列的个数（noun 取自元素记录），以及这些元素全部条目的 (轴序, 桶)、Spatial_Flags、k 直方图。「Key0 不符」在 (轴序, 桶) 直方图中单列一格，并按键序列出其前 100 条的键字与值字。

### Requirement 4: SPAMAP 键与值的编码规则

**User Story:** 作为写侧开发者，我想要一套由 `core.dll` 坐实、并在全量语料上逐条复现的 SPAMAP 键与值编码，以便 e3d-io 为一个几何写的条目与 E3D 为同一几何写的条目逐字相同。

#### Acceptance Criteria

1. THE Evidence_Doc SHALL 为最短轴、桶、Key0、条目数与 k、Spatial_Flags 各指出 E3D_Core 中计算它的例程（起始地址及行号或指令地址，按需求 1 第 8 条），以同样格式指出把毫米坐标化为 0.1 mm 整数的取整点，并写明最短轴与桶按取整前的毫米值还是按取整后的 0.1 mm 整数判定。
2. THE SPAMAP_Codec SHALL 按 Evidence_Doc 坐实的 min / max 取整规则，把 World_AABB（毫米）编为值的前 6 字 `[minx, miny, minz, maxx, maxy, maxz]`：0.1 mm 整数，可为负（语料中原生 BOX 的 minx 有 −30024）。
3. THE SPAMAP_Codec SHALL 把 Key0 编为 `N × 10^8 + 最短轴的 min`，其中 min 取第 2 条所编的值字，`N = 6 × 轴序 + 桶`。
4. WHEN 两根或三根轴的 extent 并列最小，THE SPAMAP_Codec SHALL 取 Evidence_Doc 坐实的那根轴作最短轴（SPAMAP_Baseline 有 538 条最短轴并列的条目，Corpus 不区分取哪根，按第 18 条只能以 Core_IDB 坐实）。
5. WHEN 最短轴的 extent 为 0，THE SPAMAP_Codec SHALL 按 Evidence_Doc 坐实的规则确定该元素各条目的桶与 Key0。
6. THE SPAMAP_Codec SHALL 按 Evidence_Doc 坐实的规则确定桶，含相邻两桶分界值的归属，以及最短轴 extent 超出语料所见范围时的取值（SPAMAP_Baseline 所见 N：x 1–4、y 7–10、z 13–17）。
7. THE SPAMAP_Codec SHALL 按 Evidence_Doc 坐实的规则（第 14 条）确定一个元素的条目数、每条的 k 与 k ≥ 1 条目的 6 个 AABB 值字。
8. THE Evidence_Doc SHALL 为 Corpus 中出现过的 Spatial_Flags 值（0 / 2 / 3 / 6 / 7）的每一位写明来源属性或计算，并逐值给出 Corpus 各库的条目数。
9. THE SPAMAP_Codec SHALL 按 Evidence_Doc 坐实的规则，由元素记录及 Evidence_Doc 为该规则列出的其他输入计算 Spatial_Flags。
10. FOR ALL SPAMAP_Baseline 与 Corpus 各库最新会话 SPAMAP 表中的条目，SPAMAP_Codec 只以该条目自身的 6 个 AABB 值字为输入（k = 1 的条目同样只用自身的 6 字，不取同一元素其他条目的值字）按第 3–6 条算出的 Key0 SHALL 与盘上第 1 键字相同（纯编码核对；每库一致数等于条目数，SPAMAP_Baseline 3 746 / 3 746）。
11. FOR ALL 键 refno 为 E3D_Authored_Element 的 Corpus 条目，SPAMAP_Codec 按第 9 条由该元素记录算出的 Spatial_Flags SHALL 与盘上第 7 值字相同（反例只按第 17 条剔除）。
12. IF 一个 World_AABB 的坐标超出 Evidence_Doc 按第 13 条坐实的可表示范围（Key0 与 6 个值字都须可表示；例：若键字按有符号 32 位存储与比较，N = 17 的元素须满足 `17 × 10^8 + min ≤ 2 147 483 647`，即 min ≤ 447 483 647，约 44.7 km），THEN THE SPAMAP_Codec SHALL 拒绝该元素，错误中指明 refno、越界的轴、坐标值（毫米）与可表示范围。
13. THE Evidence_Doc SHALL 坐实 SPAMAP 键字与值字按有符号还是无符号 32 位解释、E3D_Core 在该 B+ 树中比较键时按有符号还是无符号比较，以及 E3D_Core 计算 Key0 的整数宽度与越界处理。
14. THE Evidence_Doc SHALL 坐实 k 的含义、一个元素何时有 k = 1 条目、只有 k = 1 条目的元素为何没有 k = 0 条目（SPAMAP_Baseline：k = 0 的 3 625 条、k = 1 的 121 条，有 2 条的元素 118 个，故至少 3 个元素只有 k = 1 条目），以及 k = 1 条目的 6 个 AABB 值字从何算出。
15. FOR ALL 键 refno 为 E3D_Authored_Element 且有 SPAMAP_Entry 的 Corpus 元素，SPAMAP_Codec 按第 7 条、以 Evidence_Doc 为该规则列出的输入算出的条目数与各条 k SHALL 与盘上相同（反例只按第 17 条剔除）。
16. THE Codec_Check SHALL 对每个输入库分别输出 Key0（第 10 条）、条目数与 k（第 15 条）、Spatial_Flags（第 11 条）三项核对的一致数 / 总数，并各按键序列出前 100 个反例的键字与值字；一项的反例不计入另两项。
17. IF 第 11、15 条或需求 6 第 9 条的核对出现反例，THEN THE Evidence_Doc SHALL 逐个列出反例；只有以会话链证明「该条目写于该元素或其祖先此后的某次改动之前，且该改动按需求 5 第 2、3、8 条坐实为不触发 SPAMAP_Recompute」的反例可从该核对的适用实例中剔除，其余反例使所核规则为「未坐实」。
18. IF 第 2–9 条中一条规则在 Corpus 中没有适用实例，或 Corpus 的实例不能区分 Evidence_Doc 所列的候选规则（如最短轴并列时取哪根轴），THEN THE Evidence_Doc SHALL 只以 Core_IDB（「坐实」出处 ①）坐实该规则，不能以 ① 坐实时标为「未坐实」（需求 2 第 4 条）。
19. IF 为一个元素编码须用到 Evidence_Doc 标为「未坐实」的规则，THEN THE SPAMAP_Codec SHALL 在该元素的编码结果中标明所依的每条未坐实规则（Evidence_Doc 中的规则名），供 Transaction 按需求 2 第 6 条（D7）与需求 6 第 14 条处理该元素。
20. WHEN SPAMAP_Codec 以 BOX `24384/26484` 在 E3D 为其写出条目的会话中的状态为输入（301 × 400 × 500，World_AABB x ∈ [849.5, 1150.5]、y ∈ [1800, 2200]、z ∈ [2750, 3250] mm，其余属性取该会话中的值并记于 Evidence_Doc），THE SPAMAP_Codec SHALL 得出恰好一条条目：键 `[300008495, 24384, 26484, 0]`，值 `[8495, 18000, 27500, 11505, 22000, 32500, 2]`（最短轴 x、桶 3）。

### Requirement 5: SPAMAP 覆盖范围与重算时机

**User Story:** 作为写侧开发者，我想知道 `core.dll` 对哪些 noun 维护 SPAMAP、哪些改动让它重算条目，以便 e3d-io 在同样的时机维护同样的条目，不多不少。

#### Acceptance Criteria

1. THE Evidence_Doc SHALL 坐实 E3D_Core 登记 SPAMAP_Entry 前检查的判定条件，写明被检查的每个因素（noun，以及 noun 之外的每个因素，如元素属性值、库类型）。
2. THE Evidence_Doc SHALL 为每个 Geometric_Noun 给出一张重算表，每行一项输入，写明改动该项后 E3D_Core 是否对该元素做 SPAMAP_Recompute。取值为「触发」、「不触发」或「未坐实」；「未坐实」须附所用假设。表中的行覆盖两类：
   - ①「自身」：该 noun 模板中的每个属性（属性名与 hash，含 NAME），以及该元素在 owner 成员表中的次序；
   - ②「外部输入」：E3D_Core 计算该元素 World_AABB 时读取的其他元素的每个属性（如目录引用 SPRE、CATR 所指元素的属性；owner 链上的 POS / ORI 归第 8 条）。

   已知 BOX 的 `XLEN` 为「触发」（2026-09-22，会话 648）。
3. THE Evidence_Doc SHALL 坐实：E3D_Core 把一个元素挪到另一 owner 下时，是否在该改动所在会话中对下列各层的 Geometric_Noun 元素做 SPAMAP_Recompute。须逐层分别写明：被挪元素自身（为 Geometric_Noun 时）、其直接成员、第 2 层及更深的后代。
4. THE Evidence_Doc SHALL 坐实：E3D_Core 删除一个元素后，该删除所在会话的 SPAMAP 表中是否仍有键中 refno 为被删元素或其任一后代的条目。须逐层分别写明：被删元素自身（为 Geometric_Noun 时）、其直接成员、第 2 层及更深的后代。
5. THE Evidence_Doc SHALL 逐 Geometric_Noun 坐实：E3D_Core 登记或重算该 noun 元素的条目时，是否在其记录中写入 SPAMAP_Record_Item。若写入，还须写明该项的类型码、字数、数据字布局、位置（implicit 区的 word offset，或 explicit 流中的次序），以及其数据字与该元素各条目键字（Key0、k）的对应关系。
6. IF Evidence_Doc 坐实 E3D_Core 为某 Geometric_Noun 的元素写入 SPAMAP_Record_Item，THEN THE SPAMAP_Writer SHALL 在登记或重算该 noun 元素条目的同一会话中写入其 SPAMAP_Record_Item，且所写的项与 E3D_Core 对同一元素状态写出的项逐字相同（核对集：Corpus 中全部带该项的 E3D_Authored_Element，0 反例）。
7. FOR ALL Corpus 中的 E3D_Authored_Element，「该元素在所在库最新会话的 SPAMAP 表中有至少 1 条 SPAMAP_Entry」SHALL 当且仅当「该元素满足第 1 条坐实的判定条件」（对账性质；由仓内命令逐元素核对，每个 Corpus 库 0 反例；`ams8000_0001` 中 EQUI / ZONE / SITE / GROU / FIXING 的元素无一满足该条件）。
8. THE Evidence_Doc SHALL 坐实：E3D_Core 改变一个元素的 POS 或 ORI 时，是否在该改动所在会话中对该元素的 Geometric_Noun 后代做 SPAMAP_Recompute。须按改动的属性（POS、ORI）与层（直接成员、第 2 层及更深的后代）分别写明。
9. THE Evidence_Doc SHALL 坐实：E3D_Session 删除一个没有 SPAMAP_Entry 的 Geometric_Noun 元素时，该删除能否不报错地经 `SAVEWORK` 提交；报错时记下 E3D 给出的错误号。被测元素如 0.4.0 写出的 BOX，直接删除它与删除它的 owner 各测一次。
10. THE Evidence_Doc SHALL（承第 1 条）按第 1 条坐实的判定条件，对 Corpus 各库所用 noun 模板中的全部 noun 列出 Geometric_Noun 清单，并逐 noun 注明 Corpus 中该 noun 的 E3D_Authored_Element 个数。个数为 0 的 noun 标「无语料实例」，其归属只能以 Core_IDB 坐实。
11. THE Evidence_Doc SHALL（承第 5 条）逐 Geometric_Noun 给出 Corpus 中该 noun 有条目的 E3D_Authored_Element 里带与不带 SPAMAP_Record_Item 的个数。

### Requirement 6: 世界系 AABB 的来源与缺失策略（D1 / D2 / D3）

**User Story:** 作为 e3d-io 的调用方，我想让本 crate 对 BOX、CYLI 自己算出世界系包围盒、对其余几何 noun 接受我给的包围盒，以便写进去的几何元素在 E3D 里空间可见，而本 crate 不承担目录几何求解。

#### Acceptance Criteria

1. THE Evidence_Doc SHALL 坐实 E3D_Core 由 POS / ORI 沿 owner 链复合世界系变换的方法（ORI 的存储形、复合顺序，以及 owner 链上哪些 noun 的 POS / ORI 参与复合，如 ZONE、EQUI、SUBE），每个 Built_In_AABB noun 由自身属性得出包围盒的方法（解析解、局部外包盒变换或离散面），以及尺寸属性为 0 或负时 E3D_Core 的处理。
2. WHERE 元素的 noun 属于 Built_In_AABB（D1），THE Transaction SHALL 由该元素在本批结束状态中的属性与 owner 链，按第 1 条坐实的方法算出 World_AABB，无需调用方输入。
3. THE Transaction SHALL 接受调用方为任一 Geometric_Noun 元素给出的 World_AABB（世界系毫米 `[minx, miny, minz, maxx, maxy, maxz]`）。
4. WHEN 调用方为一个 Built_In_AABB 元素给出 World_AABB，THE Transaction SHALL 使用调用方给出的值、不用第 2 条的自算值（该值在 Merge_Commit 中失效时按需求 8 第 6 条）。
5. IF Spatial_Policy 为「要求」，且 Recompute_Set 中有在本批结束状态存在、从 WORL 可达、noun 属于 Geometric_Noun 而没有可用 World_AABB 的元素，THEN THE Transaction SHALL 拒绝整批提交，文件逐字节不变，错误中逐个列出这些元素的 refno（插入元素按调用方所给 refno，Merge_Commit 中重铸的另附新号）及原因：没有调用方给值且 noun 不属于 Built_In_AABB；调用方给值按需求 8 第 6 条失效；须用到未坐实规则（第 14 条，附规则名）；条目须用到无法取得的输入（第 15 条）（D3；相对 0.4.0 是行为变化，见需求 14 第 4 条）。
6. WHERE Spatial_Policy 为「跳过」，THE Transaction SHALL 在提交报告中逐个列出第 5 条所述的元素及原因（口径同第 5 条）；这些元素在新会话表中的条目按需求 7 第 11 条处理（D3）。
7. WHERE Spatial_Policy 为「关闭」，THE Transaction SHALL 使新会话表目录中的 SPAMAP 项与上一会话相同（需求 7 第 6 条；合并时上一会话为 Merge_Latest，需求 8 第 1 条），且不因元素没有可用 World_AABB 或出现第 8 条的情形而拒写（即 0.4.0 行为，D3）。
8. IF Spatial_Policy 不为「关闭」，且为 Recompute_Set 中一个没有调用方给值的 Built_In_AABB 元素按第 2 条自算 World_AABB 所需的属性（自身或 owner 链上）无法被读侧解码，THEN THE Transaction SHALL 拒绝整批提交（「跳过」时亦然），文件逐字节不变，错误中指明元素 refno 与属性名及 hash。
9. FOR ALL SPAMAP_Baseline 中有 SPAMAP_Entry 的 Built_In_AABB noun 元素（含 BOX `24384/24777`：W 2821 N 8004 U 1400，312 × 590 × 550，绕 Z 转 5°、Z 朝下，世界系约 362 × 615 × 550 mm），以及 Corpus 各库最新会话中有条目的 Built_In_AABB noun 的 E3D_Authored_Element，由该元素及其 owner 链在所读会话中的状态按第 2 条算出的 World_AABB 经 SPAMAP_Codec 编成的条目 SHALL 与盘上该元素的条目逐字相同（条数、4 键字与 7 值字；反例只按需求 4 第 17 条剔除，判据只按第 16 条放宽）。
10. IF 调用方给出的 World_AABB 含非有限数（NaN、±∞），或某轴 min 大于 max，THEN THE Transaction SHALL 在该次调用返回错误（指明 refno 与违例的轴），草稿不变。
11. WHEN 调用方在同一批中为同一元素多次给出 World_AABB，THE Transaction SHALL 只用最后一次给出的值；此前批次给出的值不计入本批。
12. IF 调用方为 noun 不属于 Geometric_Noun 的元素给出 World_AABB，THEN THE Transaction SHALL 在该次调用返回错误（指明 refno 与 noun），草稿不变。
13. WHEN 调用方为一个元素给出了 World_AABB，而该元素不属于 Recompute_Set，或在本批结束状态不存在、从 WORL 不可达，THE Transaction SHALL 在提交报告中把该 refno 列为「未使用的 World_AABB」，且不因此改动该元素的条目。
14. WHERE 构建为发布提交的默认构建，IF 确定一个元素的 SPAMAP_Entry 须用到 Evidence_Doc 标为「未坐实」的规则（第 2 条自算所依的规则，如尺寸为 0 或负的 BOX、某种 ORI 存储形、owner 链中某类 noun 的位姿；或 SPAMAP_Codec 按需求 4 第 19 条标明的规则），THEN THE Transaction SHALL 视该元素没有可用 World_AABB（需求 2 第 6 条、D7），第 5、6 条为它列出的原因附该规则名。
15. IF 按需求 4 第 7 条，一个元素的条目须用到既非调用方给出、也不能由第 2 条自算的输入（如 k ≥ 1 条目的值字来源；调用方每元素只给一个 World_AABB），THEN THE Transaction SHALL 视该元素没有可用 World_AABB，第 5、6 条为它列出的原因注明所缺的输入。
16. IF Evidence_Doc 以 Core_IDB 坐实 E3D_Core 计算 World_AABB 的算术使第 9 条的逐字相同不可复现（如单精度浮点累积误差），THEN THE Spec_Ledger SHALL 记录第 9 条改用的判据（涉及的值字及每字容差，以 0.1 mm 计）与其 Evidence_Doc 出处，第 9 条按该判据核对。
17. WHEN 本批插入一个 `Q POS WRT /*` 为 E 1000 N 2000 U 3000、世界朝向为缺省（Y is N and Z is U）的 301 × 400 × 500 BOX 且调用方未为它给出 World_AABB，THE Transaction SHALL 为它算出 World_AABB x ∈ [849.5, 1150.5]、y ∈ [1800, 2200]、z ∈ [2750, 3250] mm（即需求 4 第 20 条条目值的前 6 字）。

### Requirement 7: SPAMAP 随写入维护

**User Story:** 作为 e3d-io 的调用方，我想让 `Transaction` 在插入、改几何、换 owner、删除时自动增删改 SPAMAP 条目，以便 E3D 的空间搜索对 e3d-io 写的元素与对 E3D 自己写的元素给出同样的答案。

#### Acceptance Criteria

1. WHERE Spatial_Policy 为「要求」或「跳过」，WHEN Transaction 提交的批次插入一个 Geometric_Noun 元素，THE SPAMAP_Writer SHALL 使新会话 SPAMAP 表中键 refno 为该元素的条目恰为它的 Target_Entries（有 World_AABB 且从 WORL 可达时，即 SPAMAP_Codec 为它算出的全部条目，条数与 k 按需求 4）。
2. WHERE Spatial_Policy 为「要求」或「跳过」，WHEN Transaction 提交的批次使一个上一会话已有的 Geometric_Noun 元素至少一个重算属性（需求 5 第 2 条为该 noun 列出的属性全集）在本批结束状态的值与上一会话不同，THE SPAMAP_Writer SHALL 使新会话表中键 refno 为该元素的条目恰为它的 Target_Entries：旧条目不论条数与 k 一律不留，新旧条目键相同而值不同时存新值，上一会话表中该元素本无条目（如 0.4.0 写的 BOX）时照样登记。
3. WHERE Spatial_Policy 为「要求」或「跳过」，WHEN Transaction 提交的批次使一个上一会话已有的元素在本批结束状态的 owner 与上一会话不同（`move_to`），THE SPAMAP_Writer SHALL 使该元素（noun 属于 Geometric_Noun 时）以及需求 5 第 3 条坐实 E3D_Core 在 owner 改变时会重算的它的每个 Geometric_Noun 后代，在新会话表中的条目恰为各自的 Target_Entries。
4. WHERE Spatial_Policy 为「要求」或「跳过」，WHEN Transaction 提交的批次使一个上一会话已有的元素的 POS 或 ORI 在本批结束状态的值与上一会话不同，THE SPAMAP_Writer SHALL 使需求 5 第 8 条坐实 E3D_Core 在祖先 POS / ORI 改变时会重算的它的每个 Geometric_Noun 后代，在新会话表中的条目恰为各自的 Target_Entries（该元素自身的条目按第 2 条）。
5. WHERE Spatial_Policy 为「要求」或「跳过」，WHEN Transaction 提交的批次删除了元素，THE SPAMAP_Writer SHALL 使新会话表中不留键 refno 为下列元素的任何条目：本批删除的每个元素（不论 noun，含 Transaction 随之删除的后代）；本批结束状态下 owner 链经过被删元素的每个元素（如被删 ZONE 下的 BOX）。
6. WHEN Transaction 提交一个 SPAMAP 净编辑为空的批次（即按第 9 条得出的新表与上一会话的表逐条相同，包括：Spatial_Policy 为「关闭」；Recompute_Set 为空，如一批只含 `reorder`、引用表增删、既不在需求 5 第 2 条属性全集内也不是 POS / ORI 的属性改动；Recompute_Set 中每个元素的 Target_Entries 与其旧条目逐字相同），THE SPAMAP_Writer SHALL 使新会话表目录中的 SPAMAP 项与上一会话相同（有无、根页号与 ext 均同），且本会话追加的页中没有页头为页类型 5、名 hash 230902453 的页。
7. WHEN Transaction 提交一个 SPAMAP 净编辑非空的批次，THE SPAMAP_Writer SHALL 以写时复制方式写新会话的 SPAMAP 表：上一会话 SPAMAP 表的每一页逐字节不变；新会话 SPAMAP 表中不属于上一会话 SPAMAP 表的每一页，页号都大于上一会话最后一页的页号；`stage` 之后、写 page 0 之前（`StagedSession` 既未 commit 也未 rollback），SPAMAP_Reader 读文件最新会话所得仍是上一会话的 SPAMAP 表（page 0 的 `LSPGNO` 仍是唯一提交点）。
8. IF 上一会话的表目录没有 SPAMAP，而本批 Recompute_Set 的 Target_Entries 合计至少一条，THEN THE SPAMAP_Writer SHALL 按 E3D_Core 建表的页形（页类型 5、名 hash 230902453、键宽 4、叶值宽 7，其余页头字按 Evidence_Doc 所记）新建 SPAMAP 表，并在新会话表目录中新增一项名 hash 为 230902453、指向该表根页的表项（表目录已无空位时按第 12 条）。
9. FOR ALL 经 Transaction 成功提交的批次（insert / update / set / rename / move_to / reorder / 引用表增删 / delete 的任意组合，含同一元素多次写入、删除非几何祖先、上一会话缺条目的元素；Spatial_Policy 取三种之一），SPAMAP_Reader 按键序读回的新会话 SPAMAP 条目序列 SHALL 等于「上一会话表中键 refno 不属于 Recompute_Set 的条目」与「Recompute_Set 中各元素的 Target_Entries」之并按同一键序排列的结果（模型对照性质）。
10. FOR ALL SPAMAP 表与条目编辑集合（插入编辑带 4 键字与 7 值字，删除编辑带 4 键字；一个集合先应用其全部删除、再应用其全部插入；删除表中没有的键不改表，对应 534；插入表中已有的键不改该键的值，对应 529），SPAMAP_Writer 把同一集合应用两次所得的表 SHALL 与应用一次所得的表逐条相同（键字与值字；幂等性质）。
11. WHERE Spatial_Policy 为「跳过」，IF Recompute_Set 中一个在本批结束状态存在且从 WORL 可达的 Geometric_Noun 元素没有 World_AABB，THEN THE SPAMAP_Writer SHALL 删去键 refno 为该元素的全部旧条目，且不为它登记条目（D3）。
12. IF 本批 Recompute_Set 非空，且出现下列任一情形：SPAMAP_Writer 为本批读取的上一会话 SPAMAP 页（至少含根页）被 SPAMAP_Reader 判为不合规（需求 3）；SPAMAP_Codec 因坐标越界拒绝 Recompute_Set 中的一个元素（需求 4）；按第 8 条建表时会话页表目录已无空位，THEN THE Transaction SHALL 拒绝整批提交且文件逐字节不变，错误中指明所遇情形及涉及的页号、refno 或表目录的表数与容量（Spatial_Policy 为「跳过」时亦然）。
13. WHEN Transaction 提交的批次对同一元素有多次写入（多次 update / set、多次 move_to、update 或 move_to 之后 delete），THE SPAMAP_Writer SHALL 写出与「对该元素只作一次、直达其本批结束状态的写入」逐条相同的新会话 SPAMAP 表，表中不含由中间状态算出的条目。

### Requirement 8: 合并与重铸下的 SPAMAP

**User Story:** 作为以 base 会话打开事务的调用方，我想在库已前进时提交的合并里 SPAMAP 也按合并后的状态维护，以便别的写者此间的空间改动不被覆盖、我的改动也不丢。

#### Acceptance Criteria

1. WHEN Transaction 做 Merge_Commit，THE SPAMAP_Writer SHALL 以 Merge_Latest 的 SPAMAP 表（而非 base 会话的表）为起点维护新会话的表，即需求 6 第 7 条、需求 7 第 6–8 条所说的上一会话与库表目录都取 Merge_Latest 的（例：Recompute_Set 为空或 Spatial_Policy 为「关闭」时，新会话表目录沿用 Merge_Latest 的 SPAMAP 根，不追加 SPAMAP 页）。
2. WHERE Spatial_Policy 不为「关闭」，WHEN Transaction 做 Merge_Commit，THE SPAMAP_Writer SHALL 使新会话表中 Recompute_Set 每个 refno 的条目恰为 SPAMAP_Codec 对该元素在 Merged_State 中的状态算出的条目（即该元素的 Target_Entries；World_AABB 按需求 6 与本需求第 6 条取得）：Merge_Latest 改过的该元素属性、及其祖先的 owner / POS / ORI 均按改后的值计，不论 Merge_Latest 此间是否已为该元素写过条目。
3. WHERE Spatial_Policy 不为「关闭」，WHEN Transaction 在 Merge_Commit 中重铸一个插入元素的 refno，THE SPAMAP_Writer SHALL 以最终写入的 refno（`TransactionCommit::reminted` 所报的新号）的 word0、word1 作键第 2、3 字登记该元素的条目（调用方为原 refno 给出的 World_AABB 视为给该元素的），不以原 refno 登记任何条目；Merge_Latest 中原 refno 下的条目属于占用该号的元素，按第 4 条保留。
4. FOR ALL Merge_Commit，SPAMAP_Reader 读出的新会话 SPAMAP 表去掉键中 refno 属于 Recompute_Set 的条目后，SHALL 与 Merge_Latest 的 SPAMAP 表去掉同一 refno 集合的条目后逐条相同：按键序，4 个键字与 7 个值字全同，Merge_Latest 此间新增、改动、改挂的元素及重铸前原 refno 下的条目都在其中（模型对照性质）。
5. WHEN Transaction 做 dry run（stage 后 rollback），THE Transaction SHALL 得出与在同一文件状态上提交该批相同的 SPAMAP 结果：要么是相同的待登记与待删去条目（逐条键字与值字相同，Merge_Commit 与重铸下亦然），要么是相同的拒绝（错误种类与所列 refno 相同）；且 rollback 后文件逐字节不变。
6. IF 在 Merge_Commit 中调用方为某元素给出了 World_AABB，而 Merge_Latest 相对 base 改了该元素的 owner 或任一重算属性（需求 5 第 2 条），或改了它在 Merged_State 中任一祖先的 owner、POS 或 ORI，THEN THE Transaction SHALL 视该元素未给出 World_AABB，按需求 6 第 2、5、6 条处理（Built_In_AABB 元素由 Merged_State 自算，其余按 Spatial_Policy 拒写或跳过），拒写时错误中指明该 World_AABB 因 Merge_Latest 的改动而失效（D3）。
7. WHEN Transaction 因 `WriteError::Moved`（stage 与写 page 0 之间另一写者已提交）重新 stage，THE SPAMAP_Writer SHALL 以重新 stage 时的最新会话为 Merge_Latest 重算全部条目编辑（Recompute_Set、重铸后的 refno、第 6、8 条的判定一并重做），使提交结果对该会话满足第 2–4 条；首次 stage 不是 Merge_Commit 时亦然。
8. IF Recompute_Set 中的某元素在 Merged_State 中因 Merge_Latest 删了它的某个祖先而从 WORL 不可达（本批所改元素本身已被 Merge_Latest 删除时，照旧由冲突扫描报 562、整批拒写，不属本条），THEN THE SPAMAP_Writer SHALL 不为该元素登记条目、删去其在 Merge_Latest 表中的全部条目。
9. IF 出现第 8 条所述情形（承第 8 条），THEN THE Transaction SHALL 在提交报告中逐个列出这些元素的 refno 与被删祖先的 refno。

### Requirement 9: `e3d-write apply` 的 SPAMAP 支持

**User Story:** 作为用指令文件改模型的调用方，我想在指令里给出或省略包围盒、选择空间策略，并在报告里看到每个元素的 SPAMAP 条目，以便不写 Rust 也能写出 E3D 空间可见的几何元素。

#### Acceptance Criteria

1. WHEN 一条 `insert`、`set` 或 `move` 指令带 `aabb` 字段（世界系毫米 `[min x, min y, min z, max x, max y, max z]`），THE Write_CLI SHALL 把该 World_AABB 交给 Transaction 用于该指令的元素。
2. IF `aabb` 字段不是恰好 6 个 JSON 数、其中有值不是有限数、或某轴 min 大于 max，THEN THE Write_CLI SHALL 在写任何字节之前拒绝整个指令文件并以退出码 1 结束，消息以 `op #n` 指明指令序号，`--db` 文件逐字节不变。
3. WHERE 命令行给出 `--spatial require`、`--spatial skip` 或 `--spatial off`，THE Write_CLI SHALL 以对应的 Spatial_Policy（「要求」/「跳过」/「关闭」）运行本批；未给 `--spatial` 时按 `require` 运行（D3）。
4. WHEN Write_CLI 写报告（`--report`），THE Write_CLI SHALL 按 refno 汇总本批登记与删去的 SPAMAP_Entry（每条 4 键字与 7 值字，十进制有符号整数），并为每个 refno 注明来源指令序号（使该元素进入 Recompute_Set 的每条指令，含改其祖先 owner / POS / ORI 与删其祖先的指令）；「登记」指新会话表有、上一会话表没有的条目，「删去」指上一会话表有、新会话表没有的条目，均按 11 字整体比较。
5. WHEN 给出 `--dry-run`，THE Write_CLI SHALL 在报告中给出提交将登记与删去的 SPAMAP_Entry（口径同第 4 条，与在同一文件状态上不带 `--dry-run` 运行所报的逐条相同），且 `--db` 文件逐字节不变。
6. WHEN 一条 `set` 指令只带 `aabb`、不带属性，THE Write_CLI SHALL 只把该 World_AABB 交给 Transaction，不改该元素的任何属性（用于为 Recompute_Set 中的后代补 World_AABB）。
7. IF 一条 `rename`、`delete`、`reorder`、`reference` 或 `unreference` 指令带 `aabb`，或带 `aabb` 的指令所指元素的 noun 不属于 Geometric_Noun，THEN THE Write_CLI SHALL 在写任何字节之前拒绝整个指令文件并以退出码 1 结束，消息以 `op #n` 指明指令序号与原因，`--db` 文件逐字节不变。
8. WHEN 本批有多条指令为同一元素带 `aabb`，THE Write_CLI SHALL 以其中序号最大的一条的 `aabb` 为该元素的 World_AABB（需求 6 第 11 条）。
9. WHEN Write_CLI 写报告，THE Write_CLI SHALL 另列 Spatial_Policy「跳过」下无条目的 refno 及原因（需求 6 第 6 条）与「未使用的 World_AABB」的 refno（需求 6 第 13 条），各附来源指令序号。
10. WHEN 本批的插入元素在 Merge_Commit 中被重铸，THE Write_CLI SHALL 在报告中以重铸后的 refno（`reminted` 所报新号）汇总该元素的条目，使报告中的 refno 与条目键第 2、3 字一致，并保留该指令的 `given`。
11. IF Transaction 按需求 6 第 5 条或第 8 条拒写，THEN THE Write_CLI SHALL 以退出码 1 结束，消息逐个列出所涉 refno、原因与来源指令序号，`--db` 文件逐字节不变。
12. IF `--spatial` 的值不是 `require`、`skip`、`off` 之一，THEN THE Write_CLI SHALL 以非零退出码退出并列出合法取值，`--db` 文件逐字节不变。
13. WHERE 命令行给出 `--no-dictionary`，THE Write_CLI SHALL 保持该旗标在 0.4.0 中的行为，SPAMAP 维护照常按 `--spatial` 进行。

### Requirement 10: 真 E3D 3.1 验收门（D5）

**User Story:** 作为仓库维护者，我想让真 E3D 3.1 亲口回答 e3d-io 写的几何元素在空间上存在、位置对、随改动更新，以便 SPAMAP 写侧以 E3D 为 oracle 验收，而不只是读侧自证。

#### Acceptance Criteria

1. THE TTY_Gate SHALL 只用 PML1（`VAR !n EVALUATE … FOR ALL <noun> WITHIN VOLUME <elem> [clearance]` 与 `Q VAR`）提空间问题，不用 PML2 对象与 `COLLECT`，不对图元设 `DESC`（shadow TTY 下三者均 0xC0000005）。
2. WHEN TTY_Gate 以 e3d-io 写的 BOX 为体积、余量 100 做空间搜索，THE E3D_Session SHALL 在回答中列出该 BOX，且该问无 `(2,536)`（Q13a）。
3. WHEN TTY_Gate 以原生 BOX `24384/24777` 为体积、余量 100 000 做空间搜索，THE E3D_Session SHALL 在回答中列出提问时存在的每个 Q13_Box（Q13b）。
4. WHEN TTY_Gate 以门写的 ZONE 为体积做空间搜索，THE E3D_Session SHALL 在回答中列出该 ZONE 下 e3d-io 写的 BOX（Q13c）。
5. WHEN e3d-io 把一个 BOX 挪到世界位置不同的另一 owner 下（`move`），THE E3D_Session SHALL 在以新位置处 Probe_Element 为体积的空间搜索中列出该 BOX，并在挪动前列出过该 BOX、以旧位置处 Probe_Element 为体积的同一空间搜索中省略该 BOX（Q13d）。
6. WHEN e3d-io 改大一个 BOX 的 `XLEN`，THE E3D_Session SHALL 在以只与新范围相交的 Probe_Element 为体积的空间搜索中列出该 BOX，而改动前的同一空间搜索省略该 BOX（Q13e）。
7. WHEN e3d-io 删除一个 BOX，THE E3D_Session SHALL 在删除前列出过该 BOX 的同一空间搜索（同一体积与余量）中省略该 BOX（Q13f）。
8. WHEN E3D_Session 改大一个 e3d-io 写的 BOX 的 `XLEN` 并 `SAVEWORK`（2026-09-22 取证的改动），THE E3D_Session 在该会话中写出的该元素条目 SHALL 与 SPAMAP_Codec 对该 BOX 在该会话中的状态算出的条目逐字相同（条数、4 键字与 7 值字），由 SPAMAP_Reader 读该会话核对（Q13g）。
9. WHEN e3d-io 写一个 CYLI 和一个挂在带平移与旋转的 EQUI 下的 BOX，THE E3D_Session SHALL 在空间搜索中列出二者，且对二者 `Q POS WRT /*` 给出的世界原点与 e3d-io 为同一元素算出的世界原点各轴之差都不超过 0.1 mm（Q13h）。
10. THE E3D_Gate_Set SHALL 继续全数 PASS：TTY_Gate 的 Q0 与既有 28 问（至 Q12e）、`alternate_write_gate.ps1` 5 轮、`format0_gate.ps1`。
11. THE Phase_3_Workflow SHALL 在一份 Phase_3_Evidence_Doc 中记录一次人工 E3D 图形会话并附截图：`ADD` 一个 e3d-io 以 Spatial_Policy「要求」写入的 BOX 后该 BOX 出现在 3D 视图中，对该 BOX 做 `LIMITS CE` 不报 `(2,536)`；该会话只做一次，不算门问。
12. THE TTY_Gate SHALL 为 Q13 与 Q13a–Q13h 各输出一行 `PASS <问号>` 或 `FAIL <问号>`，判据只取该问的 `Q VAR` 输出与日志段：「列出」指该元素的 refno（`=w0/w1`）或名字出现在该问的 `Q VAR` 输出中，「省略」指二者都不出现，「无 `(2,536)`」指该问的日志段不含 `(2,536)`。
13. WHEN TTY_Gate 在 Q13a 之前以 Spatial_Policy「关闭」写入 Control_Box，并先以包含 Control_Box 的 Probe_Element 为体积、再以 Control_Box 为体积各做一次空间搜索，THE E3D_Session SHALL 在前一次回答中省略 Control_Box，且后一次报 `(2,536)`（Q13，判别力对照）。
14. IF Q13 为 FAIL，THEN THE TTY_Gate SHALL 对 Q13a–Q13h 一律输出 `FAIL <问号>`。
15. THE TTY_Gate SHALL 只以由 e3d-io 为其算出的 World_AABB 与所用余量事先判定了与被测 BOX 相交关系的 Probe_Element 作 Q13、Q13d、Q13e 的体积，判定结果写入 Spec_Ledger。
16. THE TTY_Gate SHALL 由 `e3d-write apply` 与 Gate_Ops 完成 Q13 组的全部 e3d-io 写入，会话 USER 为 `e3dio`，所问元素的 refno 取自该次运行的 `--report`，不写死 refno。
17. WHEN TTY_Gate 在 SPAMAP 维护记为完成的提交上运行结束，THE Phase_3_Workflow SHALL 把该次运行的总耗时与 Q13 组所占耗时记入 Spec_Ledger（不设阈值）。

### Requirement 11: `Transaction::open` 的耗时与发号下界（D4）

**User Story:** 作为在最大装机库上开事务的调用方，我想让 `Transaction::open` 在 1 秒内返回，且新发的号不低于 E3D 页 1 计数器，以便大库上开事务不卡，也不复用 E3D 删掉的号。

#### Acceptance Criteria

1. WHEN Transaction 以 release 构建在热文件缓存下打开 `ams7351_0001`（3 345 855 键、1 181 MB，CATA 库、无引用表），THE Transaction SHALL 使 `Transaction::open` 调用本身的墙钟时间（连续三次取中位数）不超过 1 s（Q4；现状 2 417 ms），所用机器与存储记入 Spec_Ledger。
2. THE Transaction SHALL 为每个扩展发出大于 base 会话索引中该扩展每一个 word1 的 word1。
3. WHERE Bucket_Table 列出该扩展，THE Transaction SHALL 为该扩展发出不低于其 Bucket_Floor 的 word1。
4. FOR ALL Corpus 库，Open_Fast_Path 算出的每扩展首个待发 word1 SHALL 等于「全索引扫描所得该扩展最大 word1 + 1」与「该扩展的 Bucket_Floor（Bucket_Table 列出该扩展时）」二者之大者（优化实现对朴素实现的模型对照性质）。
5. WHEN Transaction 打开 2026-09-22 所测状态的 `ams8000_0001`（受控副本），THE Transaction SHALL 在扩展 24384 发出的首个 word1 等于由该文件页 1 值 26496 按 Counter_Semantics 得出的 Bucket_Floor（「下一个待发」为 26496，「已发高水位」为 26497；活最大为 26483）；测试由该副本自身的页 1 与索引算期望值，不写死 26496。
6. IF Bucket_Table 无法解析，THEN THE Transaction SHALL 按全索引扫描结果发号，并在打开结果中标明计数器不可用。
7. THE Evidence_Doc SHALL 以 E3D_Core 的发号例程（`db_get_bucket`，`0x5aadd90`，routine 360）坐实 Counter_Semantics，并以 Corpus 核对（`ams8000_0001` 扩展 24384：活最大 26483 / 页 1 值 26496；32576：31 / 98；`ams251142_0001`：946 / 947；`TES023` 17419：315 / 331，9227 无条目）。
8. WHERE 构建为发布提交的默认构建，IF Counter_Semantics 未坐实，THEN THE Transaction SHALL 按 `c61b097` 的规则发号（每扩展取 base 会话索引中最大 word1 + 1，不以 Bucket_Floor 为下界），并在打开结果中标明计数器未用（需求 2 第 6 条、D7）。
9. THE Transaction SHALL 对 Bucket_Table 未列出的扩展（如各库主扩展）同样经 Open_Fast_Path 得出首个待发 word1，不遍历全部索引键。
10. IF Open_Fast_Path 依赖索引键的排列次序（如按 (word0, word1) 升序、沿 B+ 树取最右路径），THEN THE Evidence_Doc SHALL 坐实该次序（Core_IDB 出处，或在全部 Corpus 库上 0 反例的核对命令）。
11. WHEN Transaction 在 Merge_Commit 中重铸插入元素的 refno，THE Transaction SHALL 使重铸所得的 word1 同样满足第 2、3 条（以 Merge_Latest 的索引与提交时读到的 Bucket_Table 计）。
12. WHEN 调用方以 `open_at(base)` 打开，THE Transaction SHALL 以 base 会话的索引与打开时读到的 Bucket_Table 计第 2、3 条的下界。
13. WHEN e3d-io 在 Bucket_Floor 高于活最大 word1 + 1 的扩展（如 `ams8000_0001` 的 24384）插入一个带名字的元素并提交，THE E3D_Session SHALL 对该名字 `Q REF` 给出的 word1 等于第 5 条口径下由门副本页 1 与索引算出的期望值（TTY_Gate 门问 Q14）。

### Requirement 12: 单会话 staging 的规模（D4 / D6）

**User Story:** 作为一次写入上万到十万元素的调用方，我想让单会话 staging 的耗时随元素数近线性增长、SPAMAP 页数有界，以便十万级批量不必拆成多个会话。

#### Acceptance Criteria

1. THE Scale_Probe SHALL 由仓内受版本管理的一条命令运行，复现第二期 T2「单会话 A」的形状（n 个 insert 进同一 owner + 1 000 update + 100 delete，`ams8000_0001` 副本，release 构建），只在副本上运行，原件逐字节不变。
2. THE Phase_3_Workflow SHALL 在改 staging 代码之前，以剖析数据在 Spec_Ledger 中写明造成超线性的操作。
3. WHEN Scale_Probe 以 n = 100 000 运行，THE Transaction SHALL 使 Staging_Time 不超过 n = 10 000 时的 15 倍（现状约 73 倍：86.2 ms 对 6 297 ms，拟合约 O(n^1.8)）。
4. WHEN Scale_Probe 在 SPAMAP 维护开启下（Spatial_Policy「要求」）以 n = 10 000 运行、每个 BOX 有可用 World_AABB，THE Transaction SHALL 在 10 s 内完成 staging 与提交（Q4 原值）。
5. THE Scale_Probe SHALL 按记录 / 索引 / claim / TYPEX / SPAMAP / 其他 / 会话页分列报告追加页数（第二期 T2 的 data / index / claim / typex / other / ses 列另加 SPAMAP 列）。
6. WHEN Scale_Probe 以 n = 10 000 运行，THE Transaction SHALL 追加的非 SPAMAP 页数不超过记录页数的 1.5 倍（Q4 原值；第二期实测 1.21 倍）。
7. WHEN Scale_Probe 以 n = 10 000 运行，THE Transaction SHALL 追加的 SPAMAP 页数不超过 `2 × ⌈条目数 / L⌉ + SPAMAP 树高`，`L = ⌊(页字数 − 7) / 11⌋`，页字数取自该库文件头（`ams8000_0001` 的 512 字页得 L = 45）（D6）。
8. FOR ALL 既有写路径测试的批次与 Scale_Probe 的批次，staging 优化后写出的文件 SHALL 与 Golden_Hashes 逐字节相同（字节中性，按需求 2 第 7 条验收）。
9. THE Scale_Probe SHALL 在热文件缓存下对每个 n 运行至少 3 次，报告 open / build / stage / commit 各阶段墙钟时间的中位数与峰值 RSS（RSS 不设门槛），且至少测 n = 10 000 与 n = 100 000 两点（可加 n = 30 000 并报告拟合的幂指数）。
10. WHEN Scale_Probe 的一次运行提交完成，THE Scale_Probe SHALL 以读侧核对该 owner 的成员与按本批 insert 与 delete 算出的期望成员逐个相同（n = 100 000 时成员数远超 14 位计数 0x3FFF）。
11. THE Phase_3_Workflow SHALL 在 staging 优化的首个写侧提交之前，于其父提交上录下 Golden_Hashes（固定 `SessionRequest.clock` 与预铸 refno）并纳入版本管理。

### Requirement 13: S6b 页 1 桶表写侧（按取证条件纳入）

**User Story:** 作为与 E3D 交替写同一扩展的调用方，我想确知 e3d-io 不维护页 1 计数器会不会让 E3D 发出撞号，并在会撞时像 `core.dll` 一样维护它，以便 E3D 在 e3d-io 会话之后建的元素不与 e3d-io 的元素同号。

已有旁证：写门 Q12 中，e3d-io 的 8 200 盒会话之后 E3D `NEW` 的 EQUI / BOX 得到 `24384/34694` / `34695`，高于 e3d-io 的活最大，远高于 e3d-io 从未改写过的页 1 计数器；E3D 登录时另存的「Saving Meta Data」会话可能就是它重建桶表的时机。本需求要把这一旁证坐实到例程与字节。

#### Acceptance Criteria

1. THE Evidence_Doc SHALL 坐实 E3D_Core 何时读 Bucket_Counter 发号、何时重建 Bucket_Table，以及页 1 的落盘方式（原地重写，或写时复制并由 page 0 指向），所涉例程至少含 `DB_DB::updateBucketList`（`0x59144e0`，存盘时以 `DB_IndexTableIterator` 遍历全部元素、逐扩展取最大 word1 重建）、`DB_DB::getRefBuckets`（`0x5904cd0`）、`db_get_bucket`（`0x5aadd90`，routine 360）与 `db_add_bucket`（`0x5aaaac0`）。
2. THE Evidence_Doc SHALL 以一次 E3D_Session 实测坐实：当文件最新会话由 e3d-io 写、且 Bucket_Counter 低于 e3d-io 已发的最大 word1 时，E3D `NEW` 一个元素所得的 refno 是否与 e3d-io 已发的某个 refno 相同。
3. IF Evidence_Doc 坐实 E3D 会发出 e3d-io 已用的 refno，THEN THE Transaction SHALL 在提交时把 Bucket_Table 中每个扩展的 Bucket_Counter 抬到使其 Bucket_Floor 大于新会话索引中该扩展的每一个 word1 与本批为该扩展发出过的每一个 word1，落盘方式与 E3D_Core 相同。
4. IF Evidence_Doc 坐实 E3D_Core 原地重写页 1，THEN THE Phase_3_Workflow SHALL 在实现第 3 条之前，把「page 0 不再是唯一提交点」对崩溃一致性的影响写进 Spec_Ledger，并取得用户确认。
5. IF Evidence_Doc 坐实 E3D 不会发出 e3d-io 已用的 refno，THEN THE Spec_Ledger SHALL 把 S6b 写侧记为留待，附证据与触发条件。
6. WHERE Transaction 写 Bucket_Table，THE Transaction SHALL 写出能被 `BucketTable::parse` 解析、第 2 字等于 `2 + 3n`（n 为条目数）且每条第 3 字为 0 的页 1（四个 Corpus 库均如此）。
7. WHERE Transaction 写 Bucket_Table，THE Transaction SHALL 使 `alternate_write_gate.ps1` 5 轮全数通过。
8. WHERE Transaction 写 Bucket_Table，THE E3D_Gate_Set SHALL 含一个在 `c61b097`（不写页 1）上输出 FAIL、在该能力记为完成的提交上输出 PASS 的门问：e3d-io 提交之后，E3D_Session `NEW` 一个该扩展的元素，所得 word1 等于 e3d-io 所写页 1 的 Bucket_Floor，且与 e3d-io 已发的每个 refno 都不同。
9. WHERE Transaction 写 Bucket_Table，THE Transaction SHALL 保留上一会话 Bucket_Table 中的每个扩展条目，且使每个扩展写出的 Bucket_Counter 不低于上一会话的值。
10. WHERE Transaction 写 Bucket_Table，WHEN Transaction 做 Merge_Commit，THE Transaction SHALL 以 Merge_Latest 所对应的页 1 为起点按第 3 条抬计数器，重铸后的 refno 计入、原 refno 不计入。
11. THE Evidence_Doc SHALL 在第 2 条的实测中比对 `NEW` 前后页 1 的字节与会话链，确认 E3D_Session 在 `NEW` 之前没有先重建 Bucket_Table；已先重建时，该次实测不作第 2 条的结论。
12. THE Evidence_Doc SHALL 在第 2 条的实测中分别测「e3d-io 已发且仍存在」与「e3d-io 已发后又删去」的 word1，写明「已发」是否含后者。

### Requirement 14: 范围与文档

**User Story:** 作为仓库维护者，我想让 MISSION 与发布说明如实写出第三期把 SPAMAP 纳入写侧、包围盒从哪里来，以便后来的会话不按过时的「写侧不维护 SPAMAP」判断范围。

#### Acceptance Criteria

1. WHEN SPAMAP 写侧落地，THE MISSION SHALL 把 Out of scope 中「写侧因此也不维护 `SPAMAP`——它的值要几何算」半句替换为 D1 所定的包围盒来源与 Built_In_AABB noun 清单，使全文不再含「不维护 `SPAMAP`」。
2. THE MISSION SHALL 继续把 Built_In_AABB 之外的几何求解（目录几何、模型生成、下游渲染）列为不做，并保留「E3D 2.10 及其他版本的常量差异；写侧只写 3.1 单 extent 文件」一条。
3. WHEN SPAMAP 写侧落地，THE Release_Docs SHALL 使 `README.md` Status 节的「Not yet」与 Write path 节的「Not written」清单都不再含 `SPAMAP`，并在 Write path 节写明 Spatial_Policy 的三种取值与默认值，以及 D1 所定的 Built_In_AABB noun 清单。
4. WHEN 一个公开类型新增字段或变体、或默认行为改变（D3 的默认拒写、需求 11 的发号下界），THE Release_Docs SHALL 在 `CHANGELOG.md` 的 `Unreleased` 节写明该变化与调用方的迁移方法。
5. WHEN Write_CLI 新增 `--spatial` 与 `aabb`，THE Release_Docs SHALL 在 `README.md` 的 `e3d-write apply` 旗标清单中写明 `--spatial require|skip|off`（默认 `require`）、`aabb` 字段的形状（世界系毫米 6 数）及可带它的指令种类。
6. WHEN E3D_Gate_Set 的门问增加，THE Release_Docs SHALL 把 `README.md` 的门描述（现为「Q0 and twenty-eight questions」）改为该提交上 TTY_Gate 的实际门问数，并列出新增问号。
7. WHEN SPAMAP 写侧落地，THE MISSION SHALL 把「写回」节中已过时的「TTY 门 14 问 PASS」改为该提交上 TTY_Gate 的实际门问数。
8. THE Release_Docs SHALL 在 `README.md` 中以仓内相对路径引用本期每份 Phase_3_Evidence_Doc。
9. WHERE Transaction 写 Bucket_Table（需求 13 第 3 条的条件成立），THE Release_Docs SHALL 在 `README.md` Write path 节的已写结构中列出页 1 桶表及其落盘方式。

### Requirement 15: 回归与兼容

**User Story:** 作为下游 crate 的维护者，我想让第三期不破坏既有读写行为与公开接口，以便升级只需处理 CHANGELOG 列出的变化。

#### Acceptance Criteria

1. THE Build_Checks SHALL 在 `cargo test --no-fail-fast` 下 0 失败，且 `c61b097` 上通过的每一条测试（基线：100 个测试二进制，622 passed / 0 failed / 30 ignored）继续通过；按第 7 条改过的测试以改后的版本计，报 skip 的按第 6 条计。
2. THE Build_Checks SHALL 在 `cargo clippy --all-targets` 下 0 error，且本期改动的文件 0 warning（基线 lib 既有约 22 条 warning，均不在 `src/write/`）。
3. WHERE Spatial_Policy 为「关闭」，THE Transaction SHALL 对 `tests/write_stage6_*.rs` 的每个批次写出与 0.4.0 逐字节相同的文件（兼容性质）；比对时固定 `SessionRequest.clock`，并以预铸 refno 走原语路径、或只比不含新发 refno 的批次，使比对不受需求 11 的发号下界影响（发号下界另按需求 11 核对）。
4. THE Downstream_Crates 中的 `e3d-model` 及其 7 个 verify 变体 SHALL 不改源码即对新版本 `cargo check` 通过；因它们以 path 依赖 `../e3d-io` 指向 `main` 工作树 `D:\work\plant-code\old\vendor\e3d-io`（不是本 worktree `e3d-io-write`），该检查在本期合并到 `main` 之后、或以指向本 worktree 的依赖覆盖进行。
5. IF Downstream_Crates 出现与 e3d-io 无关的既有编译错误（如 gen-model 的 `change_is_metadata_only`，`window_root_plan.rs:628`），THEN THE Phase_3_Workflow SHALL 只在 Spec_Ledger 中记录，不在本仓修改下游。
6. IF `c61b097` 上通过的一条测试在本期运行中因样本缺失经 `common::missing` 报 skip，THEN THE Phase_3_Workflow SHALL 不把它计为继续通过，并在 Spec_Ledger 中列出该测试与所缺样本，直到它在有样本的机器上通过。
7. IF `c61b097` 的一条测试因需求 11 的发号下界（新 refno 改变）或 D3 的默认「要求」（插入非 Built_In_AABB 的 Geometric_Noun 元素）而失败，THEN THE Phase_3_Workflow SHALL 只改该测试的期望 refno 或为它显式指定 Spatial_Policy，并在 Spec_Ledger 中逐条列出所改测试与原因（CHANGELOG 按需求 14 第 4 条）。
8. THE Build_Checks SHALL 在 `cargo doc` 下 0 error，warning 数不超过 `c61b097` 上的条数（约 24 条），且本期改动的文件 0 warning。
9. IF 本期给一个公开 struct 新增字段，而该 struct 既非 `#[non_exhaustive]`、也没有 builder 或 `Default` 供下游构造，THEN THE Release_Docs SHALL 在 CHANGELOG 中写明下游结构体字面量的迁移方法。

### Requirement 16: 本期不做与触发条件

**User Story:** 作为规划者，我想把本期不做的项与其纳入条件写清，以便触发条件出现时直接升级为下一件事，而不必重新调研。

#### Acceptance Criteria

1. THE Spec_Ledger SHALL 把第二期 C 表的下列项列为本期不做并各附触发条件：UDA 表 `UDATAB` / `UDAFTB` / `UUIDAR`（触发：Corpus 新增一个会话表目录列出其中任一张的项目库）；类型 17 逐槽三方合并与冲突 576（触发：一次 Merge_Commit 中本批与 Merge_Latest 改了同一元素的同一个类型 17 属性）；extract 模型 / Central / 多 extent 文件 / compact 记录 / E3D 2.1 / 部分存盘（触发：调用方提出其中一项的明确需求，或 Corpus 中出现需要它们的文件；现语料 940 694 条叶值 flag 全为 1 即全单 extent，`ams8000_0001` 的 6 639 个元素全 wide）；`AttrValue` 加 `TextList`（第二期定为不做；触发：调用方要求写文本列表属性）；类型 18 `UDASTB`（触发：Corpus 中出现设了值的 `UDASTB`）。
2. THE Spec_Ledger SHALL 把 `docs/evidence/2026-09-22-core-dll-record-draft-data-flow.md` 的差异 ②（explicit 零值不存）、③（项变大删旧追加流末）、④（hunk 位置）、⑥（段最少 2 载荷字）、⑦（implicit 无置未设）、⑧（compact 转 wide）列为只记不改，触发条件为「有调用方要求该结构与 E3D 逐字节相同，或一道门因该差异失败」；差异 ⑦ 注明本 crate 对 `Unset` 写哨兵，是 E3D_Core 行为的超集。
3. IF 本期某道门或某条测试因上列任一差异失败，THEN THE Phase_3_Workflow SHALL 先取得用户对范围变化的确认，再为该差异补一条需求纳入本期并实现。
4. THE Spec_Ledger SHALL 把 Built_In_AABB 之外的包围盒求解列为不做，触发条件为 D1 改选备选 B。
5. THE Spec_Ledger SHALL 把「外部输入改动后重算引用它的元素的条目」列为不做：需求 5 第 2 条重算表中「外部输入」一类的改动（如 SPRE、CATR 所指元素的属性）不使引用它的元素进入 Recompute_Set（重算属性只取「自身」一类）；触发条件为 Phase_3_Evidence_Doc 坐实 E3D_Core 在该改动所在会话重算这类条目，或一道门因此失败。
6. THE Spec_Ledger SHALL 把「`Transaction::delete` 删除整棵子树」列为不做（现只删元素本身），触发条件为调用方要求随元素删除其后代。
7. THE Spec_Ledger SHALL 把 D8 的备选（owner 或祖先位姿改变时照样重算后代条目）列为不做，触发条件为用户改选 D8 备选。
8. THE Spec_Ledger SHALL 沿用第二期 §6 的不做项并各附触发条件：HTTP 层、E3D 2.1 写、改动下游源码（触发：用户提出明确需求），以及 T4 Q17 活 E3D 持锁实测门（已由 `tests/write_stage7_lock.rs` 与 IDA 取证覆盖；触发：一次 E3D_Session 的持锁行为与该测试或取证结论不符）。
9. THE Spec_Ledger SHALL 把每个不做项的触发条件写成可观察的事件：由谁（调用方、用户、某道门或某条测试、Corpus_Census）在何处（某 Corpus 库、门日志、Spec_Ledger）看到什么（如 Corpus_Census 输出中出现某表的名 hash）。
