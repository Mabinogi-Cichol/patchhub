# PatchHub 客户端使用指南

PatchHub 扫描本机 Steam 游戏库，通过订阅源查找补丁。Windows 提供安装版和便携版，Steam Deck 通过 Decky Loader 使用 PatchHub。安装入口见[项目 README](https://github.com/Mabinogi-Cichol/patchhub#安装)。

## 查找补丁

1. 扫描 Steam 游戏库。找不到游戏时，切换到“全部”筛选并搜索名称。
2. 在 Windows 的“全局来源”或 Decky 的“管理订阅源”启用所需来源。可在来源卡片手动更新；自定义来源见[使用指南](https://github.com/Mabinogi-Cichol/patchhub/blob/main/docs/custom-subscription-sources.md)。
3. 打开游戏的补丁管理页。如果来源刚更新，或上次查询失败，使用“重新匹配”。查看候选的语言、本体或 DLC、格式和发布页，再选择下载项。若来源只提供网页或网盘引导，可打开来源并填写实际下载直链。

Shiravune 更新在后台加载站点，Windows 需要 Microsoft Edge WebView2 Runtime，Decky 使用 Steam 自带浏览器。遇到站点要求本人声明或真人验证时，任务会显示更新失败。

## 安装与重新安装

关闭游戏并暂停 Steam 对该游戏的更新。选定下载项后，点击“安装补丁”或“覆盖安装补丁”。PatchHub 将可读取的 ZIP 或 EXE 归档中的文件解压到选定 Steam 游戏目录，按包内路径覆盖同名普通文件。游戏可能还需要发行商说明中的其他操作。

安装前请自行备份重要文件与存档。直接解压没有自动备份、回滚或补丁卸载功能。失败、取消或中断后，已经写入的文件会留在游戏目录。“整理中断记录”只处理记录。需要恢复原文件时，请使用自己的备份；重新安装可以再次覆盖补丁包内的文件。

Windows 可在进度窗口请求取消。Decky 安装期间请留在进度界面，离开会请求取消。请求取消后请等待任务返回结果。“文件已解压到目标目录”表示文件写入完成，补丁是否生效请在游戏中确认。

## 更新 PatchHub

在应用设置中选择“检查更新”。Windows 下载新版后按提示重启并更新。Decky 按提示更新，替换完成后插件会重新加载；期间可能暂时断开。近期版本及手动安装包见 [GitHub Releases](https://github.com/Mabinogi-Cichol/patchhub/releases)。

使用日志诊断问题前，请参阅[隐私与本地数据](privacy.md)。
