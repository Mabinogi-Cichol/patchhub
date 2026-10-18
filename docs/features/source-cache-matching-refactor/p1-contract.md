# P0/P1 基线契约（冻结稿）

- 所属功能：[source-cache-matching-refactor](design.md) · [施工记录](progress.md)
- 状态：P0 契约细化成果，P1 实施依据。数值来自静态核对与本地测试，真实网站与双端实机未验证。
- 本文只冻结**数据形状、修订语义、协议名称和调度语义**；实现细节以代码与测试为准。

## 1. 版本

| 契约 | 现值 | 目标 | 说明 |
| --- | --- | --- | --- |
| `PROTOCOL_VERSION` | 4 | **5** | 来源面方法名与事件形状变更，属破坏性升级，双端一次切换 |
| `STORE_SCHEMA_VERSION` | 1 | **2** | `settings.json` 新增 `intervals`；来源索引/详情/匹配记录使用独立 schema |
| `SOURCE_INDEX_SCHEMA_VERSION` | — | 1 | `sources/rust-v1/index/<source_id>.json` |
| `SOURCE_DETAIL_SCHEMA_VERSION` | — | 1 | `sources/rust-v1/details/...` |
| `GAME_MATCH_SCHEMA_VERSION` | — | 1 | `sources/rust-v1/matches/<appid>.json` |
| `PARSER_VERSION` / `SOURCES_ENCODING_VERSION` | 2 / 1 | 不变 | 只有实际影响身份编码时才升级；升级即失效旧索引与详情 |

## 2. 状态目录

```text
<state_dir>/sources/rust-v1/
  schema.json                     {"schema_version":2,"created_at":<unix>}
  settings.json                   见 §3
  settings.lock
  status/<source_id>.json         SourceStatus（尝试/成功/进度状态，见 §4）
  index/<source_id>.json          SourceIndex（按源索引快照，见 §5）
  details/<source_id>/<key>.json  DetailRecord（按规范 URL 共享的详情缓存，见 §6）
  matches/<appid>.json            GameMatchRecord（按游戏身份持久化的匹配结果，见 §7）
```

- `<key>` = `sha256(canonical_url(page))` 的前 32 个十六进制字符；稳定、可重算。
- 旧 `cache/`、`discovery/`、`discovery-index/` 只在升级迁移时读取一次（§8）；迁移后不再读写。
- 网络请求期间不持有存储写锁；所有发布使用现有 `write_atomic`（同目录临时文件 + rename）。

## 3. settings.json（schema 2）

沿用现有字段，新增：

```json
{
  "schema_version": 2,
  "revision": 12,
  "enabled": {"patchmoe": true},
  "added": [],
  "games": {"42": {"patchmoe": false}},
  "intervals": {"patchmoe": 24}
}
```

- `revision`：全局设置修订，现有语义不变（启用/订阅/新增/间隔变化都会递增）。
- `intervals`：`source_id -> 自动更新间隔小时数`；缺失或 `0` = 关闭。默认全部缺失（默认关闭自动更新）。
- 间隔与订阅设置修订分开记录：索引内容修订在 `SourceIndex.revision`，不写入 settings 修订。只有影响匹配输入的设置变化才触发重匹配（§7.2）。

## 4. SourceStatus（每个来源一条）

```text
schema_version: 1
code: string                 最近一次尝试结果码（source_refreshed / source_not_modified /
                            source_partial / source_offline_cached / source_refresh_failed ...）
checked_at: u64              最近一次尝试结束时间（Unix 秒）
cached_at: u64|null          最近一次成功快照发布成功的时间
entry_count: u64             当前正式快照条目数
revision: u64                当前正式快照内容修订（= SourceIndex.revision，无索引时为 0）
complete: bool               当前正式快照是否覆盖完整
```

- `next_due_at` 与 `interval_hours` 由 `list_sources` 汇总时按 settings 计算，不单独持久化，避免两份时间互相漂移。
- 失败/取消只更新 `code/checked_at`；`cached_at/entry_count/revision/complete` 保持最后成功快照。首次未完整更新时 `complete=false`、`cached_at=null`，不得把局部空集合当作无补丁依据。

