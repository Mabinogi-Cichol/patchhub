# Windows 交互重设计施工记录

设计：[design.md](design.md) · 施工计划：[implementation-plan.md](implementation-plan.md)

## 当前状态

2026-10-23 Modern 视觉迭代已获施工授权并完成主体实现，见 [Modern 视觉重设计方案](visual-design.md)。自动化、原生窗口与候选构建结果见文末施工记录；本轮状态不改变下方原 Flutter 重写的历史验收结论。

2026-10-22：P0–P7 实施及本地/用户实机验收已完成，系统/浅色/深色主题、Flutter 宿主、来源、安装/恢复和发行迁移均已落地；旧 React/Tauri 端已退役。用户确认系统交互全部通过，并明确将官方 ZIP 的 Cloudflare HTTP 403 作为外部遗留项接受，授权完成并合回 main。下方保留施工历史，最终证据与遗留项以末节为准；未推送或发布。

## 2026-10-21

- 用户要求新分支，参考 Decky 重设计 Windows，先看交互方案；随后要求终端不使用沙盒。
- 在干净 main `4079692` 上创建 `feature/windows-ux-redesign`。
- 静态核对 Windows main.tsx / SourcePanel / CSS / Tauri 命令与关闭流程，Decky 分栏、前台任务结果及可折叠来源卡片。
- 设计采用常驻游戏列表、游戏内四栏目、独立全局来源、前台进度与独立结果。保留 Windows 安装动作契约和全局恢复语义。
- 创建会话内演示原型：演示页面切换、来源折叠、安装进度与结果；示例数据不代表扫描或安装结果。
- 完成 overengineering-review 计划自审，结论见设计 §10；未委派独立审查。
- 未修改功能代码、未提交或合并、未部署及执行游戏安装。

## 验证

- 文档相对链接检查通过；git diff --check 通过。
- 原型脚本语法及概览→补丁→安装→取消/成功→关闭结果、全局来源返回流程通过 Node 模拟 DOM 检查；浏览器布局和真实焦点未验证。原型用内容区替换演示进度/结果，实际实现按设计采用模态对话框。
- 插件测试/构建/Windows 实机：待实现后验证，本轮未执行。

## 待办

1. 按细化后的 P0–P7 计划推进；整体交互已获认可。
2. 按设计实施并补充针对性检查。
3. 验证真实窗口、安装/取消/恢复与来源操作，记录证据。

## 2026-10-21 交互审阅反馈

- 用户认可整体方案，要求默认展示“有补丁”。已同步设计与原型；核对 Decky preferences.ts 默认值为 matched。功能仍待实现。

## 2026-10-21 Flutter 技术路线修订

- 用户明确要求改用 Flutter 重写 Windows。设计 §8 改为 Flutter + Dart / Material 3，复用 Rust CLI 协议与核心业务，替代 Windows React/Tauri。
- 已核对 Tauri 不只是窗口壳，还负责进程、事件、取消、来源任务与退出；方案将宿主迁移、旧数据连续性、打包切换列为必要验收范围。
- 当前 PATH 未发现 flutter；未安装 SDK、未创建 Flutter 工程、未修改应用代码。现有原型仅表达已确认交互，不是 Flutter 实现。
- 计划过度设计自审见设计 §11；实际桥接、构建与实机待验证。


## 2026-10-21 施工计划细化

- 按用户要求新增 implementation-plan.md，将重写拆为 P0–P7，每阶段记录实际入口、工作步骤、产出与退出条件。
- 补充 serve/serve-task、v5 fixture、全局恢复、旧状态目录、来源调度、NSIS/便携包、SBOM及Decky CI耦合的迁移工作。
- 设计 §8 链接具体计划；本轮仅文档，未创建 Flutter 工程或执行应用构建。
- 计划静态过度设计自审已完成；文档相对链接与差异格式检查通过。

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| P0 | 基线、工具链、Flutter工程 | 实施中：release构建和窗口启动通过，桌面交互专项待验 |
| P1 | 核心通信、旧数据接续 | 实施中：开发/候选包真实探针通过，旧/新客户端和历史事务对比待验 |
| P2 | 任务宿主、取消与关闭 | 已实现：宿主/取消/关闭18项测试及真实来源取消通过；实机关窗待验 |
| P3 | 游戏库、概览与补丁浏览 | 已实现：四栏目、筛选、多镜像、主题和布局自动化通过；实机待验 |
| P4 | 来源管理、匹配与调度 | 已实现：设置/订阅/规则/匹配/更新真实隔离联调通过；窗口交互待验 |
| P5 | 安装、结果、记录与恢复 | 已实现：EXE/本地ZIP完成/取消/备份恢复通过；在线ZIP403及窗口待验 |
| P6 | 便携/安装包、CI与升级 | 本地验证通过：完整便携/NSIS、升级/卸载/188状态文件保留；远程CI未执行 |
| P7 | 实机验收与旧版退场 | 待验收：工具故障导致真实窗口验证缺项，未移除旧版 |

## 2026-10-21 按过度设计审阅精简计划

- 用户要求落实三项审阅意见，已同步实施计划与设计 §8。
- 固定最终目录 apps/windows-flutter，删除 P7 搬迁；取消额外迁移映射文档，使用计划第二节；P5 改为依赖 P2/P3，保留 P4/P5 完成后的完整联调。
- P0–P7 仍全部待实施；本轮仅修改文档。相对链接及 git diff --check 通过。

## 2026-10-22 开始执行：P0 与 P1 首批实现

### 实际修改

