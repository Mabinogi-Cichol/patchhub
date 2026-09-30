# Decky Alpha

已实现游戏模式列表、补丁详情、来源跳转、ZIP 导入、计划确认、事务卸载与恢复。固定 Rust CLI 通过 Python JSON-lines 桥接；宿主配置可信目录和状态目录，前端不能执行 shell。

构建：仓库根目录运行 `npm ci`、`npm run build:decky`；构建 Linux 核心后运行 `npm run package:plugin -w apps/decky`。输出 `plugin-output`，安装后将 `bin/steam-patch` 设为可执行，并以游戏所属用户运行。完整指南见 `docs/client-guide.md`。

真实 Steam Deck 核心扫描及隔离事务已通过；插件 UI 的实机加载 / 控制器验收正在进行。不要把合成测试记作真实成人补丁或游戏内内容验证。
