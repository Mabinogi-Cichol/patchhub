# Steam 游戏补丁管理工具

目标：为 Windows 与 SteamOS / Steam Deck 上已安装的 Steam 游戏，提供补丁发现、匹配、安装状态检测，以及可恢复的安装与卸载体验。

当前为 P0 首批实现：Rust 核心与只读 CLI 可运行，支持多库扫描、AppID 匹配、补丁文件指纹检测与安装预览；尚无 Windows GUI、可安装的 Decky 插件或真实补丁安装 / 回滚。

- [调研与开发计划](docs/development-plan.md)
- [调研来源与证据边界](docs/research-sources.md)
- [开发进度与验证记录](docs/progress.md)
- [补丁目录说明](catalog/README.md)
- [JSON 通信契约](packages/contracts/README.md)

建议路线：共享 Rust 安装核心 + Windows Tauri 桌面应用 + SteamOS Decky 插件；先建立少量经过审核的官方补丁目录，再扩大发行商覆盖。

安装 Rust 1.94 后，在仓库根目录运行（Windows 有仓库隔离工具链时可用 scripts/cargo.ps1 代替 cargo）：

```powershell
cargo build --workspace --locked
cargo run -p steam-patch-cli -- scan --catalog catalog/catalog.json
cargo run -p steam-patch-cli -- scan --steam-root 'D:/Steam'
cargo test --workspace --locked
```

本次工作区已生成可直接运行的 Windows 调试 CLI：

```powershell
.\target\debug\steam-patch.exe scan --catalog catalog/catalog.json
```

目录目前为空，因此所有安装记录显示 no_catalog_match；这表示没有已审核条目，不表示游戏不需要补丁。Steam 工具 / 运行时暂时保留 kind=unknown。

如果使用 PowerShell 包装脚本，Cargo 的参数分隔符需作为字符串传递：

```powershell
.\scripts\cargo.ps1 run -p steam-patch-cli '--' scan --catalog catalog/catalog.json
.\scripts\cargo.ps1 clippy --workspace --all-targets '--' '-D' warnings
```

CLI 的 serve 模式与 Decky 桥接示例见 apps/decky/README.md。Linux 尚需本机编译 / CI 和 Steam Deck 实机验证。所有扫描与检测均为只读；预览 executable 永远为 false。