- 继续使用 `feature/windows-ux-redesign`；保留开始时已有的功能文档和索引改动，未提交、未合并。
- 在最终目录 `apps/windows-flutter/` 创建仅启用 Windows 的 Flutter 工程，Material 3 中文主题、既有项目图标、产品名、1180×760 默认尺寸和 850×560 最小尺寸。
- 新增 `lib/host/core_paths.dart` / `core_client.dart`：显式开发资源根、可执行文件相邻的打包资源、旧版 APPDATA 目录、v5 能力协商和一次性 JSONL；同时读取 stdout/stderr，不增加任意截止时间，未知错误保留完整 detail。
- 新增 `desktop_services.dart` 和 `app_lifecycle.dart`，引入现成文件选择、HTTPS 外部打开和关闭拦截插件。当前关闭处理仅覆盖只读工程壳；P2 活动任务取消/终态等待尚未实现，未开放安装入口。
- 新增初步真实只读浏览页、模型与 `tool/core_probe.dart`，顺序执行能力→扫描→事务→来源；默认有补丁、无结果可显式查看全部、刷新失败保留旧列表。此页面只是 P1 接入载体，P3 四栏目、全部筛选、补丁归组和选择生命周期未完成。
- 新增 `scripts/build-windows-flutter.ps1`，检查固定 SDK、构建 Rust CLI/worker、检查 Flutter、构建 release 并复制核心/资源/许可证；新增独立 CI 候选 job。旧 Tauri/Decky 构建与正式 Release 入口保持现状，P6/P7 再切换。

### 工具链与环境处理

- 使用官方发行清单固定 Flutter **3.47.7**、Dart **3.13.5**，revision `abaf9c523780a608bd46686fd5e53740a07077f8`；SDK 位于仓库忽略目录 `.tools/flutter/`，未修改全局 PATH。
- SDK ZIP SHA-256 校验通过：`c2c875da25a1a1448d18ee4e8f398b2f6fce888b4775d8f9130538e16d826cb4`。版本写入 `.flutter-version`，CI 使用同版本，依赖写入 `pubspec.lock`。
- 本机 VS 2022/2026 已存在，但 Flutter 诊断确认缺少桌面工作负载/CMake；首次普通安装退出 5007，日志要求管理员启动。已通过 UAC 安装 VS 2022 Desktop C++ 与 CMake，最小组件安装器返回 0。未重启系统。
- Windows 普通进程缺少符号链接权限。依赖解析和候选构建采用管理员进程，不修改系统开发者模式；用户已确认 UAC。相关构建日志位于 `.artifacts/windows-flutter/`。
- `flutter doctor -v` 的 Android 警告与本轮仅 Windows 工程无关；无需为本功能安装 Android 工具。

### 已执行验证

- `cargo build --locked -p steam-patch-cli`：通过。首次探针使用旧本地二进制时准确拒绝旧协议；重建后 v5 通过，没有增加协议降级。
- `flutter analyze`：通过（No issues found）。
- `flutter test --reporter expanded`：**14 项通过**。覆盖现有 v5 能力夹具、扫描身份、未知错误详情、错误版本/request_id，以及850×560下默认筛选、查看全部和刷新失败保留列表。合成数据测试不算真实窗口验收。
- 真实 Dart→CLI 探针：通过。`tool/core_probe.dart` 使用 `.artifacts/windows-flutter/p1-state`，读取 **37 个安装实例、4 个来源、0 条事务**；catalog_trust=`signed`、无 catalog_error。4 个来源为 patchmoe/kagura-cn/kagura-en/hgamer；HGamer 的 source_partial 原样保留，没有宣称完整。
- 旧目录确认存在：`C:/Users/wqj78/AppData/Roaming/io.github.steampatch.manager`。验证前仅将 sources、journal.sqlite、trust.sqlite复制到上述隔离目录，未移动/删除原数据；原 installer 下载与恢复材料未用于安装或恢复。0 条事务的副本读取结果不能证明历史事务与恢复材料迁移已验收。
- 构建脚本 PowerShell AST 解析通过；CI 仅新增配置，尚未实际远程运行。

### 审阅与未完成项

- 按 `overengineering-review` 对本批路径定位→能力查询→一次性进程→模型→页面读取及关闭路径静态自审：未引入 HTTP/FFI、额外状态框架、超时或重试；删除尚未使用的关闭处理器注册表。HTTPS 边界与旧宿主一致。此为自审，不代表任务或实机验收通过。
- P0：候选构建与仓库外启动通过；系统文件对话框、HTTPS外部打开、关闭反馈及窗口尺寸/缩放的实际交互待验证。
- P1：真实 Flutter 窗口扫描、旧/新客户端同数据对照、已有事务/恢复材料连续性、Flutter 页面读数与目录错误呈现仍待验证；模型和响应校验目前只覆盖本批读取路径，不宣称全部 v5 结果/事件已迁移。
- P2–P7 尚未实施；未执行真实游戏安装/取消/恢复、未部署设备、未发布，未移除旧客户端。

### 同日补充：候选构建与仓库外启动

- 完整运行 `scripts/build-windows-flutter.ps1`：release Rust CLI/worker、依赖锁校验、analyze、14项测试、Flutter Windows release和资源复制全部通过；退出记录 `.artifacts/windows-flutter/build.exit` 为0。候选目录 `apps/windows-flutter/build/windows/x64/runner/Release/`。
- 自审再删除页面层重复能力查询（CoreClient 仍在每次核心生命周期前查询能力）；随后补跑 analyze、受影响的 library_page_test（1项通过）及 release增量构建，全部通过。
- release目录完整携带22个文件（含 data下文件、Flutter DLL、4个插件DLL、CLI、worker、资源和许可证）；最终哈希清单 `.artifacts/windows-flutter/candidate-files.json`。这只是 P0 候选目录，不是 P6 的便携ZIP/NSIS产物。
- 使用子进程独立 APPDATA 指向 `.artifacts/windows-flutter/window-smoke/appdata`，从系统临时工作目录启动候选，未改变父进程/系统 APPDATA。系统进程信息确认实际窗口标题“Steam 黄油补丁助手”、窗口句柄非0、Responding=true；证据 `.artifacts/windows-flutter/window-smoke/window.json`。窗口已保留供查看；此证据不证明页面全部读数、对话框、外链、焦点或缩放通过。
- 从同一系统临时目录运行 Dart探针，使用候选目录内 release CLI/resources，读取37个实例、4个来源、0条事务，签名目录正常。与开发核心探针对比，完整实例ID集合及来源快照一致；证据 `.artifacts/windows-flutter/p1-bundled-probe.json`。这不是旧 Tauri客户端与Flutter客户端的完整迁移比对。
- 最终 `flutter doctor -v`：Windows桌面工具链通过，采用 Visual Studio Community 2022 17.14.30 / MSVC组件 / Windows SDK 10.0.26100.0；日志 `.artifacts/windows-flutter/doctor-final.log`。SDK仍使用显式仓库路径，未设置全局PATH。
- P0/P1继续标为实施中；P2–P7与完整实机验收未完成，不合并main，不切换正式发布入口。

