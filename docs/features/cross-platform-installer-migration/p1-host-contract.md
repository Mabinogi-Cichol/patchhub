# P1 宿主契约冻结稿（protocol_version=3）

日期：2026-10-08（Asia/Shanghai）。状态：**v1.2，已按 P1-0 初审与定向复审（attempt 2/3）修订（含 R1-R3 折入，2026-10-08）；可作为 P1 实现基线；不代表已实现**。**增量：已按 [p1-zip-orchestration.md](p1-zip-orchestration.md)（P1-3b-3 设计，2026-10-08）回写动态 publisher ZIP 契约（§1.1、§2.2、§2.3、§5.1、§7.1、§8）。** 基准为 [design.md](design.md) §6（CLI、事件与任务生命周期）、§5.2（授权）、§7.3（清理清单）、[p0-contracts.md](p0-contracts.md) §0/§6/§10/§11。本稿取代 p0-contracts §11 的“宿主 RPC/事件/终态延后”待冻结项。开发阶段接受 breaking change，不设计 v2 兼容路由。

> 决策格式：【冻结值 / 理由 / 被否替代或删除项 / 迁移动作 / 验证方式】。
> 标注：**待实现**（P1 代码）、**待实测**（需设备/样本证据）、**P4/P5 宿主**（随宿主更新，不在 P1 改 Rust 协议）。
> 归属：P1 冻结并实现 Rust 协议服务侧；P4 Windows 宿主；P5 Decky 桥接；P6 收口删除残留。

---

## 0. 全局基线与归属

### 0.1 冻结值

- 协议版本 `protocol_version = 3`（由现行 2 升级，p0-contracts §0）。核心与两端宿主同版本发布，**旧版本明确拒绝**（`unsupported_protocol`），不设兼容路由（design §6）。
- 传输：stdin/stdout **单行 JSON-lines**；一行一个对象；单行错误不终止后续请求（保持 `serve`/`serve-task` 两模式）。
- 两个执行面：
  - `serve`：一次性短调用（扫描、来源读写、能力、缓存清理）；**不含任务面方法**——任务面方法（`install_patch`）仅 `serve-task` 提供，经 `serve` 调用一律拒绝并返回 `task_required`（F2）。
  - `serve-task`：单任务、可取消、可流式（`install_patch`、`scan_sources`、`refresh_sources`）。**一个任务一个子进程**，取消/租约/EOF 走该进程 stdin 控制通道。
  - `discover_game` 为 `serve` **一次性方法（非流式）**，不得列入 `serve-task`（MF6；对齐现行 `runtime.rs` `SOURCE_STREAM_METHODS` 仅含 `scan_sources`/`refresh_sources`）。
- 请求信封：`{protocol_version, request_id, method, params}`，`deny_unknown_fields`；`request_id` 1–128 字节、非空、无控制字符。
- 响应信封：成功 `{protocol_version, request_id, ok:true, result}`；失败 `{protocol_version, request_id, ok:false, error:{code, detail?}}`（`detail` 为**新增**，见 §3.3）。
- 宿主固定注入参数（UI 永不提供）：`roots`、`catalog`、`signed_catalog`、`trust_keys`、`state_dir`、`platform`。UI 侧只传实例、补丁/选择与受限操作意图（design §6、§2.1）。

### 0.2 理由

design §6 要求“核心与两端宿主一起更新，旧版本明确拒绝”；p0-contracts §0 已冻结版本号；p0-contracts §11 明确宿主契约延后到 P1、开始实现前先冻结。

### 0.3 被否替代或删除项

- 保留 v2 兼容解析/双版本路由（用户明确不做兼容）。
- 引入常驻 daemon / socket RPC：现行“一任务一子进程 + stdin 控制”已能表达租约、取消、EOF 与进程树回收（p0-contracts §5.2），新增 daemon 会扩大攻击面且无收益。

### 0.4 迁移动作

- **P1**：`crates/cli/src/main.rs` `PROTOCOL_VERSION=3`；信封/响应形状按本节；`runtime.rs::METHODS` 收敛为 §1 白名单（见 §1.3）。
- **P4/P5**：两端宿主的 `protocol_version` 常量、响应校验同步为 3。
- **发布时序（N5）**：v3 核心与两端宿主**按 P4（Windows）/P5（Decky）同包发布**；因旧版本被明确拒绝（`unsupported_protocol`），P1–P3 期间的 v3 核心**不面向 Decky 生产用户发布**（不得把 v3 核心随旧 Decky 宿主包发布），避免中途整体不可用。Windows 端口在 P4 完成后可用，Decky 端口在 P5 完成后可用。

### 0.5 验证方式

契约测试：`protocol_version:2`/`1` → `unsupported_protocol`；未知字段 → `invalid_request`；未知方法 → `unsupported_method`；`request_id` 空/超长 → `invalid_request_id`。

---

## 1. 方法白名单与请求/响应形状（`protocol_version=3`）

### 1.1 冻结值 —— v3 方法白名单

**A. 安装器/任务面（P1 拥有）**

| 方法 | 模式 | 请求 `params`（UI 可见键） | 响应 `result` |
| --- | --- | --- | --- |
| `get_capabilities` | serve | `{}` | `Capabilities`（见 §3.1） |
| `scan` | serve | `{}`（roots/catalog 宿主注入） | `ScanResult`：`installs[].patches[]` 带 `selection_id`、`support_level`、`download_available`、`blockers`、`detection`；动态能力/阻断由核心判定 |
| `preview` | serve | `{instance_id, selection_id? \| release_id?}` | `Preview`：`{instance_id, selection_id, release_id?, executable:false, blockers, changes[]}`，**只读、不授权、不下载大型 EXE** |
| `install_patch` | serve-task | `{instance_id, selection_id? \| release_id?, operation:"install"\|"reinstall", request_id, archive?}` | `InstallOutcome`（见 §2.3），单任务：获取→分析→计划→apply |
| `list_transactions` | serve | `{}` | `[Transaction]`（新 schema，稳定 `instance_id`/`transaction_id`/`release_id`/`content_id` 归属） |
| `recover` | serve | `{}` | `[Transaction]` |
| `discard_plan` | serve | `{plan_id}` | `Transaction`（持久撤销，不删除恢复材料） |
| `cleanup_cache` | serve | `{}` | `CacheStatus`（新共享清单/清理；不删 active lease/receipt/journal/`.stage`/`.backup`/`revoked`） |

