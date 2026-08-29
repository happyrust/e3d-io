# 描述符层已经掌握，起点不在"这是什么"

第一次会话里已经走通了整条读路径：`*vir.dat` 的容器格式（2048 字节页、magic=6、
child 数组按 hash 排序二分查找、511+1 分块链）、描述符的十个 dword、
`encoded_location` 的低 20 位／高位切分、`format word` 第 29 位选 compact/wide、
以及 offset=0 表示"不在 implicit 区"。这些不需要重讲。

## Evidence

- 主动追问"物理 offset 是否已经有了"，说明已经把"有描述符"和"描述符里有位置"区分开了
- 主动提出跟 noun map JSON 对比，方向正确（虽然那份 JSON 恰好没有 offset）
- 全程按二进制事实提问，没有一次要求"大概解释一下"

## Implications

- 起点应该是**产出物的正确性**，不是格式入门。第一节课因此定为"十个 dword + 三个会让 map 出错的陷阱"，
  而不是"E3D 数据库是什么"。
- 下一步该攻的是**覆盖率空洞**，按价值排序：UDA（`dicvir.dat`，全新领域）＞
  默认值池（IDA 有线索，未落地）＞ word 6–9（writeback 前置，但当前 mission 里 writeback 已 out of scope）。
- 讲任何新东西时，默认可以直接用 `stride` / `storage_words` / `compact` / `wide` /
  `implicit 区` / `explicit 尾巴` 这些词，不用再解释。
