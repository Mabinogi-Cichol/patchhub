# P0 行为真值表：动作与安装终态

设计：[design.md](design.md)。施工记录：[progress.md](progress.md)。本文件区分当前源码事实与目标决策；wire 字段及 blocker 枚举以 P0 契约文件（由配套任务冻结） 为准。

## 判定边界

- 可发起仅表示当前快照可提供操作入口，不是写入授权。
- 授权必须在执行时用新鲜 source revision、签名/撤销、文件 facts、实例/平台能力、租约和事务互斥重新核验。旧 actions/UI enabled 不能代替授权。
- 检测是文件及收据事实；superseded 不是当前安装。自动路线为 signed release 或 publisher selection 网络获取；手动路线仅本地 signed ZIP。

## 当前事实与目标决策

| 行为 | 当前源码事实 | P0 目标决策 |
| --- | --- | --- |
| signed catalog | CLI 验签；UI 仅对 managed ZIP 要求 signed 且 ZIP link available。 | 自动 managed ZIP 必须通过签名/有效期/撤销、release/实例条件、无 blocker 与 ZIP link。 |
| unsigned catalog | publisher ZIP/EXE 检查 selection、下载、格式 link，不要求 catalogSigned。 | publisher 路线独立准入，不冒充 signed；unsigned managed ZIP 禁止自动安装。 |
| publisher ZIP | resolver 生成 zip-selection，执行时重 resolve；UI 检查下载与 ZIP link。 | 当前 selection、source 准入、非 stale、link、ZIP facts、平台和安装条件均通过。 |
| publisher EXE | exe-selection；orchestrator 下载分析并重核游戏 facts/包条件；UI 检查 EXE link。 | 可发起不等于授权；执行时重核身份、完整性、profile 与目标 facts。 |
| 本地 ZIP | Windows 限 managed ZIP；Decky 要求 managed ZIP、无 blocker、signed catalog、ZIP link；CLI 禁止 selection+archive。 | 仅 signed release。本地输入可绕过远端 artifact 缺失/下载不可用，但仍要求本机 ZIP 安装能力与非特权身份可用（当前 `links.zip.available` 包含此 gate）；还须通过签名、撤销、release/目标条件、archive 结构/链接安全和授权。不可用于 publisher ZIP/EXE。 |
| source disabled / 未订阅 | CLI 输出 source_not_enabled/source_not_subscribed；resolver 重查。 | 只阻断依赖该来源的 publisher 自动动作；独立 signed 本地路线单独判断。 |
| selection stale/missing | resolver 返回 selection_stale/selection_not_found。 | 不可发起自动安装或 fallback；刷新后重新评估新 selection。 |
| ZIP/EXE link | UI 分开检查 links.zip 和 links.exe；EXE facts 独立。 | ZIP link/身份同时约束自动与本地 signed ZIP；EXE link 约束 EXE 路线。可用不代表其他条件/授权通过。 |
| 身份优先 | signed release 冲突时优先；ZIP/EXE selection/content identity 分离。 | 保持 signed 优先，不互冒身份。 |
| 主按钮 | 两端都发送 operation=overwrite；按 committed/detection 证据显示“安装补丁”/“覆盖安装补丁”。 | 主操作恒 overwrite；无已装/部分证据 label=install，有 managed/external/partial 证据 label=overwrite。 |
| Install | core 要求目标符合 recipe baseline，偏离时拒绝覆盖。 | 保留为基线保护型首次安装 API；主按钮不发送它。 |
| Reinstall | core 要求同实例、同内容完整 committed receipt 且文件匹配。 | 只重装仍完整匹配的相同受管内容，不接管外部文件或替换不同内容。 |
| Overwrite | core 可覆盖 changed/unowned targets，仍有路径/计划/facts 约束；提交后旧 receipt superseded。 | 保留为明确接受替换当前内容的操作，并完整重核边界。 |
| 检测/事务 | 有 committed、superseded、rolled_back、needs_recovery；检测可 partial/conflict/unknown。 | committed 可作为当前证据；superseded 仅历史；rollback 非已安装；recovery 明确阻断。missing/mismatch/unknown 分开。 |

## 动作可用性矩阵

可用只表示可进入启动流程；任一必要条件失败则 unavailable 并带结构化 blocker。选择、可见性、busy 属宿主状态。