## 2026-10-22 深色模式

- 按用户要求在 `lib/app.dart` 增加 Material 3 深色配色，浅色/深色共用项目种子色，默认 `ThemeMode.system`，响应系统主题变化。
- 在 `LibraryPage` 顶部加入“外观”菜单：跟随系统、浅色、深色，菜单标记当前选择。选择保留于本次应用会话，未引入持久化依赖或改动核心数据。
- 根组件持有主题模式，复用现有页面实例；切换不重置筛选/页面状态、不触发额外扫描。主题统一覆盖现有游戏库、全局来源、卡片、输入与反馈控件。
- `flutter analyze` 通过；受影响 `library_page_test.dart` 两项通过且无点击警告，新增用例验证系统深浅变化、三态菜单选择、手动覆盖系统、切回系统、筛选保留与扫描仅一次。原协议测试沿用此前有效记录，未重复运行。
- 同步设计“信息架构与布局”中的主题行为。按 overengineering-review 沿菜单→根主题状态→MaterialApp→现有页面静态自审，未发现需修改项；未增加新状态框架、偏好存储或异常兜底。
- release 候选更新与实际视觉、焦点/缩放验收状态见后续验证记录；窗口原生标题栏未纳入本次 Flutter 内容主题的自动化验证。

- 深色模式 release 增量构建通过（25.7秒）；上轮候选正常关闭后，已重新启动新候选，继续使用隔离 APPDATA 和仓库外工作目录。最终文件哈希清单已刷新。实际深浅主题视觉、原生标题栏和键盘焦点仍待实机验收。

## 2026-10-22 继续执行并设立目标

- 用户要求继续完成 P0–P7，并已建立对应活动目标；未经完成必需验证不标记目标完成。用户另明确允许实机安装/取消/恢复验收的范围：EXE“妃神会秘史”（AppID 2303800），ZIP“你妈真漂亮”（AppID 2743810）。写入前仍先完成临时目录验证并保护当前文件。
- 读取现有 Rust/Tauri 和共享 UI 执行路径后，新增 protocol_models.dart：直接消费 Rust 生成的 v5 schema，移植现有 schema 子集与边界校验；随应用打包的 schema 与权威文件有一致性测试，构建时同步，不维护另一套 DTO 图。
- 全部 v5 响应/事件夹具对照113项通过（本轮新增 failed 终态夹具前的计数）。CoreTask 已迁移独立进程、分块JSONL、序列去重、方法允许的事件、唯一权威终态、完整stderr、启动前取消意图和cancel/EOF清理。无强杀、任意超时或数据上限。
- 实际测试发现并修复stderr覆盖末尾诊断、同步事件回调取消重入两个问题；逐事件流保留每条 match_update，不只暴露可能覆盖的最后快照。
- AppTasks 保留来源同kind/key去重（update_source按来源、match_games固定match、rematch_game按实例或AppID）与独立单安装槽。退出先阻止新任务并取消活动任务，等待真实终态和一次性写调用（recover等）结束；恢复需求或异常在退出前明确呈现，最小化不取消。
- 可控真实子进程与窗口插件模拟测试18项通过，涵盖分块/中文、1MiB stderr压力、重复/跳号事件、终态后噪声、非零退出、缺失/不完整终态、启动时/回调中取消、重复取消、来源去重、关窗等待及仍需恢复确认。窗口插件模拟不替代实机关窗证据。
- 真实来源更新取消：隔离目录 p1-state，release CLI 的 patchmoe update_source首事件后发送取消，收到sequence 0/1的index开始/停止事件，最终task_cancelled，exit=0，control_error=null；证据 `.artifacts/windows-flutter/p2-source-cancel.json`。未写真实游戏文件。
- 新增 AppModel 与四栏目页面、全局/游戏来源、补丁管理、安装对话框和操作记录。调度沿用启动及每60秒检查、只选择启用publisher且到期/非运行中的来源；HGamer不自动更新。match_update和affected_appids通过project_instances更新能力，不重扫全库。
- 共享TypeScript→Dart展示对照9项通过：筛选、外部安装状态、强/弱候选、publisher抑制HGamer、多镜像及手动条目去重、变体顺序和语言排序。测试素材从现有v5夹具派生，由 `scripts/test-flutter-presentation.mjs` 核对共享实现。
- 安装入口直接使用actions.primary.operation与automatic/local_archive能力；本地文件取消不发安装请求。已通过3项安装交互测试：本地ZIP请求字段和release/selection互斥、成功后刷新失败保留结果、Esc受控取消和关闭结果不再次取消。测试曾发现对话框未把初始焦点放入快捷键范围，现已使用Flutter Focus修复；进程仍为可控测试子进程，不代表真实游戏安装通过。
- 静态集成分析通过；后续修改仍需补充受影响检查。P3/P4/P5完整交互、真实文件级安装及P6/P7尚未完成验收。

### 共享契约必要修复

- 沿直接EXE终态路径发现：Rust InstallerOutcome/OutcomeState与validate_outcome早已允许state=failed，但生成schema的InstallerOutcome oneOf漏掉failed，导致合法不完整安装终态被宿主拒绝。已补齐failed + cancelled/failed分支，并增加两项共享夹具；不新增协议版本或新的业务状态。
- `cargo run --locked -p steam-patch-contract-gen -- generate` 与 `cargo test --locked -p steam-patch-protocol`通过（2项单测、3项集成测试）。Dart/TS/Python生成物与消费方后续补验。
- 界面须明确直接EXE未完成时可能留下写入文件，不宣称自动恢复；以核心disposition/cause/recovery为准，不从自由文本反推权限或取消成功。

## 2026-10-22 文件级、发行包与升级验证

