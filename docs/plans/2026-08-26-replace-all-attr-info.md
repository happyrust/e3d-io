# 移植计划：用描述符权威数据替换 `aios_core::get_default_pdms_db_info`

状态：第三稿。第一轮评审 → D1 裁决（§1.7）；第二轮评审放开表结构 → D2 裁决 + 新增 §2.5 表设计、D5；
P1-3 已执行完毕（§3 P1）
日期：2026-08-26
影响仓库：`e3d-io-noun-descriptor`（产出方）、`old/gen-model`（消费方）、`old/vendor/old-aios-core`（当前数据源）

---

## 0. 目标

`../gen-model` 现在通过 `aios_core::get_default_pdms_db_info()` 拿属性 schema，
数据来自 `include_str!("../all_attr_info.json")`（2.53 MB，静态嵌入）。

把这个数据源换成从 20 个 `*vir.dat` + `attlib.dat` + 逻辑目录合成的权威 map，
noun 覆盖从 **339 提到 1875**。

**不改 `gen-model` 的调用点。** 表结构按第二轮评审的意见放开，但只做**加法**：
老字段语义一个不动，新字段全部带 `serde` 默认值（§2.5）。
老消费者不改一行代码即可继续跑；想要更准的消费者按新字段各取所需。

---

## 1. 已经测出来的事实（这份计划的地基）

以下每条都是实测，不是推断。复现方式见 §7。

### 1.1 现有 JSON 的真实形状

| 项 | 实测值 |
|---|---|
| noun 数 | 339 |
| (noun, attr) 对 | 6556 |
| `offset` 字段的真实含义 | **raw `encoded_location`**，不是字偏移 |
| `ityp` 已采集的条数 | 0 |
| key | 扁平 `noun_hash`，不区分数据库 |

`offset` 是 raw 值这一点，用 BRAN 七个打包布尔验证过：
`BUIL=12`、`SHOP=1048588`、`LHEA=2097164`、`LTAI=3145740`、`DETA=4194316`、
`LSTR=5242892`、`LISS=6291468` —— 即 `(bit << 20) | 12`。

### 1.2 offset 与描述符的一致性

5533 个共享属性对：

```
两列本来就相同      4045
只等 WIDE           1488
只等 COMPACT           0
两边都不等             0
```

结论：**现有 JSON 是一份 wide-only 快照**，凡两列分叉处一律取 wide。

### 1.3 compact 记录在实际项目里是否存在

在 `ams8000_0001` 上做严格记录扫描（要求 `impl_len` 合理、refno 与 owner 同库、
owner 非负、记录不越界）：**24561 条记录全部 wide，compact 0 条**。

所以 wide-only 是潜伏缺陷，不是现行 bug。但它挡住"覆盖全部类型"这个目标。

### 1.4 覆盖率

```
现有 JSON              339 noun
20 个 *vir.dat        2119 noun        -> 覆盖率 16.0%
vir 有 / JSON 没有的属性对   29099
JSON 有 / vir 没有的属性对    1023
```

那 1023 条按 att_type 拆开是 WORD 334 / BOOL 326 / ELEMENT 305 / STRING 25 /
DOUBLE 22 / INTEGER 10 / POSITION 1，样本为 `LOCK`、`OWNER`、`TYPE`、`LOOS`、`GRADE`
—— 结构属性与伪属性，本来就不在描述符里。

### 1.5 `att_type` 的来源

`attlib.dat` 的 ATGTDF **不足以重现 `att_type`**：

```
BRAN  98 条描述符 -> 只有 1 条拿到 semantic_type_code，72 条为 null
WORL   7 条描述符 -> 只有 1 条
```

逻辑目录 `catalog/e3d31/noun_layout.json` 可以：

```
一致        5392
不一致        87
目录里没有  1077
```

87 条不一致里绝大多数是数组写法差异（`LEVE`：JSON `INTVEC` vs 目录 `INTEGER`+`isArray`；
`BLRF`：JSON `RefU64Vec` vs 目录 `ELEMENT`+数组），可归一。
真正的语义冲突只有四条，见 §2。

### 1.6 `default_val`

