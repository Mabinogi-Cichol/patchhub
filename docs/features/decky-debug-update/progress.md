# Decky debug 更新施工记录

设计：[design.md](design.md)。审阅：[review.md](review.md)。关联：[产品自更新记录](../client-self-update/progress.md)。

## 状态

2026-11-03：已实现；定向自动化、构建和真实设备完整更新链通过，实体手柄待验收。基线 main@a83e0b7，分支 feature/decky-debug-update。未提交、合并、推送或创建 Release。工作区正式版本仍为 0.5.3。

## 2026-11-04 用户纠正后的主动取消（当前结论）

- 用户点击更新即停止接收新业务，主动取消补丁/来源任务、在途扫描与自动调度；不要求用户手动取消、自然等待完成或因忙碌禁用按钮。此前等待 busy=false 的方案及“已修复”结论撤回。
- 独立 worker 立即下载、校验，宿主并行执行原取消控制行/回滚终态排空、来源取消、在途只读调用取消和必要 recover。最终 prepare 只在安全收尾完成后放行；任务失败、非安全终态、来源非取消错误或 needs_recovery 明确阻止替换。
- 来源取消不 close 服务；失败或取消更新后等待收尾结束，再恢复业务入口与原来源调度。已进入的恢复/短事务写入等待安全收尾，不强杀写入。
- 删除 busy 状态及无差别在途调用计数，只跟踪真实业务 asyncio Task 和方法。被更新打断的只读调用使用 app_update_cancelled 提示，避免显示“取消安装”。
- 复核发现原单次 CEF 请求期限会误判正常长回滚；最终准备改用既有 bridge jobs/status 的短请求观察，没有新增总时限、重试或恢复框架。

### 新增/重跑验证

- WSL Ubuntu 使用仓库 `.tools/python-linux312/python/bin/python3.12`（CI 指定 3.12）：`test_tasks.py` 8 项、`test_source_service.py` 21 项、`test_app_update*.py` 15 项、`test_install_decky.py` 41 项全部通过。旧系统 Python 3.10 的原有 ContextVar setUp 夹具曾超时并在清理等待，已停止该两次隔离测试；不计通过，未操作设备业务进程。
- 新维护测试使用真实 TaskManager、CoreBridge 和受控协议进程夹具：点击时独立 worker 已启动；控制行是 cancel；回滚回执未到时 prepare 不完成、recover 未提前执行；写入提交赢得取消竞态由原契约处理；来源取消失败及 needs_recovery 阻止替换。
- 新安装器测试：即使 loader.timeout=0 也持续观察未完成任务；终态错误直接失败并 forget 本次 job。没有以固定阈值终止正常回滚。
- Decky 111 项、typecheck、构建/打包通过；上传失败边界测试通过。正式包 debug 标记拒绝测试沿用此前 12 项记录；未运行发布全矩阵/线上 CI。

### 当前实机证据

最终：`.artifacts/debug-update-20261104-000029-c2e3a9d8/`。前两次主动取消链：`.artifacts/debug-update-20261103-235310-fb3d0cd6/`、`.artifacts/debug-update-20261103-235641-165b6c6c/`。

脚本不再等待空闲；点击前已核对实际 Backend.runningCalls 存在 scan。`cancellation.log` 自动核验：00:00:47 `app_update_cancel_started rpc=['scan']`，同刻 scan 返回 app_update_cancelled；00:00:48 `app_update_cancel_completed`；00:00:55 才开始 Loader 安装 0.5.4。预期取消的 scan 在 Loader 中以异常返回，不是自更新启动失败。

完整原生 tips → 按钮 → HTTP 下载/校验 → 原事务安装/运行核验 → Steam 重启 → 新版页面链通过。Steam PID 72231 → 74654，boot ID 不变；控件页面标记与 restart_context 一致。目标 debug SHA-256 `sha256:65c5441a0c3aeda80c939a5d7ddf412119e284f34dbdfdac60ded58ff30ca7f5`，28 个文件，bundle `ae6d5f782758f4cf9c836855ff81e93dc7706630e2a42d8e47b5a9fc57a8f117`。任务 45→45；事务 25→25、返回内容相同；新版唯一实例。

