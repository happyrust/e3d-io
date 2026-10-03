# 取证：SPAMAP 覆盖判定——E3D 给哪些元素登记条目

日期：2026-10-03
任务：write-path-phase-3 7.2
规则：spamap.coverage

> 按 `.kiro/specs/write-path-phase-3/requirements.md` 需求 1 第 7–9 条与需求 5 第 1、7、10 条写；交稿前跑
> `powershell -NoProfile -File tools/ida/check_evidence.ps1 docs/evidence/2026-10-03-spamap-coverage.md`。
> 出处 ① 按 `e67b1c0` 认 E3D_IDB：走树、入队、取框在 Core3D_IDB，noun 字段与软类型在 Core_IDB。
> 后续：`MPBXBR` 已在 `docs/evidence/2026-10-03-spamap-branch-entries.md` 读完——BRAN 与 `HNGT` noun 的成员一律为是，它们自身仍不说，理由改为头管要目录几何。

## 环境

本轮首条与末条调用对两个 IDB 各跑一次 `ib.ps1 env`。

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |
| Core3D_IDB | `D:\ida_scratch\plant3\Core3D.dll.i64`（IDB 记录的输入文件即门用 E3D 的 `Core3D.dll`，哈希见 spec 术语表 Core3D_IDB） |
| Core3D_IDB SHA-256（首条调用前） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| IDA 版本 | 9.2（两个实例相同） |
| HEAD | `6a092e5bcb0704ea45dd14e12a0c3fe496b97da5` |
| Core3D_IDB SHA-256（末条调用后） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| Core_IDB SHA-256（末条调用后） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |

交叉引用核对（复现命令第 23–32 条），全部 PASS：Core3D_IDB `0x102FB475` 2 条、`0x10220F82` 11 条、`0x102FBD62` 1 条、`0x10368418` 4 条、`0x1036CEC4` 4 条、`0x108E7A70` 2 条；Core_IDB `0x53952DC` 64 条、`0x58DE8D0` 39 条、`0x53A8D58` 24 条、`0x6453B98`（全局错误码，被大量引用）2 478 条。

## 结论

E3D 是否给一个元素登记条目，由 Core3D.dll 的 `mapboxlib/MPBOX` 定：它读元素 noun 的 IMAP（noun 字段 847458），1 与 5 交 `MPBOX2` 记本元素；`MPBOX2` 取到框就经 `MBXGO` 交 `maplib/MAPINS` 写条目，取不到框就删条目。元素改动经 `maplist/MLSADD` 按同一个 IMAP 入队，`maplist/VDEMPR` 出队时对它跑 `MPBOX`。由此读出的判定因素有五个：noun 的 IMAP；IMAP 为 7 时 owner 的 IMAP；软类型（UDET）；BRAN 与 `HNGT` 为真的 noun 改走 `MPBXBR`；box type 1 的 noun 须有目录（SPRE 非 0）。`MAPINS` 另有一道 `BTSELE(SPAMAP)`。规则 `spamap.coverage` 坐实，实现为 `rules::coverage_by_imap`，`src/core_rules.rs` 改为 Confirmed。结论按元素分三态：是、否、未坐实；未坐实的是 BRAN、四个 `HNGT` noun、UDET 元素与八个模板里没有 SPAMAP 的 noun，写侧对它们按 D7 回退。坐实集下 Property 7（复现命令第 33 条）：Corpus 各库最新会话中 E3D 写的 42 616 个元素，判定的 40 556 个全部相符、0 反例；未坐实的 2 060 个（BRAN 1 819、HANG 241）记 `waiting`。

### `spamap.coverage`

