# Windows 与 Decky 客户端自更新施工记录

- 设计：[design.md](design.md)。
- 当前状态：**双端自更新已实现；自动化、Windows 隔离 UI 完整更新链、Decky 实机独立进程完整更新链及失败恢复通过；正式 GitHub 跨版本发布与完整 Windows 安装版验收待执行。**
- 分支：`feature/client-self-update`，基于 `main@81a2965`。

## 2026-10-28：Git 提交归档

- 按用户“提交 GIT 合并 main”要求，将本轮自更新修复与追加的 Windows 单实例分别提交，文档单独归档。
- 自更新代码：`ebdc4ed`（补齐下载进度并提前检查更新目录占用）；Windows 单实例代码及必要测试/CI：`4fa4e43`（阻止重复实例并唤起已有窗口）。
- 本次提交沿用本 session 已通过的 14 项 Flutter、12 项真实更新助手、6 项原生单实例、9 项 CI 选择回归，以及 analyze/原生构建/隔离更新链记录；代码没有新增修改，不重复运行。检查工作区及暂存差异和空白，排除 `.artifacts` 本地程序、日志和夹具材料。
- 合并安排：文档提交后将本地 `main` 快进至 `fix/windows-self-update`；不推送远程、不创建 Tag 或 Release。正式 GitHub 跨版本更新与系统安装版验收仍待完成。

## 2026-10-28：追加 Windows 单实例

- 用户追加要求禁止重复实例窗口；沿用 `fix/windows-self-update` 和本功能目录，未提交或合并。
- 在原生 `wWinMain` 最早阶段获取会话级 PatchHub 互斥量，先于 Flutter 引擎/用户数据/后台任务初始化；重复进程通过主窗口专属属性定位窗口，还原最小化状态并激活，然后直接退出。安装版与便携版在同一会话也互斥，不依赖 EXE 路径或通用 Flutter 窗口类名。
- 互斥量持有至 Flutter 引擎及窗口析构完成；正常退出后释放，异常退出由内核回收。`--update-probe` 不绕过互斥量；助手等待旧进程退出后才启动新版，原生真实更新链确认可以顺利接管。
- 新增真实 C++/Win32 进程窗口回归，直接编译生产 `single_instance.cpp`：重复启动/最小化还原、8 进程并发仅一个主实例、主窗口初始化中重复启动、另一安装目录互斥、正常退出后探针启动及探针不可绕过、崩溃遗弃互斥量重新获取，共 6 项通过。夹具使用独立互斥量标识，不操作用户窗口。测试已接入 Windows CI，新增测试路径的作业选择回归 9 项通过；CI 本身尚未在线执行。
- 原生完整构建通过：MSVC 14.44 / CMake / Ninja，输出 `.artifacts/windows-self-update-fix/native-build/runner/`，在隔离目录装配本地候选 `.artifacts/windows-self-update-fix/candidate/`；未覆盖用户现有 Release 文件。旧 CMake 3.18 的运行库自动发现有警告，候选复用本地既有发行目录的 MSVC runtime 后启动通过，不代表已生成 NSIS 或完成正式打包验收。
- 真实 Flutter 候选隔离验证通过：重复进程 exit 0、首实例仍运行且窗口从最小化还原；正常关闭旧进程 exit 0 后，实际 PowerShell 助手替换元数据 0.3.0 → 0.4.0 夹具，启动新版原生进程完成 Core/首帧/commit/ack 握手，助手 exit 0、结果 completed，额外用户文件保留。证据 `.artifacts/windows-self-update-fix/native-single-instance-result.json`。这是脚本授权助手的本地夹具链，不是 UI 点击或真实 GitHub 跨正式版本发行验收。新版随后收到正常关窗请求、窗口消失，未单独获取新版退出码。
- 冒烟夹具首轮在 WindowManager 初次展示完成前执行最小化，状态被初始化覆盖而等待失败；调整为等待更新初始化记录后再次验证通过，生产代码未增加延时。CI 选择首轮沙盒因临时目录权限失败，原生权限重跑通过。
- 单实例设计/执行路径的过度设计复核通过：仅进程生命周期互斥量、窗口属性与现有 Win32 显示/激活 API；没有新配置、持久锁文件、轮询、等待阈值或更新豁免。原下载 UI/Dart 服务与助手逻辑本次未变，沿用上节 14/12 项验证及 analyze 记录。
- 未执行：真实 GitHub 跨版本升级、正式 Windows 安装包/系统安装版完整矩阵；未发布。运行中的旧构建不具备新互斥逻辑，需正常退出后启动新候选才生效。

