# 规格修订：Built_In_AABB 并入成员框；祖先任一重算属性、成员次序与 IMAP 3 成员的重算（里程碑 D 收尾）

状态：**评审通过**（2026-10-03 22:35，Plannotator 门 `approved`，无批注 → Q1–Q4 全按推荐项），已照本稿改入三份 spec 文件
日期：2026-10-03
分支 / worktree：`write-path` @ `D:\work\plant-code\old\vendor\e3d-io-write`，HEAD `d8fecb6`
依据：`docs/evidence/2026-10-03-spamap-recompute.md`（D0 / D2）、`docs/evidence/2026-10-03-spamap-aabb-members.md`（D3）；D 计划 §6；2026-10-03 20:52 的决定（祖先其他重算属性与容器 REORDER「补进规格」）
改动范围：`.kiro/specs/write-path-phase-3/{requirements,design,tasks}.md`。只改规格文字；读侧 Built_In_AABB 已在 `628d741` 落地，写侧（第 8 组）尚未实现，按改后的规格做。

---

## 0. 一句话

里程碑 D 的取证查出规格有三处与 E3D 不符，本修订一并补上：

1. **Built_In_AABB 少算了成员**（D3）：E3D 的 `MPBOX2` 对 BOX、CYLI 这类 IMAP 5 元素，把它的 IMAP 3 成员（负实体 NBOX、NCYL、NCON、NPYR 等）的框并进元素框。规格只写了「由自身属性与 owner 链」。
2. **祖先的重算口径太窄**（D2）：规格只让「换 owner、改 POS / ORI」重算后代；E3D 对祖先任一重算属性的改动（如 EQUI 的 NUMB，DCHC 4）都重算整棵子树，元素在 owner 成员表中被挪次序（`reorder`）时也重算它与它的整棵子树。需求 7 第 6 条还把「只含 `reorder` 的批」举为 Recompute_Set 为空的例子，与 D2 实测相反。
3. **没写 IMAP 3 成员的改动**（D0 + D3）：IMAP 3 元素没有自己的条目，改它时 `MLSADD`（n 30–42）上溯到第一个 IMAP 不为 3 的祖先入队，删它时 `MPDEL`（n 27–30）把 owner 入队；这个祖先的框并入了成员，所以要重算。

## 1. 修订一：Built_In_AABB 并入 IMAP 3 成员的框

| 位置 | 现文 | 改为 |
|---|---|---|
| 术语表 **Built_In_AABB** | e3d-io 自行计算 World_AABB 的 noun 集合（D1 推荐默认：BOX、CYLI）。 | e3d-io 自行计算 World_AABB 的 noun 集合（D1 推荐默认：BOX、CYLI）。所算的 World_AABB 含该元素 IMAP 3 成员的框（`aabb.members`），成员只算 Built_In_AABB 成员清单中的 noun（NBOX、NCYL、NCON、NPYR）；有清单外的 IMAP 3 成员时没有自算值（需求 6 第 15 条）。 |
| 需求 6 第 1 条 | …每个 Built_In_AABB noun 由自身属性得出包围盒的方法（解析解、局部外包盒变换或离散面），以及尺寸属性为 0 或负时 E3D_Core 的处理。 | …每个 Built_In_AABB noun 由自身属性得出包围盒的方法（解析解、局部外包盒变换或离散面），E3D_Core 并入元素框的成员（哪些 IMAP、在什么坐标系）及 Built_In_AABB 成员清单中每个 noun 的包围盒方法，以及尺寸属性为 0 或负时 E3D_Core 的处理。 |
| 需求 6 第 2 条 | …由该元素在本批结束状态中的属性与 owner 链，按第 1 条坐实的方法算出 World_AABB… | …由该元素及其 IMAP 3 成员在本批结束状态中的属性与 owner 链，按第 1 条坐实的方法算出 World_AABB… |
| 需求 6 第 15 条 | …（如 k ≥ 1 条目的值字来源；调用方每元素只给一个 World_AABB）… | …（如 k ≥ 1 条目的值字来源；调用方每元素只给一个 World_AABB；Built_In_AABB 元素有 noun 在成员清单之外的 IMAP 3 成员时，该成员的框）… |
| design 规则表 | `aabb.box`、`aabb.cyli`、`aabb.degenerate` 一行 | 拆为：`aabb.box`（BOX、NBOX）、`aabb.cyli`（CYLI、NCYL）、`aabb.cone`（NCON）、`aabb.pyramid`（NPYR）、`aabb.members`（IMAP 5 元素并入 IMAP 3 成员框及成员坐标系）、`aabb.degenerate`；出处需求 6 第 1 条 |
| design Built_In_AABB 节 | `SpatialState` 三个方法；`AabbOutcome` 四个变体；`aabb.degenerate` 只在尺寸 ≤ 0 时查 | 加 `SpatialState::members`、`BUILT_IN_MEMBER_NOUNS`、`AabbOutcome::MissingInput`；`aabb.degenerate` 在尺寸小于 0.5 mm 时查（`8cd4f8c` 已改实现）；`aabb.members` 只对有成员的元素查 |

