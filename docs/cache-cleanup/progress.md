# 下载缓存清理施工进度

关联设计：[design.md](design.md)。本文件记录本轮施工的完成项、验证方式与遗留问题。

## 任务清单

| 项 | 状态 | 说明 |
| --- | --- | --- |
| 设计文档归类与施工记录 | 已完成 | 原 `docs/cache-cleanup-design.md` 移入 `docs/cache-cleanup/design.md`，新增本进度文件 |
| `backend/plugin_settings.py` | 已完成 | 自动清理开关持久化，默认开启，损坏回退默认（原名 `settings.py`，因运行时同名冲突改名） |
| `backend/caches.py` | 已完成 | 缓存枚举 / 统计 / 清理（publisher + core） |
| `backend/publisher.py` | 已完成 | `cache_files` / `_unlink_within` 辅助、`cache_dir` / `clear_cache` / `caches` |
| `main.py` 接线 | 已完成 | 计划元数据、提交后按开关清理、`get_settings` / `set_auto_cleanup` / `cache_status` / `clear_cache` |
| `src/cache.ts` + `src/index.tsx` | 已完成 | 纯逻辑模块与全局设置“下载缓存”分区（开关 + 占用 + 清理确认） |
| 后端测试 | 已完成 | `test_settings`(4) / `test_caches`(5) / `test_cache_cleanup`(6) / `test_publisher` 缓存(2) |
| 前端测试 | 已完成 | `tests/cache.test.ts`(3) |
| 文档同步 | 已完成 | privacy / client-guide / install-design / README / decky-ux design / progress |
| 构建与检查 | 已完成 | `npm run typecheck -w apps/decky`、`npm run build:decky`、后端 `unittest` |
| 真实 Deck 部署 | 已完成 | 按哈希部署并要求重载；首次 reload 暴露 `decky_loader.settings` 同名冲突，改为 `plugin_settings.py` 后正常加载 |
| 真实 Deck 后端验收 | 已完成 | 经 Decky 路由实时调用 `get_settings` / `cache_status` / `clear_cache` / `set_auto_cleanup` 均通过，用户文件被保留 |
| 真实 Deck UI 与安装验收 | 待手柄 | 前端 bundle 已含“下载缓存”分区文案；开关/占用/弹窗的实机渲染与真实安装后自动清理待手柄确认 |
| 事务暂存/备份处理 | 已完成 | 提交后自动删除该事务 `.stage`；新增 jobs 占用统计与手动“清理事务备份与暂存” |
| 独立侧栏“下载缓存”入口 | 已完成 | 全局设置新增与“管理订阅源”并列的侧栏页 |

## 日志

### 2026-10-04

