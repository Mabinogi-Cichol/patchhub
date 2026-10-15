# Decky 终端安装与更新施工记录

- 设计：[design.md](design.md)。
- 状态：终端安装器、README 短链入口、故障恢复与 CI 分发已实现，短链已创建并核对目标，隔离自动化、引导命令及真实 Release 只检查通过；Steam Deck 实机验收、禁用插件升级与安装器 Release 附件发布仍待完成。

## 2026-10-15

- 用户确认需求为在 Steam Deck 终端安装或更新本 Decky 插件。
- 核对现有仓库打包、Release 附件及 Loader 安装技能，确认可复用当前 Decky ZIP 和 SHA-256 清单；当前仓库没有面向用户的终端安装器。
- 查询 Decky 上游 CLI 安装需求与安装源码，确认需要项目自己的入口和有明确兼容范围的 Loader 调用，不能假设已有稳定通用 CLI。
- 记录版本冻结、校验、任务检查、备份、正常 Loader 安装与重载、状态验证及失败恢复要求；桥接依赖和弹窗协调仍待验证。
- 本轮仅只读调研并编写设计/施工记录；未连接或修改 Steam Deck，未部署、安装插件或操作游戏，未发布新附件。
- 自动化、首次安装、更新、回滚及双模式兼容性均待实现后验证。

### 成功后清理旧版备份

- 用户补充要求：安装成功后删除备份的旧版。已同步设计流程与验收标准，成功条件包括正式文件、重载后版本、插件实例及 RPC 检查全部通过。
- 成功后仅删除本次创建的旧插件备份及回滚包；安装失败或结果未确认时继续保留。清理失败单独提示可重试，不误报为安装失败。
- 当前仍为待实现，本轮未实际删除设备或本地已有备份。

### 自动恢复与无残留要求

- 用户进一步明确：失败和结果未确认时也应自动回滚旧版并删除本次残留，核心宗旨是不在用户设备留尾巴。本节替代上一节“失败或结果未确认时继续保留”的默认处理；上一节保留为决策历史。
- 已修订设计：成功后清理；失败或结果未确认时先协调 Loader 写入状态，自动恢复并验证安装前状态，再删除本次下载、备份、回滚包、暂存、临时日志、恢复标记与工作目录。首次安装失败恢复为未安装状态。
- 普通异常、取消、可捕获退出和预检失败均纳入自动收尾。断电、失联或回滚失败等无法当场收尾时，仅临时保留必要恢复材料；下次运行优先自动恢复并清理，不默认让用户手动处理，也不提前删除唯一旧版。
- 验收新增成功/失败双路径无残留、响应丢失不并发回滚、进程中断后续做，以及恢复/清理失败后的可重入检查。状态仍为待实现，未执行设备修改或文件删除。

### 实现与隔离验证

用户要求开始实现后完成以下修改：