## 2. 修订二：祖先任一重算属性与成员次序

| 位置 | 现文 | 改为 |
|---|---|---|
| 术语表 **重算属性** | 需求 5 第 2 条重算表「自身」一类中取值「触发」的属性；取值「未坐实」的行按 D7 处理。 | 需求 5 第 2 条重算表「自身」一类中取值「触发」的属性；取值「未坐实」的行按 D7 处理。E3D_Core 按属性字典判定、与 noun 无关（`spamap.recompute`：DCHC 不为 0 且不带 WNOEVT），所以对不属 Geometric_Noun 的祖先（EQUI、SUBE、ZONE 等）同样适用。 |
| Recompute_Set ② | 上一会话已有、至少一个重算属性在本批结束状态的值与上一会话不同的每个 Geometric_Noun 元素 | 上一会话已有、至少一个重算属性在本批结束状态的值与上一会话不同，或在 owner 成员表中的次序在本批改变的每个 Geometric_Noun 元素 |
| Recompute_Set ④ | 上一会话已有、owner 在本批结束状态与上一会话不同的元素，其…后代中需求 5 第 3 条坐实… | 上一会话已有、owner 在本批结束状态与上一会话不同，或在 owner 成员表中的次序在本批改变的元素，其…后代中需求 5 第 3 条坐实… |
| Recompute_Set ⑤ | 上一会话已有、POS 或 ORI 在本批结束状态与上一会话不同的元素，其…后代中需求 5 第 8 条坐实… | 上一会话已有、至少一个重算属性（不论 noun；POS、ORI 在内）在本批结束状态与上一会话不同的元素，其…后代中需求 5 第 8 条坐实… |
| 需求 5 第 8 条 | …E3D_Core 改变一个元素的 POS 或 ORI 时，是否…对该元素的 Geometric_Noun 后代做 SPAMAP_Recompute。须按改动的属性（POS、ORI）与层… | …E3D_Core 改变一个元素的 POS、ORI 或其他重算属性时，是否…对该元素的 Geometric_Noun 后代做 SPAMAP_Recompute。须按改动的属性（POS、ORI，其他重算属性至少一例）与层…（D2 已以 POS、ORI、EQUI 的 NUMB 坐实） |
| 需求 7 第 4 条 | …使一个上一会话已有的元素的 POS 或 ORI 在本批结束状态的值与上一会话不同，THE SPAMAP_Writer SHALL 使需求 5 第 8 条坐实 E3D_Core 在祖先 POS / ORI 改变时会重算的它的每个 Geometric_Noun 后代…（该元素自身的条目按第 2 条）。 | …使一个上一会话已有的元素至少一个重算属性（不论 noun；POS、ORI 在内）在本批结束状态的值与上一会话不同，或使它在 owner 成员表中的次序改变（`reorder`），THE SPAMAP_Writer SHALL 使需求 5 第 3、8 条坐实 E3D_Core 会随之重算的它的每个 Geometric_Noun 后代…（该元素自身的条目按第 2 条）。 |
| 需求 7 第 6 条 | …Recompute_Set 为空，如一批只含 `reorder`、引用表增删、既不在需求 5 第 2 条属性全集内也不是 POS / ORI 的属性改动… | …Recompute_Set 为空，如一批只含引用表增删、不是任何元素重算属性的属性改动… |
| design SPAMAP_Writer 节 | 「重算属性取 `recompute_row` 为 `Triggers` 或 `Unconfirmed` 的属性；④⑤ 只收 `descendants_included` 为真的那些层。」 | 同上，另加：⑤ 的祖先可以是任意 noun，`recompute_row` 在 8.4 扩到 Recompute_Set 走到的祖先 noun（现只覆盖 Geometric_Noun 模板）；成员次序按 `member_order_row` |