- Flutter 全量163项测试通过；新增最小窗口850×560、125%/150%字体、深浅主题4项布局检查通过。真实Windows系统缩放不等同于文本缩放，仍待验收。analyze无问题；随后关窗焦点修正后正在重建并补验。
- 来源专项3项通过：到期publisher调度、停用/手动/HGamer/活动任务排除；真实返回revision用于保存；保存后投影失败单独保留提示；来源展开/折叠不发RPC。
- Python v5宿主13项、TS协议夹具与Flutter共享展示对照通过。新增Flutter依赖清单解析1项、SBOM原回归1项、两平台打包边界8项通过。核心install_tests 62项通过，均在临时库内，包含写入取消、备份损坏、恢复资料和直接写入失败。
- 完整旧状态副本补验：37个游戏、4个来源、5条事务（2 committed、3 superseded）。此前P1部分副本的0条事务不能作为历史迁移证明；完整资料实际位于installer/rust-v1，现已完整保留并读取。证据p1-full-state-probe.json。
- 临时签名ZIP经真实Dart CoreTask→release CLI安装，收到6条事件及committed终态，补丁文件和未相关save.dat核验通过。较大归档第一次未触发writing阶段取消而正常完成，按完成记录；对实际extracting阶段取消后得到failed/cancelled_incomplete，验证直接写入不伪装回滚，新增schema分支被真实终态覆盖。
- 用户授权真实游戏前已完整备份并逐文件SHA-256比对：2303800为12文件/1,379,698,501字节，2743810为10文件/1,515,440,757字节。备份和清单保存在忽略目录p5-real/backups与*-baseline.json；原用户事务库未修改。
- EXE妃神会秘史：最初仓库盘隔离state目录因继承权限导致沙箱private staging DACL失败，未改系统权限；改用正常用户临时目录后真实下载1,228,278,216字节，SHA-256为7336c5c9876bf239811478001741862256610c77298c745d3bc035a8bcdd94f3，实际安装收到1630条事件与committed/completed；所有文件等于原有已补丁基线。随后真实extracting写入取消收到6条事件与failed/cancelled_incomplete，已从校验备份恢复data.xp3，整目录哈希完全等于验收前基线。手动备份恢复不称为核心自动回滚。
- ZIP你妈真漂亮：发布站自动下载返回download_failed/download_http_error，独立HEAD确认HTTP403，未进入写入。已有ZIP哈希519980a217f7ce408f33492d0d872236d230e34fbd68c4aad031d73a1a9d7d2f，内部patch.xp3等于现有已安装文件；另用独立测试签名、本地归档路线验收，不能称为发布站自动路线通过。实际结果待本次任务结束后补记。
- P6脚本改为完整Flutter Release目录，携带DLL/data/plugins、VC++运行库、core/worker、签名资源与许可证；打包前核对必需文件、schema和Dart lock摘要。SBOM包含实际解析Dart包与许可证文本、Flutter/Dart SDK、Rust与仓库npm依赖；3个SDK内部包未自带LICENSE，报告保留license-review标记并指向NOTICES.Z，不伪造许可表达式。
- 便携ZIP与NSIS候选均已生成。NSIS沿用旧安装路径和当前用户卸载身份，升级删除旧主程序并替换快捷方式；卸载按构建文件清单逐项删除，不递归删除用户目录，保留APPDATA恢复资料。
- 无既有已安装客户端时，在p6-upgrade-app目录静默安装历史0.1.0再升级0.2.1：两个安装器均exit0，原目录/Steam Patch Manager卸载键保留，MainBinaryName变为steam_patch_windows.exe，旧exe消失、无重复卸载条目。用完整旧数据副本、仓库外CWD、仅系统PATH启动升级候选，窗口标题正确且Responding=true；尚不能代替页面内容和键盘的视觉验收。
- Windows CI、一键build.bat、npm build:windows、run-windows-check入口、client-guide已接入Flutter和两类包。只做本地验证，未推送或运行远程CI、未发布。
- Windows Computer Use接口按技能读取并尝试初始化，两次超时，reset后重试仍返回“js execution timed out; kernel reset, rerun your request”。未使用猜测坐标或替代API伪造窗口验收；真实125%/150%系统缩放、键盘/长文本视觉及任务关窗操作仍待补。
- 关窗静态复核发现覆盖层保留底层焦点，已用ExcludeFocus/AbsorbPointer阻止覆盖期间底层交互，保留页面上下文；正在补验新候选。

### 仍待完成

1. ZIP本地实际终态、写入取消与基线恢复；发布站403路线在可访问环境补验。
2. 新候选最终打包/静态检查、旧数据对齐及卸载保留资料证据；来源真实设置/投影/匹配联调。
3. 真实窗口系统缩放、键盘焦点、长名称/错误、安装/取消/关窗操作验收；Computer Use工具故障尚未排除。
4. 必需验收完成后才移除旧apps/windows及专属依赖，验证Decky/共享协议，按用户指令处理提交/合并。当前活动目标未完成。

## 2026-10-22 本轮收尾证据与待验收

