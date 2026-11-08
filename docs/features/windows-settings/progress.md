# Windows 设置入口与缓存管理施工记录

设计：[design.md](design.md)。示意：[ux-prototype.html](ux-prototype.html)。

## 当前状态

功能已实现，37 项 Flutter 定向检查（含 1 项真实字体渲染矩阵）、静态分析、格式检查、13 项核心缓存回归及 6 次隔离核心服务调用通过。原生窗口和用户视觉/游戏验收待完成。未清理真实缓存；功能代码已本地提交，本次按用户明确要求提交文档并合并本地 main，未推送、发布或部署。

## 2026-11-09：方案阶段（历史）

- 核对 main 工作区干净，创建 feature/windows-settings 用于本功能的设计与后续实现。
- 核对 Windows 游戏/来源切换、AppUpdatePanel 常驻版本行及自动提示条件、应用级更新初始化和最近检查时间字段。
- 核对 cache_status / cleanup_cache 已在协议和核心实现；确认清理目录、受保护文件、安装互斥和 partial/failed 等结果，首版无需新增后端分类能力。
- 完成三页签、正常底栏隐藏、新版本常驻、下载阶段延续、缓存确认与错误反馈设计，并完成过度设计静态审阅。
- UX 中的版本、占用、来源检查时间、成功/失败均为演示数据，交互不调用核心、不联网，不代表功能已经实现。

## 方案阶段验证与待办（历史）

- 已执行：代码只读核对、方案过度设计审阅；文档相对链接检查、HTML ID 唯一性与脚本引用目标检查通过。缓存页独立设计图 ux-cache.png 已打开检查，文字与布局无截断；该图为绘制示意，非实际应用截图。
- HTML 浏览器自动截图未产出文件，因此未声称浏览器交互、深浅色或窄窗口已通过视觉验收；交互示意仍待用户查看及实现阶段验证。
- 待实现：设置容器、关于页更新动作接线、条件底栏、缓存管理。
- 待验证：Flutter 定向测试/analyze、真实窗口深浅主题与小窗口、隔离目录清理验证、用户 UX 验收。
- 发布前完整 CI、线上 CI、设备/游戏验收均未执行；本轮仅方案，不以示意替代验证。


## 2026-11-09：实现与定向验证

### 实际修改

- `LibraryPage` 一级入口改为齿轮“设置”，有提示与文字语义；游戏区与设置区使用 IndexedStack 保留已有列表、筛选、搜索、选择和页面状态。侧栏 232px，采用现有主题的蓝色强调，来源页直接复用 GlobalSourcesPage。
- 新增设置页签“全局来源 / 缓存管理 / 关于”；首次默认来源页，再次进入保留最近页签。内容区独立滚动，不增加路由库或持久化导航配置。
- `AppUpdatePanel` 删除恒显版本行、稍后关闭及 lastCheckWasManual 门槛。版本和检查状态进入关于页；同一 AppUpdate 通过小型 AppUpdateScope 提供详情动作。应用级 initialize 保持不变。
- 自动与手动发现新版本后均显示全局底栏，关闭详情仍保留；下载显示实际字节，取消/失败保留详情，ready 保留重启入口。显式跳过隐藏当前版本，更高版本重新提示。
- 保留复查期间和失败时的已发现版本；ready 不发起复查，避免丢失已下载入口。准备重启失败经原模型通知，关于与底栏同步错误；不改原检查调度、下载校验、安装或退出协议。
- 新增由 AppModel 持有的 CacheModel；首次打开缓存页才统计。调用现有 cache_status / cleanup_cache，不改 Rust、协议或生成物。重复点击禁用，切页不丢失结果，不添加超时、重试或清理框架。
- 缓存页分别展示占用、实际释放量、部分统计、部分失败、占用、错误详情和手动重试。确认一次后清理；清理结果不被后续统计错误覆盖。SelectableText 的滚动存储与详情展开状态使用独立 key，避免 bool/double 状态冲突。
- 更新原有缩放与原生视觉测试的入口，新增设置回归及可重跑的原生夹具视觉测试；原生测试只用假核心数据，不执行更新/清理真实缓存。

### Windows 定向验证

执行目录为 `apps/windows-flutter`，Flutter/Dart 使用仓库 `.tools/flutter/bin/`；Flutter 的 APPDATA 指向仓库 `.artifacts/windows-settings/profile`，避免写用户配置。Flutter 测试运行器的本机回环连接被沙箱阻止后，在授权的沙箱外重跑通过；不是产品连接失败。

