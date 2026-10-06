# 流程精简调研

日期：2026-10-06。代码基线：`d2f551f`。设计：[design.md](design.md)。施工记录：[progress.md](progress.md)。

用户提供的“执行 EXE 补丁安装计划”聊天已通过 read_thread 读取，结合当前 EXE 流程图（当时工作区已有的 flow.md）、代码审阅（当时工作区已有的 review.md） 与实际源码核对。本次为静态调研；旧聊天和旧验收不代表精简流程已实现。

## 1. 当前冗余来自多个层次

| 环节 | 当前证据 | 精简影响 |
| --- | --- | --- |
| EXE 三项人工确认 | [install.ts](../../../apps/decky/src/install.ts) `ExeConfirmations`、`exeConsentReady`；[index.tsx](../../../apps/decky/src/index.tsx) 三个 ToggleField；[exe_installation.py](../../../apps/decky/backend/exe_installation.py) `CONFIRMATIONS`、`_accept` | 要同时删除 UI gate、RPC 参数及 receipt 人工声明，不能仅隐藏勾选或后台传 true |
| 下载后手动安装 | [index.tsx](../../../apps/decky/src/index.tsx) `finishTask` 将 prepare 结果存入页面，安装计划区再打开确认弹窗；[task-controller.ts](../../../apps/decky/src/task-controller.ts) 分别 startPrepare/startInstall | 需要一个后端任务连续执行，计划只留内部 |
| 全局与 EXE 备份规则 | [main.py](../../../apps/decky/main.py) `prepare_install` 强制 EXE settings.keep_backups，`_cleanup_after_install` 对 EXE must_keep；[plugin_settings.py](../../../apps/decky/backend/plugin_settings.py) keep_backups | 删除用户设置和成功后的长期保留，而非破坏正在执行的失败恢复 |
| EXE 备份无法清掉 | [caches.py](../../../apps/decky/backend/caches.py) `protected_exe_jobs` 以目录身份保护整个 EXE state，`delete_files` 剔除 backup 删除 | 需要改为真实事务终态/迁移保护，不能只改 cleanup_after_install |
| 卸载多层接口 | [index.tsx](../../../apps/decky/src/index.tsx) 补丁与记录两个入口；[main.py](../../../apps/decky/main.py) `uninstall`；[bridge.py](../../../apps/decky/backend/bridge.py) 方法白名单；[runtime.rs](../../../crates/cli/src/runtime.rs)、[install.rs](../../../crates/core/src/install.rs) | 产品/API/状态生产端共同删除；restore 中的安装回滚继续使用 |
| Windows 仍有旧步骤 | [Windows UI](../../../apps/windows/src/main.tsx) 手动获取、计划、window.confirm、卸载；[Tauri](../../../apps/windows/src-tauri/src/main.rs) operation 与确认参数 | 共享协议变更需同步；Windows 只迁移原来支持的 ZIP 范围 |

## 2. 重装并非已有通用能力

- [install.rs](../../../crates/core/src/install.rs) `ensure_no_active`（555 行起）在 prepare/apply 阻止同实例 committed 以及未完成事务；apply 对同一个 committed plan 是校验后返回，并不会重新写文件。
- [publisher.py](../../../apps/decky/backend/publisher.py) `prepare` 也拒绝未卸载/回滚的事务；[exe_installation.py](../../../apps/decky/backend/exe_installation.py) `_idle` 同样拒绝 committed。
- [main.py](../../../apps/decky/main.py) `_forget_without_backups`（444 行起）与 [caches.py](../../../apps/decky/backend/caches.py) `forget_transactions`（188 行起）通过删除无备份事务来绕开普通 ZIP 的重装阻塞。该方式会丢失安装来源/归属事实，不能成为新方案。
- [runtime.rs](../../../crates/cli/src/runtime.rs) 扫描根据同 release 的 committed journal 将已安装外部文件投影为托管状态；成功后直接删 journal 会改变检测语义。
- [exe_facts.py](../../../apps/decky/backend/exe_facts.py) `game_facts`（128 行起）仍检查实例、build、语言、已安装 DLC 和固定基线；prepare/validate_apply 前后重复检查 facts。
- [exe_profiles.json](../../../apps/decky/backend/exe_profiles.json) 当前三档为 VBF 中文 v5、夏日中文 DLC v2.02.1、夏日中文本体 v1.03。基线为 VBFI.exe 或 Game.exe/Script.vdf，载荷为 extra.xp3 或 www 文件。此分离支持调查同 profile 重装，不能推广到覆盖自身基线的未来 profile。

