# v5 共享数据契约

本文件是**跨端共享协议参考**（core / Windows 宿主 / Decky 桥接共用），冻结 `protocol_version = 5` 的传输、方法白名单、事件与终态形状。**仅支持 v5**：旧版本或未来版本 的请求一律以 `unsupported_protocol` 拒绝，不设兼容路由。

权威契约见 [P1 契约](../../docs/features/source-cache-matching-refactor/p1-contract.md) 与 [P4 宿主落点](../../docs/features/source-cache-matching-refactor/p4-host-map.md)，唯一 wire 类型与校验位于 `crates/protocol`。`crates/contract-gen` 由 Rust 类型确定性生成 [JSON Schema](generated/v5.schema.json)、[TypeScript](generated/protocol.ts) 与 [Python 纯数据](generated/protocol.py)，双端共用 [夹具](fixtures/v5/cases.json)。

生成：`cargo run --locked -p steam-patch-contract-gen -- generate`；检查：`cargo run --locked -p steam-patch-contract-gen -- check`。生成器不打入插件，core 不依赖 wire 协议。

## 传输与信封

- stdin/stdout **单行 JSON-lines**，一行一个对象；单行错误不终止后续请求。包含行末一个 LF，不设固定整行大小上限。
- 请求：`{protocol_version, request_id, method, params}`，`deny_unknown_fields`；`request_id` 1–128 字节、非空、无控制字符；Steam instance_id 保留完整路径身份，单独上限 16 KiB，其余业务 ID 256 字节。
- 响应成功：`{protocol_version, request_id, ok:true, result}`；失败：`{protocol_version, request_id, ok:false, error:{code, detail?}}`。`detail` 可选、完整不截断的底层诊断；无法解析的输入 `request_id` 为 null。
- 宿主固定注入必需的 `params.host`，包含 `roots`/`catalog`/`signed_catalog`/`trust_keys`/`state_dir`/`platform` 六字段；可空路径也必须显式给出 null；UI 只传实例、选择与受限操作意图，不传配方/目标路径/任意 URL。

## 执行面

- `serve`：一次性短调用（扫描/投影、能力、来源读写、缓存清理）。
- `serve-task`：单任务、可取消、可流式（`install_patch`、`update_source`、`match_games`、`rematch_game`），一个任务一个子进程。任务面方法经 `serve` 调用一律拒绝并返回 `task_required`。
- 任务控制通道（非 JSON-RPC 方法）：`serve-task` stdin 行 `cancel`（先持久撤销再停获取/worker）；stdin EOF 等价 `cancel` 并回收 worker 进程组。

## 方法白名单（21）

与 `get_capabilities.methods` 精确一致：

- 安装器/只读面（12）：`get_capabilities`、`scan`、`project_instances`、`preview`、`install_patch`、`list_transactions`、`recover`、`discard_plan`、`cleanup_cache`、`cache_status`、`backup_status`、`cleanup_backups`。
- 来源面（9）：`list_sources`、`list_game_sources`、`set_source_enabled`、`set_game_source`、`add_source`、`set_source_update_interval`、`update_source`、`match_games`、`rematch_game`。

旧 `refresh_sources`/`scan_sources`/`discover_game` 已删除，调用返回 `unsupported_method`；更早的 `plan_install` 等已删除方法同样如此。

`get_capabilities` 是唯一能力协商入口，返回 `installer`、`sources` 子对象与合并 `methods`。`install_patch` 为唯一安装入口（仅 `serve-task`）：`{instance_id, selection_id? | release_id?, operation:"install"|"reinstall"|"overwrite", archive?, cleanup_policy:"disabled"|"after_commit"}`；operation 和 cleanup_policy 均必需，幂等 ID 仅使用外层 request_id。`selection_id` 与 `release_id` 互斥（同给 `invalid_request`，皆缺 `selection_or_release_required`）。后端按 `release_id`（签名，优先）/ `zip-selection-*`（动态 ZIP）/ `exe-selection-*`（动态 EXE）路由。

来源面参数与结果：

- `project_instances {instance_ids: string[]}` → `{settings_revision: number|null, instances: InstanceProjection[]}`（只投影请求的实例，缺失实例 `instance_not_found`）。
- `set_source_update_interval {source_id, interval_hours}`（`0`=关闭）→ `{settings_revision, source_id, interval_hours}`。
- `update_source {source_id, force?}`（仅 `serve-task`）→ `SourceUpdateResult{source_id, settings_revision, status: SourceStatus, revision, complete, affected_appids}`。
- `match_games {instances?, name_hints?, preferred_language?, force_details?}`（仅 `serve-task`；instances 缺省=当前库全部 installed）→ `{settings_revision, source_revisions, results: GameMatch[]}`。
- `rematch_game {instance_id?|appid?, name_hints?, preferred_language?}`（仅 `serve-task`，二者至少一个）→ 同上；未安装游戏可用 appid+空名。
- `GameMatch{appid, instance_id?, status, candidates, diagnostics, source_revisions}`，status 六态：`pending|publisher_match|fallback_match|no_match|incomplete|failed`。
- `list_sources` 的 `SourceSummary` 新增 `revision`、`complete`、`interval_hours`、`next_due_at?`；到期时间只由 `checked_at + interval_hours*3600` 派生，不单独持久化。

