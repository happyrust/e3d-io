# 计划：E3D Studio M1 后半——S4 浏览收尾、S5 属性与会话（S6 并行约束、S7 前提）

状态：**评审通过**（2026-10-10 06:41，Plannotator 门 `approved`，无批注 → Q1–Q10 全按推荐项）
日期：2026-10-10
范围：`vendor/e3d-studio`（`crates/e3d-studio` 为主；`crates/e3d-convert` 只加不改）。`vendor/e3d-io-write` 只新增本文件、更新 M1 计划的执行记录，不改源码。
依据：`docs/plans/2026-10-09-e3d-studio-m1-plan.md`（下称「M1 计划」，Plannotator 2026-10-09 `approved`）；`e3d-studio` 的 spec `.kiro/specs/e3d-studio-m1/{requirements,design,tasks}.md` @ `9579f9c`；`docs/plans/2026-09-30-e3d-studio-gui-design.md`（下称「设计」）§3、§4.3、§4.6；ADR 0002；本次审核（§1.3）。

## 0. 结论

1. **S4 做了一半，是看不见的那一半。** 引擎到界面的管道通了：库线程协议与 worker（3.1、3.2）、多库列表 `OpenLibraries`、头信息面板与横幅（3.3）、懒加载树的纯数据模型（3.6 前半，含 Property 2 的 proptest）。用户看得见的浏览——树、搜索、面包屑、源版本、项目库列表——一个都还没有。spec 52 个子任务已勾 15 个（1.1–1.5、2.1–2.7、3.1–3.3），约 29%。
2. **上一会话 00:20 起做的「3.6 界面一半」没有落盘。** `e3d-studio` main 在 `9579f9c`，工作树干净、无 stash、reflog 无 reset；所有被跟踪文件的修改时间都是 00:32:57，即整棵树在那一刻被 checkout 回了 HEAD。本计划把 3.6 的界面一半从头做。
3. **顺序**：C 3.6 界面 → D 3.7 搜索 + 3.8 面包屑与前进后退 → E 3.4 源版本 + 3.5 项目库列表 → F 3.9 / 3.10 性能 / 3.11 检查点 → S5（4.1 单版本解码接口先行）。S6 可另开会话并行，接口约束见 §2.6；S7 按 M1 计划，前提见 §2.7。
4. **两处修正上一会话 23:36 方案的说法**：(a) 3.5 不需要给 e3d-convert 加「逐库回调」接口——`e3d_convert::inventory::describe(path, root)` 已是公开函数，GUI 后台线程逐库调它即可；(b) 树的渲染不用 gpui-base 的 `Tree`（它要整棵 `TreeItem` 先建好，与 D11 的懒加载不合），直接用 gpui 的 `uniform_list` 画 `TreeModel::lines()`，gpui-base 的 `Tree` 内部也是这么画的。
5. 本计划不改 D1–D14，不改设计 §5 的验收；协议上动两处（`Found` 带成员序号与行、`AttributesReady` 载荷改枚举，§3 Q2、Q6），都在 D10「界面与库线程只经通道收发带 Gen 的请求与应答」之内，spec 3.1 与 design 协议表随实现同步。

## 1. 审核

### 1.1 进度

| 阶段 | 状态 | 提交 |
|---|---|---|
| S0 建仓与基线 | 完成 | `d70f6f7`、`c5705b4` |
| S1 补完 T1 | 除 S1.6 外完成 | `fa729de`、`a9458cd` |
| S2 spec | 完成 | `313a5c8`、`72708fb`（3.1 改写） |
| S3 应用骨架 | 代码与自动验证完成；2.8 等用户手验 4 项 | `7f218af`、`9232ea1`、`1d86e62`、`35db70f` |
| S4 3.1–3.3、多库列表 | 完成 | `438cc04`、`b504e70`、`b4eec47`、`f91f3e8` |
| S4 3.6 模型 | 完成（3.6 未勾） | `9579f9c` |
| S4 3.6 界面、3.4、3.5、3.7–3.11 | 未开始 | — |
| S5、S6、S7 | 未开始 | — |

最后一次检查点（`9579f9c`）：`cargo test -p e3d-studio --offline` 24 / 0（本次复跑一致）；`cargo test -p e3d-convert` 11 / 0 + 3 / 0（3 ignored）；clippy e3d-convert 3 条（S0 基线）、e3d-studio 0 条；`cargo doc --no-deps` 0 条。

### 1.2 发现

