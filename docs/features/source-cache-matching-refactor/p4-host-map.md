# P4 双端宿主改动测绘（Decky / Windows）

- 状态：P4 只读测绘文档，供实现分工使用；未修改生产代码、未提交 Git、未部署设备。
- 上级文档：[design.md](design.md) · [P0/P1 契约](p1-contract.md)（§9 协议 v5、§10 调度语义、§11 删除范围） · [P0 基线](p0-baseline.md)。
- 分支：`feature/source-cache-matching-refactor`。测绘时 protocol/CLI/双端宿主仍为 **v4**；`crates/core/src/sources/{model,store,index,match,schedule}.rs` 正被 P1 并行修改，该目录行号可能漂移；其余文件行号取当前工作区。
- 本轮结论来自静态代码核对与既有 P0 实测；未运行双端应用，未做真实网络或实机验证。

标记约定：**【事实】** 为当前代码可核对的行为；**【建议】** 为实现选择，需实现者确认。

## 0. 关键结论（先读）

1. **【事实】一次 Decky 整库扫描固定产生 3 次 `bridge.scan()`**：初始读取（`main.py:97`，经 `:110`）+ `refresh_sources` 后整库重投影（`main.py:85`，经 `:122`）+ `scan_sources` 后整库重投影（`main.py:85`，经 `:128`；调用链见 `p0-baseline.md` §5.1）。单游戏 `start_game_discovery` 还会先关闭旧会话，再产生 2 次整库 `scan`（`main.py:169-173`、`scan_session.py:67-68`、`main.py:74`）。
2. **【事实】Windows 一次整库来源扫描产生 N+2 次 `core_call("scan")`**（N = 安装实例数）：每个 `source_update` 事件 1 次、任务结束 1 次、初始 1 次（`apps/windows/src/main.tsx:199,263,330`；`p0-baseline.md` §5.5）。
3. **【事实】两端都用整库 `scan` 取单行**：Decky `_project_row`（`main.py:72-80`）与 `_refresh_rows`（`main.py:82-94`），Windows 事件/任务后 `setScan(fresh)`（`main.tsx:199-206,263-266`）。P4/P5 必须改为 `project_instances` / 任务返回行。
4. **【事实】任务槽位互相取消/阻塞**：Decky 单一 `_scan_session`（`main.py:64,107-109,169-170`），安装任务启动时关闭扫描会话（`main.py:300-304`）；Windows 单一 `SourceScans` 槽位（`src-tauri/src/main.rs:183-192,570-572,1362-1365`）并把来源任务与安装 `Jobs` 分开。§11 要求删除“共用扫描会话造成的相互取消”。
5. **【事实】协议 v5 是整链切换**：`PROTOCOL_VERSION`（`crates/protocol/src/lib.rs:21`）、方法表（`:66-147`）、CLI 方法分派（`crates/cli/src/runtime.rs:100-104,782-890`）、生成器硬编码常量（`crates/contract-gen/src/main.rs:88,96-102,33,114`）、双端白名单/事件闸门（`apps/decky/backend/bridge.py:185,237,251`；`main.rs:899-907`）必须同步；否则出现“核心 v5 + 宿主 v4”的半套 RPC，任一端直接 `unsupported_method`/`unsupported_protocol`。**【建议】** 将 v5 wire 切换 + 双端宿主 + 生成物 + 测试作为一个不可拆分的提交单元（P4 切换提交），P5 只做删除与文档，见 §5.1。
6. **【事实】设置持久化职责在核心**：Decky `plugin_settings.py` 只存 `auto_cleanup`（schema 4，`:60,69,78`），Windows 无独立设置文件；来源间隔/订阅在 Rust 状态目录（`p1-contract.md` §2-3，`model.rs:147-157` 已加 `intervals`）。P4 宿主不应再复制间隔状态。
7. **【事实】生成物的“v4”文件名被多处硬编码**：`package-plugin.mjs:26`、`package-artifacts.py:17,53`、`install-decky.py:189,191`、`test_package_artifacts.py:33,103`、`test_install_decky.py:25`、`test-ui-contract.mjs:11`、`test-game-status.mjs:51`、`protocol_test_support.py:6`、`src-tauri/src/main.rs:2192`、`scripts/test-install-decky-bridge.mjs:39`。bump v5 时逐处更新或明确保留 `v4.schema.json` 文件名（见 §4.3）。

## 1. 现状调用链（代码事实）

### 1.1 Decky：界面动作 → Python 宿主 → Rust RPC

顶层：前端 `callable` 声明在 `apps/decky/src/index.tsx:17-41`；`Plugin` 公开方法经 `_await_startup` 包装（`apps/decky/main.py:341-357`），启动完成前一直等待 `_startup_ready`。

#### 1.1.1 整库扫描/刷新（当前 `scan(refresh_sources=False)`）

| 步骤 | 位置 | 行为 |
| --- | --- | --- |
| 1 | `index.tsx:219` “重新扫描” → `session.refresh()`（`session.ts:77-82`）→ `readSnapshot`（`index.tsx:43-49`）→ `scan(false)` | 前端只调一次 `scan` |
| 2 | `main.py:103-105` | `_require_idle()`：安装任务忙时直接 `installer_busy`；`_scan_generation += 1` |
| 3 | `main.py:107-109` | 关闭上一次 `_scan_session`（取消旧任务） |
| 4 | `main.py:110` → `_read_report`（`main.py:96-101`） | **`bridge.scan()` 第 1 次**（`main.py:97`；`bridge.py:92-93` = `_call("scan", {})`），`sources.prime(report)`（`source_service.py:30-48`） |
| 5 | `main.py:111-115` | 记录 `_scan_games` 与设置修订，构造 `current()` 栅栏 |
| 6 | `main.py:131` `ScanSession(..., project_on_update=False)` | 后台 worker 启动（`scan_session.py:37`） |
| 7 | `scan_session.py:83` → `main.py:117-123` | 先跑 `sources.refresh()` → `bridge._call("refresh_sources", {force})`（`source_service.py:124-131`） |
| 8 | `main.py:119-122` | `current()` 通过后：`sources.list_sources()`、合并诊断、`_refresh_rows` → **`bridge.scan()` 第 2 次**（`main.py:85`），整库逐行 `clear/update`（`main.py:88-92`） |
| 9 | `scan_session.py:91` → `main.py:125-129` | 再跑 `sources.scan_sources()` → `bridge._call("scan_sources", {...})`（`source_service.py:133-151`，无 `instance_id` = 整库逐游戏） |
| 10 | `main.py:127-128` | `_refresh_rows` → **`bridge.scan()` 第 3 次** |
| 11 | `index.tsx:44` 返回 snapshot；`session.acceptSnapshot`（`session.ts:59-76`） | `report.scan_progress.running` 时每 1s 调 `scan_status(generation)`（`index.tsx:18,20`；`main.py:135-141`）直到 `running=false` |
| 12 | `main.py:133` / `scan_status` | 返回快照；`scan_status` 只读内存会话，不触发 Rust |

- **【事实】** `ScanSession` 事件处理在 `scan_session.py:42-77`：`source_progress` 只更新 `scan_progress`（`:46-51`），`source_update` 更新行 `discovery`，`project_on_update=True` 时每事件调用 `project(row)`（`:52-75`）；整库扫描显式传 `False`（`main.py:131`）。
- **【事实】** 旧事件已经叫 `source_progress`/`source_update`（`scan_session.py:46,52`；`crates/protocol/src/lib.rs:966-1011`），但形状是 v4（`completed/total/running/settings_revision`，无 `phase/source_id`；`source_update` 两变体 `discovery` 或 `source_id+status`）。v5 改形状，见 §2.3。

#### 1.1.2 单游戏查询/发现（现有两条路径）

| 路径 | 位置 | 行为 |
| --- | --- | --- |
| 前端“重新查找补丁” | `index.tsx:400` → `session.discover()`（`session.ts:83-132`）→ `discoverGameRpc("start_game_discovery")`（`index.tsx:21,23-31`） | 返回 `query_report` 后走 `acceptSnapshot` |
| 宿主 `start_game_discovery` | `main.py:161-184` | 校验 `_discovery_game`（`:143-159`）；同一查询且 worker 未完成则复用（`:165-168`）；否则关闭旧会话（`:169-170`）、生成号 +1（`:171`）、`_read_report` = **整库 scan 1 次**（`:173`）、`ScanSession(..., target_instance=instance_id)`（`:183`，`project_on_update` 默认 `True`）→ 每个 `source_update` → `_project_row` → **整库 scan**（`main.py:72-80`） |
| 单游戏查询实现 | `main.py:181-182` → `source_service.py:133-151` | 带 `instance_id` 的 `scan_sources`，Rust 只查该实例（`crates/cli/src/runtime.rs:906-934`） |
| 旧 `discover_game` RPC | `main.py:193-208` → `source_service.py:153-171` → `bridge._call("discover_game")`（`source_service.py:163`） | 结束后 `_project_row` = **整库 scan 1 次**（`main.py:206`）；当前前端未调用该 RPC（`index.tsx` 只声明 `start_game_discovery`），仅测试覆盖 |
| 取消 | `main.py:186-191`（`cancel_source_query`）→ `session.close()`（`scan_session.py:106-109`） | 取消扫描会话，UI 由 `index.tsx:220` 调用 |