- 新增单文件 [`scripts/install-decky.py`](../../../scripts/install-decky.py)，使用 Python 3.10+ 标准库，支持 latest 正式版、`--version`、`--check`、默认终端确认、显式 `--yes` 和 `--recover`。不需要 Node.js、Rust、pip 包或克隆完整仓库。
- 冻结 GitHub Release Tag 与附件来源，核对公开 `SHA256SUMS.txt`；在任何正式目录修改前拒绝错误哈希、重复/越界 ZIP 路径、链接、特殊权限、不符插件身份/版本、不完整 runtime、非 Linux x86-64 ELF 或缺少执行位。
- 通过 Steam CEF 的唯一 `SharedJSContext` 复用 `DeckyBackend`；核对官方 **v3.2.9** Tag 的安装事件、确认参数、`runningCalls` 和 Linux 根目录实现，运行时限制版本与对象结构。不读取认证 token，不另开 Loader `/ws`，不裸解压正式目录、不结束后端、不重启 Loader。
- 通过 systemd 的目录配置/标准布局确认插件根目录。安装前后核对任务、正式文件、运行版本、唯一前端实例、有效挂载、真实 RPC 与 HTTP bundle 哈希；只合并恢复本插件 Loader 设置成员与相邻位置，保留其他插件当前设置，不操作外部业务状态或游戏文件。
- 恢复包保存完整旧插件生产文件和权限，持久化阶段、唯一 JS job ID、Loader boot/PID/starttime、双方文件清单与哈希。对缓存父目录使用 `flock`，不创建锁文件；工作目录限于当前用户的固定私有暂存目录，清理拒绝链接/硬链接/未知文件。
- 在准备安装后至验收/恢复完成期间保留本插件写 RPC 保护，阻止界面启动新任务，并在确认前再次核对任务。新版后端 RPC 启动失败时，保留的保护允许恢复旧版；不影响其他插件调用。
- 响应丢失时查询保留中的 Promise 与活动写请求；无法确认 Loader 已停止写入时不并发回滚、不删除仍使用的包。同 Loader 进程下上下文丢失会明确报未完成；进程已更换且无活动写入后允许下次继续恢复。
- 失败/取消自动恢复旧版；首次安装失败通过 Loader 卸载并核对恢复为未安装。确认写入停止后先删除目标 ZIP；回滚失败只保留必要恢复记录和唯一旧版 ZIP。成功/恢复通过后删除全部本次材料和 JS 保护/记录，再检查工作目录不存在；清理失败分别报告新版或原状态已验证，并允许下次重试。
- CI 增加 Python 与 JS 桥接测试，并在 Decky 构建附件和下次 Release 中分发独立 `install-decky.py`；现有 Release 阶段会将它纳入 `SHA256SUMS.txt`。本轮未触发远端 CI、上传附件或发布版本。
- 更新客户端说明、主 README 提示、功能设计和索引；保留本文件前面的调研及决策历史。

验证结果：

| 检查 | 实际结果 | 证据 |
| --- | --- | --- |
| WSL Ubuntu 22.04 / Python 3.10 隔离测试 | 30 项通过，Exit Code 0；覆盖首次安装/更新、写前拒绝、重载/RPC/正式文件失败、取消、未知活动写入、上下文丢失、Loader 更换、可重入回滚、清理失败、外部数据保留、锁/路径/权限/CDP 帧 | `.artifacts/decky-cli-linux-tests.log` |
| Windows Python 包校验与适配错误测试 | 7 项通过，23 项 POSIX 专用测试跳过；Exit Code 0。Linux 权限和恢复结论采用 WSL 结果 | `.artifacts/decky-cli-python-tests.log` |
| Node 执行实际内嵌 JS 桥接 | 9 项通过，Exit Code 0；确认事件精确匹配、外来事件转交、任务复核、活动写入禁止并发、设置合并、版本拒绝、RPC/HTTP/前端校验、跨阶段保护及取消异常收尾 | `.artifacts/decky-cli-bridge-tests.log` |
| 现有发行打包回归 | `python -m unittest discover -s scripts -p test_package_artifacts.py`，Exit Code 0 | `.artifacts/decky-cli-packaging-tests.log` |
| 真实 GitHub `v0.1.0` Release 只下载/校验/清理 | 24 个文件，包 SHA-256 `1a6f7e04f5a0f21a8737cf37a326e8c1bb1951c31ce25abf244a0d5b50d05b57`，冻结版本/附件及公开哈希/runtime/ELF 均通过；本次隔离下载目录已不存在。仅调用下载与校验流程，未连接 Loader | `.artifacts/decky-cli-live-release-check.log` |
| 当前已有 Decky ZIP 兼容校验 | 本地已有生产包 24 个文件，通过安装器校验；没有重建或改动该包 | 本轮命令结果 |
| 独立安装器分发预演 | 将源文件按 CI 方式复制到 `.artifacts/install-decky.py` 并核对字节一致；不代表附件已经公开发布 | 本轮命令结果 |