- **F1 树的状态不能住在面板里。** dock 按名字从布局文件重建面板，构造时只有 `cx`（`b4eec47` 为此加了全局 `OpenLibraries`）。树的展开 / 选中 / 定位历史若放在 `TreePanel`，面板一关就丢；而面包屑（3.8）、属性表（4.2）、前进后退都要同一份「当前选中」。要一个按库挂着的浏览状态实体（§2.1、Q1）。
- **F2 `TreeModel` 没有 owner 指针。** `Node` 只记 `row / expanded / members`，`Line.parent` 只对可见行有。面包屑要选中元素的 owner 链，要么每次靠 `Found.path`，要么给 `Node` 加 `parent: Option<RefNo>`（`fill` 时记下）。后者是十几行的改动，顺带让「← 回 owner」不依赖可见行。
- **F3 搜索命中落不到懒加载树上。** `Found { path: Vec<RefNo> }` 只有 refno；树里未读的成员是按**序号**占槽的（`members: Vec<Option<RefNo>>`），不知道命中元素在 owner 成员表里排第几就放不进去。worker 走 owner 链时每一级都 `find_element` 过 owner，顺带解析成员表就能给出序号与该行的 `ElementRow`，代价是每级多解析一条记录（Q2）。
- **F4 `Children` 请求要有在途约束。** `GenGate` 每类只认最新一代：可见区变化就发新请求，旧应答会被丢，但模型里的未读槽还在，`wanted` 下次还会要——不会丢数据，但滚动时会连发。要一个纯数据的 `Loader`：同一 `(owner, window)` 在途时不重发，应答到达或窗口变化才发下一个；可单测（§2.1）。
- **F5 单版本解码是 S5 的门槛。** `worker.attributes` 在 `compare: None` 时直接回「单版本解码在 S5（任务 4.1）接入」；没有 Compare_Release 属性表就出不了东西。e3d-convert 的 `Reading { status, value }` 没有需求 4.1 要的「存储」列；e3d-io 的 `ExtractedAttribute` 带 `storage: StorageDescriptor` 与 `layout`，新接口要把它带出来（Q6）。
- **F6 源版本对话框的提示源。** 需求 2.2 列「DAB 标签、`DBBLBX`、路径」。e3d-io 没有读 `DBBLBX` 的接口；它只在 3.1 的 `DB_DB::setMetaData()` 写 `DBMDAT/DBMVAL/DBMEXT` 时留下，同时把 `Saving Meta Data.` 写进会话注释（`schema.rs` 的 `E3D31_BINARY_SESSION_MARKER`）。`Library::sessions()` 已有注释，可以直接判「有没有 3.1 的元数据写入痕迹」，不用动 e3d-io。`project_version_evidence`（`COMM.VERS`）要扫整个项目才有结果，不适合弹框时等（Q5）。项目根从库路径怎么推，spec 没写（Q4）。
- **F7 3.5 的字段。** `DatabaseEntry` 有 `db_type / bytes / extents / session（最新会话号）/ dab_tag / template_version / header_format / creation_info / error`，没有会话**数**；需求 2.5 写「会话数」。读会话数要每库多走会话链，450 个库的列表就过不了 2 s（Q9）。「能否核验」用 `e3d_convert::release` 的 `verifiable(db_type)`（D6）。
- **F8 环境。** 共享 `D:\Rust\target` 的构建锁本次 `cargo test` 等过一阵——还有别的 cargo 在用，多会话并行编译会互相等。`des.exe`（写路径 10.5）仍开着，S1.6 与 6.7 继续等。依赖仍指 `../e3d-io-write` 的 `write-path`（`ba80a4c`），P3 之后改指。2.8 的 4 项用户手验未做。e3d-io-write 工作树里仍是 10.5 的两个未跟踪脚本，不碰。

### 1.3 本次审核做了什么

- 读：M1 计划；spec 三份全文；设计 §3；`e3d-studio` 的 `db/{protocol,library,open,worker}.rs`、`panels/{mod,library,header,tree}.rs`、`panels/tree/model.rs`、`workspace.rs`、`settings.rs` 头部；`e3d-convert` 的 `inventory.rs`、`verify.rs` 的公开项与 `ElementComparer::decode`；gpui-kit 的 `component/{tree,table,breadcrumb,virtual_list}.rs`、`base/tree.rs` 的 `uniform_list` 用法；gpui-pre 0.3.7 `uniform_list.rs` 的 `UniformListScrollHandle` 接口；e3d-io 的 `schema.rs`（`KNOWN_VERSIONS`、`database_version_evidence`、`project_database_paths`）、`record/descriptor.rs` 的 `ExtractedAttribute`。
- `git`：`e3d-studio` 的 `status` / `log` / `stash list` / `reflog`；`e3d-io-write` 的 `status` / `log`；文件修改时间。
- 跑：`cargo test -p e3d-studio --offline` 24 / 0。
- 没有做：没改任何文件；没起应用；没跑 clippy（上次 `9579f9c` 的数字见 §1.1）。

## 2. 计划