#### 1.1.3 来源订阅/设置动作

| 界面动作 | 位置 | 实际 RPC |
| --- | --- | --- |
| 全局设置 → 管理订阅源（列表） | `index.tsx:505-526`（`SourcesPage`），数据来源 `report.sources`（`index.tsx:512-522`）；包装 `GlobalSources`（`:592-600`） | 无独立列表 RPC；`Plugin.list_sources`（`main.py:213-214`）存在但前端未调用 |
| 启用/停用、添加源 | `index.tsx:521,523-524` → `setSourceEnabled`/`addSource`（`:34,36`）→ `main.py:220-232` | `source_service.py:95-100` → `bridge.call_sync("set_source_enabled"/"add_source")`；成功后 `_reload_snapshot`（`:86-93`）再 `readSnapshot()` = **又一次整库 scan**（`index.tsx:598`） |
| 游戏详情 → 订阅源管理 | `index.tsx:475-486`，`changeGameSource`（`:330-336`）→ `setGameSource`（`:35`）→ `main.py:234-239` | `source_service.py:102-105`；随后 `session.invalidateDiscovery` + `readSnapshot()`（**整库 scan**） |
| 全局设置页路由 | `route.ts:2-9`；`index.tsx:52-53,583-587` | `sources/subscriptions`、`sources/cache` 两条子路由 |

- **【事实】** 所有来源设置写操作都会触发一次整库扫描（`index.tsx:598` 的 `readSnapshot`），与 design §4.3“设置写入不重算整库”冲突，P4 需改为局部刷新。
- **【事实】** Decky 全局设置页没有任何“每源间隔/手动更新/最近成功/最近尝试/进度/重新匹配”控件；现有只有“刷新来源”整源按钮（`index.tsx:512`）与每源开关（`:521`）。

#### 1.1.4 任务、取消、事件转发（Decky）

- **【事实】** 安装任务：`TaskManager`（`apps/decky/backend/tasks.py:75-179`），单前台槽位 `busy()`（`:101-102`）、`start()` 拒绝第二任务（`:115-116`）、`cancel_task`（`main.py:323-326`）通过 `Task.request_cancel` 置位（`tasks.py:51-56`）。
- **【事实】** 来源任务没有独立 TaskManager，只存在于 `ScanSession.worker`；`bridge._call` 的取消 Event 由 `current_task` 上下文提供，否则新建（`bridge.py:187-191,202-214,335-368`）。
- **【事实】** 事件闸门：`bridge.py:236-256`——只允许 `SOURCE_EVENTS | {"install_progress"}`（`:237`），`install_progress` 校验后写任务进度（`:244-250`），非安装事件只允许 `method in {"scan_sources","refresh_sources"}`（`:251`），否则 `contract_mismatch`；`validate_event` 由生成的 v4 `SCHEMA` 校验（`protocol_validation.py:173-180`）。
- **【事实】** 启动：`_main`→`_initialize`（`main.py:31-67`）依次 bridge 预检（`:56`）、`prepare_host_state`（`:57`）、`TaskManager`（`:59`）、`Settings`（`:60`）、`SourceService.initialize`（`:61`，内部 `call_sync("list_sources")`，`source_service.py:24-28`）、`recover`（`:67`）。**没有任何启动/到期调度**。
- **【事实】** 卸载：`_unload`（`main.py:331-338`）关闭扫描会话、`tasks.shutdown()`、`sources.close()`。

### 1.2 Windows：界面动作 → Tauri command → Rust core

顶层：`apps/windows/src/main.tsx` 通过 `invoke` 调用 Tauri 命令；一次性只读方法走通用 `core_call`（`main.tsx:32,48-50`；`main.rs:450-457`），安装/来源任务走专用命令。

#### 1.2.1 整库扫描与自动来源扫描

| 步骤 | 位置 | 行为 |
| --- | --- | --- |
| 1 | `main.tsx:324-344` `refresh()`（挂载 `:346`；顶部“重新扫描” `:514`） | `loadCapabilities`（`:159-166`）→ `core_call("scan")`（`:330`）+ `core_call("list_transactions")`（`:331`） |
| 2 | `main.rs:375-405` `execute_core_call` → `require_capabilities`（`:393-395`，实现 `:260-267`）→ `build_params`（`:396`）→ `run_one_shot`（`:407-448`） | 启动 `steam-patch serve` 子进程，一行 JSON 请求/一行响应 |
| 3 | `main.tsx:343` | `supportsSourceScan(caps)` 检查 `caps.methods.includes("scan_sources")`（`:153-155`），有已安装实例就 **自动** `startSourceScan(false)` |
| 4 | `main.tsx:250-279` → `invoke("start_source_scan", {force, instanceId, preferredLanguage})`（`:259`） | Rust `main.rs:1324-1438`：`require_capabilities`、`source_scan_request`（`:1272-1294`），`serve-task` 子进程写请求 |
| 5 | `main.rs:1361-1394` | 抢 `SourceScans` 单槽（`:1362-1365`，忙则 `source_scan_in_progress`），保存 `SourceScanJob` |
| 6 | `main.rs:1400-1435` + `read_source_task_output`（`:941-961`） | `read_core_stream` 解析事件（`:876-920`），`source_progress` 写进度（`:952-956`），其余 `app.emit(name, value)`（`:957`）；结束保存 `slot.results`（`:1421-1426`） |
| 7 | `main.tsx:176-222` | `listen("source_progress"/"source_update")`（`:228-229`）；`source_update` 且含 `discovery` 时 **每个事件再 `core_call("scan")`**（`:194-206`），并更新行 `discovery`；`source_progress` 更新 `sourceScan`（`:184-193`） |
| 8 | `main.tsx:263-266` | 任务返回后再 **`core_call("scan")` 一次**，再 `applyScanResults`（`:235-248`） |
| 9 | `main.tsx:281-295` → `invoke("cancel_source_scan")`（`main.rs:1466-1475`） | 写 `cancel\n`（`main.rs:820-822,702-706`） |
| 10 | `main.tsx:39-46,518` | 进度/取消状态由前端 `SourceScanState` 维护，无宿主快照轮询（`source_scan_status` `main.rs:1440-1464` 未被前端调用） |

#### 1.2.2 来源管理（Windows）

| 界面动作 | 位置 | 实际调用 |
| --- | --- | --- |
| 打开“来源管理”抽屉 | `main.tsx:514,548-567` → `SourcePanel.tsx` | 打开时 `loadSources`（`main.tsx:348-351,370-382`） |
| 读全局/游戏订阅 | `main.tsx:373-375` | `invoke("list_sources")` / `invoke("list_game_sources")`（`main.rs:474-486`） |
| 刷新列表 / 刷新来源索引 / 强制刷新索引 | `SourcePanel.tsx:85-90` → `main.tsx:384-390`（reload）、`:411-416`（refreshSources） | `invoke("refresh_sources", {force})` → `main.rs:536-644`（**`serve-task` + 同一 `SourceScans` 槽**，`:570-572`） |
| 启用/停用全局源 | `SourcePanel.tsx:47` → `main.tsx:392-399` | `set_source_enabled` + 重读 `list_sources`/`list_game_sources`（**无 scan**） |
| 游戏订阅开关 | `SourcePanel.tsx:119-120` → `main.tsx:401-409` | `set_game_source` + 重读（**无 scan**；提示“可点击重新查找生效”`:408`） |
| 添加来源 | `SourcePanel.tsx:129-136` → `main.tsx:418-425` | `add_source` + 重读 |
| 游戏详情“重新查找” | `main.tsx:545` → `findGame`（`:297-307`）→ `startSourceScan(true, instanceId)` | 与整库扫描共用单槽；单游戏路径 = 1 个事件 `scan` + 结束 `scan`，**共 2 次整库 scan** |
| 打开来源网页 | `SourcePanel.tsx:48,92-93` / `main.tsx:427-430` → `main.rs:1501-1509` | `open_source`（仅 https） |

#### 1.2.3 Windows 任务通道与生命周期

