# PROP 数据库编辑与使用 PRD

这份文档只写特性库 PROP 怎么被 core.dll 登记、怎么被用、编辑与引擎侧写回能确定到哪一步。界面触发写在第 3.4 节，证据是 Everything3D 2.10 的 PML，不是 3.1 的窗体。

证据分两截。本轮用 ida-bridge 客户端 `idalib-29552` 只读 `D:\ida_scratch\plant3\core.dll.i64`，没有保存。库文件对应 `E:\reverse\e3d\shadow_e3d31_aps_all\core.dll`，50,071,544 字节。上一会话对 `acp7008_0001` 会话 47 的层次、槽位和 e3d-io 读数，本轮没有重跑，下面单独标明。

## 1. 库类型登记

`provir`（0x5d64a1c）、`CMPW`（0x5d64a24）、`Property`（0x5d64a34）、`Property database`（0x5d64a40）挨在同一段字符串里。`provir`、`Property database`、`CMPW` 的引用都进 `DB_DatabaseTypes::LoadDatabaseTypes_Generated`（0x5a2a4d0）。这个函数只被 `DB_DatabaseTypes` 构造函数（0x5a29d30）调用。

构造函数把参数写进 204 字节的 `DB_DatabaseType`。PROP 这一条：

| 偏移 | 读法 | PROP 的值 |
| --- | --- | --- |
| 4 | 字符串，没有单独的 getter 名 | `Property` |
| 28 | `Number` / `Type`（`DB_DbType`）。`FindByNumber` 比这个字段 | 4 |
| 32 | `Hash`。`FindByHash` 比这个字段 | 857806 |
| 36 | 字符串，没有单独的 getter 名 | `Property database` |
| 60 | 字符串，没有单独的 getter 名 | `Property` |
| 84 | `TypeName` | `PROP` |
| 108、112 | 两个整数，没有 getter 名 | 都是 2 |
| 116 | `InitialElement` | `CMPW` |
| 140 | 文件名。`IsRealType` 读偏移 156，落在这个字符串对象内部；文件名为空则为 0 | `provir` |
| 164 | `UserCreatable` | 真 |
| 200 | `GlobalOnly` | 假 |
| 201 | `HasDrawings` | 假 |
| 202 | `HasLinkedDocuments` | 假 |

查找还有 `FindByType(string)`。`UserCreatable` 为真，表示这个库类型允许用户创建。`GlobalOnly`、`HasDrawings`、`HasLinkedDocuments` 对 PROP 都为假。

另一处 `CMPW` 字面量在 0x5d87e08，引用者是 `sub_57B25A0`。不是上面这条登记，本轮没拆。

## 2. 使用

使用按「先认出库，再顺着元素，再读属性」这三步。第 2、3 步的活数据来自上一会话，本轮未重跑。

### 2.1 认出库

调用方用类型码 `PROP`、编号 4 或哈希 857806 从 `DB_DatabaseTypes` 取出这一条。文件名是 `provir`，世界元素类型是 `CMPW`。

### 2.2 顺着元素

上一会话在 `acp7008_0001` 会话 47 上对过层次，本轮未重跑：

`WORL` → `CMPW` → `CMPT` → `CMPD`

`CMPD` 不直接挂在 `CMPW` 下，中间是 `CMPT`。同一形状在 `acp7026_0001`、`ams5200_0001`、`ams251048_0001` 上也成立。

目录库里的 `GPART.CMPR` 指向 PROP 库里的 `CMPD`。例：`/GAADTDAKD20033` → `CMPD` `/DAKD20033-DT-CW`（15200/39447）→ `CMPT` `/CAP_WEIGHTS` → `CMPW` `/KG-WEIGHTS` → `WORL` `/*`。

全局符号有 `NOUN_CMPW` 和 `NOUN_CMPD`（`DB_Noun const*`）。名词怎么挂上属性，本轮只看到入口 `DB_Noun::addProperty`（0x58d72c0）：先 `expandList`，再往名词上加属性。函数体没有往下拆。

### 2.3 读属性

上一会话对过物理槽，本轮未重跑。宽布局在 `provir.dat`：`OUTD` 在隐式区第 11 字（2 字），`CWEI` 在第 19 字（2 字）。双精度低字在前。显式流里「属性哈希取负」的 PML 公式压过槽位。

读的时候分三种，上一会话用 e3d-io 公开接口核对过：

