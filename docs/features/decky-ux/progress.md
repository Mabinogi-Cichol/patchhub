# Decky UX 施工进度与验收记录

## 2026-10-07 前台安装与结果面板调查

- 状态：要求修订、代码调查完成，修复待实现；未构建、部署或实机验证。当前要求见 [设计](./design.md)。
- `apps/decky/src/task-progress-view.tsx` 第 47、67–73 行：原生关闭走 onClose，正文明确关闭后安装继续，并渲染两个按钮。终态仍显示进度条与统计。
- `apps/decky/src/index.tsx` 第 217–229 行：取消与关闭分开处理，onClose 仅关闭模态；已取消自动关闭，完成 / 失败继续用进度视图。第 275 行页面卸载未请求取消。
- `apps/decky/src/task-controller.ts`：轮询未绑定窗口可见性，dismount 仅停止观察；启动响应在卸载后返回时未取消，resume 可重新观察非终态任务。
- `apps/decky/backend/tasks.py`：后端明确允许离开 UI 后继续；lease 字段与参数仍在，但没有租约超时检查。`apps/decky/main.py` 第 607–613 行查询不续约，只有显式 cancel_task 请求取消。当前代码确实允许后台继续安装。
- 本地 `@decky/ui` ModalRoot 提供原生关闭回调，showModal 的 Close 不触发关闭回调。需统一退出取消链路、处理启动期间退出、补齐前台生命周期与后端超时保障、拆分结果视图；无内容区按钮时的原生提示和焦点待实机核验。
- 本轮仅调查和文档更新；插件修复、取消恢复检查、原生提示与结果面板实机验收待执行。

- 设计依据：[`design.md`](./design.md)
- 问题与修复计划：[`fix-plan.md`](./fix-plan.md)
- 更新日期：2026-10-07

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
| 前台安装、原生关闭取消与结果面板 | 已部署，运行时 / RPC / 真实页面与合成渲染验证通过；真实下载 / 恢复与手柄待验收 | 39 文件哈希一致；HTTP bundle 与本地构建一致；进度面板无旧按钮、正文提示取消；原生关闭触发 cancel_task；终态切换独立结果面板 |

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

## 2026-10-06 补丁来源文案修正

- 按用户反馈移除匹配补丁的“官方 EXE / 官方 ZIP”概括标签，直接显示订阅源名称；EXE 同时补齐 source_name，中文语言使用可读名称。新安装生成的本地目录名称同样使用来源名称。
- 安装方式、来源按钮及相关 EXE 引导提示改用中性文案；来源身份、审核、下载白名单和安装判断保持原有逻辑。
- 离线旧安装记录在扫描投影时重新生成来源名称，按来源页域名识别订阅源，找不到配置时显示域名或“来源未知”；不改写既有本地签名目录。
- 本地既有 EXE 安装与扫描投影检查 22 项：21 项通过、1 项跳过（Windows 符号链接条件）；Decky 类型检查、Rollup 构建与 git diff --check 通过。本轮未部署到 Steam Deck，界面/实机待验证，未提交 Git。

## 2026-10-06 来源文案部署与实机验证