- 状态：坐实（按元素三态，未坐实的部分见「未坐实的元素」）
- 判定：`mapboxlib/MPBOX` @ `0x102FB475`（Core3D_IDB），`pseudocode.n` = 96–119。n = 96–97：`UDSOFT` 取软类型，`ATNINT` 读其 noun 字段（调用 @ `0x102FB8E7`，字段字 @ `0x10B2FD84` = 847458，即 IMAP）。n = 98–103：noun 为 BRAN（808220）或 `HTCOMP` 为真时交 `mapboxlib/MPBXBR` @ `0x102FC293`，IMAP 当 0。n = 104–115：IMAP 为 7 时 `CRETUR` 到 owner 读它的 IMAP，为 2 当 1，否则当 3。n = 116–117：IMAP 为 1 或 5（`(IMAP & ~4) == 1`）时交 `mapboxlib/MPBOX2` @ `0x102FBD62` 记本元素。n = 118–119：IMAP 为 0 时停；2、3 不记本元素，继续下探成员。
- 入队与出队：`maplist/MLSADD` @ `0x10220F82`（Core3D_IDB）n = 23–25：`UDSOFT` 加 `ATNINT`（字段字 @ `0x10B22B80` = 847458，调用 @ `0x10221067`），IMAP 为 0 不入队；n = 30–42：IMAP 为 3 时沿 OWNER（字段字 @ `0x10B22B84` = 10206636）上溯，到第一个 IMAP 不为 3 的祖先入队；n = 43–57：IMAP 为 7 且 owner 的 IMAP 不为 2 时改由 owner 入队；n = 58 以标志 1（@ `0x10B22B88`）入队；n = 18–21 跳过 GROU、GPWL、WORL（三者的 IMAP 都是 2）。`maplist/VDEMPR` @ `0x102208C7` n = 34–40：对标志为 1 的队项调 `MPBOX`。整库重建 `mapboxlib/MAPDB` @ `0x102FACF4` n = 55–83 逐个顶层元素调 `MPBOX`。元素自身改动即单独入队、单独走 `MPBOX`，所以只判它自己（IMAP 7 再看 owner）；`MAPDB` 遇 IMAP 0 不再下探，只影响整库重建。
- 字段从哪来：core.dll `ATNINT` @ `0x58DE8D0`（Core_IDB）n = 13–22：`DB_Noun::findNoun` 找不到 noun 时值为 0、错误 54；n = 24–29 经 `DB_Noun::getField` 取值。IMAP 由 `DB_Noun::ReadData` @ `0x58D6D20` n = 77–79 以字段 847458 读进 noun 对象 +0xA8，`DB_Noun::spatialMap` @ `0x58DB9D0` 返回此字。`MPBOX` 不看 `ATNINT` 的错误码，字典里没有的 noun 按 IMAP 0 处理（Corpus 里的 DBMETA、DBMDAT 即此类，无条目）。
- 软类型：core.dll `UDSOFT` @ `0x53952DC` n = 9–22：先取 SOFTYP（字段字 @ `0x5D60B50` = 243798436）；是系统 noun（hash 小于 387 951 929）且不是 UDA、UDET 时再取 UDTYPE（@ `0x5D60B54` = 81285816），UDTYPE 大于 0 且有效即改用它。UDET 的 IMAP 在项目字典里、不在 `attlib.dat`，所以 noun hash 超出系统名范围或 UDTYPE 非 0 的元素为未坐实。
- BRAN 与 `HNGT`：core.dll `HTCOMP` @ `0x53A8D58` n = 10–12 为 `ATNLOG(noun, HNGT)`（字段字 @ `0x5D677E8` = 930590）。`attlib.dat` 里 HNGT 为真的 noun 恰是 HANG、LUG、SUPC、TRUNNI。这四个与 BRAN 都交 `MPBXBR`，该例程未读，所以为未坐实。
- 目录（box type 1）：`MPBOX2` n = 31–36：元素没有 WBOX（字段字 @ `0x10B2FDB4` = 1014845）时由 `dbboxlib/BOXELE` @ `0x1036A31C` 取框。`BOXELE` n = 85 读 BOXTYP（字段字 @ `0x10B68EE8` = 243811541，由 `DB_ModelBuilder` 的属性插件转 `DB_Model::getBoxType`），n = 98–101：BOXTYP 为 1 时调 `dbboxlib/MBXSTL` @ `0x10368418`。`DB_Model::getBoxType` @ `0x108E7A70`（Core3D_IDB）n = 84–128：SCTN、GENSEC、FIXING、FITT、PFIT、COFI、SJOI、PJOI、SCOJ、PCOJ、CMPF、CMFI、SBFI、WLJOIN、SUBJ 为 1。`MBXSTL` n = 108–114 调 `dbboxlib/GTSTDT` @ `0x1036CEC4`，后者 n = 123–125 与 169–171：SPRE（字段字 @ `0x10B690EC` = 643429）为 0 时几何集为 0。`MBXSTL` n = 214–223：几何集取不到（`MDBTYP` 不为 2 或出错）时，SCTN、WLJOIN 改以两端位置取框（n = 225–236），GENSEC 不报错（n = 238–240），其余 noun 在 n = 242–255 置失败。`MPBOX2` n = 88–93 把失败交 `mapboxlib/MBXGO` @ `0x102FD8C2`，后者 n = 26–29 调 `MAPREM` @ `0x1038DFC8` 删条目、不写。所以 SCTN、GENSEC、WLJOIN 之外的 12 个 box type 1 noun（其中 COFI 的 IMAP 为 3，本就不记）在 SPRE 为 0 时为否。SPRE 非 0 而目录链在后面断开（`GTSTDT` n = 126–166）时同样取不到框；这属于 World_AABB 是否存在，规则仍判是，写侧此时要调用方给框（需求 6）。
- `BTSELE`：`maplib/MAPINS` @ `0x1038F5E4` n = 146–148：`BTSELE(SPAMAP)`（表名字 @ `0x10B6C8B4` = 230902453）不过即跳过写入。core.dll `BTSELE` @ `0x5A3F7C0` n = 7 转 `sub_5A3D030`，其中属性一级的检查未读完。设计模板（`desvir.dat`）里没有 SPAMAP 属性的被记 noun 恰是 DTPNCH、DTSTEL、DTSUBM、FSPINS、ICRPNT、LADDER、RSFINS、TUBI，它们为未坐实。
- 查过、不是因素的：WBOX 只给框（设计模板里没有带 WBOX 的 noun，测试断言），OBST 与 DDEP 只定 flags（`MPBOX2` n = 84–86、`MPBOX` n = 44–45）。库类型：`MPBOX`、`MLSADD` 不读库类型，`MAPDB` 本轮只读了调 `MPBOX` 的循环；Corpus 中唯一没有 SPAMAP 表的 `ams7351_0001` 是目录库，所含 10 个 noun 按 IMAP 全部判否，库类型不改变结论。
- 实现：`src/spatial/rules.rs` 的 `coverage_by_imap`（按上列次序判）、`imap_of`（IMAP 1 / 2 / 3 / 5 / 7 五张表，239 / 188 / 39 / 60 / 1 个 noun）、`HNGT`、`NO_SPAMAP_IN_TEMPLATE`、`CATALOGUE_BOXED`。`src/spatial/noun_fields.rs` 读 `attlib.dat` 的 noun 字典：目录槽 4 按序列字段，槽 6 索引各 noun 的记录；记录头字为 −1 时取缺省，为 0 时随 SYNO 所指 noun 继承。`tests/spamap_coverage_real.rs` 对 `schema/e3d31/binaries/attlib.dat`（1 931 个 noun，527 个 IMAP 非 0）逐 noun 重读，五张 IMAP 表与 HNGT 表都与字典相同，模板无 SPAMAP 的表与 `desvir.dat` 相同（复现命令第 34 条）。
- Corpus 核对：坐实集下 Property 7（第 33 条）：`ams8000_0001` 会话 646 6 076 / 6 639（`waiting` 563，全是 BRAN），`ams7999_0001` 会话 200 33 212 / 34 709（`waiting` 1 497：BRAN 1 256、HANG 241），`ams251142_0001` 会话 10 948 / 948，`TES023` 会话 109 320 / 320；合计一致 40 556、`waiting` 2 060、总 42 616，判定的元素 0 反例。`ams7351_0001` 没有 SPAMAP 表，所含 10 个 noun 无一被记。`spamap-check --rules confirmed` 的 `coverage` 项给出同样的数（第 35 条）。
- SPRE 一项逐元素对得上：PJOI 在 `ams8000_0001` 0 / 248 有条目、在 `ams7999_0001` 1 394 / 1 874 有条目，SCOJ 0 / 389 与 533 / 922，SUBJ 0 / 428 与 0 / 1 015，FITT 22 / 22 与 4 / 4，PFIT 194 / 194。有条目的恰是 SPRE 非 0 的（Property 7 无反例）。
- 需求 5 第 7 条的括注：`ams8000_0001` 会话 646 中 E3D 写的 EQUI 77 个、ZONE 10 个、SITE 3 个都没有条目，规则判否。GROU、FIXING 在该会话里没有 E3D 写的元素——括注出自 09-22 门用副本里 e3d-io 写的 GROU `24384/31485` 与 FIXING `24384/31488`；按本规则 GROU（IMAP 2）为否，FIXING 要 SPRE 非 0 才为是。

