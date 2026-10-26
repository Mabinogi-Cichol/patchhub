# Windows 视觉与下载进度施工记录

设计：[design.md](design.md)。

## 当前状态

视觉、下载速度及关窗迟缓修复已实现，完整179项测试、真实数据渲染和release正常退出实测通过。2026-10-23 已按用户要求在 feature/windows-visual-polish 分别本地提交；未合并或发布。中性主题随后由 [Windows Modern 视觉迭代](../windows-ux-redesign/design.md)替代，原验证与修改事实保留为历史记录，不冒充用户实机验收。

## 2026-10-27 深色主题中性化（本轮）

- 分支沿用当前界面修复 `fix/windows-source-theme-layout`；按用户反馈调整 Modern 深色主题的蓝灰表面为黑与深灰，设计见对应补充章节。
- 已实现：深色采用 monochrome 基础表面，页面背景 #101010、卡片/标题栏/侧栏 #191919、输入容器 #242424、选中背景 #303030，文字与边框使用中性灰。蓝色保留为小面积操作、链接、焦点、进度强调；浅色方案保持。
- `flutter test test/modern_desktop_test.dart test/library_page_test.dart test/scaled_layout_test.dart test/global_sources_layout_test.dart test/modern_layout_test.dart`：38 项通过，包含所有深色 Material 3 表面灰阶、背景/卡片/输入层次、正文与次级文字在各容器上至少 4.5:1 对比度，以及深浅主题、窗口与文字缩放回归。
- 使用临时 Flutter 渲染测试加载 Windows 字体与 MaterialIcons，生成并检查 `.artifacts/dark-neutral-preview.png`（1400×900、夹具来源数据）；背景、侧栏、卡片、输入区均为黑灰，图标及蓝色小面积强调正常。临时测试已移除。
- 过度设计复核：变更集中于统一 ColorScheme 和已有对比度测试，无逐页颜色分支、主题状态、配置或依赖新增；未发现需修改项。
- 相关文件 `flutter analyze` 无问题，`git diff --check` 通过；`scripts/build-windows-flutter.ps1 -Preview -SkipRust` 构建通过，本地预览位于 `apps/windows-flutter/build/windows/x64/runner/Release/patchhub.exe`。原生窗口验收、安装版替换未执行，历史验证记录保留。

## 2026-10-22 实现

- 下载速度：新增 DownloadRate，以相邻已接受下载事件的bytes_done增量除以Stopwatch单调时间差；接入CoreTask事件验证/序列去重之后，避免重复事件影响采样。首次下载样本显示“下载速度：计算中…”，随后显示B/s、KB/s、MB/s或GB/s。事件字节不增长时显示0 B/s；计数重置重新采样，离开下载阶段清空速度。没有改Rust或v5协议、没有额外定时器、重试、平滑窗口或大小上限。速度随既有进度事件刷新，表示相邻采样间的速率。
- 主题：Material 3 monochrome中性色板；浅色白底/黑字/灰色容器、深色炭灰底/浅灰字。普通按钮和分段选中态保持黑白灰，进度、TextButton链接、文本选择及控件焦点使用蓝色；不再使用原绿色种子色。错误保留原有语义提示。系统/浅色/深色切换机制不变。
- 图标：原样读取 docs/images/branding/logo.png，转换为16/24/32/48/64/128/256像素ICO并替换Runner资源；NSIS显式引用同一ICO用于安装/卸载程序。没有重绘或改变品牌颜色。

## 验证结果

- flutter analyze 无问题；完整Flutter测试174项通过，包括新增速度数学/阶段重置与对话框具体速度和字节进度测试。针对性下载/安装/取消及850×560、125%/150%字体布局11项通过。证据：.artifacts/windows-flutter/visual-polish-build.log、visual-polish-targeted.log。
- 新增Widget测试首次在假异步区启动子进程而等待，修正为runAsync中启动及等待事件后通过；仅停止已确认悬挂的测试宿主，没有终止安装核心或真实应用。
- 使用真实扫描DTO、本机中文与图标字体渲染14张图：两种主题、1180×760及850×560/150%字体、概览/补丁/来源和长错误详情；无布局异常，已查看深浅主题补丁页。渲染不等同Windows系统DPI截图，也没有重复本轮不受影响的真实游戏安装。证据：visual-polish-render.log、visual-polish-renders/。
- ICO含全部七种尺寸，256px帧RGBA逐像素等于原图LANCZOS缩放，透明与不透明alpha均保留；最终release应用及NSIS二进制均包含这七帧原始图标数据。证据：visual-polish-icon.json、visual-polish-embedded-icons.json。
- Windows release构建成功；便携ZIP39条目逐文件等于最终构建目录。NSIS使用同一品牌ICO编译成功。新包已生成，现有用户运行的旧窗口未被强行关闭或覆盖。
- 按overengineering-review沿事件接收→序列去重→速率采样→进度显示、主题及Runner/NSIS资源路径自审，未发现需修改的过度设计项。计数重置处理对应阶段/任务内重新下载的实际字节语义；首次两点采样与单调时间对应速率定义。未新增生产限制、后台任务或兼容宿主。