成功后 Loader 恢复干净 0.5.3，服务/隧道、debug 源/检查缓存/worker、本轮远端暂存及本地隔离源码/测试包清理；清单、输入哈希、RPC、日志、截图、build-provenance.json 和 summary.json 保留。正式 clean 包 SHA-256 `352a3022d14360838e38d56204619fc3f6160a4bc237d86864179fd8a1984885`。实体手柄及真实游戏写入中触发更新未执行；本轮实机仅在途扫描主动取消，写入/回滚由隔离协议夹具验证。

## 实际修改

- debug 包携带独立 loopback 会话源标记，检查器和独立安装进程只替换候选/附件来源；生产 GitHub HTTPS 身份校验保留。正式打包明确拒绝标记文件。
- debug 检查/提醒及独立 worker 结果与生产缓存隔离；原有安装事务互斥、备份、失败恢复、校验和 Loader 验证未替换。
- `scripts/decky-debug-update.py` + `decky_debug_update_device.py` 封装 Devkit 配对读取、严格 SSH 主机校验、ZIP 上传哈希、设备 loopback HTTP 服务、两个 debug 包生成、无旧版备份的 Loader 调试部署、原生 tips/页面点击、真实进度记录、客户端重启、新版加载及清理。
- 新版验证后，通过 Loader 恢复输入的干净当前包，清理 debug 源/检查缓存/worker、服务、隧道与本轮远端暂存；本地保留清单、RPC、JSON、日志与截图，临时源码副本/HTTP 文件树/测试 ZIP 清理。
- 控件所属页面的 timeOrigin 经 RPC 传给重启进程；修正 SharedJSContext 全局 performance 不代表实际页面的问题。

## 真实失败与修正

1. 首轮脚本点击后立即读取尚未生成的 worker 文件，误判异步 RPC 失败；改为等待真实状态文件生成。
2. 23:14:44、23:15:57、23:19:37 三次 `start_app_update` 被原有维护门禁拒绝，错误为“请先完成或取消补丁、来源与扫描任务，再更新插件”，对应实机多条错误 tips。按钮实际上已触发，先前关于输入事件未触发的判断撤回；未保留额外指针事件/重复点击框架。当时状态 RPC 暴露 busy 并让脚本等待空闲，23:24 后的轮次避开了错误；用户随后明确撤回该方向，因此不能视为产品问题修复。busy 字段/计数及脚本等待现已删除，改为主动取消；下述后续记录为当前结论。
3. 更新卸载/Steam 重启时 CEF 正常断开，脚本通过 RPC 观测会丢失连接；改为 SSH 读取独立 worker 状态文件跨越卸载，随后等待真实 Steam PID 变化及 CEF/新版页面恢复。
4. 第二轮失败保留的远端暂存在补取运行证据、Loader 清理源标记后已按绝对路径核验删除；未动其他轮次历史资料。

## 初始入口阶段定向验证（历史记录）

未执行发布完整矩阵，没有 tag/Release/推送。

| 范围/平台 | 命令或证据 | 结果 |
| --- | --- | --- |
| 更新服务/独立进程，WSL Ubuntu Python 3.10 | `python3 -m unittest discover -s apps/decky/backend -p "test_app_update*.py" -q` | 14 项通过，包括维护忙碌状态、debug 缓存隔离及独立进程存活 |
| 安装器，WSL Ubuntu | `python3 -m unittest discover -s scripts -p test_install_decky.py -q` | 39 项通过；真实 loopback HTTP 字节进度、正式 HTTP 拒绝、debug 会话/附件冻结、SHA-256 错误及原安装恢复回归 |
| 正式打包，Windows Python 3.12 | `python -m unittest discover -s scripts -p test_package_artifacts.py -q` | 12 项通过，正式包拒绝 debug 源标记 |
| 一键脚本上传失败边界，Windows | `python -m unittest discover -s scripts -p test_decky_debug_update.py -v` | 1 项通过；未安装时只清理本轮暂存，严格 SSH 主机校验 |
| Decky UI，Windows Node | `npm test -w apps/decky` / `npm run typecheck -w apps/decky` | 111 项通过、类型检查通过；模拟 SharedJSContext=123、实际页面=456，RPC 和重启指示使用 456 |
| 当前候选构建/打包 | `npm run package:plugin -w apps/decky` / `python scripts/package-artifacts.py --platform decky` | 通过，复用未修改且已验证的 0.5.3 Linux runtime；最终 clean ZIP SHA-256 `685cc29313f87e3a4617b8e3b2fb32feeb6a3e64a2f08651ec323ee49db3b9b3` |
| 更高测试版本，WSL Rust 1.94.0 | 隔离源码同步 workspace/manifests/locks 到 0.5.4；`cargo build --locked --release -p patchhub-cli -p patchhub-installer-worker --bin patchhub-core --bin patchhub-worker`；`cargo metadata --locked --format-version 1`，生成 SBOM 后打包 | 通过；SBOM/包/运行版本均 0.5.4。主工作区未升版本，原 0.5.3 Linux 二进制已逐字节恢复 |
| 结构检查 | Python py_compile、`git diff --check` | 通过 |

