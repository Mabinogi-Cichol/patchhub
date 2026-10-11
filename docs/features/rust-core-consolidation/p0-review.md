# P0 独立复核与放行记录

关联：[设计](design.md)、[施工记录](progress.md)、[契约](p0-contract.md)、[行为表](p0-behavior.md)、[数据/债项](p0-data-debt.md)、[基线](p0-baseline.md)。日期：2026-10-11（Asia/Shanghai）。

结论：**P0 冻结与静态复核通过，可进入 P1；整个 feature 与 v4 生产 API 尚未实现。** 独立 reviewer 只读检查，不修改代码。主 Agent修订后进行定向复核，三项 P2 均关闭，无遗留具体 finding。

| 初审发现 | 作者实际修正 | 定向复核 |
| --- | --- | --- |
| facts unknown 一律阻断，易误禁无 committed plan 的动态首次安装 | 行为表拆分 DetectionStatus unknown 与执行必需 facts 读取失败；unknown 本身不禁成熟 overwrite，路径/准入/新鲜 facts 仍强制重核 | 通过 |
| recovery=false 与契约 nullable 冲突；cleanup失败固定 performed=false，无法表达 Partial | 无恢复为 null；needs_recovery=failed+required；保留实际 cleanup status/performed/removed_bytes/failures | 通过 |
| 未来 v4 基准只改 version，仍发 v3 顶层配置 | scan 按 version4 构造六字段 params.host；版本 choices3/4，文档明确 v4 adapter 尚待真实 CLI | 通过 |

复核覆盖接口职责与直接 API 边界、signed/publisher/本地路线、三 operation/主按钮、unknown/恢复/部分清理、现有数据读取与切换、v4 字段/事件/版本/限额、基准计数/证据真实性。契约作者额外核对并收口了完整行含 LF 上限、现有带 path/detail 的错误转换、幂等重放 typed cause 从现有 error 精确码恢复、不新增数据库 schema。

独立 reviewer 实际执行 PowerShell compact JSON UTF-8+LF 计数：install 请求 384 字节；三个 outcome 509/572/633 字节，与 [静态样本](p0-wire-samples.json) 及基线一致。主 Agent另执行 [基准 adapter 结构检查](../../../.artifacts/rust-core-consolidation/p0-harness-check.json)：v3 参数保留、v4 六字段 host、请求超限在进程前拒绝、完整 8 MiB 行接受/8 MiB+1拒绝。结构检查用 subprocess double，没有冒充真实 v4 运行。

多 Agent：p0_contract（specialist，gpt-6.1-sol/high，公共契约风险升档）；p0_behavior 与 p0_data_debt（worker，gpt-6-luna/medium，balanced默认）；p0_review（reviewer，gpt-6.1-sol/high，公共API/数据保全升档）。共4个，fork_turns均none，文件写入范围互斥。主 Agent负责测量工具、集成、修复和最终文档验证。

未运行：v4/new API 编译或真实协议、生成契约漂移、cache/trust新增竞态、双端包、Steam Deck 连接/部署/安装及实机。P0普通/计数临时副本编译和扫描基准由主 Agent执行，详见基线；独立 reviewer没有重跑完整基准。以上实施与验收仍是 P1–P6 门槛，不在P0提前声明完成。