## 5. SourceIndex（按源索引快照）

```text
schema_version: 1
source_id: string
revision: u64                内容修订：仅当 entries/complete/pending 实际变化时 +1
complete: bool               覆盖完整（分页取完且成功解析）
checked_at: u64              产生该快照的尝试结束时间
updated_at: u64              成功发布时间（失败不更新）
etag: string|null            条件请求校验器（列表页）
modified: string|null
entries: SourceIndexEntry[]
pending: string[]            未完成的分页续跑队列；非空 => complete=false
```

`SourceIndexEntry`：

```text
title: string
aliases: string[]            列表页可直接取得的别名；默认空
page: string                 规范化的详情/产品/文章/分类 URL；表格类来源为条目来源页
kind: "product"|"article"|"category"|"entry"
appid: u32|null              列表页实际给出的 AppID（不猜测、不补齐）
url: string|null             列表页直接给出的制品 URL（patch.moe / 通用表格）
language: string|null
date: string|null
```

- 只收录列表页能直接取得的信息；**不为补齐字段遍历详情页**。
- `revision` 用规范化内容比较得出；仅更新时间变化不递增，避免无输入变化时重匹配。
- 同一源同一轮更新内按规范 URL 去重；详情缓存按 §6 跨轮次、跨游戏复用。
- 只有完整成功结果替换正式索引；部分失败/取消保存 `pending` 与尝试状态，保留旧正式索引。

## 6. DetailRecord（按来源 + 规范 URL 共享）

```text
schema_version: 1
source_id: string
page: string                 规范 URL（与 index entry.page 对齐）
index_revision: u64          抓取时命中的索引修订
checked_at: u64
complete: bool               该页面所需详情工作是否全部完成
candidates: SourceCandidate[]
diagnostics: Diagnostic[]
```

- 缓存归来源所有；不同游戏引用同一页面只保存/请求一次。
- 手动重匹配、来源更新后当前库命中项才刷新命中详情；不更新未安装游戏的详情。
- `index_revision` 变化表示详情可能过期；匹配任务决定是否重取。没有可靠变更信号时重新读取，不假设详情永不变化。

## 7. 游戏匹配记录

### 7.1 GameMatchRecord（`matches/<appid>.json`）

```text
schema_version: 1
appid: u32
name: string
localized_name: string|null
input_key: string            影响匹配的输入摘要（见 §7.2）
status: "pending"|"publisher_match"|"fallback_match"|"no_match"|"incomplete"|"failed"
sources: {<source_id>: SourceMatchRecord}
fallback: FallbackMatchRecord|null
updated_at: u64
```

`SourceMatchRecord`：

```text
index_revision: u64
status: "no_index"|"index_incomplete"|"matched"|"no_candidate"|"detail_incomplete"|"detail_failed"
candidates: SourceCandidate[]
detail_pages: string[]
checked_at: u64
diagnostics: Diagnostic[]
```

`FallbackMatchRecord`：

```text
query_key: string
status: "not_queried"|"matched"|"no_match"|"incomplete"|"failed"|"disabled"|"not_needed"
candidates: SourceCandidate[]
checked_at: u64
diagnostics: Diagnostic[]
```

顶层 `status` 由各来源与 fallback 结果按设计 §3.2 推导：

1. 任一普通源 `matched` 且候选可信 → `publisher_match`，fallback 不作为显示依据。
2. 普通源无命中且覆盖完整，HGamer `matched` → `fallback_match`。
3. 普通源覆盖完整、无命中，HGamer `no_match`（或 `disabled`，即用户主动关闭补查）→ `no_match`。`disabled` 时界面显示“已启用来源中未找到补丁”，不声称 HGamer 已查无结果。
4. 普通源未初始化/不完整、详情未完成、没有任何启用的普通源、fallback 未查询或失败/取消 → `incomplete`（已有正结果仍显示，但不得声称确认无补丁）。普通源不完整时不联网补查 HGamer（先提示更新来源）。
5. 全部失败 → `failed`。

### 7.2 input_key 与增量条件

