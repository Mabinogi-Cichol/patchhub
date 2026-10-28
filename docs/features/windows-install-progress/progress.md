# Windows 安装进度界面施工记录

设计：[design.md](design.md)。

## 状态

已实现，相关回归、Windows 原生夹具渲染与 Release 构建通过；用户视觉验收与真实游戏安装待完成。分支 `feature/windows-install-progress`，从本轮干净的 `main` 创建。代码已本地提交，文档单独提交；未合并、推送或发布。

## 2026-10-29

- 已核对原 InstallDialog、CoreTask 取消与进度路径、详情页 SteamArtwork 接口及安装流程回归。
- 已按用户图例完成设计与计划过度设计审阅：复用现有数据与任务生命周期，仅修改呈现，不新增防御策略。

### 实际修改

- 安装弹窗改为 640 逻辑像素宽、16 像素圆角与细边框；顶部标题 / 关闭图标，左侧封面，右侧游戏与补丁信息、阶段与进度，底部独立描边取消按钮。
- 详情页向弹窗传递已有 SteamArtwork Future，复用 GameArtwork 的本地图片显示与缺图提示。最终原生截图已显示本机 `1011940` 的简体中文封面；没有下载或生成替代图片。
- 下载阶段显示百分比、容量、速度和“下载完成后将继续校验与安装”；隐藏文件数。未知字节总量不借用文件总数计算下载百分比；写入等阶段仍显示文件数与当前路径。
- 顶部关闭按钮与 Esc 一样请求现有协作取消；请求后按钮禁用，等待实际回执。保留成功 / 失败 / 取消 / 恢复结果、错误与清理详情、刷新错误和操作记录入口。
- 正文放入可滚动区域，底部操作区独立；长文本换行，大字体不溢出。取消按钮显式使用标准密度，避免 Windows 紧凑密度缩小 44 像素点击区。

### 验证

- `flutter test --no-pub test/installation_flow_test.dart test/app_lifecycle_test.dart`：**13 项通过**。涵盖完整安装、刷新失败、Esc、关闭应用、下载速度，以及本轮增加的未知总量、文件阶段切换与右上角取消后关闭结果。
- `flutter analyze --no-pub`：无问题。
- `flutter test --no-pub integration_test/install_progress_visual_test.dart -d windows --dart-define=PATCHHUB_VISUAL_OUTPUT=...`：Windows 原生夹具矩阵通过，最终 **12 张截图**。深浅主题分别覆盖普通窗口下载 / 未知总量 / 写入 / 取消结果，以及 850 × 560 窗口、150% 字体、长名称下的下载与取消结果；无 Flutter 布局异常。最终一轮包含真实本地封面与标准密度按钮。
- 已查看深 / 浅主题正常下载及大字体截图，核对层级、对齐、进度信息与底部操作区。证据位于 `.artifacts/windows-install-progress/`，矩阵回执为 `result.json`。这些是受控 CLI 夹具进度，不是真实游戏安装数据。
- `flutter build windows --release --no-pub`：通过，日志 `build.log`。候选为 `apps/windows-flutter/build/windows/x64/runner/Release/patchhub.exe`；本轮只改 Dart UI，没有重编译 Rust 或生成发行安装包。
- 真实游戏安装、生产下载链和用户视觉验收未执行。

### 实现过度设计审阅

已沿详情页安装 → 封面参数 → 弹窗进度呈现 → 取消 → 真实终态 → 刷新 / 结果关闭路径复核。未发现需修改项：复用现有任务和图片契约，没有新增协议、超时、重试、状态机或业务硬限制。布局宽度与紧凑断点只控制显示，不裁剪业务数据；既有错误与恢复细节保留。

### 本地提交

- 用户要求“提交 GIT”，代码和必要测试提交为 `c3e3648`（`feat(windows): 按图例重设计安装进度界面`）。
- 设计、施工记录与功能索引独立提交。沿用本节已有的 13 项回归、12 张原生截图、静态分析及 Release 构建验证；本次仅补充提交记录与检查差异，未重复运行产品测试。
- 保留 `feature/windows-install-progress` 分支，未合并 main、推送或发布。