- `install_patch` 后端判别 **签名 release**（`catalog.release_id`）与 **动态选择路由**（`zip-selection-*` / `exe-selection-*`）；后端路由，不看前端自称（design §6、§5.1）。**判定顺序**：先校验 `selection_id`/`release_id` 互斥（同给 → `invalid_request`；皆缺 → `selection_or_release_required`）→ 有 `release_id` 走**签名路由（优先）** → 否则按 `selection_id` 前缀分派：`zip-selection-` → 动态 ZIP 编排（P1）；`exe-selection-` → P2/P3，P1 返回 `exe_route_unimplemented`；其他前缀/格式 → `invalid_selection_id`。任何候选/选择身份（`candidate-*`/`zip-selection-*` 等）不得冒充 `release_id`，`archive` **仍仅签名 `release_id` 路由**有效。
- **动态路由错误码（P1-3b-3）**：`invalid_selection_id`、`selection_not_found`、`selection_stale`、`zip_download_unavailable`、`zip_download_ambiguous`，以及 P1 临时占位 `exe_route_unimplemented`（P2/P3 EXE worker 落地后移除）。**`source_not_released` 已移除：动态 ZIP 暂无此门；候选日期字段补齐后另行引入（F1）。**
- **执行面门禁（F2）**：`install_patch` 为任务面方法，**仅经 `serve-task` 执行**；经 `serve` 调用一律拒绝并返回 `task_required`（不得用永真租约/永不置位的 cancel 执行）。
- `preview` 只接受候选/选择与 release；动态路径不返回可执行授权（`executable:false` 恒成立）。
- `install_patch`/`preview` 的 `selection_id` 与 `release_id` **互斥**（同时提供 → `invalid_request`）；二者皆缺 → `selection_or_release_required`（替换 v2 的 `release_required`，R3）。
- 保存的本地 ZIP 导入（`archive`）：**仅对签名 `release_id` 路由有效**；动态 selection（`zip-selection-*`/`exe-selection-*`）路由携带 `archive` 一律拒绝（`archive_not_allowed_for_selection`），防止绕过 design §2.1“动态安装只接受后端从核实来源解析的 URL、不新增任意 EXE 导入”。签名路由的 `archive` 仅允许宿主层校验后传入的**受控绝对路径**（Decky 限 `~/Downloads` 或 `state/downloads`；Windows 只接受 `.zip` 普通文件）。**UI 不传配方、不传目标路径、不传任意 URL**。
- `archive` **核心侧校验**（不信任宿主层，design §2.1/§4.1）：必须是**普通文件**（拒符号链接/硬链接，`no_links`）、扩展名与魔数为 **`.zip`**、大小为普通文件且不超过 publisher ZIP 冻结上限（design §8，包 ≤8 GiB；超限 → `archive_too_large`）。任一项不满足即拒绝，不进入分析/计划。

**B. 来源面（沿用 shared-source-discovery，收敛为有消费者者）**

| 方法 | 模式 | 请求 | 响应 | 消费者 |
| --- | --- | --- | --- | --- |
| `list_sources` | serve | `{}` | `[SourceSummary]`（含 `settings_revision`、`origin`） | 两端来源面板 |
| `list_game_sources` | serve | `{instance_id \| appid}` | `[GameSourceSubscription]` | 两端来源面板 |
| `set_source_enabled` | serve | `{source_id, enabled}` | `{settings_revision, source_id, enabled}` | 两端 |
| `set_game_source` | serve | `{appid \| instance_id, source_id, subscribed}` | `{settings_revision, appid}` | 两端 |
| `add_source` | serve | `{rule}` | `{settings_revision, rule}` | 两端 |
| `refresh_sources` | serve-task（流 `source_update`） | `{force}` | `{settings_revision, statuses}` | 两端 |
| `scan_sources` | serve-task（流 `source_progress`/`source_update`） | `{force}` | `{settings_revision, results}` | 两端 |
| `discover_game` | **serve**（一次性，非流式） | `{instance_id, name_hints?, preferred_language?, force?}` | `DiscoveryResult` | 两端 |

- `get_capabilities` 是**唯一合并入口**：其 `methods` 同时列出 A+B 全部 v3 方法（来源方法的权威定义仍属 shared-source-discovery，本文件只冻结“集合与命名”）。

**C. 任务控制通道（不是 JSON-RPC 方法）**

`serve-task` 进程 stdin 上的行控制（与事件共用一个 stdout 流的写回通道分离）：

| 控制行 | 语义 |
| --- | --- |
| `renew` | 续租；核心刷新前台租约 deadline（§5.2） |
| `cancel` | 请求取消：先持久撤销，再停获取/worker；写入阶段按 journal 恢复 |
| EOF（stdin 关闭） | 等价 `cancel`，并回收 worker 进程组 |

**D. 显式删除/替换的旧方法（对照现行 24 方法白名单）**

