# Steam 游戏补丁管理工具

目标：为 Windows 与 SteamOS / Steam Deck 上已安装的 Steam 游戏，提供补丁发现、匹配、安装状态检测，以及可恢复的安装与卸载体验。

当前为 Alpha 开发中：共享核心已支持来源脚本查询、动态匹配及本地安装计划、SQLite 备份事务、安装 / 卸载 / 恢复；Windows Flutter 与 Decky 客户端已实现并构建。Windows 与指定 Steam Deck 内置 SSD 已通过真实《夜勤病栋》的补丁安装 / 检测 / 卸载恢复；用户确认 Deck 桌面模式及 GE-Proton8-8 游戏模式补丁生效。其他样本、运行矩阵和发布门槛仍在验收中。

- [调研与开发计划](development-plan.md)
- [调研来源与证据边界](research-sources.md)
- [开发进度与验证记录](progress.md)
- [动态来源与静态目录移除](features/remove-static-catalog/design.md)
- [JSON 通信契约](../packages/contracts/README.md)
- [本地数据与隐私](privacy.md)
- [Alpha 发行检查](release-checklist.md)

建议路线：共享 Rust 安装核心 + Windows Flutter 桌面应用 + SteamOS Decky 插件；内置来源规则与解析脚本，由用户本地扫描、查询和匹配补丁。

安装 Rust 1.94 后，在仓库根目录运行（Windows 有仓库隔离工具链时可用 scripts/cargo.ps1 代替 cargo）：

```powershell
cargo build -p patchhub-cli --locked
cargo run -p patchhub-cli -- scan
cargo run -p patchhub-cli -- scan --steam-root 'D:/Steam'
cargo test -p patchhub-core -p patchhub-cli --locked
```

本次工作区已生成可直接运行的 Windows 调试 CLI：

```powershell
.\target\debug\patchhub-core.exe scan
```

扫描以用户本机 Steam 清单为准，补丁候选来自来源查询与本地缓存；不分发逐游戏静态补丁目录。历史实机结果见施工记录，其他样本仍需实际验收。

如果使用 PowerShell 包装脚本，Cargo 的参数分隔符需作为字符串传递：

```powershell
.\scripts\cargo.ps1 run -p patchhub-cli '--' scan
.\scripts\cargo.ps1 clippy --workspace --all-targets '--' '-D' warnings
```

完整构建、使用与来源管理步骤见 [客户端指南](client-guide.md)。安装仍检查实际来源选择、游戏条件和下载内容。只读预览与可执行安装计划是不同接口。

Windows 一键构建：安装客户端指南中的依赖后，双击根目录 `build.bat`，或在终端运行 `build.bat --no-pause`。构建结果位于 `.artifacts`，详细依赖和输出说明见 [客户端指南的构建章节](client-guide.md#构建)。

GitHub Actions 在推送、Pull Request 和手动触发时检查代码并分别构建 Windows 与 Decky 客户端；完成后在运行详情页的 Artifacts 下载构建包。

推送 `v*` Tag（例如 `v0.1.0-alpha.1`）后，全部检查与构建成功才自动发布 GitHub Release；含 `-` 的 Tag 发布为预发行版。操作步骤见 [Tag 发布](client-guide.md#tag-发布)。

客户端构建完成后运行 `python scripts/package-artifacts.py`，生成 Windows 便携 ZIP、Decky ZIP 与校验清单，保存在忽略目录 .artifacts。Windows 便携包解压后运行 patchhub.exe；NSIS 是管理工具的可选安装方式，游戏补丁本身仍按配方解压文件。

只打包一端时，使用 `python scripts/package-artifacts.py --platform windows` 或 `--platform decky`，对应生成 `checksums-windows.json` 或 `checksums-decky.json`。
