# Decky UX 施工进度与验收记录

- 设计依据：[`design.md`](./design.md)
- 问题与修复计划：[`fix-plan.md`](./fix-plan.md)
- 更新日期：2026-10-04

## 施工进度（持续更新）

文档归类记录（2026-10-04）：将 `docs/decky-ux/` 迁入 `docs/features/decky-ux/`，保留既有设计、修复计划及施工记录并更新相关引用；后续遵循仓库根目录 `AGENTS.md` 的功能文档规则。此次归类只检查目录与文档链接。

| 工作项 | 状态 | 证据 / 下一步 |
| --- | --- | --- |
| 方案确认与任务拆分 | 已完成 | 用户授权 multi-agent 开始施工；保留此前未提交修改 |
| 侧栏游戏列表、名称滚动、原生二级导航 | 已实现，实机已验证路由与栏目结构 | 实机确认原生侧栏 4 栏目与概览、订阅源页渲染 |
| 概览匹配源与补丁管理布局 | 已实现，实机已验证概览 | 概览显示匹配来源 patch.moe 与实时查询状态 |
| 原生遮罩进度面板 | 已修复并部署，原生渲染验证通过 | 0–100 参数、限高与底部按钮实机验证；真实下载 / 安装与手柄操作待验收 |
| 下载 / 安装进度与安全取消 | 已实现并通过后端测试 | 真实字节上报、任务租约、取消竞态和安全恢复测试通过 |
| 自动化测试与审查 | 已通过代码检查与测试 | Rust 55 项、Python 45 项（Deck）；前端 19 项（本轮新增 3 项路由编码测试）；格式检查、Clippy 与两个前端类型检查通过 |
| 实机路由与顶部布局修复 | 已修复并部署验证 | URL-safe base64 路由段；`.patch-page` 顶部内边距；实机截图确认 |
| 侧栏顶部标签与分组分割线 | 已实现并实机验证 | `showTitle` 渲染两行不可聚焦标签；栏目 `visible:true` 使 separator 生效；概览截图确认 |
| 右侧四页排版重构 | 已实现并实机验证 | 移除重复游戏名与临时内边距；统一 `Field` 键值行；概览双按钮同行；五处界面可见文本与截图核对 |
| 已安装状态分支与接管入口 | 已实现并实机验证 | 已安装不再显示安装准备；有事务显示卸载；外部安装提供“重新安装并接管”，成功后即可卸载 |
| Steam Deck 部署与实机验收 | 已部署，验收中 | 逐文件哈希复核一致；概览、订阅源、补丁管理页实机通过；侧栏切换已修复；事务残留已清理；进度面板与取消待手柄确认 |
| 安装后下载缓存清理 | 已部署，后端实机验收通过，UI 待手柄 | 自动清理开关（默认开启）+ 备份保留开关（默认关闭）+ 独立侧栏“下载缓存”页 + 事务备份/暂存清理；设计见 [cache-cleanup](../cache-cleanup/design.md)、进度见 [progress](../cache-cleanup/progress.md) |
| 全局设置入口与页面往返 | 已修复并部署，实机验证通过 | 侧栏入口改名“全局设置”；订阅源与缓存改用同级子路径；连续两轮往返通过 |

### 施工记录

- 2026-10-04：按用户要求，本次旧版前端备份已取回本机 `.artifacts/decky-ui-feedback-20261004-backup/`，两个文件 SHA-256 与 Deck 原备份一致；删除 Deck 上本次整个 `/home/deck/steam-patch-validation/decky-ui-feedback-20261004` 暂存目录（含旧版备份、上传文件、manifest 与部署脚本），核实目录不存在。后续备份与临时文件放本机，设备端只保留运行所需文件。