- 最终Flutter全量 **169项通过**，release构建成功；随后修正测试支架硬编码本机SDK目录为从package_config解析实际Flutter位置，受影响真实子进程/关窗/安装测试22项补验通过，analyze无问题。此修正仅影响测试；当前便携包包含最新生产代码。
- 真实隔离来源联调通过：set_source_enabled、间隔3→0、游戏订阅关闭→投影→重新订阅、新增停用规则，核心返回revision依次4至11；match_games限定实例12事件、rematch_game 3事件均正常终态；project_instances返回1个实例。独立完整Kagura更新返回source_refreshed、complete=true、470条、exit0；未写真实游戏。
- ZIP本地安装收到1569条事件与committed/completed，整目录文件哈希等于已有已补丁基线。随后实际写入取消收到4条事件与failed/cancelled_incomplete；从校验备份恢复patch.xp3，整目录哈希再次完全等于验收前基线。对应归档/独立测试签名的说明保留；官方自动获取仍因HTTP403待验。
- 真实EXE和ZIP均使用直接覆盖语义。关窗复核发现failed结果此前没有生成退出提示，现已补齐；failed/needs_recovery均保留可见提示后再由用户选择退出，新增回归通过。恢复界面专项验证全局跨游戏范围、取消不执行、逐项恢复失败详情、结果后刷新失败保留；共4项关窗/恢复测试通过。
- 完整Python后端100项通过（2项跳过），Decky类型检查与Rollup构建通过；共享TS/展示契约与打包/SBOM回归继续通过。
- 最新便携ZIP39条目逐项字节比对构建目录一致、无重复条目，已从独立目录启动，只有系统PATH，窗口标题正确且响应。使用正常用户临时APPDATA的完整旧状态副本，实际便携核心探针读取37游戏/4来源/5事务；与旧状态副本的实例ID、事务ID、来源enabled/interval设置全部一致。证据p6-data-continuity.json。此为旧数据与两种核心入口对照，不冒充旧Tauri界面逐项验收。
- 历史0.1.0→最终0.2.1升级→卸载三个步骤均exit0。旧主程序与旧安装器README被清理；安装目录残留0文件，卸载键移除，原APPDATA188文件哈希完全未变。首次卸载发现的旧README残留已修复并复验；证据p6-upgrade-final.json与p6-state-preserved.json。
- 最终NSIS SHA-256：67251ebf707387f151f717f446a408c8833c63df125eb3b22835c4ae7a13aa44。便携ZIP SHA-256：98e2fd26f46fc1d846b80c22dc70e7c38856a3ffb26788e3675dd7f43266ad74。附件仅为本地候选，未发布。
- 按overengineering-review沿本轮宿主、来源、安装、恢复及打包实际路径自审：未发现需保留的无依据复杂度；未增加任务总超时、数据截断上限、通用队列、双通信栈或新后端。schema边界来自既有v5契约，保留以拒绝混装/畸形数据；固定运行文件校验对应真实Flutter分发依赖。审阅中发现的关窗焦点与failed提示缺口已修复。专项审阅不替代未执行的实机视觉检查。
- 已打开最新便携候选并请求用户检查真实深浅主题、Tab/Enter/Esc、最小窗口与125%/150%系统缩放。Windows Computer Use初始化及reset重试均超时，尚不能补足视觉和实际窗口输入证据。待反馈前保持P7待验，保留旧端，不提前移除或合并。

当前剩余验收：真实窗口/原生文件选择及来源外链、任务期间关窗的实际输入；官方ZIP在线获取（403）；旧/新界面的细项对照。自动化、文件级和升级证据已完成的项目不无理由重跑。目标保持活动，未标记完成。

## 2026-10-22 继续核对 P7

- 上一目标轮完成了旧宿主只读执行对照：从旧main.rs原样提取core_command/read_all/response_error/validate_response/protocol_error/method_is_task/run_one_shot，在忽略目录独立运行。只用AppHandle路径stub指定同一核心；没有替换其JSONL或验证逻辑。与Dart宿主顺序读取同一完整旧状态副本，scan、list_transactions、list_sources的整个载荷逐项相等，包含候选、动作DTO和历史记录。证据p7-host-equivalence.json及old-host-probe/extracted-functions.json；此项不冒充Tauri窗口或路径解析的实际操作。
- 上轮额度曾令自动审批无法完成，相关操作没有执行。本轮正常审批已恢复。Windows Computer Use初始化仍超时，没有采用猜测输入或标记窗口验收通过。
- 用真实扫描数据、本机中文字体和Material图标渲染深浅主题、1180×760/850×560、150%字体下的概览、补丁管理和来源，共14张包含错误详情的本地图片；不提交字体、不把渲染图冒充系统DPI截图。渲染检查发现长错误会把顶部Column撑出屏幕（实测overflow 2078px）。
- 修复：将读取/投影/目录错误合并为简短提示，完整信息放入可滚动详情；不截断原错误，不改变读取成功/失败事实，不丢上次成功游戏库。最小窗口150%字体的永久回归通过，详情保留完整开头、路径与末尾，可关闭返回游戏库。真实数据渲染复验通过。
- 当前静态分析无问题，完整Flutter单元/Widget测试170项通过。新增SDK自带integration_test入口，用本项目Windows窗口、实际AppLifecycle与真实核心在临时AppID42上验证原生关闭→取消→终态。为了不让测试断开连接，finishExit在测试中只记录已请求退出；真实销毁窗口仍需另行验收。代码已写，原生执行尚待权限准备，不记为通过。
- 新增prepare-windows-native-acceptance.py，可复现生成操作系统临时目录中的Steam库、签名ZIP及独立状态；测试拒绝真实库和非42实例，核对无关存档并恢复临时原文件。已成功生成夹具。测试数据大小用于观察既有进度采样，不是生产限制。
- integration_test依赖变化令Flutter重新生成插件链接，本机普通进程无符号链接权限。沿用此前已授权的管理员pub get准备入口，Windows返回“操作已被用户取消”；没有执行准备、没有改系统开发者模式、没有反复弹出。已向用户明确询问是否重新弹出。
- 当前桌面候选和ZIP/NSIS仍是上一已验证构建，尚不包含本轮长错误修复；待符号链接准备完成后运行原生集成测试、刷新Dart依赖SBOM并重建两个候选，不能沿用旧哈希声称当前源码已打包。旧端仍保留，目标未完成。

原生集成验收准备/运行（待验证）：

```powershell
$nativeSpec = python scripts/prepare-windows-native-acceptance.py
Set-Location apps/windows-flutter
flutter test integration_test/windows_acceptance_test.dart -d windows "--dart-define=STEAM_PATCH_NATIVE_SPEC=$nativeSpec"
```

执行前需已有release CLI/worker及正常Flutter插件链接。命令只写新建的临时库和忽略的证据目录；没有对真实游戏的新增授权。Windows 125%/150%系统缩放、原生文件选择/外链、实际销毁窗口仍按剩余验收记录处理。

### 本轮退出核对

- 原生验收夹具已由release CLI实际scan复核：仅1个AppID42实例、catalog_error=null、local_archive.available=true；没有执行安装，证据p7-native-fixture-verified.json。
- 最后一次analyze无问题，170项单元/Widget测试通过；深浅主题、真实中文/图标字体、长错误详情渲染复验通过。原生集成测试尚未执行，最新源码尚未重建ZIP/NSIS。
- Windows实机验收工具初始化失败已跨多个目标轮重复，用户视觉反馈尚未收到；可替代的本项目原生集成测试当前又被插件符号链接权限阻断，管理员准备UAC返回用户取消，正等待是否重弹的明确答复。此前目标轮完成了对照与验证，本轮也完成了实际修复，不能将其记为整计划完成。
- 当前没有可独立推进的必需验收：需用户允许重新执行管理员pub get（只建链接）、完成真实窗口反馈，及在ZIP发布站可访问时复核在线路线。未绕过UAC、未移除旧端、未合并/发布。目标按阻塞审计处理，收到用户输入后继续剩余工作。


