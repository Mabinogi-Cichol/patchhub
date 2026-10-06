# EXE 补丁实现审阅

日期：2026-10-06。基线：`d2f551f`，开始审阅时工作区干净。范围：当前 EXE profile、准备 / apply、隔离 worker、Rust 事务、Decky UI 与相关测试；不修改功能代码、不部署、不运行官方安装器。

关联：[设计](design.md) · [施工记录](progress.md) · [完整流程图](flow.md)。

使用两个 GPT-6 Luna / medium 只读 explorer，分别梳理后端与前端 / 测试，主 Agent 复核结论与代码定位。沿用已有自动化与实机记录，本次结论来自静态代码和异常路径推演，未重新执行测试或故障注入。

## P1：纯新增 EXE 事务无法通过界面卸载

- 位置：[index.tsx](../../../apps/decky/src/index.tsx) 第 274、461、499 行；[Transaction DTO](../../../packages/ui/src/index.ts) 第 128–135 行。
- 触发：例如已审核的 VenusBlood FRONTIER 中文 v5，事务只新增三个文件，没有原文件备份。后端 `_annotate_backups` 将 `backups` 标为 `false`。
- 当前结果：补丁详情按 `backups !== false` 过滤可卸载事务，记录入口也使用同一判断；两个按钮均隐藏，界面错误提示“无法卸载，只能重新安装”。这阻断了该正式 profile 的界面卸载路径。
- 依据：Rust 从计划计算 `has_replacements`；[main.py](../../../apps/decky/main.py) 第 516–524 行已允许 `has_replacements=false` 的事务无备份卸载。[test_cache_cleanup.py](../../../apps/decky/backend/test_cache_cleanup.py) 已覆盖后端纯新增卸载。共享 UI 类型未暴露该字段，也没有据此判断按钮。
- 建议：传递 `has_replacements`，纯新增事务允许卸载；存在替换且备份缺失时继续拒绝。文案分别说明移除新增文件与恢复原文件。补充两个 UI 用例：纯新增无备份可以卸载、替换且备份缺失不可卸载。
- 状态：待修复。本次未改代码。

## P2：准备取消的清理 RPC 失败时，计划可能仍可应用

- 位置：[exe_installation.py](../../../apps/decky/backend/exe_installation.py) 第 350–355 行。
- 触发：Rust 已生成计划与 stage，`exe-policy.json` 已保存有效租约，最后复核 / 取消检查进入异常清理；`bridge.discard_plan()` 在请求尚未由 Core 执行时抛出临时 RPC 错误。
- 当前结果：`release_lease()` 在同一 `try` 内位于 `await discard_plan()` 之后，RPC 抛错会跳过释放租约；异常被吞掉。计划可保持 `planned` 且 `lease_active=true`。后端随后删除拥有的 `normalized.zip`，但 apply 使用持久 stage，而非 ZIP。
- 影响：来源、profile、游戏事实仍相同且尚在 900 秒期限内时，通过事务列表取得的计划 ID 仍可能通过 `validate_apply` 和 Rust apply；取消 / 失败的准备没有可靠撤销安装授权，也可能暂时阻止重新准备。
- 边界：若 Core 已执行 discard、只是响应丢失，journal 已为 `rolled_back`，后续 apply 会拒绝；若 profile、来源或实例条件确实变化，对应守卫仍会拒绝。本问题不表示取消后自动安装，也不绕过其它安装确认。
- 测试缺口：[test_exe_installation.py](../../../apps/decky/backend/test_exe_installation.py) 第 234–253 行的取消 / 来源变化用例使用成功 discard 的 FakeBridge，未覆盖此失败分支。本次未执行 RPC 故障注入。
- 建议：独立于 discard 的成败，先使本地 receipt / 租约失效，或在可靠 `finally` 中撤销授权；不能用删除 ZIP 代替授权撤销。补充“已生成 receipt、取消、discard 请求未执行即失败、再 apply 必须拒绝”的故障用例。
- 状态：待修复。本次未改代码。

## 已知体验问题与验收边界

施工记录已有夏日 DLC 同步卸载约两分钟、客户端超时而后端最终成功的事实。当前 [main.py](../../../apps/decky/main.py) 第 512–525 行和界面仍直接等待同步 uninstall RPC，宜纳入可观察后台任务，展示实际进度与最终事务状态。本次没有再次运行大规模卸载。

已有记录支持三个审核 profile 的真实下载、文件安装与恢复，以及 Linux 自动化、隔离与部分 QAM 验证。本次不把它们扩展为游戏内生效、存档兼容、全部语言 / DLC 组合或实体手柄已验收；英文本体等剩余范围继续以 [acceptance.md](acceptance.md) 为准。
