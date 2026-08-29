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
- 手上的实物：`E:\reverse\e3d\shadow_e3d31_aps_all\*vir.dat`（20 个）、`attlib.dat`、
  AMS 样例库、以及一份只覆盖 WORL 的 live 内存 dump

## Out of scope

- 写回（writeback）。先把读侧的 map 做完整。
- 几何求解、模型生成、下游渲染。
- E3D 2.10 及其他版本的常量差异。
