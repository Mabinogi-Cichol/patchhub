# Steam 游戏补丁管理工具

为 Windows 与 SteamOS / Steam Deck 上已安装的 Steam 游戏查找补丁来源、检测补丁状态，并对支持的补丁提供安装、重新安装与失败恢复。

工具会扫描本机 Steam 游戏库，按游戏与平台匹配补丁目录。你可以按游戏名称或 AppID 搜索，查看官方来源、兼容性和文件变更，再决定是否安装。游戏库和安装记录保存在本机，默认不上传游戏库，不启用遥测。

当前版本为 Alpha，提供 Windows 桌面应用、Decky 插件和命令行工具。部分目录条目仅提供官方来源引导；自动安装适用于具备受支持安装链路的补丁，包括逐游戏审核配方，以及在安装时按安装器行为自动检查条件的通用 EXE 包。Steam Deck 插件仍在实机验收中。本轮后端协议已升级，Windows 客户端尚未适配，暂不保证可用。

Decky 按订阅的网站实时发现补丁：每个源独立定义请求、解析、游戏匹配和下载域名规则。已提供 patch.moe 专用适配器及可配置的 HTML 表格适配器，新游戏由官网查询产生，无需维护游戏列表。扫描时查找可自动安装补丁；点击安装后自动下载、校验并安装。保留包内相对路径，不运行包内安装程序；临时恢复材料在成功后清理。效果未逐游戏验证。详见 [源适配器与订阅](docs/source-adapters.md)。

功能设计与施工记录见 [功能开发文档](docs/features/README.md)，开发文档归类规则见 [AGENTS.md](AGENTS.md)。

## 使用方法

### 获取与启动

- **Windows**：从本仓库的 GitHub Releases 下载 Windows 便携 ZIP，完整解压后运行 `steam-patch-windows.exe`；也可以使用 NSIS 安装包安装。
- **Steam Deck**：需要先安装 Decky Loader，再通过其插件安装入口安装 Decky ZIP。插件中的 `bin/steam-patch` 需要具有可执行权限，并以游戏所属用户运行。
- **Actions 构建包**：在本仓库 GitHub Actions 的成功运行详情页，下载 `windows-portable`、`windows-nsis` 或 `decky-plugin` Artifact。

自行构建与 Decky 配置步骤见[客户端指南](docs/client-guide.md#构建)。

### 查找与安装补丁

1. 启动 Decky 插件，等待游戏库扫描及补丁来源查找完成。游戏或库磁盘发生变化后点击“重新扫描”。
2. 搜索游戏名称或 AppID，选择游戏查看状态。
3. “有可用补丁”可点击“安装补丁”，工具自动下载、校验并安装；“已安装补丁”可点击“重新安装补丁”。安装前关闭游戏并暂停 Steam 更新。
4. “找到补丁来源”表示找到了匹配来源，但工具无法自动安装；进入游戏详情查看来源并按官方说明处理。

安装期间可查看进度或取消；关闭页面后任务继续，重新打开可查看结果。安装失败时工具自动尝试恢复，未能完成恢复的事务会保留材料并提示处理。工具不提供补丁卸载，也不保留用于卸载的长期备份。

目录没有匹配条目不代表游戏没有补丁。“有可用补丁”表示扫描时已具备自动安装链路，实际下载和安装仍会重新校验来源、内容、安装条件及游戏状态（通用 EXE 补丁在安装时自动检查条件）。

### 命令行扫描

使用 `steam-patch` 命令行程序扫描本机游戏库，输出 JSON：

```powershell
steam-patch scan --catalog catalog/catalog.json
```

其中 `catalog/catalog.json` 为补丁目录文件路径。指定 Steam 安装目录或平台：

```powershell
steam-patch scan --steam-root 'D:/Steam' --catalog catalog/catalog.json --platform windows
```

SteamOS 使用 `--platform steamos`；多个 Steam 根目录可重复传入 `--steam-root`。运行 `steam-patch --help` 查看命令与参数。