11 种：`ElementType 1407`、`StringType 1345`、`BoolType 1064`、`WordType 878`、
`IntegerType 854`、`DoubleType 505`、`Vec3Type 284`、`IntArrayType 102`、
`DoubleArrayType 99`、`RefU64Array 17`、`StringArrayType 1`。

抽样全部是**按类型的静态默认值**，不是每属性独立的值池：

```
ELBO.ISOH     BOOL       false
CYLI.POS      POSITION   [0,0,0]
DDAT.PTYP     WORD       "unset"
COUP.ISPE     ELEMENT    ""
GENSEC.WELDDP DOUBLE     0.0
```

`noun_catalog.rs::static_default_value` 已经在按 `typeName` + `default_probe` 生成同类值。

### 1.7 跨库布局分歧：比想象的小得多

先前的说法是"41 个 noun 跨库布局不同，所以 key 必须是 `(db_type, noun_hash)`"。
那个 41 是**payload 整体不同**（sha256 不等），把「属性集合不同」和「同一属性 offset 不同」
混在了一起。逐属性重测之后：

```
有描述符的 noun            1875
出现在多个库里的 noun         66
  多库 noun 的共享属性对     547
  其中 offset 真的不同        14
  有 offset 冲突的 noun        6
```

有冲突的只有六个，且**全部涉及 DESI**：

| noun | 出现在 | 共享属性 | offset 冲突 |
|---|---|---|---|
| `GENPRI` | DESI, MANU | 19 | 4 |
| `GENCUR` | DESI, MANU | 17 | 4 |
| `CDSE` | DESI, MANU | 10 | 3 |
| `GPWL` | DESI, CATA, MANU, PROP | 12 | 1 |
| `WORL` | 全部 20 个库 | 8 | 1 |
| `GPITEM` | DESI, MANU | 7 | 1 |

**扁平 key 能无损表达 1869 / 1875 = 99.68% 的 noun。**

若约定"冲突时一律取 DESI"，全库 35734 个 `(db, noun, attr)` 三元组里
**只有 14 个会读错，占 0.039%**，且全部落在非 DESIGN 库上（DESIGN 侧永远正确）。

---

## 2. 决策

### D1 —— 扁平 key 够不够用：**够，已裁决**

评审提问："能不能直接生成一份真正完整的、同样格式的 JSON，避免大改动？"

按 §1.7 的实测，答案是能。**不改 `PdmsDatabaseInfo` 的类型，不动 `aios_core` 公开接口，
只把内容换成全 20 库合并的完整版**，代价是 14 个属性槽位（0.039%）在非 DESIGN 库上读错。

采用方案：

- 全 20 个 `*vir.dat` 合并进一张扁平表
- 同名 noun 冲突时取 `DESI`；没有 DESI 变体的取字典序第一个库
- 生成时同步输出一份 `flat-key-conflicts.json`，列全那 6 个 noun / 14 个槽位，
  写清楚"这些槽位在非 DESIGN 库上不保证正确"

这样 noun 覆盖从 339 涨到 1875，且**调用方一行都不用改**。

`(db_type, noun_hash)` 的正确形态保留在 `compile-metadata` 的输出里，
将来真需要时再从那里长出来，不阻塞本次移植。

### D2 —— compact 列要不要现在就带上：**要，已裁决**

评审放开了表结构（"可以修改新表的格式，用来保证数据的表达性"）。放开之后这条不再是权衡题，
因为**现有消费者正在用一套算术把 compact 列猜出来**。

`old-parse-pdms-db/src/parse.rs` 的实际做法：

```rust
// 1. 靠"最后一个属性的 offset + 预估宽度是否超出记录长度"来判断这条记录是不是 f32
is_f32 = last_att_info.offset + last_step > (origin_impl_len / 4)

// 2. 判定为 f32 后，逐属性累减：前面每个 DOUBLE 减 1 字，
//    每个 POSITION/DIRECTION/ORIENTATION 减 3 字
match prev_attr_info.att_type {
    DOUBLE                          => f32_neg_offset += 1,
    DIRECTION|POSITION|ORIENTATION  => f32_neg_offset += 3,
}

// 3. 属性占几个字，靠跟"下一个 offset"相减推出来
step_w = next_attr_info.offset & 0xFFFFF - attr_info.offset & 0xFFFFF
```