所有测试在隔离文件系统或假 Steam/Loader 上执行；Windows 不模拟通过 POSIX 执行权限测试。没有为本功能修改插件运行代码或 Rust 代码，因此未重跑无关全量构建。

待验证与边界：

- **未连接或修改 Steam Deck**；真实首次安装、更新、失败恢复、重载后 RPC、实际中断信号与游戏/桌面双模式均待实机验证。模拟 `KeyboardInterrupt` 和恢复记录续做不等于完成真实信号或断电验收。
- 已禁用插件没有可读运行任务，当前安全拒绝升级，需先在 Decky 启用；禁用态直接升级仍未实现。
- 仅 Loader 3.2.9 和可解析 systemd 布局纳入适配；其他版本/额外环境文件拒绝，不静默使用直接覆盖。
- 当前核对 Loader 报告的后端版本和真实 RPC；同版本不同构建重装缺少后端进程内构建指纹，实机验收需特别核对，不能仅凭相同版本号证明换了后端进程。
- 失联、同 Loader 上下文丢失且写入无法确认、回滚失败或磁盘故障时不能承诺立即无残留；明确保留必要材料，下次优先收尾，不提前删除唯一旧版。
- 尚未提交、部署或发布；当前 README 没有宣称一个已存在的 Release 安装器下载入口。

### README 复制一行直接安装

- 用户进一步明确交付流程：GitHub README 提供一行命令，用户在 Steam Deck 桌面模式 Konsole 粘贴执行即可安装，不要求先手动下载并保存 Python 文件。本节替代上一阶段以“先保存安装器文件”为主要入口的说明。
- 主 README 已写入完整单行命令；客户端说明使用同一命令，保留指定版本、只检查、无人交互与恢复的追加参数用法。安装与更新使用同一行，默认保留终端版本确认。
- 命令使用系统 Bash / curl，从 `https://raw.githubusercontent.com/Mabinogi-Cichol/steam-hentai-patch-searcher/main/scripts/install-decky.py` 完整下载到内存，下载成功且非空后才调用 `python3 -c`；不保存临时安装器，也不占用 Python 的终端 stdin。curl 即使已经输出部分源码，只要退出非零就不会执行它；Python 非零退出会原样返回。
- 使用 `git ls-remote --symref origin HEAD` 只读确认 GitHub 默认分支为 `main`。源码与 README 同步推送后即可使用此入口，无需先发布新的安装器 Release 附件；实际插件包继续从冻结版本的 Release 下载并校验。
- 增加 [`scripts/test_decky_bootstrap.py`](../../../scripts/test_decky_bootstrap.py)，直接提取 README 中的原样命令执行，使用假 curl 和隔离目录。WSL Ubuntu 22.04 下 **7 项通过，Exit Code 0**：两份 README 命令一致、成功与参数转发、部分下载失败不执行、空下载拒绝、安装器失败退出码、终端输入保留、实际安装器从内存运行 `--help`；执行目录均无安装器残留。日志：`.artifacts/decky-cli-bootstrap-tests.log`。
- Windows 下文档一致性 **1 项通过，6 项 Linux 专用测试跳过，Exit Code 0**；日志：`.artifacts/decky-cli-bootstrap-windows-tests.log`。CI 已增加此测试。安装器主体未修改，沿用上一阶段的 30 项 Linux 与 9 项桥接验证，不重复执行。
- 本轮未推送 GitHub、未连接 Steam Deck、未执行真实桌面模式安装。命令引导隔离测试通过不等于已完成桌面模式端到端验收；Loader 3.2.9、CEF 可用、启用插件升级等既有适配边界保留在用户说明中。

### 缩短为参考项目的短链管道入口

