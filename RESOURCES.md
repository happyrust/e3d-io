# E3D 3.1 二进制解析 Resources

这套格式没有公开规范。所有可信来源要么是二进制本身，要么是从二进制推出来并留了证据链的文档。
凡是"网上说"的一律不进这个表。

## Knowledge

- [`pdms-io-fork-engine-v2/docs/ida-3.1-structures.md`](file:///D:/work/plant-code/pdms-io-fork-engine-v2/docs/ida-3.1-structures.md)
  最高权威。§14 是描述符表的完整证据链，逐条指回 `sub_5AFA740`（文件头 + 分块读）、
  `sub_5AF6AB0`（二分查找）、`sub_5B03900`（payload 游走）。
  **用于**：任何"这个字段为什么是这个含义"的问题。

- [`pdms-io-fork-engine-v2/docs/ida-3.1-attributes.md`](file:///D:/work/plant-code/pdms-io-fork-engine-v2/docs/ida-3.1-attributes.md)
  ATGTDF / ATGTIX / EXMAP 的语义类型体系。
  **用于**：`data_type` 与 `ExpType` 的对应、默认值池的线索。

- [`pdms-io-fork-engine-v2/ida_exports/3.1/`](file:///D:/work/plant-code/pdms-io-fork-engine-v2/ida_exports/3.1)
  IDA 导出的原始反汇编与结构布局。文档和它冲突时，以它为准。
  **用于**：核对常量（`DATA_PAGE_MAIN = 7618377`、`DATA_PAGE_INDEX = 13387743` 等）。

- `E:\reverse\e3d\shadow_e3d31_aps_all\*vir.dat`（20 个文件）
  实物。任何关于布局的断言都应该能在这里当场验证。
  **用于**：全量统计、跨库比对、反例查找。

- `src/record/{template_file,template,descriptor,explicit}.rs`
  本仓库的读路径实现，注释里带 IDA 函数号。
  **用于**：想知道"代码实际怎么分派"，比读文档快。

## Evidence（一次性但高价值的实测）

- `D:\work\plant-code\old\gen-model\output\noun_descriptor_slots.json`
  从运行中的 `core.dll` 内存 dump 出来的描述符表（`coreBase=0xf6c0000`、`rvaDbStack=0x18e4024`）。
  只覆盖 WORL 一个 noun，7 条描述符 × 10 dword，与磁盘解析结果**逐字相等**。
  **用于**：证明磁盘解析等价于运行时行为。也是把它扩成全量回归的起点（当时 `dbsWalked=1 / dbsInMdb=69`）。

- `evidence/20260826-noun-catalog/`
  BRAN 的 TTY `ATTLIST` 对账（121 行零遗漏）、describe / extract-complete 的实测输出、构建与测试日志。
  **用于**：任何"改动前后行为有没有变"的比较基线。

## Wisdom（Communities）

暂无。AVEVA E3D 的数据库格式没有公开的技术社区，官方渠道不讨论文件格式。
最接近"同行评议"的做法是：**任何新结论都在第二个独立来源上复现一次**——
磁盘解析 vs 内存 dump、二进制 vs TTY 输出、DESIGN 库 vs 其他 19 个库。
这条工作纪律代替社区，写进 `NOTES.md`。

## Gaps

- **UDA 的物理定义**从哪来，尚无来源。`dicvir.dat`（DICT 库，95 个 noun）是最可能的地方，未验证。
- **默认值池**（`0xFFFFFFFF` 哨兵指向的那张表）在 IDA 里是 `dword_6C21390[dword_6C21200[i-1]-1]`，
  但还没有从文件里定位到它。
- **word 6–9** 那四个无描述符认领的字，含义未知。做 writeback 之前必须搞清楚。
- **storage_type_code 的完整语义**只有部分是确认的（5/9 布尔、2/6 实数、4/8/16/17 引用），
  7、10、14、15、18、19 目前靠回落逻辑处理。
