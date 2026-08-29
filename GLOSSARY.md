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