## 本地候选

- 便携ZIP：.artifacts/steam-patch-manager-windows-portable.zip，18,368,621字节，SHA-256 `716283c7f4299122434ddf57c12131ba93b9e9bb568ba73fd90bd6049086df96`。
- NSIS：.artifacts/Steam.Hentai.Patch_0.2.1_x64-setup.exe，SHA-256 `a45b494a2f1b529127b6bfbbe531bbfde049d010bf8ed9e9bf21857c68c46cbe`。
- 详细打包证据：visual-polish-portable-verified.json、visual-polish-nsis.log、visual-polish-sbom.json。未发布、未推送，旧重写的官方ZIP站点403遗留不在本次视觉修改中伪称解决。

## 追加：关窗迟缓修复

用户报告关闭Windows窗口时长时间卡顿，已沿实际关闭路径核对并复现两个独立原因：

1. AppTasks.shutdown会等待所有一次性调用，但scan/project_instances/preview原先走无取消的serve；文件检测可能仍在读取大文件。三项读取现复用既有CoreTask/serve-task，关窗同时发送协作取消、等待子进程真实终态，并阻止新RPC；恢复、设置等写调用继续等待真实完成。返回应用后恢复接收调用。取消的扫描不发布部分结果或继续后续查询。
2. 原生release退出测试复现flutter_windows.dll异常0xc0000005/0xc000041d，8,105ms后异常退出；事件日志中旧候选也有同一异常。window_manager 0.5.2的destroy仅调用PostQuitMessage，跳过WM_CLOSE/WM_DESTROY正常路径。清理完成后现改为setPreventClose(false)再close，通过正常Windows消息顺序销毁窗口；未改系统错误报告设置或本机依赖缓存。

### 验证

- 真实可控子进程5项新增回归：普通scan DTO不变；三种读取在预检未完成时也能接收取消；已有recover写入仍被等待。与窗口生命周期/安装一起的13项针对性检查通过。
- 最终analyze无问题，全量179项通过；release构建成功，包含上一轮下载速度、主题和图标修改。证据close-fix-final-build.log。
- 真实核心读取完整旧状态副本时主动取消：等待1,436ms，scan返回cancelled，两核心子进程均exit0。仅取消只读扫描，未执行游戏安装。证据close-fix-real-core.json。
- 首次原生关窗测试发现异常退出后没有记为通过；崩溃基线保存在close-fix-crash-baseline.json，失败回执close-fix-release-window.json。
- 最终真实release窗口在serve-task扫描子进程运行期间收到正常关窗，880ms内进程退出，exit_code=0，没有强杀。证据close-fix-native-final.json；此次覆盖实际窗口销毁，区别于插件模拟或仅检查进程消失。
- 这些耗时为本机具体场景实测，不是生产超时阈值或所有任务的退出时长承诺。安装/恢复正在清理时仍须等待真实完成；没有用任意时长代替故障判断。
- 按overengineering-review复核：直接复用现有取消机制与正常窗口关闭路径，移除不可取消的读取路径；未新增强杀、双通信协议、重试、超时或通用任务框架。单独跟踪三种昂贵读取仅用于关窗发送取消，不接管已有写事务。

新包已按上述修复重建，以本节随后记录的哈希为准，前文视觉候选和首次关窗候选哈希保留为历史证据。本轮未提交、合并、推送或发布。

### 含关窗修复的最终候选

- 便携包：`.artifacts/steam-patch-manager-windows-portable.zip`，18,369,578字节，SHA-256 `523000ddc72f547f50c76299e6595707ee9c8f41e72aefe549601282cb568815`。
- NSIS：`.artifacts/Steam.Hentai.Patch_0.2.1_x64-setup.exe`，SHA-256 `d9c94e8ade8d07e021106c928027078615fb45803d5d65ec5ff3168a92faaff4`。
- 最终便携包39条目与构建目录逐项一致；原生退出测试进程92908对应Application Error记录0条。证据close-fix-final-portable-verified.json、close-fix-native-final-events.json。

## 2026-10-23 本地提交

- `c4e962f`：安装下载速度及必要测试。
- `ca62c44`：三项昂贵读取的协作取消、正常窗口关闭及必要测试。
- `aed058a`：程序与安装/卸载器共用品牌图标。
- 文档独立提交，既有 179 项和后续 Modern UI 的 205 项回归记录继续有效，没有新增代码修改或重跑无关验证；没有合并、推送或发布。