每一步一到两个提交；每个提交前 `cargo test --offline`、`cargo clippy --all-targets`、`cargo doc --no-deps` 与基线比，不新增失败与警告；只 `git add` 碰过的路径；spec 的 tasks 随步打勾、Spec_Ledger 记数字；M1 计划 §5 在每步结束补一条。界面证据在 D、E、S5 末各起一次应用（`E3D_STUDIO_DATA_DIR` 指临时目录），cua-driver 截图，续写 `docs/evidence/2026-10-09-e3d-studio-s4.md`（按日期分节），S5 另起 `2026-10-xx-e3d-studio-s5.md`。

### 2.1 C：3.6 界面一半——树面板（Q1、Q10）

**落点**

- 新 `db/browse.rs`：`Browse` 实体，每个打开的库一个，`OpenLibraries` 的 `Opened` 多持一个 `Entity<Browse>`，与 `Library` 同生同灭。字段：`tree: TreeModel`、`loader: Loader`、`trail: Trail`（§2.2 的定位历史）。订阅自己那个 `Library` 的事件：`Opened` → 用 `Library::root()` 重建树（没有根元素的库类型显示「此库没有元素树」）；`Children { refno, window, total, rows }` → `tree.fill`，`loader` 清在途；`ElementFailed`（子行类）→ 清在途、记日志、状态栏显示原文；`Found` → §2.2。对外事件 `BrowseEvent::{Lines, Selected(RefNo)}`。
- `TreeModel` 加 `parent: Option<RefNo>` 与 `ancestors(refno) -> Vec<RefNo>`（F2）；`Line` 不变。
- `Loader`（纯数据，`browse.rs` 内）：`want(wanted: Option<(RefNo, Range)>) -> Option<(RefNo, Range)>`——与在途相同就 `None`，不同就换新的并记为在途；`answered(refno, window)` 清在途。窗口上限 200 行；预取量 = 一屏行数。
- `panels/tree.rs`：`TreePanel` 从空壳换成真面板，`NAME` 仍是 `"TreePanel"`（布局文件兼容）。渲染 `uniform_list("tree", lines.len(), cx.processor(...))` + `.track_scroll(&UniformListScrollHandle)`：processor 拿到 `visible_range` 后只**记录**可见区，请求在 `cx.defer` 里由 `Browse` 经 `Library::request_children` 发，不在渲染里改状态。每行：缩进 `depth × 12 px`、展开箭头（有成员才画；点箭头切换，点行选中，双击切换）、noun、名字或位置名、右侧淡色 refno；未读槽画一行淡色「…」。选中行用 `theme.list_active`。
- 键盘：面板 `key_context("ElementTree")`——不用 `"Tree"`，gpui-base 的 `tree::init` 已把 ↑ ↓ ← → 绑在那个上下文上、动作类型又是 `pub(crate)`，接不到手。`app.rs` 的 `actions!` 加 `TreeUp / TreeDown / TreeCollapse / TreeExpand / TreeToggle`，`bind_keys` 以 `Some("ElementTree")` 绑 up / down / left / right / enter，面板 `on_action` → `TreeModel::key`。键盘或搜索改变选中时 `scroll_to_item(ix, ScrollStrategy::Center)`；鼠标点选不滚。
- 面板只认当前库：订阅 `OpenLibraries`，`Changed` 或 `Library { current: true, .. }` 就重画；切库时各库的 `Browse` 各自保留展开与选中。

**测试**（计入 3.9）：`Loader` 的 proptest——任意可见区与应答序列下在途至多一个、每个曾可见的未读槽最终被请求过；`TreeModel::ancestors` 单测；现有 24 条不变。

**验收**：起应用开 `acp7048_0001`：展开 WORLD → 成员出现，滚动时未读槽逐段填上；← → ↑ ↓ Enter 都对；一次只有一个 `Children` 在途（日志里数）。截图 1 张。3.6 的「界面一半」打勾，3.6 整条打勾。

### 2.2 D：3.7 搜索、3.8 面包屑与前进后退（Q2、Q3）

**搜索（3.7）**

- 工具栏的搜索框已有（`InputState`），接回车：`SearchQuery::parse` → 格式错误直接进状态栏（`SearchError::Format` / `EmptyName` 的两句话）；否则 `Library::find`（当前库）。
- 协议：`Response::Found { gen, path: Vec<FoundStep> }`，`FoundStep { row: ElementRow, index: usize }`，根在前、命中在后，`index` 是该元素在其 owner 成员表中的序号（根为 0）。worker 的 `owner_chain` 已对每级 owner `find_element`，顺带 `parse_as` 取成员表定位序号并产出 `ElementRow`；`MAX_OWNER_DEPTH` 不变。`LibraryEvent::Found` 同步。spec 3.1 与 design「Library_Worker」协议表的 `Found` 行随实现改写。
- `Browse::reveal(path)`：自上而下，对每一级 owner `tree.expand`，再 `tree.fill(owner, index..index + 1, owner.member_count, [row])` 把下一级那一行填进槽，最后 `select(命中)`、发 `Selected`、面板滚到它；周围的未读槽由 §2.1 的正常机制补读。
- `NotFound { reason }` → 状态栏原文（worker 已给「名字表里没有 /X」「索引里没有 =a/b」）。
- 测试：worker 的 `acp7048_opens_lists_finds_and_compares` 加断言——`=15240/1968` 的链每级 `index` 与直接读成员表一致；`Browse::reveal` 在纯模型上的单测（填进去的行等于路径、选中是命中、其余槽仍未读）。