- 2026-10-04：按用户反馈修复进度显示、弹窗过高、全局设置入口文案与下载缓存无法切回订阅源四项问题。使用一名 worker（`gpt-6-sol / medium`）实现独立进度视图，主 Agent 集成并修复设置导航。读取 Steam 原生 UI 源码确认 `ProgressBar.nProgress` 直接用于百分比宽度（原实现将 0–100 再除以 100）；新增进度百分比函数、异常 / 未知总量处理与两项回归测试。弹窗内容限高、长名称 / 路径置于滚动区、底部取消 / 返回按钮固定可见，并按原生外层弹窗留白修正高度及 CSS 优先级。全局设置页改为 `/sources/subscriptions` 与 `/sources/cache` 同级路径，避免原生侧栏前缀匹配抢占；游戏页“全局设置”入口跳转到包含订阅源与缓存的全局页面，旧 `/sources` 入口仍默认显示订阅源。
- 2026-10-04：本轮 25 项前端测试、Decky 类型检查、Rollup 构建及 `git diff --check` 通过。通过 Steam CEF 在真实原生组件中渲染合成任务：50% / 100% 分别得到 `width:50%` / `width:100%`，未知总量不设宽度；当前逻辑视口 854×534 中，超长名称和文件路径时弹窗范围为 y=73–459，底部按钮区域 y=367–441，内容区确有内部滚动。部署前确认无活动任务，仅更新前端 `dist/index.js` / map 并逐文件核对 SHA-256（JS `118ad749…46661`），旧版备份在 `/home/deck/steam-patch-validation/decky-ui-feedback-20261004/backup`。Decky 热重载后仅有一个插件实例；点击原生侧栏连续两轮“下载缓存 → 管理订阅源”往返，路由、标题和内容正确；游戏页“全局设置”点击后进入全局订阅源并显示缓存栏目。未发起真实下载 / 安装；实体手柄焦点与滚动仍待用户验证。

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
- 2026-10-04：执行 `fix-plan.md` 修复。先通过 SSH 与 Steam CEF 调试端口（`deck@192.168.31.186`，`127.0.0.1:8080`）复制 Steam 原生 UI chunk（`steamui/chunk~2dcc5aaf7.js`）并读取内层组件源码，确认两点机制：其一，`PagedSettings` 的 `showTitle` 为真时，`title` 渲染在左栏顶部的 `PagedSettingsDialog_Title` div 内，且该节点不可聚焦；其二，`M` 组件渲染 `"separator"` 前会过滤——只有当分隔线之后存在 `visible` 为真值的页面时才保留，原实现未设 `visible`，因此分隔线被丢弃。据此实施：`SidebarNavigation` 传 `showTitle` 与两行不可聚焦标题节点（工具名 + 游戏名/全局设置）；四个游戏栏目与全局源栏目补 `visible: true`；移除四页内容中的 `<h1>{game}</h1>`，由侧栏标签与原生 `DialogHeader` 分别承担游戏名与栏目名；新增 `InfoRow`（原生 `Field` 左标签右值）统一概览、补丁管理、操作记录、订阅源五处键值信息；概览补丁卡片改为名称、匹配源与“查看补丁 / 打开来源网页”双按钮同行；移除 `.patch-page` 的 56px 临时顶部内边距。
- 2026-10-04：本轮修复的自动化检查与部署。`npm run typecheck -w apps/decky`、19 项前端测试、Rollup 构建均通过；仅部署前端 `dist/index.js`（本地与 Deck `sha256` 均为 `272903a3…fd794`），旧文件备份于 `/home/deck/steam-patch-validation/ux-ui-20261004/backup-dist`。用 Decky `importPlugin` 热重载时出现新旧实例重名问题，改用 `restartWebhelper` 让加载器按后端清单重新加载，确认插件 `onDismount` 与路由 `/steam-patch-manager/:instance/:panel?` 均正常注册。
- 2026-10-04：实机验收（CEF 纤维树、可见文本与截图）。概览、补丁管理、操作记录、游戏内“管理订阅源”与全局订阅源五处均通过：左栏顶部显示不可聚焦的“Steam 补丁管理”与游戏名/“全局设置”（检测 `.patch-nav-header` 内可聚焦元素数为 0），游戏名不再在右侧重复；`role="separator"` 分组线高度 1px 且截图可见；键值信息左右对齐；概览双按钮同行；全局源页逐项显示名称、站点、启用状态、查询结果、查询或缓存与缓存时间；侧栏栏目名与原生标题一致。仍未完成：受当前数据量所限，四页内容未产生滚动溢出，切换栏目保留滚动位置与长名滚动未能实机复现（相关 `panelScroll` 机制本轮未改动）；手柄 A/B 焦点逐项遍历需实体手柄，本轮仅确认标签不进入焦点序列。
- 2026-10-04：修复用户反馈“补丁已安装但补丁管理页底部仍显示安装准备”。定位到补丁管理页无条件渲染“安装准备”面板，仅在已托管安装时禁用主按钮；对外部安装（`external`）按钮甚至保持可用，且与详情内的“不能自动卸载”说明并存。按 `design.md` 5.1 / 5.3 改为按安装状态分支：未安装才显示“安装准备”；已安装且存在本工具 `committed` 事务时显示“已安装”并提供“卸载补丁”；外部安装显示“通过其他方式安装；本工具没有原文件备份，不能自动卸载”；检测到已安装但无本工具事务记录时明确提示无法自动卸载。移除详情面板中重复的外部安装说明，并把原先单独的卸载面板合并进“已安装”面板。本地 `typecheck`、19 项测试与 Rollup 构建通过；部署 `dist/index.js`（本地与 Deck `sha256` 均为 `fe328c6e…b520dc`），重启重载后实机确认补丁管理页不再出现“安装准备”，底部为“已安装”与外部安装说明。
- 2026-10-04：按用户确认增加“重新安装并接管”入口。外部安装没有本工具事务与备份，`uninstall` 只能读取本工具的 `committed` 事务并用 `original_sha256` 还原，因此无法直接卸载。现在已安装面板在无本工具事务时增加说明与入口：重新走一次准备/安装，在写入前备份当前文件，成功提交后本工具即拥有可用于卸载的事务；该操作会覆盖当前补丁文件，有阻碍时禁用并显示原因。有 `committed` 事务时仍优先显示“卸载补丁”。本地 `typecheck`、19 项测试与 Rollup 构建通过；部署 `dist/index.js`（本地与 Deck `sha256` 均为 `b4b07c7b…28a52e`），重启重载后实机确认外部安装补丁显示“通过其他方式安装”说明与“自动下载并重新安装”入口。
- 2026-10-04：实现安装成功后清理下载缓存。后端新增 `plugin_settings.py`（自动清理开关，默认开启）、`caches.py`（枚举 / 统计 / 清理）与 `publisher.clear_cache/caches`；`main.py` 在 `start_install` 返回 `committed` 后按开关删除本次下载的 `publisher/.../cache/archive.zip`（含 `download.json`）与 `downloads/<sha>.zip`，判定依据是推导出的缓存路径 + `no_links` + 归属校验，不依赖 `acquisition`，避免误删用户 `~/Downloads` 与事务备份。前端新增 `src/cache.ts` 与“管理订阅源 > 下载缓存”分区（开关、占用、清理确认弹窗）。新增后端 17 项与前端 3 项测试；`typecheck` 与 Rollup 构建通过。
- 2026-10-04：部署并实机验收下载缓存清理。备份后按 SHA-256 上传到 Deck 插件目录并触发 reload；首次 reload 暴露 `decky_loader.settings` 与插件 `settings.py` 同名冲突导致启动 `ImportError`，改名为 `plugin_settings.py` 后正常。经 Steam CEF（隧道 `18080→8080`）连接 Decky 路由，实时调用 `get_settings`（`auto_cleanup: true`）、构造缓存后 `cache_status`（`319B / 5 文件`，精确区分 publisher 109B / core 210B）、`clear_cache`（`removed_bytes: 319` 并清零、保留非摘要命名的用户文件）与 `set_auto_cleanup` 往返均通过；验收残留已清理。前端 bundle 含新分区文案。真实安装后自动删除与手柄 UI 渲染未做（不触发约 4GB 下载），列为待办。
- 2026-10-04：按用户反馈扩展“下载缓存”。查明 `.stage`/`.backup` 是核心事务文件（设备上共 7.7G）而非下载缓存，且面板原先只在全局“管理订阅源”页底部。新增：安装提交后自动删除该事务 `.stage`；`cache_status` 增加 `jobs` 占用；`clear_transaction_backups` 清理 `committed`/`uninstalled`/`rolled_back` 事务的暂存与备份（进行中/需恢复保留）；把入口改为全局设置中与“管理订阅源”并列的独立侧栏页“下载缓存”，含“下载缓存”“事务备份与暂存”两个分区。重新部署并重载，实机 `get_settings` 显示 stage/backup 各 `4,128,944,753` 字节；部署后的 `caches.py` 在 Deck Python 3.13 上用临时目录验证只删目标文件、保留无关文件；`dist/index.js` 含新分区文案。未点击真实清理（会删除 7.7G 并禁用该补丁卸载），交由用户确认。
- 2026-10-04：按用户确认立即执行真实清理，状态目录从 7.7G 降到 344K。新增备份保留开关 `keep_backups`（默认关闭）：关闭时安装提交后同时删除 `.backup`，默认无法卸载、只能重新安装；开启时保留备份以支持卸载。`list_transactions` 增加 `backups` 字段，无备份事务隐藏卸载按钮，后端 `uninstall` 以 `uninstall_backups_missing` 拒绝；“下载缓存”页“事务备份与暂存”分区新增该开关。重新部署（备份 `cache-cleanup-20261004-3`）并重载，实机验证 `get_settings`（`keep_backups:false`）、`set_keep_backups` 往返、事务 `backups:false`、卸载拒绝与 bundle 文案均通过；未做真实安装后的自动删除与手柄 UI 渲染。
- 2026-10-04：修复默认（无备份）模式下“无法重新安装”。核心 `install::prepare` 的 `ensure_no_active` 会因旧 `committed` 事务拒绝新准备，而默认关闭备份后该事务无法卸载。因本机与 Deck 都没有 Rust 工具链，采用 decky 侧桥接：`scan` 不再因无 `.backup` 的 committed 事务产生 `publisher_existing_transaction`（有备份仍阻止）；`prepare_install` 在准备前对无任何备份的状态目录删除核心 `journal.sqlite*` 与 `jobs/`，保留 `catalog.json`/签名/密钥。重新部署（备份 `cache-cleanup-20261004-4`）并重载，实机扫描该补丁 `blockers` 为空、真实状态目录验证只清 journal/jobs、清理后 `list_transactions` 为空且检测为 `installed_external`、可进入重新安装；后端 72 项测试与类型检查通过。
- 2026-10-04：按用户要求隐藏游戏列表下方的离线库警告。仅在 Decky 侧栏渲染时过滤 `library_unavailable`，核心诊断与其他扫描警告保留。类型检查与 Rollup 构建通过；前端已备份并部署到 Deck（备份 `/home/deck/steam-patch-validation/offline-library-20261004/backup`，`index.js` SHA-256 `d992059f…b9a4ff`）并热重载。通过 Steam CEF 调用插件真实 scan（34 个游戏，含离线 UUID 库诊断），对已加载侧栏组件使用该报告进行渲染核验：离线库警告为 0，现有筛选下保留 1 个游戏，插件实例为 1；此项为组件渲染验证，未做手柄截图验收。