1. 槽里是双精度：`attribute`、`get_double`、`get_number` 都给出数。例 `/RB-S-20`（15200/4）的 `CWEI` = 0.2，`OUTD` = 0.0。
2. 常量公式：`attribute` 给出原文，`get_double` 是类型不匹配，`get_number` 能求值。例 `/DAKD20033-DT-CW` 的 `CWEI` 原文 `( 30.93 )`，求出 30.93。
3. 带 `ATTRIB RPRO` 的公式：`attribute` 给出原文，`get_number` 在空环境和只拿这只 `CMPD` 当目录件时都是未解。`RPRO` 要沿目录件的 `DTRE` 去设计表查键。这只特性元素身上没有那张表。

`attlib.dat` 里的 `OUTD`、`CWEI` 是伪属性，不给物理偏移。

### 2.4 运行时引用缓存

`DB_DataPropertyCache::getPropertyReference`（0x591bb70）的形参是当前 `DB_Element`、一个 int、以及用来接结果的 `DB_Element`。元素非空且全局开关 `dword_6395264` 打开时，它走静态表 `propertyCache_`。同组还有 `cachePropertyReference`（0x591b8c0）和 `clearPropertyCache`。缓存键的 int 本轮没有定成哪一个属性号。

## 3. 编辑

能确定的是库类型上的创建许可、引擎侧写属性与 `SAVEWORK` 落盘的函数链，以及写完以后数据长什么样。谁在界面上打开窗体或点保存按钮，没有追到。

### 3.1 创建许可与已排除的三处入口

`UserCreatable` 为真，PROP 允许创建。创建出来的世界元素类型是 `InitialElement` 给出的 `CMPW`。

此前未拆的三处符号均已核对，均不属于元素写属性或数据库保存路径：

1. `sub_57B25A0`（0x57b25a0）：静态初始化函数，分配 `0x130` 字节构造 `DB_Noun("CMPW", 1)` 并存入全局指针 `NOUN_CMPW`（0x6423268）。同批静态初始化还有 `NOUN_CMPD`（`sub_57B2300`，0x57b2300 → 0x64225c4）、`NOUN_CMPT`（`sub_57B24C0`，0x57b24c0 → 0x6421c94）、`NOUN_CMPF`（`sub_57B23E0`，0x57b23e0 → 0x64236cc），以及属性全局指针 `ATT_OUTD`（0x572e710 → 0x641baa4）、`ATT_CWEI`（0x567f0f0 → 0x641c1b4）、`ATT_CMPR`（0x5673770 → 0x641c870）。
2. `DB_Noun::addProperty(int a1, int a2)`（0x58d72c0）：先调 `DB_Noun::expandList(a1, &v7)`，遍历展开后的每个 `DB_Noun*`，若其偏移 `+52..+56` 的向量里还没有 `a2`，则调 `sub_59FCED0(&a2)` 追加。这是往名词定义上挂属性号的 schema 登记，不修改元素实例。
3. `DB_DataPropertyCache`（`getPropertyReference` 0x591bb70、`cachePropertyReference` 0x591b8c0、`clearPropertyCache` 0x591bad0）：调用方在 `Core3D.dll`（`idalib-40832`）的 `sub_108C5230`（0x108c5230，跟踪名 `"DB_DataProperty::getPropertyResultAsValue"`）。它先调 `DB_Qualifier::getFortranIntQualifier(a3)` 取出 `RPRO` 限定词整数哈希作为 `int a2` 缓存键查 `getPropertyReference`；未命中时调 `sub_108C4E80`（0x108c4e80，`"DB_DataProperty::getPropertyRefFromCatalog"`，沿目录件 `ATT_PTDTRE` / `ATT_DTRE` 查找 `ATT_DKEY` 匹配的成员）并调 `cachePropertyReference` 写入 `propertyCache_`，再去读 `ATT_PPRO` / `ATT_DPRO`。因此它是 `RPRO` 读路径的内存缓存，在保存或回滚时只由 `DB_DBPlugger::ClearCaches`（0x591b240）和 `sub_5992F70`（0x5992f70）清表。

### 3.2 写属性函数链（隐式槽双精度 vs 显式流公式）

写属性时，上一会话看到的两条物理落点（本轮未重跑活数据）在 `core.dll` 中对应两条明确的写入函数链：

