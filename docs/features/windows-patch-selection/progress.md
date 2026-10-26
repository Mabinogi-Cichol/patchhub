# Windows 补丁选择修复施工记录

设计：[design.md](design.md)。

## 状态

选择身份修复已实现并分开提交代码与文档至 feature/windows-patch-selection；此前 45 项相关自动化测试与 Flutter 静态分析通过。本轮标题和候选安装交互补充已实现，验证见下方；本轮按标题修复、交互修复和文档拆分本地提交。尚未合并、发布或更新运行中的安装版，真实来源实机验证待执行。

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

## 2026-10-26 补充工作：候选项统一安装交互

- 用户明确要求移除标题拼接后缀，并取消非推荐候选追加勾选确认的交互。复用当前功能分支和文档。
- 共享 Rust 投影统一使用来源 patch_title，删除“手动选择”和 URL 文件名拼接。下载文件名保留在双端原有下载项/镜像区域，沿用现有 URL 解码显示。
- Windows 删除 confirmed 状态、确认复选框、安装门禁和“待确认适用条件”状态；Decky 删除 manualConfirmation 状态、ToggleField、安装入口和按钮门禁，统一安装按钮文案。共享摘要与 Windows 摘要均按真实 actions 显示“有可用补丁”，底层精确选择的兼容状态显示“按所选下载项安装”。
- 保留现有默认排序、镜像去重、来源/候选/下载地址/游戏实例绑定，以及真实 blocker、本地文件要求、并行任务门禁。没有将未知来源信息改写成已确认元数据。
- 新增真实 Flutter 控件回归：打开补丁下拉框，切换到其他候选，无需勾选即可安装，并断言请求使用所选 selection_id、进入安装完成结果；更新面板可用性断言。后端回归断言所有候选标题严格等于 patch_title。

### 本轮验证

- `flutter test test/installation_flow_test.dart test/modern_desktop_test.dart test/patch_selection_test.dart test/library_presentation_test.dart test/detail_expansion_test.dart`：26 项通过。
- `flutter analyze`：No issues found。
- `scripts/cargo.ps1 test -p patchhub-core --test projection --offline`：24 项通过，含精确下载/实例绑定、来源禁用、真实阻断与标题断言。默认权限首轮单项遇到 source_state_busy，获准原生执行后完整投影测试通过。
- Decky `npm test`：68 项通过；`npm run typecheck`、`npm run build`：通过。
- `node scripts/test-flutter-presentation.mjs --check`、`git diff --check`：通过。
- Decky 移除确认开关的初次修改曾被自动审批误解为违背用户意图并拒绝；核对用户明确取消额外勾选的原文，以及后端绑定测试证据后重新提交同项修改，审批通过。
- 多 Agent：1 个 worker（gpt-6-luna / medium）完成共享展示和摘要回归，主 Agent 完成双端交互与后端集成。

### 本轮过度设计复核与限制

- 按 overengineering-review 沿投影 → 候选/镜像选择 → 按钮可用性 → 最新选择核对 → 安装请求检查。删除仅服务于候选专用确认的状态、提示和分支；真实安装约束保持在既有边界。未引入替代确认、额外配置或重试。
- 未重新构建/替换 Windows 安装版，未部署 Steam Deck、未进行真实网站刷新或游戏安装；Decky 测试/构建不代表实体手柄及真实设备通过。

## 2026-10-27 提交记录

- 按用户要求将标题修复、双端候选统一交互、文档分别提交。沿用上轮已通过的验证，未重复运行测试；提交前检查差异及暂存区。
- Flutter 三个生成文件的原有工作区状态不纳入本轮提交；不推送、不合并或发布。