**面包屑与历史（3.8）**

- 面包屑用 gpui-kit 的 `Breadcrumb` / `BreadcrumbItem`，内容是 `tree.ancestors(selected)` + 选中本身，每项「noun 名字」（无名显示位置名），点一项 → `Browse::select` 并滚到它。位置：工具栏下、dock 上的一条横条，属于 `Workspace`（Q3）。无选中时不画。
- `Trail`（纯数据，`browse.rs`）：带游标的 `Vec<RefNo>`，`visit(refno)` 截断游标后的尾巴再压入，`back() / forward()` 移游标并返回目标；连续同一元素不重复压；上限 200。`Workspace::on_back / on_forward` 从 `not_yet` 换成调当前库 `Browse` 的 `back / forward` → `select`（不再压栈）→ `Selected` 事件；属性表（S5）订阅它，这就是「树和属性表一起跳」。
- 测试：`Trail` 单测（分叉后前进被截断、空栈、上限）。

**验收**：`/` 名字与 `=15240/1968` 都能定位到并展开；错格式与未命中进状态栏；面包屑可点；Alt+← / Alt+→ 来回。截图 2 张（命中展开、面包屑）。3.7、3.8 打勾。

### 2.3 E：3.4 Source_Release、3.5 项目库列表（Q4、Q5、Q9）

**3.4 源版本**

- 新 `db/source_release.rs`：`ProjectReleases` 全局，`BTreeMap<PathBuf, String>`（项目根 → `E3dVersion.id`），存 App_Data_Dir 的 `project-releases.json`（`settings.rs` 头注释早已预留这个文件）；读坏回空表并记日志。`project_root_of(path)`：按 Q4 推。
- 触发：库 `Opened` 或项目文件夹打开时，项目根没有记录就弹对话框（`window.open_dialog`），一次一个，排队。对话框列 `e3d_io::schema::KNOWN_VERSIONS`（E3D 2.1、E3D 3.1）单选、**不预选**；下面是提示，不是判定：最新会话 DAB 标签（`2_28_*` 是 2.1 引擎、`2_35_*` 是 3.1 引擎写的）、`CRINFO`、路径、会话注释里有没有 `Saving Meta Data.`（3.1 写 DBMDAT/DBMVAL/DBMEXT 的痕迹，对应需求 2.2 的 `DBBLBX`）。取消 = 不选，属性区显示「请选择源版本」。
- 工具栏加「源版本」`Select`，显示当前库项目根的选择，改选即改映射并重发属性请求（S5）；标题栏改成设计 §3.1 的 `e3d-studio — <库名>（<源版本>）`。
- `Library` 多一个 `source: Option<&'static E3dVersion>` 的读法（从 `ProjectReleases` 查），worker 不变。
- 测试：`project_root_of` 单测；`ProjectReleases` 读写往返与坏文件回默认（Property 7 的口径）。

**3.5 项目库列表**

- 新 `db/inventory.rs`：`ProjectInventory` 实体，每个打开的项目根一个（`OpenLibraries` 旁边再放一个 `OpenProjects`，同样全局）。后台 `std::thread`：`e3d_io::project_database_paths(root)` 得路径表（先把总数和路径发回，列表立刻有行），再逐库 `e3d_convert::inventory::describe(path, root)` 发回填充；smol channel，前台 `cx.spawn` 收，每 30 ms 合并一次 `notify`。丢弃实体即置取消标志，线程读完当前这个库就退出。
- 「库」页签分两段：上段仍是已打开的库，下段是项目库列表，用 gpui-kit 的 `DataTable`（`TableDelegate` 自实现）：列 = 路径（相对项目根）、类型、大小、会话（最新会话号，Q9）、DAB 标签、extent 数、核验（`verifiable` → ✓，否则「不在核验范围」）、错误（读不出头的库原文）。按类型 / 大小 / DAB 标签排序（`ColumnSort`），顶部过滤框按子串过滤路径与类型。双击或回车一行 → `OpenLibraries::open`。
- `Workspace::path_selected` 的 `Project` 分支从只记状态栏改成打开 `ProjectInventory`，并触发 3.4 的源版本弹框。
- 测试：排序与过滤是纯函数，单测；`describe` 的调用不在测试里重复（e3d-convert 已测）。