| 旧方法 | v3 处置 | 依据 |
| --- | --- | --- |
| `plan_install` | **删除**（内部编排，UI 不可见） | design §6 单任务 `install_patch`；UI 不传配方 |
| `start_install` | **删除**（并入 `install_patch`） | 同上 |
| `fetch_artifact` | **删除**（并入 `install_patch`） | 同上 |
| `migrate_state` | **删除**（旧状态迁移取消） | design §2.3/§7.1；p0-contracts §4.2 |
| `get_source_state` | **删除**（`import_report` 取消） | design §7.3 |
| `get_source_settings` | **删除**（导出/切换检查点取消） | design §7.3、shared-source-discovery p5 设计 |
| `list_source_matches` | **删除**（无 UI 消费者；**N3** 删除前复核 `source_service.py:18-19`，见 §1.3） | design §7.3 |
| `get_source_policy` | **删除**（仅测试/宿主内部旧读取，消费者迁移后无调用者；**N3** 删除前复核，见 §1.3） | design §7.3 |
| `resolve_source_record` | **删除**（旧安装消费者切换后无调用者；**N3** 删除前复核，见 §1.3） | design §7.3 |
| `install_patch` | 保留，语义扩展为单任务动态路由 | design §6 |
| `discard_plan`/`recover`/`list_transactions`/`scan`/`preview`/`get_capabilities` | 保留 | design §6 |

**E. 明确删除的旧字段**

- `Transaction.preconditions`（旧 journal 读取字段，新计划为空，p0-contracts §2；design 已声明不读旧 journal）。
- `Preview.executable` 恒 `false`，不再作为授权信号；动态路径授权只经 `install_patch` 内部 plan。
- `Capabilities.parser_version/query_version` 保留但**移入 `sources` 子对象**；顶层 `installer` 子对象为新增。
- `scan.result.sources`（全局来源摘要）与 `installs[].matches`/`discovery`：保留只读投影，但**安装授权一律不来自 scan**（design §2.2/§5.1）。

### 1.2 理由

design §6 表把接口收敛为 `get_capabilities / scan|preview / install_patch / list_transactions|recover / discard_plan|cancel、EOF / 缓存清理`；§7.3 要求删除已无消费者的来源安装适配。当前 24 方法中 `plan_install/start_install/fetch_artifact` 是旧两步/手动获取面，`migrate_state` 与来源导入接口属本期清理对象。

### 1.3 迁移动作

- **P1**：`runtime.rs::METHODS` 与 `SOURCE_METHODS` 按 §1.1 重写；`execute` 删除 `plan_install`/`start_install`/`fetch_artifact`/`migrate_state` 分支；`install_patch` 改为单任务动态编排入口；新增 `cleanup_cache`；删除 `get_source_state`/`get_source_settings`/`list_source_matches`/`get_source_policy`/`resolve_source_record` 的 CLI 分支（来源侧方法回收由 P6 统一核对）。
- **P1（契约文档，MF2）**：更新 `packages/contracts/README.md`——协议 `protocol_version=3`、16 方法白名单（A+B 全集）、事件/终态形状（`install_progress` 平铺字段、`InstallOutcome`）、`error.detail`；**删除** `plan_install`/`start_install`/`fetch_artifact` 的方法描述。该文件当前为 v2 陈旧内容（白名单仍含这三方法），须与 §6.3 共享 UI 同步更新，并按 §7.1/§7.4 校验与 `get_capabilities.methods` 精确一致。
- **N3（删除前复核）**：删除 `list_source_matches`/`get_source_policy`/`resolve_source_record` 前，须复核 `apps/decky/backend/source_service.py` 中所述 “optional export or Windows-host reads”（`source_service.py:18-19`），确认无隐藏消费者；无消费者才在 P1 删 CLI 分支，否则把删除推迟到 P5 消费者切换后。
- **P4**：Windows `Operation` 枚举删除 `MigrateState/GetSourcePolicy/ResolveSourceRecord/ListSourceMatches` 等。
- **P5**：Decky `bridge.py::METHODS` 收敛；`publisher.py`/`exe_generic.py` 等消费者随 P5 删除。

### 1.4 验证方式

- 白名单测试：对每个已删方法返回 `unsupported_method`；`get_capabilities.methods` 精确等于 v3 集合（两端断言同一集合）。
- 形状测试：`deny_unknown_fields` 拒绝 `{"shell":"evil"}`；`install_patch` 缺 `instance_id` → `instance_required`；`selection_id` 与 `release_id` 互斥（同给 → `invalid_request`），二者皆缺 → `selection_or_release_required`；`operation` 非 `install|reinstall` → `invalid_operation`（R3）。
- 动态路由测试：`exe-selection-<24hex>-<appid>` 与签名 `release_id` 分别路由，候选 ID 不冒充 release（design §5.1）。

---

## 2. 事件契约

### 2.1 冻结值 —— 命名空间与阶段

- **命名空间分离**：安装器事件前缀 `install_`；来源事件前缀 `source_`。**删除通用 `progress` 事件名**（统一为 `install_progress`）。来源事件沿用 `source_progress`/`source_update`（design §6“与 sources 事件分离”）。
- **阶段枚举（host 可见，9 值）**：`connecting`、`downloading`、`validating`、`analyzing_installer`、`extracting`、`planning`、`applying`、`rolling_back`、`cleanup`。
  - 现 `install.rs` 内部子状态 `backing_up`/`writing`/`committing` **不得**作为事件阶段外泄，统一折叠为 `applying`（可选字段 `substage` 携带 `backing_up|writing|committing`，仅诊断用，不参与逻辑）。
  - `rolling_back` 期间忽略取消（保持 `operation::restoring` 语义，commit 优先）。

### 2.2 事件字段

`install_progress`（单行 JSON）冻结字段。**平铺（N2）**：以下字段直接位于事件对象顶层，**替代现行宿主读取的嵌套 `row["progress"]`**（`apps/decky/backend/bridge.py:180-186`）；P4/P5 宿主须去掉嵌套读取，并将 `(request_id, sequence)` 校验扩展到全部 `install_progress`（不再仅 `source_*`）。