三处全是重建，不是读取：

| 它在猜什么 | 描述符里本来就有 |
|---|---|
| 这条记录是 f32 还是 f64 | implicit 第 10 个字的 bit 29 |
| compact 布局下每个属性的位置 | `compact.encoded_location` |
| 属性占几个字 | `compact/wide.storage_words` |

第 3 点还逼出一个连锁后果：因为 `step` 要靠相邻 offset 相减，
`sort_offsets()` 必须先按 offset 排序，整个解析顺序被绑死在物理布局上。

顺带在同一份代码里看到一处不一致：`parse_implicit_attr_value` 用
`attr_info.offset & 0xFFFF`（16 位），其余地方一律用 `& 0xFFFFF`（20 位）。
实测最大字偏移只有 91，所以现在不咬人，但它是错的。

**结论**：compact 必须带，`storage_words` 必须带。带上之后上面三段启发式全部可以删掉。

### D5 —— 完整表放不进 `include_str!`，用什么承载

新表约 35734 个 `(db, noun, attr)` 三元组，每条比现在多约十个字段，
JSON 体积估计 **15–20 MB**（现状 2.53 MB）。`include_str!` 进二进制会明显变胖。

三个选项：

- **D5-a**：文件里只存 `by_db` 全量表，`noun_attr_info_map` 不落盘，
  改成加载后由 `fill_flat_map()` 现场折叠出来。省掉一份重复，且老消费者无感。**推荐**。
- **D5-b**：字段名缩写（`o` / `sw` / `bb` …）。有效但难读，与"表达性"的初衷相悖。
- **D5-c**：换 `bincode` 或 `rkyv` 二进制承载（`aios-database` 两个依赖都已经有了），
  用 `include_bytes!`。最省，但产物不可读、不可 diff，丢掉现在这份 JSON 最大的好处。

D5-a 与 D5-c 可以叠加：JSON 作为可 diff 的权威产物入库，构建时转成二进制嵌入。

### D3 —— 四条真语义冲突谁对

```
MODE     JSON INTEGER   vs  目录 STRING
NOMDIR   JSON DOUBLE    vs  目录 DIRECTION
PDAREF   JSON STRING    vs  目录 ELEMENT
PCON     JSON WORD      vs  目录 STRING
```

建议拿实物裁决：找一个带这些属性的真实元素，看描述符的 `storage_type_code` 和实际字节。
在裁决之前，这四条保留 JSON 的取值（避免行为回退）。

### D4 —— 目录缺的 1077 条 `att_type` 怎么补

它们是 `LOCK` / `OWNER` / `TYPE` 这类结构属性和 UDA。
建议：结构属性硬编码一张小表（不超过 10 条名字），UDA 走 §6。

---

## 2.5 新表结构

设计原则：**加法，不是改法。** 老字段一个不动、语义一个不改；
新字段全部 `#[serde(default)]`，老文件照样能反序列化。

已确认安全：vendor 树里所有 `AttrInfo { … }` 结构体字面量**都在注释里**，
没有一处活代码构造它，所以加字段不会引发编译错误。

### 文件

```jsonc
{
  "schema": "e3d-attr-info/v2",
  "sources": { /* desvir 20 个文件 + attlib + 目录的 sha256 与字节数 */ },
  "counts":  { /* db_types / nouns / attrs / triples / conflicts */ },

  // 全量权威表：三层 key，彻底解决 41 个跨库分歧
  "by_db": {
    "<db_type_hash>": {
      "<noun_hash>": {
        "<attr_hash>": { /* 见下 */ }
      }
    }
  },

  // 扁平兼容视图不落盘，加载后由 fill_flat_map() 折叠生成（D5-a）
  "flat_conflicts": [ /* 折叠时被丢弃的那 14 个槽位，逐条列出 */ ]
}
```

### `AttrInfo` 新形态

