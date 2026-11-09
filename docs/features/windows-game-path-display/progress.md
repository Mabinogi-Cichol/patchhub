# Windows 游戏路径显示修复施工记录

设计见 [design.md](design.md)。

## 状态

已实现，定向自动化与静态分析通过；原生应用验收待执行。

## 2026-11-10

- 在 `fix/windows-game-path-display` 分支实现展示层格式化，覆盖“更多游戏信息”、无补丁游戏信息与多库库位置。
- 选择复制使用普通路径；原始模型及文件操作路径不变。
- 更新现有长路径布局/真实选择复制回归，补充盘符、UNC 与不转换路径用例。
- 按 overengineering-review 技能复核方案及调用路径：转换只发生在文本构造处；打开目录仍传原始路径，无额外配置、状态或防御流程，未发现需修改项。
- `dart format`：已格式化五个修改的 Dart 文件。
- `flutter test --no-pub test/game_path_display_test.dart test/game_path_layout_test.dart test/detail_expansion_test.dart test/patch_management_flow_test.dart`：15 tests 全部通过，覆盖盘符/UNC 转换、其他路径保留、四种窗口尺寸与两档缩放、真实选择复制及原模型不变。首次默认依赖解析因 pub.dev 网络不可用失败，改用现有依赖；沙箱内测试进程回环连接失败，获准在沙箱外重跑上述命令后通过。
- `flutter analyze --no-pub`：No issues found。
- `git diff --check`：通过。
- 未构建、部署或执行原生应用验收；未合并或推送。用户要求本地 Git 提交，代码与文档分开提交。
