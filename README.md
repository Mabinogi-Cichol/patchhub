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

## 实机截图

Steam Deck 游戏模式：

| 游戏列表 | 补丁选择 |
| --- | --- |
| ![游戏列表](docs/images/decky/01-game-list.png) | ![补丁选择](docs/images/decky/02-patch-selection.png) |

| 安装确认 | 下载进度 |
| --- | --- |
| ![安装确认](docs/images/decky/03-install-confirmation.png) | ![下载进度](docs/images/decky/04-download-progress.png) |