结论：需保留最小安装收据、增加显式同内容重装语义，并把当前内容快照与原版基线分开。历史成功记录不应一概当作正在执行的事务，也不能一概删除来放行。

## 3. 后台任务目前依赖界面存活

- [tasks.py](../../../apps/decky/backend/tasks.py) `LEASE_SECONDS=15`，watchdog 在没有 UI 续租时请求取消；`task_status` 续租。
- [task-controller.ts](../../../apps/decky/src/task-controller.ts) `dismount` 会取消活动任务；启动 RPC 返回时若 UI 已离开，也发送 cancel；[index.tsx](../../../apps/decky/src/index.tsx) 页面 cleanup 调用 requestCancel。
- 当前任务 kind 是 prepare/install；`TaskManager._run` 只有 install 按 committed/rolled_back 解释终态。新增全链路任务后需要重写这一判断，不能按普通任务 work 返回就宣布成功。
- 单次动作、QAM 切换、关闭进度后继续安装要求重设后台任务生命期；简单将 startInstall 放入前端 prepare 完成回调仍有重挂载、取消竞态和无人观察时自动取消的问题。

## 4. 安装恢复与卸载必须拆开

[install.rs](../../../crates/core/src/install.rs) `apply`/`apply_files` 先持久记录进度与覆盖前内容，再逐文件替换；失败调用 `restore(..., false)`。`uninstall` 调用恢复路径并记录 uninstall_requested；`recover` 为未完成操作使用同一份事实。这些逻辑共享，但用途不同。

可以删除：成功后还原原版的产品能力、长期 backup blob、其用户开关、全局手动清理入口、新产生的 uninstalling/uninstalled 及卸载调用链。

仍需要：未提交安装的短期恢复材料、写前验证、临时文件原子替换、最小崩溃记录、锁和恢复冲突识别。它们的归宿是自动处理失败，正常用户无需选择策略。旧未完成卸载仍须一次性兼容，否则升级即可能丢失恢复路径。

EXE `_space` 预算包含 EXE、ZIP、stage、原内容和目标临时文件；移除长期保留不会自动降低这些安装峰值。

## 5. 之前审阅的两项缺陷如何纳入

依据调研时工作区已有的 EXE review.md（未纳入本 feature 提交）：

- **P1 纯新增补丁 UI 无法卸载**：目标流程删除所有卸载入口，因此不在此 feature 中继续扩展卸载按钮。历史审阅仍保留待修事实，实施完成后注明被新产品范围替代，不能现在标记已修复。
- **P2 prepare 取消时 discard RPC 失败跳过 release_lease**：[exe_installation.py](../../../apps/decky/backend/exe_installation.py) 350 行附近；主入口的取消和 discard 路径也要一起检查。删除 normalized.zip 无法撤销 Core 已持久保存的 stage。新流程须先可靠撤销授权，并在 apply 阶段检查同任务授权/取消，不依赖 discard 请求是否成功；此项是自动安装前置验收。

## 6. 自动适用性与人工同意不能混为一谈

`game_facts` 已能核对语言、安装版别、已安装 DLC 和基线；它明确注明 DLC 安装不是购买权证据。旧 EXE 设计曾写入更广的未购买 DLC 条件，当前 profile 的本体文案则表述为未安装 DLC，二者不是完全相同的事实。

新计划删除确认步骤，保留适用性审核资料并在 P0 明确每档要求的可自动验证证据。不能生成“用户已同意/已声明”的假数据，也不能把 DLC 缺席当成未购买。仅靠现有本地 manifest 不能解决所有来源适用条件；这属于实施前必须明确的审核边界，不是让用户重新勾选的理由。

`exe_profiles.release_id` 使用完整 profile 摘要，文案/schema 修订会改变 ID；旧安装记录迁移须使用载荷、实例和 variant 等稳定事实绑定，不能名称匹配。

## 7. 调研边界

本次未执行测试、下载补丁、安装、卸载、EXE、游戏、远程设备连接或部署。没有声称普通游戏卸载会删除所有额外文件，也不以该假设触发工具对游戏目录的自动清理。

Luna 只读 explorer 完成调用链和删除边界调查，主 Agent 核对关键路径、用户提供聊天与现有文档，负责设计整合。终端普通沙箱启动失败后，只读调研使用 require_escalated 成功访问本地源码；没有扩大到设备或网络写操作。