### Geometric_Noun 清单（需求 5 第 10 条）

Corpus 所用的两份 noun 模板：设计模板 `desvir.dat` 的 992 个 noun 与目录模板 `catvir` 的 152 个，去重后 1 135 个。逐 noun 按上面的判定条件归类；复现命令第 34 条逐行打印 noun、IMAP、三态，以及各库「有条目 / E3D 写的元素」个数。下面括号里是这两个数，库名省去 `ams` 与 `_0001`。

- **是**（IMAP 1 或 5，274 个）。有语料实例的 41 个：ATTA（8000 286 / 286，7999 4 560 / 4 560）、BEND（8000 628 / 628，7999 1 715 / 1 715）、BOX（8000 45 / 45，7999 275 / 275，TES023 64 / 64）、CAP（7999 58 / 58）、CONE（7999 92 / 92，TES023 20 / 20）、COUP（7999 50 / 50）、CTOR（7999 14 / 14，TES023 9 / 9）、CTRAY（251142 128 / 128）、CWBRAN（251142 61 / 61）、CYLI（8000 34 / 34，7999 1 035 / 1 035，TES023 137 / 137）、DATU（7999 1 / 1）、DISH（7999 7 / 7，TES023 8 / 8）、ELBO（8000 161 / 161，7999 1 221 / 1 221）、ELCONN（8000 34 / 34）、EXTR（8000 74 / 74，7999 214 / 214）、FBLI（7999 4 / 4）、FLAN（7999 210 / 210）、FTUB（8000 1 559 / 1 559，7999 2 490 / 2 490）、GASK（7999 107 / 107）、HELE（7999 73 / 73）、HROD（7999 14 / 14）、INST（7999 35 / 35）、NOZZ（7999 59 / 59，TES023 4 / 4）、OLET（7999 136 / 136）、PANE（8000 1 / 1，7999 541 / 541）、PCLA（7999 77 / 77）、PCOM（7999 3 / 3）、PNOD（8000 248 / 248，7999 1 870 / 1 870）、PYRA（7999 9 / 9，TES023 47 / 47）、REDU（8000 1 / 1，7999 203 / 203）、RNODE（251142 138 / 138）、RTOR（7999 15 / 15）、SCLA（7999 4 / 4）、SCTN（8000 528 / 528，7999 3 285 / 3 285）、SLCY（TES023 16 / 16）、SNOU（7999 7 / 7）、STRT（8000 2 / 2）、TEE（7999 145 / 145）、VALV（8000 1 / 1，7999 244 / 244）、VTWA（7999 1 / 1）、WELD（8000 1 / 1，7999 139 / 139）。无语料实例的 233 个（归属只以出处 ① 坐实）：ABOX、ACONE、ACTOR、ACYLI、ADISH、AEXTR、AHU、AIDARC、AIDCIR、AIDLIN、ANCI、APOLYH、APYRA、AREVO、ARTOR、ASLCY、ASNOU、BATT、BBLT、BNDLIN、BOUN、BPANEL、BPFITT、BRCO、BWLD、CABLE、CCURVE、CLEV、CLNTIL、CLOS、CNODE、CNVCOM、COWL、CPLANE、CPLATE、CPOINT、CPROTR、CROS、CSEAM、CSTIFF、CSURF、CTJOIN、CTSUPP、CTWALL、DAMP、DOOR、DRAW、DTBOLT、DTHOLE、DTPLAT、DTPROF、DTWELD、DUCT、ELFITT、EQUCOM、EYNT、EYRD、FEBEAM、FEFRED、FEMIMG、FESHEL、FETRUS、FILT、FIXTUR、FLEX、FLOOR、FLRCOV、FNCARE、FNCBRN、FPFITT、FURNIT、GENCUR、GENNC、GENNG、GENPC、GENPG、GENPRI、GENSEC、GRDM、GRIDCY、GRIDPL、GRIL、GWALL、HACC、HANCI、HANDRA、HBEAD、HBRCKT、HBRFLA、HBRSTI、HCLIP、HCOMPT、HCURVE、HDOPLA、HFAN、HFLANG、HIBEA、HIBRA、HICPLA、HICSTI、HICUR、HIDOU、HIFLA、HIPIL、HIPLA、HISTI、HMARK、HNUT、HPILLR、HPIN、HPLATE、HRGATE、HRPANE、HRPOST、HRSOF、HRTERM、HSAD、HSTIFF、HULCON、HVACFI、HVBRCO、HVFLAN、HVHACC、HVIDAM、HVSADD、HVSKIR、HVSPLR、HVSTIF、HVTPPO、IBNDRY、ICLIP、ICRCUR、ICTOUT、IDAM、IEDGE、IENDFR、IENDTO、IFCPLA、IHOLE、ILIM、INFITT、INODE、INOTCH、INSURQ、IPANE、IPILLR、IPLATE、ISEAM、ISECT、ISTIFF、JNODE、KICKPL、LDRCAG、LDRRUN、LJSE、LOAP、LUANCI、MESH、MFIX、OFST、PCLI、PEXSP、PGBOX、PIACT、PIBLK、PIGEN、PLAT、PLEN、PLTFRM、PLUG、POHE、POLYHE、POST、PTAP、PULLN、PVOL、PWCHA、PWMAN、RAIL、RBRAN、RCPL、REVO、RLADDR、RPLA、RSEG、SCREED、SDAFIX、SHOE、SHU、SILE、SKIR、SLADDR、SLUG、SNUB、SOST、SPAC、SPLR、SPMSPC、SRFCUR、SRFSUR、STIF、STLS、STRFLT、STRLNG、STWALL、SUBCOM、SWBR、TAPE、THRE、TP、TRANCI、TRAP、TREAD、TRNB、TRNCUR、TRNN、TRNS、TRREDU、UBOL、UNIO、VENT、VFWA、VSPR、WALL、WASH、WINDOW、WKSUR、WLJOIN、WLPANE、WLPROF、WPAD、XCELL、XGEOM。
- **是，须 SPRE 非 0**（box type 1、IMAP 1 或 5，11 个）。有语料实例的 5 个：FITT（8000 22 / 22，7999 4 / 4）、PFIT（7999 194 / 194）、PJOI（8000 0 / 248，7999 1 394 / 1 874）、SCOJ（8000 0 / 389，7999 533 / 922）、SUBJ（8000 0 / 428，7999 0 / 1 015）。无语料实例：CMFI、CMPF、FIXING、PCOJ、SBFI、SJOI。
- **看 owner**（IMAP 7，1 个）：PALJ，无语料实例；owner 的 IMAP 为 2 时为是，否则为否。
- **未坐实**（13 个，另加软类型为 UDET 的元素）。有语料实例的 2 个：BRAN（8000 3 / 563，7999 219 / 1 256）、HANG（7999 0 / 241）。无语料实例：DTPNCH、DTSTEL、DTSUBM、FSPINS、ICRPNT、LADDER、LUG、RSFINS、SUPC、TRUNNI、TUBI。
- **否**（两份模板中其余的 836 个：IMAP 为 0、3，或为 2 而不是 BRAN 与四个 `HNGT` noun；字典里没有的 noun 也按 IMAP 0 为否）。有语料实例的 47 个全部没有条目：CATA（7351 0 / 12）、CATE（7351 0 / 89 845）、CONSEQ（251142 0 / 14）、CTBEND（251142 0 / 18）、CTMTRL（251142 0 / 39）、CTRISE（251142 0 / 8）、CTSTRA（251142 0 / 96）、CTTEE（251142 0 / 6）、CWAY（251142 0 / 4）、DATA（7351 0 / 2 507 075）、DTSE（7351 0 / 744 948）、EQUI（8000 0 / 77，7999 0 / 90，TES023 0 / 6）、FRMW（8000 0 / 135，7999 0 / 1 421）、HATTA（251142 0 / 61）、HVAC（8000 0 / 2）、LNKI（7999 0 / 71）、LNKS（7999 0 / 71）、LOOP（8000 0 / 74，7999 0 / 215）、MNUM（每库 0 / 1）、NBOX（7999 0 / 17）、NCON（7999 0 / 2）、NCYL（7999 0 / 231）、NPYR（7999 0 / 18）、NXTR（7999 0 / 1）、PAVE（8000 0 / 7，7999 0 / 2 164）、PIPE（8000 0 / 47，7999 0 / 518）、PLOO（8000 0 / 1，7999 0 / 541）、POINTR（251142 0 / 172）、RATTA（251142 0 / 16）、REST（7999 0 / 184）、RPATH（251142 0 / 61）、RTFEAT（251142 0 / 61）、SBFR（8000 0 / 2，7999 0 / 1）、SECT（7351 0 / 2 169）、SITE（8000 0 / 3，7999 0 / 4，251142 0 / 1，TES023 0 / 1）、SNOD（8000 0 / 389，7999 0 / 922）、SPEC（7351 0 / 1）、SPWL（7351 0 / 1）、STRU（8000 0 / 135，7999 0 / 1 423）、SUBE（7999 0 / 28）、SUBS（8000 0 / 41，7999 0 / 3）、TATTA（251142 0 / 61）、TEXT（8000 0 / 24，7351 0 / 1 802）、TMPL（TES023 0 / 2）、VERT（8000 0 / 435，7999 0 / 2 336）、WORL（每库 0 / 1）、ZONE（8000 0 / 10，7999 0 / 25，251142 0 / 1，TES023 0 / 4）。