`input_key = sha256(canonical_json({name, localized_name, name_hints, preferred_language, subscriptions}))`，其中 `subscriptions` 只包含**下载/匹配相关**的每游戏源订阅与全局启用状态快照（`source_id -> bool`），不含页面查看状态、更新时间、缓存年龄；`name_hints` 为前端提示与本地化名合并后的有序数组（localized_name 优先、去重、≤3 条）。

- 新增实例（可信 AppID + 实例）只匹配自身。
- `input_key`、相关来源 `index_revision` 或订阅设置变化时重算该游戏；否则直接读取持久化结果。
- 相同 AppID 多实例共享 `matches/<appid>.json`；每个实例的 DLC/路径/安装状态仍由安装事务与投影各自持有。
- 来源更新后只重算该来源：命中项刷新详情，保留其他来源结果，再重判 HGamer 显示。
- 来源禁用/退订后重算受影响关联，但不删除缓存。

## 8. 迁移（schema 1 → 2）

原子发布、失败保留原数据并报告，不静默回退为空结果：

| 旧数据 | 新表示 | 覆盖语义 |
| --- | --- | --- |
| `settings.json` v1 | v2，`intervals` 为空 | 启用/新增/订阅原样保留，`revision` 保留 |
| `cache/<source>.json`（patch.moe / 表格） | `index/<source>.json`：entries `kind="entry"`，`url`/`appid`/`language`/`date` 映射 | `cached_at` 存在 => `complete=true`；否则 `complete=false` |
| `discovery-index/<source>.json`（Kagura） | `index/<source>.json`：records 映射 `title/page/kind` | `pending` 为空 => `complete=true`，否则 `false` 并保留 pending |
| `discovery/<source>/<appid>.json` | `matches/<appid>.json` 的 `sources`/`fallback` 候选 | 按来源索引是否完整判定；旧逐游戏缓存**不得**冒充完整全源快照 |
| `status/<source>.json` | 原文件兼容读取，缺失新字段取默认 | 保留最近尝试结果 |

- `schema.json` 最后写入 2；迁移中崩溃可幂等重跑。迁移完成后旧文件不再读取，但保留不删除。
- 旧 `discovery` 候选的 `candidate_id`/`fingerprint` 原样保留，避免人工选择/安装收据失联。

## 9. 协议 v5 方法面（20）

安装器/只读面（11）：`get_capabilities`、`scan`、`project_instances`、`preview`、`install_patch`、`list_transactions`、`recover`、`discard_plan`、`cleanup_cache`、`backup_status`、`cleanup_backups`。

来源面（9）：

| 方法 | 通道 | 参数 | 结果 |
| --- | --- | --- | --- |
| `list_sources` | serve | — | `SourceSummary[]`（含 `revision/complete/interval_hours/next_due_at/checked_at/cached_at/status/entry_count`） |
| `list_game_sources` | serve | `instance_id` 或 `appid` | `GameSourceSummary[]`（含 `subscribed/matched`） |
| `set_source_enabled` | serve | `source_id, enabled` | `{settings_revision}` |
| `set_game_source` | serve | `instance_id/appid, source_id, subscribed` | `{settings_revision}` |
| `add_source` | serve | `rule` | `{settings_revision, rule}` |
| `set_source_update_interval` | serve | `source_id, interval_hours`（0=关闭） | `{settings_revision, source_id, interval_hours}` |
| `update_source` | serve-task | `source_id, force?` | `SourceUpdateResult{source_id, settings_revision, status, affected_appids}` |
| `match_games` | serve-task | `instances?`（实例 ID 列表；缺省=当前库全部）、`force_details?` | `{settings_revision, source_revisions, results: GameMatch[]}` |
| `rematch_game` | serve-task | `instance_id` 或 `appid` | `{settings_revision, source_revisions, results: GameMatch[]}` |
| `project_instances` | serve | `instance_ids: string[]` | `{settings_revision, instances: InstanceProjection[]}` |

- 删除：`refresh_sources`、`scan_sources`、`discover_game`（P5 切换后调用返回 `unsupported_method`）。
- `scan` 仍是整库只读投影；`project_instances` 是单行/批次投影入口，不整库哈希。
- `GameMatch`（wire DTO）：