## 2026-10-22 最终验收与旧端退役

- 用户明确同意重新弹出管理员 pub get。首次命令虽返回0，但之前失败留下的插件元数据使 Flutter 跳过链接创建；沿固定 SDK 的 refreshPluginsList 路径确认后，仅备份并移除生成的 .flutter-plugins-dependencies，再执行已授权的同一命令。四个 Windows 插件真实符号链接已建立；未改系统开发者模式或安全设置。
- 原生 Windows 集成测试通过：本项目真实窗口在临时 AppID42 上启动真实 Rust ZIP 写入，经 windowManager.close 触发原生关闭事件并发送取消，最终 failed/cancelled_incomplete、exit0、无诊断。返回应用后实际重新读取事务，保留本次记录；无关存档不变，临时原文件已恢复。证据 p7-native-test.log、p7-native/steam-patch-windows-acceptance-bg3o__ky/native-close-result.json 及三张窗口客户区渲染截图。该测试对 finishExit 的替换仅用于不断开测试连接，不把它称为销毁窗口证明。
- 原生测试截图发现关窗时会将主动跳过刷新误写为刷新失败，且取消原因重复。已修正为“应用正在关闭；返回应用后会重新读取游戏与记录状态”，取消不完整标题明确为“已取消，安装未完成”；返回应用的生命周期回调刷新真实模型。新增回归通过。最终 analyze 无问题、Flutter全量171项通过；真实中文/图标与长错误14张渲染检查通过。
- release 便携包在仅系统 PATH、仓库外工作目录、完整旧状态副本下正常启动，标题正确且响应。发送正常关窗请求后等待在途一次性核心调用，后续检查进程已退出；没有强杀、没有因观察等待时长终止正常任务。证据 p7-release-window-close.json；实际销毁窗口由此单独覆盖。
- 用户对最新候选明确回复“全部通过”：系统/浅色/深色主题、850×560最小窗口、Windows 125%/150%系统缩放、Tab/Enter/Esc、本地ZIP原生文件选择取消和来源外链。该证据为用户实机反馈，不冒充自动化操作或系统截图。
- 官方ZIP下载及来源页复查仍为Cloudflare HTTP403。用户明确回复“记录外部遗留项，完成并合并”。因此本次验收接受此唯一外部在线路线缺项；不得将独立签名本地ZIP证据改称发布站自动下载通过。没有绕过站点挑战、改变核心错误语义或增加盲目重试。
- 在上述验收后移出旧 apps/windows，移除其 npm/Cargo workspace 成员及 Tauri/Vite 专属锁文件依赖；旧文件仅保留于Git历史和忽略的本地验收备份。保留 Decky/共享UI与协议，更新当前开发指南；历史发行说明保留原事实。根 typecheck 现在仅检查仍使用TypeScript的Decky，Windows由Flutter构建入口执行analyze/test。
- 受清理影响的检查通过：Decky类型检查与Rollup构建、TS契约与9项Dart展示对照、Rust契约生成检查、协议2单测+3集成、release CLI/worker构建、SBOM 2项回归。此前后端100项通过（2跳过）、安装器62项、真实两款游戏文件级安装/取消/基线恢复、旧数据37游戏/4来源/5事务与历史升级/卸载证据继续有效；未无理由重复真实写入。
- 按overengineering-review复核本轮实际路径：长错误滚动详情解决已复现溢出；关窗刷新跳过和返回回调对应真实生命周期。未新增任务超时、截断上限、强杀、重试框架或双宿主。测试夹具AppID42/临时目录检查仅保护验收数据，不限制生产输入。退役删除旧路径，未保留运行时兼容分支。
- 最终锁文件下SBOM为476组件，包含实际Flutter/Dart、Rust与剩余npm依赖；6个SDK内部包没有独立LICENSE，报告保留license-review及Flutter NOTICES指向，不伪造许可。重建便携与NSIS并复验打包边界；最终附件哈希见随后记录。

### 遗留与交付边界

- 唯一接受的外部遗留：官方ZIP站点Cloudflare HTTP403，需在站点可访问时重新核验在线获取；本地ZIP和EXE实际安装/取消已通过，真实游戏文件均恢复到验收前基线。
- Computer Use运行时故障未作为产品缺陷处理；实际系统交互已由用户验收补齐。
- 用户已授权按计划完成并合回main。仅本地提交/合并；没有执行Git推送、Release发布或Steam Deck部署，未运行远程CI。

### 最终候选附件（2026-10-22）

- 便携包：`.artifacts/steam-patch-manager-windows-portable.zip`，18,304,374字节，SHA-256 `6a825485342515169e78a053c7feb038806bf56b8af39f6e403a015d2caa1c39`。
- NSIS：`.artifacts/Steam.Hentai.Patch_0.2.1_x64-setup.exe`，SHA-256 `40aeabe9c28f7f49866c7cabf285c6041ed267453edd63178e7fe18b7cc293ec`。
- 最终便携包39条目与构建目录逐项字节一致，未混入旧端或验收夹具；打包8项回归通过。证据 p7-final-portable-verified.json、p7-final-package-tests.log、p7-final-sbom.json。
- 上文早期附件哈希属于历史构建，不代表本次交付。最终包仅因旧端依赖移除刷新核心/SBOM/指南，Flutter界面代码与用户已验收候选一致。

### 本地提交与合并准备

- `d662b6a`：补齐直接安装未完成的合法协议终态及共享夹具。
- `a14c807`：Flutter客户端、深浅主题、任务/来源/安装/恢复、原生验收、构建打包与旧端退役。
- 文档按仓库要求独立提交；提交前暂存区与diff检查通过，仅包含本功能及必要契约修复，没有验收归档、真实游戏文件、私钥或工具链。按用户明确授权合回main，保留外部403遗留。

## 关窗验收证据更正与后续修复