**验收**：打开 `D:\AVEVA\Projects\E3D2.1\AvevaCatalogue`（或 Marine）：列表逐行出现，排序、过滤、双击打开；第一次打开弹源版本框、不预选、选后记住，重启不再弹。截图 2 张（对话框、库列表）。3.4、3.5 打勾。

### 2.4 F：3.9 单测、3.10 性能实测、3.11 检查点（Q7）

- **3.9**：Property 1（`GenGate` 单测，已有）、Property 2（proptest，已有）、Property 3（解析单测，已有）加 §2.1 的 `Loader` 性质，打勾。
- **3.10** 按需求 7.6：release 构建（Q7）、热文件缓存、每项 3 次取中位数。量法——在代码里加 `log::debug!` 级的计时点（不进 release 日志的默认级别），用 `E3D_STUDIO_TIMING=1` 打开：
  1. 打开 `ams7351_0001` 到出头信息：`OpenLibraries::open` → `Opened` 收到 → 下一帧画完。判据 < 1 s（目标 < 100 ms）。
  2. 展开 8 200 个成员的 ZONE：按键 → 第一个 `ChildrenReady` 填进树 → 画完。判据 < 150 ms。先用一个 `#[ignore]` 的 worker 测试在 `ams7351_0001` 里找出成员最多的 ZONE 并记下 refno（设计 §4.6 的数来自那里）。
  3. 展开时 UI 单帧：用 gpui-kit 的 `fps` crate 做可开关的角标（`E3D_STUDIO_FPS=1`），或帧计时日志。判据 < 16 ms。
  4. 约 450 个库的项目列表：线程起 → 最后一行到。判据 < 2 s。
  数字连机器信息记入 Spec_Ledger「性能实测」；不符就按真实值记并回报，不改判据（需求 10.3）。
- **3.11**：test / clippy / doc 与基线比；界面截图齐（树、搜索、面包屑、源版本框、库列表）；tasks 3.x 全勾；M1 计划 §5 记 S4 完成。「库」页签的点选 / 关闭按钮此时顺带实点一次（上一会话留的）。

### 2.5 S5：属性与会话（Q6）

- **4.1 e3d-convert 只加**：`verify.rs` 新增 `ElementDecoder`（一个发布版本的 `TemplateProvider` + noun 缓存）与 `decode(&mut engine, &ReleaseSchema, refno) -> Result<ElementReading, ElementFailure>`；`ElementReading { refno, noun, name, compact, rows: Vec<AttributeReading> }`，`AttributeReading { attribute, value: Option<String>（`DescriptorValue` 的 `Display`）, status（`status_label`）, storage（由 `ExtractedAttribute.storage / layout` 写成「implicit 字偏移 n / explicit / hunk」）}`。`ElementComparer::decode` 内部改成两次调同一段读记录与抽取，`compare` 的签名与结果不变；`corpus.rs` 的数不变即证明。`lib.rs` 导出新类型。
- **worker / 协议**：`Attributes { compare: None }` 走 `ElementDecoder`（按版本 id 缓存）；`Response::AttributesReady { gen, attributes: Attributes }`，`enum Attributes { Single(ElementReading), Compared(ElementComparison) }`；`LibraryEvent::Attributes` 同步。spec 3.1 的 `AttributesReady` 字段随实现改写。
- **4.2 属性表**：`AttributesPanel` 换成真面板，`DataTable`，列 = 名字 / 值 / 状态 / 存储；订阅当前库 `Browse` 的 `Selected` → 没有 Source_Release 显示「请选择源版本」，否则 `request_attributes(refno, source, compare)`；`ElementFailed` 显示种类与原文、不显示部分行。过滤框按名字或值子串；双击一行开对话框显示完整值（文本 / 数组 / hunk 原文）；右键菜单复制值 / 属性名 / refno / 整行（制表符分隔），`cx.write_to_clipboard`；Ctrl+C（已绑在 `AttributeTable` 上下文）复制选中行的值。面板 `key_context("AttributeTable")`。
- **4.3 对照**：工具栏加「对照版本」`Select`（空 / 另一个版本）；设了就带 `compare` 请求，表多「目标值」与「标记」两列（`RowKind::label`，不一致标红、默认值变化标黄），「只看差异」开关过滤 `RowKind::Equal`，`ToggleDiffOnly`（Ctrl+D）从 `not_yet` 接上。
- **4.4**：worker 测试 `acp7048_opens_lists_finds_and_compares` 加断言 `HRSTAR` 1200 / 0、`FRAD` 1200 / 50、`POSTEX` 1000 / 100 且 `RowKind::Mismatch`；界面上对同一元素截图。
- **4.5 会话**：`SessionsPanel` 列 `Library::sessions()`（号、时间、用户、注释、DAB 标签）；行上「按此会话查看」→ `Library::reopen(SessionSelector::Number(n))`；标题栏加「— 会话 n」，「最新」恢复常态。`Browse` 在 `Opened` 后若之前有选中，发 `Find(=refno)` 走 `reveal` 重新定位（需求 5.4 的「仍存在时保持选中」）。
- **4.6 新会话**：状态栏的提示改成可点的按钮 → `reopen(Latest)`；`Refresh`（F5）从 `not_yet` 接上。验证按需求「细化时采用的默认」：Corpus_31 某库的 Scratch_Copy 开着，用 e3d-io-write 的 `e3d-write apply` 给副本追加一个会话，5 s 内出提示；命令与输出进证据。
- **4.7 Property 4**：`corpus.rs` 加一条——对 `acp7048_0001` 全部元素，`ElementComparer::compare` 各行的 `finding` 集合等于 `verify_database` 的发现集合（常跑，0.1 s 级）。
- **4.8** 选中到出属性表 < 10 ms（单版本与对照各量），计时点同 3.10 的做法。
- **4.9** 检查点：同 3.11；S5 证据文档；M1 计划 §5。

