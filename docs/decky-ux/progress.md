# Decky UX 施工进度与验收记录

- 设计依据：[`design.md`](./design.md)
- 问题与修复计划：[`fix-plan.md`](./fix-plan.md)
- 更新日期：2026-10-04

## 施工进度（持续更新）

| 工作项 | 状态 | 证据 / 下一步 |
| --- | --- | --- |
| 方案确认与任务拆分 | 已完成 | 用户授权 multi-agent 开始施工；保留此前未提交修改 |
| 侧栏游戏列表、名称滚动、原生二级导航 | 已实现，实机已验证路由与栏目结构 | 实机确认原生侧栏 4 栏目与概览、订阅源页渲染 |
| 概览匹配源与补丁管理布局 | 已实现，实机已验证概览 | 概览显示匹配来源 patch.moe 与实时查询状态 |
| 原生遮罩进度面板 | 已实现，实机验证中 | 已修复取消 / 断连竞态和已处理结果重放 |
| 下载 / 安装进度与安全取消 | 已实现并通过后端测试 | 真实字节上报、任务租约、取消竞态和安全恢复测试通过 |
| 自动化测试与审查 | 已通过代码检查与测试 | Rust 55 项、Python 45 项（Deck）；前端 19 项（本轮新增 3 项路由编码测试）；格式检查、Clippy 与两个前端类型检查通过 |
| 实机路由与顶部布局修复 | 已修复并部署验证 | URL-safe base64 路由段；`.patch-page` 顶部内边距；实机截图确认 |
| Steam Deck 部署与实机验收 | 已部署，验收中 | 逐文件哈希复核一致；概览、订阅源、补丁管理页实机通过；侧栏切换已修复；事务残留已清理；进度面板与取消待手柄确认 |

### 施工记录

