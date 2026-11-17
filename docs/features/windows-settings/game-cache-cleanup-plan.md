# 缓存管理新增“本地游戏数据缓存”清理方案

状态：**已实现（Core / Protocol / CLI / Windows Flutter / Decky 定向自动化测试与静态检查通过；双端实机待验收）**。隶属 [Windows 设置入口与缓存管理设计](design.md)，施工与验证结果记录于 [progress.md](progress.md)。

## 1. 背景与目标

`feature/unified-source-index-detail` 分支已将订阅源与游戏缓存数据结构统一重构为 `Index -> Detail + Match` 模型（存储于 `<state_dir>/sources/rust-v1/`）。当前 Windows 设置页“缓存管理”与 Decky 全局设置“缓存、备份与更新”页签均仅覆盖安装器侧的“下载缓存”（`cache_status` / `cleanup_cache`），缺少对本地缓存的游戏与订阅源数据的查看和清理入口。

本方案在共享 Rust Core / Protocol 层提供统一的 `game_cache_status` 与 `cleanup_game_cache` 能力，并在 **Windows 客户端** 与 **Decky 插件** 的对应设置页签中同步新增 **“本地游戏数据缓存”** 清理功能，同时严格保留用户配置与已安装数据。

## 2. 清理范围与保留边界

### 2.1 存储路径边界（`<state_dir>/sources/rust-v1/`）

依据 [store.rs](../../../crates/core/src/sources/store.rs) 中 `StorePaths` 的实际目录布局划分：

| 类别 | 目录 / 文件 | 处理方式 | 说明 |
| --- | --- | --- | --- |
| 游戏与订阅缓存 | `index/`、`index-progress/`、`details/`、`matches/`、`discovery/`、`discovery-index/`、`status/`、`cache/` | **统计并清理** | 订阅源索引快照与进度、按需抓取的 `DetailRecord`、按 AppID 缓存的 `GameMatchRecord`、补查缓存、`SourceStatus` 及遗留 v1 `cache/` |
| 用户与系统配置 | `settings.json`、`settings.lock`、`schema.json` | **严格保留** | 来源启用状态、自定义名称、自动更新间隔、单游戏订阅关系、自定义导入 Lua 脚本与规则、存储 schema 标记 |

其他状态目录（`installer/rust-v1/` 下载与恢复材料、`source-browser/` 浏览器会话、`updates/` 应用更新状态）不属于本功能范围，保持原样。

### 2.2 符号链接与路径安全

- 统计与清理仅遍历上述 8 个白名单子目录，不接受调用方传入路径。
- 遍历前及逐项删除时使用 `fs::symlink_metadata` 检查，遇到符号链接/目录联接时跳过并记录诊断/失败项，不跟随链接删除外部文件。
- 目录不存在时视为 `0 B / 0 项`，不额外创建空目录。

## 3. 核心、协议与 CLI 设计

### 3.1 复用现有返回结构，不新增 DTO

在 [protocol/src/lib.rs](../../../crates/protocol/src/lib.rs) 新增两个方法，直接复用现有的 `CacheInventory`（`{ total_bytes, entries, diagnostics }`）和 `CleanupResult`（`{ status, performed, removed_bytes, failures }`）：

| 方法名 | 参数 | 返回类型 | 说明 |
| --- | --- | --- | --- |
| `game_cache_status` | `{ host }`（需 `state_dir`） | `CacheInventory` | 只读统计 `sources/rust-v1/` 下 8 个缓存子目录的总字节数与文件数 |
| `cleanup_game_cache` | `{ host }`（需 `state_dir`） | `CleanupResult` | 获取 `settings.lock` 后清理 8 个缓存子目录中的文件与空子目录，保留 `settings.json` 与 `schema.json` |

不修改原有 `cache_status` / `cleanup_cache` 的语义与返回结构，避免影响 Decky 端与现有安装缓存契约。

### 3.2 Rust Core 与 CLI 落点

