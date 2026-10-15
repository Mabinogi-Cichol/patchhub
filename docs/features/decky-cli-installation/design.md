# Decky 终端安装与更新设计

- 状态：终端安装器、README 短链入口及隔离自动化已实现，短链已创建并核对目标；Steam Deck 实机验证与安装器 Release 附件发布待执行。
- 施工记录：[progress.md](progress.md)。

## 目标与范围

用户已确认需求是在 Steam Deck 终端安装或更新本 Decky 插件，不是通过 CLI 给游戏安装补丁。提供项目自己的安装入口，复用 GitHub Release 中的 `steam-patch-manager-decky.zip`；不要求用户安装 Node.js、Rust 或克隆仓库。前提是已安装并启动 Steam 与 Decky Loader。

核心要求：安装器完成退出时不在用户设备上留下本次下载包、旧版备份、回滚包、暂存文件、临时日志或恢复标记。成功时保留可用新版，失败或结果未确认时自动恢复安装前状态并清理，不把恢复与清理常规转交用户手动处理。

## 用户入口

支持安装最新正式版、指定版本、更新现有插件，以及只下载和检查而不安装的模式。终端确认展示目标版本与现有版本，完整无人交互模式必须由用户显式选择。

用户在 Steam Deck 桌面模式打开 Konsole，从 GitHub README 复制一行命令，即可下载并执行 [`scripts/install-decky.py`](../../../scripts/install-decky.py)。默认入口不要求用户手动保存脚本、选择目录或克隆仓库；安装/更新使用同一行命令。

主 README 将两种交付方式分节说明：命令行在线安装自动下载并校验正式 Release；ZIP 离线安装由用户预先下载完整 `steam-patch-manager-decky.zip`、传到 Steam Deck，并通过 Decky 的本地文件入口安装。离线流程不调用在线安装器；提前保存 Python 安装器文件仍属于在线下载安装方式。两种流程共用 Decky 已安装和更新前结束活动任务的前提，安装后的进入插件步骤集中说明。

