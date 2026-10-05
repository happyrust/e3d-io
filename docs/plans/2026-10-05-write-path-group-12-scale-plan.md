# 第 12 组：staging 规模优化

稳定基线为 e1425db。只推进 12.1–12.5；不改发号、页 1 写侧或几何规则，不替代 10.5/10.6 人工验收。

1. 保留 open-only 和 32/128/257 预铸金样入口。新增 release Scale_Probe：同 owner 插入 n 个 BOX、更新 1000 个已落盘 BOX、删除 100 个已落盘 BOX。准备副本不计时；输出 open/build/stage/commit、进程 peak RSS、七类追加页与完整成员读回结果。
2. 在 10k/100k 上各跑至少三次热缓存；采样剖析原始写侧，保存原始数据，在 Spec_Ledger 记录实际热点后才优化。
3. 优化首提交的直接父提交固定时钟、预铸 refno，录制并重复验证混合规模金样；保持所有活跃旧金样和精确退役关系。
4. 只优化有剖析证据的热点；记录顺序、合并/重试、错误及页面字节不变。运行原金样、空间参考模型和混合批次读侧检查。
5. 三次复测 Off/Require：100k/10k stage 比值 ≤15，Require 10k stage+commit ≤10s，非空间页 ≤1.5×record，空间页满足规格界限。运行完整 Cargo、clippy/doc，保留源库哈希；验收后才勾任务。

状态：基线提交5548ad0，首次stage比例62.63/31.95失败的原始数据与剖析保留在`docs/evidence/2026-10-05-staging-scale-baseline.md`。在该提交两遍录金样（5键、0剔除，旧文件不动），随后只修改已剖析的INDEX/成员/空间热点；首优化提交直接父仍为5548ad0。三热轮Off/Require的stage比例7.23/8.72，通过≤15；Require10k stage+commit231.74ms，通过≤10s，页数界限全部通过。12个完整文件及页数与基线相同；Cargo768/0/30、Clippy/doc63/25，真TTY39PASS。详细数据与限制见`docs/evidence/2026-10-05-staging-scale-acceptance.md`。12.1–12.5验收后勾选；10.5/10.6与第13组仍待办。