- **【事实】** 安装任务 `Jobs`（`main.rs:161-170`），`start_install`（`:1073-1241`）独占写槽；`install_status`/`cancel_install`（`:1243-1270`）轮询/取消（`main.tsx:355-367,517`）。
- **【事实】** 来源任务单槽 `SourceScans`/`SourceScanJob`（`main.rs:172-192`），`refresh_sources` 与 `start_source_scan` 共用（`:570-572`、`:1362-1365`）。**这不满足 §10“不同源可排队”、design §4.3“重新匹配一个游戏不能取消其他来源更新”**。
- **【事实】** 事件方法白名单按方法硬编码（`main.rs:899-907`）：`install_patch`→`install_progress`；`scan_sources`→`source_progress` 或含 `discovery` 的 `source_update`；`refresh_sources`→含 `source_id` 的 `source_update`。其它一律 `invalid_core_response`。
- **【事实】** `read_core_stream` 用 `Method` 枚举校验响应（`:913-916`；`Method` 来自 `steam_patch_protocol`，`Cargo.toml:17` 直接依赖）。
- **【事实】** 关闭窗口：有安装或来源任务时 `prevent_close` 并取消（`main.rs:1518-1540`）。

## 2. RPC 与事件转发层（v5 落点）

### 2.1 Decky bridge.py + protocol_validation.py

| 机制 | 现状（事实） | v5 需改 |
| --- | --- | --- |
| 方法集合 | `protocol_validation.py:18-26` 全部从生成 `protocol.py` 读取；`METHODS`/`SOURCE_METHODS`/`CONTROLLED_METHODS` 自动跟随生成物 | 无需手改，但生成物必须重新生成 |
| 请求方法闸门 | `bridge.py:172-173`（`method not in METHODS` → `unsupported_method`）、`protocol_validation.validate_request`（`:148-156`） | 自动跟随 |
| 任务通道方法 | `bridge.py:185-186` 免预检白名单、`:188-193` 由 `CONTROLLED_METHODS` 决定 `serve-task` | `update_source`/`match_games`/`rematch_game` 应进 `CONTROLLED_METHODS`（由协议 `is_task` 派生） |
| 读锁 | `bridge.py:161-169` 只给 `scan`/`list_transactions` 加 journal 读锁 | **【建议】** 评估 `project_instances` 是否需要同一锁；它不读 journal，可维持无锁 |
| 事件闸门 | `bridge.py:236-256`：`SOURCE_EVENTS | install_progress`（`:237`）、非安装事件仅允许旧两方法（`:251`） | 更新 `SOURCE_EVENTS`（生成器 `main.rs:88`）并放行新任务方法；否则新事件一律 `contract_mismatch` |
| 事件校验 | `protocol_validation.validate_event`（`:173-180`）走生成的 `Event` schema | 自动跟随，但 v5 事件形状需先改 Rust 类型 |
| 序列 | `CoreBridge._source_event`（`bridge.py:142-154`）要求从 0 递增、ID 匹配；`EventSequence` 同语义（`crates/protocol/src/lib.rs:1102-1137`） | 不变 |
| 能力校验 | `validate_capabilities`（`:183-199`）比对 `methods` 长度/顺序、`contracts`、`INSTALLER_VERSIONS`、`SOURCE_VERSIONS`、`sources.methods` | 自动跟随生成常量；但 `SOURCE_VERSIONS` 字面量在生成器里手写（见 §4.3） |
| 取消 | controlled 调用：`signal_cancel` 轮询 Event 写 `cancel\n`（`:202-214`）；EOF 等价取消（docstring `:202-204`；`bridge.py:348-362`） | 新任务复用同一机制 |
| 一次调用 API | `CoreBridge._call`（`:156-169`）、`call_sync`（`:370-376`） | 新增 5 方法可复用 `_call`（task）或 `call_sync`（读/写） |

**【建议】Decky 新增面（宿主 RPC，不是 wire）**：`set_source_update_interval(source_id, interval_hours)`、`update_source(source_id, force)`、`match_games(instances?, force_details?)`、`rematch_game(ref, force_details?)`、`project_instances(instance_ids)`；每个 serve-task 任务维护自己的 `cancel Event` + 进度快照，暴露 `source_task_status(task_generation)`/`cancel_source_task(task_generation)` 供轮询（Decky 前端没有推送通道）。不要复用 `TaskManager` 的安装单槽（`tasks.py:101-116`）。

### 2.2 Windows main.rs 的 Operation/build_params/stream/槽位

| 机制 | 现状（事实） | v5 需改 |
| --- | --- | --- |
| `Operation` 枚举 | `main.rs:20-54`：12 个方法（含 `RefreshSources`、`DiscoverGame`），`method()` 映射 `:37-53` | 删旧两项；新增 one-shot 的 `SetSourceUpdateInterval`、`ProjectInstances`；task 方法建议走专用命令（与 `start_install`/`start_source_scan` 同型） |
| `CallArguments` | `main.rs:126-139`，`camelCase + deny_unknown_fields`：instance/release/selection/source/enabled/subscribed/rule/force/name_hints/preferred_language | 增 `intervalHours`、`instances`、`forceDetails`（或按方法拆独立 command 参数） |
| `build_params` | `main.rs:280-373`：合并 host + 参数、逐方法必填校验、`protocol::decode_request` 预校验 | 增 5 方法校验（`interval_hours` 必需、`update_source` 需 `source_id`、`rematch_game` 需 instance/appid、`project_instances` 需 `instance_ids`） |
| 一次性调用 | `run_one_shot`（`:407-448`）+ `execute_core_call`（`:375-405`） | 新读/写方法走同路径（`set_source_update_interval`、`project_instances`） |
| 任务调用 | `start_source_scan`（`:1324-1438`）、`refresh_sources`（`:536-644`）各自 spawn `serve-task`、`read_*_task_output`、维护槽位 | 抽共用 task-spawn 或复制最小骨架；**槽位语义必须从“单槽”改为“按任务/来源”** |
| 流解析 | `read_core_stream`（`:876-920`）：`Method` 枚举校验 + 每方法事件白名单（`:899-907`）；`read_source_task_output` 硬编码 `Method::ScanSources`（`:950`）、`read_refresh_stream` 硬编码 `Method::RefreshSources`（`:972`） | 加 `Method` 新变体/事件白名单（`source_progress` 带 phase、`source_update`、`match_progress`、`match_update`）；新任务读取函数带对应 `Method` |
| 事件转发 | `app.emit(name, value)`（`:957,994`）；前端 `listen`（`main.tsx:228-229`） | 事件名扩为 4 类；`match_update` 需要按 appid/instance 合并行，不能触发整库 scan |
| 进度快照 | `SourceProgress = Option<SourceProgressEvent>`（`:173`），`source_scan_status` 返回 `{running,cancelled,progress,results}`（`:1440-1464`） | 改为按任务/来源键控的快照；供重连/前端轮询与自动调度判定 |
| 能力前置 | 每个命令先 `require_capabilities`（`:260-267,393-395,546,1084,1333`） | 自动跟随 v5；但 `validate_capabilities` 要求 `sources.methods` 与 `SOURCE_METHODS` 精确一致（`crates/protocol/src/lib.rs:446-494`） |
| 退出清理 | `Closing` + 关闭时取消来源任务（`:1523-1537`） | 调度线程与所有新任务也要响应 `Closing` |

**【建议】Windows 命令面**：保留 `list_sources/list_game_sources/set_*/add_source/open_source`；删 `refresh_sources`、`discover_game`、`start_source_scan/source_scan_status/cancel_source_scan`（P5）；新增 `set_source_update_interval`、`update_source`、`match_games`、`rematch_game`、`project_instances` 与 `source_task_status`/`cancel_source_task`（宿主内部）。`core_call` 的 `CoreOperation`（`main.tsx:32`）保持只读四项。

### 2.3 目标 v5 方法/事件与宿主落点（契约 §9/§9.1）