- 2026-10-03：开始施工。两条工作流并行：前端 `gpt-6-sol / medium`，后端 `gpt-6-astra / high`；主 Agent 负责集成、文档和部署。当前仅标记实际启动的工作，未将源码检查当作实机验收。
- 2026-10-03：确定异步任务接口：启动准备 / 安装返回任务 ID；状态查询续约 15 秒租约，取消接口请求安全停止。状态区分执行、取消、回滚和终态，进度包含字节数、文件数、大小、速度和结果。接口正在实现，尚未完成验收。
- 2026-10-03：Deck 解锁后 SSH 连接恢复。检测到无现成 Rust/C 编译器，在 `/home/deck/steam-patch-validation/ux-build/tools` 准备隔离 Rust 1.94 与 Zig 工具，不修改 SteamOS 系统分区。
- 2026-10-03：隔离工具已可运行。前端初版类型检查和打包通过（UI Agent 报告，待集成复验）；两名子 Agent 随后因额度限制停止，主 Agent 接手未完成实现、测试与审查。独立 Agent 审查尚未完成，不能标记为已通过。
- 2026-10-03：用户要求继续使用子 Agent 后，两名原 Agent 已恢复施工；新增只读 reviewer（`gpt-6-astra / high`）审查取消与回滚安全性。Deck 再次断连，本地测试继续。
- 2026-10-03：按用户最新要求停止所有 Astra Agent，后续模型最高 `gpt-6.1-sol`。后端由 `gpt-6.1-sol / high` 接续保存的实现，前端继续使用 `gpt-6-sol / medium`；主 Agent 接手审查。Rust 依赖与核心代码已实际编译，CLI 首次构建因源码包漏带 `catalog/trust-keys.json` 失败，补齐后复验。
- 2026-10-03：补齐源码包后，Deck 隔离目录的 `cargo test -p steam-patch-core -p steam-patch-cli --locked` 通过，安装相关 27 项包含取消与恢复；Python 新增任务测试通过。Windows 本地 3 项真实 CLI 桥接测试受缺少 Windows 二进制限制，改在 Deck 构建 CLI 后验证，未将环境错误隐藏为通过。
- 2026-10-03：最新后端通过 Rust 55 项（安装 30 项）、Python 45 项（含真实 CLI 桥接）、`cargo fmt --all -- --check`、Clippy `-D warnings`；Linux release 构建完成。Windows 前端类型检查通过。取消等待最多为当前网络连接 / 读取的剩余 15 秒；断连还需先等待 15 秒租约到期，期间不会把取消请求误报为已结束。
- 2026-10-03：前端 16 项测试、类型检查及 Rollup 打包通过。新版 Python / Rust / 前端已部署，逐文件哈希校验成功，旧版备份位于 `/home/deck/steam-patch-validation/ux-ui-20261003/backup`。通过 Steam 现有后端连接重载，实际收到前端导入完成日志；未以文件部署成功替代实机界面验收。
- 2026-10-04：接管续作并核对进度。重新运行本地前端测试（19 项，含本轮新增 3 项路由编码测试）、类型检查和 Rollup 打包，均通过；Python 侧 42 项通过，3 项真实 CLI 桥接因 Windows 缺少 `target/debug/steam-patch.exe` 未运行（Deck 侧此前已通过），未把环境缺失记作失败。使用 Steam Devkit 密钥（`C:/Users/wqj78/AppData/Local/steamos-devkit/steamos-devkit/devkit_rsa`）恢复与 `deck@192.168.31.186` 的 SSH。
- 2026-10-04：逐文件复核 Deck 部署哈希与本地一致（`main.py`、`dist/index.js`、`dist/index.js.map`、`bin/steam-patch`、`backend/*.py`），插件进程于 2026-10-03 23:58:19 重载后持续运行且日志无异常。实机确认新版前端已加载：全屏路由由原生侧栏导航渲染，栏目为概览 / 补丁管理 / 操作记录 / 管理订阅源；概览显示匹配来源 `patch.moe` 与“已实时查询”；订阅源页显示启用状态、160 条结果、缓存时间及原生开关。
- 2026-10-04：修复实机发现的两个问题。其一，`instance_id` 含 `/`，Steam 路由只部分解码单个 URL 段，导致点击游戏进入“游戏已不在当前扫描结果中”降级页；改用 URL-safe base64 路由段，新增 `apps/decky/src/route.ts` 及 3 项单测，`sources` 入口单独走不编码路径。其二，全屏内容顶部被系统状态栏遮挡；将 `.patch-page` 顶部内边距调整为 56px。重新构建后备份并部署（`backup-route-fix`、`backup-route-fix-2`），通过 Steam 现有前端 `importPlugin` 重载，实机截图确认概览页、订阅源页正常且顶部不再遮挡。
- 2026-10-04：实机数据中 3239270 仍保留历史 committed 事务（`night-shift-nurses-20241120`）与一个未作废的 `planned` 计划，使已匹配补丁显示 `publisher_existing_transaction` 阻断、安装准备按钮不可用。补丁管理页各状态布局、下载 / 安装进度面板与取消仍需手柄实机确认，未将结构验证记作完整 UI 验收。
- 2026-10-04：经用户授权清理 3239270 的验证事务残留。先备份 `journal.sqlite` 与 publisher `journal.sqlite` 为 `.bak-20261004`，再删除 4 条 base 事务与 1 条 publisher `planned`，未调用 uninstall、未改动游戏文件；`scan` 复核该补丁 `blockers` 为空。本轮 UX 施工改动已提交为 `61bc3e9`（27 个文件）。publisher 缓存为空，界面“自动下载”会重新下载约 4GB 补丁，未在无人值守时触发，进度面板与取消的实机确认保留给手柄操作。
- 2026-10-04：修复用户反馈的“侧栏点击后不切换、选中态不变”。定位到 `@decky/ui` 导出的 `SidebarNavigation` 是路由驱动：当前页由 `matchPath(location.pathname, page.route)` 决定，切换通过替换 URL；且先注册的 `/steam-patch-manager/:instance` 前缀匹配会吞掉子段，使第二个路由永不命中。改为单一可选参数路由 `/steam-patch-manager/:instance/:panel?`，每个栏目使用完整子路由，`fnSetNavigateToPage` 注册内部切换器供“查看补丁”等程序化跳转，`disableRouteReporting` 避免联动 Steam 路由器。实机点击“补丁管理”后选中态与右侧内容（补丁详情、匹配源、当前状态）均正确切换。
- 2026-10-04：按用户要求整理文档。将 `docs/decky-ux-design.md` 迁移到专属目录 `docs/decky-ux/`，拆分为 `design.md`（方案）与本文件 `progress.md`（进度），并新增 `fix-plan.md`（问题与修复计划）。对照设计文档与实机渲染，确认待修复差异：侧栏缺少工具名与游戏名不可交互标签、缺少“管理订阅源”上方分组线、右侧面板重复渲染游戏名且信息未左右对齐。相关文档引用已更新。