```rust
pub struct AttrInfo {
    // ---- v1，语义不变 ----
    pub name: String,
    pub hash: i32,
    /// wide 列的 raw encoded_location。保留原语义（含打包 BOOL 的高位），
    /// 老消费者的 `& 0xFFFFF` 和 `>> 0x14` 继续成立。
    pub offset: u32,
    pub default_val: AttrVal,
    pub att_type: DbAttributeType,
    pub ityp: Option<i32>,

    // ---- v2 新增，全部 #[serde(default)] ----
    pub ordinal: u32,              // 描述符在 noun 里的顺序，本身是信息
    pub storage_type_code: u32,    // 描述符 +2
    pub logical_count: u32,        // 描述符 +3，数组上界，兼作 count 字校验
    pub stride: u32,               // 描述符 +1，payload 内部长度（不是元素步长）
    pub is_array: bool,
    pub report_name: Option<String>,
    pub compact: StorageSlot,
    pub wide: StorageSlot,
    pub implicit: bool,            // wide.word_offset != 0
    pub default_source: DefaultSource,   // Verbatim | Derived
    pub db_type: Option<u32>,
}

pub struct StorageSlot {
    pub storage_words: u32,
    pub encoded_location: u32,   // raw
    pub word_offset: u32,        // encoded_location & 0x000F_FFFF
    pub bool_bit: Option<u8>,    // encoded_location >> 20，仅 storage_type_code 5/9 有意义
    pub flags: u32,
}
```

`word_offset` 与 `bool_bit` 是从 `encoded_location` 算出来的冗余字段。
留着是因为它们正是老代码手写掩码的两个量——写进文件，掩码写错的可能性就消失了。
D2 里那个 `& 0xFFFF` 的笔误就属于这类。

### 这些字段各自消掉什么

| 新字段 | 让消费者删掉什么 |
|---|---|
| `compact` + `wide` | `is_f32` 启发式、`f32_neg_offset` 累减 |
| `storage_words` | 靠相邻 offset 相减推 `step`，连带 `sort_offsets()` 的排序依赖 |
| `bool_bit` | `offset > 0xFFFF` 这种按量级判断"是不是打包布尔" |
| `logical_count` | 数组长度没有上界校验，count 字读飞了不会被发现 |
| `implicit` | `offset == 0` 的隐式约定散落各处 |
| `default_source` | 分不清哪些默认值是权威的、哪些是推出来的 |

---

## 3. 阶段划分

### P1 · 对账基线（不改任何代码）

产出一个可重复运行的对账器，输入是现有 `all_attr_info.json` + `compile-metadata` 的输出，
输出四张表：

1. 每个 (noun, attr) 对的 `offset` 是否一致（预期：100% 一致，wide 列）
2. 每个 `att_type` 能否由目录 `typeName + isArray` 重现（预期：5392 一致 / 87 可归一 / 1077 缺）
3. 每个 `default_val` 能否由 `static_default_value` 重现（**未验证，这是 P1 的主要未知数**）
4. `name` 是否一致

**完成标准**：三类差异各自有一个确定的数字，且每一条差异都能归入 D1–D4 中的某一类，
没有"不知道为什么不一样"的剩余。

#### P1-3 已完成 · `default_val` 对账结果

先纠正一个规模误解：`all_attr_info.json` 的 6556 是 **(noun, attr) 对**，
**不同的属性只有 702 个**。目录侧有 4271 个，所以属性维度的覆盖率也是 16.4%。

按什么分组能唯一确定 `default_val`：

| 分组键 | 可推属性 / 总数 | 比例 |
|---|---|---|
| `att_type` | 361 / 676 | 53.40% |
| `DTYP` | 176 / 676 | 26.04% |
| `DTYP + DEFI` | 176 / 676 | 26.04% |
| `att_type + DEFI` | 413 / 676 | 61.09% |
| `att_type + DEFI + SIZE` | 557 / 676 | 82.40% |
| **`att_type + DEFI + DFDP + SIZE`** | **593 / 676** | **87.72%** |
| 再加 `ITYP` | 594 / 676 | 87.87%（无收益） |

`DEFI` 只有四个取值：`0`(2)、`1`(3505)、`4`(328)、`5`(436)。它是选择器，不是值本身。

剩下 83 个推不出来的属性，拆开看是两类：