| 契约项 | 形状要点 | Decky 落点 | Windows 落点 |
| --- | --- | --- | --- |
| `update_source` | serve-task；`source_id, force?` → `SourceUpdateResult{source_id,settings_revision,status,affected_appids}` | `source_service.update_source` + `main.py.update_source`；任务注册/去重 | 新命令 + `serve-task` 流；同源去重 map |
| `match_games` | serve-task；`instances?, force_details?` → `{settings_revision,source_revisions,results: GameMatch[]}` | `source_service.match_games` + 宿主方法 | 新命令 + `match_progress`/`match_update` 流 |
| `rematch_game` | serve-task；instance/appid | 同上（单游戏） | 同上；不能取消其它任务 |
| `set_source_update_interval` | serve；`source_id, interval_hours`（0=关闭） | `source_service._call_sync` 同类方法 | 一次性命令 |
| `project_instances` | serve；`instance_ids` → `{settings_revision,instances}` | 单行/批次投影，替代 `_project_row`/`_refresh_rows` | 一次性命令，替代事件后整库 `scan` |
| `scan` | 只读整库投影（不再刷新来源） | `main.py.scan` 改为仅 `_read_report` | 已有；删除自动 `startSourceScan`（`main.tsx:343`） |
| `source_progress` | `{source_id?,phase: index/details/matched,completed,total,running,settings_revision}` | `bridge` 事件闸门 + `source_service` 进度快照 | `read_core_stream` 白名单 + `SourceProgress` 改形状 |
| `source_update` | `{source_id,status:SourceStatus,settings_revision}` | 同上；`SourceService._apply_summaries` 更新 | 删旧 `discovery` 变体处理（`main.tsx:194`） |
| `match_progress` | `{phase: inventory/index/details/fallback,completed,total,running,settings_revision}` | 新事件名进 `SOURCE_EVENTS`/bridge | 新白名单/新前端监听 |
| `match_update` | `{appid,instance_id?,status,candidates,diagnostics,settings_revision}` | 宿主行合并，不整库 scan | 行合并；不整库 scan |

**【事实】** 现有生成器把 `SOURCE_EVENTS` 硬编码为两项（`crates/contract-gen/src/main.rs:88`），Decky 运行时读该常量（`bridge.py:237`）。新增 `match_progress`/`match_update` 必须同时改生成器与事件类型（`crates/protocol/src/lib.rs:1012-1019` 的 `Event` 枚举）。

## 3. 设置与来源管理 UI 落点

### 3.1 Decky

| 区域 | 文件/行 | 现状 | P4 新增 |
| --- | --- | --- | --- |
| 全局设置“管理订阅源” | `apps/decky/src/index.tsx:505-526`（`SourcesPage`）、`:583-587`（路由）、`:592-600`（`GlobalSources`） | 每源：名称/站点/启用/条目数/状态/缓存时间/最近查询/开关；顶部“刷新来源”整源按钮（`:512`）；添加源（`:523-524`） | 每源“手动更新”按钮、间隔下拉（关闭/6h/12h/24h/3d/7d/14d）、最近成功（`cached_at`）、最近尝试（`checked_at`+code）、当前 phase 进度条（`source_progress`）、更新按钮在运行中显示“更新中/取消”；列数据改从 `list_sources` 取（`Plugin.list_sources` 已存在 `main.py:213-214`，前端目前未接） |
| 游戏详情“订阅源管理” | `index.tsx:475-486`（`sources` 面板）、`:330-336`（`changeGameSource`） | 每源开关、匹配文案、查询状态、打开网页 | 保留；增加“来源覆盖不完整”提示、HGamer 未查询/禁用说明；返回行用 `project_instances` |
| 游戏详情“查找来源” | `index.tsx:394-402` | “重新查找补丁”（`:400`）调 `start_game_discovery`，取消（`:395`） | 改为“重新匹配”（`rematch_game`，复用普通源索引、不刷新来源）；缺索引时提示“先更新来源”；显示“索引匹配/详情获取”任务阶段 |
| 主面板进度 | `index.tsx:202`（scan_progress）、`:220`（取消来源查询） | 以扫描会话为粒度 | 改为来源更新/匹配任务进度（可能多任务，按 source/game 显示） |
| 全局设置入口 | `index.tsx:218` | 打开 sources 路由 | 不变 |

- **【事实】** 前端依赖 `report.sources[].status/cached_at/checked_at/entry_count`（`index.tsx:517-520`）；`reason()` 对旧状态码有映射（`packages/ui/src/index.ts:94-112`）。
- **【建议】** 间隔/到期展示放在共享 `packages/ui`（见 3.3），Decky/Windows 只做布局，避免两份文案漂移。

### 3.2 Windows

| 区域 | 文件/行 | 现状 | P4 新增 |
| --- | --- | --- | --- |
| 来源管理抽屉 | `apps/windows/src/SourcePanel.tsx:53-140` | 进度区（`:75-81`，按“已查询 X/Y 款游戏”）、全局来源（`:83-102`）、整源“刷新来源索引/强制刷新索引”（`:85-90`）、此游戏订阅（`:104-127`）、添加源（`:129-136`） | 每源卡片（`SourceCard` `:32-51`）增：手动更新、间隔下拉、最近成功/最近尝试、phase 进度；替换整源两个刷新按钮；fallback 源单列“补查来源”并强调不参与定时更新 |
| 主界面来源区 | `apps/windows/src/main.tsx:514`（来源管理按钮）、`:518`（来源扫描进度条）、`:519`（通知） | 单一 `sourceScan` 状态 | 按任务/来源显示；后台自动更新可提示但不阻塞 |
| 游戏详情“查找来源” | `main.tsx:545` | “重新查找”（`force=true` → `start_source_scan`） | 改为“重新匹配”（`rematch_game`）；显示覆盖状态、HGamer 状态 |
| 游戏列表状态 | `apps/windows/src/sources.ts:114-147`（`patchChip`/`sourceChip`）、`main.tsx:524-527` | 旧 status 语义 | 改为 §7.1 六态（待匹配/普通源命中/HGamer 命中/无结果/未完成/失败），并区分“未查询/不完整” |
| 事件处理 | `main.tsx:176-233` | 单一 active request token（`:133,179-182`） | 多任务并发时按 `request_id`/`source_id` 维持 map；`match_update` 按 appid/instance 合并；删除含 `discovery` 的旧分支（`:194-221`） |

- **【事实】** `SourcePanel` 的 props 类型（`SourcePanel.tsx:5-24`）没有间隔/更新回调，需扩展；`statusTone/chipLabel/statusLabel/sourceRoleOf` 在 `sources.ts:49-98,230-232`。
- **【建议】** Windows 侧可用 Tauri `listen` 推送（已有），无需轮询；但重连/隐藏后仍需要 `source_task_status` 快照（可复用 `install_status` 模式，`main.rs:1243-1259`）。

### 3.3 `packages/ui` 共享文案/匹配函数

| 位置 | 现状职责 | P4 更新 |
| --- | --- | --- |
| `packages/ui/src/index.ts:2-8` | 从生成 `protocol.ts` 导入/导出 wire DTO；`ScanResult` 追加宿主字段 | v5 新 DTO（`GameMatch`、`SourceUpdateResult`、4 类事件）自动可用；清理旧 `ScanSourcesResult` 类类型引用 |
| `:11-13 candidateMatched` | 可信匹配判定（appid/verified_alias/official_title） | 保留（语义不变） |
| `:14-38 displayPatches`、`:39-41 hasMatchedPatch` | 由 `patches + discovery.candidates` 生成显示联合 | `InstanceProjection.discovery` 保持形状但状态值改为六态（契约 §9 末段）；补丁/候选合并逻辑基本不动，需更新状态判断 |
| `:43-54 installSummary` | 依赖旧 discovery 状态（`complete/empty/cache_complete/partial/...`） | 改为新顶层状态 + 覆盖不完整/HGamer 未查询/禁用文案；与 Windows `patchChip` 对齐 |
| `:88-293 reason` | 旧状态码与大量错误码中文文案 | 删/改旧来源状态；增加 `publisher_match/fallback_match/no_match/incomplete/failed/pending`、`no_index/index_incomplete/no_candidate/detail_incomplete/detail_failed`、`not_queried/disabled/not_needed` 等文案 |
| `:341-348 progressPercent/progressStageLabel` | 安装进度 | 新增来源/匹配 phase 文案（index/details/matched；inventory/index/details/fallback） |
| `:349-380 safeSourceUrl/sourceRefreshFailed/sourceQueryComplete/sourceRefreshSucceeded` | 旧来源状态判定 | 更新或替换为 `matchStatus*`/`sourceCoverageComplete`/`fallbackDisabled` 等共享判定 |
| 新增建议 | — | `sourceIntervalHours`/`sourceIntervalLabel`（预设 0/6/12/24/72/168/336）、`sourceScheduleText(lastAttempt,nextDue)`、`fallbackDisplayStatus(record)`（实现 design §3.2 五条规则）、`sourceCoverageText` |

**【事实】** 两端各自维护了重复文案：Decky `discovery.ts:3-11,22-26`；Windows `sources.ts:59-98,171-232`。**【建议】** P4 把“匹配状态/间隔/最近尝试/覆盖不完整/HGamer 状态”收进 `packages/ui`，两端只保留布局差异，减少 §P4 的 UI 漂移。