### 未坐实的元素（需求 2 第 4 条）

- 所作假设：无。规则对这些元素返回「不说」，写侧按 D7 回退：视同没有可用 World_AABB，交给 Spatial_Policy。
- 须用到它的输入类别：noun 为 BRAN、HANG、LUG、SUPC、TRUNNI 的元素（`MPBXBR` 未读）；软类型为 UDET 的元素；DTPNCH、DTSTEL、DTSUBM、FSPINS、ICRPNT、LADDER、RSFINS、TUBI 的元素（`BTSELE` 未读完）；IMAP 为 7 而 owner 属于上述之一的元素。
- Corpus：BRAN 在 `ams8000_0001` 563 个（3 个有条目）、在 `ams7999_0001` 1 256 个（219 个有条目）；HANG 在 `ams7999_0001` 241 个（都没有条目）；其余类别没有实例。BRAN 有的有条目、有的没有，说明由 `MPBXBR` 决定，不能只看 noun。要坐实这两类须读 `MPBXBR`，它也是 `spamap.entries_k` 的 k = 1 与管件 flags 的来源。

## 复现命令

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FB475
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x10220F82
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102208C7
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FACF4
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FBD62
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x102FD8C2
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1036A31C 80 101
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x108E7A70 84 131
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x10368418
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1036CEC4
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1038F5E4 145 153
powershell -NoProfile -File tools/ida/sql.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 "SELECT printf('0x%X', a) AS ea, read_i32(a) AS v FROM (SELECT 0x10B2FD84 AS a UNION ALL SELECT 0x10B22B80 UNION ALL SELECT 0x10B2FDB4 UNION ALL SELECT 0x10B2FDB8 UNION ALL SELECT 0x10B690EC UNION ALL SELECT 0x10B68EE8 UNION ALL SELECT 0x10B22B84 UNION ALL SELECT 0x10B6C8B4 UNION ALL SELECT 0x10B22B88)"
powershell -NoProfile -File tools/ida/sql.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 "SELECT printf('0x%X', address) AS ea, disasm FROM instructions WHERE address BETWEEN 0x102FB8CB AND 0x102FB8E7 OR address IN (0x10221051, 0x10221067) ORDER BY address"
powershell -NoProfile -File tools/ida/pseudo.ps1 0x53952DC
powershell -NoProfile -File tools/ida/pseudo.ps1 0x58DE8D0
powershell -NoProfile -File tools/ida/pseudo.ps1 0x53A8D58
powershell -NoProfile -File tools/ida/pseudo.ps1 0x58D6D20 76 79
powershell -NoProfile -File tools/ida/pseudo.ps1 0x58DB9D0
powershell -NoProfile -File tools/ida/pseudo.ps1 0x5A3F7C0
powershell -NoProfile -File tools/ida/sql.ps1 "SELECT printf('0x%X', a) AS ea, read_i32(a) AS v FROM (SELECT 0x5D677E8 AS a UNION ALL SELECT 0x5D60B50 UNION ALL SELECT 0x5D60B54)"
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FB475 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x10220F82 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x102FBD62 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x10368418 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x1036CEC4 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x108E7A70 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x53952DC
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x58DE8D0
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x53A8D58
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x6453B98
cargo test --release --test spamap_codec_real -- --nocapture --test-threads 1
cargo test --release --test spamap_coverage_real -- --nocapture --test-threads 1
cargo run --release --quiet --bin e3d-descriptor -- spamap-check --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001 --input D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001 --input D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001=646 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams7999_0001=200 --session D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams251142_0001=10 --session D:\AVEVA\Projects\E3D3.1\TEST\TES000\TES023=109 --schema schema/e3d31 --rules confirmed
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/ib.ps1 env
```

第 33 条在 `every_element_e3d_wrote_has_entries_exactly_when_coverage_says_so` 里逐库打印 coverage 的一致数 / 总数与 `waiting` 的 noun；第 34 条核对 IMAP、HNGT 与模板表，并逐 noun 打印上面的清单；第 35 条输出 JSON，各库 `coverage` 的 `agree` / `total` / `waiting` 即上文数字。

## 反例

`spamap.coverage` 判定的元素：无（40 556 / 40 556）。

未坐实的 2 060 个元素（BRAN 1 819、HANG 241）记 `waiting`，不是本规则的反例；其中 BRAN 有 222 个有条目，见「未坐实的元素」。