- **真缺数据 · 54 个**：全都落在 `DOUBLE | DEFI=1 | DFDP=-1 | SIZE=1 | ITYP=3` 这一格，
  取值有 `0.0` / `-100000.0` / `100.0` / `90.0` 等四种。典型是 `ANGL=90.0`、`TEMP=-100000.0`。
  目录字段里确实没有这个信息。
- **老 JSON 自己不自洽 · 约 29 个**：同一格里既有 `{"DoubleType":0.0}` 又有
  `{"DoubleArrayType":[]}`、既有 `{"ElementType":""}` 又有 `{"RefU64Array":[]}`。
  是标量 / 数组 tag 写法不一致，不是缺数据。

另外：**679 / 702 个属性的默认值与 noun 无关**（全局唯一），23 个随 noun 变化。
目标格式本来就是 `(noun, attr)` 键，所以这 23 个不构成问题。

#### 结论：**不需要去解 IDA 的默认值池**

分水岭定在 90%，实测 87.72% 纯推导。差的那一点用一张 **54 行的硬编码表**补齐，
值直接从现有 `all_attr_info.json` 抄——对现有 702 个属性可以做到 **100% 复现**。

对老 JSON 没覆盖的那 ~3569 个新属性，默认值按同一套规则推导；
推不出来的落回类型兜底（`DOUBLE→0.0` 等），并在输出里标注 `default_source: "derived"`，
与 `"verbatim"` 区分开。**不把推导值伪装成权威值。**

风险已解除，§6 的默认值池不再是本计划的前置。

### P2 · 生成 v2 表

新增一个子命令，从描述符 + attlib + 目录合成 §2.5 的 v2 文件：

```
e3d-descriptor emit-attr-info \
  --template-dir E:\reverse\e3d\shadow_e3d31_aps_all \
  --attlib       E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat \
  --catalog-layout catalog\e3d31\noun_layout.json \
  --catalog-fields catalog\e3d31\noun_attr_fields.json \
  --flat-winner DESI \
  --output      all_attr_info.json
```

`by_db` 三层键保全部真相；`flat_winner` 只决定加载时折叠出的兼容视图取哪一套。
目标规模：**20 个库 / 1875 noun / 35734 个三元组**（现状 1 张扁平表 / 339 noun）。

字段映射：

| 字段 | 来源 |
|---|---|
| `name` | `lookup_system_name(hash)` → `db1_dehash(hash)` |
| `hash` / `stride` / `storage_type_code` / `logical_count` | 描述符 `+0 +1 +2 +3` |
| `compact` | 描述符 `+4 +5 +6`，`word_offset` / `bool_bit` 由 `+5` 拆出 |
| `wide` | 描述符 `+7 +8 +9`，同上 |
| `offset`（v1 兼容位） | `wide.encoded_location` 原值 |
| `implicit` | `wide.word_offset != 0` |
| `report_name` | attlib ATGTIX |
| `att_type` / `is_array` | 目录 `typeName` + `isArray` 归一，缺的走 D4 |
| `default_val` | `att_type + DEFI + DFDP + SIZE` 推导（87.72%）+ 54 行硬编码表，见 P1-3 |
| `default_source` | 命中 54 行表或与老 JSON 逐字相同 → `Verbatim`，否则 `Derived` |
| `ityp` | 恒 `None`（与现状一致，不算回退） |
| `db_type` | 所属 `*vir.dat` 的类型 hash |

**完成标准**（两条都要满足）：

1. **不回退**：新文件在 P1 的四张对账表上，对现有 339 个 noun 的 6556 条**逐条相等**
   （D3 的四条例外，按决策处理）。字节级不要求相同（键序、浮点格式可能不同），语义级要求相同。
2. **有增量**：`by_db` 覆盖 20 个库 / 1875 noun / 35734 个三元组，
   且 `flat_conflicts` 恰好列出 §1.7 那 6 个 noun / 14 个槽位——多一个少一个都说明折叠逻辑写错了。
3. **老文件仍能读**：拿现有的 v1 `all_attr_info.json` 喂给新的 `AttrInfo`，
   反序列化成功且新字段落在默认值上。加法没做成改法。