## 2026-10-27：Windows 下载进度与多实例更新失败修复

- 修复分支：`fix/windows-self-update`，基于 `main@2bf0dda`；专项分析与验收项见[修复计划](fix-plan.md)。
- 现场只读证据：更新结果文件仅记录 `Application still running in target directory`，目标是本地 Flutter Release 目录；同目录有两个仍持有响应窗口的 PatchHub 实例（PID 1216100、1253460），均由 Explorer 启动。未关闭或终止用户窗口，也未改动该目录中的发行文件或用户更新记录。
- 原因：助手等待发起者退出后才检查其他进程，另一实例会令安装在替换前失败，导致窗口已关闭且没有新版启动。`HandshakeException` 是另一网络操作的错误，旧界面缺少清晰分隔，不能据此认定安装失败由 TLS 引起。
- 已实现：下载进度条/百分比/可读大小、未知长度不定进度、下载附件与校验阶段提示；清除上次进度；历史安装结果与当前操作错误分隔及重新检查入口；助手准备阶段拒绝同目录其他客户端并报告进程/PID，保留退出后的全部进程检查。
- 2026-10-28 最终验证：`flutter test --no-pub test/app_update_test.dart test/app_update_panel_test.dart test/app_lifecycle_test.dart` 14 项通过；`python -m unittest discover -s scripts -p test_windows_update.py -v` 12 项通过（包含强化后的双实例真实进程回归，两个客户端均保持存活，助手必须在 15 秒测试期限内拒绝准备；生产未增加超时）；`flutter analyze --no-pub` 无问题。助手原有的替换、坏哈希、失败恢复、中断恢复、无关文件保留及卸载回归均通过。
- 网络实测：相同 Dart `HttpClient`、请求头及默认 TLS 验证访问官方 latest API 和 v0.4.0 校验清单，均 HTTP 200（API 返回 v0.4.0，清单 278 字节）；当前没有重现握手异常，不能由此断言间歇性网络故障已消除。诊断脚本与结果摘要在 `.artifacts/windows-self-update-fix/`。
- 验证环境：首轮沙盒 Flutter 因全局 telemetry 文件拒绝写入而失败，改用原生权限完成检查；真实助手测试也使用原生权限以读取测试进程。未将权限问题记为代码测试失败。
- 过度设计复核：沿准备、退出、占用拒绝、替换及恢复路径核对，准备阶段检查对应现场两个同目录窗口的实际文件占用；复用现有检查函数，无新常驻服务、配置、固定延时、强杀或自动网络重试。复核范围与未验证项见[实现复核](review.md)。
- 未执行：真实 GitHub 跨正式版本更新、用户当前应用更新、系统安装版完整验收；尚未构建、提交或发布本次修复包。TLS 握手异常的网络原因尚未复现，当前只改善可辨识性与手动重试入口，未绕过证书校验。

## 阶段状态

| 阶段 | 状态 | 证据 / 后续条件 |
| --- | --- | --- |
| 调研与设计 | 已完成 | 核对现有双端宿主、Core v5、CI、NSIS、Decky 终端安装器与上游 Loader 3.2.9 源码；形成客户端层方案 |
| P0 安装可行性原型 | 本地 Release 夹具完整链通过；正式发布链待验收 | Windows 真实 UI 下载/确认、退出及新版握手通过；Decky 实机下载、Loader 替换/重载、验证及恢复通过 |
| P1 检测与提示 | 已实现 / 自动化通过 | 宿主 GitHub 检测、SemVer、每日调度、手动检查、跳过版本、通知与附件冻结校验；无 Core 协议改动 |
| P2 Windows 自更新 | 已实现 / 隔离完整更新链通过；安装版系统级验收待执行 | 统一 ZIP 助手、事务恢复、动态卸载清单、安装版版本维护；本地 HTTPS 响应夹具 + 真实 UI/原生程序通过 |
| P3 Decky 自更新 | 已实现 / 实机夹具完整链通过；正式跨版本链待验收 | Loader 3.3.0 完整自更新执行链、失败恢复、GitHub 检测和页面通过；测试后恢复正常 0.3.0 |
| P4 发布与验收 | CI 改造已实现 / 正式发布验收待执行 | 草稿上传/下载校验/公开步骤已改造；未创建或修改公开 Release |

## 2026-10-24：调研与设计