```text
appid: u32
instance_id: string|null
status: 顶层状态（§7.1）
candidates: SourceCandidate[]     普通源可信命中优先；无普通源命中时含 fallback 正结果
diagnostics: Diagnostic[]
source_revisions: {source_id: u64}
```

`InstanceProjection.discovery` 保留现有形状，改为从 `GameMatchRecord` 投影，`status` 取值同上六种。

### 9.1 事件（复用现有 JSONL 任务通道）

- `source_progress`：`{source_id?, phase:"index"|"details"|"matched", completed,total,running,settings_revision}`。索引更新与详情阶段明确区分，界面不得把整条任务宣称为纯离线。
- `source_update`：`{source_id, status: SourceStatus, settings_revision}`；`update_source` 终态与手动/自动更新状态回显。
- `match_progress`：`{phase:"inventory"|"index"|"details"|"fallback", completed,total,running,settings_revision}`。
- `match_update`：`{appid, instance_id?, status, candidates, diagnostics, settings_revision}`。
- 事件 `sequence` 语义沿用现有 `EventSequence`。

### 9.2 capabilities

`SourceCapabilities` 改为：

```text
index_schema_version: 1
detail_schema_version: 1
match_schema_version: 1
schedule_schema_version: 1
adapter_version: 1
parser_version: 2
encoding_version: 1
methods: 9 个来源方法
```

`contracts` 四个既有版本不变。host 能力校验同步更新，先于任何状态目录写入。

## 10. 调度语义（P4）

- 到期 = 应更新，不代表缓存失效；失败或取消继续使用最后成功快照，并准确显示状态。
- 自动更新仅在宿主运行时执行（Decky 插件运行期间 / Windows 应用运行期间）；不安装系统服务。启动/恢复后检查到期项，每源补一次，不补跑错过周期。
- `next_due_at = status.checked_at + interval_hours*3600`（`interval_hours>0` 且源启用且非 fallback 源时有效）；到期判断使用宿主时钟与 `list_sources` 返回的时间。
- 失败后下一次到期从本次尝试结束时间起算，不立即重试；无自动无限重试。
- 同源重复手动更新：宿主发现该源已有活动任务时返回现有任务，不叠加；不同源可排队。
- HGamer（`fallback_indexer`）不参与全站自动更新，只在游戏匹配时按游戏查询；正结果与已完成空结果可复用。

UI 控件预设（“管理订阅源”每源）：关闭、6 小时、12 小时、24 小时、3 天、7 天、14 天；默认关闭。协议接受任意 `u64` 小时间隔，控件预设不构成核心硬限制。

## 11. 删除范围（P5 切换后）

- Rust：逐游戏 `discover_game`/`scan_sources`/`refresh_sources` 会话与 `DiscoveryRecord/Success/Attempt`、`needs_refresh`、按游戏 `discovery/` 写入、`cache/` 读写、来源事件触发全库 scan 的回填。
- Decky：`ScanSession` 中 refresh+scan 混合流程、`start_game_discovery` 关闭扫描会话、`_project_row`/`_refresh_rows` 整库重投影、旧 RPC 转发。
- Windows：`refresh_sources`/`scan_sources`/`discover_game` 命令与前端调用、共享 source task 槽位的旧语义。
- 保留：按源索引、详情缓存、匹配记录、自动更新调度、取消、DLC、身份、人工选择与安装内容绑定契约测试，改接新入口。

## 12. P1 自动化门槛

1. `SourceIndex`/`DetailRecord`/`GameMatchRecord` 往返与原子发布；失败/取消不覆盖成功快照。
2. schema 1 → 2 迁移：settings、patch.moe 缓存、Kagura 索引、逐游戏 discovery 候选、status；迁移失败保留原文件并报错；迁移幂等。
3. `input_key` 与来源 `index_revision` 只在真实输入变化时改变（更新时间、页面查看不改变）。
4. 同一规范 URL 的详情只保存一份；不同游戏共用。
5. `cargo test -p steam-patch-core`、`cargo run -p steam-patch-contract-gen -- check` 通过。

## 13. 实现修订（P5/P6，2026-10-19；保留 §1–§12 冻结内容）