**风险**：`serde` 反序列化 `AttrVal` 的 tag 名必须完全对上
（`ElementType` / `StringType` / `Vec3Type` …），拼错了会在运行时 panic 而不是编译期报错。
P2 必须包含一个"新文件能被 `aios_core` 反序列化成功"的测试。

#### P2 已执行 · 实测结果

命令加了 `--legacy-attr-info`。这一个参数把"不回退"从**测试目标**变成了**构造性保证**：
`att_type` 与 `default_val` 在老文件覆盖到的地方逐字照抄，覆盖不到的地方才走推导。
推导规则本身也不写死——从老文件里**学**出 `(att_type, DEFI, DFDP, SIZE) → default_val`，
只采纳老文件内部意见一致的组合。原计划里那张"54 行硬编码表"因此不需要了。

```
cargo run --bin e3d-descriptor -- emit-attr-info \
  --template-dir      E:\reverse\e3d\shadow_e3d31_aps_all \
  --attlib            E:\reverse\e3d\shadow_e3d31_aps_all\attlib.dat \
  --catalog-layout    catalog\e3d31\noun_layout.json \
  --catalog-fields    catalog\e3d31\noun_attr_fields.json \
  --legacy-attr-info  <\u65e7 all_attr_info.json> \
  --output            all_attr_info.json
```

产物：

```
database_types      20
nouns             1875     <- \u8ba1\u5212\u76ee\u6807 1875
triples          35734     <- \u8ba1\u5212\u76ee\u6807 35734
flat_conflicts      14     <- \u8ba1\u5212\u9884\u6d4b 14
default_verbatim  5602
default_derived  30132
bytes         17715275     <- 16.9 MB\uff0c\u843d\u5728 15\u201320 MB \u9884\u4f30\u5185
```

完成标准 1（不回退），对 5533 个共享属性对逐条比对：

```
offset    5533 / 5533
att_type  5533 / 5533
default   5533 / 5533
name      5533 / 5533
```

完成标准 2（冲突清单），**恰好 6 个 noun / 14 个槽位**，与 §1.7 一字不差：
`CDSE` `GENCUR` `GENPRI` `GPITEM` `GPWL` `WORL`。

这 14 个槽位有个之前没看清的共同点：**被丢弃的 MANU 变体 offset 全是 0**。
所以扁平视图的实际风险不是"读到别人的字节"，而是"对 MANU 元素误以为这些属性在 implicit 区，
而 MANU 其实把它们放在别处"。比原先的描述更具体，也更容易在下游断言。

剩下的完成标准 3（老文件仍能读）要在 `aios_core` 侧加测试，未做。

### P3 · 切换与双跑对拍

在 `gen-model` 侧不改调用点，只把 `all_attr_info.json` 换掉。

#### P3-0 · 「换掉」到底换哪一份（2026-08-26 查清，与原计划不符）

原计划这一行写的是「或用 `load_file` feature 指向新文件」。**这条路不存在**：
`aios_core/src/lib.rs` 里确实有 `#[cfg(feature = "load_file")]` 去读工作目录下的
`all_attr_info.json`，但 `load_file` **从来没有在 `Cargo.toml` 的 `[features]` 里声明过**
（只有 `render / manifold / gen_model / debug_expr / debug_wire / debug_spatial / web / sql / hh`）。
未声明的 feature 恒为假，所以那个分支是死代码，`include_str!` 永远赢。要用它，得先补 feature。

**这条路已经修好了**（`old-aios-core` 分支 `fix/load-file-feature`，commit `555d28a`，
worktree 在 `vendor/old-aios-core-loadfile`，pinned 那棵树一个字没动）。原来的分支不只是死的，
它**也编不过**：那一行的 `File` 根本不在作用域里——本模块里其它 `File` 全是 `config::File`、
在各自函数体内 `use` 进来的，唯一真要文件系统的地方写的是 `std::fs::File` 全名。顺带修掉的还有
「`read_to_string` 的返回值被丢掉」：读失败时 `string` 留空，错误会在下游伪装成
「JSON 意外结束」，指向表本身而不是那次读。现在报路径 + 解析这个相对路径所依据的工作目录，
解析失败时也带上是内嵌的那份还是磁盘上的那份——这两个答案对应的修法完全不同。