后续用户报告关闭窗口卡顿。本轮原生退出复验发现旧候选flutter_windows.dll崩溃，Windows错误处理亦会延长退出。此前本记录的release关闭检查仅确认进程结束，未读取退出码，不能单独证明正常销毁窗口。已在[Windows完善施工记录](../windows-visual-polish/progress.md)记录读取协作取消及正常WM_CLOSE/WM_DESTROY修复，并以真实release退出码0和880ms实测补足证据；不把此前异常退出追认成正常退出。

## 2026-10-23 Modern 视觉方案（方案阶段历史）

- 用户认为现有 UI 不美观，提供视觉参考图，允许使用 Steam 自定义艺术作品，并明确要求先设计方案。
- 静态核对当前 Flutter 的主题、游戏库、补丁管理、GameInstall 字段、DesktopServices 和窗口配置。现有界面没有游戏图片接入，尚无打开游戏目录方法；不将来源名称作为游戏开发商。
- 新增 visual-design.md：定义深色主视觉与浅色对应、封面导航、Hero 横幅、补丁卡、底栏及三档窗口布局，保留四栏目及原安装动作契约；资料链接见方案 §6。
- 艺术作品方案采用本机自定义优先、本机默认图其次、缺图占位；本机路径、账号与图片格式待真实样本核验。未调用 Steam 账号接口、下载游戏美术或安装图源插件。
- 已进行 overengineering-review 计划静态自审，结论见方案 §9；未委派子 Agent。没有实施代码审阅或真实窗口验收。
- 工作区已有 windows-visual-polish 的代码与文档未提交修改；本轮在当前功能分支复用所属目录，保留这些改动，不切换或重置分支，不修改功能代码、提交、合并或部署。
- 后续：用户确认视觉方案后，协调既有未提交工作，按 V1–V4 实施并补充必要验证；旧 P0–P7 的验收不代表本轮视觉迭代完成。
- 文档检查：本轮四份文档的相对链接检查、git diff --check 通过；发现原设计引用已退役旧端的四个失效链接，已改为带历史提交依据的路径说明。未运行应用测试或构建，本轮无功能代码修改。

## 2026-10-23 Modern 视觉施工

### 实现范围

- 用户随后明确要求「开始施工」并设立任务目标；在当前 `feature/windows-visual-polish` 功能分支复用本功能目录，保留原有未提交的图标、下载速度与关窗修复，未切换/重置分支，未提交、合并或发布。
- `app.dart` 落实深蓝灰与浅色对应表面、蓝色主按钮、细边框、统一圆角及输入框。主按钮深色值调整为 `#346BE0`，使白色按钮正文达到方案要求的对比度。
- `library_page.dart` 实现品牌工具栏、四项筛选及真实计数、封面游戏列表、带遮罩的 Hero、四栏目及状态徽标；保留需处理筛选与原选择逻辑。小窗口的搜索/筛选和空状态可滚动，长标题在横幅阅读区可完整查看。
- `patch_management_page.dart` 实现竖封面＋条件/状态/下载选择的主卡、宽窗 AppID/build 摘要、技术详情与固定底栏。来源与元信息同排换行，收紧实际截图中偏大的纵向留白；不修改安装动作、手动确认、本地 ZIP、取消和结果契约。
- `DesktopServices` 增加打开所选实例目录；错误用就近反馈说明。通过隔离测试验证调用路径及失败不会启动安装，未为测试打开真实游戏目录或启动资源管理器。
- 复用 window_manager 的拖动、双击、系统菜单及窗口控制接口合并标题区，关闭仍走既有 AppLifecycle。初始化核心失败的页面也保留同一工具栏，不因隐藏系统标题栏而丢失关闭入口。
- 原生 runner 增加只读 `steamArtworkContext` 方法，读取 Steam 安装根及当前账号；Dart 读取当前/唯一最近账号的自定义图与按 AppID 分目录的 Steam 默认图（含哈希子目录和中文变体），不改核心协议或全盘扫描。
- 图像按实际展示尺寸解码；列表优先自定义作品，其次默认竖封面，避免宽图过度裁切。扫描刷新使用图片键代次，使同路径替换的图片也能重读；切换游戏等待新图时清除旧图。缺图/无法解码时显示带诊断的占位，不阻断业务。

### 自动化与实际发现

- 完整 Flutter 回归 205 项通过，最终源代码回归日志 `.artifacts/windows-flutter/modern-final-regression.log`。覆盖原安装/取消/恢复/来源/宿主，以及新增图片账号隔离、中文/哈希图片、缺图、刷新、切换游戏、目录操作、主题对比度和布局。
- 新增布局矩阵：1440×900、1180×760、850×560，深浅主题与 1.0/1.25/1.5 文字缩放，共 18 组；验证主要按钮可点击、正文滚动不移动底栏、Shift+Tab/Tab 能在底栏操作间往返。普通文字与按钮对比度检查通过。
- 第一轮发现最小窗口 150% 字体下空状态溢出；沿实际报错控件定位并改为滚动阅读。图片测试中的 Windows 路径分隔符差异已统一，不调整业务阈值来掩盖失败。
- 当前机器的 VS 2022 已不可用，VS 2026 缺少 Flutter 自动发现所需组件，标准 `flutter build windows`/`flutter test -d windows` 被工具链检测阻断。诊断见 `modern-doctor.log` / `modern-standard-build.log`。
- 使用本机已有 VS 2026 中的 MSVC 14.44、Android SDK CMake 3.18.1 和 Ninja 完成真实编译。直接构建时复现原模板 `/wd"4100"` 被 Ninja 传为无效数值参数，改为等效 `/wd4100`；未安装软件、修改 Flutter SDK 或系统配置。具体本机命令保存在 `.artifacts/windows-flutter/build-modern-native.cmd` 和 `build-modern-release.cmd`，这是当前环境的构建证据，不是新增产品配置层。

### 原生视觉验收