本节只记录实现与冻结契约的差异，不重写已冻结条款；后续若要统一契约，应另立修订版本。

1. **`SourceStatus` 落盘字段**：§4 期望 status 含 `revision/complete`；实现落盘 `checked_at/cached_at/entry_count/code`，`revision/complete` 由 `index/<source>.json`（`SourceIndex`）得出，并通过 `SourceSummary`（`revision/complete/interval_hours/next_due_at`）与 `update_source` 结果对外暴露（`crates/core/src/sources/model.rs`、`registry.rs::list_sources`、`update.rs::SourceUpdateOutcome`）。语义（失败不更新成功快照）与 §4 一致。
2. **`DetailRecord` 形状**：§6 写 `candidates/diagnostics/index_revision`；实现存 `fact: PageFact`（query 无关的页面事实：标题/规范 URL/kind/steam_ids/aliases/mirrors/text/正文链接/分类内联文章/next_page）+ `index_revision/complete/diagnostics`。候选在匹配任务中按游戏由 `identity::build_candidates` 生成；这是 design §9 “索引与详情分层”的实现落点，详情缓存仍按来源+规范 URL 跨游戏共享。
3. **`GameMatchStatus::failed` 可达性**：§7.1 第 5 条“全部失败 → failed”。实时匹配中，普通源详情获取失败被记录为 `SourceMatchStatus::detail_incomplete`，HGamer 失败为 `FallbackMatchStatus::failed`；`derive_game_status` 在“普通源不完整 + fallback 失败”时输出 `incomplete`（保留诊断），不会输出 `failed`。`failed` 目前只由 v1 迁移记录（`detail_failed` + `failed`）产生。P6 按实现行为断言并记录为偏差；未改状态机。
4. **矩阵 8 提交绑定与 `source_not_enabled`**：`update_source` 在完整采集后、发布索引前重读持久化 settings；若源已被停用，则保存 collected pages 为 `index-progress`、不写正式索引/成功状态，返回错误码 `source_not_enabled`（UI 已有同名文案）。`match_games`/`rematch_game`/`rematch_source` 在保存 `matches/<appid>.json` 前比较 settings 修订；若修订变化且该游戏重算的 `input_key` 变化，则丢弃本次结果并保留上次已提交记录（无记录则保持 pending）。无关设置（他源 interval 等）变化不丢弃结果。该绑定是 P6 对 §4.3 的最小实现补充。
5. **Kagura 索引入口（已修复）**：原实现 `collect_index` 只遍历 `rule.pages.first()`（或 `rule.url`），`sources.json` 中 `kagura-en` 的第二入口 `product-category/patches/patch/` 未被消费。修复后无 progress 时从 `rule.url` + `rule.pages` 全部入口起爬（canonical 去重），每个入口各自沿 `next_page` 分页；fixture `kagura_en_crawls_every_configured_entry_root` 通过（3 列表页、4 条目、complete=true、不抓详情）。真实全量更新耗时/条目合并结果仍未跑。
6. **HGamer 真实站点（已修复）**：当前站点把 `<p>「term」找到 N 筆結果</p>` 放在 h1 兄弟 section，原“h1 父节点摘要”契约不成立；实现改为 `result_summary` 扫描含查询词且可解析计数的 `<p>`，找不到回退整页文本，不解析 RSC。`hgamer_live` 真实网络测试通过：1250180 为 3 逻辑请求、complete、candidates=0、identities=1（完整无结果）；1227890 为 5 逻辑请求、complete、candidates=2、identities=1（appid 匹配，镜像为 kagurafan web）。原“未验证/阻塞”结论作废。
7. **协议 v5 与宿主**：`PROTOCOL_VERSION=5`、20 方法、4 类事件已按 §9/§9.1 实现；`GameMatch` wire 不含 fallback 子状态（宿主只显示顶层六态），HGamer 的子状态仅存在于持久化 `matches/` 与匹配诊断中。
8. **`update_source` 结果字段**：在 §9 的 `SourceUpdateResult{source_id, settings_revision, status, affected_appids}` 基础上额外返回 `revision/complete`（当前正式快照的修订与完整性），供宿主刷新汇总；不是新方法或新协议版本。