- 用户范围：双端自动检测 GitHub 新 Release 并提示，调研 Core/客户端分层并提供设计；本轮不实施。
- 工作树开始时干净，按仓库规则从 main 创建本功能分支，新增设计/施工记录并更新功能索引。未提交或合并。
- 当前事实以仓库重新核对为准：Flutter 是 Windows 当前客户端，原有记忆中的迁移阶段不作为本轮完成状态依据。
- 只读查询 `https://api.github.com/repos/Mabinogi-Cichol/steam-hentai-patch-searcher/releases/latest`，返回规范仓库 `Mabinogi-Cichol/steam-hentai-patch` 的 `v0.3.0`，发布时间 `2026-10-23T18:23:52Z`（北京时间 2026-10-24 02:23:52），draft/prerelease 均为 false。
- 核对附件：Windows ZIP 19,199,825 字节、NSIS 14,551,300 字节、Decky ZIP 7,003,317 字节，另有 `SHA256SUMS.txt`、两端 checksums/SBOM 和终端安装脚本。未下载或安装这些附件；仅核对 API 元数据。
- 阅读官方 GitHub Releases/限流文档和 Loader v3.2.9 utilities/browser/plugin-loader 源码；本地核对 `@decky/api`，确认不能假设插件 callable 可直接调用安装管理路由。
- 关键结论：自更新归客户端宿主，Core/协议不变；Windows 独立助手接管退出后更新，Decky 复用安装器引擎并由独立进程跨卸载执行；完整包一次更新配套 GUI/插件、Core、worker。
- 设计级过度设计审阅已完成：移除共享 Rust 更新协议/SDK、预发布配置、任意硬限制和重试框架的预设；保留由实际文件占用与卸载路径支持的门禁和恢复。实机可行性仍为待验证项，不等同通过实现审查。

## 本轮验证

### 2026-10-24：移除自动检查开关

- 移除自动检查开关，保留手动检查、“稍后”和“跳过此版本”；经用户澄清，不采用“始终启用”的表述，用户通过“跳过此版本”控制该版本的更新提醒。
- 同步移除设计中的 `auto_check` 偏好字段和关闭自动检查验收项；其余偏好仅用于跳过版本等既定行为。

### 2026-10-24：Windows 自更新统一使用 ZIP

- 按用户确认，安装版和便携版共用 ZIP 下载、校验、退出协调、文件替换与恢复助手；取消原设计的 NSIS 自更新路径。NSIS 继续用于首次安装、手动安装和卸载。
- 安装版仅额外维护本产品注册表版本值和当前受管理文件清单，保留安装路径、产品名、卸载器及快捷方式；便携版不创建安装注册信息。
- 核对已有 NSIS 卸载列表为构建时编入，设计要求首次接入版本的卸载器改为读取已提交文件清单，确保 ZIP 多次更新后仍能正确卸载。v0.3.0 安装版需手动安装包含该契约的新版本，再使用自更新。
- 同步修订架构图、附件选择、Windows 安装流程、实施阶段、验收矩阵和设计级复杂度审阅；功能仍待实现，未运行程序更新或部署。

- 文档结构与相对链接检查通过：两份功能文档及功能索引的本地 Markdown 链接目标均存在；`git diff --check` 通过（另核对新增文档空白）；仅针对文档。
- 未运行 Rust/Flutter/Decky 功能测试或构建：没有功能代码改动。
- 未执行 UI 自更新、真实 Windows 替换、Decky 连接/部署/重载、失败恢复、断电或正式发布验收。

## 后续工作

1. 使用下一正式新版本验证从提示/确认、GitHub 下载到双端独立执行者完成跨版本更新的整条链路；没有创建测试 Release 或伪造线上新版。
2. 完成 Windows 系统安装版注册表、快捷方式、自定义位置及连续多次更新后的真实卸载矩阵；现有隔离程序文件/清单/恢复验证不能替代这些项目。
3. 完成 Steam Deck 实体手柄操作、正式新旧版本升级与断电恢复矩阵，按真实证据更新记录。CI 发布流程已修改但未实际发布验证。

## 2026-10-24：实施与阶段验证