| 输入/状态 | automatic | local_archive | reinstall | overwrite | 主按钮 |
| --- | --- | --- | --- | --- | --- |
| signed managed ZIP 有效、ZIP link 可用、无 receipt | 是 | signed 输入单独评估 | 否 | 是 | overwrite/install |
| signed ZIP 远端 artifact 缺失/下载不可用，ZIP 安装能力仍可用 | 否 | 选本地 ZIP 后评估 | 按 receipt 判断 | 独立判断 | 有本地输入才显示，label 按证据 |
| unsigned managed ZIP | 否 | 否 | 否 | 否 | 禁用 |
| publisher ZIP 有效 selection、source 启用/订阅、ZIP link 可用 | 是 | 否 | 仅匹配 receipt | 条件通过可用 | overwrite，label 按证据 |
| publisher EXE 同上且 EXE link 可用 | 可发起，包条件执行时重核 | 否 | 仅完整受管证据 | 执行时重核条件 | overwrite，label 按证据 |
| stale/missing selection、source disabled/unsubscribed、对应 link 不可用 | 否并列 blocker | signed 路线独立判断；ZIP capability/host identity blocker 时也不可用 | 按证据与准入 | target 无法 resolve 则否 | 不发起自动路线 |
| 已选本地 signed ZIP，远端 artifact 缺失/下载不可用且本机 ZIP 能力可用 | 否 | 是 | 按 receipt | 是 | overwrite，label 按证据 |
| 同内容 committed 且文件匹配 | 准入通过时可用 | 路线允许时 | 是 | 是 | overwrite/overwrite |
| 外部或 partial/conflict | 独立判断 | 仅 signed 路线 | 无受管匹配时否 | facts/授权通过后可用 | overwrite/overwrite |
| 当前实例 needs_recovery、不安全路径或实例不可用 | 四种动作阻断 | 阻断 | 阻断 | 阻断 | 禁用并说明原因 |
| 检测 unknown（例如动态初装无 committed plan） | 按路线准入判断，不凭 unknown 禁用 | 仅 signed 路线，按准入 | 无完整受管证据时否 | 准入通过可用 | 标签按收据/可靠文件证据 |
| 活动任务或执行必需 facts 读取失败 | busy 是宿主交互；核心锁/新 facts 强制拒绝执行 | 相同重核 | 相同重核 | 相同重核 | 不把快照当授权 |