### 3.4 需要新增的展示项（逐项）

1. **手动更新（每源）**：按钮 + 运行中禁用/取消 + 结果状态（`source_update`）。
2. **自动间隔（每源）**：预设关闭/6h/12h/24h/3d/7d/14d；值来自 `list_sources.interval_hours`，写 `set_source_update_interval`。
3. **最近成功**：`cached_at`；**最近尝试**：`checked_at` + `status.code`（失败/部分/未修改分开显示）。
4. **进度**：`source_progress.phase=index|details|matched`；不得把整条任务显示为纯离线（design §4.4/契约 §9.1）。
5. **重新匹配（每游戏）**：已命中、无补丁、失败都显示；缺普通源缓存提示“先更新来源”。
6. **来源覆盖不完整**：`SourceStatus.complete=false` 或 `SourceMatchRecord.status in {no_index,index_incomplete,detail_incomplete,detail_failed}`、顶层 `incomplete`。
7. **HGamer 未查询/禁用**：`FallbackMatchRecord.status in {not_queried,incomplete,failed}` 显示“未完成”；`disabled` 且普通源无结果时显示“已启用来源中未找到补丁”，不得声称 HGamer 查无结果（design §3.2）。
8. **普通源后来命中隐藏 HGamer**：显示层按契约 §7.1 顶层状态推导，不在宿主复制策略（design §4）。

## 4. 契约生成与产物

### 4.1 `cargo run -p steam-patch-contract-gen -- generate` 生成什么

生成逻辑：`crates/contract-gen/src/main.rs:30-116`；输出根 `:139`；`generate`/`check` 模式 `:133-156`。

| 产物 | 生成位置 | 内容 |
| --- | --- | --- |
| `packages/contracts/generated/v4.schema.json` | `main.rs:33` | schemars 全量 schema（`$defs`） |
| `packages/contracts/generated/protocol.ts` | `main.rs:39-67` | ts-rs 类型 + `PROTOCOL_VERSION/METHODS/INSTALL_STAGES/SOURCE_METHODS/CONTRACTS/SCHEMA` + `validId/makeRequest`（`:57-65`） |
| `packages/contracts/generated/protocol.py` | `main.rs:68-113` | 纯数据：`PROTOCOL_VERSION/METHODS/SOURCE_METHODS/CONTROLLED_METHODS/SOURCE_EVENTS/INSTALL_STAGES/limits/CONTRACTS/INSTALLER_VERSIONS/SOURCE_VERSIONS/RESULT_TYPES/SCHEMA` |
| `packages/contracts/fixtures/v4/cases.json` | `main.rs:114` → `fixtures::cases()`（`crates/protocol/src/fixtures.rs:130-498`） | 每方法请求/成功/失败、事件、序列、早期错误 |

检查：`check` 模式逐字节比对（`:141-156`），漂移非零退出；`println!("v4 contracts {mode}: ok")`（`:157`）也要随版本改。

### 4.2 双端如何消费

| 消费方 | 机制 | 位置 |
| --- | --- | --- |
| Decky 运行时 | 插件包内 `backend/protocol.py`；仓库测试从 `packages/contracts/generated/protocol.py` 加载 | `protocol_validation.py:10-16`；打包 `package-plugin.mjs:25` |
| Decky 包校验 | `backend/v4.schema.json` 与生成物逐字节一致 | `package-plugin.mjs:26`；`package-artifacts.py:49-69`（`:53`）；`install-decky.py:189-191` |
| Decky 测试 | `fixtures/v4/cases.json` | `protocol_test_support.py:6`；`test_bridge.py:61-110` |
| Windows Rust | 编译期依赖 `steam-patch-protocol`（`apps/windows/src-tauri/Cargo.toml:17`），无生成物 | `main.rs:15` 起 |
| Windows TS | `import type` 生成 `protocol.ts` + 共享夹具 | `apps/windows/src/sources.ts:30-31`；`main.tsx:36`；`test-game-status.mjs:51` |
| 共享 UI | `../../contracts/generated/protocol` 类型 | `packages/ui/src/index.ts:2-4` |
| 契约一致性测试 | 生成 TS + fixtures | `scripts/test-ui-contract.mjs:9-11`；`scripts/test-install-decky-bridge.mjs:39,124` |
| Windows 宿主测试 | 读取 fixtures 文件 | `src-tauri/src/main.rs:2188-2197` |

- **【事实】** Decky 运行时只 import `protocol.py`（`protocol_validation.py:10-16`，schema 内嵌为 `SCHEMA`），**不读取** `v4.schema.json`；该文件只用于打包/安装时的产物一致性校验（`package-artifacts.py:49-57`；`install-decky.py:189`）。Windows 不消费 `.py`/`.schema`，只编译 `steam-patch-protocol` 并 `import type` 生成 `protocol.ts`。
- **【事实】** `fixtures/*/cases.json` 不进入 Decky 插件包（`protocol_test_support.py:1-2` 注明 non-packaged；`package-plugin.mjs:23-24` 后端 allowlist 不含测试文件），但它是 Rust/Python/Windows 宿主测试的共同输入。

### 4.3 bump v5 必须同步的生成物/常量/校验

| 类别 | 位置 | 要做什么 |
| --- | --- | --- |
| 版本常量 | `crates/protocol/src/lib.rs:21`；`apps/decky/plugin-output/backend/protocol.py`（生成）；`package-plugin.mjs:37`、`install-decky.py:191` 的 `runtime.json protocol_version` | 4→5，生成后重新打包 |
| 方法面 | `crates/protocol/src/lib.rs:66-147`（`Method/ALL_METHODS/METHODS/SOURCE_METHODS/is_task`） | 20 方法：删 `refresh_sources/scan_sources/discover_game`；加 `project_instances`、`set_source_update_interval`、`update_source`、`match_games`、`rematch_game`；`is_task` 改为 install+3 个新 task 方法 |
| 参数/结果 DTO | 同文件 `:159-192`（Params/RequestParams）、`:495-565`（results/MethodResult） | 新增 `ProjectInstancesParams`、`SetSourceUpdateIntervalParams`、`SourceTaskParams{source_id,force?}`、`MatchGamesParams{instances?,force_details?}`、`RematchGameParams` 等；结果类型按契约 §9 |
| 事件 | 同文件 `:920-1101`（类型/`Event`/`validate_event`） | `source_progress` 增 `source_id?/phase`；`source_update` 收敛为 `{source_id,status,settings_revision}`；新增 `match_progress`/`match_update`；删 `GameSourceUpdateEvent` |
| capabilities | 同文件 `:427-494` | `SourceCapabilities` 改为 §9.2 七项 + methods；`validate_capabilities` 校验必须同步（`:452-480`） |
| fixtures | `crates/protocol/src/fixtures.rs:130-498`；capabilities 字面量在 `:4-6` | 每方法两响应 + 请求；4 类事件与非法样本；旧方法请求按“未知方法”处理（删 `success_result` 分支 `:55-96`）；capabilities fixture 同步新 `sources` 版本字段 |
| 生成器硬编码 | `crates/contract-gen/src/main.rs:88`（`SOURCE_EVENTS` 两项）、`:96-98`（`INSTALLER_VERSIONS`）、`:100-102`（`SOURCE_VERSIONS`：`discovery_schema_version/adapter/parser/query`）、`:33,114`（文件名）、`:56`（TS 别名，含旧 `GameSourceUpdateEvent/RefreshSourceUpdateEvent`）、`:157`（日志） | 更新为 v5：事件 4 类；`SOURCE_VERSIONS` 七项（index/detail/match/schedule/adapter/parser/encoding）；文件名 v5 或保持；别名改为新事件名 |
| 文件名与校验 | 见 §0-7 的 10 个文件中十余处引用 | 若改名为 `v5.schema.json`/`fixtures/v5`，逐处更新；若保留 v4 文件名则只改内容（但 README/日志语义不一致，建议改名） |
| 文档 | `packages/contracts/README.md:3,5,8-31,41`；`docs/client-guide.md:66`；`docs/features/README.md:9`；本功能 `progress.md` | 升 v5/20 方法/新事件；P4 完成后补施工记录 |
| 生成物 | `packages/contracts/generated/*`、`fixtures/v4/*`（或 v5） | 只由生成器写入；禁止手改；`contract-gen check` 作为 CI 门 |

**【事实】** `SOURCE_EVENTS`、`INSTALLER_VERSIONS`、`SOURCE_VERSIONS` 不是从 Rust 类型派生，而是生成器内手写字面量；改 `SourceCapabilities` 时如果漏改，生成物与协议校验会分叉（Decky `validate_capabilities` 用的是生成常量，`protocol_validation.py:191-195`）。**【建议】** P4 改生成器时同时加一个一致性断言/测试（生成器值 == `Capabilities` 契约字段），避免下一次再漂移。

