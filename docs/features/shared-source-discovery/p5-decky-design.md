# P5：Decky 切换到共享 Rust 来源引擎（集成契约）

设计：[design.md](design.md)。P0/P1 规范：[p0-p1-rust-design.md](p0-p1-rust-design.md)。施工记录：[progress.md](progress.md)。

状态：**P5 实现中**。日期：2026-10-08（Asia/Shanghai）。

本文件冻结 P5 的 Rust 增补方法与 Python `SourceService` 适配契约。消费方（`publisher.py`/`exe_profiles.py`/`exe_generic.py`/`exe_installation.py`）保持**零改动**，靠鸭子类型适配。

## 1. Rust CLI 增补（冻结）

在 `crates/cli/src/runtime.rs::METHODS` 增加方法；`Params` 增加可选 `appid: Option<u32>`。

| 方法 | 参数 | 返回 |
| --- | --- | --- |
| `list_source_matches` | `instance_id` **或** `appid` | `{ "matches": [<match>] }`（`registry.matches(game, include_disabled=false)` 的结果） |
| `get_source_state` | — | `{ "settings_revision": u64, "import_report": <ImportReport>|null }` |
| `get_source_settings` | — | `{ "settings_revision": u64, "enabled": {id:bool}, "added": [rule], "games": {"<appid>": {source_id:bool}} }` |

同时对既有方法/输出做**附加**改动（不破坏现有字段）：

- `scan`：每个 install 行新增 `matches: [<match>]`（只读缓存 + 规则匹配，不联网）。
- `list_sources`：每项新增 `origin: "builtin" | "added"`。
- `set_game_source` / `list_game_sources` / `list_source_matches`：接受 `appid` 替代（或补充）`instance_id`；解析优先级 instance_id → appid。
- `SourceStore`：首次 `open` 的 `ImportReport` 持久化到 `<state>/sources/rust-v1/import-report.json`（有界），供 `get_source_state` 读取；不改变导入幂等语义。

`ImportReport` = `{ imported_settings: bool, imported_cache: usize, imported_discovery: usize, imported_index: usize, diagnostics: [Diagnostic], revision: u64 }`。

## 2. Python 薄适配层（冻结）

新模块 `apps/decky/backend/source_service.py`：`SourceService`，鸭子类型等价 `SourceRegistry`。
新模块 `apps/decky/backend/source_engine.py`：`open_source_engine(...)`、`engine-switch.json` 读写、导入诊断聚合、Python↔Rust 设置摘要。
回退引擎：`sources.py::SourceRegistry` 保持不变，可由设置/环境选择。

> 2026-10-11 P6 收口：上列 `source_engine.py`、`engine-switch.json` 与 Python 回退引擎均为当时的双引擎契约，已被 [跨平台安装器迁移设计](../cross-platform-installer-migration/design.md) §7.3 第 2 行取代：`source_engine.py` 已删除，只保留单一 Rust 来源引擎。原文保留为 P5 契约记录。

### 2.1 属性

| 属性 | 来源 |
| --- | --- |
| `.state_dir` | 本地 Path |
| `.engine` | `"rust"` |
| `.settings_revision` | 快照 int（来自 `scan`/写响应） |
| `.diagnostics` | 快照 list（导入 + 最近刷新/诊断） |
| `._rules` | 快照：`list_sources` 的规则 dict（flatten） |
| `.builtins` | 快照：`origin == "builtin"` 的规则 |

### 2.2 方法

| 方法 | 类型 | 映射 |
| --- | --- | --- |
| `initialize()` | async | `get_capabilities` + `list_sources` + `get_source_state`（首启导入诊断）；能力不足→回退 Python |
| `prime(scan_report)` | sync | 由 Rust `scan` 报告填充 `_summaries`/逐实例 `sources`/`discovery`/`matches`/`settings_revision`/`diagnostics` |
| `list_sources()` | sync 读 | 快照 |
| `list_game_sources(game)` | sync 读 | 快照（逐行 `sources`）；冷时 `call_sync("list_game_sources", appid=…)` |
| `matches(game, include_disabled=False)` | sync 读 | 快照（逐行 `matches`）；冷时 `call_sync("list_source_matches", appid=…)` |
| `cached_candidates(game)` | sync 读 | 快照（逐行 `discovery.candidates`） |
| `cached_discovery(game)` | sync 读 | 快照（逐行 `discovery`） |
| `game_subscribed(appid, source_id)` | sync 读 | 由逐行 `sources[].subscribed`，缺省 `True` |
| `source_for_release(release_id)` | sync 读 | 本地逻辑（`_rules` 前缀匹配，等价原算法） |
| `validate_download(source_id, url)` | sync 读 | 本地逻辑（`_rules` 的 `download_hosts` + HTTPS/.zip 校验；安装层再复核） |
| `set_enabled(source_id, enabled)` | sync 写 | `call_sync("set_source_enabled")` |
| `add_source(rule)` | sync 写 | `call_sync("add_source")` |
| `set_game_source(appid, source_id, subscribed)` | sync 写 | `call_sync("set_game_source", appid=…)` |
| `refresh(force=False, on_event=None, cancel=None)` | async | `_call("refresh_sources", controlled)`，映射 `SourceStatus[]`→diagnostics |
| `scan_sources(force, on_event, cancel)` | async | `_call("scan_sources", controlled)` |
| `discover_game(game, name_hints, preferred_language, force, generation, is_current=None)` | async | `_call("discover_game", controlled)` |
| `close()` | async | 置取消事件，无持久进程 |

