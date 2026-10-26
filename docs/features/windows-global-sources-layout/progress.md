# Windows 全局来源布局修复施工记录

方案见 [设计文档](design.md)。

## 当前状态

首轮状态键修复已拆分本地提交并合并本地 main。本轮排版与主题图标补充位于 `fix/windows-source-theme-layout`，已实现、自动化及深浅主题渲染检查通过，代码与文档分别本地提交，未合并；本地图标空白修复后的 Release 已重建并核验字体，原生 Windows 界面待验收，未替换安装版或发布发行包。

## 2026-10-27 排版与主题图标补充（本轮）

- 分支：`fix/windows-source-theme-layout`，从本地 main 创建；保留工作区原有构建入口改动。
- 已将启用开关放入卡片标题、收窄间隔输入框，并按自动更新、索引状态/手动更新、调度详情分组；按钮和详情统一左对齐，窄窗口自然换行。
- 三种主题使用显示器、太阳、月亮图标，标题栏随所选模式更新；菜单保留勾选状态。
- 按后续反馈，将跟随系统的显示器替换为左半太阳与右半月亮组合，标题栏与菜单共用同一绘制函数；浅色与深色图标保持。
- 组合图标补充验证：`flutter test test/library_page_test.dart` 两项通过；相关页面和测试静态分析通过。早先预览截图仍为显示器图标，未重新生成。
- `flutter test test/global_sources_layout_test.dart test/source_model_test.dart test/library_page_test.dart test/scaled_layout_test.dart`：13 项通过。来源页按 580×560 可用内容尺寸验证深浅主题及 1.0/1.5 文字比例，覆盖输入、滚动、详情展开及反复折叠；主题回归核对三态图标、系统明暗变化与游戏筛选保留，扫描次数仍为 1。
- 首轮新增断言错误地假设滚出缓存区域的四张卡片均已构建，导致两项失败；改为检查当前已构建卡片，未增加产品逻辑。
- `flutter analyze`：No issues found。`git diff --check` 通过。
- 使用临时 Flutter 渲染测试加载 Windows 正文字体与 MaterialIcons，生成并检查 1400×900 深浅主题截图：`.artifacts/source-layout-dark.png`、`.artifacts/source-layout-light.png`。截图为夹具数据，无真实游戏库；临时测试已移除，图片保留用于审阅。
- 过度设计复核：沿来源展开、开关保存、间隔保存、任务更新和主题选择路径检查；复用既有接口、状态键及主题状态，仅引入界面分组与三态图标映射，无新增业务硬限制、防御分支、依赖或配置，未发现需修改项。
- 原生 Windows 窗口验收、打包和安装版替换待执行。

## 2026-10-27 图标空白反馈

- 用户反馈半太阳半月亮图标空白。新增 20×20 尺寸检查与加载字体后的左右半区像素检查，原实现在 widget 环境均通过，未复现布局折叠为零。
- 检查本地 Release 的 MaterialIcons 字体，两种主题字形均不存在；第一次普通 Release 重建成功但字体仍为 4084 字节、旧时间戳，两个字形仍缺失。
- 核对当前 SDK 构建代码后，定位资源增量输入遗漏 `app.dill`；删除单个资源缓存标记重建，并在项目 Windows 构建入口同步此处理，保留原有未提交构建入口改动。
- 组合图标改为固定 20×20 尺寸，三种模式均直接使用常量 Icon；`flutter test test/library_page_test.dart` 两项通过，新增检查确认系统图标左右两半均实际绘制。
- 构建脚本 PowerShell 语法解析通过；删除资源缓存标记后 `flutter build windows --release` 通过。最终字体由 4084 增至 4460 字节，两个 cmap 表均确认 U+EF9F→glyph 11、U+F162→glyph 23，均为非零。
- 临时改用最终 Release 字体再执行 `library_page_test.dart`，两项及左右半区像素检查通过，并检查 `.artifacts/theme-header-fixed.png` 中半太阳半月亮实际可见。测试随后恢复使用标准测试资源，无成品路径依赖或截图写入残留。
- 相关 Dart 文件静态分析通过，`git diff --check` 通过。过度设计复核：缓存处理只针对已确认复用旧字体的 Windows Release 资源标记，保留字体裁剪和其他构建缓存，无新增依赖、重试或配置。
- 未执行原生 Windows 窗口启动验收，未替换安装版；本地新构建位于 `apps/windows-flutter/build/windows/x64/runner/Release/patchhub.exe`。

## 2026-10-27 相同明暗下的主题模式切换

- 新增系统深色/浅色两项回归，修改前均失败：应用 themeMode 已改为手动模式，但入口仍保留 system-theme-icon。原有两项回归通过，说明此前未覆盖相同实际明暗的模式切换。
- 菜单选择更新应用主题时同步标记当前页面重建，图标重新读取已有 themeMode；未增加模式副本、监听框架或页面重置。
- `flutter test test/library_page_test.dart` 四项通过，包括系统深色/浅色下各四次同明暗模式切换；入口图标同步变化，实际明暗保持，游戏筛选保留且扫描次数仍为 1。
- 相关文件 `flutter analyze` 无问题，`git diff --check` 通过。按过度设计技能复核，修改仅在已有用户选择入口通知当前页面重建，未改变主题状态来源或引入新抽象。
- `scripts/build-windows-flutter.ps1 -Preview -SkipRust` 构建通过，已重新生成图标资源并更新本地预览 `apps/windows-flutter/build/windows/x64/runner/Release/patchhub.exe`；原生窗口验收及安装版替换仍未执行。

## 2026-10-27

- 新增四来源滚动/展开回归，旧代码失败：`type 'bool' is not a subtype of type 'double?' in type cast`，定位到高级规则展开状态与页面滚动位置共用 PageStorage。
- 原有 `source_model_test.dart` 三项通过，说明原测试未覆盖滚动后展开路径。
- 为高级规则、来源详情及其输入框/可选择详情文本设置独立状态键；将自动更新间隔标题移至输入框上方并增加间距。
- 第一轮修改后回归仍失败，进一步获取堆栈定位到 `TextField` 内部 `ScrollPosition.restoreScrollOffset`：仅隔离展开区仍不足，输入框内部滚动也必须拥有独立状态键。修订后来源/缩放相关 8 项通过。
- 最终 `flutter test test/global_sources_layout_test.dart test/source_model_test.dart test/scaled_layout_test.dart`：11 项通过。新增四项覆盖深浅主题、1.0/1.5 文字比例，850×560 下滚动、高级规则展开、发行商卡片及详情展开、间隔输入、反复折叠展开和标题与输入框间距。扩展测试期间发现定位在动态高度变化后落到屏幕外，改为明确回到列表顶部并等待布局稳定；未因此添加产品逻辑。
- `flutter analyze`：No issues found。`git diff --check` 通过。
- 按 overengineering-review 技能沿滚动恢复、展开状态保存和输入框按间隔值重建路径复核：仅拆开实际冲突的状态身份并调整标题布局；无新增硬限制、重试、异常兜底、配置或抽象，未发现需修改项。
- 未执行原生 Windows 窗口截图/现场验收，未替换用户已安装版本；不得将本轮自动化结果表述为原生实机通过。
- 按用户要求合并至本地 `main`：仅功能索引新增行冲突，保留补丁选择修复与本次全局来源修复两项；代码无冲突且无新增实现改动，沿用上述验证结果。