### 4.4 需要改的构建/打包脚本

| 脚本 | 行 | 改动 |
| --- | --- | --- |
| `apps/decky/scripts/package-plugin.mjs` | `:23-26,37` | 后端允许清单（删除 `scan_session.py` 或新增模块）；生成物文件名；`runtime.json` 协议 5 |
| `scripts/package-artifacts.py` | `:17,53,59` | `PRODUCTION_BACKEND` 集合；schema 文件名；runtime 协议版本 |
| `scripts/install-decky.py` | `:189,191` | required 文件清单；runtime 协议版本 |
| `scripts/test_package_artifacts.py` / `test_install_decky.py` | `:33,103` / `:25` | 同步清单与版本断言 |
| `scripts/test-ui-contract.mjs` | `:10-11` | fixtures 路径；可选新增 v5 事件/方法断言 |
| `apps/windows/scripts/test-game-status.mjs` | `:51,65` | fixtures 路径；输出文案 |
| `scripts/test-install-decky-bridge.mjs` | `:39,124` | stub caps/协议版本 |
| `apps/windows/scripts/prepare-sidecar.mjs` | — | 不依赖协议版本，无需改（`rustc/cargo` 需在 PATH，见 §6.3） |
| `apps/windows/vite.config.ts` / `tauri.conf.json` | — | 不依赖协议版本，无需改 |

## 5. P4 改动清单（按文件、依赖顺序）

### 5.1 切换边界（必须与协议 v5 同一提交）

**【事实】** v5 白名单删除旧 3 方法后，旧宿主调用会立即 `unsupported_method`；版本号不同会在写入前被 `unsupported_protocol` 拒绝（`crates/protocol/src/lib.rs:261,604`；`protocol_validation.py:148-156`；`main.rs:407-417`）。**【建议】** 采用单一“wire switch”提交单元，内容同时包含：

1. `crates/protocol` + `crates/cli` + `crates/contract-gen` + 重新生成的 `packages/contracts/**`（v5）；
2. Decky Python（`bridge.py`/`protocol_validation.py` 自动 + `main.py`/`source_service.py`/删除 `scan_session.py`）+ Decky TS；
3. Windows `main.rs` + Windows TS；
4. `packages/ui` 文案/判定；
5. 打包脚本与测试。

P1–P3 可以先把新能力做在 core 内部（`crates/core/src/sources/*`、CLI 内部函数、core 测试），但**在 wire switch 提交之前不要把 `PROTOCOL_VERSION` 改成 5、也不要从 `METHODS` 里删旧方法**；否则分支在该提交上无法构建/测试双端宿主。若 P1 决定提前升 v5，则 P4 的宿主改动必须并入同一合并单元（不能作为后续独立提交发布）。

### 5.2 Rust/契约（联合 owner）

| 文件 | 改什么 | 为什么 | 顺序 |
| --- | --- | --- | --- |
| `crates/protocol/src/lib.rs` | 版本、方法、Params/Result、4 事件、capabilities、validate_* | 唯一 wire 源头 | 1 |
| `crates/protocol/src/fixtures.rs` | 新方法请求/成功/失败、4 事件、非法样本、旧方法移除 | 生成夹具 + 双端共享测试 | 2 |
| `crates/cli/src/runtime.rs` | `Params` 新字段（`:18-48`）；`METHODS/SOURCE_METHODS/SOURCE_STREAM_METHODS`（`:100-104`）；`capabilities()`（`:165-199`）；`execute_source_method`（`:782-875`）；`execute_streaming`（`:877-890`）；新增 update/match/rematch/interval/project_instances 实现（P2/P3 提供 core 函数）；删旧 `scan_sources_impl/refresh_sources_impl` 调用 | 核心执行面 | 3 |
| `crates/cli/src/main.rs` | 流式方法分派 `:149-151`；测试字符串 4→5（`:386-845`） | `serve-task` 通道 | 4 |
| `crates/contract-gen/src/main.rs` | 文件路径、SOURCE_EVENTS/SOURCE_VERSIONS、TS 别名、日志 | 生成器 | 5 |
| `packages/contracts/**` | `generate` 输出 + README | 双端消费物 | 6 |

### 5.3 Decky Python

| 文件 | 改什么 | 为什么 | 顺序 |
| --- | --- | --- | --- |
| `apps/decky/backend/protocol_validation.py` | 预期无需改代码（自动跟随生成物）；确认 `validate_capabilities` 对新 `SOURCE_VERSIONS` 键集合仍工作 | 能力/事件校验 | 6 后 |
| `apps/decky/backend/bridge.py` | 事件闸门（`:236-256`）放行新任务方法与 4 类事件；免预检名单（`:185-186`）按需；读锁（`:161`）评估 | 传输层 | 7 |
| `apps/decky/backend/source_service.py` | 替换 `refresh/scan_sources/discover_game`（`:124-171`）为 `update_source/match_games/rematch_game/set_interval/project_instances`；任务注册表（task_id、kind、source/appid、cancel Event、progress、result）；同源去重；`_apply_summaries` 保留 | 来源任务层 | 8 |
| `apps/decky/main.py` | 删除 `ScanSession` 导入/会话（`:18,64,107-109,131,165-170,183,300-304,331-338`）、`_project_row/_refresh_rows`（`:72-94`）、`start_game_discovery/cancel_source_query/discover_game/scan_status`（`:135-208`）；`scan` 改纯 `_read_report`；新增 5 个公开 RPC + 任务状态/取消；启动/卸载调度任务；`_require_idle` 只保留安装与短写 | 宿主入口 | 9 |
| `apps/decky/backend/scan_session.py` | P5 删除（或 P4 保留空壳仅到切换提交） | §11 | 10 |
| `apps/decky/backend/plugin_settings.py` | 不改（间隔在核心 settings） | 避免双份设置 | — |
| `apps/decky/backend/tasks.py` | 不改；来源任务不要塞进单槽 | 安装任务语义 | — |
| `apps/decky/scripts/package-plugin.mjs` | 后端 allowlist / schema 名 / runtime 版本 | 打包 | 11 |

### 5.4 Decky TS

| 文件 | 改什么 | 顺序 |
| --- | --- | --- |
| `apps/decky/src/index.tsx` | callables（`:34-41`）加 `list_sources/set_source_update_interval/update_source/match_games/rematch_game/project_instances/source_task_status/cancel_source_task`；`SourcesPage`（`:505-526`）加每源手动更新/间隔/最近成功/最近尝试/phase 进度；删除“刷新来源”（`:512`）；`GameManager`“重新查找补丁”（`:400`）改“重新匹配”；`sources` 面板（`:475-486`）加覆盖/HGamer 状态；`changeGameSource`（`:330-336`）改局部刷新；主面板进度/取消（`:202,220`）改任务粒度 | 9 后 |
| `apps/decky/src/session.ts` | 删除 `discover`/`scanStatus` 轮询（`:59-76,83-132`）；新增来源/匹配任务快照轮询与 `project_instances` 合并；`invalidateDiscovery`（`:133-142`）改为按任务结果失效 | 同上 |
| `apps/decky/src/discovery.ts` | `discoveryStatus`（`:3-11`）改为六态/HGamer 覆盖文案；`candidateSource`（`:22-26`）保留 | 同上 |
| `apps/decky/src/route.ts` | 若新增设置子页（可选）扩展 `settingsRoutes`（`:2-9`） | 视 UI 方案 |

### 5.5 Windows Tauri Rust

| 文件/位置 | 改什么 | 顺序 |
| --- | --- | --- |
| `main.rs:20-54` | `Operation` 增删；task 方法用专用命令 | 6 后 |
| `main.rs:126-139,280-373` | `CallArguments` 新字段、`build_params` 校验 | 同上 |
| `main.rs:172-195` | `SourceScans` 单槽改为任务注册表（key = task_id / source_id），保存 progress、results、cancel、kind | 同上 |
| `main.rs:536-644,1324-1475` | 删除 `refresh_sources`/旧 `start_source_scan`；新增 `update_source/match_games/rematch_game` 命令、`source_task_status/cancel_source_task`；同源去重返回现有任务；不同源可并行/排队 | 同上 |
| `main.rs:876-998` | `read_core_stream` 新 `Method` 与事件白名单；4 类事件转发；进度快照结构 | 同上 |
| `main.rs:450-534` | 新增 `set_source_update_interval`、`project_instances` 一次性命令 | 同上 |
| `main.rs:1518-1560` | `setup` 启动到期调度线程（见 §5.8）；`invoke_handler` 更新 | 同上 |
| `main.rs` 测试段（`:1565-2706`） | 方法清单/请求构造/流测试/夹具路径更新 | 13 |

