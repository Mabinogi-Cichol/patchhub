# Alpha 数据契约

本文件是**跨端共享协议参考**（core / Windows 宿主 / Decky 桥接共用），冻结 `protocol_version = 3` 的传输、方法白名单、事件与终态形状。**不支持 v2**：`protocol_version` 为 1 或 2 的请求一律以 `unsupported_protocol` 拒绝，不设兼容路由。

权威契约与迁移说明见 [p1-host-contract.md](../../docs/features/cross-platform-installer-migration/p1-host-contract.md)；本文件只保留宿主实现所需的稳定摘要，字段以代码为准（core 的 `catalog` / `trust` / `install` 与 `crates/cli/src/runtime.rs`）。

## 传输与信封

- stdin/stdout **单行 JSON-lines**，一行一个对象；单行错误不终止后续请求。请求行 ≤64 KiB（超限 `request_too_large`），宿主响应/事件行 ≤8 MiB。
- 请求：`{protocol_version, request_id, method, params}`，`deny_unknown_fields`；`request_id` 1–128 字节、非空、无控制字符。
- 响应成功：`{protocol_version, request_id, ok:true, result}`；失败：`{protocol_version, request_id, ok:false, error:{code, detail?}}`。`detail` 可选、≤2 KiB、截断后的底层诊断；无法解析的输入 `request_id` 为 null。
- 宿主固定注入 `roots`/`catalog`/`signed_catalog`/`trust_keys`/`state_dir`/`platform`；UI 只传实例、选择与受限操作意图，不传配方/目标路径/任意 URL。

## 执行面

- `serve`：一次性短调用（扫描、能力、来源读写、缓存清理）。
- `serve-task`：单任务、可取消、可流式（`install_patch`、`scan_sources`、`refresh_sources`），一个任务一个子进程。任务面方法经 `serve` 调用一律拒绝并返回 `task_required`。
- 任务控制通道（非 JSON-RPC 方法）：`serve-task` stdin 行 `renew`（续 15s 前台租约）、`cancel`（先持久撤销再停获取/worker）；stdin EOF 等价 `cancel` 并回收 worker 进程组。

## 方法白名单（16）

与 `get_capabilities.methods` 精确一致：

- 安装器/任务面（8）：`get_capabilities`、`scan`、`preview`、`install_patch`、`list_transactions`、`recover`、`discard_plan`、`cleanup_cache`。
- 来源面（8）：`list_sources`、`list_game_sources`、`set_source_enabled`、`set_game_source`、`add_source`、`refresh_sources`、`scan_sources`、`discover_game`。

`get_capabilities` 是唯一能力协商入口，返回 `installer`、`sources` 子对象与合并 `methods`。`install_patch` 为唯一安装入口（仅 `serve-task`）：`{instance_id, selection_id? | release_id?, operation:"install"|"reinstall", request_id, archive?}`；`selection_id` 与 `release_id` 互斥（同给 `invalid_request`，皆缺 `selection_or_release_required`）。后端按 `release_id`（签名，优先）/ `zip-selection-*`（动态 ZIP）/ `exe-selection-*`（动态 EXE）路由。

`scan.result` 每行含 `game`、`match_status`、`patches`，`patches[].download_available` 为后端判定的"可进入安装"标记（签名目录声明或动态来源解析且无阻断，均由核心判定）；本机链路可用性另见 `installer.links.*.available`；`preview.result` 只读，`executable` 恒为 `false`。旧方法 `plan_install`/`start_install`/`fetch_artifact`/`migrate_state`/`get_source_state`/`get_source_settings`/`list_source_matches`/`get_source_policy`/`resolve_source_record` 已删除，调用返回 `unsupported_method`。

## 事件：`install_progress`

字段**平铺**在事件对象顶层（不再有嵌套 `progress`），`(request_id, sequence)` 用于去重，`sequence` 从 0 起严格递增、每任务独立：

`event`=`"install_progress"`、`protocol_version`、`request_id`、`sequence`、`instance_id`、`selection_id?`、`stage`、`substage?`、`bytes_done`、`total_bytes?`、`files_done`、`total_files?`、`current_file?`、`archive_bytes?`。

- `stage`（9）：`connecting`、`downloading`、`validating`、`analyzing_installer`、`extracting`、`planning`、`applying`、`rolling_back`、`cleanup`。内部子状态 `backing_up`/`writing`/`committing` 折叠为 `applying`，仅以 `substage` 诊断。
- `bytes_done` 随阶段可重置为 0；`total_bytes`/`total_files` 未知为 `null`，不伪造百分比。
- 来源事件 `source_progress`/`source_update` 沿用现状，仅 `protocol_version` 升 3、`sequence` 语义一致。

## 终态：`InstallOutcome`

`install_patch` 的唯一最终响应 `result`（终态只写一行，其后不再出现事件）：

```json
{
  "transaction_id": "...", "plan_id": "...", "instance_id": "...",
  "selection_id": "...", "content_id": null, "release_id": "...",
  "state": "committed", "error": null,
  "created_at": 0, "updated_at": 0, "has_replacements": false,
  "recovery": null, "cleanup": { "performed": true }
}
```

- `state`：`committed` | `superseded` | `rolled_back` | `needs_recovery`（`uninstalled` 已删除）。
- `content_id` **可选**，仅 EXE 动态路由产生；签名 ZIP 路由的终态身份为 `release_id`，宿主不得等待 `content_id` 非空。
- `needs_recovery` 显式上报（`recovery:{required,code?,detail?}` 且 `required=true`），不伪装 `cancelled`/`completed`；恢复/清理诊断放 `recovery`/`cleanup`（`{performed,removed_bytes?,code?,detail?}`）。
- 计划建立前的失败（archive 校验、下载、分析、转换、计划）不返回 `InstallOutcome`，而是 `ok:false + error{code, detail?}`；下载期取消 `error.code="cancelled"`，不产生事务。

## 能力与错误

- `get_capabilities.installer.links.*.available`（按平台探针）决定是否可安装；`formats` 仅表示核心支持该类格式，不代表本机可用。隔离缺失只阻断对应 EXE，来源页与签名 ZIP 保留。
- 新增/冻结错误码：`unsupported_protocol`、`capability_mismatch`、`installer_schema_unsupported`、`installer_lease_expired`、`cleanup_failed`、`task_required`。契约不匹配明确报错并保留真实 `detail`，不做“仅静态扫描”降级。

## 目录与信任

目录模型与 schema 位于 `catalog-payload.schema.json`；权威 Rust 类型在 core 的 `catalog` / `trust` / `install` 与 `crates/cli/src/runtime.rs`。签名封装含 `schema_version`、`key_id`、`payload`（base64 原始 JSON）、`signature`（base64 Ed25519）；payload 含 `catalog`、`generated_at`、`expires_at`、`revoked_release_ids`。

目录记账使用 SQLite 防回退，同一写事务重新检查 revision。扫描失败时不能将 unknown 解释成未安装。语言按目录声明与审核配方选择，不能只根据游戏名称推断。