### 2.6 S6 并行约束（若另开会话，Q8）

S6 只依赖 S1、S3，可与 S4、S5 并行。为了不打架：

- 文件：`src/convert/{mod,wizard,job}.rs`、`panels/jobs.rs`、`panels/report.rs`、两份 locales 各加 `Convert.* / Jobs.* / Report.*` 一节；`workspace.rs` 只改 `on_convert` 的函数体（`Convert` action 与工具栏按钮已在）；不改 `db/`、`panels/tree*`、`panels/attributes.rs`、`app.rs`、`Cargo.toml`。
- 要用 S4 / S5 的地方按这些名字写，S4 / S5 保证提供：`OpenLibraries::current()`、`ProjectInventory` 的行（E 之后；之前用「当前库 + 选文件」顶着）、`ProjectReleases` 查源版本、`Browse::reveal(refno)` 与 `Workspace::set_compare_release(version)`（报告步「双击回到对照」用，S5 之前只做到选中）。
- 提交只 `git add` 自己的路径；共享 `D:\Rust\target` 的锁会让两边编译互相等，接受。
- S6 内部顺序按 spec 5.1–5.10；5.8 的比对脚本不可跳过。

### 2.7 S7 前提

- 2.8 的 4 项用户手验（对话框选中后路径进状态栏、拖入、菜单弹出、语言下拉），S3 收口要它。
- S1.6 与 6.7 要真 E3D，等 10.5 的 `des.exe` 退出或用户明说可并用。
- 收口 P3 把 e3d-io `main` 快进后，`Cargo.toml` 的 `e3d-io` 改指 `../e3d-io`，重跑 `corpus.rs`（含 `--ignored`）；在那之前任何 e3d-io 读侧新接口都不加（本计划不需要加）。
- 设计 Q5（大库上计划步的耗时）在 S6 的 5.2 量。

## 3. 待定问题（每题第一项为推荐）

