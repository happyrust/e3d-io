# E3D 3.1 二进制解析 Glossary

这份词表是本工作区的统一用语。所有课程、参考卡、学习记录都按这里的说法写。
凡是这里定过的词，别在别处换个说法。

## 文件与容器

**noun**：
E3D 的元素类型，例如 BRAN、ZONE、BOX。在二进制里永远以 db1 hash 出现，不以字符串出现。
_Avoid_: 元素类型、element type、class

**db1 hash**：
把大写名字按 27 进制从后往前折叠再加 `0x81BF1` 得到的 32 位整数。可逆，所以能反推回名字。
_Avoid_: 名字哈希、PDMS hash

**`*vir.dat`**：
随 E3D 安装包发布的静态 schema 文件，每种数据库类型一个（`desvir.dat` 对应 DESIGN）。
文件头第 2 个字是该库类型名的 db1 hash。
_Avoid_: 模板文件、schema 文件（口语可用，写文档时用文件名）

**noun descriptor（描述符）**：
`*vir.dat` 里描述"某个属性在某个 noun 的记录里怎么存"的一条变长记录，确认的前缀是十个 dword。
_Avoid_: template attr、属性定义、字段描述

**descriptor table（描述符表）**：
一个 noun 的全部描述符，按 payload 内部的 `stride` 串成链。`payload[9]` 是条数。
_Avoid_: 模板、noun template（代码里叫 `NounTemplate`，写文档时用中文"描述符表"）

## 记录布局

**implicit 区**：
元素记录里由描述符按固定 word offset 直接寻址的那段。定长的数值、引用、布尔住在这里。
_Avoid_: 主体区、固定区

**explicit 尾巴**：
implicit 区之后挂的 `flag=1` 块，按 `hash + type_code + 长度 + 数据` 反复排列。
变长文本和大数组住在这里。
_Avoid_: 扩展区、附加属性区

**word offset**：
描述符 `encoded_location` 的**低 20 位**，属性在 implicit 区里的起始字下标。
`0` 是有效值，含义是"不在 implicit 区"，不是"缺数据"。
_Avoid_: 偏移、地址、位置

**bool bit index**：
`encoded_location` 的**高位**（`raw >> 20`）。多个布尔共用一个 word 时靠它区分。
_Avoid_: 位号、flag 位

**compact / wide**：
同一条描述符里并列的两套存储方案。compact 用 f32，wide 用 f64，两套的 word offset 会
一路岔开。二者要么都有位置、要么都是 0，不存在只有一套的情况。
_Avoid_: 32 位/64 位布局、f32Form

**format word**：
元素 implicit 区的**第 10 个字**。第 29 位（`0x20000000`）为 1 表示这条记录用 wide 布局。
读任何属性之前必须先读它。
_Avoid_: 标志字、recFlags（live dump 里叫 recFlags，指同一个东西）

**stride**：
描述符自己在 payload 字节流里的长度，用来走到下一条描述符。
**它不是元素记录里的步长**——早期版本把两者混为一谈过，是个已知陷阱。
_Avoid_: 步长、间隔

**storage_words**：
这个属性在元素记录里占几个字。数组的话包含开头那个 count 字。
_Avoid_: 长度、size

## 值与寻址

**RefNo**：
两个 u32 组成的元素引用。`dbno = (word0 & 0x1FFF) | ((word0 >> 13) & 0x3E000)`。
_Avoid_: 引用号、element ref

**Fortran 字交换**：
f64 在盘上按 `(低 32 位, 高 32 位)` 两个大端字存放。读的时候要把两半对调再拼。
_Avoid_: 字节序问题、endian 坑

**word hash 文本**：
一个 u32 槽位里装的其实是短文本的 db1 hash，反推即可得到字符串（例如 `726491` → `BOXI`）。
_Avoid_: 编码文本、压缩字符串

## 元数据来源

**ATGTDF / ATGTIX / ATGTSX**：
`attlib.dat` 里的三张表。ATGTDF 给属性的语义类型与尺寸，ATGTIX 给属性元数据记录的位置
（人类可读名、报表名由此而来），ATGTSX 给 noun→属性的粗映射。
三者都**不给物理 offset**。

**逻辑目录**：
从运行中的 E3D 导出的 `noun_layout.json` + `noun_attr_fields.json`。给顺序、pseudo 标记、
默认值探针、isUda。同样**不给物理 offset**。
_Avoid_: catalog、noun map（"noun map"在本工作区特指最终要产出的合并产物）