### 5.6 Windows TS

| 文件 | 改什么 | 顺序 |
| --- | --- | --- |
| `apps/windows/src/sources.ts` | `:30-42` 类型（删 `ScanSourcesResult/SourceScanStatus/SourceEvent` 旧联合，加 `GameMatch/SourceUpdateResult/SourceTaskStatus/4 事件`）；`:49-98` 状态 tone/chip/label；`:105-157` 行摘要；`:230-232` 角色 | 13 后 |
| `apps/windows/src/SourcePanel.tsx` | `:5-24` props；`:32-51` SourceCard 增手动更新/间隔/最近成功/最近尝试/phase 进度；`:85-90` 替换整源刷新；`:104-127` 游戏节加“重新匹配”/覆盖/HGamer 状态 | 同上 |
| `apps/windows/src/main.tsx` | `:32` 不扩 `CoreOperation`；`:153-155` 能力判定改用新方法（如 `update_source`）；`:176-233` 多任务事件处理（4 类、按 request/source 键控）；删 `handleSourceEvent` 中整库 scan（`:199`）；`:250-295` 改 update/match 任务启动/取消；`:297-307,343,411-416,545` 删旧调用/自动扫描，加“重新匹配”；`:518` 进度按任务 | 同上 |

### 5.7 packages/ui

`packages/ui/src/index.ts`：更新 `installSummary`（`:43-54`）、reason 映射（来源状态段 `:94-112`）、来源状态 helper（`:357-380`）、新增间隔/调度/fallback 显示函数；`displayPatches`（`:14-38`）按 v5 `discovery` 六态核对。顺序：wire 生成物生成后（6）即可改，早于两端布局（避免两端各写一份）。

### 5.8 调度实现落点（P4 核心新增）

- **【事实】** 契约 §10 规定 `next_due_at = status.checked_at + interval_hours*3600`，且 `list_sources` 汇总返回 `interval_hours/next_due_at`（`p1-contract.md` §4、§10）；核心已有纯函数 `crates/core/src/sources/schedule.rs:12-30`。
- **【建议】Decky**：`main.py._initialize` 在 `sources` 初始化与 `recover` 之后（`:61,67`）启动一个 `asyncio` 循环任务：立即检查一次，之后每 60s 调 `list_sources`，对 `role=publisher && enabled && interval_hours>0 && next_due_at<=now` 且无在飞任务的源发起 `update_source`（同源去重）。`_unload`（`:331-338`）取消该任务；不新增系统服务、不持久化计时器。Steam Deck 休眠恢复没有插件事件，下一 tick 的“到期检查”即自然满足“恢复检查一次、每源补一次、不补跑周期”。
- **【建议】Windows**：`main.rs.setup`（`:1518-1541`）启动一个线程循环（同样 60s），通过一次性 `serve` 调 `list_sources`、通过 `serve-task` 启动到期源更新；响应 `Closing`（`:1523-1537`）退出。不要在 React 侧做调度（窗口隐藏时仍需运行）。
- **【建议】** 到期选择写成纯函数（输入 `SourceSummary[]` + `now`，输出 `source_id[]`），Decky Python / Windows Rust 各自单测，不启动网络。

### 5.9 依赖顺序汇总

```
P1/P2/P3 core 内部能力（不升 wire、不删旧方法）
  → [wire switch] 5.2 protocol+CLI+contract-gen+生成物
  → 6 生成
  → 5.7 packages/ui
  → 5.3 Decky Python / 5.5 Windows Rust（可并行，各自先 RPC 层）
  → 5.4 Decky TS / 5.6 Windows TS（依赖各自宿主 RPC 与 ui 文案）
  → 5.8 调度（依赖 list_sources 新字段与 update_source）
  → 测试全绿（§6）
  → P5 删除 ScanSession/旧 RPC/旧测试/旧文档
```
同一 switch 提交内若必须分步提交，至少保证每个提交 `cargo check --workspace` + 双端 typecheck 可过（宿主不得调用已从白名单删除的方法）。

## 6. 测试落点

### 6.1 现有测试必须更新/删除

| 测试 | 现状 | P4 动作 |
| --- | --- | --- |
| `apps/decky/backend/test_bridge.py:61-110` | 消费生成的 `requests/responses/events/sequences` | v5 fixtures 自动扩展后应通过；新增方法/事件类型覆盖靠 fixtures，不手写第二份表 |
| `apps/decky/backend/test_v4_bridge.py:193-201`（版本门在 `test_bridge.py:121-143`） | 已删方法列表、协议版本 | 更名/更新为 v5；删除方法列表改为 `refresh_sources/scan_sources/discover_game` |
| `apps/decky/backend/test_scan_projection.py:24-61` | `_project_row`/`_read_report`/`_refresh_rows` | `_refresh_rows` 用例删除；`project_instances` 单行投影替代；`_read_report` 保留 |
| `apps/decky/backend/test_discovery_rpc.py:84-111` | `start_game_discovery`/`cancel_source_query`/`scan_status` | 替换为 `rematch_game`/`match_games`/任务状态/取消用例 |
| `apps/decky/backend/test_scan_session.py` | 整文件测 `ScanSession` | 删除（P5 或切换提交） |
| `apps/decky/backend/test_source_service.py:141-186` | `refresh`/`scan_sources`/`discover_game` | 替换为新方法 + 同源去重 + 事件映射 + settings 重载 |
| `apps/decky/backend/test_settings.py` | 插件 auto_cleanup | 不改 |
| `apps/decky/tests/session.test.ts:125-372` | discovery/EXE 动态选择/`scan_status` 轮询 | 保留安装相关；discovery 用例改 match/rematch 与任务快照；删除 `scan_status` 轮询用例 |
| `apps/decky/tests/discovery.test.ts`、`install.test.ts` | 旧发现状态/安装摘要 | 更新状态文案与筛选语义 |
| `apps/windows/scripts/test-game-status.mjs:51,65` | fixtures 路径与 v4 文案 | v5 路径；更新 `patchChip` 状态；补间隔/覆盖文案用例 |
| `apps/windows/src-tauri/src/main.rs:1574,1931,1941,2034,2293,2188` | 旧方法清单/请求构造/流/夹具路径 | 更新为 5 新方法 + 4 事件 + v5 夹具路径 |
| `crates/protocol/src/fixtures.rs` | v4 全量夹具 | 按 v5 重写每方法/事件/非法样本；旧方法请求作为未知方法样本 |
| `crates/cli/src/runtime.rs:1229-1330,1559-1576` 等 | `discover_game` 缓存/`scan_sources` 流测试 | 移除/替换 |
| `crates/cli/src/main.rs:525-543,577-645` 等 | v4 信封/任务方法拒绝/序列 | 更新版本与任务方法清单 |

### 6.2 需要新增的测试

1. **Decky `test_source_tasks.py`**：`update_source` 结果映射；同源重复返回现有任务；不同源可同时运行；取消写 `cancel` 且保留旧快照；`source_progress` 四 phase 映射；`match_progress/match_update` 行合并；`project_instances` **不调用** `bridge.scan`（断言 `scan` 调用次数）。
2. **Decky `test_scheduler.py`**：假时钟下到期一次、不补跑、禁用/fallback 源排除、失败后下次到期从 `checked_at` 起算、间隔 0 关闭、启动检查一次。
3. **Windows Rust 纯函数测试**：到期选择、`build_params`（5 方法）、新流解析（4 事件/非法事件/序列）、单槽改多任务后的同源去重、`match_update` 不触发整库 scan（前端测试无法覆盖 Rust，但可测事件转发形状）。
4. **Windows `test-game-status.mjs`**：新 chip/label、间隔预设、HGamer disabled/未查询文案、`sourceCoverage` 文案。
5. **`packages/ui` 共享测试**（由 `npm run test:contracts` 或新 test）：`installSummary` 六态、fallback 展示规则（design §3.2 五条）、间隔 label。
6. **协议/生成器**：`SOURCE_VERSIONS` 与 `SourceCapabilities` 一致性、v5 capabilities 门、旧方法请求拒绝、4 类事件 shape/phase 枚举。

### 6.3 可运行命令（Windows 主机）

Rust 工具链不在 PATH，先设置隔离环境（与 `p0-baseline.md` §1.2 相同）：

```powershell
$root = 'D:\Project Files\git\steam-hentai-patch-searcher'
$env:CARGO_HOME = "$root\.artifacts\cargo-isolated\cargo"
$env:RUSTUP_HOME = "$root\.artifacts\cargo-isolated\rustup"
$env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
Set-Location $root
```