- 只读目标 `integration_test/modern_visual_test.dart` 使用已有扫描快照和当前 Steam 图片，不启动核心进程，不做游戏写入。标准 Flutter 启动器受上述环境问题阻断，改由现有 CMake/Ninja 编译该验收入口并直接启动实际 Windows release 程序。
- 原生 runner 图片方法返回真实 Hero / 中文竖封面 / Header，诊断为空；具体来源见 `.artifacts/windows-flutter/modern-native-renders/artwork-context.json`。图片未打包进候选或 Git。
- 验收脚本显式注册模拟文本输入，避免 Windows integration binding 默认使用系统键盘时，`tester.enterText` 在 release 中没有真正过滤游戏；之前的找不到列表项不记作产品通过，也未通过修改产品筛选绕过。
- 关窗测试等待实际 WM_CLOSE → AppLifecycle 回调的完成信号，避免在异步通知前断言；放在截图矩阵之后。最大化/还原与关闭回调已通过。测试保留窗口供框架输出结果，不将该拦截测试当作生产进程退出码验证。
- 已完成深浅主题 × 三档外部窗口尺寸、最小窗口 150% 文字缩放的 18 张原生截图，覆盖概览、补丁管理以及默认窗口的记录、订阅、全局来源；结果及实际客户端尺寸见 `modern-native-renders/result.json` / `metrics.json`，每组实际客户区略小于外部窗口，未将两者混写。
- 截图中的补丁及安装状态来自既有快照，用于视觉验证；不代表本轮重新检测或安装了这些游戏。本机无可用自定义 grid 图片，真实用户自定义资源仍未实测，优先级/账号与文件名行为以隔离夹具验证。

### 过度设计实现自审

依照 overengineering-review 沿列表选择→图片解析→补丁选择→安装入口/结果、目录打开、原生关窗检查，未发现需要新增机制的项目。图片键代次针对 Flutter FileImage 按同路径缓存导致用户刷新仍见旧图的实际行为；占位只处理展示资源失败。保留既有核心能力与错误，不新增任务中心、图片下载队列、持久缓存、任意超时/大小限制或未知字段抓取。

本轮为静态自审结合实际测试及原生界面证据，未委派独立审阅；不代替用户对最终视觉喜好的验收。后续候选构建、正常启动/退出与文档终检结果继续记在下节。

### 最终候选与验收边界

| 阶段 | 最终状态 |
| --- | --- |
| V1 视觉基础与框架 | 已实现，深浅色、封面导航、Hero、状态、栏目及窗口工具栏自动化与原生渲染通过 |
| V2 艺术作品 | 已实现，本机默认图及原生位置读取通过；自定义优先级/账号隔离/最近账号以夹具验证 |
| V3 补丁内容与底栏 | 已实现，原安装契约回归、固定底栏、目录调用及错误提示通过 |
| V4 统一与验收 | 205 项完整回归、静态分析、18 张原生截图、普通入口 release 构建与启动/退出检查通过；用户视觉验收待确认 |

- 最终完整回归：`modern-final-regression.log`，205 项通过。收尾复核修正最近账号解析对合法昵称中花括号/转义引号的处理，4 项图片测试复验通过（`modern-artwork-final.log`）；静态终检 `modern-final-analyze.log` 无问题。未因文档或错误文案调整重复无关安装矩阵。
- 最终原生 UI 日志：`modern-native-complete-stdout.log`，`All tests passed`；18 张截图（排除早期 `missing-row.png` 诊断图），`modern-native-renders/result.json` 为 `passed: true`。标题与状态改为可换行的同排布局，复验最小窗口 150% 字体时状态不再被横幅底边截断。
- 普通入口候选来自 `lib/main.dart`，构建日志 `.artifacts/windows-flutter/modern-release-verified.log`。可运行目录 `.artifacts/windows-flutter/modern-ui-candidate/`，入口 `steam_patch_windows.exe`；包含本次 Flutter 程序、既有核心/worker、运行时和来源资源，不含真实游戏图片或扫描快照，不是对外发行包。
- 候选 EXE SHA-256：`7849893EEC6B14A5D408203234BA81F08BA07537B4FF369F48AA5EEDBE6F1B67`；Dart AOT `data/app.so` SHA-256：`B197C2A32A55EBDF7BE7DCB18ADBE2ABC1755FE8E4A9DBB79004351CE9260929`。完整文件校验见 `.artifacts/windows-flutter/modern-candidate-manifest.json`；品牌 PNG 与原图 SHA-256 完全一致，没有重绘品牌。
- 对最终普通入口候选使用独立 `APPDATA` 启动，出现实际原生窗口后请求正常关闭，退出码 **0**、关闭 **890ms**（早一轮候选为 802ms）；证据 `.artifacts/windows-flutter/modern-smoke-result.json`。未使用强杀或任意退出超时，不触碰用户原状态目录，也没有执行游戏安装。早期失败后保留关闭拦截的只读测试进程仅按已记录 PID/路径清理，不作为生产关窗证据。
- 未执行：真实用户自定义 grid 图片、系统显示设置在 125%/150% 两档间切换、用户最终视觉验收。本轮的文字缩放矩阵不冒充系统 DPI 切换；原生截图是在当前系统环境取得。安装/取消/恢复沿用既有有效记录并通过相关回归，本轮没有新增真实游戏写入。
- 标准 Flutter Windows 构建仍依赖补齐本机 VS 组件；现有 MSVC/CMake/Ninja 的直接构建与实际运行已验证，不将其写成标准命令已恢复。没有安装软件或改动系统显示设置。
- 所有既有未提交工作保持；没有提交、合并、推送、Release 或 Steam Deck 部署。新增图片与界面代码仍在工作区，待用户查看最终视觉效果。
- 文档终检：四份关联文档的相对链接与 `git diff --check` 通过；最终图片枚举采用 Dart 标准跟随链接行为，没有额外排除合法链接资源。候选图片文件核查仅包含项目品牌 Logo，没有复制 Steam 游戏艺术作品。

### 2026-10-23 Git 提交

- 用户明确要求提交 Git，按仓库技能与功能/文档拆分规则执行本地提交。
- `f41e30d`：Modern UI、Steam 艺术作品读取、窗口工具栏、目录操作与必要测试；不包含图像缓存、截图、候选程序或工具链。
- 此前同分支的下载速度 `c4e962f`、关窗修复 `ca62c44`、品牌图标 `aed058a` 分别提交，相关文档单独归档。
- 本功能文档独立提交，沿用已完成验证，未重复运行测试、修改功能代码、合并、推送或发布。上文未提交的表述保留为当时施工状态，以本节为最新记录。
