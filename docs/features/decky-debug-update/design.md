# Decky debug 本地自更新源

状态：已实现，自动化与真实设备完整 debug 更新链通过；实体手柄待验收。

施工记录：[progress.md](progress.md)。关联：[产品自更新](../client-self-update/design.md)。

## 目标与范围

不发布 GitHub Release，从真实插件启动检查、原生 tips、更新按钮走真实 HTTP 下载、SHA-256 和包校验、Loader 安装及运行验证、Steam 客户端重启和新版加载。复用生产安装器及事务恢复；不修改游戏、来源或安装数据库。

## 设计

仅手工生成的 debug 插件包包含 `debug-update-source.json`，其 base_url 必须为 `http://127.0.0.1:<port>/<session>`。正式 package-plugin 的显式文件清单排除该文件。没有标记时全部沿用 GitHub HTTPS 校验；不能通过候选或 RPC 指定任意更新源。

debug 使用同一 Release 形状的 latest/tags 元数据、固定附件名和 SHA256SUMS，重新核对 Release 与附件身份；仅来源 URL 不同。检查/提醒缓存独立为 app-update-debug.json，执行记录独立缓存目录。安装恢复事务仍使用原产品路径，避免并发安装及破坏恢复。

一键脚本读取当前 Devkit 设备和密钥，要求已可信 known_hosts，上传两个已构建且版本一致的 debug 包（当前版/更高版），设备 loopback 临时 HTTP 服务及 debug 配置通过包携带；先以 Loader 安装当前 debug 包，再由真实页面完成更新。服务、暂存及 debug 标记/缓存在 finally 清理；已验证新版加载后，通过 Loader 恢复输入的干净当前版本，避免虚构的高测试版本影响正式更新。保留本地清单和证据。debug 部署不创建旧版备份，产品自更新自身必要的事务备份保留。

不新增通用服务器、重试或总任务时限。已有连接/HTTP 错误直接报告；实体手柄单列待验收。

## 验收

定向 Python 回归覆盖正式 HTTP 拒绝、debug loopback 约束、候选冻结、哈希失败及状态隔离。WSL 执行 Linux 定向测试；实机记录 tips、下载中百分比、安装结果、Steam PID 与 boot ID、新版 RPC/唯一实例及清理结果。


## 已证实的运行约束

2026-11-04 用户纠正后的产品行为：点击更新即切换到更新流程。宿主关闭新的业务入口、停止启动扫描/调度追加任务，主动请求取消已有补丁与来源任务，并取消在途扫描等只读 RPC。补丁写入不通过强杀中断，沿 TaskManager → Core 取消控制行 → 真实回滚/终态回执收尾；正在进行的恢复或短事务写入等待其安全收尾。SourceService 使用可复用的 cancel_pending，不因一次更新而永久关闭服务。

独立 worker 可立即检查、下载及校验冻结包，不受业务忙碌的拒绝门禁阻断。最终替换前的 prepare_app_update 等待同一取消/恢复任务完成；取消失败、补丁任务未进入安全终态或 recover 仍报告 needs_recovery 时明确失败，不能安装。取消/恢复还在推进时继续等待，不增加总时限、强杀写入、重试或另一套恢复状态机。安装器复用现有桥接 job + status 查询，避免将正常长回滚塞进单次 CEF socket 等待。

此前让脚本等待 busy=false 才点击的方案已撤回，busy 字段和泛化在途调用计数也已删除。脚本会捕获/发起真实无参数 scan() 后立即点击原生更新按钮；设备日志自动断言取消完成早于 Loader 安装。被更新打断的只读请求返回 app_update_cancelled，用户提示为“已停止当前操作，正在更新应用”，不误称为安装取消。失败后保留真实错误，收尾完成后重新开放业务与来源调度。

React 模块执行上下文与实际承载控件的 Steam 页面不同。客户端重启标记从更新控件的 `ownerDocument.defaultView.performance.timeOrigin` 获取，并传至原有 RPC/独立进程；重启指示也比较这个实际页面标记，不能取 SharedJSContext 的全局 performance。

正式 `package-artifacts.py` 明确拒绝 debug 源标记；debug 脚本在正式打包之后生成两个带标记测试 ZIP。上传前后核对完整 ZIP SHA-256，再复用产品冻结附件、校验清单、digest、版本/身份/ELF/runtime 文件和运行验证。测试 HTTP 服务以每 64 KiB 暂停 50 ms 的方式留出真实 UI 渲染中间百分比的时间，此节奏仅在测试服务器，不限制产品下载。

## 一键命令

准备两份由本功能代码正常构建的完整干净包：当前版本及更高的测试版本。测试版本必须从隔离源码修改 workspace 版本后同步 manifests/locks，构建 Linux runtime 并生成对应 SBOM；不能只篡改 ZIP 的 package.json。工作区正式版本不变。命令会在测试完成后恢复传入的干净当前包，因此当前包也必须是希望保留的调试候选。

```powershell
python scripts/decky-debug-update.py --current-package <current.zip> --target-package <higher-version.zip> --known-hosts <trusted-known_hosts>
```

依赖：Python 3.10+（宿主构建工具按仓库版本要求）、Node 22+、OpenSSH、运行中的 Devkit 客户端和设备 CEF。默认 Devkit loopback 32010、CEF 本地转发 18081；端口有冲突时传对应参数。设备、登录用户、密钥及 home 均在每轮读取，known_hosts 必须已经可信；脚本不自动采纳新主机密钥。CEF 复用设备现有 Decky 连接，不新建 Loader `/ws`。

每轮证据输出到 `.artifacts/debug-update-<time>-<id>/`，包含输入哈希、Release 元数据/清单、原生 tips/就绪/下载/新版截图、进度、前后 RPC、清理及构建证据。失败保留原始错误；若产品事务尚需恢复或 worker 仍在执行，禁止覆盖插件/删除恢复材料，暂存保留并报告实际未清理状态。没有安装发生的上传失败只清理本轮暂存。
