# Steam 游戏补丁管理工具

为 Windows 与 SteamOS / Steam Deck 上已安装的 Steam 游戏查找补丁来源、检测补丁状态，并对支持的 ZIP 补丁提供安装、备份、卸载与事务恢复。

工具会扫描本机 Steam 游戏库，按游戏与平台匹配补丁目录。你可以按游戏名称或 AppID 搜索，查看官方来源、兼容性和文件变更，再决定是否安装。游戏库和安装记录保存在本机，默认不上传游戏库，不启用遥测。

当前版本为 Alpha，提供 Windows 桌面应用、Decky 插件和命令行工具。部分目录条目仅提供官方来源引导；自动安装只适用于具有审核配方、可信目录和匹配游戏版本的补丁。Steam Deck 插件仍在实机验收中。

## 使用方法

### 获取与启动

- **Windows**：从本仓库的 GitHub Releases 下载 Windows 便携 ZIP，完整解压后运行 `steam-patch-windows.exe`；也可以使用 NSIS 安装包安装。
- **Steam Deck**：需要先安装 Decky Loader，再通过其插件安装入口安装 Decky ZIP。插件中的 `bin/steam-patch` 需要具有可执行权限，并以游戏所属用户运行。
- **Actions 构建包**：在本仓库 GitHub Actions 的成功运行详情页，下载 `windows-portable`、`windows-nsis` 或 `decky-plugin` Artifact。

自行构建与 Decky 配置步骤见[客户端指南](docs/client-guide.md#构建)。

### 查找与安装补丁

1. 启动工具，等待游戏库扫描完成。游戏或库磁盘发生变化后，点击“重新扫描”。
2. 搜索游戏名称或 AppID，也可以使用“有补丁”“已安装”“需处理”筛选。
3. 选择游戏与补丁，查看检测状态、兼容性和提示。点击“在外部浏览器打开官方来源”查看补丁说明。
4. 对支持安装的条目，点击“从审核来源获取”；如果来源要求手动下载，在浏览器下载后点击“导入本地 ZIP”。Windows 使用文件选择器；Decky 的手动导入范围为用户 Downloads 或工具下载缓存。
5. 点击“生成安装计划”，核对要新增或替换的文件及阻止安装的提示。“查看变更预览”仅供查看，不能直接执行安装。
6. 关闭游戏并暂停 Steam 更新，再点击“确认安装这些变更”。等待操作完成，工具会刷新游戏状态与安装记录。

只有来源引导的条目需要按官方说明手动处理。目录没有匹配条目，不代表游戏没有补丁；“状态未知”也不代表补丁未安装。目录签名到期、游戏版本未知或补丁包未通过验证时，工具会禁止自动安装。

### 卸载与恢复

选择游戏，在“本工具事务”中点击“卸载并恢复”，关闭游戏并暂停 Steam 更新后确认。工具只恢复自身保存的备份；如果文件已被游戏更新或手动修改，或者备份损坏，会保留相关文件并报告冲突。其他方式安装的补丁无法通过本工具卸载。

安装被中断后，可点击“恢复事务”，关闭相关游戏并暂停 Steam 更新后确认，检查并恢复未完成的操作。

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