下列是现有准入/执行诊断例，不能全部直接作为封闭 ActionBlockerCode：source_not_enabled、source_not_subscribed、selection_stale、selection_not_found、invalid_selection_id、archive_not_allowed_for_selection、managed_install_exists_or_needs_recovery、reinstall_unowned_or_incomplete、reinstall_content_identity_changed、baseline_conflict、zip_link_or_special_file、linked_target、exe_conditions_not_satisfied、needs_recovery。Decky 当前仅对 rolled_back 状态且 error 精确等于 cancelled/task_cancelled/lease_expired 映射 cancelled，目标仍改为 typed cause。P0 用 typed ActionBlockerCode 收敛；确切字段及集合见 [契约 §5](p0-contract.md#5-权威动作-dto)。baseline_conflict/path/facts 等执行错误保留为 DomainError，不凭名称追加到动作 enum。detail 只解释，不参与授权/终态推断。

## 安装结果与任务终态

依结构化 transaction/cause 判定，禁止检查错误文本是否包含 cancel/lease。transaction.state 是领域事实，disposition 是宿主终态（completed/cancelled/failed）；needs_recovery 保留为事务状态，并以 recovery.required=true 明确恢复需求。

| 结果/事件 | transaction.state | disposition | cause/recovery | cleanup |
| --- | --- | --- | --- | --- |
| 已提交，即使随后 cancel/lease | committed | completed | recovery=null | 独立结果 |
| 用户取消且事务回滚 | rolled_back | cancelled | typed UserCancelled | 不删恢复材料 |
| 租约过期且事务回滚 | rolled_back | cancelled | typed LeaseExpired | 不删恢复材料 |
| 普通 worker/网络/计划/应用失败 | rolled_back 或无事务 | failed | DomainFailure | 独立结果 |
| 无法保证安全提交/完整回滚 | needs_recovery | failed | recovery required | 保留 stage/backup |
| revoked/信任失效/条件拒绝 | 无事务或真实回滚态 | failed | DomainFailure，不是取消 | 独立结果 |
| CLI/worker/protocol/EOF/版本失败，无领域 outcome | 无领域事务 | 宿主 task failed；显式用户取消事件可 cancelled | typed TransportError/宿主事件 | 不造领域 outcome |
| committed 后 cleanup 失败 | committed | completed | recovery=null | 保留真实 status（partial/failed/busy）、performed、removed_bytes 与 failures；可重试 |
| 新事务替代旧收据 | 新 committed，旧 superseded | 新事务 completed | identity 分别保留 | 分开记录 |

Outcome 包含事务事实、identity、Completed/Cancelled/Failed、typed cause、recovery 和独立 cleanup。core 单点映射；无领域 outcome 的传输错误由宿主记录。

## 固定验收样本

| ID | 输入 | 动作预期 | 执行/终态断言 |
| --- | --- | --- | --- |
| A1 | signed ZIP 有效、link 可用、无 receipt | automatic/local 可用；primary overwrite/install | 首次提交 completed |
| A2 | unsigned managed ZIP | automatic/local false，有 trust blocker | core 写入前拒绝 |
| A3 | publisher ZIP 有效 selection、启用/订阅、ZIP link | automatic true，即使 catalog unsigned | revision 改变后旧 actions 不授权 |
| A4 | EXE selection/link 有效、包条件未检查 | 可发起；ZIP link 独立 | 条件失败不写入 |
| A5/A6 | source disabled/需订阅但未订阅 | automatic false，对应 blocker | direct API 同样拒绝 |
| A7 | stale/not found selection | automatic false，理由区分 | 不 fallback |
| A8 | signed 本地 ZIP、远端 artifact 缺失/下载不可用，本机 ZIP 能力可用 | automatic false、local true | 签名/结构/目标失败无写入 |
| A9 | selection 携带 archive | invalid | archive_not_allowed_for_selection |
| A10 | 匹配 committed receipt，reinstall | reinstall true；主操作 overwrite/overwrite | 成功遵循 superseded 规则 |
| A11 | reinstall 无 receipt/文件不匹配 | reinstall false | 拒绝不写入 |
| A12 | 外部/partial 明确 overwrite，facts 通过 | overwrite 可用，label overwrite | 成功替代或报真实 rollback/recovery |
| A13 | Install 面对偏离 baseline 字节 | 不作为主按钮 | baseline_conflict |
| T1 | committed 后收到 cancel | — | completed，不变 cancelled |
| T2/T3 | typed UserCancelled/LeaseExpired + rolled_back | — | cancelled，保留具体原因 |
| T4 | DomainFailure rolled_back，错误文本含 cancel | — | failed，不字符串推断 |
| T5 | needs_recovery | — | failed + recovery.required=true，保留恢复材料 |
| T6 | transport failure，无 transaction | — | 宿主失败，不造事务 |
| T7 | committed + cleanup error | — | completed，cleanup 独立可重试 |
| T8 | overwrite 新提交 | — | 新 completed，旧 superseded |

## 历史语义与修订

保留 signed 优先、身份分离、ZIP/EXE 各自链路、来源状态检查、Install/Reinstall/Overwrite 语义、主按钮恒 overwrite、取消回滚、commit 优先、superseded、recovery/cleanup 分离。

有意修订事实所有者：UI 当前仍调用 patchInstallable/canAutoInstall，Windows 拼 auto_installable/reinstall_available；Decky tasks.py 依错误字符串映射取消，main.py 另跑 cleanup 并合并结果。目标改由 core actions/outcome 单点提供，宿主消费结构化终态，不改变上述可观察行为。

## 静态证据

- packages/ui/src/index.ts：managed/publisher trust/link 条件及旧派生字段。
- apps/windows/src/main.tsx：committed/detection 标签、主操作 overwrite、本地 archive 限制。
- apps/decky/src/index.tsx：动态 projection、本地输入与 overwrite 请求。
- crates/cli/src/runtime.rs：source blockers、selection/release 互斥、archive 禁止与 selection 混用、transaction state。
- crates/core/src/install.rs：三 operation 的 baseline/receipt/overwrite 语义及 superseded。
- crates/core/src/installer/exe_orchestrator.rs：EXE facts、reinstall evidence、条件失败。
- apps/decky/backend/tasks.py：commit 优先及当前字符串 cancellation 映射。
- apps/decky/main.py：commit 后 cleanup 与结果合并。
- core/CLI install tests 与 resolve/orchestrator/EXE fixtures：rollback、recovery、superseded、overwrite、stale、signed 优先、ZIP/EXE。

本次只做文档与静态源码核对；未运行测试或协议冒烟，固定样本待契约 fixture/实现验证。


## P2 终态补充（2026-10-12）

事务 writer 的固定非空 `; cleanup_pending:<诊断>` 后缀不改变原终态。先分离固定语法，取消只依据完整 `cancelled` / `task_cancelled` / `lease_expired` 机器码；普通失败保留原机器码。cause.detail 保留有界清理诊断，Transaction.error 保留完整事实，任意其他自由详情不能证明取消。应用回滚或 planned 丢弃已经确认 rolled_back 后，清理失败仍返回实际事务，并用相同原因重放；commit 仍 completed，needs_recovery 仍 failed。此为 [契约 §4](p0-contract.md) 的 P2 受限修订，运行证据见 [P2 验证](p2-validation.md)。
