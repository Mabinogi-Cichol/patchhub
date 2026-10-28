# PatchHub

![PatchHub：Steam 游戏补丁助手](docs/images/branding/banner.png)

为 Steam 游戏查找、下载并安装成人内容补丁，支持 **Windows** 和 **Steam Deck**。自动扫描本机游戏库，汇总白舟、神乐等补丁来源；支持的补丁可直接安装，其余可打开来源网页手动处理。

## 安装

从 [GitHub Releases](https://github.com/Mabinogi-Cichol/patchhub/releases/latest) 下载适合设备的安装包。

### Windows

- **安装版**：下载 `patchhub-windows-x64-setup.exe`，运行并按提示安装。
- **便携版**：下载 `patchhub-windows-portable.zip`，完整解压后运行 `patchhub.exe`。

### Steam Deck

先安装并启用 [Decky Loader](https://github.com/SteamDeckHomebrew/decky-loader)。

#### 命令行在线安装

在桌面模式打开 **Konsole**，保持 Steam 和 Decky 运行，执行：

```bash
curl -fsSL https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/install.sh | bash
```

输入 `y` 确认安装，更新也使用同一条命令。适用版本及要求见[安装器说明](apps/decky/README.md#终端安装器开发版)。

#### ZIP 离线安装

1. 从 Releases 下载 `patchhub-decky.zip`，传到 Steam Deck。
2. 打开 Decky 设置，通过本地 ZIP 安装入口选择该文件；部分版本需先启用开发者模式。
3. 返回游戏模式，按 **“…” → Decky → PatchHub** 打开插件。

## 本地构建

- Windows 快速预览：双击 `build-windows.bat`，增量构建 Rust 核心与 Flutter 应用后自动打开，跳过测试、静态分析和发行打包。应用位于 `apps/windows-flutter/build/windows/x64/runner/Release/patchhub.exe`。需要安装包、便携 ZIP、SBOM 和校验文件时使用原有 `build.bat`。
- Decky：双击 `build-decky.bat`，通过默认 WSL 发行版构建 Linux 核心，再生成插件 ZIP、SBOM、校验文件及安装脚本。需 Windows Node.js/npm、Python 3.11+，以及已配置 Linux Rust 工具链和原生编译依赖的 WSL；工具链目录沿用 `scripts/build-linux.sh` 的 `.tools/linux-cargo`、`.tools/linux-rustup`。

发行产物位于 `.artifacts`。命令行运行可加 `--no-pause` 禁用结束暂停，加 `--help --no-pause` 查看说明。发行构建请依次运行，共用的 SBOM 和总校验表由最近一次发行构建生成。

应用升版只修改根 `Cargo.toml` 的 `workspace.package.version`，构建入口自动同步其他清单；不构建时运行 `python scripts/project_version.py --sync`。生成的清单与锁文件需一并提交，详情见[版本管理](docs/client-guide.md#应用版本)。

## 实机截图

Steam Deck 游戏模式：

| 游戏列表 | 补丁选择 |
| --- | --- |
| ![游戏列表](docs/images/decky/01-game-list.png) | ![补丁选择](docs/images/decky/02-patch-selection.png) |

| 安装确认 | 下载进度 |
| --- | --- |
| ![安装确认](docs/images/decky/03-install-confirmation.png) | ![下载进度](docs/images/decky/04-download-progress.png) |