```powershell
# 契约与 Rust
cargo run --locked -p steam-patch-contract-gen -- check        # 生成物无漂移
cargo run --locked -p steam-patch-contract-gen -- generate     # bump 后重生成
cargo test --locked -p steam-patch-protocol
cargo test --locked -p steam-patch-windows                     # Tauri 宿主测试（含 fixtures）
cargo test --locked -p steam-patch-core -p steam-patch-cli
cargo check --locked --workspace
# 可选：cargo clippy --locked -p steam-patch-core -p steam-patch-cli -p steam-patch-protocol -p steam-patch-contract-gen --all-targets -- -D warnings

# Node/TS（仓库根 workspaces）
npm run test:contracts          # scripts/test-ui-contract.mjs
npm run typecheck               # windows + decky
npm test -w apps/decky          # node --experimental-strip-types --test tests/*.test.ts
npm run test:status -w apps/windows
npm run build:decky
npm run build:windows

# Decky Python（从仓库根运行；discover 会把 backend 加入 sys.path，测试用裸模块名导入）
python -m unittest discover -s apps/decky/backend -p "test_*.py"

# 打包/安装脚本回归
python -m unittest discover -s scripts -p "test_package_artifacts.py"
python -m unittest discover -s scripts -p "test_install_decky.py"

# Windows 便携包（可选，需 sidecar 构建）
npm run bundle -w apps/windows   # 内部 prepare-sidecar 调 rustc/cargo，需沿用上面的 PATH
```

- **【事实】** Decky `package:plugin` 需要 `target/linux/release/{steam-patch,sf-worker}` ELF（`package-plugin.mjs:10-14`），Windows 主机不能生成，需 Deck/CI 或 Linux 构建后执行。
- **【事实】** Windows `prepare-sidecar.mjs:8` 直接调 `rustc --print host-tuple`，必须让隔离工具链的 `bin` 在 PATH 中。
- **【事实】** `npm run build:windows` = `tsc --noEmit && vite build`（`apps/windows/package.json:8`），不含 Tauri 打包；`npm run bundle` 才执行 `prepare:sidecar + tauri build`（`:11-12`）。

## 7. 风险与最小改动建议

### 7.1 P4 同时触碰的共享文件（避免冲突）

| 文件 | 冲突源 | 建议 owner/顺序 |
| --- | --- | --- |
| `crates/protocol/src/lib.rs`、`fixtures.rs` | 所有方法的 wire 定义 | 单一 owner 一次改完；其他人只读 |
| `crates/contract-gen/src/main.rs` + `packages/contracts/generated/*` | 生成物不可手改、字节级校验 | 生成器改完由同一人 `generate`；禁止并行编辑生成物 |
| `crates/cli/src/runtime.rs` | P2/P3 新增实现函数 vs P4 方法表/分派 | 约定：P2/P3 只加 `*_impl` 函数与 core 层；P4 owner 改编 `METHODS`/`dispatch`/`capabilities` |
| `apps/decky/backend/bridge.py`、`protocol_validation.py` | 传输层，多方法共用 | 单一 owner；事件闸门与生成物同步 |
| `apps/decky/main.py`、`source_service.py` | 所有新 RPC 与调度 | Decky Python owner 一人；被删的 `scan_session.py` 不要与他人交叉编辑 |
| `apps/windows/src-tauri/src/main.rs` | 命令/槽位/流解析集中 | Windows Rust owner 一人；该文件很大，非必要不并行 |
| `apps/windows/src/{main.tsx,SourcePanel.tsx,sources.ts}` | 来源 UI 与事件处理互相引用 | Windows TS owner 一人；先改 `sources.ts` 类型再改组件 |
| `apps/decky/src/{index.tsx,session.ts,discovery.ts}` | 会话与来源页互相引用 | Decky TS owner 一人 |
| `packages/ui/src/index.ts` | 双端共享，任何文案/类型变化都影响两端测试 | 先于两端布局改动，单 owner |
| `scripts/{package-artifacts,install-decky,test-ui-contract,test-install-decky-bridge}` | 版本/文件清单 | 与生成物同批改 |
| `docs/features/README.md`、本功能 `progress.md` | AGENTS 规则要求索引/施工记录 | Lead/文档 owner 在 P4 完成后统一更新 |

### 7.2 可复用、无需新写的机制

- **JSONL 任务通道/取消/EOF**：`CoreBridge._call`（`bridge.py:156-369`）与 CLI `serve-task`（`crates/cli/src/main.rs:108-214`）；Windows 侧 `read_core_stream`/`ClosableWriter`/`cancel_and_close`（`main.rs:876-920,144-158,710-716`）。
- **事件 `sequence` 语义**：`EventSequence`（`crates/protocol/src/lib.rs:1102-1137`）；CLI 统一注入 `protocol_version/request_id/sequence`（`main.rs:185-205`）。
- **进度快照/取消轮询**：Windows `install_status` 模式（`main.rs:1243-1259`）；Decky `scan_status` 的 generation 栅栏（`main.py:135-141`）可平移为任务快照。
- **来源摘要缓存**：`SourceService._apply_summaries/_reload_snapshot`（`source_service.py:54-63,86-93`）。
- **设置持久化**：核心 `settings.json`（`model.rs:147-157`；`store.rs` 当前 P1 修改）与 `write_atomic`；宿主不要自建间隔存储。
- **到期计算**：核心 `schedule.rs:12-30` + `list_sources.next_due_at`；宿主只做 `now >= next_due_at` 比较与去重。
- **UI 文案/判定**：`packages/ui` 的 `reason()`、`installSummary()`、`sourceRefresh*`；P4 扩展而非复制。

### 7.3 风险清单

| 风险 | 证据 | 缓解 |
| --- | --- | --- |
| 半套 RPC（核心 v5 + 宿主 v4） | `PROTOCOL_VERSION` 硬校验（`crates/protocol/src/lib.rs:261,604`；`protocol_validation.py:149,160,174`） | §5.1 单一切换提交；CI 同时跑 contract-gen check + 双端 typecheck/test |
| 生成器手写常量漂移 | `contract-gen/main.rs:88,96-102` 与协议类型分离 | 同批更新 + 一致性测试；`check` 作为门 |
| v4 文件名散落 | §0-7 的 10 个文件中十余处引用 | 统一改名并 grep `v4.schema` 与 `fixtures/v4`；或明确保留并记录理由 |
| Decky 包内协议旧 | `package-plugin.mjs:23-26`；`protocol_validation.py:10-16` 启动即加载 | 重新生成 + 重打包 + `package-artifacts.py` 校验 |
| Windows 单槽阻塞/取消他任务 | `main.rs:570-572,1362-1365`；`main.py:169-170` | P4 改按任务/来源键控；同源去重返回现有任务，不同源可并发/排队 |
| 事件形状变更被旧前端静默丢弃 | Windows 旧分支 `main.tsx:194`；Decky 闸门 `bridge.py:251` | 同批改监听/闸门；把“事件未处理”纳入事件测试 |
| `scan.sources` 缺新字段导致来源页无间隔 | `ScanReport.sources` 复用 `SourceSummary`（`projection.rs:146`）；Decky 来源页读 `report.sources`（`index.tsx:513`） | 来源管理页显式调 `list_sources`；或让 `SourceSummary` 同步扩展（需契约确认） |
| 删除 `scan_session.py` 破坏打包白名单 | `package-artifacts.py:17`；`package-plugin.mjs:23` | P5 删除时同步脚本与测试 |
| 自动调度补跑风暴/立即重试 | 契约 §10 | 只消费 `next_due_at`；每源每轮最多一次；失败后从 `checked_at` 重算；不保存错过周期 |
| 并发实现者改同一文件 | 见 §7.1 | 按 owner 切分；P4 不动 `crates/core/src/sources/*`（P1 正在改） |

### 7.4 最小改动建议（按优先级）

1. **先落 wire v5 + 宿主 RPC，不动调度**：保证 5 方法/4 事件、`project_instances` 取代整库 `scan`、删除旧调用；此步完成后 `scan` 固定 1 次、单行投影 0 次，满足 P4 门槛的主要部分。
2. **再落 UI**：`packages/ui` 文案 → Decky `SourcesPage`/`GameManager` → Windows `SourcePanel`/详情；先手动更新/重新匹配，再加间隔与最近成功/尝试/phase 进度。
3. **最后落调度**：两端各一个循环 + 纯函数到期选择 + 假时钟测试；不新建服务/数据库/持久计时器。
4. **P5 只删除**：`ScanSession`、旧 RPC/命令/旧事件变体、旧测试与文档引用；不要与调度/UI 行为改动混在同一提交。