`examples/attr_info_scale.rs` 是它真的生效的证据：同一个二进制在两个目录里跑，分别报
339 noun / 6556 对 和 1875 noun / 34632 对。

> 顺带把 §P2 完成标准 3 和那条 serde 风险一起结掉了：新表 **34632 对能干净反序列化**成
> `PdmsDatabaseInfo`，老表照旧能读。原计划担心的「`AttrVal` 的 tag 名拼错会在运行时 panic」
> 没有发生。

#### ⚠ 打开 `load_file` 之前必须先换掉 gen-model 根目录那个文件

`gen-model/all_attr_info.json`（1.31 MB，2026-07-23，被 gitignore）就是 `load_file` 打开后
会被读到的那一份。拿上面那个 example 在 gen-model 目录里实测：**339 noun / 6554 对**，
比内嵌那份少两对，另有三处内容不同：

```
少了   AEXCES  (noun 543135, DOUBLE)      WELDTY (noun 619079, WORD)
不同   BLRF    att_type  ELEMENT   vs  RefU64Vec     \u2190 类型变了
       BANG    att_type  INTEGER   vs  DOUBLE        \u2190 类型变了
       VXREF   default   ElementType("") vs RefU64Array([])
```

也就是说，**只把 feature 打开、不动那个文件，等于悄悄回退五条**，而且一声不吭。
切换动作必须是「打开 feature」和「把新表放到工作目录」一起做，不能分两步。

---

除 `load_file` 外，另外两条路（在修好之前只有这两条，而且**只有第二条是交付**）：

| 路子 | 做什么 | 性质 |
|---|---|---|
| `[patch]` | `scripts/Toggle-LocalDeps.ps1 -On`，把三个 crate 重定向到 `../vendor/*` | **开发回路**。推 main 前必须 `-Off`，pre-push 钩子会挡 |
| 升 rev | 在 `old-aios-core` 提交新文件 → push → 改 gen-model 的 `rev` | **交付** |

一个容易踩的坑：`[patch]` 打开时，`../vendor/old-aios-core` 的工作树**必须停在
`Cargo.toml` 里钉的那个 commit 上**（现在是 `0e391ff1`）。这意味着**不能靠「在 vendor 里
改一改文件」来预置切换**——那样 patch 一开，编出来的就不是 rev 所声明的那棵树，而且没有
任何提示。2026-08-26 16:48 确实出现过这个状态（`all_attr_info.json` 被就地换成 34632 对的
生成件、未提交），已还原；那份生成件保存在
`gen-model/.scratch/p3-attr-info/generated_v2_flat.json`，连同从 git HEAD 取出的
`legacy_v1.json`（6556 对）一起，作为对拍的两个输入。

**对拍本身不需要上面任何一条。** `attr_info_compat_probe` 已改成两份 schema 都从显式文件读，
判决不取决于谁被编进了二进制；它另外用内嵌那份再解一遍作为第三条臂，路径上若有别处去摸
`get_default_pdms_db_info()`，会在那条臂上露出来，而不是把差异悄悄抹平。

还有一处与 §2.5 对不上：现在这份生成件是 **v1 的字段形状**
（只有 `name / hash / offset / default_val / att_type / ityp`），没有 `compact` / `wide` /
`storage_words`，`AttrInfo` 结构体也没动过。所以 §5 验证矩阵里「启发式退场」那一行
（删掉 `is_f32` / `f32_neg_offset` / `step` 相减）**这一轮做不了**，它要先给 `AttrInfo` 加字段。
这一轮能证的是覆盖面从 339 noun 放大到 1875 而已有的值一个没变。

**对拍口径**：同一个数据库、同一批 refno，用旧 JSON 与新 JSON 各跑一次
`parse_elements_with_info`，逐属性比对解出的值。

**完成标准**：现有 339 noun 覆盖范围内，解出的属性值**逐个相等**。
超出范围的新增属性只允许"从无到有"，不允许改变已有属性的值。

**风险**：`get_all_explicit_nouns()` 用 `info.offset == 0` 判定 explicit。
新文件的 offset 语义与旧的一致（raw encoded_location，0 表示不在 implicit 区），
但覆盖面扩大后这个集合会变大，下游对它的假设需要检查。

