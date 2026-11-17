# Windows 设置入口“订阅更新中”状态设计方案

状态：**已实现（定向自动化验证通过；原生窗口视觉与动画效果待验收）**。所属功能：[design.md](design.md) · 施工记录：[progress.md](progress.md)。

## 1. 背景与目标

Windows 客户端在启动时、每分钟定时调度或用户手动操作时，会在后台执行订阅源更新（`update_source`）。目前右上角设置入口仅在空闲与设置页打开状态间切换（`设置` / `返回游戏`），用户在游戏库界面无法直观感知后台正在更新订阅源。

本方案在右上角设置入口新增“订阅更新中”视觉状态：当存在运行中的 `update_source` 任务且当前未处于设置页时，展示圆形进度条并将文案切换为“订阅更新中”，同时兼顾浅/深色主题美观与轻量流畅的切换动画。

## 2. 状态判定与交互契约

### 2.1 单一状态源

直接复用 `AppModel` 现有的 `AppTasks.sources` 映射（由 `CoreTask` 事件驱动并自动 `notifyListeners()`），不新增额外轮询、定时器、缓存标志或重复状态字段：

```dart
bool get sourceUpdating => tasks.sources.values.any(
  (task) => task.method == 'update_source' && task.running,
);
```

无论触发来源是启动检查、定时周期更新还是设置页手动刷新，只要任意 `update_source` 任务处于 `running` 状态，`sourceUpdating` 即为 `true`；任务结束（完成、失败或取消）后自动恢复为 `false`。

### 2.2 三态优先级

| 条件 | 左侧图标 / 指示器 | 按钮文案 | Tooltip 提示 | 点击行为 |
| --- | --- | --- | --- | --- |
| `settingsOpen == true` | `Icons.arrow_back` | `返回游戏` | `返回游戏` | 关闭设置页并返回游戏列表 |
| `settingsOpen == false && sourceUpdating == true` | 圆形进度条（15×15） | `订阅更新中` | `订阅源正在后台更新，点击打开设置` | 打开设置页（默认可查看来源更新状态） |
| `settingsOpen == false && sourceUpdating == false` | `Icons.settings_outlined` | `设置` | `设置` | 打开设置页 |

- **交互保持可用**：更新态下不禁用设置按钮，用户可随时点击进入设置页查看具体更新中的订阅源或进行其他设置。
- **进入设置页优先级**：当用户在更新态点击按钮进入设置页后，按钮切换为 `返回游戏`，避免设置页内按钮文案引起歧义；返回游戏列表时若后台仍在更新，自动恢复展示 `订阅更新中`。

## 3. 视觉美观规范

遵循 [windows-visual-polish](../windows-visual-polish/design.md) 与 [app.dart](../../../apps/windows-flutter/lib/app.dart) 的黑白灰中性表面 + 蓝色强调体系，确保在 52px 高度的 `DesktopToolbar` 中与右侧主题切换及窗口控制按钮协调统一：

- **容器形态（轻量胶囊芯片）**：
  - 常态 / `返回游戏`：保持简洁文本按钮外观（透明底色、无描边）。
  - `订阅更新中`：过渡为圆角胶囊容器（`BorderRadius.circular(18)`，内边距 `EdgeInsets.symmetric(horizontal: 12, vertical: 6)`），提供清晰而不刺眼的后台任务提示。
- **深浅色主题配色**：
  - **浅色模式**：`scheme.primaryContainer`（`#E3EDFF`）柔蓝底色 + `scheme.onPrimaryContainer`（`#18489F`）文字 + `scheme.primary` 细描边（约 `0.28` 透明度）。
  - **深色模式**：`scheme.surfaceContainer`（`#242424`）中性炭灰底色（避免深色背景泛蓝相）+ `scheme.secondary`（`#78A9FF`）高亮文字与进度环 + `scheme.secondary` 细描边（约 `0.32` 透明度）。
- **圆形进度条规格**：
  - 尺寸：`15 × 15`（与原 `20px` 图标视觉重心对齐），`strokeWidth: 2.2`，`strokeCap: StrokeCap.round`。
  - 颜色：浅色模式使用 `scheme.primary`，深色模式使用 `scheme.secondary`，搭配微弱轨道底色 `trackColor: accent.withValues(alpha: 0.18)`，增强旋转弧线的质感与辨识度。

## 4. 动画效果设计

全部使用 Flutter 内置隐式动画组件与内置 `CircularProgressIndicator`，不引入第三方动画库、自定义 `TickerProvider` 或额外定时逻辑：

1. **容器背景、描边与宽度平滑展开**：
   - 使用 `AnimatedContainer`（`duration: Duration(milliseconds: 220)`，`curve: Curves.easeOutCubic`）平滑过渡背景色、边框色与圆角。
   - 外层配合 `AnimatedSize`（`duration: Duration(milliseconds: 220)`，`curve: Curves.easeOutCubic`，`alignment: Alignment.centerRight`），使按钮在“设置”（2 字）与“订阅更新中”（5 字）之间切换时以右侧为锚点顺滑展开/收起，不发生突兀跳变，且左侧标题栏已通过 `Expanded` + `Flexible` 约束，不会挤出布局溢出。
2. **图标 ↔ 圆形进度条切换动画**：
   - 使用 `AnimatedSwitcher`（`duration: Duration(milliseconds: 200)`，`switchInCurve: Curves.easeOutCubic`，`switchOutCurve: Curves.easeInCubic`），`transitionBuilder` 组合 `FadeTransition` 与 `ScaleTransition`（`0.82 -> 1.0`），为三种图标状态绑定独立 `ValueKey`（`back` / `updating` / `settings`）。
3. **文案切换动画**：
   - 使用 `AnimatedSwitcher`（`duration: Duration(milliseconds: 200)`）配合 `ValueKey` 区分文案，与外层 `AnimatedSize` 同步完成淡入淡出和宽度伸缩。

## 5. 实施落点与验证计划

### 5.1 拟修改文件

- [app_model.dart](../../../apps/windows-flutter/lib/models/app_model.dart)：在 `AppModel` 新增只读 getter `sourceUpdating`。
- [library_page.dart](../../../apps/windows-flutter/lib/pages/library_page.dart)：将右上角设置入口抽离为紧凑的状态组件（如 `_SettingsEntryButton`），实现三态切换、胶囊样式与隐式动画。
- `apps/windows-flutter/test/library_page_test.dart`：补充 Widget 回归测试。

### 5.2 验证计划

- **Widget 定向测试**：
  - 空闲态显示 `Icons.settings_outlined` 与 `设置`；
  - 存在运行中的 `update_source` 任务时显示 `CircularProgressIndicator` 与 `订阅更新中`；
  - 更新态点击按钮可正常打开设置页，此时入口切换为 `Icons.arrow_back` 与 `返回游戏`；
  - 任务结束后（且未打开设置页）自动恢复为 `Icons.settings_outlined` 与 `设置`。
- **静态检查与缩放/主题检查**：
  - 执行 `flutter analyze --no-pub` 与 `dart format --output=none --set-exit-if-changed`；
  - 检查浅色/深色主题及 `850×560` 最小窗口、125%–150% 文本缩放下的布局对齐与无溢出表现。