| 字段 | 类型 | 语义 |
| --- | --- | --- |
| `event` | `"install_progress"` | 事件名 |
| `protocol_version` | `3` | 版本 |
| `request_id` | string | 任务绑定（= `install_patch` 的 `request_id`） |
| `sequence` | u64 | **从 0 起严格递增**，每任务独立 |
| `instance_id` | string | 安装实例 |
| `selection_id` | string? | 动态路径的选择 ID；**动态 ZIP（`zip-selection-*`）必填**，签名路径可空 |
| `stage` | enum(9) | 上表 |
| `substage` | string? | `applying` 内部细分，仅诊断 |
| `bytes_done` | u64 | **随阶段可重置为 0** |
| `total_bytes` | u64? | **未知为 `null`，不伪造百分比** |
| `files_done` | usize | |
| `total_files` | usize? | 未知为 `null` |
| `current_file` | string? | |
| `archive_bytes` | u64? | 获取到的包大小（宿主未知时省略） |

`source_progress` / `source_update`：字段保持现状（`completed/total/running/settings_revision`、`appid/index/instance_id/settings_revision/discovery`），仅 `protocol_version` 升 3、`sequence` 语义与 `install_progress` 一致。

### 2.3 终态字段（`InstallOutcome`）

`install_patch` 的**唯一最终响应** `result`：

```
{
  transaction_id, plan_id, instance_id,
  selection_id, content_id?, release_id,       // 归属：content_id 仅 EXE 动态路由产生（可选，R2）
  state: "committed"|"superseded"|"rolled_back"|"needs_recovery",
  error: string|null,
  created_at, updated_at, has_replacements: bool,
  recovery: { required: bool, code?: string, detail?: string } | null,  // needs_recovery 诊断
  cleanup:  { performed: bool, removed_bytes?: u64, code?: string, detail?: string } | null
}
```

- **终态一次**：任务最终响应只写一行；`install_progress` 之后不得再出现事件。
- `needs_recovery` 明确上报，**不伪装 `cancelled`/`completed`**（p0-contracts §5.2）。
- 恢复/清理诊断放在 `recovery`/`cleanup`，不塞进 `error` 单串。
- **`content_id` 为可选字段（R2）**：仅 EXE 动态路由产生；签名 ZIP 路由的终态身份为 `release_id` + `recipe_identity`（p0-contracts §2.2），宿主不得等待 `content_id` 非空。
- **动态 publisher ZIP（P1-3b-3）**：动态 ZIP 的 `release_id` = 来源 release id（`patchmoe-<appid>`，否则 `source-<rule-id>-<appid>`）；`selection_id` = `zip-selection-*`；**`content_id` = `null`**（同 R2，宿主不得等待非空）；内容身份见 p0-contracts §2.4 `install::recipe_identity`（落 `plan.recipe_sha256`）。
- **`uninstalled` 已删除（MF4）**：`install_patch` 不产生该 state，与 design §2.2“不新增长期备份/卸载开关”对齐。

**非终态失败 / 取消（MF4）**

- **计划建立前的失败**（archive 校验、下载、分析、转换、计划失败）**不返回 `InstallOutcome`**，而是 `ok:false` + `error:{code, detail?}`：如 `archive_not_allowed_for_selection`、`archive_too_large`、`download_hash_mismatch`、`download_failed`、`analysis_failed`、`convert_failed`、`plan_failed`。此时无事务/终态写入，宿主按普通 RPC 失败展示真实诊断（§3.2/§3.3），**不得伪装成 `cancelled`/`completed`**。
- **下载期用户取消**：先持久撤销，再停获取/worker；**完成恢复才确认取消**（§5.3）。此时尚未建立计划，返回 `ok:false` + 取消诊断（`error.code` **固定为 `cancelled`**，非 `InstallOutcome`），不产生事务。
- **计划建立后取消**：按恢复结果返回 `InstallOutcome`——恢复成功 `state="rolled_back"`；恢复冲突 `state="needs_recovery"`（`recovery.required=true`）；取消迟到而提交已成功则返回 `state="committed"`（提交成功优先，§5.3）。

### 2.4 乱序/旧任务约束

- 宿主必须按 `(request_id, sequence)` 去重：`request_id` 与当前活动任务不符、或 `sequence ≤ 已见最大值` 的事件**丢弃**，不得更新当前页面（对齐 `apps/windows/src/main.tsx` 的 `activeEventRef` 逻辑；Decky `bridge.py::_source_event` 同理）。
- 终态之后到达的 `install_progress` 事件（无论当前任务还是旧任务）一律忽略；但终态行之后不得再出现第二个终态响应行（见 §2.8）。

### 2.5 理由

design §6 明确阶段清单、字节重置、未知 total、递增 sequence、实例与选择 ID、终态内容 ID/事务归属/恢复与清理诊断、与 sources 事件分离、乱序/旧任务不得更新当前页面。

### 2.6 被否替代或删除项

- 保留通用 `progress` 名（与来源事件混淆，且不带 `instance_id`/`selection_id`）。
- 让内部 `backing_up/writing/committing` 直接作为 host 阶段（阶段集合失控，UI 文案各自 mapping）。
- 把完整分析报告塞进事件/终态（超 8 MiB 单行，p0-contracts §6/§10）。

### 2.7 迁移动作

- **P1**：`main.rs::serve_task` 的进度发射改为 `install_progress` 信封（补 `request_id`/`sequence`/`instance_id`）；`operation.rs` 只暴露 9 阶段，内部子状态折叠。
- **P4/P5**：宿主事件名与监听器改为 `install_progress`；`stageLabels` 更新为 9 阶段；删除旧 `progress` 监听。

### 2.8 验证方式

- 事件顺序：合成进度流断言 `sequence` 严格递增、`total_bytes:null` 不产出百分比、阶段重置字节。
- 乱序/旧任务：注入乱序与异 `request_id` 事件，断言 UI/桥接状态不变。
- 终态一次：终态后注入迟到 `install_progress` 事件 → 忽略（§2.4）；终态后再注入第二个终态响应行 → 宿主判 `invalid_core_response`。

---

## 3. 能力协商与错误契约