### 2.3 sync/async 策略

混合：**快照读 + 本地写用阻塞一次性 `serve`（`CoreBridge.call_sync`）+ 网络用异步流**。消费方保持同步，不做 async 化。冷读兜底 `call_sync`。快照新鲜度由 `prime()` 保证（`_read_report` 在 `_project_row`/`prepare`/`resolve` 前 `prime`）。

### 2.4 `bridge.py` 改动

- `METHODS` 增补全部来源方法。
- `CONTROLLED_METHODS` 增补 `discover_game`、`refresh_sources`、`scan_sources`（走 `serve-task`）。
- `SOURCE_EVENTS = {"source_progress","source_update"}`：`_call` 新增 `on_event`/`cancel`/`timeout`；事件按 `request_id` 匹配、`sequence` 单调；事件不作为最终响应。
- 新增 `async def call_sync(self, method, params)`：`serve` 一次性调用，返回 `result`。
- 取消沿用 `cancel\n`/EOF。

## 3. 导入与引擎切换

- **导入**：Rust 首次 `open` 隐式执行（`rust-v1/schema.json` 幂等）。`get_source_state.import_report` 供插件上报诊断；不改 Python 旧文件。
- **引擎选择**：插件设置 `source_engine`（默认 `"rust"`，可 `"python"`），环境覆盖 `STEAM_PATCH_SOURCE_ENGINE`。`_initialize` 读取；Rust 不可用（能力缺失/`unsupported_method`）→ 记录并回退 `SourceRegistry`，旧 `sources/` 状态不动。
- **检查点**：每次切换由 `source_engine.py` 原子写 `<state>/sources/rust-v1/engine-switch.json`（schema 见 p0-p1 §4.6：`active_engine`/`switch_seq`/`rust{revision,settings_hash}`/`python{settings_hash,captured_at}`/`last_common`）；回退前用 `get_source_settings` 导出 Rust 当前设置并映射旧格式，保留迁移后新选择。

> 2026-10-11 P6 收口：本节引擎选择设置/环境变量、`engine-switch.json` 检查点与 Rust→Python 设置导出/回退已被 [跨平台安装器迁移设计](../cross-platform-installer-migration/design.md) §7.3 第 2 行取代：无引擎选择、无切换/共同快照、无旧引擎回退，直接初始化共享来源服务。原文保留为 P5 契约记录。

## 4. scan / scan_status 投影

- `scan` 先渲染缓存（含逐行 `discovery`/`sources`/`matches`），随后 `ScanSession.run()` 依次 `refresh_sources`→`scan_sources`，用 `source_progress`/`source_update` 事件喂 `scan_progress` 与逐行 `discovery`。
- 并发/预算/去重全部交给 Rust；移除 Python `Semaphore(3)`/`wait_for(90)`。
- `scan_status(generation)` 语义不变（generation/revision 校验 + `scan_stale`）。
- `discover_game` RPC 内部改由适配层调用；`is_current` 门与 `status="stale"` 投影不变。

## 5. 消费者与 RPC 兼容

- `publisher.py`/`exe_profiles.py`/`exe_generic.py`/`exe_installation.py` **零改动**（依赖 `_rules`/`builtins`/`game_subscribed`/`cached_candidates`/`matches`/`validate_download` 的同形接口）。
- `main.py` RPC 名称与返回形状不变（`list_sources`→数组；`set_source_enabled`/`add_source`→`list_sources()`；`set_game_source`→`{"appid":…}`；`scan_status`→快照）。
- 已知语义微差：added 规则遮蔽 builtin id 时，`builtins` 以 origin 判定，`exe_profiles.rule` 可能返回 `exe_source_disabled`（原 `exe_source_changed`），两者均拒绝，记入 `progress.md`。

## 6. 测试

- 保持：`test_exe_profiles.py`/`test_exe_generic.py`/`test_exe_installation.py`/`test_scan_projection.py`/`test_acquisition.py`/`test_discovery_rpc.py`（鸭子类型契约）。
- 回退保留：`test_sources.py`/`test_discovery_registry.py`/`test_discovery.py`（Python 引擎回退覆盖）。
- 重指向：`test_publisher.py`（用 fake service）、`test_scan_session.py`（事件驱动）。
- 新增：`test_source_service.py`、`test_source_engine.py`、`test_source_import.py`、扩展 `test_bridge.py`。
- 委托给 Rust 的行为（规则校验/缓存 TTL/分层提交/候选校验/分页/revision/身份合并/URL 白名单）不再在 Python 重复，由 Rust 矩阵覆盖。

> 2026-10-11 P6 收口：上列“回退保留”的 Python 引擎测试（`test_sources.py`/`test_discovery_registry.py`/`test_discovery.py`）与 `test_source_engine.py` 随双引擎契约一并移除（[跨平台安装器迁移设计](../cross-platform-installer-migration/design.md) §7.3 第 2、6 行）；仅删除维护兼容的测试。原文保留为 P5 契约记录。

## 7. 落地顺序

1. Rust 增补（本文件 §1）。2. `bridge.py`。3. `source_service.py`。4. `source_engine.py`。5. `plugin_settings.py`。6. `main.py`/`scan_session.py`。7. 测试。8. 文档。

> 2026-10-11 P6 收口：第 4、5 步的 `source_engine.py` 与 `plugin_settings` 引擎选择项已随双引擎契约退出（[跨平台安装器迁移设计](../cross-platform-installer-migration/design.md) §7.3 第 2 行）；本文件保留 P5 当时的落地顺序记录。
