# 取证：<题目>

日期：YYYY-MM-DD
任务：write-path-phase-3 <任务号>
规则：<本文坐实或考察的规则名，逗号分隔，与 `src/core_rules.rs` 的 `RULES` 同名>

> 按 `.kiro/specs/write-path-phase-3/requirements.md` 需求 1 第 7–9 条写；交稿前跑
> `powershell -NoProfile -File tools/ida/check_evidence.ps1 <本文路径>`。

## 环境

本轮首条与末条调用各跑一次 `tools/ida/ib.ps1 env`，原样抄入；本轮也读 Core3D_IDB 时，另以 `--target-idb` 对它首尾各跑一次。每个 IDB 的两次 SHA-256 必须相同（只经 IB_Wrapper 的一轮取证不改 E3D_IDB）。`check_evidence.ps1` 把写着 `Core3D_IDB` 的 SHA-256 行归 Core3D_IDB、其余归 Core_IDB，分组比对。本轮没读 Core3D_IDB 就删去它的三行。

| 项 | 值 |
|---|---|
| Core_IDB | `D:\ida_scratch\plant3\core.dll.i64` |
| Core_IDB SHA-256（首条调用前） | `<64 位十六进制>` |
| Core3D_IDB | `D:\ida_scratch\plant3\Core3D.dll.i64` |
| Core3D_IDB SHA-256（首条调用前） | `<64 位十六进制>` |
| IDA 版本 | `<env 所报>` |
| HEAD | `<40 位十六进制>` |
| Core3D_IDB SHA-256（末条调用后） | `<64 位十六进制>` |
| Core_IDB SHA-256（末条调用后） | `<64 位十六进制>` |

## 结论

每条规则一节，节名即规则名。

### `<规则名>`

- 状态：坐实 / 未坐实
- 例程：`<函数名>` @ `0x<起始地址>`（Core_IDB / Core3D_IDB）；行为所在：`pseudocode.n` = `<行号>`，或指令 @ `0x<地址>`
- Corpus 核对：库 `<…>`；命令见「复现命令」第 `<n>` 条；一致数 / 总数 = `<a>` / `<b>`
- 未坐实时另写：所作假设；须用到它的输入类别；反例见「反例」

## 复现命令

只用三种入口：`tools/ida/ib.ps1`、按仓库根相对路径调用的仓内脚本、`cargo`。不写临时目录、实例号（`idalib-<n>` / `idaui-<n>`）与 ida-bridge 安装目录；所用 SQL / IDAPython 文件放 `tools/ida/` 下并入库。在仓库根执行。

```powershell
powershell -NoProfile -File tools/ida/ib.ps1 env
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/ib.ps1 exec --sql "SELECT name, start_ea FROM funcs WHERE name = '<函数名>'"
powershell -NoProfile -File tools/ida/pseudo.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 0x<起始地址>
powershell -NoProfile -File tools/ida/check_xrefs.ps1 0x<地址>
cargo run --release --bin e3d-descriptor -- spamap-census --input <库>
powershell -NoProfile -File tools/ida/ib.ps1 --target-idb D:\ida_scratch\plant3\Core3D.dll.i64 env
powershell -NoProfile -File tools/ida/ib.ps1 env
```

## 反例

无；或逐条：库、refno、期望、实得、所依的未坐实规则名。