- 用户授权“开始实施”；Windows 安装版/便携版统一 ZIP，保留“跳过此版本”，没有自动检测开关。Core 和 v5 协议未修改。
- Windows：Dart 宿主更新服务和界面、外部 PowerShell 更新助手、ZIP 路径/哈希/清单验证、退出协调、文件/安装版版本信息恢复、新版首帧与 Core 探针握手、待确认启动不执行正常业务初始化、故障恢复入口；NSIS 改为读取当前清单卸载，并与更新共享目录锁。
- Decky：Python 宿主更新服务、React 更新区域和通知去重、独立会话安装进程、下载进度/取消、任务与调度维护门禁、重载期间等待提交、恢复入口；复用并随插件打包受信任的当前安装器。
- 构建：生成 Windows 版本与文件清单，核对 Cargo/Flutter/Decky/Tag；更新生产 backend allowlist 和 CI 输入选择；Release 改为草稿完整上传、下载 SHA-256 核验后公开，拒绝自动替换公开附件。
- 自动化：Flutter 全量 213 项通过，`flutter analyze --no-pub` 通过；Windows 真实 PowerShell 更新助手 6 项通过（成功、坏哈希、失败回滚、无关文件冲突、模拟中断恢复、更新后卸载）；Decky 后端 107 项通过（2 项跳过），前端 68 项通过，桥接 14 项通过；Ubuntu WSL 的安装器 34 项全部通过；打包 8 项、CI 选择 9 项通过。后续代码变更只补跑受影响检查。
- Windows 构建环境：标准 `flutter build windows` 仍受本机 VS 自动发现问题阻断。沿用仓库已有 MSVC 14.44 / CMake / Ninja 命令构建普通 `lib/main.dart` 候选，未修改系统工具链；候选 NSIS 已编译。证据在 `.artifacts/client-self-update/`。
- Windows 真实原生隔离验证：`.artifacts/client-self-update/native-smoke-result.json` 记录 helper exit 0、版本 0.3.0、额外文件保留、事务目录已清理；实际 Flutter 程序完成 Core capabilities 与首帧/提交握手，随后响应正常。正常关闭请求成功，约 701ms 退出；该次 Get-Process 附着未返回退出码，因此不宣称已确认 exit 0。使用独立 APPDATA，不修改现有安装或游戏。
- 自动审批拒绝批量移除既有 CEF 响应大小限制和操作超时，理由为无界等待及可用性风险。本轮保留这些既有边界，不绕过拒绝；这是与设计中全面精简旧防御路径的差异，后续需单独基于实证审议。下载 ZIP 的任意大小/数量限制已在获准的修改中删除。
- 用户打开并连接 Devkit 后，按部署技能确认当前配对与已有可信主机记录；设备 Loader 为 v3.3.0（原方案只核对过 3.2.9），旧插件 `Steam 补丁管理` v0.2.0，无非终态补丁或来源任务。已核对官方 v3.3.0 utilities/browser/plugin 源码及设备只读桥接 probe，扩展并验证管理适配。
- 设备第一次候选安装失败：本地旧 Linux Core 缓存与当前生成协议不一致，新后端报 `unsupported_protocol`。真实 Loader 安装事务自动恢复旧版 v0.2.0、运行验证通过并清理缓存事务，未将该次部署记为成功。随后从当前锁定源码重建 Linux Core/worker，当前候选实际 v5 preflight 通过，再进行候选安装验证。
- 旧插件完整备份：`.artifacts/client-self-update/old-plugin.tar.gz`，SHA-256 `0051fa516d26d2a1f5379ddea86d54a4d9a7144299c135231ac4a2a397957d12`；旧版任务/设置/数据库基线与设备证据保存在同目录，未提交个人数据。

### Steam Deck 后续验证结果