| 范围 / step | 命令与输入 | 结果 / 证据 |
| --- | --- | --- |
| Flutter 单元与 widget | `flutter test --no-pub test/settings_page_test.dart test/app_update_panel_test.dart test/app_update_test.dart test/library_page_test.dart test/scaled_layout_test.dart test/global_sources_layout_test.dart ../../.artifacts/windows-settings/widget_visual_test.dart --reporter expanded` | **37 项通过**。含切页清理不丢结果、禁重复点击、确认取消、部分统计、完成后统计失败、partial/failed/busy、空统计与 installer_busy、返回游戏选择/搜索/筛选、无底栏、自动/手动更新、下载进度/取消/失败/ready/跳过、更高版本提示、来源原操作与布局。日志 `.artifacts/windows-settings/flutter-tests.log`。 |
| Flutter analyze | `flutter analyze --no-pub` | **No issues found**，日志 `.artifacts/windows-settings/analyze.log`。 |
| Dart 格式 | `dart format --output=none --set-exit-if-changed`，全部 13 个改动/新增 Dart 文件 | **0 changed**；无 Rust 改动，因此不触发 Rust fmt / Clippy。 |
| Flutter 真实字体渲染 | 上述临时 widget_visual_test.dart，Segoe UI / Microsoft YaHei UI / MaterialIcons；1180×760（1.0 倍文字）、850×560（1.5 倍文字），深浅主题 × 三页签 | **12 张渲染图生成**，见 `.artifacts/windows-settings/widget-visual/`。已目视检查深色宽屏缓存、浅色宽屏来源、深色窄屏关于、浅色窄屏缓存；修正窄屏标签换行后复验。图中 0.3.0 / v0.4.0 与 2KB 为夹具值，不是实际运行数据或原生窗口截图。内容通过滚动查看，底栏独立占用布局。 |
| 原生 Windows 测试尝试 | `flutter test --no-pub -d windows integration_test/settings_visual_test.dart --reporter expanded` | Debug 构建成功；**窗口测试未运行**：已安装 PatchHub 正在运行，原生全局单实例锁让测试进程转向现有窗口后退出，Flutter 无法建立 debug connection。已核对 runner/main.cpp 与进程路径，未关闭/修改现有实例。日志 `.artifacts/windows-settings/native-visual.log`。 |
| 核心隔离缓存回归 | 根目录 `cargo test -p patchhub-core --locked installer::cache::tests --lib` | **11 通过，1 原有忽略**（超十万真实条目的压力用例）。覆盖保护数据/恢复材料、实际释放量、安装共享锁、深宽目录与任务元数据回收。日志 `.artifacts/windows-settings/cache-core.log`。 |
| 核心公共边界 | `cargo test -p patchhub-core --locked --test cache` | **2 通过**，含空状态只读与 junction 越界拒绝；沙箱拒绝临时 junction 创建后在授权的沙箱外重跑通过。日志 `.artifacts/windows-settings/cache-boundary.log`。 |
| 当前核心构建 / JSON 服务 | `cargo build -p patchhub-cli --bin patchhub-core --locked`，随后 `serve` 的 6 次 cache_status / cleanup_cache 调用 | **通过**。状态目录为 `.artifacts/windows-settings/cache-service-*/state`；空统计→空清理→三目录统计 15B→完成实际释放 15B→空统计→含恢复材料的 partial 实际释放 3B。来源/订阅标记、receipt、游戏补丁标记和 .backup 恢复文件均保留。报告 `.artifacts/windows-settings/cache-service.json`，构建日志 `core-build.log`；未触及用户真实缓存或游戏。 |

以上 Rust 构建出现增量缓存硬链接退化为复制的工具链警告，不影响通过结果。未执行超十万文件压力项、发布前完整 CI matrix 或线上 CI。

### 过度设计审阅

按仓库 overengineering-review 技能，基于 feature/windows-settings 本次未提交实现，沿“齿轮入口→设置/返回→原来源操作”、“关于→同一更新模型/详情→下载/取消/失败/跳过”和“缓存统计→一次确认→核心清理→结果/再统计”追踪实际路径，**未发现需修改的过度设计项**。

状态仅用于页签保留、真实在途任务和结果区分；核心调用仍由现有 CoreClient 跟踪。禁用重复操作对应真实重复清理，确认对应删除下载包的影响，错误直接展示；目录边界、互斥和恢复材料保护仍由现有核心执行。未新增硬上限、通用导航/清理框架、轮询、任意超时、自动重试或失败假成功。删除旧“稍后”与手动检查屏蔽的全部生产分支/专属字段。此结论为实际代码静态审阅与上述定向验证，不代表原生窗口、真实安装或用户验收通过。

### 剩余验收

- 已安装实例退出后重跑原生 settings_visual_test.dart，检查真实窗口尺寸、DPI、键盘焦点、滚动与深浅主题；当前原生窗口未通过，不能以 widget 渲染或原设计 PNG 替代。
- 用户确认实际界面；较长真实游戏列表的返回位置、真实来源后台任务跨页、真实安装占用、真实下载/重启更新及游戏体验仍待验收。本次使用夹具/隔离服务，不修改真实来源或游戏。
- 线上 CI 与发布前完整检查不属于本次执行范围；本次仅按用户追加授权执行本地提交及 main 合并，不推送、发布或部署。


## 2026-11-09：本地提交与 main 合并

- 用户追加要求“提交 GIT 并合并 main”，沿用本 session 已完成的有效验证，不重复运行测试。
- 功能及必要测试单独提交：`86066c4 feat(windows): 新增设置页签与缓存管理并调整更新入口`；方案、交互示意、功能索引与验证记录另作文档提交。
- 合并前核对本地 main 与功能分支原始基线均为 `b967fb6`，采用快进方式将功能提交纳入本地 main；不改写历史，保留功能分支。
- 原生窗口/真实使用待验收结论不因本地合并而改变；未推送、发布或部署。
