# 按变更选择 CI

保留 `.github/workflows/ci.yml` 单一工作流。main push 和目标为 main 的 PR
先运行 `changes`，由 `select_jobs.py` 的 `RULES` 白名单选择 Core、Windows、Decky。
未匹配路径不触发模块构建；新增源码目录、公共输入或构建脚本时，须同步更新规则和测试。

| 输入 | 选择的 job |
| --- | --- |
| Rust crates、契约、Cargo 配置、信任密钥、工作流与选择器 | Core、Windows、Decky |
| Windows Flutter 与其构建、安装、SBOM 脚本 | Windows |
| Decky 前后端、安装入口与专用脚本 | Decky |
| 共享 UI、npm 配置、公共打包、SBOM、许可证、目录数据 | Windows、Decky |
| SF Python 参考实现、测试和夹具 | Core |
| 普通文档与项目技能 | 不运行模块构建 |

`docs/client-guide.md` 和 `docs/privacy.md` 会装入 Windows 发行包，修改它们运行
Windows job；其他 Markdown 文件不触发构建。

push 比较事件的 before 与 after，包含一次 push 中的所有提交；PR 使用 base 与 head
的合并基点差异，不把仅存在于目标分支的变更算入 PR。重命名按删除旧路径、增加新路径处理。
checkout 获取完整历史，Git 差异计算失败直接失败，不返回空变更。
首次 main push、`v*` 标签和手动运行选择全部模块。发布仍只由 `v*` 标签触发。

`core` 保留 Rust 和 SF 参考实现检查；Decky 后端和安装测试移到 `decky`；
Flutter SBOM 测试移到 `windows`；公共打包和 SBOM 测试在两个平台分别执行。

纯文档提交也会有轻量的 `changes` 和汇总 `ci`，但不会安装 Rust/Flutter/npm 依赖或构建客户端。
`ci` 在检测或所选模块失败、取消时失败，允许未选中的模块跳过。
若配置 PR 必需检查，建议使用 `ci`；分支保护需在 GitHub 上单独设置，本次不自动修改。

本地验证选择规则和真实 Git 差异行为：

```sh
python -m unittest discover -s .github/ci -p 'test_*.py' -v
```
