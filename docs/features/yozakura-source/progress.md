# YOZAKURA 内置订阅源施工记录

[设计](design.md)

## 2026-11-15

- 已核对官网目录、WordPress 文章接口和 Steam AppID。官网样例有三个独立 ZIP，文章 150 的 Steam 链接存在误指。
- 重新施工撤回了专门针对 YOZAKURA 的客户端分组和共享语言识别改动。Lua 负责站点解析和误链修正，并声明同一页面的直链属于同一补丁目标。核心复用已有下载项投影与精确选择。
- 完成配置驱动的内置脚本注册和 WordPress 文章 URL 兼容。旧嵌入方式计算出的 Shiravune、Kagura 国内、Kagura 海外、HGamer SHA-256 与新二进制读出的值逐一一致。
- Windows 隔离 Steam 夹具使用新构建 CLI 运行 `update_source`、`match_games`、`scan`。官网索引完整刷新，得到 5 个有补丁的作品。只有英文名称的两个样例分别匹配 AppID 4370020 和 4376090。文章 161 返回三个 ZIP，文章 150 返回三个 EXE。
- 用真实扫描结果直接执行 Windows Dart `displayEntries` 与 Decky 共用 TypeScript `displayPatches`。两篇文章各显示一个补丁条目及三个唯一下载选择，官网标签和文件地址逐项对应。
- Windows 定向验证通过。执行 `cargo fmt --all -- --check`、`cargo clippy --offline --locked -p patchhub-core -p patchhub-cli --all-targets -- -D warnings` 和 `cargo test --offline --locked -j 1 -p patchhub-core -p patchhub-cli`。测试使用允许回环网络的环境运行本地下载夹具。
- 官网真实分页未触发第二页。补丁实际下载和安装、客户端原生界面、Steam Deck 实机与线上 CI 待验收。