- **Q1 树状态落点。** A（推荐）每库一个 `Browse` 实体（树模型、在途请求、选中、定位历史），`OpenLibraries` 持有，面板只渲染；关面板、切库、换布局都不丢。B 放 `TreePanel` 里，面板重建即丢，面包屑与属性表另找选中。
- **Q2 搜索命中怎么落到懒加载树。** A（推荐）`Found` 的每级带 `ElementRow` 与该元素在 owner 成员表中的序号，worker 走 owner 链时顺带读，界面把这一行直接填进槽；协议改一处字段，spec 3.1 与 design 协议表同步，D10 不变。B 命中后对链上每个 owner 请求整段成员——8 200 个成员的 ZONE 要全读，违背需求 3.2。
- **Q3 面包屑位置。** A（推荐）工具栏之下、dock 之上一条 `Workspace` 级横条，任何布局下都看得见。B 树面板顶部。C 属性面板顶部。
- **Q4 项目根怎么从库路径推。** A（推荐）库所在目录名形如三个字母 + `000`（`acp000`、`ams000`）时取其父目录为项目根，否则取所在目录；打开项目文件夹时就是该文件夹。B 一律取所在目录——同一项目下不同子目录里的库各算一个根，要选多次。C 每次让用户指。
- **Q5 源版本对话框的提示项。** A（推荐）DAB 标签（标出 `2_28_*` / `2_35_*` 对应的引擎）、`CRINFO`、路径、会话注释是否含 `Saving Meta Data.`（即需求 2.2 的 `DBBLBX` 痕迹）；都来自已打开的库，不等待。B 再加 `project_version_evidence` 的 `COMM.VERS`，后台扫整个项目，结果迟到再补进对话框。
- **Q6 单版本解码的形状（4.1）。** A（推荐）新增 `ElementDecoder` / `ElementReading` / `AttributeReading`（带存储列），`ElementComparer::decode` 内部改走同一段读记录，公开接口与语料数字不变；`AttributesReady` 载荷改成「单版本 / 对照」枚举。B 不加接口，用 `ElementComparer` 以 source = target 的 pair「自己对照自己」充当单版本——多解一遍、没有存储列、要给每个库再建一套模板缓存。
- **Q7 性能实测的构建。** A（推荐）按需求 7.6 用 release 构建，首编一次（估计 5–10 分钟，要抢 `D:\Rust\target` 的锁），数字直接可记。B 先用现有 dev 构建（成员 opt-level 1、依赖 3）量一次作参考，release 留到 S7 的 6.1 一起量。
- **Q8 S6 要不要现在另开会话。** A（推荐）等 E（3.5 库列表）提交后再开，选库步少写一段「当前库 + 选文件」的临时代码。B 现在就开，按 §2.6 的约束。
- **Q9 3.5 的「会话数」列。** A（推荐）列显示最新会话号（`DatabaseEntry.session` 已有），需求 2.5 的字眼改成「最新会话号」，用户确认后记入 Spec_Ledger「决策」。B e3d-convert 加读会话数，每库多走会话链，450 个库的列表难过 2 s。
- **Q10 无名元素的「位置名」口径。** A（推荐）M1 写成 `noun 序号`（序号 = 在 owner 成员表里的位置，1 起），worker 读窗口时就有，不另读兄弟；design 的 Lazy_Tree 一节注明口径。B 按 E3D 的同类型序号（`ZONE 3 of /SITE` 里的 3 只数 ZONE），要读窗口之前所有兄弟的 noun——8 200 成员就是 8 200 次索引查找，耗时未量，可能过不了 150 ms。

## 4. 不做

- M2、M3（会话差异、记录检查器、引用我的、schema 差异视图、SLADDR 重写、3.1 → 2.1、项目级转换）。
- 不改 D1–D14、不改设计 §5 的七条验收；spec 的 3.1 协议写法与「位置名」口径随实现同步（Q2、Q6、Q10 定后）。
- 不改 e3d-io-write 的源码与测试；不碰 10.5 的两个未跟踪脚本；不动别的会话的未提交改动。
- 不在本计划内做 2.8 手验、S1.6、S7，只列前提（§2.7）。
- 不推送、不建远端。

## 5. 执行记录

（按步追加：日期、提交、跑了什么、数字；与 spec 的 Spec_Ledger 和 M1 计划 §5 对应。）

- 2026-10-10 **C 完成**（spec 3.6 打勾），`e3d-studio` `e9c4f91`。
  - 落点与 §2.1 一致：`db/browse.rs` 的 `Browse` 实体（`OpenLibraries::Opened` 多持一个 `Entity<Browse>`，`current_browse()` 给面板；字段 `tree`、`loader`、`visible`——`trail` 留到 D 与 `Trail` 一起加）；`Loader::want / answered`（`answered` 只认 owner：gate 已保证到达的是最新一代，而 worker 会把窗口夹到成员数，答回的窗口未必等于请求的）；`TreeModel` 加 `parent` / `ancestors` / `line_of`；`panels/tree.rs` 的 `uniform_list` 渲染、`cx.defer_in` 报可见区、`ElementTree` 上下文的 `TreeUp / TreeDown / TreeCollapse / TreeExpand / TreeToggle`；选中变化 `scroll_to_item(Center)`（点选的行本就在视野里，不动）。
  - 顺带改了 `TreeModel::wanted`：预取放宽的窗口不再圈进已读的槽（实跑看到 TPWL 13 个成员先读 `0..10` 再重读 `0..13`，改后是 `0..4` + `4..13`）。
  - 读不出成员的 owner：`ElementFailed` 到了就把它折回去（否则未读槽会反复要），原文由 `Workspace` 放进状态栏与日志（locale 键 `Library.ElementFailed`）。
  - 测试 `cargo test -p e3d-studio --offline` 28 / 0（+4：`Loader` 2 条、proptest「任意树 / 展开 / 可见区序列下在途至多一个且可见未读槽终被读到」、`ancestors`）；`cargo clippy --all-targets` e3d-studio 0、e3d-convert 3（基线）；`cargo doc --no-deps` 0。
  - 界面实跑（证据 `docs/evidence/2026-10-09-e3d-studio-s4.md` 2026-10-10 节，截图 03–05）：`acp7048_0001` 点箭头 / 双击 / ↑ ↓ ← → Enter 全套；`ams5052_0001` 的 621 成员 CCTA 滚轮下滚，请求 `0..35 → 35..70 → 70..107 → 107..141 → 141..178 → 178..212`，每个到了才发下一个、首尾相接。「…」槽在截图里抓不到（毫秒级就填上）。
  - 驱动注意事项（补 S3 证据那节）：这版 cua-driver 的 `click(pid, x, y)` 以客户区物理像素偏移计，不按截图比例换算；GPUI 不吃 PostMessage 点击，要 `delivery_mode: "foreground"`；`press_key(pid)` 的 PostMessage 按键能到；滚动要自己 `SetCursorPos` + `mouse_event(WHEEL)`；GPUI 的 UIA 提供者在窗口不在前台时常超时，截图用 `CopyFromScreen`。桌面坐标（`scope: "desktop"`）的点击会落到当时在前面的别的窗口，一次误击让 Chrome 下载了 `ams7351_0001` 的副本，已删。
  - 3.10 用得上的数：`ams5052_0001`（CATA，77 810 页）里成员最多的是 `SECT /CNPESPECGEN_DESCRIPTIONS/SDTE =13244/218060`（24 104 个成员，WORLD 第 33 个成员 `CATA /CNPESPECGEN-NI` 的第 1 个成员）；`acp7048_0001` 最多的是一个 TMPL 42 个。`ams7351_0001`（CATA，604 643 页）没扫。
  - 下一步 D（§2.2）。
