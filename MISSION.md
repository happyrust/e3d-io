# Mission: E3D 3.1 noun 描述符与完整 noun map

## Why

产出一份能覆盖 E3D 3.1 全部类型的 noun map JSON，让下游工具不必再打开 `core.dll`、
不必再开 E3D TTY，就能知道任意 noun 的任意属性在二进制记录里的确切位置、尺寸和类型。
这份 map 是后续所有解析/导出工作的地基——它错一个 offset，下游全错，而且不报错。

## Success looks like

- 能说出任意一条描述符十个 dword 各自的含义，不用查文档
- 拿到一段元素记录的原始字节，能手工推出某个属性占哪几个字、值是多少
- 知道自己产出的 map 在哪些类型上**一定是对的**、哪些地方是**已知空洞**，并能说出空洞的成因
- 能判断一个新增的数据源（live 导出、TTY、内存 dump）该并进 map 的哪一层，而不是覆盖掉物理权威

## Constraints

- 唯一权威是二进制与 IDA 证据，不接受"看起来像"的推断；每个结论要能指回具体字节或具体反汇编
- 现有代码 `e3d-io` 已经跑通读路径，学习要贴着这份代码走，不另起炉灶
- 手上的实物：`schema/e3d31/binaries/` 里的 `*vir.dat`（20 个）与 `attlib.dat`、
  AMS 样例库、以及一份只覆盖 WORL 的 live 内存 dump

## 写回（2026-09-20 起纳入）

读侧的 map 做完整之后，同一份地基反过来用：`e3d_io::write` 往 E3D 3.1 库**追加一个会话**，
写出的文件本 crate 的读侧原样读得回来，真 E3D 3.1 Design 也认（TTY 门 39 问 PASS：Q0、既有 28 问、Q13/Q13a–h 与 Q14，
`docs/evidence/2026-10-05-allocation-q14-gate.md`；Q15 追加验证见 `docs/evidence/2026-10-06-s6b-q15-gate.md`）。另有交替写门 Q15 验证页 1 桶计数器推进后 E3D 不复用已删编号。Transaction 原地抬页 1 并同步，再由 page 0 提交会话；两者之间失败可能空掉一段编号，上一会话仍可读。口径与读侧一样只认二进制与 IDA 证据：
每个写侧模块都是对应读侧的逆函数，读侧是它的验收方（`docs/plans/2026-09-20-dabacon-write-plan.md`
§7 是逐步的账，`docs/plans/2026-09-21-write-path-next-steps.md` 是收口与下一步）。

## Out of scope

- Built_In_AABB 之外的几何求解、模型生成、下游渲染。写侧维护 `SPAMAP`：优先调用方世界框，其次自算 BOX / CYLI 与已坐实成员聚合；默认 Require，缺框拒写，调用方可显式 Skip 或 Off。
- E3D 2.10 及其他版本的常量差异；写侧只写 3.1 单 extent 文件。