### 3.1 `get_capabilities` 冻结形状

```json
{
  "protocol_version": 3,
  "installer": {
    "schema_version": 1,
    "selection_version": 1, "content_version": 1,
    "adapter_version": 1, "policy_schema_version": 1, "report_schema_version": 1,
    "formats": { "zip": "supported", "exe": "supported" },
    "strategies": ["managed_zip", "publisher_zip", "publisher_exe"],
    "links": {
      "zip": { "available": true,  "blockers": [] },
      "exe": { "available": false, "blockers": [ { "code": "exe_sandbox_unavailable", "detail": "..." } ] }
    }
  },
  "sources": {
    "discovery_schema_version": 1, "adapter_version": 1,
    "parser_version": 2, "query_version": 1,
    "methods": ["list_sources", "list_game_sources", "set_source_enabled",
                "set_game_source", "add_source", "refresh_sources", "scan_sources", "discover_game"]
  },
  "methods": [ "<A+B 全部 v3 方法>" ]
}
```

- `links.zip`/`links.exe` 为**按平台探测**结果：Windows 反映 LPAC/Job Object 探针状态，SteamOS 反映 bwrap；`available=false` 附 `blockers[].code`（复用共享 UI `reason` 键，如 `exe_sandbox_unavailable`）。
- 隔离缺失**只阻断对应 EXE**，来源页与签名 ZIP 保留（design §4.3）。
- 能力为**协商快照**；运行时仍重核（design §4.3 “运行时仍重核”）。
- **可安装性判据（N1）**：是否可安装以 `installer.links.*.available` 为准；`formats`（如 `formats.exe:"supported"`）**仅表示核心支持该类格式**，不代表本机当前可用（示例中 `formats.exe:"supported"` 与 `links.exe.available:false` 并存即此意）。UI 不得以 `formats` 判定可安装。

### 3.2 删除旧核心降级

- Windows `ensureSourcesSupported`（`main.tsx`）**删除**：不得再把旧核心/任意 RPC 异常降级为“仅静态扫描”，不显示“当前核心版本尚未支持来源查询”这类吞错文案。
- 契约不匹配（协议/能力/方法缺失）→ 明确报错，UI 显示真实诊断（`reason(detail)`）。
- 普通 RPC 故障保留实际 `error.detail`，不替换为能力缺失。

### 3.3 错误形状（新增 `detail`）

- `error = { code, detail? }`；`detail` **可选、≤2 KiB、截断后的底层诊断**（核心错误串/子进程诊断），供 UI 展示真实原因。
- 冻结新增错误码：`unsupported_protocol`（版本不匹配）、`capability_mismatch`（能力含缺失方法/协议）、`installer_schema_unsupported`（installer schema/策略不匹配）、`installer_lease_expired`、`cleanup_failed`、`task_required`（任务面方法经 `serve` 调用，F2）。
- 保留并用：`request_id_binding_conflict`、`install_request_in_progress_or_interrupted`、`installer_busy`、`needs_recovery` 相关码。

### 3.4 理由

design §4.3/§6；shared-source-discovery 已把 `get_capabilities` 作为协议/能力校验入口。§7.3 明确删除 Windows 静态扫描降级并“保留实际诊断”。

### 3.5 被否替代或删除项

- 保留 `ensureSourcesSupported` 的 `catch(() => false)` 降级。
- 用 `error.code` 单串承载诊断（无法区分契约错误与现场故障）。
- 顶层平铺 `parser_version`（与 installer 版本混层）。

### 3.6 迁移动作

- **P1**：扩展 `sources::session::Capabilities` 或新增 `installer` 能力子对象（合并到 `get_capabilities` 响应）；平台探针接口留给 P3/P4 注入。
- **P4**：删除 `ensureSourcesSupported` 与降级文案；错误处理改为展示 `detail`。
- **P5**：`source_service.REQUIRED_METHODS` 更新为 v3 集合。

### 3.7 验证方式

- 能力形状测试：两端断言 `installer.links.exe.available` 与平台探测一致；隔离缺失时 `blockers[].code` 可映射文案。
- 降级测试：注入旧核心/方法缺失 → 明确错误 + 真实诊断，**不**回退静态扫描。

---

## 4. 容量限制与单行协议

### 4.1 冻结值

| 项 | 上限 | 行为 |
| --- | --- | --- |
| 请求行 | **≤64 KiB**（含换行） | 超限 → `request_too_large`，流不终止 |
| 宿主响应/事件单行 | **≤8 MiB** | 宿主侧 → `response_too_large` |
| worker 报告/描述符 | **≤16 MiB** | 完整报告留受控目录（`installer/rust-v1/reports/<content_id>/`），UI 只拿受限摘要/计数/诊断 |
| 编码 | JSON-lines，单行一个对象 | 非法单行 → `invalid_request`/`invalid_core_response`，不终止后续 |

- 保持 p0-contracts §6“描述符/响应 ≤16 MiB”“完整报告留受控目录”，design §6“请求 ≤64 KiB、宿主响应行 ≤8 MiB、worker 报告 ≤16 MiB，不能直接塞进 UI 单行”。
- **只读/互斥交叉引用（N4）**：`scan`/`list_transactions` 的一致只读快照与 `installer_busy` 语义以 [p0-contracts.md](p0-contracts.md) §4.1 为准（`Store::open_ro` + `SQLITE_OPEN_READ_ONLY`、不取 `journal.lock` 独占；真实安装/恢复 busy 仍上报；同 state 排队、不同 state 独立），覆盖 design §6.1；本节不重复冻结锁细节。

### 4.2 理由 / 被否替代

沿用现行 `main.rs MAX_REQUEST_BYTES=64KiB`、Windows `MAX_RESPONSE=8MiB`、worker 16 MiB。被否：把完整 worker 报告放进 `install_progress`/终态（超单行、UI 无法承载）。

### 4.3 迁移动作 / 验证方式

