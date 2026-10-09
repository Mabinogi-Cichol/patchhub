# P5 Decky 切换

设计：[design.md](design.md)。施工记录：[progress.md](progress.md)。复核：[p5-gate-review.md](p5-gate-review.md)。日期：2026-10-10（Asia/Shanghai）。状态：P5 验收完成；已实现并正式部署，ZIP/EXE 真安装、写入回滚、实体手柄与两款游戏内生效均通过。

## 实施

- Decky Python 只配置固定 CLI/worker 与平台身份，桥接协议 v3 RPC、任务和来源事件。删除引擎切换入口与 Python 安装来源适配，生产包只包含六个宿主 backend 模块。
- 动态候选传 selection_id，签名 ZIP 传 release_id。核心每三秒续租，前台租约失效/取消后等待核心真实提交或回滚；终态不会被迟到遥测覆盖。九阶段和应用子阶段显示真实进度。
- 直接初始化当前 sources/rust-v1，保留其设置。旧任务和来源切换材料留在不读取目录；新任务历史位于 host-v3，自动清理偏好仍读取状态根。
- root Loader 按可信用户目录降权运行整个 CLI。仅初始化固定新目录；所有权异常拒绝，不递归 chown。
- 旧任务准入在正式替换前通过 Loader 停用；独立开发切换工具确认 RPC/磁盘终态一致后发布 schema 1 写阻断记录。三款旧 committed 游戏按实例与稳定 AppID 保守保护，Steam 移库也无法绕过；新生产运行时不读取旧 journal。
- Linux 构建同时产生 CLI 与 sf-worker，清单固定二者字节与 SHA；生产 ZIP 包含项目/参考许可和 CycloneDX SBOM（744 个组件，许可元数据无缺失）。
- 实装发现签名/本地 ZIP 可能在 stage 前发送 archive 遥测，CLI 输出边界将内部空默认阶段映射 connecting；宿主保持严格校验。新增真实 operation::run→archive→Cursor 回归。

## 自动化验证

| 检查 | 结果 |
|---|---|
| Python backend | 159 项通过，5 项按平台跳过；Linux 所有权额外 4 项通过 |
| Decky frontend | 最终 63 项通过；两客户端 typecheck 与 Decky build 通过；共享游戏状态 Windows 10 项通过 |
| Linux Rust core/CLI | core 库 274 + 集成 64 项通过，5 项忽略；首帧修复后 CLI 41 项通过；Linux CLI/worker release 成功 |
| SBOM / 打包 / 切换工具 | 1 / 6 / 6 项通过；白名单、ELF、哈希、许可与当前 SBOM 验证通过 |
| Windows Rust | 定向库/CLI 检查通过；完整集成链接出现 link.exe 环境错误，Linux 完整回归补齐；未把链接错误当作功能通过 |
| 独立复核 | 已报告问题闭合，具体见复核记录 |

## 正式部署与状态保留

本轮使用 Devkit 当前 Steam Deck，已核对并获用户确认本轮新 SSH 指纹。通过匹配的 Loader v3.2.10 正式本地 ZIP + SHA 安装流程替换。首次旧插件与完整状态备份、回滚 ZIP 均已取回本地并验 SHA，未覆写首次备份。

- 最终部署包（首帧及反馈归属修复后）：6,830,439 bytes，SHA-256 `93d954acac110a61ca34ee8321f16470bf53a96423b504ac8fcc49a7a456bc05`。
- 正式目录 21 文件逐项大小/SHA 一致；CLI 与 worker ELF 可执行；Loader active，运行插件实例严格为 1。
- CLI SHA `cf14ba30b9411b3175a08914de3cd470e2f582f3aefff62bad59c0ee1f6b95c4`；worker SHA `570300b39ca572f8ae77a4958fd8345a138de7b083d5d8bbd1c56a1e44b26890`。
- 实际 HTTP bundle 200、107,783 bytes，SHA `a43dab9d482ffc9bd1f76b0de9e8860fc302d93f08c5dcfcd77b667b19f12815` 与本轮构建相同；真实页面显示新保护文案与 EXE 版本/DLC条件。
- 切换前 10 任务、9 事务全部终态；7 本旧 journal 无未结束事务。旧 publisher/journal/tasks 的 56 文件与首次备份逐字节一致。
- 旧 committed AppID 1189440、1788100、2743810 写保护，保护 SHA `181e701b151b60d86ad1d74d3531b6418b152ba71342cbb6e87b49d9d9eb0c4f`；均扫描显示已有补丁需检查。
- 原来源 settings SHA `94bd041a5ab24648ede675b1e31ad011c46e6b87da97ab5b3f3ece213ab2dbc8` 未变，auto_cleanup=true 保留，Loader order/frozen/hidden/disabled 基线一致。
- host-v3 与 installer/rust-v1 为 deck:deck 700；实机 serve-task 安装进程 UID/GID 1000/1000。
- 能力协议 3，ZIP/EXE links.available=true；真实扫描 27 个游戏，诊断指出两处当前未挂载的 Steam 库，未当作空库丢数据。

## 隔离、游戏与界面验收