## 完整设备链证据

设备每轮重新核对 Devkit 选中 steamdeck、当前地址/配对密钥及已有可信主机记录。Loader 3.3.1，服务 active。最终完整自动命令一轮无需人工补点；DevTools DOM 点击是实机操作证据，不等于实体手柄。

最终证据：`.artifacts/debug-update-20261103-233320-23280798/`；此前独立自动通过：`.artifacts/debug-update-20261103-232438-4bb78246/`、`.artifacts/debug-update-20261103-232641-e95a98cd/`。最终新版截图等待扫描结束并滚动到实际版本行后获取。

- tips 实际显示“发现新版 v0.5.4”，点击进入真实“缓存、备份与更新”，从当前 0.5.3 点击更新。
- 真实 HTTP ZIP 7,909,723 bytes，中间百分比观测 10/21/31/42/53/74；后续 backing_up 85、installing 90、verifying 95、completed 100。下载进度截图可见单行真实下载百分比，不是合成进度。
- debug ZIP SHA-256 `5085a04b8ab84cb2f973187fa67f5ce05e1208ac868dec451db1b7ccd1a5d3d0`，28 个目标文件；实际 bundle SHA-256 `926096bae7338dd5de0b9ef5ba4f957344d0c062855825f32b91bfefc4051333`。沿产品安装器完成清单/digest/版本/runtime ELF 校验，正式文件与实际 HTTP bundle 校验、后端/前端唯一实例及 RPC 通过。
- Steam PID 64577 → 67060；系统 boot ID 不变。实际页面标记 1793719646002.1 与 worker restart_context 完全相同；新版页面标记 1793720046015.9，显示当前版本 0.5.4。
- 实际 `scan()`、`list_tasks`、`list_transactions` 正常；任务 45→45，无活动任务；事务 25→25，完整事务返回内容相同。没有执行游戏补丁安装/卸载或清理游戏/来源数据库。
- 新版验证后 Loader 安装同一功能的干净 0.5.3 输入包；最终插件 0.5.3、唯一实例且 Loader active。debug 标记/检查缓存/worker、服务/隧道和所有本轮远端 debug-update 暂存已清理，无旧版插件备份保留。产品更新期间必要的事务备份由产品流程正常生成并清理。

截图：`native-tips.png`、`update-ready.png`、`http-progress.png`、`new-version.png`。`summary.json`、`build-provenance.json`、`source.json`、`progress.json`、`baseline.json`、`after.json`、`clean.json`、`cleanup.json` 保留摘要与清单。

## 待验收

- 实体手柄焦点、A/B 返回和手柄点击更新未执行。
- 正式 GitHub 跨发行版本更新仍未执行，本轮 debug 链不能代替正式来源验收。
- 线上 CI 未推送/未执行；不把本地及 WSL 通过写成线上通过。

最终只读复核 `final-device.json`：插件 0.5.3、Loader active，debug 源/worker/检查缓存均不存在，测试服务与远端暂存目录为空。`loader-update-errors.log` 保留三次原始门禁失败；后续完整轮次没有新增 PatchHub 方法失败。