- 用户授权“部署并验证”。通过现有 Devkit 配对核实设备 `192.168.31.186`、插件身份及 Loader `v3.2.10`；部署前两条历史任务为 completed / cancelled，无非终态安装任务。沿用上节本地检查和构建，未重复运行。
- 插件根目录为 root 所有，采用当前 Steam 已认证连接的 Loader 本地 ZIP 安装。新包 33 个文件；相对旧插件只有 `main.py`、`backend/exe_installation.py`、`dist/index.js` 及 sourcemap 共 4 项内容变化，Linux CLI 和其他正式文件哈希相同，未打包旧自动生成的 Python 字节码。包 SHA-256：`c3a05752005a724b43e71b36fc023c0213183a851b4ba6ddc20fd2529965cb4d`。
- 首次完整备份 34 个旧文件，回滚 ZIP SHA-256：`cece8cd03006ed6986224fdf3113e30b3052577857daf929e500e7c5616b25aa`。已取回至忽略目录 `.artifacts/source-label-deck-20261006-1507/remote-evidence/`，逐文件及归档摘要复核通过；证据归档 SHA-256：`199b1b47289a579c9cf2c531c4301e44b5545dd685b2d0c26d1dd138f5c8ee0e`。
- 正式目录 33 个文件逐项 SHA-256 通过，设备实际 HTTP 前端响应与本地构建一致：`9f1d7a6b7ff86e55a7db393c618a9994d46ed6ff571ba9fcc79fa9f9082464bf`。Loader 服务 active，插件实例 1，真实 list_tasks / list_transactions 正常。安装造成的本插件排序变化已合并恢复，冻结、隐藏、停用列表与部署前一致。
- 真实 scan(false) 返回 39 个库实例，来源扫描 37 / 37 完成。VBF build 5430305 返回 `VenusBlood FRONTIER International · Kagura Games 国内 · EXE · 简体中文`、`source_name=Kagura Games 国内`，现有文件检测 installed_managed；8 条事务的身份、状态和错误字段与部署前一致。重载时 5 个 journal.sqlite 摘要变化，对应既有启动迁移重新保存终态记录的行为；未删除数据库或安装记录，未执行游戏补丁下载/安装。
- 初始 scan RPC 超过 CDP 25 秒等待窗口，后续读取确认完成。页面切换期间有一次旧扫描 generation 的 scan_stale 日志；最终页面无错误且扫描完成。已通过当前五参数 importPlugin 显式重导入；旧补丁选择弹窗关闭后仍留下遮罩，确认无活动任务后按技能重启 Steam JS 上下文，重新检查全部运行状态与画面。
- 真实 QAM 和 VBF 补丁管理页截图已查看：具体来源可读，旧“官方 EXE”标签消失，安装方式为“已审核 EXE 安装”，页面无横向溢出，滚动到底部可见来源链接与重新安装按钮。CSS viewport 854×534，屏幕图 1281×801。证据：`.artifacts/source-label-deck-20261006-1507/qam-final.png`、`vbf-final.png`、`vbf-bottom.png`、`final-ui.json`、`final-remote.json`。DevTools 导航/滚动只证明对应实机界面行为，实体手柄操作未验收。
- 已恢复 VBF 补丁管理页顶部、关闭本次菜单，用户筛选保持“有补丁”，订阅选择未变；已在取回且校验备份后仅清理本轮远端暂存目录。未提交 Git。


## 2026-10-07 前台安装与结果面板实现及自动化验证

- 状态：已实现，自动化验证通过；尚未部署，Steam 原生提示 / 焦点与实体手柄待实机验收。当前设计见 [design.md](./design.md)。本节更新前面调查阶段的“修复待实现”状态，历史记录保留。
- 移除“取消安装”“关闭进度窗口”和结果页返回按钮。使用 ModalRoot / Focusable 原生关闭提示；运行中原生关闭 / Esc 统一请求取消，遮罩点击不关闭。取消期间保留恢复状态，终态切换独立 TaskResultView，只展示实际结果与原生关闭提示。
- 进度面板在启动 RPC 返回前打开，期间关闭会排队取消；启动响应在页面卸载后返回也会请求取消。实际离开管理页、隐藏页面、卸载模态或插件会取消并停止续约。重连只取消遗留非终态任务并观察必要恢复，不重新开始安装。
- 后端恢复 15 秒前台租约及独立监视器；公开 task_status 查询续约，list_tasks / 历史查询不续约，超时后的查询无法复活任务。超时只设置受控取消标志，不强杀写入进程；沿用现有安全恢复和提交优先规则。
- 正常完成、失败、已安全取消、仍需恢复 / 后端中断分别显示实际结果，取消不再自动跳过结果面板；关闭终态仅清除界面状态，不发出取消或卸载请求。
- 前端全部 51 项测试通过，含任务启动 / 卸载竞态、取消重试、安全恢复观察、重连结果及实际 JSX 的无按钮 / 无终态进度渲染。Decky 类型检查和 Rollup 构建通过。
- 后端 task / history 16 项和安装编排 5 项（合计 21 项）通过，覆盖断连超时、租约续约、列表不续约、迟到查询拒绝续装、等待安全恢复、提交竞争、恢复失败和公开 RPC 接线。
- git diff --check 通过。普通沙箱命令无返回，获自动审批后使用本地执行完成修改与检查。未进行远程部署、真实下载 / 写入或实体手柄操作；不把模拟原生控件的渲染检查记作 Steam 实机验收。未提交 Git。

## 2026-10-07 前台安装与结果面板部署与实机验证

