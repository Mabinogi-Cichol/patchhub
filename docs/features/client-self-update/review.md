# 自更新实现复核

## 2026-10-28 Windows 单实例增量复核

- 结论：本次实现符合追加需求，无需新增框架或兜底。范围为原生入口、`SingleInstance`、构建清单、真实进程测试和 CI 接线；沿用当前修复分支的此前验证。
- 路径：`wWinMain → 立即获取命名互斥量 → 主实例创建 Flutter/窗口，重复实例还原已有窗口后退出`。零等待获取是判断“已占用”的系统契约，不是任意任务超时；互斥量持有至窗口/引擎析构，内核在异常退出后回收。入口不按参数豁免，因此更新探针不能产生重复窗口。
- 窗口匹配使用本应用专属属性，避免通用 Flutter 类名或相同标题误匹配其他应用。初始化期间已有互斥量但尚无 HWND 时，重复进程退出，由第一个进程完成正常展示；没有为这个短暂阶段添加轮询/重试。还原/激活使用 Win32 API，不等待其他进程消息处理。
- 证据：6 项直接编译生产模块的原生进程/窗口回归、9 项 CI 选择回归、完整原生构建通过；隔离真实 Flutter 重复启动与助手替换/启动/提交链通过。在线 CI、真实 GitHub 跨正式版本和系统安装版验收未执行；本地夹具范围见[施工记录](progress.md)。
- API 契约参考：[CreateMutexW](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw)、[WaitForSingleObject](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitforsingleobject)、[ShowWindowAsync](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindowasync)、[SetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow)。前台激活仍受 Windows 前台窗口规则约束，不强制绕过系统规则。

## 2026-10-28 Windows 修复增量复核

- 结论：未发现本次修复需继续精简的机制。基线 `main@2bf0dda`，范围仅 `fix/windows-self-update` 的 Windows 更新服务、弹窗、PowerShell 助手和对应测试；不替代原双端复核或完整发行验收。
- 从 `AppUpdatePanel.apply → AppUpdate.prepare → prepared.json → AppTasks.shutdown → apply.json → finishExit` 沿实际路径核对：助手新增检查在 prepared 之前拒绝同目录其他 GUI，发起者窗口保持打开并显示错误；准备期间允许 ParentId 与其 Core 工作，未把正常扫描误判为阻塞。退出后仍调用原 `Assert-Idle` 检查全部目标进程，随后才备份/替换；已避免为解决已知占用而强杀其他窗口或盲等固定时长。
- 下载继续沿原附件冻结、流式读取、哈希校验路径执行，只新增阶段文字与计数重置；进度值归一化满足 Flutter 控件契约，不限制合法包大小。TLS 只解释并保留原始错误，重新检查由用户触发，未关闭证书验证或加入自动重试。
- 验证：14 项 Flutter 回归、12 项真实 PowerShell 回归通过，analyze 无问题。双实例回归保持调用者与另一实例均存活，实际证明准备被拒绝；单实例退出后替换/探针握手成功，不同目录实例不阻塞。相同 Dart 客户端的官方 API/清单请求当前均 200；此前 TLS 中断根因未复现。
- 未构建修复包，未操作用户已有窗口或安装目录；原生 Flutter 界面截图、正式跨版本更新与系统安装版完整验收仍待执行。详见[本次修复](fix-plan.md)和[施工记录](progress.md)。

## 2026-10-24 原始功能复核

- 日期：2026-10-24；范围：本功能分支相对 `main@81a2965` 的双端更新路径及打包、安装/卸载和 CI 改动。
- 依据：[设计](design.md)、[施工记录](progress.md) 和 [overengineering-review](../../../.agents/skills/overengineering-review/SKILL.md)。这是主 Agent 的代码路径与设计对照复核，未创建独立审阅 Agent。

## 结论

未发现需要新增通用更新框架、Core 协议或常驻服务的依据。检测、用户确认、下载、退出/卸载、替换、验证、提交或恢复的路径已逐项核对；真实失败和未完成验收没有写成成功。保留的独立进程、互斥、文件清单和最小恢复记录分别对应调用者会被卸载、Windows 文件占用、多文件替换及既有卸载器固定清单等实际约束。

## 复核中处理的问题

- Windows 失败准备阶段原可能清理另一个事务的恢复标记：清理时核对标记绑定的目标和工作目录，并增加真实 PowerShell 回归。
- PowerShell 5.1 默认编码会破坏中文路径恢复记录：所有 JSON 读取显式 UTF-8，成功更新、失败恢复和更新后卸载均在中文路径夹具通过。
- 锁释放后删除载体可能与下一进程获取锁交叉：保留固定空锁文件，卸载时收尾；ZIP/NSIS 不打包该控制文件。
- Decky 来源调度在等待 reload 后可能越过维护门禁：等待完成后再次检查门禁；宿主跟踪在途调用，更新前检查补丁、来源和扫描等工作。
- 新前端挂载早于事务提交会提前扫描：前端等待更新结果，新后端待确认阶段不做业务恢复或自动任务；终端与 GUI 安装结束均释放宿主维护状态。
- Loader 冻结 Python 的默认 CA 和 `sys.executable` 不适用于宿主 HTTPS/外部 Python 脚本：使用实机核实的系统 CA 和系统 `python3`，不关闭 TLS，不另启动 Loader。
- 旧 Linux 缓存二进制虽有 ELF 与 runtime 哈希，仍可能与当前协议生成物不一致：真实部署发现后自动恢复旧插件，重新构建并执行实际 v5 preflight 后部署成功。CI 本身从当前源码构建，不新增一套兼容旧协议的路径。
- 本地 Release 夹具完整链发现 Windows 临时目录与混合分隔符处理、提示区域可见性问题，以及 Decky 下载结果丢弃插件名称的契约错误，均直接修正实际路径并验证。Decky 轮询在进程退出检查后重读终态，避免把刚完成的结果覆盖为失败；不增加额外重试状态。
- Decky 排版修订只保留一个全局设置更新组件，将其移入既有页面容器并沿用 `Field` / 整行按钮；删除 QAM 重复挂载。未引入新配置、公共布局框架或业务状态，类型/构建与实机画面通过。

## 保留边界

- Loader 管理 API 属于版本相关实现；只开放已核对的 3.2.9 / 3.3.0，不将其作为新功能的可配置通用后端。
- 既有 CEF 限制删除被自动审批拒绝，本轮保留。没有借改变执行通道绕过；这部分不计为已完成精简。
- 当前 GitHub latest 与运行候选同为 0.3.0，完整执行链用测试进程内的本地 HTTPS 响应夹具完成，没有发布测试 Release。正式 GitHub 跨版本更新、系统安装版注册表/快捷方式矩阵和实体手柄操作仍需验收。