1. **直存双精度（写入隐式槽，如宽布局 `OUTD` 第 11 字、`CWEI` 第 19 字）**：
   入口 `DB_Element::putAtt(const DB_Attribute *, double)`（0x5948380）→ `DB_Element::putAtt(const DB_Attribute *, const DB_Qualifier &, double)`（0x5948080）→ `DB_Element::internalPutAtt(const DB_Attribute *, const DB_Qualifier &, double, bool)`（0x5941ed0）。`internalPutAtt` 校验 `updateAllowedWithClaim` 与 `DB_LegalityChecksPlugger` 后，对常规浮点属性调 `DB_Element::dabPutAtt(const DB_Attribute *, double)`（0x5930330）。`dabPutAtt` 调 `DB_Element::elGotoCPP(this, 1)` 定位当前元素，取正属性哈希 `v4 = DBE_Base::id(a2)`，调用 `sub_5AAD070(v4, a3)`（0x5aad070，Dabacon 524 号例程 `db_cxx_put_real` → `sub_5AD37A0` 0x5ad37a0 → `sub_5AB7B90(a2, 2, 2, a3, ...)` 0x5ab7b90）。在 `sub_5AB7B90` 内，正属性哈希落在 `[531442, 387951929]` 区间，调 `sub_5AB5920` 取出属性描述符 `v119`，按当前名词是否带 `0x20000000` 宽布局位取 `v119[8]`（宽）或 `v119[5]`（窄）的低 20 位字偏移 `v19 = v126 & 0xFFFFF`，将 2 字双精度直接写进当前元素隐式区 `&v20[2 * v19]`。
2. **写 PML 公式（写入显式流，键为属性哈希取负，公式压过槽位）**：
   入口 `DB_Element::putAtt(const DB_Attribute *, const DB_Expression &)`（0x5946a30）→ `DB_Element::putAtt(const DB_Attribute *, const DB_Qualifier &, const DB_Expression &)`（0x5947c60）。该函数检查 `DB_Attribute::catparam(a2, db_type)` 为真且属性在 `ATT_ATTLIS` 内，通过 `updateAllowedWithClaim` 与合法性插件校验后，对 `DB_Expression::style(a4) == 0` 的表达式调 `DB_Element::dabPutAtt(const DB_Attribute *, const DB_Expression &)`（0x59300d0）。`dabPutAtt` 取属性哈希 `v5 = DBE_Base::id(a2)` 并**取负为 `-v5`**，连同表达式的 32 位字数组与字数 `v11` 传入 `sub_5AAA0B0(-v5, v9, &v11, 0)`（0x5aaa0b0，Dabacon 624 号例程 `db_cxx_put_int_vector` → `sub_5AD26A0` 0x5ad26a0 → `sub_5AB7B90(-v5, 7, v19, v7, ...)`）。在 `sub_5AB7B90` 内，负键 `-v5` 不在正属性哈希区间内，跳过隐式槽，直接在当前元素显式流缓冲区（`*(_DWORD *)(v7 + 4 * v8 + 28)`，从第 4 字起按 `(word[i+1] & 0x3FFFFFF) + 2` 扫描）写入键为 `-v5`、类型标签为 `7`（整数向量）的记录。常量公式可以事后求值；带 `ATTRIB RPRO` 的公式只留下原文，单看这只 `CMPD` 求不出数。

### 3.3 保存写回函数链（`SAVEWORK`）

保存不按 `"provir"` 文件名单独分支，而是对已打开的 `DB_DB` 实例（`this+108` 即 `*((_DWORD *)this + 27)` 为 Dabacon 库号 `dbno`）统一提交：