生产沙箱控制探针在本轮独立验证目录执行，18 项通过，覆盖 stage 权限失败关闭、非 root、只读输入、未授权文件/网络/子进程、环境/FD、输出限额、取消/超时/父退出回收、启动清理。该例程验证真实生产隔离控制，真实 sf-worker 转换另以安装结果判断。

用户另行明确授权《夜勤病栋》ZIP 重装及《夏日狂想曲》EXE 安装/取消恢复；完整两游戏基线 TAR 已取回本地，SHA `0b4ffb242ed642d81922f2763f119ae700adb0fc674943856c258422b99eafc4`。用户随后另行授权《妃神会秘史》简中 v4 EXE 实装，完整游戏首次备份同样已取回并验 SHA `ffd5affd0e98d10959318b7b24bb5254909f53130a621e6aa00df5db90ab777b`。

- ZIP 下载取消：实机前台任务下载 862,379,239 / 4,081,525,083 bytes 后请求取消，0.6 秒内观察到 cancelled；结果屏幕显示安全取消。对两游戏 7,182 原有文件完整对比，0 变化。此例在下载阶段取消，不能代替写入阶段回滚。
- ZIP 写入回滚：写入第 1 文件、4,823,213 bytes 后请求取消，实际核心 rolled_back；7,182 原文件 SHA 全恢复，新增残留 0，recover RPC 返回 []。ZIP 完整重装：真实终态 committed，事务 `9340e65b-17e7-450f-b49c-fbd05136d0fe`，自动清理 4,361,041,458 bytes；50 文件、4,128,944,753 bytes 全部匹配事务计划 SHA；复扫 installed_managed。实际观察到重新下载，缓存复用未通过验收。
- EXE：当前 Steam manifest 明确 schinese、DLC 1390150 已安装；选择 Summer Memories Plus DLC CN Patch v2.02.1，下载 279,515,928 bytes，SHA `de370e86645e273bae205517b49077a9fd682a9d73309c518f5524d96216ecad`。分析安全失败 `exe_analysis_failed` / `[worker] exe_project_options_unsupported`，未开始游戏写入；静态复核确认：SF9.5.2.0 的 CSetupFileData 尾字节（metadata offset 45133）为 1，通用 Rust 与 Python 图解析器都仅支持 0；5874 条记录均如此，样本与旧人工 profile 摘要完全一致。当前缺少该字段通用语义证据，保持安全拒绝，不能按旧 profile 摘要准入，不能标为真实 EXE 安装通过。
- 《妃神会秘史》EXE：用户扩展写入授权后通过真实页面启动简中 v4，selection_id 绑定；下载 1,228,278,216 bytes，通过 deck 用户 bwrap 沙箱分析/提取为 1,373,855,199 bytes ZIP，真实 committed（事务 `3772fd05-252f-4966-8992-057765433b2c`）；实际 1 个载荷文件、1,373,855,065 bytes，SHA 匹配计划。11 原文件仅计划内 1 文件改变，无新增游戏载荷，仅固定 0-byte `.steam-patch-manager.lock`；复扫 installed_managed。selection 与 content_id 绑定，自动清理完成后收据仍可读，recover=[]。
- QAM/全屏：真实设备截图已检查。QAM CSS viewport 854×454，全屏 854×534，未发现横向溢出；来源归属与 EXE 语言/版本/DLC、当前安装能力可见。补丁详情与订阅源底部滚动可达，操作记录与缓存设置页核对通过。正式包重载后已安装状态/事务历史仍在，重开无历史任务弹窗；QAM 复扫两款显示已安装补丁。
- 实机发现终态反馈跨游戏残留，已修复 note/error 的实例归属与异步晚到处理；新增 3 回归，正式包更新后页面核对。
- 实体手柄与游戏内生效：2026-10-10 用户明确确认“实体手柄与两款游戏均通过”，对应本插件 QAM/全屏焦点、A/B 返回，以及《夜勤病栋》《妃神会秘史》运行与补丁生效。实体手柄验收依据用户确认；安装取消/回滚依据上述独立 RPC 与文件摘要证据。

详细证据与首次备份保存在忽略目录 `.artifacts/p5-decky/`；正式部署清单 `deploy-manifest.json`，旧状态备份 `pre-cutover.tar.gz`、回滚包 `rollback-plugin.zip`、游戏备份 `game-baseline.tar`，RPC/屏幕/沙箱 JSON 和截图按本节文件名归档。原始库路径只留本地证据，不进入发布文档。

## 归档与保留

首次插件/状态、回滚 ZIP 和三款游戏完整备份保留本地；远端证据归档 `remote-evidence.tar.gz` SHA `259ec0b5a3452f515b48a0f6dd4c8b6e9275bd06b25c399e55c048a6e28cb7a2`，16 文件逐项验哈希。归档校验与无活动任务确认后，严格检查绝对路径并清理本轮远端暂存目录；仅关闭本轮 SSH 隧道，正式插件/state/游戏未清理。验收收尾时尚未创建 Git 提交；后续用户要求的 Git 归档见 [施工记录](progress.md)，P6 未启动。2026-10-10 用户确认实体手柄与两款游戏通过，关闭此前人工待验项，P5 按本轮范围结项；Summer DLC 未支持格式与缓存复用未通过的事实保留，不扩写为通过。