- 建立专门文件夹 `docs/cache-cleanup/`，移入设计文档并新建本进度文件。
- 确认实现策略：后端持有开关；删除按“推导缓存路径 + `no_links` + 路径归属”判定，不依赖 `acquisition`；不触碰 `~/Downloads`、事务备份、暂存与订阅源缓存。
- 实现后端：`plugin_settings.py`（默认开启、原子写入，部署时因 `decky_loader.settings` 同名冲突由 `settings.py` 改名）、`caches.py`（`scan/status/clear/clear_core`）、`publisher.py` 缓存辅助、`main.py` 接线。
- 实现前端：`src/cache.ts`（`formatSize/cacheSummary/cacheClearNote/autoCleanupNote`）、全局设置“下载缓存”分区（`ToggleField` + 占用 + `ConfirmModal` 清理）。`index.tsx` 的 `size` 抽取为 `formatSize` 复用。
- 验证：后端 62 项测试中仅 3 项因缺少未编译的核心二进制（`target/debug/steam-patch.exe`）报环境错误，另有 1 项符号链接用例在 Windows 跳过；新增用例全部通过。前端类型检查、Rollup 构建、`node --test`（22 项，含新增 3 项）通过。
- 已同步 `docs/privacy.md`、`docs/client-guide.md`、`docs/install-design.md`、`apps/decky/README.md`、`docs/decky-ux/design.md`、`docs/progress.md`。
- 部署到真实 Deck（`deck@192.168.31.186`，插件目录 `/home/deck/homebrew/plugins/steam-patch-manager`）。先备份到 `/home/deck/steam-patch-validation/cache-cleanup-20261004/backup`，再按 SHA-256 校验上传 `main.py`、`backend/{plugin_settings,caches,publisher}.py`、`dist/index.js(.map)`、`README.md`，本地与远端哈希一致。
- 首次重载后插件启动失败，日志显示 `ImportError: cannot import name 'Settings' from 'decky_loader.settings'`：Decky 运行时已有顶层 `settings` 模块，劫持了 `from settings import Settings`。该冲突只在 Decky 沙箱出现，本地测试无法发现。将模块改名为 `plugin_settings.py`、更新导入与测试后重新部署，删除远端旧 `settings.py` 与 `__pycache__`，再次 reload 后进程正常、日志无报错。
- 通过 Steam CEF 调试端口（SSH 隧道 `18080→8080`）的 DevTools 协议，经 `__DECKY_SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED_deckyLoaderAPIInit.connect(1, 'Steam 补丁管理')` 实时调用插件方法：`get_settings` 返回 `auto_cleanup: true` 且缓存为空；构造 publisher 缓存 109B（`archive.zip` 100 + `download.json` 2 + `download-x.tmp` 7）、core 缓存 210B（`<sha>.zip` 200 + `*.partial` 10）后 `cache_status` 精确返回 `total 319 / 5 文件`；`clear_cache` 返回 `removed_bytes: 319` 且清零，磁盘核对仅保留非摘要命名的 `user-keep.zip`；`set_auto_cleanup` 往返 `false→true` 并落盘 `settings.json`。验收用文件与临时 release 目录已清理。
- 前端 bundle 复核：远端 `dist/index.js` 含转义后的“安装成功后自动删除下载缓存”“当前占用”“清理下载缓存”等文案，确认新版 UI 已部署。
- 用户反馈 `.stage` / `.backup` 未被删除且找不到缓存面板。排查确认：这些是核心事务文件而非下载缓存，设备上 `publisher/.../jobs/` 共 7.7G（stage ≈3.85G + backup ≈3.85G），而下载缓存已空；面板确实存在但位于全局“管理订阅源”页底部，且每个游戏页的同类标签页没有。
- 本轮增强：安装提交后自动删除该事务 `.stage`；新增全局独立侧栏页“下载缓存”（与“管理订阅源”并列），内含“下载缓存”和“事务备份与暂存”两个分区；`cache_status` 增加 `jobs.stage_bytes/backup_bytes`；新增 `clear_transaction_backups` 手动清理 `committed`/`uninstalled`/`rolled_back` 事务的暂存与备份（保留进行中 / 需恢复事务），确认文案明确“删除备份后无法卸载、只能重新安装”。
- 按用户确认**立即执行真实清理**：经 Decky 路由调用 `clear_transaction_backups`，状态目录从 **7.7G 降到 344K**，`publisher/.../jobs/` 文件全部清除（`.backup` 已删，该已安装补丁不再可卸载）。
- 按用户要求新增**备份保留开关 `keep_backups`，默认关闭**：关闭时安装提交后同时删除 `.backup`（仅保留 `.stage` 的自动删除逻辑说明见下），开启时保留备份以支持卸载。`list_transactions` 为每条事务增加 `backups` 布尔；无备份时界面隐藏“卸载”并只显示“重新安装”，后端 `uninstall` 以 `uninstall_backups_missing` 拒绝。设置页“事务备份与暂存”分区新增该开关。
- 重新部署（备份 `cache-cleanup-20261004-3`）并重载。实机 `get_settings` 返回 `keep_backups: false`；`set_keep_backups` 往返 `true→false` 正常；`list_transactions` 对既有事务返回 `backups: false`；对无备份事务调用 `uninstall` 得到 `uninstall_backups_missing`；`dist/index.js` 含“保留原文件备份”“不保留原文件备份”“无法卸载，只能重新安装”等文案。后端 35 项测试、类型检查与构建通过。
- 用户反馈“显示无法重新安装”。定位为核心 `install::prepare` 的 `ensure_no_active` 因旧 `committed` 事务拒绝新准备（`managed_install_exists_or_needs_recovery`），且默认关闭备份后该事务永远无法卸载。因本机与 Deck 都没有 Rust 工具链、无法重建核心，采用 decky 侧桥接：`scan` 不再把无 `.backup` 的 committed 事务计为 `publisher_existing_transaction`（有备份仍阻止）；`prepare_install` 在准备前对“无任何备份”的状态目录调用 `caches.forget_transactions`，删除核心 `journal.sqlite*` 与 `jobs/` 但保留配方/签名/密钥。
- 重新部署（备份 `cache-cleanup-20261004-4`）并重载。实机 `scan` 对该补丁返回 `blockers: []`；用真实 publisher 状态目录验证 `caches.forget_transactions` 只删 journal/jobs、保留 `catalog.json`；清理后 `list_transactions` 为空、检测为 `installed_external`，界面具备“重新安装”入口。后端 72 项测试、类型检查通过。遗留：长期应由核心提供“丢弃已完成且无备份事务”的方法。

## 待实机验证

- 手柄打开全局“下载缓存”侧栏页，确认两个分区、两个开关、占用与确认弹窗的焦点与文案。
- 开启“保留原文件备份”后安装一个真实补丁：确认 `.backup` 保留、可卸载；关闭后安装：确认 `.stage`/`.backup` 均被删除、界面只提供重新安装、卸载被拒绝。
- 默认（无备份）模式下点击“重新安装”，确认能重新下载、生成计划并提交覆盖安装；记录被清理后重新建立。
- 手动清理后再次“自动下载并准备安装”可重新下载并复用流程正常。

## 已知取舍与遗留

- `.backup` 是全量原件，卸载还原必须依赖它；真正的“增量备份”是核心层改造，暂未实现（详见 design.md）。
- 默认关闭备份保留意味着默认无法卸载；这是产品决策，用户可在“下载缓存”页开启。
- 自动清理所需信息保存在内存 `_plan_metadata`；若插件重启后再安装“重启前准备但未安装”的计划，可能跳过自动清理，缓存留待手动清理。正常 UI 流程（准备后立即安装）不受影响。
- 删除前默认不做 SHA-256 二次校验，仅做路径 + 链接校验，以避免刚写完又重读数 GB；依据是文件位于 0700 状态目录且路径由内部推导。
- 订阅源查询缓存不在清理范围。
