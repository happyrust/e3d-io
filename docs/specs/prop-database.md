# PROP 数据库 to-spec

日期：2026-10-14。按已经核对过的结论写，不再采访。不发 GitHub issue。

证据在 `docs/prop-database-prd.md`。活数据是 `acp7008_0001`（db 7008，会话 47），本文件没有重跑。

## 问题

特性库 PROP 存在 `provir.dat`。目录件用 `GPART.CMPR` 指到库里的 `CMPD`。读 `OUTD` / `CWEI` 时，直存双精度、常量公式、带 `ATTRIB RPRO` 的公式不是同一条路。写和保存发生在引擎里，界面脚本不叫 `putAtt`，也不叫 `SAVDBX`。

## 方案

读数只用 e3d-io 已经公开的 `attribute`、`get_double`、`get_number`。

- 槽里是双精度：三条接口都给数。
- 常量公式：`attribute` 给原文，`get_double` 报类型不匹配，`get_number` 求值。
- `ATTRIB RPRO`：在目录件上用 `catalogue_property_number`。它顺着 `CMPR` 读特性元素上的公式，键先查该目录件的 `PTDTRE`，没有再查 `DTRE`。只把这只 `CMPD` 自己当目录件时，它身上没有设计表，`get_number` 仍是未解。

编辑和保存保持现状：界面发 PML 命令，引擎走已有的 `putAtt` 和 `SAVEWORK`。这一份规格不新写写回实现。

## 用户故事

1. 读到直存的外径和元件重。`/RB-S-20`（15200/4）的 `CWEI` 是 0.2，`OUTD` 是 0.0。
2. 读到常量公式的数。`/DAKD20033-DT-CW`（15200/39447）的 `CWEI` 原文是 `( 30.93 )`，`get_number` 求出 30.93。
3. 读到查表公式的原文，并知道数还没算出来。舱口 `/Rectangular_Non_Weathertight_Escape_Hatch_Weight`（15200/33778）的 `CWEI` 是 `( ( ATTRIB RPRO LIDV + ATTRIB RPRO COMV + ATTRIB RPRO CBAV ) * 7850 + 5 )`。
4. 从目录件走到特性元素。`/GAADTDAKD20033` 的 `CMPR` 指到上面第二只 `CMPD`，再经 `CMPT` `/CAP_WEIGHTS`、`CMPW` `/KG-WEIGHTS` 到 `WORL` `/*`。
5. 在 Paragon 里保存当前会话。菜单和 Ctrl+S 都进 `CALLG MSAVEW`，确认后的 PML 命令是 `SAVEWORK`。改属性时 Apply 发出「属性名 + 值」，不在脚本里选择槽位或公式。

## 实现决定

- 层次是 `WORL` → `CMPW` → `CMPT` → `CMPD`。`CMPD` 不直接挂在 `CMPW` 下。
- 库类型：类型码 `PROP`，编号 4，哈希 857806，世界元素 `CMPW`，文件名 `provir`，`UserCreatable` 为真。
- 宽布局：`OUTD` 在隐式区第 11 字（2 字），`CWEI` 在第 19 字（2 字）。双精度低字在前。显式流里属性哈希取负的公式压过槽位。
- `attlib.dat` 里的 `OUTD`、`CWEI` 是伪属性，不拿来当物理偏移。
- 直存写入走 `DB_Element::putAtt(double)`。公式写入走 `putAtt(DB_Expression)`，键是属性哈希取负。
- 保存不按 `provir` 分支。入口是 `SAVDBX` / `SAVEWORK`，再进 `DB_DB::saveWork` → `db_save_work`。
- 界面证据是 Everything3D 2.10 的 PML。3.1 安装目录没有 `PMLUI`。`PMLUI\cat` 下没有 `msavew`。含 `SAVEWORK` 的宏在 Design、Draft、Isodraft。不能据此说 Paragon 的 `CALLG` 落到 Design 那一份。
- 带 `ATTRIB RPRO` 的公式留在未解，直到目录件的 `DTRE` 和设计表接进来。

## 测试决定

不新开解析缝。对照 `acp7008_0001` 会话 47 上已经核过的三只 `CMPD`，各走 `attribute`、`get_double`、`get_number`（空环境，以及只拿该 `CMPD` 当目录件）。

| 元素 | 期望 |
| --- | --- |
| `/RB-S-20`（15200/4） | 三条接口都是 `CWEI` = 0.2，`OUTD` = 0.0 |
| `/DAKD20033-DT-CW`（15200/39447） | `attribute` 为文本 `( 30.93 )`；`get_double` 为 `TypeMismatch`；`get_number` 为 `Evaluated { value: 30.93, form: Pml }` |
| `/Rectangular_Non_Weathertight_Escape_Hatch_Weight`（15200/33778） | `OUTD` 公式 `( 0 )` 由 `get_number` 求出 0.0；`CWEI` 的 `ATTRIB RPRO` 原文由 `attribute` 给出，`get_number` 为 `Unresolved` |

不改这只库。探针用完即删。

## 范围外

- 不在没有目录件的时候猜一张设计表。只拿 `CMPD` 自己当目录件，`ATTRIB RPRO` 仍是未解。
- 不改 PROP 库，不改 e3d-io 的写回。
- 不打开活的 E3D 窗口，不编 3.1 的菜单名。
- 不把 `CALLG MSAVEW` 断成 Design 的 `PMLUI\des\gen\msavew`。
- 不给类型对象偏移 4、36、60 的字符串补 getter 名，不给偏移 108、112 的整数起名。
- 不发 GitHub issue。