1. **Core（[store.rs](../../../crates/core/src/sources/store.rs) / [sources/mod.rs](../../../crates/core/src/sources/mod.rs)）**：
   - 抽取可清理子目录列表常量（`index`、`index-progress`、`details`、`matches`、`discovery`、`discovery-index`、`status`、`cache`）。
   - 提供只读统计函数（当 `sources/rust-v1` 不存在时不创建目录，直接返回 `0`）与带 `settings.lock` 的清理函数，返回 `CacheInventory` / `CleanupResult`。
2. **Protocol & Contracts（[protocol/src/lib.rs](../../../crates/protocol/src/lib.rs)、`packages/contracts/`、`apps/windows-flutter/assets/protocol-v5.schema.json`）**：
   - 注册 `game_cache_status` 与 `cleanup_game_cache` 到 `METHODS` 与 `result_schema_name`。
   - 同步更新 `protocol-v5.schema.json`、生成物校验与 Flutter `protocol_models.dart`。
3. **CLI（[runtime.rs](../../../crates/cli/src/runtime.rs)）**：
   - 在请求分发中接入 `game_cache_status` 与 `cleanup_game_cache`，校验 `state_dir` 存在后调用 Core 能力。

## 4. Windows 客户端界面与状态设计

### 4.1 缓存管理页签（[cache_page.dart](../../../apps/windows-flutter/lib/pages/cache_page.dart)）

保留现有 **“下载与临时缓存”** 卡片，在其下方新增 **“本地游戏数据缓存”** 卡片，布局与交互风格保持一致：

1. **展示与统计**：
   - 进入缓存管理页签时，除 `cache_status` 外同步调用 `game_cache_status`。
   - 显示“缓存占用”（总大小与条目数；`diagnostics` 非空时提示“部分文件无法统计”；为空时显示 `0 B` 与“暂无需要清理的缓存”；读取失败显示“占用读取失败”与详情）。
   - 提供“刷新占用”按钮。
2. **确认与清理**：
   - 点击“清理游戏缓存”弹出一次确认弹窗，明确说明：
     - 将删除本地缓存的订阅源索引、游戏详情、匹配结果与更新状态记录；清理后需要重新更新订阅源或匹配游戏。
     - 游戏文件、已安装补丁、来源启用与自动更新设置、游戏订阅关系及自定义导入脚本会保留。
   - 确认后调用 `cleanup_game_cache`，按钮显示“正在清理…”并禁用重复点击，切换页签或返回游戏不取消任务。
   - 完成后展示 `completed` / `partial` / `failed` 状态、实际释放大小与失败详情（可展开查看并手动重试），并重新统计占用。
3. **并发保护**：
   - 当 `AppModel` 中存在运行中的来源任务（`update_source` / `match_games` / `rematch_game`）时，禁用“清理游戏缓存”按钮并提示“有订阅源更新或游戏匹配任务正在进行，请任务结束后重试。”
4. **内存态同步**：
   - `cleanup_game_cache` 完成后，无论结果成功或部分成功，触发 `AppModel.refresh()` 重新拉取 `project_instances` 与 `list_sources`，使游戏列表、补丁候选与来源状态（`not_refreshed`）立即与磁盘一致，不在内存中残留已清理的过期候选。
5. **底部“清理时会保留”说明调整**：
   - 将底部保留说明更新为区分两类缓存的共有保留项（已安装补丁与游戏文件、全局来源与游戏订阅配置及自定义导入脚本、未完成安装恢复材料），避免与新增的“清理游戏缓存”产生文案矛盾。

### 4.2 状态模型（[cache_model.dart](../../../apps/windows-flutter/lib/models/cache_model.dart) & [app_model.dart](../../../apps/windows-flutter/lib/models/app_model.dart)）

- 在 `CacheModel` 中为“本地游戏数据缓存”维护独立的统计/清理状态（`gameInventory`、`gameResult`、`gameLoading`、`gameCleaning`、`gameInventoryError`、`gameCleanupError`）或通过统一的子状态对象管理，避免两张卡片互相覆盖结果与错误。
- `CacheModel` 清理游戏缓存成功后通过回调通知 `AppModel.refresh()`。

## 5. Decky 插件端设计（`apps/decky/`）

Decky 端与 Windows 端共享同一套 `game_cache_status` / `cleanup_game_cache` 核心协议，在全局设置的 **“缓存、备份与更新”** 页签（`GlobalCache`）中同步新增“本地游戏数据缓存”区块：