`scan.result` 每行含 `game`、`match_status`、`patches`、`discovery`，`patches[].actions` 是必需动作 DTO，包含 automatic/local_archive/reinstall/overwrite 的可用性与结构化阻断、primary 的 operation/label/input；双端直接消费，不保留 download_available 或 UI 派生权限字段。`scan` 仍是整库只读投影；`discovery` 来自持久化匹配记录（无记录=`pending` 空候选），本调用不联网、不逐游戏发现。`preview.result` 只读，`executable` 恒为 `false`。

## 事件

四类事件均为平铺 JSONL 任务通道事件，`(request_id, sequence)` 用于去重，`sequence` 从 0 起严格递增、每任务独立；宿主只发 payload，由 `serve-task` 外层注入 `protocol_version`/`request_id`/`sequence`。

- `install_progress`：`instance_id`、`selection_id?`、`stage`、`substage?`、`bytes_done`、`total_bytes?`、`files_done`、`total_files?`、`current_file?`、`archive_bytes?`。`stage`（9）：`connecting`、`downloading`、`validating`、`analyzing_installer`、`extracting`、`planning`、`applying`、`rolling_back`、`cleanup`。内部子状态 `backing_up`/`writing`/`committing` 折叠为 `applying`，仅以 `substage` 诊断；`bytes_done` 随阶段可重置为 0，未知总量为 `null`。
- `source_progress`：`source_id`、`phase:"index"|"details"|"matched"`、`completed`、`total`、`running`、`settings_revision`。索引更新与详情/匹配阶段明确区分，界面不得把整条任务宣称为纯离线。
- `source_update`：`source_id`、`status: SourceStatus`、`settings_revision`；`update_source` 终态回显。
- `match_progress`：`phase:"inventory"|"index"|"details"|"fallback"`、`completed`、`total`、`running`、`settings_revision`。
- `match_update`：`instance_id?`、`appid`、`status`（`GameMatch` 六态）、`candidates`、`diagnostics`、`settings_revision`。

## 终态：`InstallerOutcome`

新安装终态保留原事务字段，并包含 `disposition`（completed/cancelled/failed）、`cause`、`recovery` 和 `cleanup`。`state` 仅 committed/rolled_back/needs_recovery。宿主用 disposition 决定提示，取消只在回滚完成且无恢复需求时成立。历史 Transaction 保留原 error 字符串，不反向推导新终态；同一外层 request_id 的重试返回稳定结果。

计划建立前失败返回 `ok:false + error{code,detail?}`。DomainError 代码为开放字符串，宿主保留未知合法代码，不能把 detail 或历史错误文本解释成权限。

## 能力与错误

- `get_capabilities.sources`（v5）：`index_schema_version`、`detail_schema_version`、`match_schema_version`、`schedule_schema_version`、`adapter_version`、`parser_version`、`encoding_version`（除 `parser_version=2` 外均为 1）与 9 个来源方法；`validate_capabilities` 要求版本与 methods 精确一致。生命周期写入前校验版本、methods、四项 contracts 版本及全部 installer/source 版本；不匹配必须先于状态目录创建或修改。
- `get_capabilities.installer.links.*.available`（按平台探针）决定是否可安装；`formats` 仅表示核心支持该类格式，不代表本机可用。隔离缺失只阻断对应 EXE，来源页与签名 ZIP 保留。
- 新增/冻结错误码：`unsupported_protocol`、`capability_mismatch`、`installer_schema_unsupported`、`cleanup_failed`、`task_required`、`unknown_source`、`source_not_indexable`、`instance_required`、`instance_not_found`。契约不匹配明确报错并保留真实 `detail`，不做“仅静态扫描”降级。

## 目录与信任

目录模型与 schema 位于 `catalog-payload.schema.json`；权威 Rust 类型在 core 的 `catalog` / `trust` / `install` 与 `crates/cli/src/runtime.rs`。签名封装含 `schema_version`、`key_id`、`payload`（base64 原始 JSON）、`signature`（base64 Ed25519）；payload 含 `catalog`、`generated_at`、`expires_at`、`revoked_release_ids`。

目录记账使用 SQLite 防回退，同一写事务重新检查 revision。扫描失败时不能将 unknown 解释成未安装。语言按目录声明与审核配方选择，不能只根据游戏名称推断。
