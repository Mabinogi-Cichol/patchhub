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

## 2026-11-10：推送后 CI #117 修复

- 失败提交：`1fb4687463123ae37f4525a57b70e5256866d181`；[CI #117](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/42667124190)，Windows job 的 `./scripts/build-windows-flutter.ps1 -SkipRust` 在 `flutter test --no-pub` 失败，310 passed / 5 failed / 2 skipped。core 与 decky 未选中；汇总 job 因 Windows 失败而失败。
- 5 项失败为 `scaled_layout_test.dart` 四个缩放/主题用例及 `settings_page_test.dart` 的清理导航用例，均使用 `library_page_test.dart` 的 FixtureClient。该夹具省略协议必填的 `install_path` / `library_path`，新展示调用的 `as String` 在无补丁详情构造时抛出 Null 类型转换错误；旧字符串插值仅将缺失字段显示为 `null`，掩盖了夹具不完整。此前 15 项定向测试未覆盖该夹具路径。
- 在独立分支 `fix/windows-game-path-display-ci` 补齐夹具的路径、availability 与 kind 必填字段，并在既有缩放回归中核对无补丁详情展示的普通路径。生产逻辑不变。
- 按 overengineering-review 复核：修正测试输入以符合既有契约，不新增生产兜底或协议兼容分支，未发现需修改项。
- 重跑 CI 失败步骤对应的 `flutter test --no-pub`：316 passed / 1 skipped，退出码 0；日志 `.artifacts/windows-game-path-ci-test.log`。先前失败的 5 项均通过；本机可执行项与 CI 的条件跳过项数量不同。
- `flutter analyze --no-pub`：No issues found；两个修改测试文件的 `dart format` 和 `git diff --check` 通过。
- 未重复运行 Rust/Decky、打包构建或完整发布矩阵。按用户要求将测试修复与文档分开提交，并合并到本地 main；未推送，修复后 SHA 的线上 CI 待验证；原生应用验收状态不变。