### 5.1 Python 后端与桥接（[bridge.py](../../../apps/decky/backend/bridge.py) & [main.py](../../../apps/decky/main.py)）

1. **[bridge.py](../../../apps/decky/backend/bridge.py)**：新增 `game_cache_status()`（调用 `self._call("game_cache_status", {})`）与 `cleanup_game_cache()`（调用 `self._call("cleanup_game_cache", {})`）。
2. **[main.py](../../../apps/decky/main.py)**：
   - 将 `game_cache_status` 加入只读方法集合，直接转发 `self.bridge.game_cache_status()`。
   - 新增 `cleanup_game_cache(self)`：先执行 `self._require_idle()`（并拒绝存在运行中来源任务时的并发清理），再调用 `self.bridge.cleanup_game_cache()` 并返回 `CleanupResult`。

### 5.2 前端界面与会话同步（[index.tsx](../../../apps/decky/src/index.tsx)、[cache.ts](../../../apps/decky/src/cache.ts)、[session.ts](../../../apps/decky/src/session.ts)）

1. **Callable 注册与反馈文案**：
   - 在 [index.tsx](../../../apps/decky/src/index.tsx) 注册 `game_cache_status` 与 `cleanup_game_cache` callable。
   - 在 [cache.ts](../../../apps/decky/src/cache.ts) 补充游戏数据缓存清理结果的提示文案（如区分 `completed` / `partial` / `failed`，与现有 `cacheClearNote` 风格一致）。
2. **`GlobalCache` 页签布局**：
   - 在“下载缓存”与“安装备份”之间新增 `<PanelSection title="本地游戏数据缓存">`：
     - 进入页签时同步调用 `gameCacheStatus()` 统计当前占用（复用 `cacheSummary` 展示大小与条目数，并在 `diagnostics` 非空时展示警告）。
     - 说明文字明确清理对象（订阅源索引、游戏详情、匹配结果与更新状态记录）及保留项（游戏文件、已安装补丁、来源与游戏订阅配置及自定义导入脚本）。
     - 提供 `<Action>` 按钮“清理游戏缓存”；当 `busy` 或 `Object.values(state.sourceTasks).some(task => !sourceTaskTerminal(task.state))` 为真时禁用按钮并提示任务结束后重试。
3. **清理后状态刷新**：
   - 清理完成后重新拉取 `gameCacheStatus()` 更新占用展示，并调用 `session` 刷新游戏列表与来源列表快照，使游戏管理页与“管理订阅源”页签立即与磁盘一致。

## 6. 过度设计审阅与约束

- **不引入通用清理框架或分项勾选**：仅提供“下载与临时缓存”与“本地游戏数据缓存”两个明确卡片/区块，不做按游戏/按来源粒度删除、TTL 自动过期或定时清理配置。
- **不新增协议返回类型**：直接复用 `CacheInventory` 与 `CleanupResult`。
- **不改变现有下载缓存逻辑**：`CacheService` 及其既有测试保持原样。

## 7. 分阶段验证计划

1. **Core & CLI 单测**：覆盖 `sources/rust-v1/` 8 个缓存目录的统计、清理、`settings.json` / `schema.json` / 导入脚本保留、空状态目录不报错及符号链接防护。
2. **Protocol & Contracts 校验**：核对 `protocol-v5.schema.json`、生成物一致性及 Flutter `protocol_models.dart`。
3. **Windows Flutter 定向测试与静态检查**：扩展 `cache_page_test` / `settings_page_test`，覆盖占用展示、确认弹窗、清理完成刷新、来源任务运行中禁用、部分失败与错误保留；运行 `dart format` 与 `flutter analyze`。
4. **Decky 后端与前端单测**：扩展 `apps/decky/backend/test_cache_cleanup.py` 与 `apps/decky/tests/`，覆盖 `bridge.py` / `main.py` 转发、空闲校验与 `GlobalCache` 展示/清理/刷新逻辑。
5. **实机验收（待执行）**：Windows 原生窗口与 Steam Deck 游戏模式下，在隔离状态目录分别验证两类缓存互不干扰及清理后主界面与来源状态同步复位。
