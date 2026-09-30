# 调研来源与证据边界

调研日期：2026-09-30。以下区分公开来源支持的事实、工程建议与未验证事项；没有下载或执行真实补丁，也没有扫描用户实际 Steam 游戏库。

| 来源 | 本次可核实内容 | 使用边界 |
| --- | --- | --- |
| [HGamer 首页](https://hgamer.tw/zh-TW) | 搜索索引展示游戏补丁、版本与最近更新栏目 | 可作为线索入口，不证明目录完整、下载授权或安全 |
| [HGamer publishers](https://hgamer.tw/zh-TW/publishers) | 用户提供的发行商门户地址；直接读取返回 Cache miss | 本次尚未读取完整清单；API 与抓取规则待核实 |
| [Decky Loader](https://github.com/SteamDeckHomebrew/decky-loader) | Steam Deck 插件加载器的官方项目 | 宿主版本、权限、分发与上架要求需按目标版本再验证 |
| [Decky 插件模板](https://github.com/SteamDeckHomebrew/decky-plugin-template) | Decky 官方组织提供插件开发入口 | PoC 以当前模板固定 commit，不能依赖未承诺的 Steam 私有接口 |
| [Valve Proton](https://github.com/ValveSoftware/Proton) | Proton 基于 Wine，让 Windows 游戏运行于 Linux | 不代表补丁 EXE 或具体游戏已兼容 |
| [Tauri capabilities](https://v2.tauri.app/security/capabilities/) | 前端可用权限可按 window / webview 与平台配置 | 自定义核心命令仍需要参数与路径检查 |
| [Tauri sidecar](https://v2.tauri.app/develop/sidecar/) | 支持打包外部二进制，要求对应目标架构的二进制命名 | 本项目可用直接 Rust commands；此来源只证明备选分发机制 |
| [HGamer 补丁目录](https://hgamer.tw/zh-TW/patches) | 本次读取快照显示 653 条、类型与语言过滤 | 数量是页面快照，不保证实时或 Steam 专属覆盖 |
| [Proton FAQ](https://github.com/ValveSoftware/Proton/wiki/Proton-FAQ) | 说明每游戏 prefix 位于库内 steamapps/compatdata/[appid]/pfx | 文档较旧；当前实际路径仍需验证，不硬编码为唯一位置 |
| [Steam InstallScripts](https://partner.steamgames.com/doc/sdk/installscripts) | 游戏安装路径因用户设置而异 | 没有为 libraryfolders / appmanifest 格式提供稳定公共契约 |
| [Decky 插件数据库](https://github.com/SteamDeckHomebrew/decky-plugin-database) | 官方组织的插件商店提交入口 | 不证明本项目符合上架要求 |

## 具体补丁来源案例

| 候选来源 / 游戏 | 已看到的证据 | 建议接入与缺口 |
| --- | --- | --- |
| [Dharker / Negligee](https://dharkerstudio.com/negligee.html) | 官方页面说明 138MB，解压后把 adultpatch.rpa 加入游戏 game 目录，保留原文件；菜单可显示内容版本 | 新增文件配方候选；页面未见明确补丁版号，仍需哈希、AppID、包获取与双端验证 |
| [Paper / Dischan / Mahou Arms](https://paper.moe/downloads) | 官方页面说明 patcher 自动识别版本；有 Steam Deck 用 Linux patcher 与 Windows 固定版本 bundle | 首版仅引导官方程序；未执行程序，不能推导具体文件改动或托管回滚 |
| [072 Project 门户样本](https://hgamer.tw/zh-TW/patches/colosseum-clothes-ripper-the-erotic-arena-rpg-steam-patch) | HGamer 标为 v1.0.0、8MB ZIP，说明免费登录 / 结账后下载，并称修复事件与 BUG | 账号流程只提供引导 / 导入；这不是已证实的内容恢复补丁，需另选符合目标的样本；外站未独立核实 |
| [azucat 门户样本](https://hgamer.tw/zh-TW/patches/the-too-cute-girl-who-moved-in-next-door-patch) | HGamer 标为 Ver.1.043、175MB ZIP，说明 R18 而非去码，解压 app_Data 后覆盖 | 覆盖文件配方候选；开发者下载页读取失败，包内容 / 哈希 / 兼容性待核实 |

另有 Saikey Studios 样本由来源 Agent 查到，但本轮主 Agent 复查详情页不可访问，因此不将它列为已复核的自动下载候选。以上候选均未下载文件或检查实际包内容。

## 来源接入核查模板

每款候选补丁记录：发行商、官方发布页、Steam AppID 证据、正式版 / Demo、游戏 build、语言、补丁类型、发布日期、包格式、大小、下载门槛、安装说明、包哈希、检测文件、Windows / Deck 测试结果、访问与分发许可、最后核查时间。

未核实字段使用 null 或 unknown，不通过名称相似自动填充。搜索摘录没有说明的包格式不能猜为 ZIP；页面列出的版本不等于已确认兼容当前 Steam build。

后续人工来源调查应优先使用 HGamer 指向的发行商官方站。只有得到固定文件、版本和安装证据后，才能把 discover_only 提升为 managed_zip。