- **P1**：`install_patch` 终态只返回 `InstallOutcome` + 有界诊断；报告路径经 `content_id` 受控引用。
- 测试：超长请求行；超大 worker 报告 → 拒绝并报 `exe_worker_response_limit`，不中止流。

---

## 5. 幂等/取消/租约宿主侧契约

### 5.1 幂等（request_id 绑定）

- 冻结绑定元组：`(instance_id, selection_id|release_id, operation, resolved content_id)`（design §6“绑定实例、选择、操作与已解析内容”）。
- **动态 publisher ZIP 绑定（P1-3b-3）**：动态 ZIP 的 `selection_id` **必填**、`release_id` 为 null、内部 `content_id` 存 `install::recipe_identity` 摘要（p0-contracts §2.5/§3.1）。
- **签名路由授权绑定（实施冻结值，P1 gate 裁决）**：签名 `release_id` 路由复用 `installer/rust-v1` 新授权，绑定 `source_id="signed"`、`settings_revision=catalog.revision`、由 release 派生 `candidate_sha256`/`rule_fingerprint`、现场 manifest 语言；宿主不感知也不派生该内部绑定。签名路由迁移收窄为“根/账本迁移 + facts/撤销/租约重核”（[p1-zip-orchestration.md](p1-zip-orchestration.md) §4.1 step 2/§9）。
- **计划建立前的失败不写 `error`**（保留绑定行、允许同 `request_id` 重试）；计划建立后回填 `plan_id`/`content_id`，终态后回填 `transaction_id/state/error`。
- 账本：`installer/rust-v1/requests.sqlite`（p0-contracts §3.1），**替代旧顶层 `install_requests.sqlite` 的读取**。
- 行为：
  - 相同 `request_id` 相同绑定 → **返回原任务/原终态**，不二次写入（幂等）。
  - 相同 `request_id` 不同绑定 → `request_id_binding_conflict`。
  - 崩溃/中断 → 不触发第二次写入；诊断 `install_request_in_progress_or_interrupted`，或返回可信终态。
- **`request_id` 生成（MF1）**：宿主为**每次用户发起的 install/reinstall 动作**生成新的唯一 `request_id`；仅对该动作的**传输/UI 重试**复用同一 id；**终态后新动作（含同内容重装）必须换新 id**。**不按绑定元组确定性派生**——否则同内容重装会同键→同 id→幂等短路，返回旧终态而不再写入。
- **重放算法（N6）**：账本绑定在计划解析后可写 `content_id`。重放比较顺序：
  1. 先比 `(instance_id, operation)` 与选择身份 `selection_id|release_id`；不符即 `request_id_binding_conflict`（此时可能尚未解析 `content_id`）。
  2. 若已有计划/账本记录携带 `content_id`，再比 `content_id`；不同即冲突。
  3. 尚未解析 `content_id`（首次请求在计划前崩溃/重试）时，同 `request_id` 且同 `(instance_id, operation)` 与选择身份视为同一动作继续/返回；解析出内容后**补写** `content_id`，此后冲突判定以完整绑定为准。
- 重启**不恢复旧未完成写入许可**，先恢复事务（design §6、§7.1）。

### 5.2 租约（15s）与续租转发

- 前台租约 TTL = **15 秒**（p0-contracts §5.1）；`PLAN_TTL ≤ 900 秒`不放宽。
- 宿主**转发续租**：任务通道 `renew` 控制行；核心掌握真实授权/取消。租约过期 → 持久撤销。
- 宿主在 UI 前台轮询/心跳时发送 `renew`；迟到重连不能复活已过期租约（对齐 `tasks.py::status`）。

### 5.3 取消 / EOF / 退出

- 顺序：**先持久撤销**（`jobs/<uuid>/revoked` + `sync_all`）→ 再停获取/worker；进入写入按 journal 恢复，**完成恢复才确认取消**（design §6、p0-contracts §5.1）。
- EOF/父退出 → `cancel` + worker 进程组回收（Linux SIGTERM→1s→SIGKILL；Windows Job `KILL_ON_JOB_CLOSE`）。
- **提交成功优先于迟到取消**。
- 恢复冲突 → `needs_recovery`（`InstallOutcome.recovery.required=true`），不伪装 `cancelled/completed`。

### 5.4 理由 / 被否替代

沿用现行 `install_patch` 幂等账本与持久撤销（`runtime.rs`）、`tasks.py` 15s 租约、`bridge.py` 取消语义。被否：仅内存标志（崩溃/新进程不生效）；把续租留在纯宿主侧而核心不知（核心无法掌握真实授权）。

### 5.5 迁移动作

- **P1**：`install_patch` 绑定元组扩展为含 `selection_id`/`content_id`；账本迁 `installer/rust-v1/requests.sqlite`；任务通道实现 `renew`；租约 deadline 落 `jobs/<plan_id>/lease.json`（`{deadline_ms, renewed_at}`，`write_atomic`；F3 已补做）。
- **P4/P5**：宿主转发 `renew`；Decky `tasks.py` 租约与 `task_status(renew)` 对齐桥接。

### 5.6 验证方式

- 幂等：同 `request_id` 重放返回同一 `transaction_id`；改 `operation`/选择 → 冲突。
- 逐阶段取消：download/analyze/convert/plan/apply；撤销写失败/discard 失败 → `needs_recovery`；迟到取消不覆盖 committed。
- 租约：不续租 → 15s 内撤销；`renew` 保持授权；EOF/父退出后进程树无残留。

---

## 6. 双端宿主适配要点

### 6.1 Windows（Tauri）

- **命令 → 方法映射**（P4 更新）：
  - 保留/改名：`core_call(scan|preview|list_transactions|recover)`、`source_capabilities`→`get_capabilities` 读取、`start_install`→`install_patch`、`install_status`（进度）、`cancel_install`（写 `cancel` 控制行）、`choose_archive`/`open_source` 保留；来源命令集收敛为 §1.1 B。
  - 删除：`MigrateState` 及 `setup()` 里的启动迁移调用（`main.rs`）、`get_source_policy`、`resolve_source_record`、`list_source_matches`。