## 2026-10-04 提交前验证

- 本次提交包含游戏订阅源选择、全局设置路由与进度面板修复：按 AppID 持久化源选择，全局设置使用同级路由；进度参数为 0–100，面板限高且底部按钮持续可见。已同步设计说明，本轮未新增实机验收。
- 前端 25 项测试、Windows / Decky 类型检查、Decky Rollup 构建和 53 个本地文档链接检查通过。
- 后端全量 78 项中 74 项通过、1 项符号链接用例跳过，3 项 CLI 桥接集成用例因缺少 `target/debug/steam-patch.exe` 报环境错误；单独运行本次修改涉及的 `test_sources` 与 `test_acquisition` 共 26 项全部通过。
- 未将缺少核心程序的集成检查记为通过，本轮未新增网络或实机验收。

## 2026-10-04 游戏订阅源实机验证

- 本轮按用户要求将游戏详情底部入口改为独立“订阅源管理”，使用游戏自身的 `/sources` 子路由。每个源提供“用于此游戏”开关和游戏匹配结果，按 AppID 持久化；全局网站配置继续从主面板进入。扫描和准备安装均遵守游戏选择，未选择的源仍可查看匹配结果并重新选择。
- 本地前端 25 项测试、Decky 类型检查和 Rollup 构建通过；后台 75 项中 74 项通过、1 项符号链接用例跳过。全量中的另外 3 项 CLI 桥接用例缺少本机 `target/debug/steam-patch.exe`，未验证。新增覆盖游戏之间隔离、持久化、旧配置兼容、全局修改保留游戏选择，以及关闭后拒绝准备该源补丁。
- 已备份并部署 `main.py`、`backend/sources.py`、`dist/index.js` 和 sourcemap，备份目录为 `/home/deck/steam-patch-validation/game-sources-20261004/backup`。部署后哈希与本地一致：前端 `8dce2e79…4380670`，入口 `ef1d12e7…0c2dfc5`，源注册表 `cbb60357…be40748`；`plugin_loader` 为 active，加载插件实例为 1。
- Steam CEF 实际挂载页面验收：从《夜勤病栋》概览点击左侧“订阅源管理”，仍显示当前游戏标题与“用于此游戏”，不出现全局规则 JSON 表单。通过实际原生 checkbox 点击关闭再开启，页面更新并恢复匹配补丁；单独 API 验证关闭该游戏的 patch.moe 后不再匹配该源，其他 37 个游戏的源选择及全局源状态保持原状，随后恢复测试前选择。本次没有触发安装或下载补丁，实体手柄遍历和截图验收未执行。
- 证据脚本：`.artifacts/game-sources-ui-check.js`、`.artifacts/game-sources-live-check.js`、`.artifacts/game-sources-toggle-check.js`；同名脚本保存在上述 Deck 验证目录。
