# Windows 全局来源布局修复施工记录

方案见 [设计文档](design.md)。

## 当前状态

已实现并通过针对性自动化验证；原生 Windows 界面待验收。开发分支：`fix/windows-global-sources-layout`（按用户要求使用修复分支）。代码与文档已拆分本地提交，并按用户要求合并本地 `main`；未推送、打包或部署。

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