1. **命令与 MDB 入口**：`sub_59F0EE0`（0x59f0ee0，跟踪名 `"SAVDBX"`，对应 `SAVEWORK`）。若传入具体库号 `*a1`，调 `DB_DB::findDB(*a1)` 取 `DB_DB*` 并调单库 `DB_DB::saveWork(const std::string &)`（0x59112d0）；若 `*a1 == 0`，调 `DB_MDB::saveWork`（0x59f8ce0 / 0x59f8e60）遍历当前 MDB 的库列表，进入 `DB_DB::saveWork(const std::vector<DB_DB *> &, MR_Message *, const std::string &)`（0x5911420 → 0x59114b0）。
2. **单库落盘与缓存清理**：`DB_DB::saveWork`（0x59114b0）先调 `DB_DB::DBSaveWorkAllowed`（0x58f2ab0）和 `DB_DBPlugger::CommitPending`（0x5a43a40），再对每个库调 `DB_DB::dosaveWork(const std::string &)`（0x58ff0c0；局部元素列表保存走 `0x58ff3a0` → `DB_DB::internalSaveWork` 0x59094f0）。`dosaveWork` 校验库打开模式字符串（`this+252`）为 `"WRITE"` 或 `"SHARE"`（对应 `UPDATE` / `MULTIWRITE`）、库存在未提交修改（`sub_5AAD850(dbno, &v15)` 或 `this+116 == 2`）且用户有效（`DB_DB::checkUserStillValid`），随后调用 `db_save_work(int *a1)`（0x5aaf340，Dabacon 198 号例程 → `sub_5AD9880` 0x5ad9880 → `sub_5AE68D0` 0x5ae68d0；排除临时元素时调 0x5aaeb40 即 466 号例程 `db_partial_save_work_all_but`，局部保存调 0x5aaeb10 即 460 号例程 `db_partial_save_work`）把当前会话修改写回库文件。写回成功后调 `DB_DBPlugger::ClearCaches`（0x591b240）清空缓存、调 `DB_DB::specialCaseSavework`（0x5913a20）更新会话号（`this+304`），最后由 `DB_DB::saveWork` 调 `UNDO_CareTaker::ClearAllStates` 清空 undo 状态。

### 3.4 界面上谁去保存、谁去改属性

3.1 安装目录没有 `PMLUI`。下面是 Everything3D 2.10 的脚本。`PMLUI` 里没有 `putAtt`，也没有 `SAVDBX`。

保存：

- `PMLUI\cat\gen\fsystem` 是 Paragon 主菜单。菜单 Paragon 里的 `Save Work`，以及工具条按钮 `.mtbSaveWork`，回调都是 `CALLG MSAVEW`。
- `Paragon.uic` 的按钮 `AVEVA.Catalogue.Project.ButtonSaveWork`（Ctrl+S）绑到命令 `AVEVA.DesignGeneral.CommandSaveWork`。`PMLLIB\common\commands\designsavework.pmlcmd` 的 `execute` 执行 `!!runSynonym('CALLG MSAVEW')`。
- `PMLUI\cat` 下没有名为 `msavew` 的宏。含 PML 命令 `SAVEWORK` 的 `msavew` 在 `PMLUI\des\gen\msavew`、`PMLUI\DRA\gen\msavew`、`PMLUI\iso\admin\msavew`。Design 那份先问 `Savework! Are You Sure?`，确认后发出 `SAVEWORK`。文件布局不能证明 Paragon 的 `CALLG` 会落到 Design 这份宏。
- `!!comSavework()`（`PMLLIB\common\functions\comsavework.pmlfnc`）也会发 `SAVEWORK`。上面的 Paragon 按钮不调用它。

改属性：

- 同一份 `fsystem` 的 Modify 菜单，`Attributes...` 打开窗体 `_CDCATTDEF`。窗体定义在 `PMLUI\clib\query\fattdef`，标题是 `Modify Attributes`。Apply 走 `CALLIBQ MATTDEF`。
- `PMLUI\clib\query\mattdef` 在选项 `MOD` 下，对有改动的属性发出 PML 命令：属性名后面接一个值。数值和带括号的表达式都走这一句，脚本自己不分隐式槽和公式。落到隐式槽还是显式流，仍由第 3.2 节的 `putAtt(double)` / `putAtt(DB_Expression)` 决定。

## 4. 本轮没有做的事

没有打开 E3D 窗口。`NOUN_CMPW` / `NOUN_CMPD` 的登记在第 3.1 节。没有把偏移 4、36、60 的三个字符串对上 getter。没有给偏移 108 和 112 的两个整数起名。没有重跑 `acp7008_0001` 的槽位和 e3d-io 读数。从头对 `core.dll` 做的那次 rea 自动分析，写这份文档时还在跑，文档不引用它的输出。界面这一截只读了 2.10 的 PML，没有把 `CALLG MSAVEW` 断成 Design 的那份 `msavew`。

## 验证

ida-bridge 对 `idalib-29552` 的字符串、交叉引用、构造函数和 getter 查询均为 `exec: ok`。没有保存 IDB。上一会话的层次和读数没有在本轮重跑。