README 命令为 `curl -fsSL https://ba.sh/tz4y | bash`，采用用户给出的 [DeckyClash 在线安装入口](https://github.com/chenx-dust/DeckyClash/blob/main/README_CN.md) 的短链管道形式。`https://ba.sh/tz4y` 已通过 ba.sh 服务实际创建，目标为本仓库 `main` 分支的 [`install.sh`](../../../install.sh)；不使用短链时，客户端文档保留直达 GitHub 的同等入口。

`install.sh` 将下载与参数转发细节移入脚本：完整下载 Python 源码到内存变量，成功且非空后调用 `python3 -c`。源码下载失败（包括已收到部分源码）时退出，不执行不完整 Python。引导脚本的操作全部位于函数体，最后一行才调用该函数；未下载到调用行的截断引导脚本不会执行安装。外层采用普通 shell 管道，与参考项目一致：curl 失败会输出错误，但不承诺外层管道的退出码等于 curl 退出码；安装器和内层下载的非零退出码会由最终 Bash 返回。

追加参数使用 `curl -fsSL https://ba.sh/tz4y | bash -s -- --check` 等形式；管道占用 stdin 时，Python 安装器沿用已有 `/dev/tty` 确认逻辑。没有安装器临时文件，因此不增加脚本清理/中断残留问题；执行流程仍依赖系统 Bash、curl 和 Python 3.10+ 标准库，不依赖 Node.js、Rust、pip 包或仓库其他文件。普通用户以 Steam 用户运行，不使用 sudo。

源码入口跟随本仓库 `main` 分支，无需等待新 Release 增加安装器附件；插件 ZIP 继续从冻结版本的正式 Release 下载并校验。CI 仍在下次 Release 将安装器作为独立 `install-decky.py` 附件并纳入 `SHA256SUMS.txt`，供需要手动保存文件的用户使用。具体一行命令与追加参数用法见[客户端说明](../../../apps/decky/README.md#终端安装器开发版)。桌面模式真实端到端安装仍待验收。

参数为 `--version vX.Y.Z`（省略时取 latest 正式版）、`--check`（只下载与检查）、`--yes`（显式无人交互）、`--recover`（只续做上次收尾）、`--cef-port` 与 `--homebrew`。交互默认拒绝；脚本经 stdin 输入时从 `/dev/tty` 获取人工确认，没有终端则必须显式指定 `--yes`。

## 当前实现与兼容范围

- Loader 适配固定为 **3.2.9 / v3.2.9**，依据该 Tag 的 `browser.py`、`wsrouter.ts`、`plugin-loader.tsx` 与 `localplatformlinux.py` 核对，并在运行中检查路由对象和确认处理器形状。未经验证的版本明确拒绝。
- 使用 Python 标准库的本机 CDP WebSocket 客户端，连接 `127.0.0.1:8080` 的唯一 `SharedJSContext`，调用 `window.DeckyBackend`。不读取或导出 token，不创建 Loader `/ws`，不自动开启 CEF、重启 Steam 或停止 Loader。
- 安装目录通过 `plugin_loader` 的 systemd 配置核对：支持 `PRIVILEGED_PATH` / `UNPRIVILEGED_PATH` / `PLUGIN_PATH` 与标准 `service/PluginLoader` 布局。存在额外 `EnvironmentFiles` 或无法解析的布局时拒绝；`--homebrew` 只能与发现结果一致，不能指定任意写入目录。
- 已安装插件必须处于启用状态，并通过真实 `list_tasks` 确认所有任务终态。**禁用状态的升级暂不支持**：无法读取实际运行任务时拒绝，用户需先在 Decky 启用。此项仍属待解决范围，不能宣称已完成禁用插件验收。
- 在匹配本次 name/version/hash/installType 时只抑制本次 Loader 弹窗，终端确认后使用事件提供的 request ID；不相关事件仍转交既有界面。请求退出时恢复原处理器。自安装前检查至新版验收/旧版恢复完成，暂时拦截现有连接中本插件的新写 RPC，拒绝已有未完成写 RPC，并在确认前再次读取任务；不改其他插件的 RPC。保留中的事务保护能证明原已空闲插件未接受新写调用，因此新版启动失败、RPC 不可用时仍可恢复旧版；上下文丢失则回到严格的任务与写入状态检查。
- 新版验收检查正式文件清单、Linux 执行权限、Loader 后端版本、唯一前端实例及其版本/有效挂载、真实 `get_capabilities` / `list_tasks` RPC 和实际 HTTP 前端字节哈希。不同版本仍需实机证据验证，模拟测试不代替实际挂载结果。
- 当前适配检查 Loader 返回的后端版本和真实 RPC；相同版本重复安装时尚无后端进程内构建指纹，不能据此宣称已证明不同构建的进程身份。这是实机验收需要特别核对的边界。

## 恢复记录与磁盘范围

事务目录固定为 Steam 用户 `~/.cache/steam-patch-decky-install/`，每次事务使用随机 Loader job ID，目录所有者必须为当前用户且权限为 0700。使用缓存父目录 inode 的非阻塞 `flock` 防止多个终端安装器并发，不创建残留锁文件。目录只允许 `package.zip`、`rollback.zip`、`journal.json` 与原子写入临时 `journal.tmp`。

`journal.json` 在每次 Loader 写操作前持久化 pending job ID、Loader boot/PID/starttime、原版本/清单/包哈希、目标版本/清单/包哈希、插件设置和阶段。回滚包、恢复记录及目录执行 fsync。阶段为 `prepared → installing → complete`，失败为 `installing → restoring → restored`；只有 `prepared`（尚未改动）、`complete`（新版已验证）和 `restored`（原状态已验证）允许直接续做清理。

运行中 JS job 保留 Promise 结果，结合 Loader `runningCalls` 检查活动写入。Python 响应丢失后先重连查询，不把超时当作结束。若 JS 上下文已丢失而 Loader 进程未变，无法证明旧写入已停止，会保留最少材料并明确报未完成；Loader 进程已更换且无活动写入后才允许续做恢复。安装器不会为绕过此边界擅自重启 Loader。

确认原安装已经停止后删除不再使用的目标 ZIP，仅保留恢复记录和唯一旧版 ZIP；恢复失败时继续保留，下一次优先恢复。成功/恢复通过后清理有界重试，再核对目录不存在。清理失败的输出区分“新版已验证”与“安装前状态已恢复”，不误报全部完成。

## 安装流程与技术边界

1. 从本仓库 Release 获取明确版本及 ZIP、校验清单，冻结实际版本，避免查询 latest 与下载之间发生版本切换。
2. 校验公开文件名、SHA-256、ZIP 路径与插件身份、版本、runtime 清单、Linux ELF 和执行权限。拒绝异常包，不把任意 ZIP 安装到插件目录。
3. 发现并核对当前 Decky Loader 安装入口。优先复用现有 Steam/Loader 连接，不能另建 Loader WebSocket 抢占现有前端连接；具体桥接方式与运行时依赖仍需实现验证。
4. 已安装插件时读取实际任务，存在安装、下载、回滚等非终态任务或状态无法确认时拒绝升级；保存旧插件、可安装回滚包与本插件 Loader 设置。
5. 调用 Loader 正常安装流程，不裸解压覆盖正式目录，不手动结束插件后端或停止整个 Loader 服务。确认事件必须属于本次版本、插件名与包哈希，不能猜 request ID。
6. 安装后核对正式文件、实际运行版本与插件实例及 RPC；仅安装接口返回不算成功。保留外部状态与其他插件设置，必要时只恢复本插件原设置成员关系。
7. 正式文件、重载后的实际版本、插件实例和 RPC 检查全部通过后，自动删除本次旧插件备份、回滚包、下载副本、暂存文件、临时日志及恢复标记，并核对本次工作目录已消失，才报告全部完成。
8. 安装失败、运行检查未通过或结果未确认时，自动进入恢复流程。先确认 Loader 不再执行本次安装，防止安装与回滚同时写入；再通过 Loader 恢复旧插件及本插件原设置，核对旧版文件、运行版本及应有的启用/禁用状态。首次安装没有旧版时恢复为安装前未安装的状态。恢复验证通过后，自动清理本次全部临时材料，报告安装失败但已恢复、已清理，不默认保留失败备份。

## 中断恢复与清理边界

- 安装前写入最小恢复记录，绑定本次唯一工作目录、原状态、包哈希和当前阶段。普通异常、取消及可捕获的退出信号均进入自动恢复/清理；下载或预检失败尚未改变插件时直接清理即可。
- 超时或响应丢失不等于安装已结束，必须先核对后台状态。无法排除 Loader 仍在写入时，不并发启动回滚或删除仍被使用的包。
- 断电、强制结束进程、设备失联、磁盘故障或回滚失败时，无法保证当场恢复并删除所有材料。仅保留完成恢复必需的最小记录与唯一回滚材料，明确报告尚未完成；下次运行先自动续做恢复与清理，再允许新安装。不安装常驻清理服务，也不提前删除唯一旧版来制造“无残留”。
- 清理有界重试并再次检查实际结果；失败时区分插件当前状态与清理状态，整体不得报告全部完成。恢复材料仅在已验证目标版本或原状态可用后删除。
- 删除范围按本次创建清单及解析后的绝对路径限定，拒绝符号链接或越界。不删除用户传入的原始 ZIP、既有其他备份、插件外部业务状态、游戏数据或其他插件。`只检查` 模式退出时也清理本次下载与临时文件。

## 已核对依据与待解决项

- [Decky 上游 CLI 安装需求 #961](https://github.com/SteamDeckHomebrew/decky-loader/issues/961) 在本轮查询时仍开放，不能把规划中的命令称为已存在的官方安装命令。
- Loader 当前源码通过 `utilities/install_plugin` 请求安装、发出带 request ID 的确认事件，再通过 `utilities/confirm_plugin_install` 执行安装；见 [utilities.py](https://github.com/SteamDeckHomebrew/decky-loader/blob/main/backend/decky_loader/utilities.py) 与 [browser.py](https://github.com/SteamDeckHomebrew/decky-loader/blob/main/backend/decky_loader/browser.py)。这些接口属于版本相关实现，需要明确兼容范围。
- 项目既有 [Decky 部署技能](../../../.agents/skills/decky-deploy-verify/SKILL.md) 已有通过 Steam CEF 复用认证连接安装本地 ZIP 的设备证据。技能 helper 依赖 Node.js，不能直接作为面向普通用户的终端安装器。
- 已实现无需额外开发环境的桥接、终端确认/弹窗协调、首次安装和启用插件升级、隔离故障恢复。待实机确定：桌面模式与游戏模式的 CEF 可用性、首次安装/升级/失败恢复的真实 Loader 行为、禁用插件任务检查方案、同版本重装的后端进程身份。

## 验收标准

- 最新版与指定版本均冻结正确附件，错误哈希、异常包、下载中断在修改正式目录前失败。
- 首次安装与更新通过 Loader 完成；更新保留本插件状态和设置，不影响其他插件。
- 成功安装后，仅保留新版插件及应有业务状态，本次下载、备份、回滚包、临时日志、恢复标记和工作目录均不存在。
- 安装失败、检查未通过及结果未知均进入自动恢复；旧版或首次安装前未安装状态恢复并验证后，同样无本次临时残留。
- 覆盖下载失败、错误包、安装失败、重载失败、响应丢失、取消和中断后再次运行；验证不会在 Loader 活动写入时回滚，恢复与清理可重复执行，不影响其他数据。
- 模拟失联、回滚失败和清理失败，确认不会提前删除唯一恢复材料或误报完成；恢复条件满足后，下次运行自动收口恢复并清理。
- 活动任务、缺少 Loader、Steam 未运行或接口不兼容时给出明确错误，不静默转为直接覆盖目录。
- 完成隔离自动化、Steam Deck 真实首次安装/更新/失败恢复及重载后 RPC 验证；未执行的项目继续标记待验证。
