# Windows 补丁选择修复施工记录

设计：[design.md](design.md)。

## 状态

已实现，45 项相关自动化测试与 Flutter 静态分析通过；运行中安装版更新与真实来源实机验证待执行。代码与文档分开提交至 feature/windows-patch-selection；尚未合并或发布。

## 2026-10-26

- 用户确认同一候选由发现转为可安装时出现“原补丁已不在当前来源结果中”。代码确认发现 key 与安装分组 key 不一致。
- 范围限定为 Windows 选择身份、选择器及回归测试。
- DisplayEntry 新增统一 selectionKey，发现条目与可安装分组均使用 source_id + candidate_id；不改变展示 key、共享 TypeScript 契约或后端协议。游戏选择、概览入口、补丁下拉框及页面状态均接入统一选择身份。
- 新增 patch_selection_test.dart：非首项候选由发现转可安装、再转回发现时保持选择且不显示移除提示；核实安装面板使用所选候选下载项；真实候选移除时仍提示；不同来源同名候选身份隔离；无候选的 release 身份保持原样。

## 验证结果

- `flutter test test/patch_selection_test.dart test/library_page_test.dart test/library_presentation_test.dart`：13 项通过。
- `flutter test test/modern_desktop_test.dart test/modern_layout_test.dart test/detail_expansion_test.dart test/installation_flow_test.dart`：32 项通过，覆盖安装流程、选择器所在面板、技术详情及多尺寸/缩放/明暗布局。
- `flutter analyze`：No issues found。
- `git diff --check`：通过；现有 generated_plugin_registrant.cc/.h 与 generated_plugins.cmake 的用户修改未纳入本次修复。
- 本地默认权限运行 Flutter 首次遇到用户目录遥测日志访问拒绝，使用获准的原生执行后上述检查通过。

## 过度设计复核

按 overengineering-review 技能沿选择游戏 → 概览选择 → 来源结果刷新 → 选择器 → 安装面板路径检查。统一身份直接解决已确认的状态切换误判，不引入额外恢复分支、配置、限制、重试或超时；保留真实条目移除和下载项变化提示。未发现本次改动需进一步精简的问题。

## 待验证

- 未重新构建、替换用户正在运行的 0.3.0 安装版，未执行真实网站刷新或真实补丁安装；自动化通过不代表这些实机环节通过。