## 3. 修订三：IMAP 3 成员的改动

| 位置 | 现文 | 改为 |
|---|---|---|
| Recompute_Set | ①–⑥ | 加 ⑦：本批插入、删除、换了 owner、改了成员次序或至少一个重算属性的每个 IMAP 3 元素，其第一个 IMAP 不为 3 的祖先（插入与改动按本批结束状态，删除与换出按上一会话），该祖先属 Geometric_Noun 时（`MLSADD` n 30–42、`MPDEL` n 27–30；其框按 `aabb.members` 并入成员） |
| 需求 7 | 第 1–13 条 | 加第 14 条：WHERE Spatial_Policy 为「要求」或「跳过」，WHEN 批次插入、删除、改动、挪动或重排一个 IMAP 3 元素，THE SPAMAP_Writer SHALL 使 Recompute_Set ⑦ 所指的祖先在新会话表中的条目恰为它的 Target_Entries。 |
| tasks 8.4、8.5、Property 9 | Recompute_Set 按定义 ①–⑥ | 按定义 ①–⑦；参考实现与 proptest 生成器加成员次序、祖先非几何重算属性、IMAP 3 成员三类改动 |

## 4. 不改的

- 需求 4 第 17 条：18 个反例由修订一解释，不需要新的剔除依据（D3 开始时提过的「E3D 重算对照」不再加）。
- 需求 6 第 16 条：Built_In_AABB 1 590 / 1 590 逐字，不放宽。
- `BUILT_IN_NOUNS` 与 D1：仍是 BOX、CYLI；成员清单另列，不算扩充 D1。

## 5. 待你拍板

批准本稿 = 全按推荐项（A）。要改哪条，在对应行批注。

| Q | 问题 | 推荐（A） | 备选 |
|---|---|---|---|
| Q1 | Built_In_AABB 成员清单 | NBOX、NCYL、NCON、NPYR：Corpus 里出现的 IMAP 3 成员只有这四种，45 / 45 覆盖；其余负实体（NSNO、NDIS、NCTO、NRTO、NXTR、NREV、NPOLYH、NSLC）与非实体 IMAP 3 noun 按 D7 要调用方给框 | B：再读 `BOXELE` 其余 box type，补全全部负实体（Corpus 无实例，只能以 IDA 坐实） |
| Q2 | 祖先「重算属性」的口径 | 按属性字典（DCHC 不为 0 且不带 WNOEVT），不论 noun；8.4 把 `recompute_row` 扩到祖先 noun | B：只收 D2 实测过的 POS、ORI、NUMB，其余祖先属性按 D7 视为触发（宁多勿少） |
| Q3 | IMAP 3 成员改动（修订三） | 写进 Recompute_Set ⑦ 与需求 7 第 14 条，第 8 组实现 | B：本期不做，记入 Spec_Ledger「本期不做与触发条件」（e3d-io 改负实体时其 owner 的条目可能过期） |
| Q4 | 落地方式 | 评审通过后照本稿改三份 spec 文件，一个提交；第 8 组计划（里程碑 E）按改后的规格写 | B：先只改需求文字，design / tasks 留到写 E 计划时再改 |
