# 2026-10-05 发号计数器与 INDEX 次序（11.1）

## 环境

- HEAD：`03364323be382c3a42a8b46e6b7620404946932a`，其上未提交的只读诊断与探针。
- Core_IDB：`D:\ida_scratch\plant3\core.dll.i64`；IDA 9.2；只通过 IB_Wrapper 查询，不保存 IDB。
- Core_IDB 开始 SHA-256：`18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C`。
- Core_IDB 结束 SHA-256：`18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C`。
- 真 E3D 3.1 TTY，经现有 `l3_suite --check-driver` 驱动；独立 scratch 项目副本。源库 AMS8000 长度 34 037 760；运行前后哈希相同，见保存的 result.json。
- 登录差异：计划的 `e3dio/XXXXXX` 未进入场景，驱动异常退出，scratch 最新会话仍 646。按既有门配置用 SYSTEM 登录完成探针，`Q USER` 为 SYSTEM，SAVEWORK 会话 User 为 Windows 账户 dpc。此处不宣称原生会话 User=e3dio；CLI 的 User=e3dio 是另一个会话字段。源项目账户未修改。
- 原始行号和指令、TTY 查询、诊断与测试输出保存在 [assets](assets/2026-10-05-allocation/)。

## 结论

### alloc.counter_semantics

页 1 的计数器是 **NextAvailable**：首个待发 word1 就是存储值，Bucket_Floor 不再加一。写侧后续取 `max(index_max + 1, Bucket_Floor)`；本提交只确认规则，不修改 Transaction 的发号行为，不写页 1。

可区分语义的调用路径：

- `db_get_bucket` @ `0x5aadd90`，pseudocode.n 3：routine 360 派发到 `sub_5AC3810`；后者 @ `0x5ac3810` n 35 调 `sub_5AF60F0`。后者 @ `0x5af60f0` n 5 调 `sub_5AF59E0`。
- 桶读取 @ `0x5af59e0` n 26–32 读物理页 1 并检查类型，n 82–83 直接复制两字到输出，不加一；n 96 移除已领取的桶。
- NEW 建块 @ `0x5ab2d40` n 137 调 `sub_5AEA9C0`。真正发号 @ `0x5aea9c0` n 36 读取控制块属性 7 到输出 a3；缓存为空时 n 194 调 `sub_5AF7480`。后者 @ `0x5af7480` n 76–78 同样直接复制页 1 两字，第三字登记用户。
- `sub_5AEA9C0` n 203–212 保留输出 a3，把 `a3[1] + 1` 写回缓存属性 7；n 205–208 对原生有符号序号上界拒绝。因此返回的是当前待发号，缓存推进的是下一待发号。
- `DB_DB::updateBucketList` @ `0x59144e0` n 125–153 的 live max 更新不是计数器取值公式，不能仅从“计数器大于活最大”推断语义。旧 2026-09-22 文档的 max+1 收敛推断不足以覆盖保留号码，本次以直接复制、缓存推进及真 NEW 为准。

真实探针（SYSTEM，源库会话 646；独立 Q REF / Q SESSION 标记）：

| 时点 | 精确会话 | 页 1 / 24384 | 活最大 | 原生新 refno |
|---|---:|---:|---:|---|
| 登录前 | 646 | 26506 | 26483 | — |
| 登录后、NEW 前 SYSCOM 复制 | 646 | 26506 | 26483 | — |
| 首次 NEW，SAVEWORK 后复制 | 647 | 26507 | 26506 | 24384/26506 |
| 删除首次元素，再 NEW，SAVEWORK | 648 | — | — | 24384/26507 |
| 退出后的单独诊断 | 649 | 26508 | 26507 | — |

SAVEWORK 的 647/648 来自日志，不以退出后的 649 冒充。删除后下一次 NEW 没有重用 26506。`before-new` 与 `after-first-save` 文件在同一 TTY 内用 SYSCOM 复制，消除了另一次登录可能重建桶的歧义。