- 重建后的 Linux Core/worker 与当前生成契约通过实际 `CoreBridge.preflight`；经 Loader 正常安装候选 0.3.0 并完成运行验证，缓存安装事务已清理。第一次失败时的旧版恢复属于真实设备证据，而非模拟测试。
- 实机发现 Loader 冻结 Python 默认 CA 解析失败：真实 GitHub 检查报证书链错误。已改为系统 `/etc/ssl/cert.pem` CA，并用系统 `python3` 启动独立执行者，清除冻结环境的 `LD_LIBRARY_PATH` 影响；保持 TLS 验证。修订候选已重新部署。
- 此阶段 Decky ZIP：7,031,919 字节，SHA-256 `ef76ccca18d104d863a10b945e9633953d8b08974ed2a6e435b0e50e3b073da5`。设备后端版本 0.3.0、唯一有效前端实例 0.3.0、真实 Core capabilities 协议 5；HTTP `dist/index.js` SHA-256 `20f241d02de1e03240e1a8b60d87eabddb888f9781178c3f576f1e583e3f5a00` 与本次本地构建一致。后续完整链验证的最终包见下节。
- 实际 `check_app_update` 成功读取公开 latest `v0.3.0`，状态 `up_to_date`；无 GitHub Token。真实游戏模式全局设置中点击“检查更新”，显示“当前已是最新正式版。”，页面无横向溢出。截图 `.artifacts/client-self-update/deck-update-checked.png`，原始 RPC/DOM 记录见 `deck-verify-final.json` / `deck-update-checked.json`。实体手柄未操作。
- 独立进程测试使用实际 `AppUpdate.start`，安装行为替换为隔离无安装脚本：宿主 SIGTERM、原插件目录删除后 worker 继续写出完成哨兵。在 WSL 和实际 SteamOS 均通过。该结果验证进程生命周期，不等同于前端确认后跨正式版本自更新已验收。
- 部署后 `journal.sqlite` 与 `trust.sqlite` SHA-256 均与部署前完全一致，Loader 其他插件顺序/成员保留，本插件身份按当前命名由“Steam 补丁管理”变为“Steam 黄油补丁助手”。没有安装或恢复游戏补丁。
- Windows 复核追加第 7 项真实 PowerShell 回归：失败的准备事务不能删除其他事务的恢复标记；中文路径的 JSON 统一 UTF-8。固定空维护锁载体保留在程序目录，卸载时删除，不进入发行 ZIP；下载、暂存与备份仍按事务清理。
- 实现复核见 [review.md](review.md)。本阶段尚未提交、合并或发布；最终 Windows/Decky 发行验收不因部分机制通过而自动标记完成。

### 本地 Release 夹具完整链与收尾

- 未发布测试 Release。Windows 使用测试进程的 `HttpOverrides`，Decky 使用测试启动环境的 `sitecustomize`，仅将固定官方 URL 的响应替换为本地 Release JSON、校验清单与完整 ZIP；这些注入不进入发行包。网络检测另以真实公开 latest 验证。
- Windows：真实 Flutter 更新组件点击下载、完成 SHA-256 校验、点击“重启并更新”、正常关闭宿主，实际 PowerShell 助手等待父进程退出后替换隔离旧目录 0.2.9，启动真实原生 0.3.0 程序并完成 Core/首帧/提交握手；无关用户文件保留、事务目录清理。最新源码构建后再次通过，结果 `.artifacts/client-self-update/windows-full-chain-result.json`；验证后的原生窗口通过正常关窗退出。
- 完整链发现并修复 Windows 临时目录 API 使用错误、下载阶段提示区域不可见、重复初始化与有效混合分隔符路径比较问题；未增加生产测试入口或固定等待后强制覆盖。新增 native chain 测试默认跳过，只有显式提供隔离夹具才启动外部真实更新器。
- Decky：生产 `AppUpdate.start` 启动真实外部安装器；启动宿主退出后进程继续下载、验证，并经实际 Loader 把 0.3.0 替换为测试 ZIP 的 0.3.1，运行验证完成，结构化结果 `completed`，事务目录清理。证据 `.artifacts/client-self-update/deck-full-chain-result.json`。这是执行链实机验证，未通过真实手柄或前端按钮开始该次更新。
- 完整链发现 `release_package` 丢弃校验结果中的 `name`，Loader 无法使用该结果；改为保留完整校验结果并补充契约断言，WSL 34 项安装器测试通过。状态轮询同时补充终态重读，防止进程退出检查覆盖刚写出的成功结果；新增竞争回归通过。
- SSH 测试会话退出曾终止所属会话进程；保留 SSH 验证会话后执行正常。真实 Loader 宿主不在 SSH 测试会话内；设备 POSIX 独立进程测试另已通过。没有据此增加生产进程保活框架。
- 测试后经 Loader 恢复正常 0.3.0。最终候选 ZIP 7,032,016 字节，SHA-256 `67cb6026030377a5a4e3dd5fe327ec5d038ab07e61ab4ce3527fa1a1ff931de7`；后端/唯一前端实例 0.3.0、协议 5、HTTP bundle 哈希与上节一致；真实检测为 `up_to_date`。
- 清理前核对首次完整备份哈希与本地副本一致，确认无恢复目录、无执行进程，删除本轮远端暂存及测试 `v0.3.1` 的终态结果；正常更新状态恢复为 `execution: null` / `needs_recovery: false`。Loader active、非终态补丁任务 0，持久数据库哈希仍与原始基线一致。保留本地备份和全部证据。
- 最新增量检查：Flutter analyze 无问题，自更新 3 项通过（native chain 默认 1 项跳过，显式夹具执行通过）；Windows 真实 PowerShell 7 项通过；Decky 更新服务 8 项通过；WSL 安装器 34 项、打包 8 项通过。先前未受影响的全量测试结果沿用。
- 最终真实扫描返回 27 个安装实例，证据 `deck-scan-result.json`。单次 CEF 等待窗口不足以覆盖扫描，改为异步记录后读取完成结果，没有终止 Core 正常工作或修改生产超时；测试界面已恢复 Steam 游戏库。

