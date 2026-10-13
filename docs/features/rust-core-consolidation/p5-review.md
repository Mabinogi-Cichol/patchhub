# P5 协议与宿主生命周期独立复核

日期：2026-10-13（Asia/Shanghai）。关联：[设计](design.md)、[施工记录](progress.md)、[验证](p5-validation.md)。

本阶段四个 GPT-6.1 Sol/high 子 Agent，fork none：三路 specialist 负责 protocol/generator、Windows、Decky；第四路 reviewer 只读审阅。主 Agent 负责 CLI、共享 UI、集成修复、实际矩阵及证据。审阅没有扩大到部署或真实游戏写入。

## 发现与闭环

| 发现 | 修复及验证 |
| --- | --- |
| P1 Windows 传输错误 kill writer，以及晚到数据改写有效终态 | 改为 cancel/EOF/有界排空，超时继续持有安装互斥直到真实退出；首 terminal 封闭解析。PowerShell 真实子进程覆盖 applying 后错误、接收取消/EOF、真正 committed/rolled_back 标记；35 宿主测试通过 |
| P2 Python 能力字段比 Rust 宽松 | schema 固定 formats、strategies、wire-link available/blockers 一致性；Rust/Python 消费同图和跨语言负向夹具 |
| P2 256-byte 实例上限拒绝合法 Steam 路径 | instance ID 唯一 16 KiB 限额，完整身份保留；core/ledger/Windows/Decky一致。真实扫描与 signed 安装、重复重放和 UTF-8 边界通过 |
| P3 recovery.code:null Python/Rust 行为不一致 | Python 接受结构声明的 nullable 值，固定夹具覆盖；必需机器码字段仍严格 |
| 后续复核：ledger 残留 256-byte 限额 | 引用 core 唯一实例常量；长实例 signed 安装及两次幂等 replay 通过 |
| 后续复核：历史大诊断进入新 outcome 导致超限掩盖 commit | InstallerOutcome.error 使用 2048-byte 展示副本，保持原 journal/ledger。9 MiB 旧诊断两次 replay 仍为 completed/committed，原持久字串不变；schema 同步 exact/+1/null/历史 Transaction.error 夹具 |
| 根 Agent 补查：Decky terminal 后非法 UTF-8/JSON/超限行仍影响结果 | 首个有效 terminal 后只做原始 64 KiB 分块排空；真实 StreamReader 覆盖三类异常和冲突第二终态 |
| 最终复核边界：Decky terminal 后 peer 不 EOF 可能无限等待 | 添加时间/累计字节上限与后台退出观察，保留首 receipt；定向测试确认结果及时返回、不发送额外 control、不 kill writer |
| 同包冒烟：CLI 早拒绝空 ID 与生成错误信封不一致 | ErrorResponse 的必需 request_id 允许无可靠 ID 时为 null；版本早拒绝纯读取合法 ID 回填，Success ID 仍必需字符串。共享夹具、生产者回归和重建后同包冒烟通过 |

## 结论与范围

指定协议/身份/大诊断/终态问题已修复，最终只读复核无剩余 actionable finding。实际测试由主 Agent 和实现 Agent 执行，reviewer 本身没有重跑完整矩阵。完整数量、构建、共享夹具与隔离同包冒烟见 [P5 验证](p5-validation.md)。

P6 全功能独立 gate、Windows 最终安装包、真实 UI/游戏/Steam Deck 验收尚未执行；本结论只关闭 P5 的具体发现，不宣告整个 feature 完成。