**noun map**：
本工作区要产出的最终合并产物：以 `(db_type, noun_hash)` 为键，合并物理描述符、语义元数据、
逻辑目录、以及项目级 UDA 的一份 JSON。
_Avoid_: 元数据文件、schema dump

## 发布版本与转换

**发布版本**：
AVEVA 的一个 E3D 产品版本，如 E3D 2.1（安装目录叫 Everything3D2.10）、E3D 3.1。本工作区用 schema 包 id 指称：`e3d210`、`e3d31`。
_Avoid_: 版本号（单说分不清是发布版本、DAB 标签还是模板版本）、库版本

**schema 包**：
一个发布版本的 `*vir.dat`、`attlib.dat`，以及在它旁边导出的逻辑目录的合称。库文件不记自己属于哪个发布版本，读属性之前要由人选定 schema 包。
_Avoid_: 版本包、模板集

**DAB 标签**：
会话页里 8 字节的 Dabacon 引擎构建标签（如 `2_28_19_`、`2_35_23o`）。它说明哪个引擎写下了这个会话，不说明库属于哪个发布版本。
_Avoid_: 库版本、E3D 版本

**模板版本**：
库页 0 的 `TMPLVN`：E3D 打开库时拿它的绝对值与 `*vir.dat` 头字 3 比对的模板兼容号，不等就拒开。
_Avoid_: 模板号（那是 `TMPLNO`）

**源版本 / 目标版本**：
一次转换中，输入库按哪个发布版本解读；输出库要被哪个发布版本读对。
_Avoid_: 旧版 / 新版（3.1 → 2.1 时正好反过来）

**转换**：
从一个按源版本解读的库，得到一个目标版本能读出相同元素、相同属性值的库。
_Avoid_: 升级（AVEVA 的 `UPGDNO` 升级是库内的迁移号，另一回事）、迁移、导出

**转换路线**：
一个（源版本，目标版本）组合，加上它采用的转换方法。
_Avoid_: 转换方向、转换模式

**原样携带**：
输出字节与输入字节相同的转换。只有等价核验通过时它才成立。
_Avoid_: 直接复制、无损转换

**等价核验**：
对库里每个元素，分别按源版本和目标版本的 schema 包解码，再按属性 hash 逐行比对的检查。
_Avoid_: 校验、验证（「验证」留给 E3D 门）

**E3D 门**：
用真 E3D 会话打开库、逐问读回并判 PASS / FAIL 的验收。
_Avoid_: 测试、实测（口语可用）

**迁位属性**：
两个发布版本的描述符给了不同 implicit 区位置或不同存储类型的同一属性。存过值的元素在目标版本下会读到别的字。
_Avoid_: 变更属性、错位

**扩容属性**：
只存在 explicit 尾巴里、两版存储类型相同、只改了容量的属性，如 `STEX` 从 480 字节到 1 024 字节。
_Avoid_: 变更属性

**默认值变化**：
元素没存某属性的值，而两个发布版本给出的默认值不同。变的是显示出来的值，不是存下的数据。
_Avoid_: 默认值错误、不一致

**转换计划**：
动手之前对一个库、一条转换路线做的只读判断：哪些检查通过、哪些阻断、会有哪些迁位属性出场。
_Avoid_: 预检报告、dry run（本仓库的 dry run 专指 e3d-write 的 stage 后 rollback）

**核验报告**：
等价核验对一个库给出的逐项结论与计数，随输出一起保存。
_Avoid_: 日志、对比结果

## 发号与桶表

**Bucket_Table（桶表）**：
按扩展保存 RefNo 发号计数器的元数据表。
_Avoid_: 发号索引、元素索引

**Bucket_Counter（桶计数器）**：
Bucket_Table 中某个扩展的发号计数值。其含义由该发布版本的 Counter_Semantics 确定。
_Avoid_: 活最大、已发最大

**Counter_Semantics（计数器语义）**：
Bucket_Counter 表示“下一个待发值”还是“已发高水位”的约定。
_Avoid_: 计数方式

**Bucket_Floor（桶发号下界）**：
按 Counter_Semantics 从某个扩展的 Bucket_Counter 得出的最小可发 word1。
_Avoid_: 活最大加一、桶最大值
