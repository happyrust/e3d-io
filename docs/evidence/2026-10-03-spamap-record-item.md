# 取证：SPAMAP_Record_Item——元素记录里的 SPAMAP 属性

日期：2026-10-03
任务：write-path-phase-3 7.4
规则：spamap.record_item.<NOUN>

> 按 `.kiro/specs/write-path-phase-3/requirements.md` 需求 1 第 7–9 条与需求 5 第 5、11 条写；交稿前跑
> `powershell -NoProfile -File tools/ida/check_evidence.ps1 docs/evidence/2026-10-03-spamap-record-item.md`。

## 环境

本轮首条与末条调用对两个 IDB 各跑一次 `ib.ps1 env`。

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |
| Core3D_IDB | `D:\ida_scratch\plant3\Core3D.dll.i64`（IDB 记录的输入文件即门用 E3D 的 `Core3D.dll`，哈希见 spec 术语表 Core3D_IDB） |
| Core3D_IDB SHA-256（首条调用前） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| IDA 版本 | 9.2（两个实例相同） |
| HEAD | `3731d13ce3396e19fdbc0eaa522bea4c142b2e3f` |
| Core3D_IDB SHA-256（末条调用后） | `03129C29C5D87E7F465ADCC6DB79B49B04DE631E10F738CE57CFDFC91F9DA712` |
| Core_IDB SHA-256（末条调用后） | `18D5C15525696730E83D35A88D58B421DF0A15D699F82CCF0821908A72A9992C` |

交叉引用核对（复现命令第 7–13 条），全部 PASS：Core_IDB `0x5AB91B0` 11 条、`0x5AB7B90` 24 条、`0x5AB0E60` 16 条、`0x5AB5BC0` 62 条、`0x5AFDF10` 42 条、`0x6453B98`（全局错误码，被大量引用）2 478 条；Core3D_IDB `0x1038F5E4` 2 条。

## 结论

SPAMAP 条目与记录项由 core.dll 同一段类型 14 表属性代码维护：Core3D 的 `maplib/MAPINS` 每写一条条目就 `DPUTIT(SPAMAP, 下标, Key0, 7 字)`，core 的 `sub_5AB91B0` 删旧键、插新键，并把该下标的 Key0 写进元素记录的 `SPAMAP` 属性（类型 14）对应槽位，记录里原先没有这一项时先把整列填成 0x80000001。记录项因此是按 k 排列的 Key0 列：第 k 槽为该元素 k 条目的 Key0，没有条目的槽为 0x80000001（core 另把 0x80000000 也当空槽）。这段代码不看 noun，所以凡 E3D 给了条目的元素都带记录项（模板里没有 SPAMAP 的 noun，`BTSELE` 不过，本就没有条目）。规则 `spamap.record_item.<NOUN>` 坐实为「各 noun 相同」，`src/core_rules.rs` 改为 Confirmed（`RecordItem::Key0ByK`）。Corpus 核对（复现命令第 15 条）：基线会话 640 有条目的 3 628 个元素全部带记录项且逐槽相符；Corpus 各库最新会话 25 517 / 25 517，46 个 noun 无一例外。

### `spamap.record_item.<NOUN>`

