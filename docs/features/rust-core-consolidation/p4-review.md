# P4 独立审阅与修复记录

设计：[design.md](design.md)。施工：[progress.md](progress.md)。验证：[p4-validation.md](p4-validation.md)。

2026-10-12，独立只读 reviewer（GPT-6.1 Sol/high）检查动作契约、生产者、消费者与执行边界，回传两项可操作 P2 问题。主 Agent 已实施修复并运行最终矩阵。本记录区分已收到的审阅意见与主 Agent 的修复验证；审批服务用量中断后没有收到 reviewer 最终全范围 findings[] 报告，因此不将该报告记为已通过。P6 全范围独立 gate 仍待执行。

| 问题 | 修复 | 证据 |
| --- | --- | --- |
| 当前 committed 收据存在但 plan 不可读，动态无文件证据/签名文件缺失时错误显示 install | 单独使用当前实例、匹配 release/selection 的 committed 记录作为标签证据；完整 plan、content 与 live hashes 仍是严格 Reinstall 的要求；superseded/rolled_back 不计当前标签收据 | projection 的 unreadable/empty/unsafe plan 及 unreadable_missing 真值表回归 |
| Windows 根据写入 availability 隐藏整个安装/预览区，使禁用来源、过期/unsigned 目录等无法只读查看变更 | 预览区始终供选中 patch 使用，主写按钮由 actions 控制；不可用时保留禁用按钮，Decky 保持一致 | Windows 类型检查；主 Agent 检查 JSX 预览入口与写入动作条件独立 |

主 Agent 额外修正并验证：旧执行允许无收据但文件全匹配，以及 owned receipt 的损坏文件走 Reinstall；现已要求 owned/intact，同内容损坏修复使用 Overwrite。现有 needs_recovery 互斥原本已有效，新增明确码与扫描后跨 patch 的回归，并未将最初误读记为实际绕过漏洞。ZIP 与 EXE 使用各自生产收据身份（ZIP source release/recipe hash，EXE content binding）。未知 capability 拒绝原因保留于有界 detail 并保守映射封闭动作码；无新的同义权限字段。

真实 asyncio StreamReader 的超限 ValueError/LimitOverrunError 也归一化为 response_too_large；使用真实行读取器回归，避免仅 fake readline 的长度检查形成虚假证据。

没有修改真实游戏、连接设备或部署。复核完成时修复均在本地工作树，尚未提交。

后续提交记录（2026-10-12）：P3/P4 代码已本地提交 `19ffc8a`，文档独立提交。最终全范围独立 gate 仍待 P6。