- 用户认为上一阶段一行命令过长，要求参考 [DeckyClash 中文 README](https://github.com/chenx-dust/DeckyClash/blob/main/README_CN.md) 的在线安装命令。核对其使用 `curl 短链接 | bash`，本节替代上一阶段在 README 直接展示下载变量和 `python3 -c` 的命令形式。
- 新增根目录 [`install.sh`](../../../install.sh)，将完整下载 Python 源码、非空检查、失败退出与参数传递移入引导脚本。脚本先定义函数，最后调用；没有下载到调用行的截断引导脚本只定义函数，不执行安装。不创建引导/Python 安装器临时文件。
- 已通过 ba.sh 网页实际创建短链接 **`https://ba.sh/tz4y`**，目标为 `https://raw.githubusercontent.com/Mabinogi-Cichol/steam-hentai-patch-searcher/main/install.sh`。浏览器打开短链后，地址确实跳转到上述目标；目标当前返回 404，原因是本轮新文件尚未推送。命令行 HEAD 探测在执行环境返回 403，未把它当作已通过的 curl 端到端验证。
- 主 README 与客户端说明的主命令统一为 **`curl -fsSL https://ba.sh/tz4y | bash`**。追加参数改用 `| bash -s -- --check` 等标准形式；客户端说明同时保留不依赖短链的 GitHub 原始地址入口。
- 更新引导测试，使用假 curl 完整经过“短链命令 → 本地真实 install.sh → Python 源码”两段流程。WSL **9 项通过，Exit Code 0**：README 一致性/长度、参数和空格、Python 下载失败/空下载、安装器退出码、真实 PTY 下 `/dev/tty` 确认输入、内存运行实际安装器 `--help`、截断引导不执行、GitHub 直达入口。执行目录没有脚本残留；日志继续为 `.artifacts/decky-cli-bootstrap-tests.log`。
- Windows 下 **1 项通过、8 项 Linux 专用测试跳过，Exit Code 0**；日志为 `.artifacts/decky-cli-bootstrap-windows-tests.log`。安装器主体未改，沿用此前安装/恢复与桥接验证，不重复运行。
- 短链创建截图：`.artifacts/decky-cli-shortlink.png`。没有提交/推送代码、发布附件或连接 Steam Deck；公开命令需代码推送后才能使用，桌面模式实机端到端验收仍待执行。

### 分别说明在线与离线安装

- 用户要求 README 分别介绍命令行在线安装和 ZIP 离线安装。主 README 现分为两个独立小节，并在下载表格中分别链接。
- 在线小节保留短链命令、版本确认、自动下载与校验、安装器当前兼容范围；离线小节说明在可联网设备下载完整 ZIP、传到 Steam Deck、通过 Decky 本地 ZIP / 文件入口安装，以及部分版本需启用开发者模式。
- 公共前提与安装后进入插件的步骤集中说明；明确 ZIP 已包含核心程序，本地安装无需从 GitHub 下载依赖，补丁查询与下载仍需联网。
- 客户端 README 同步增加 ZIP 离线安装说明，修正“保存安装器文件”与“离线安装”的混淆：单独保存 Python 安装器后运行仍会联网下载插件包。
- 本轮仅调整文档；沿用已有运行验证，只检查文档链接、两份 README 的在线命令一致性及差异格式，不重跑安装/恢复测试。没有提交、推送、部署或发布。

### 提交与推送整理

- 用户明确授权提交并推送。核对仓库为 `Mabinogi-Cichol/steam-hentai-patch-searcher`，分支 `main`；同步远端后本地与 `origin/main` 无差异，初始暂存区为空。
- 本次范围为终端安装器、引导脚本、配套测试/CI 和功能文档，按仓库规则拆分文档与功能代码提交；忽略目录 `.artifacts/` 中的下载、日志和截图不纳入提交。
- 沿用本 session 已完成的 Linux 安装/恢复、桥接、引导和真实 Release 校验记录；仅因文档公开状态调整复核链接与差异，不重复运行无改动的运行时验证。
- 将当前说明中的“尚未推送”改为入口跟随 `main` 分支的长期说明，保留前面各阶段的历史事实；未把提交/推送等同于 Steam Deck 实机验收或 Release 附件发布。