- 状态：已部署到 Steam Deck，运行时、真实 RPC、真实页面与合成任务渲染验证通过；真实下载 / 安装的取消与恢复、实体手柄焦点与原生按键验收待完成。设计见 [design.md](./design.md)。本节取代上面“实现及自动化验证”小节中“尚未部署”的状态，历史记录保留。
- 部署来源：本轮开始前同一功能已有一次 Loader 本地 ZIP 安装（39 文件，包 SHA-256 `27c42406587741b3cb92a0cf200c1eb67d65c17b7b84d1cdf0814858cd5d5c57`，远端暂存 `foreground-install-deck-20261007-160602`）。本轮对该结果做完整校验并补齐验证，未重复安装。设备 `192.168.31.186`，插件身份 `Steam 补丁管理`，插件目录 `/home/deck/homebrew/plugins/steam-patch-manager`（root:root 755，走 Loader 安装路径），Loader 服务 active。
- 构建与文件校验：本地 `npm run build -w apps/decky` 复现 `dist/index.js` SHA-256 `80e7cdb5d695e55022702a88c642b9198ae544f012387e1394664a2c849b8ea7`、map `21b851e923e64ea5978854d59a6c8537e133abca80c798275065199989368f95`，与设备 HTTP `/plugins/Steam 补丁管理/dist/index.js` 实际响应（200，97768 字节）及正式文件一致；`main.py` `adb6b2b3ce41a5a05d7d96161999599d9f354befef7b7ab8b5dff0e017fbe4d2`、`backend/tasks.py` `a93b60d48ed2fd7aba241370d714908337c19d50415386ef121ab6fc477c7a18` 与本地一致。部署清单 39 项逐文件 SHA-256 全部匹配（运行期仅多出 `backend/__pycache__`）。后端进程于 16:09:32 启动，晚于文件写入 16:09:31，确认运行的是新后端。
- 自动化检查：前端 51 项测试、Decky 类型检查与 Rollup 构建通过；后端 `TaskTests` 7 项、`AcquisitionProgressTests` 4 项、`test_task_history` 2 项、`test_install_pipeline` 5 项通过。`ControlledBridgeTests` 3 项在 Windows Python 3.10 下挂起，经 HEAD 同目录对照确认与本轮改动无关（环境问题），未记为通过。
- 真实 RPC：`list_tasks` 返回 3 条终态、无活动任务；`list_transactions` 返回 8 条；`scan(false)` 返回 40 个库实例、generation 3，含 1 条 SD 卡库 `library_unavailable` 诊断（设备路径不存在），来源扫描继续进行。
- 真实页面：QAM 的 Decky 标签渲染标题“Steam 补丁管理”、搜索框、筛选“有补丁”和 6 个游戏（长名省略号截断），底部“全局设置 › / 重新扫描”；全屏游戏补丁管理页渲染侧栏“Steam 补丁管理 / 妃神会秘史”四栏目与补丁详情、安装入口。均为当前构建的实机画面。
- 合成任务渲染（明确标注为合成，未触发真实下载或写入）：通过当前 `DeckyBackend.call('loader/call_plugin_method', …)` 拦截 `list_tasks` / `task_status` / `cancel_task` 注入运行中任务（未调用 `start_patch_install_task`）。进入补丁管理页后进度面板出现：标题“安装补丁”、阶段“下载补丁 · 25%”、原生进度条、格式化后的速度 / 大小 / 文件 / 用时与当前文件，正文“关闭将取消安装，并恢复本次修改的文件。”；不存在旧的“取消安装”“关闭进度窗口”按钮与“关闭后安装仍会继续”文案。原生关闭（Esc）触发 `cancel_task`（计数 1→2），未自动跳过。任务转终态后切换为独立结果面板：仅“安装结果 / 已安全取消。”，无进度条、统计或内容区按钮，底部原生提示“B 关闭”。
- 观察项：进度面板右下角原生提示显示默认“B 返回”，未显示设计中的取消专用文案（结果面板显示“B 关闭”）；取消后果已由正文明确说明。该标签与焦点行为需实体手柄复核。
- 已恢复合成拦截并复核：无合成残留、无活动任务、真实 `list_tasks` 与部署前一致、插件实例 1；已关闭本轮菜单并把界面恢复到库首页。进入时页面被一个上一轮安装遗留的“重新安装 Steam 补丁管理”原生确认弹窗遮挡，已用“取消”关闭（未触发重装）。远端暂存目录 `foreground-install-deck-20261007-160602` 在确认本地备份（`rollback.zip` `ac1ca699a44cfbba996654a0ccf3f75957fb533cfae9fa4238d2885bd850608c`、`new-plugin.zip` `27c42406587741b3cb92a0cf200c1eb67d65c17b7b84d1cdf0814858cd5d5c57`）与证据均已取回后清理；本地保留备份与证据于 `.artifacts/foreground-install-deck-20261007-160602/`、`.artifacts/foreground-install-deck-20261007-1617/`。
- 待验收：真实下载 / 安装过程中按原生关闭键取消并安全恢复、失败 / 需恢复文案、安装完成后自动清理下载缓存；实体手柄 A/B 焦点遍历、长名称滚动与顶部 / 底部按键提示。未提交 Git。