- 2026-10-10 **D 完成**（spec 3.7、3.8 打勾），`e3d-studio` `5558ba6`。（此条迟记，主计划 §5 已先行。）
  - `Found.path` 改 `Vec<FoundStep>`（每级 `row` + `index`），`Browse::reveal` 沿链 `fill` + `expand` 把命中填进懒加载树；`Trail`（上限 200 的带游标栈）与 Alt+← / Alt+→；`Workspace` 级面包屑（gpui-kit `Breadcrumb`）。
  - 测试 `cargo test -p e3d-studio --offline` 30 / 0；clippy e3d-studio 0、e3d-convert 3（基线）；doc 0。实跑 `=15240/1968` 展 6 层、`/SLH_Template_World`、Alt 往返、未命中与格式错误提示；证据 `docs/evidence/2026-10-09-e3d-studio-s4.md` 2026-10-10 节与截图 06–08。
  - 下一步 E（§2.3）。
- 2026-10-16 **E 完成**（spec 3.4、3.5 打勾），`e3d-studio` 待提交。
  - 3.4：`db/source_release.rs`——`ProjectReleases`（项目根 → release id，`project-releases.json` 存 App_Data_Dir；坏文件报错降级空表，未知 id 视为未选）、`project_root_of`（`xxx000` 归项目根）、`dab_tag_release`（`2_28_*`→2.1、`2_35_*`→3.1，e3dio001 等不算数）、`current_root`。`workspace.rs`：开项目/开库时 `want_release_prompt`（每根每运行一次，Esc/暂不选记已问；队列一次一个）；对话框三提示（DAB 标签+引擎映射、CRINFO、`Saving Meta Data.` 痕迹）不预选、未选禁「确定」；工具栏 `源版本` Select 改当前根；标题栏 `— 库名（源版本）`；属性面板未选显「请选择源版本」（`panels/attributes.rs` 从宏壳换成真面板）。
  - 3.5：`db/inventory.rs`——`ProjectInventory`（每项目根一扫描线程：`project_database_paths` 先占位、逐库 `describe` 三页头回填、30ms 节流、drop 取消）与 `OpenProjects` 全局；`sorted`/`filtered` 纯函数（缺值永排尾）。`panels/library.rs` 下半段 `DataTable` 八列（路径/类型/大小/会话/DAB/extent/核验/错误），排序、框过滤、双击或 Enter 打开行；核验列走 `SchemaPair::verifiable`。
  - 修了两个真错：排序比较子里 `None` 在降序时跑到最前（拆开缺失判定与方向）；`open_dialog` 在 Root 更新中嵌套调用 panic（`cannot read Root while it is already being updated`），改 `cx.defer_in` 延一帧。
  - 测试 `cargo test -p e3d-studio --offline` 37 / 0（+7）；`cargo test -p e3d-convert` 11 / 0 + 3 / 0（3 ignored）不变；clippy e3d-studio 0、e3d-convert 基线；doc 0。
  - 实跑 AvevaCatalogue（证据同上文件 2026-10-16 节与截图 09–12）：首开弹框三提示齐全不预选、确定禁用；radio+确定写 `project-releases.json`；暂不选后属性区显「请选择源版本」；清单 44 个库逐行出、过滤 `7001`、大小降序、双击 acp7320 开库；重启不再弹、下拉框显示已存值、核验列标「不在核验范围」。
  - 下一步 F（§2.4）。
