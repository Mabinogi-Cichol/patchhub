# 过度设计与路径复核

2026-11-03，基线 main@a83e0b7，对 feature/decky-debug-update 工作区的设计和实际实现复核；使用 [overengineering-review](../../../.agents/skills/overengineering-review/SKILL.md)。本轮由主 Agent 复核，未委派独立审阅。

结论：未发现仍需增加配置框架、常驻服务、重试或时长门禁的依据。保留来源标记/loopback 会话/生产打包拒绝，对应 debug HTTP 不能放宽正式来源这一明确边界；保留产品事务与备份，因为 Loader 会先卸载旧插件。调试 bootstrap/清理仅调用 Loader，不创建旧插件备份，不直接写 root 插件目录。

执行路径：正式包无标记 → 原 GitHub 候选及 HTTPS/冻结附件；debug 包标记 → 独立候选和提醒缓存 → 原生启动 tips → 实际按钮 RPC → 停止新业务/主动取消在途业务与恢复收尾（和下载并行） → 同一独立安装器（来源来自正式插件目录标记，不能从候选/RPC 任意传入）→ SHA-256/包/运行时校验 → 原产品门禁/事务/Loader/恢复 → Steam 重启 → PID/boot/实际页面及 RPC 核验 → Loader 恢复干净当前候选 → 源/缓存/服务/暂存清理。

复核中直接精简/修正：

- 实际日志证明按钮已触发，错误来自启动工作尚未结束；删除试验用可信指针输入辅助脚本，不保留重复点击或输入重试。早期脚本等待忙碌状态的建议已撤回，不能视为产品问题修复。现由点击触发主动取消，并在替换前核对安全终态。
- 卸载后不能依赖插件 RPC/CEF 持续观测，改读既有独立进程结果文件，没有新增产品状态机或恢复层。
- 去掉 release_package 中重复且未使用的 URL 后缀计算。
- 上传失败纳入 finally；尚未 bootstrap 时只清理其唯一暂存，不为了清理执行无关安装。
- 新版加载证明后恢复干净当前输入包，避免虚构高测试版本长期影响正式检测；无需旧版备份或通用版本切换配置。

验证依据与未验收边界见 [progress.md](progress.md)。该结论不替代实体手柄或正式 GitHub 跨版本验收。


## 2026-11-04 主动取消增量复核

沿 start_app_update → AppUpdate.start（独立 worker 下载）→ 关闭业务入口/取消扫描与调度 → TaskManager.shutdown + SourceService.cancel_pending → 原桥接控制行/回滚回执 → recover → prepare_app_update → 原 Loader 安装路径复核。业务调用只用当前 asyncio Task/方法映射跟踪；已删除无差别 _active_host_calls 计数与 busy 字段，没有新增租约、总时限、强杀写入或通用任务框架。

安全必要项：受控写入必须保留原桥接的终态排空；来源取消中出现非取消错误也报告失败；补丁失败/非安全终态及 needs_recovery 阻止安装。读取取消有明确 app_update_cancelled 语义。SourceService 取消不永久关闭服务，取消更新/失败后恢复来源调度；业务入口在取消收尾未结束时仍保持关闭。

本轮修正一个真实超时路径：prepare 可能等待正常长回滚，单次 CEF socket 的请求期限不能代表该任务失败。复用已有 bridge jobs/status 做短请求观察，退出时 forget 本次 job，不另建重试/恢复框架。自动化覆盖 loader.timeout=0 时仍观察未完成回滚，以及终态失败禁止安装。实机只读扫描取消与完整更新链见施工记录，真实游戏写入中取消及实体手柄不冒充已验收。