四库复核：AMS8000 为 `(24384,26506),(32576,98)`，对应活最大 26483/31，主扩展 16192 无桶项；AMS251142 为 `(2013287686,947)`、活最大 946；TES023 为 `(17419,331)`、活最大 315，主扩展 9227 无桶项；AMS5100 字典无桶项。数字取自本次文件，不沿用旧例中的 26496。

### alloc.index_order

原生固定宽 INDEX 按两字 **有符号词典序**比较；正常 native refno 两字非负，排除 start marker 后，与现有 `RefNo::Ord` 一致。本规则实现命名 `NonnegativeLexicographic`，不把变长表的无符号分支误认为 INDEX 比较。

- `sub_5AFDF10` @ `0x5afdf10` n 240 调固定宽路由 `sub_5B00F10`，n 463 调叶内定位 `sub_5B010F0`。
- `sub_5B00F10` @ `0x5b00f10` n 59–73 逐字比较固定宽键；指令 @ `0x5b00f88` 是 `cmp edx, esi`，随后 @ `0x5b00f8a` / `0x5b00f8c` 为 `jl` / `jg`，确认有符号。
- `sub_5B010F0` @ `0x5b010f0` n 82–96 同样按逐字有符号比较；n 147–155 是另一条变长键分支，不能混用。
- 下表测试直接检查 walk 原始次序的每个相邻键严格递增、两字非负；没有先排序。前驱与独立反向扫描比较，冷缓存最多 4 页，未读取记录页。最新会话全量检查，历史选择 oldest/middle；链中间可能仅保留会话 2，如实记录。

| Corpus | 保留会话 / 键数 | 最多前驱页读 |
|---|---|---:|
| AMS8000 | 1 / 0；323 / 6637；646 / 6639 | 3 |
| AMS7999 | 1 / 0；100 / 34707；200 / 34709 | 3 |
| AMS7351 | 1 / 0；2 / 2；116 / 3345855 | 4 |
| AMS251142 | 1 / 0；2 / 2；10 / 948 | 2 |
| TES023 | 1 / 0；101 / 320；109 / 320 | 2 |

15 个会话原始次序反例 0；前驱对照反例 0。最大 u32 查询边界另由合成树测试覆盖，它是读侧边界处理测试，不是原生允许负序号的证据。

## 复现命令

```powershell
tools/ida/alloc_evidence.py
pwsh -NoProfile -File tools/e3d/allocation_probe.ps1 -Scratch D:/e3dio-alloc-probe-final -Login SYSTEM/XXXXXX
cargo test --test index_cursor_real --test next_word1_cli --test bucket_table_real --test core_rules_registry -- --nocapture
pwsh -NoProfile -File tools/ida/check_evidence.ps1 docs/evidence/2026-10-05-allocation-counter-index.md
```

诊断 CLI `next-word1 --db FILE --extent WORD0 --session N` 在只读路径中先冷 seek 后独立全索引 scan，比较两个最大值；页 1 是打开时物理页 1，不伪装成历史会话的页 1。报告当前计数器语义、回退原因、Bucket_Floor 和期望号；不引用写侧 Allocator。

## 反例

- 四库 counters > live max 本身不能区分“已发高水位”与“下一个待发号”；因此补了 NEW 返回与缓存 +1、同 TTY 的登录后复制。
- 单引号 ALPHA LOG 场景未进入查询，首次两轮结果为 Indeterminate；修成既有宏使用的双引号后 SYSTEM 场景完成，失败不计 PASS。
- 原生 e3dio 登录未通过，独立目录保留失败报告；原生登录使用既有 SYSTEM 配置，用户差异不隐藏。
- 未坐实 `bucket.collision` / `bucket.persist`，两者继续 Pending；本轮不以本探针替代第 13 组的页 1 持久化与交替撞号取证。
- 未执行 F3–F7；当前写侧仍 max+1，Q14、release open 计时与发号下界验收没有提前打勾。