- **事件**：监听/转发 `install_progress`（新）、`source_progress`、`source_update`；`stageLabels` 改为 9 阶段；删除旧 `progress`。
- **契约校验**：`validate_response` 版本改 3；错误保留 `detail`；删除 `ensureSourcesSupported` 降级（§3.2）。
- **request_id**：宿主为**每次用户发起的 install/reinstall 动作**生成新的唯一 `request_id`；仅对该动作的传输/UI 重试复用同一 id；终态后新动作（含同内容重装）必须换新 id（**不按绑定元组确定性派生**，MF1）。
- **archive**：仅 `valid_archive`（普通 `.zip`）后传，UI 不传配方/路径。

### 6.2 Decky（Python 桥接）

- `bridge.py`：`METHODS`/`SOURCE_METHODS`/`CONTROLLED_METHODS` 收敛为 v3 白名单；`protocol_version=3` 校验；`SOURCE_EVENTS` 扩为 `install_progress` + `source_progress/source_update`；转发事件带 `request_id/sequence` 校验（**N2**：去掉 `row["progress"]` 嵌套读取，按 §2.2 平铺字段消费，校验扩展到全部 `install_progress`）；新增 `renew` 转发。
- `main.py`：安装路由改为单任务 `install_patch`；删除 `_migrate_startup`/`_reclaim_plans` 对旧 journal 的依赖（改由 `recover`/`discard_plan` v3 语义）；`_dispatch_id` 的 `publisher:` 包装随 P5 消费者删除。
- `source_service.py`：`initialize()` 不再读 `get_source_state.import_report`；`REQUIRED_METHODS` 更新；删除 `validate_download`/`source_for_release`/旧 release 解析（P5）。
- 租约：`tasks.py` 15s 与核心租约对齐，宿主转发 `renew`。

### 6.3 共享 UI DTO / 文案（`packages/ui/src/index.ts` + `reason`）

需同步的字段：

| DTO/位置 | 变更 |
| --- | --- |
| `Transaction` | 增 `content_id`、`selection_id`；删 `backups?`（改 `has_replacements`） |
| `Patch` | 增 `selection_id`；`exe.profile_id` 随 profile 注册表删除（P6） |
| `Capabilities`（Windows `sources.ts`） | 改为 §3.1 形状（`installer.links.*`） |
| `InstallStatus.progress.stage` | 收敛为 9 阶段 |
| `reason(code)` | **新增**键：`unsupported_protocol`(已有)、`capability_mismatch`、`installer_schema_unsupported`、`installer_lease_expired`、`cleanup_failed`、`contract_mismatch`；**删除**无生产者键（旧 `publisher_*`/`exe_profile_*` 随 P5/P6） |
| `packages/contracts/README.md` | 升 v3（MF2）：16 方法白名单、`install_progress` 平铺字段/`InstallOutcome` 形状、`error.detail`；**删除** `plan_install`/`start_install`/`fetch_artifact` 描述 |

- UI 只传 `instance_id` + `selection_id|release_id` + `operation`；不传 `recipe`、目标路径、roots/catalog/keys（design §6、§2.1）。

### 6.4 理由 / 被否替代

design §6“宿主随新契约更新并只转发身份/任务/事件”“UI 不传配方”；§7.3 删除旧核心降级与来源适配。被否：宿主保留旧方法别名做兼容；UI 传 recipe/路径（扩大攻击面）。

### 6.5 验证方式

- Windows：`cargo build`（Tauri）、`tsc`/`vite build`；命令集断言不含已删方法；事件监听为 `install_progress`。
- Decky：`tsc`/vitest；`test_bridge.py`（版本 3、事件转发、cancel/renew）、`test_tasks.py`、`test_install_pipeline.py`、`test_source_service.py` 更新。

---

## 7. 验收与测试要求

### 7.1 协议契约测试（P1，Rust）

- 拒绝 v2/v1：`unsupported_protocol`；未知字段 `invalid_request`；未知方法 `unsupported_method`；`request_id` 非法 `invalid_request_id`。
- 字段校验：`install_patch` 缺失/非法参数；`deny_unknown_fields`。
- 事件顺序：`install_progress.sequence` 严格递增；`request_id` 绑定；`total_bytes:null` 无伪造百分比；终态后无事件。
- 终态一次：流中第二个“终态”行 → `invalid_core_response`。
- 幂等：同 `request_id` 同绑定重放（传输/UI 重试）返回同一 `transaction_id`；异绑定 → `request_id_binding_conflict`；**同内容重装（新用户动作）必须使用新 `request_id`**，产生新事务而非短路返回旧终态（MF1）。
- 容量：请求超 64 KiB、响应/报告超限。
- archive 拒绝（MF5/R1）：动态 `exe-selection-*` 路由携带 `archive` → `archive_not_allowed_for_selection`；超 8 GiB → `archive_too_large`；符号链接/非 `.zip` 普通文件 → 拒绝。
- 能力：v3 形状、平台 `links` 探测、方法集合两端一致。
- 契约文档一致性（MF2）：`packages/contracts/README.md` 与 `get_capabilities.methods`（16 方法）、事件/终态形状、`error.detail` 精确一致；不再出现 `plan_install`/`start_install`/`fetch_artifact`。
- 单测参考：`crates/cli/src/main.rs::tests`、`runtime.rs::tests`、`install_flow_tests` 更新为 v3。
- **动态路由矩阵（P1-3b-3）**：`release_id` / `zip-selection-*` / `exe-selection-*` / 未知前缀 / `selection_id` 与 `release_id` 互斥 / 皆缺 → 预期码（前缀分派 `zip-selection-*`→动态 ZIP、`exe-selection-*`→`exe_route_unimplemented`；其他前缀→`invalid_selection_id`；互斥→`invalid_request`；皆缺→`selection_or_release_required`）；动态 selection + `archive` → `archive_not_allowed_for_selection`。
- **动态 ZIP 编排（P1-3b-3）**：幂等（同 `request_id` 同绑定重放返回同 `transaction_id`；异 `operation`/选择 → `request_id_binding_conflict`；计划前失败**不写 `error`** 可重试；同内容重装换新 `request_id` → 新事务）；取消（计划前无事务 `cancelled`；计划后 `rolled_back`；迟到取消不覆盖 `committed`）；恢复（`recover` → `rolled_back`/`needs_recovery`）；重装（同内容 `committed`；新字节/语言/DLC → `reinstall_content_identity_changed`；无主 → `reinstall_unowned_or_incomplete`）。
- **`stage`/`substage` 映射（P1-3b-3）**：动态 ZIP 断言 `connecting→downloading→validating→planning→applying(substage=backing_up|writing|committing)→cleanup`；`analyzing_installer`/`extracting` 对 ZIP 不发出；`sequence` 严格递增、阶段重置字节。