- 状态：坐实（各 noun 相同）
- 例程：core.dll `sub_5AB91B0` @ `0x5AB91B0`（Core_IDB，类型 14 表属性写），`pseudocode.n` = 261–331：元素宽 v20（SPAMAP 为 1 字，即 Key0），下标 a2，槽偏移 = v20 × (a2 − 1)（n = 262–266），即 k；表键 = [值, refno 第一字, refno 第二字, a2 − 1]（n = 267–271，与 `spamap.table_shape` 相同）；记录尚无此项时整列填 0x80000001（n = 274–284）；已有时读出该槽旧值，旧值不是 0x80000000 或 0x80000001 即有旧键要删（n = 286–303）；n = 316–331 比较新旧值。n = 396–419：按新长度调 `sub_5AB7B90` @ `0x5AB7B90` 把整列写回记录（该例程是 core 写记录属性的通用入口，24 处调用，复现命令第 6 条）。
- 入口：Core3D `maplib/MAPINS` @ `0x1038F5E4` n = 146–152：`BTSELE(SPAMAP)` 过了才 `DPUTIT(SPAMAP, 下标, Key0, 7 字)`（core `DPUTIT` @ `0x5AB0E60` 经例程 147 到 `sub_5AB91B0`，见 encoding 文档）。
- 类型码、字数、布局、位置：类型码 14；数据字为按 k 排的 Key0（每槽 1 字），长度至少为最大 k + 1，空槽 0x80000001。位置由 noun 的设计模板给出：原生 BOX `24384/24777` 的 `SPAMAP` 在 implicit 区 word offset 35、存储 2 字，读侧解出值 `[299969986]`，即其唯一条目（k = 0）的 Key0（复现命令第 14 条）。
- 与条目键字的对应：记录项第 k 槽 = 键为 [Key0, refno, k] 的那条条目的 Key0；条目删去时该槽改回空槽字。
- 实现：规则无函数，`RecordItem::Key0ByK` 记下布局，写侧（8.x）按它在登记 / 重算条目的同一会话写记录项（需求 5 第 6 条）。
- Corpus 核对：新测 `tests/spamap_record_item_real.rs`（第 15 条）逐元素核「每条条目的 Key0 在第 k 槽、其余槽全是空槽字」：基线会话 640 3 628 / 3 628；`ams8000_0001` 会话 646 3 628 / 3 628、`ams7999_0001` 会话 200 21 257 / 21 257、`ams251142_0001` 会话 10 327 / 327、`TES023` 会话 109 305 / 305，合计 25 517 / 25 517。
- 逐 noun（需求 5 第 11 条；Corpus 各库最新会话合计，每个 noun 都是带记录项 = 有条目的 E3D_Authored_Element 数、不带 0）：ATTA 4 846、BEND 2 343、BOX 384、BRAN 222、CAP 58、CONE 112、COUP 50、CTOR 23、CTRAY 128、CWBRAN 61、CYLI 1 206、DATU 1、DISH 15、ELBO 1 382、ELCONN 34、EXTR 288、FBLI 4、FITT 26、FLAN 210、FTUB 4 049、GASK 107、HELE 73、HROD 14、INST 35、NOZZ 63、OLET 136、PANE 542、PCLA 77、PCOM 3、PFIT 194、PJOI 1 394、PNOD 2 118、PYRA 56、REDU 204、RNODE 138、RTOR 15、SCLA 4、SCOJ 533、SCTN 3 813、SLCY 16、SNOU 7、STRT 2、TEE 145、VALV 245、VTWA 1、WELD 140。BRAN 的 222 个只有 k = 1 条目，记录项为 `[0x80000001, Key0]` 形式，同样相符。

## 复现命令

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/pseudo.ps1 0x5AB91B0 261 331
powershell -NoProfile -File tools/ida/pseudo.ps1 0x5AB91B0 396 419
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x1038F5E4 145 153
powershell -NoProfile -File tools/ida/sql.ps1 "SELECT printf('0x%X', x.from_ea) AS site, f.name AS caller FROM xrefs x JOIN funcs f ON x.from_ea >= f.start_ea AND x.from_ea < f.end_ea WHERE x.to_ea = 0x5AB7B90 AND (x.type_name IS NULL OR x.type_name <> 'flow') ORDER BY x.from_ea"
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x5AB91B0
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x5AB7B90
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x5AB0E60
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x5AB5BC0
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x5AFDF10
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x6453B98
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x1038F5E4 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64
cargo run --release --quiet --bin e3d-descriptor -- extract --db D:\AVEVA\Projects\E3D3.1\AvevaMarineSample\ams000\ams8000_0001 --refno 24384/24777 --schema schema/e3d31
cargo test --release --test spamap_record_item_real -- --nocapture --test-threads 1
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/ib.ps1 env
```

第 14 条输出 JSON，`SPAMAP` 一行的 `storage_type_code`、`storage.word_offset`、`storage.storage_words` 与 `value` 即上文；第 15 条逐库打印「带记录项 / 有条目」「逐槽相符」与逐 noun 数。

## 反例

无：基线 3 628 / 3 628，Corpus 25 517 / 25 517，46 个 noun 全部「带 = 有条目、逐槽相符」。