### Decky 更新入口与排版修订

- 按用户反馈移除 QAM 侧边栏中的更新区域；保留全局设置“缓存、备份与更新”页，以及既有通知跳转入口。
- 原更新组件与 `GlobalCache` 并列，位于 `.patch-page` 之外，导致边距和滚动范围与缓存/备份不一致；现在嵌入同一个页面容器。版本与发布日期使用标准 `Field` 行，按钮使用与现有设置相同的 `layout="below"` / `bottomSeparator="none"`，提示采用可换行段落。
- `npm run typecheck` / `npm run build`、`git diff --check` 通过；这是可逆的界面调整，未改更新引擎或增加配置、状态分支，沿用原有引擎测试记录。
- 经 Loader 部署实机候选：ZIP 7,032,365 字节，SHA-256 `99eb78dce54d548b7dd6d0c0cbe957a0da22461a4455093e0ad8b0cf19590f65`。修改前完整备份 `.artifacts/client-self-update/layout-before.tar.gz`，SHA-256 `45c520367158b2d087f894b651debd645ef0298639d5ac17c39f65c2648f52d8`。
- 真实全局设置点击检查更新通过，截图 `layout-update-checked.png`；按钮处于统一页面容器内，无横向溢出。QAM 已打开的旧组件需返回 Decky 列表再进入；重新进入后的真实 DOM 仅含全局设置/重新扫描，不含应用更新/检查更新，记录 `layout-qam-final.json`。
- 运行中的唯一插件/前端实例 0.3.0、Core 协议 5，非终态补丁及来源任务均为 0；HTTP bundle SHA-256 `01b605e8110944629aae1dbcfd331cc1836a47e996cc806aba158248122ce4e5` 与本地本次构建相同。实体手柄未操作。
- 验证后恢复 Steam 游戏库并关闭菜单；核对本地备份后清理本轮远端界面验证暂存，保留本地备份、截图和 RPC 证据。

### 2026-10-24 Windows 更新提示与布局修订

- 手动检查结果移入可滚动弹窗；底部固定版本/检查行保持原高度。自动检查发现 available 且未跳过/未暂缓时才新增底部提醒。检测服务记录本次实际检查是否手动触发，不改变查询/下载/安装引擎。
- 自动提醒直接查看既有结果；ready/downloading 时重新打开详情不再次检测，避免丢失下载完成状态。下载、取消、跳过、Release 页面和重启更新沿用现有入口。
- 新增 5 项更新面板回归：手动最新/失败/包未就绪/新版本均不改变主内容矩形；自动提醒、查看不重复检查、下载后关闭重开仍可重启更新通过。Flutter analyze 无问题；本轮完整 Flutter 回归 219 项通过、1 项既有原生更新链夹具测试跳过。没有重新执行已验证且未修改的更新替换引擎或 Decky 验收。
- 原生 Windows 布局验收因本机 Visual Studio 2026 缺少 Flutter 要求的 C++ 组件未能启动，截图与新包待验证。详细界面修复与证据见[Windows 施工记录](../windows-ux-redesign/progress.md)。

- 2026-10-24 用户要求构建后已补齐 VS 工作负载/CMake 并成功生成含本轮 Windows UI 修复的 Release；隔离 APPDATA 启动和正常退出通过，原生更新弹窗几何断言通过。产物/30 张原生布局截图及验证边界见[Windows 本轮施工记录](../windows-ux-redesign/progress.md)。本次仅本地候选，不是正式跨版本发行验收。

## 2026-10-25：提交归档

- 按用户要求将自更新功能提交到 `feature/client-self-update`，代码提交 `d3d2e68`，文档独立提交；未推送或合并。
- 本次只收录双端自更新、必要打包/CI/安装卸载改动及对应测试和文档，不收录品牌素材或其他 Windows 页面布局改动。
- 沿用本轮已有验证记录，提交前检查差异和空白；正式 GitHub 跨版本发布、Windows 安装版完整矩阵与实体手柄仍待验收。
