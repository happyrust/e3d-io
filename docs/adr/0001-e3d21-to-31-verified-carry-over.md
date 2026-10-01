---
status: accepted
---

# E3D 2.1 → 3.1 的转换是原样携带加等价核验，不改写字节

E3D 3.1 就地使用 2.1 写的库：AVEVA 自带的 3.1 样例里，AvevaMarineSample 的 455 个库有 425 个与 2.1 版逐字节相同（其余是 3.1 在原文件上追加了会话，或只差页 0 的动态字），AvevaCatalogue 的 44 个全部相同；两版 `desvir` / `catvir` / `dicvir` 的模板版本字（头字 3）都是 1。本机 E3D 2.1 下 536 个 DESI / CATA / DICT 库、1 535 万个元素在两版 schema 包下逐属性解码，2.65 亿行里除 SLADDR 的 52 行外全部相等（2026-09-30 实测，见 `docs/plans/2026-09-30-e3d-studio-gui-design.md` §1）。所以 2.1 → 3.1 的转换把库连同全部 extent 原样拷出，再在源、目标两个 schema 包下逐元素比对，比对通过才落到目标位置。证明「目标版本读到的就是源版本的数据」的是核验，不是改写。

## Considered Options

- **规范化**：追加一个会话，把 compact 记录展宽成 wide，按 3.1 描述符重排每条记录。E3D 3.1 自己只在第一次修改某条 compact 记录时才展宽它（`sub_5AECBE0`，`docs/evidence/2026-09-22-core-dll-record-draft-data-flow.md` §1.3），而语料里 78.1 万条 compact 记录在两版下读法相同。重写只带来风险（写路径拒 compact、只写单 extent），没有可核验的收益。
- **新建库重灌**：e3d-io 不能建库，而且会丢掉会话历史。

## Consequences

- 有迁位属性的 noun 会被核验判为不一致。目前只有 SLADDR：3.1 删了 `FOOTRO`，其后 13 个属性前移，2.1 写的梯子在 3.1 schema 下读到的是前一个属性的值。默认不落位、逐元素点名；E3D 3.1 实际怎么读它，由一道 E3D 门坐实后再定是补读侧规则，还是加一步按 3.1 布局重写。
- 默认值变化（`UDA.UCASES`、`CABLE.FILCHK`）按目标版本显示，只作提示：E3D 3.1 读同样的字节也是这么显示的。
- 需要改写字节的转换路线（如 3.1 → 2.1）另立 ADR，不在本决定内。