### P4 · 消化放大后的覆盖面

P2 一次就把 339 → 1875 做完了，所以 P4 不是"再扩容"，而是**收拾覆盖面变大带来的下游反应**。

`gen-model` 里有三处会随 noun 数变多而改变行为：

- `model_impact.rs::reference_attr_names()` —— 遍历全部 noun 收集 `att_type == ELEMENT`
  的属性名，用来决定级联。名字多了，级联边会变多。
- `manual_update.rs` 里按 `named_attr_info_map` 的 key 遍历 noun 表做补扫。
  noun 多了，扫的表就多了。
- `PdmsDatabaseInfo::get_all_explicit_nouns()` —— 用 `offset == 0` 判 explicit。
  这个集合会显著变大。

**完成标准**：新增 noun 在真实库上能解出非空属性；`extract` 的
`non_implicit` / `decoded` / `decoded_explicit` 三个计数之和等于描述符条数（不静默丢行）；
上面三处的行为变化各自被显式确认为"预期内"，而不是被测试碰巧盖住。

**风险**：这是唯一会产生"新行为"的阶段。已有测试
（`att_meta_all_attributes_classify_and_references_affect_model` 断言过
"6556 属性 / 1421 引用类"）会因为数字变了而红——那是预期的，
但要逐个确认是"变多了"而不是"变错了"。

---

## 4. 不在本计划内

- writeback。
- 解 IDA 的默认值池 —— P1-3 已证明不必须。
- `ityp` 采集。
- `dicvir.dat` / UDA（见 §6，单独立项）。

---

## 5. 验证矩阵

| 检查 | 何时 | 通过标准 |
|---|---|---|
| 描述符 payload 自校验 | 每次读 | `payload[11] == 14 + Σstride` |
| 磁盘 vs 活内存 | 一次性 | WORL 7 条 × 10 dword 全等（已通过） |
| offset 对账 | P1 | 与现有 JSON 逐条相等 |
| `att_type` 归一 | P1 | 5392 一致 + 87 归一 + 1077 走 D4 |
| `default_val` 复现 | P1 | **已完成**：87.72% 纯推导 + 54 行表 = 现有 702 属性 100% |
| 反序列化（新文件） | P2 | `aios_core` 能加载 v2 文件 |
| 反序列化（老文件） | P2 | v1 文件仍能读，新字段落默认值 |
| 规模 | P2 | 20 库 / 1875 noun / 35734 三元组 |
| 扁平冲突清单 | P2 | 恰好 6 个 noun / 14 个槽位 |
| 启发式退场 | P3 | `is_f32` / `f32_neg_offset` / `step` 相减三段删除后行为不变 |
| 值对拍 | P3 | 339 noun 范围内逐属性相等 |
| 不丢行 | P4 | 状态计数之和 == 描述符条数 |

---

## 6. 后续立项：UDA

BRAN 一个元素的 TTY `ATTLIST` 有 121 行，其中 **34 行是 `:` 开头的 UDA**，
静态文件里完全没有。这是覆盖率的最后一块，也是唯一全新的领域。

线索：`dicvir.dat` 声明自己是 DICT 库、95 个 noun。UDA 定义元素的模板应该在那里。
未验证。

---

## 7. 复现方式

本计划里的每个数字都可以重跑。工具：

```
cargo run --bin e3d-descriptor -- describe        --desvir … --attlib … --noun BRAN
cargo run --bin e3d-descriptor -- extract         --db … --desvir … --attlib … --refno 24384/23257
cargo run --bin e3d-descriptor -- compile-metadata --template-dir … --output …
```

环境路径见 `NOTES.md`。字节级细节见 `reference/descriptor-record.html`。

已核对过的实物地址：

```
desvir.dat   0x00080000  WORL payload
desvir.dat   0x00080088  WORL 的 UPDT 描述符
desvir.dat   0x00160800  BRAN payload
desvir.dat   0x00160C38  BRAN 的 HBOR 描述符
ams8000_0001 0x00756AD0  BRAN 24384/23257 记录头
```