### 7.2 宿主构建/类型

- Windows：Tauri `cargo build` + 前端 `tsc`/`vite build` 通过；`build_params` 校验测试更新。
- Decky：`tsc`、vitest、Python 桥接单测通过。

### 7.3 桥接测试

- `test_bridge.py`：`protocol_version=3` 校验；`install_progress`/`source_*` 转发；`cancel`/`renew` 控制行；事件乱序拒绝。
- `test_tasks.py`：15s 租约续租/过期；取消/EOF。
- `test_install_pipeline.py`：单任务 `install_patch` 幂等/重装/恢复。
- `test_source_service.py`：`REQUIRED_METHODS` v3；删除 import_report 断言。
- `discover_game` 为 `serve` 一次性调用（非 `serve-task`）：断言其不经任务通道、不产生流式事件（MF6）；对应 §1.1 B 模式列与 §0.1 执行面。

### 7.4 待实测/待实现

- 事件阶段折叠、租约 `renew`、`installer/rust-v1/requests.sqlite` 幂等账本、`cleanup_cache` 清单接口均为 **P1 待实现**。
- 两端 `install_progress` 实机转发、Windows Tauri 命令收敛为 **P4 宿主**；Decky 桥接收敛为 **P5 宿主**。
- 来源方法删除（`get_source_state` 等）随消费者切换为 **P5**，逐项核对为 **P6**。

---

## 8. 待决策 / 待实测汇总（不静默假设）

| 项 | 归属 | 状态 |
| --- | --- | --- |
| `install_patch` 单任务内 acquire→analyze→convert→plan→apply 的阶段映射（analyzing/extracting 与 worker action 对应） | §1/§2 | **待实现**（P1/P3） |
| 平台隔离探针注入 `get_capabilities.installer.links` 的接口 | §3 | **待实现**（P1 接口 + P3/P4 注入） |
| `renew` 控制行频率与租约落盘（`jobs/<plan_id>/lease.json`） | §5 | **已冻结/已补做**（P1，F3；`{deadline_ms, renewed_at}` 阶段边界刷新） |
| `cleanup_cache` 返回 `CacheStatus` 字段全集 | §1/§4 | **待实现**（P1，依 p0-contracts §3.2 可清目录） |
| Windows/Linux 实机事件顺序与终态一次 | §2/§7 | **待实测**（P4/P5） |
| `reason` 新增/删除键的最终清单 | §6.3 | **待实现**（P1 定键，P4/P5 同步） |
| 是否保留 `list_source_matches`/`get_source_policy` 的宿主内部用途 | §1.1 | 已核对无消费者 → **删除**；若 P5 出现新消费者再单独评估 |
| `archive` 本地导入的宿主校验边界（是否改为不透明引用） | §1.1/§6 | **待实现**（P1 保留受控绝对路径；P4/P5 对齐） |
| 动态 publisher ZIP 编排（选择 ID / resolve 数据流 / 账本绑定 / 阶段映射 / 错误码） | §1/§2/§5 | **已冻结**（P1-3b-3 设计，2026-10-08）：见 [p1-zip-orchestration.md](p1-zip-orchestration.md)；P1 实现 `install_patch` 动态 ZIP 路由 |

---

## 9. 与 design/p0 的一致性

- 覆盖 design §6 全部接口组：get_capabilities / scan|preview / install_patch / list_transactions|recover / discard_plan|cancel、EOF / 缓存清理。
- 覆盖 design §5.2 授权：终态显式返回 `content_id`/`release_id`；UI/worker 无私钥/配方/路径。
- 覆盖 design §7.3 清理的 **RPC 可见项**：删除 `get_source_state`/`get_source_settings`/`list_source_matches`/`get_source_policy`/`resolve_source_record` 与 `migrate_state`，并删除 Windows `ensureSourcesSupported` 降级。
- **非 RPC 清理项（MF3）**：`source_engine.py`/`plugin_settings.py` 的 `engine-switch.json`/engine-switch 设置与 `EngineSwitch*`/conflicts，以及 SourceService 的 `matches`/`cached_candidates`/`_rules`/`builtins` 等安装适配，**不在本契约 RPC 白名单内**，随 P5/P6 按 design §7.3 逐项删除；本稿不重复枚举其内部方法。
- 承接 p0-contracts §0（protocol 3）、§6（worker ≤16 MiB / 完整报告受控目录）、§11（本稿即 P1 宿主契约冻结）。
- **N7（定稿回写，非本次修订）**：本稿定稿后在 [p0-contracts.md](p0-contracts.md) §10/§11 回写“宿主契约已冻结（指向本稿）”，并同步 [progress.md](progress.md) 与 `docs/features/README.md` 索引。
- 未冻结项已按归属标注；未以“全部可安装”开放入口。
