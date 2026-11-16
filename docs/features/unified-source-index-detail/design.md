# 订阅源缓存与抓取模型统一重构（Index -> Detail + Lua Step 推进）

状态：**待实现（仅完成架构调研、方案设计与过度设计审阅）**。施工与验证记录见 [progress.md](progress.md)。当前开发分支：`feature/unified-source-index-detail`（基于 `feature/script-source-adapters@c0e94d1` 创建）。

## 1. 背景与核心问题

当前 `crates/core/src/sources/` 中的订阅源索引与详情模型按早期预设的网页抓取形态划分，存在两处核心耦合：

1. **双层下载字段与双轨候选生成分支**：
   - [model.rs](../../../crates/core/src/sources/model.rs) 中 `SourceIndexEntry`（`url`、`language`、`date`）与 `PageFact`（`mirrors`、`published_at`、`text`、`steam_ids`）两层各自携带下载与元数据字段，通过 `kind = "entry" | "product" | "article" | "category"` 区分。
   - [matching.rs](../../../crates/core/src/sources/matching.rs) 拆分为 `match_entry_rows`（调用 `select_entries` + [registry.rs](../../../crates/core/src/sources/registry.rs) 的 `candidate_from_index_match`）和 `match_page_entries`（调用 `resolve_fact` + `candidates_from_fact` + [identity.rs](../../../crates/core/src/sources/identity.rs) 的 `build_candidates_with_download_group`）两套独立分支；[registry.rs](../../../crates/core/src/sources/registry.rs) 的 `index_candidates` / `index_matches` 也单独依赖 `kind == "entry"`。
2. **宿主混杂特定网站拓扑与硬编码两级跳转状态机**：
   - `PageFact` 混杂了业务结果与 Kagura 国内 WordPress 站点的网页导航拓扑字段（`article_links: Vec<FactLink>`、`article_facts: Vec<PageFact>`、`next_page: Option<String>`）。
   - [matching.rs](../../../crates/core/src/sources/matching.rs) 的 `process_category` 在 Rust 核心里写死了 `category -> next_page -> article_facts / article_links` 的两级跳转状态机；[adapters/mod.rs](../../../crates/core/src/sources/adapters/mod.rs) 的 `discover` 也包含遍历 `article_facts` 的逻辑。遇到一步到位或需要 2~3 次以上跳转的站点结构时，宿主无法用统一模型自然表达。

## 2. 目标与范围

1. **固化统一的 `Index -> Detail` 纯业务数据结构**：
   - **`SourceIndexEntry`（游戏索引层）**：只保留目录索引与匹配所需字段（`title`、`aliases`、`page`、`appid`）、抓取进度字段（`current_step`、`total_steps`、`step_data`）以及已就绪的 `details: Vec<SourceDetail>`。
   - **`SourceDetail`（补丁详情层）**：取代 `PageFact`、`FactLink`、`DiscoveryPage` 及 `SourceIndexEntry` 内联下载字段，固化纯补丁业务数据（`title`、`page`、`steam_ids`、`aliases`、`mirrors`、`single_download_group`、`text`、`languages`、`published_at`、`body_links`、`reported_format`、`upstream_page`）。
   - **移除宿主网页拓扑与双轨分支**：从 Rust 模型与调度中彻底移除 `kind = "entry" | "product" | "article" | "category"`、`article_links`、`article_facts`、`next_page`、`FactLink`、`DiscoveryPage`，删除 `process_category`、`select_entries`、`match_entry_rows`、`candidate_from_index_match`，统一从 `SourceIndexEntry + SourceDetail` 生成 `SourceCandidate`。
2. **由 Lua 掌控抓取步数（Step）与推进逻辑**：
   - **一步到位型源（`patchmoe`）**：在 `collect_index`（Step 1）直接填好 `SourceIndexEntry` 与 `details: Vec<SourceDetail>`（`current_step = 1, total_steps = 1`）。匹配与离线投影直接读取已就绪的 `details` 生成候选，0 次网络请求。
   - **多步按需型源（`kagura-cn`、`kagura-en`、`yozakura`）**：在 `collect_index`（Step 1）只构建 `SourceIndexEntry`，将 `details` 留空并记录 `current_step = 1, total_steps = N`（及可选 `step_data`）；匹配命中且详情未完成时，宿主循环调用 Lua 的 `resolve_detail(ctx, rule, state)` 逐步推进直至 `current_step == total_steps`，并将进度与最终 `details` 持久化至 `DetailRecord`。
   - **按游戏补查源（`hgamer`）**：`discover(ctx, rule, query, previous)` 直接产出统一的 `details: Vec<SourceDetail>` 与 `identities`，复用同一套候选生成管线。
3. **保持单文件 Lua 与空内置可恢复契约**：
   - 内置来源与自定义导入来源共用同一套 Lua 接口和校验边界；清空构建时的内置来源注册后，仍可通过普通导入入口仅凭仓库中的 5 份单文件 Lua 原文与规则恢复全部行为。

## 3. 核心数据模型设计

### 3.1 `SourceDetail`（补丁详情层）

在 [model.rs](../../../crates/core/src/sources/model.rs) 中定义统一的 `SourceDetail`，替代 `PageFact`、`FactLink` 与 `DiscoveryPage`：

```rust
/// One resolved patch detail (replaces `PageFact`, `FactLink`, `DiscoveryPage`
/// and `SourceIndexEntry` inline download fields).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDetail {
    pub title: String,
    pub page: String,
    #[serde(default)]
    pub steam_ids: Vec<u32>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub mirrors: Vec<Mirror>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub single_download_group: bool,
    #[serde(default)]
    pub text: String,
    /// Explicit languages supplied by the source (e.g. `["zh-CN"]` on patchmoe);
    /// when empty, inferred from mirror URLs or `text`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Normalized release date (`"YYYY-MM-DD"` / `"YYYY/M/D"`) or `"COMING SOON"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    /// Absolute URLs inside the detail body, used for base/DLC cross-link association.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_links: Vec<String>,
    /// Optional format reported by a fallback indexer (`"unknown" | "zip" | "exe" | "web"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_format: Option<String>,
    /// Optional upstream publisher URL referenced by a fallback indexer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_page: Option<String>,
}
```

### 3.2 `SourceIndexEntry`（游戏索引层）

在 [model.rs](../../../crates/core/src/sources/model.rs) 中重构 `SourceIndexEntry`：

```rust
fn default_step() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIndexEntry {
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub page: String,
    #[serde(default)]
    pub appid: Option<u32>,
    #[serde(default = "default_step")]
    pub current_step: u32,
    #[serde(default = "default_step")]
    pub total_steps: u32,
    /// Opaque Lua context for the next step; untouched by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_data: Option<serde_json::Value>,
    /// Populated immediately when `current_step == total_steps` in Step 1 (e.g. `patchmoe`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<SourceDetail>,
}

impl SourceIndexEntry {
    pub fn is_complete(&self) -> bool {
        self.total_steps >= 1 && self.current_step >= self.total_steps
    }
}
```

### 3.3 `DetailRecord`（按需详情与 Step 进度缓存）

在 [model.rs](../../../crates/core/src/sources/model.rs) 中将 `DetailRecord`（落盘于 `details/<source_id>/<key>.json`）对齐到 Step 模型：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailRecord {
    pub schema_version: u32,
    pub source_id: String,
    pub page: String,
    pub index_revision: u64,
    pub checked_at: u64,
    pub current_step: u32,
    pub total_steps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_data: Option<serde_json::Value>,
    #[serde(default)]
    pub details: Vec<SourceDetail>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl DetailRecord {
    pub fn is_complete(&self) -> bool {
        self.total_steps >= 1 && self.current_step >= self.total_steps
    }
}
```

## 4. Lua 接口与 Step 推进契约

### 4.1 脚本导出入口

| 脚本入口 | 适用角色 | 输入参数 | 返回结构与契约 |
| --- | --- | --- | --- |
| `collect_index(ctx, rule, previous)` | `publisher` | `previous`: `{ entries, pending, complete, etag, modified }` 或 `ctx.null` | `{ entries: Vec<SourceIndexEntry>, pending: Vec<String>, complete: bool, etag, modified, not_modified, diagnostics }`。一步到位型源返回 `current_step = 1, total_steps = 1` 及非空 `details`；多步按需型源返回 `current_step = 1, total_steps >= 2`、可选 `step_data` 及空 `details` |
| `resolve_detail(ctx, rule, state)` | `publisher`（仅当存在 `current_step < total_steps` 条目时必需） | `state`: `{ title, aliases, page, appid, current_step, total_steps, step_data, details }` | `{ current_step: u32, total_steps: u32, step_data: Option<Value>, details: Vec<SourceDetail>, diagnostics: Vec<ScriptDiagnostic> }`。每次调用由 Lua 读取 `state.current_step` 与 `state.step_data` 推进下一步抓取；支持动态调整 `total_steps`（如分类页发现未展开子文章链接时将 `total_steps` 从 `2` 扩为 `3`） |
| `match_appid(ctx, rule, site_ids, query)` | 可选 | `site_ids: Vec<u32>`, `query: QueryContext` | `bool` 或 `nil`（缺省时宿主按 `site_ids.contains(&query.appid)` 判定） |
| `discover(ctx, rule, query, previous)` | `fallback_indexer` | `query`, `previous` | `{ details: Vec<SourceDetail>, identities: Vec<IdentityFact>, complete: bool, diagnostics: Vec<ScriptDiagnostic> }` |

### 4.2 边界校验与 Step 推进循环

1. **宿主校验函数 `validate_detail(rule, detail)`**：
   - `detail.title.trim()` 非空；`detail.steam_ids` 不含 `0`；
   - `detail.page` 必须通过 `validate::allowed_url(&detail.page, &rule.page_hosts, false)` 并规范化为 `canonical_page_url`；
   - `detail.mirrors` 中每项 `mirror.url` 必须通过 `validate::navigation_url`（若 `!rule.discovery_only` 且为一步到位源，至少包含一个通过 `validate::allowed_url(url, &rule.download_hosts, true)` 的下载镜像），并由 `identity::format(&mirror.url)` 归一 `mirror.format`；
   - `detail.reported_format` 若存在，必须属于 `"unknown" | "zip" | "exe" | "web"`；
   - `detail.upstream_page` 若存在，必须通过 `validate::navigation_url`；
   - `detail.published_at` 若存在且非 `"COMING SOON"`，在一步到位表格源或由脚本调用 `ctx.normalize_date` 时保持合法日期格式。
2. **`collect_index` 契约校验**：
   - 每条 `entry` 满足 `entry.title.trim()` 非空、`entry.appid != Some(0)`、`entry.page` 在 `rule.page_hosts` 内；
   - 步数满足 `1 <= entry.current_step <= entry.total_steps`；
   - 若 `entry.is_complete()`（`current_step == total_steps`），`entry.details` 必须非空，且每条 `detail` 通过 `validate_detail(rule, detail)`。
3. **`resolve_detail` 宿主推进循环（[matching.rs](../../../crates/core/src/sources/matching.rs)）**：
   - 命中 `entry` 时，若 `entry.is_complete()`，直接使用 `entry.details`，不触发网络请求或 Lua 调用。
   - 若 `!entry.is_complete()`：
     1. 记录 `entry.page` 到 `detail_pages`；
     2. 查询 `store.load_detail(&rule.id, &entry.page)`：当 `!force_details` 且存在 `record.index_revision == index.revision` 的记录时，若 `record.is_complete()` 则直接返回 `record.details`；若未完成则以该 `record` 的 `(current_step, total_steps, step_data, details)` 作为断点初态；否则以 `entry` 的字段作为初态。
     3. 当 `state.current_step < state.total_steps` 时循环：
        - 检查 `cancel`；
        - 调用 `adapter.resolve_detail(rule, &state, ctx, cancel)`；
        - 校验返回的每个 `SourceDetail`；
        - **前进性不变量**：若返回的 `next.current_step <= state.current_step` 或 `next.current_step > next.total_steps`：
          - 若返回携带 `diagnostics`（如中途页面请求失败导致当前 Step 未能完成），保存已取得的 `next.details` 与 `next.step_data` 到 `DetailRecord`（保持 `current_step < total_steps`），记录 `diagnostics` 并标记 `detail_failed = true` 后退出循环；
          - 否则报 `script_source_contract_error`（`invalid_detail_step`），防止无进展死循环。
        - 若 `next.current_step > state.current_step`：更新 `state` 并原子落盘 `DetailRecord`（实现 Step 级断点续传）。
     4. 循环完成（`state.current_step == state.total_steps`）后，若 `state.details` 中包含 `detail.page != entry.page` 的独立子文章详情（如分类页解析出的内联文章或子链接文章），同时按 `detail.page` 写入一份已完成（`current_step = 1, total_steps = 1`）的 `DetailRecord`，供跨入口直接复用。

## 5. 统一匹配与候选生成流水线

1. **消除双轨匹配分支（[matching.rs](../../../crates/core/src/sources/matching.rs)）**：
   - 删除 `select_entries`、`match_entry_rows`、`match_page_entries`、`process_category`。
   - `match_publisher_source` 统一流程：
     - 过滤 `index.entries` 中满足 `entry_relevant(entry, query, known)` 的条目，按非 DLC 变体优先排序；
     - 对已完成的 `entry` 直接取 `entry.details`，对未完成的 `entry` 通过 `resolve_entry_details` 推进获取 `details`；
     - 对同一来源下已完成的一步到位直链详情，若多条语言记录指向同一归档 URL，按 `zh-CN` 优先合并（保持多语表格行去重语义）；
     - 统一调用 `candidates_from_detail` -> `identity::build_candidates_with_download_group` 生成 `SourceCandidate`；
     - 若第一轮仅扫描了 `total_steps <= 2` 的直接条目且已获得可信候选（且 `query.related_dlc` 为空），可跳过同源剩余更深层级的分类条目，保持原有最少请求行为。
2. **统一候选构造函数（[identity.rs](../../../crates/core/src/sources/identity.rs) 与 [registry.rs](../../../crates/core/src/sources/registry.rs)）**：
   - 删除 [registry.rs](../../../crates/core/src/sources/registry.rs) 中的 `candidate_from_index_match` 与 `source_match_from_entry`。
   - `identity::build_candidates_with_download_group` 统一接收 `SourceDetail`（或对应字段）：
     - 若 `detail.published_at` 为 `"COMING SOON"`（忽略大小写），跳过不生成候选；
     - 若 `detail.languages` 非空，优先采用该显式语言列表；为空时从镜像 URL 和 `detail.text` 推断；
     - 当 `confirmed_ids` 非空时 `match_basis = MatchBasis::Appid`；当 `!rule.discovery_only`（如 `patchmoe`）或关联 DLC 精确标题命中时，`game_appid = Some(query.appid)` 且 `match_basis = MatchBasis::OfficialTitle`；当命中 `upstream_identity` 时 `match_basis = MatchBasis::VerifiedAlias`；
     - 统一填充 `published_at`、`upstream_page`、`format_reported`、`format_observed`、`candidate_id` 与 `fingerprint`。
   - `registry.index_candidates(game)` 与 `registry.index_matches(game, include_disabled)` 直接遍历 `index.entries` 中 `entry.is_complete()` 且命中的条目，调用同一套离线 `SourceDetail -> SourceCandidate` 构建函数，不再依赖 `kind == "entry"`。

## 6. 缓存兼容与失效策略

1. **On-disk Schema 版本递增**：
   - [mod.rs](../../../crates/core/src/sources/mod.rs) 中 `SOURCE_INDEX_SCHEMA_VERSION` 由 `1` 升为 `2`，`SOURCE_DETAIL_SCHEMA_VERSION` 由 `1` 升为 `2`。
   - [store.rs](../../../crates/core/src/sources/store.rs) 的 `load_index`、`load_index_progress`、`load_detail` 检查顶层 `schema_version`：
     - 当 `schema_version < current_version`（即升级前遗留的 v1 索引/详情文件）时，直接返回 `Ok(None)`，视为待刷新缓存，不因旧字段（`kind`、`fact` 等）触发 `deny_unknown_fields` 报错阻断；
     - 当 `schema_version > current_version`（如未知高版本 `99`）或 JSON 格式损坏时，继续严格返回错误（保持 `preload_snapshot_rejects_corrupt_schema_versions` 等契约）。
2. **解析身份与安装选择绑定保持不变**：
   - 内置脚本代码更新后，`ScriptDescriptor.sha256`、`rule_fingerprint` 与 `parser_fingerprint` 自动更新；
   - `current_record` 自动淘汰旧 `parser_fingerprint` 的 `GameMatchRecord` 条目；
   - [installer/resolve.rs](../../../crates/core/src/installer/resolve.rs)、[exe_resolve.rs](../../../crates/core/src/installer/exe_resolve.rs)、[manual_selection.rs](../../../crates/core/src/installer/manual_selection.rs) 继续通过 `dynamic_zip_candidates` 与 `parser_fingerprint` 验证候选，无需修改安装器协议。
3. **v1 历史迁移辅助函数对齐**：
   - [index.rs](../../../crates/core/src/sources/index.rs) 中的 `from_cache` 生成 `current_step = 1, total_steps = 1` 且带 `details: vec![SourceDetail { ... }]` 的 `SourceIndexEntry`；
   - `from_discovery_index` 生成 `current_step = 1, total_steps = 2` 且 `details: vec![]` 的 `SourceIndexEntry`。

## 7. 五个内置 Lua 脚本迁移映射

| 脚本文件 | 角色与步数 | `collect_index` (Step 1) | `resolve_detail` (Step 2..N) / `discover` |
| --- | --- | --- | --- |
| [patchmoe.lua](../../../crates/core/resources/source-scripts/patchmoe.lua) | `publisher`，一步到位（`total_steps = 1`） | 抓取首页与三语表单，按下载 URL 归并标题与 `aliases`，直接构造 `current_step = 1, total_steps = 1` 且含完整 `details = { { title, page, aliases, steam_ids, mirrors = {{ url, format = "zip" }}, languages = { language }, published_at = date } }` 的条目 | 无需 `resolve_detail`（Step 1 已完成） |
| [kagura-en.lua](../../../crates/core/resources/source-scripts/kagura-en.lua) | `publisher`，两步按需（`total_steps = 2`） | 遍历多入口目录与分页，产出 `current_step = 1, total_steps = 2, details = {}` 的商品索引条目 | Step 1 -> 2：抓取 `state.page`，提取标题、`#tab-patch` 正文、`steam_ids`、`mirrors`、`published_at`、`body_links`，返回 `current_step = 2, total_steps = 2, details = { detail }` |
| [yozakura.lua](../../../crates/core/resources/source-scripts/yozakura.lua) | `publisher`，两步按需（`total_steps = 2`） | 请求 WP REST API，保留 `?p=` 文章地址与单 AppID，产出 `current_step = 1, total_steps = 2, details = {}` 的条目 | Step 1 -> 2：抓取 `state.page`，提取正文与下载镜像，返回 `current_step = 2, total_steps = 2, details = { { ..., single_download_group = true } }`；保留 `match_appid` |
| [kagura-cn.lua](../../../crates/core/resources/source-scripts/kagura-cn.lua) | `publisher`，2~3 步动态按需（文章 `total_steps = 2`，分类 `total_steps = 2 -> 3`） | 遍历首页与分页：文章条目设 `current_step = 1, total_steps = 2`；分类条目设 `current_step = 1, total_steps = 2, step_data = { kind = "category" }` | 文章条目：Step 1 -> 2 直接抓详情页完成。分类条目：Step 1 -> 2 抓分类页（含分类分页），提取内联文章 `details` 并收集未展开子文章链接 `links`；若 `links` 为空则返回 `current_step = 2, total_steps = 2` 完成；若 `links` 非空则返回 `current_step = 2, total_steps = 3, step_data = { kind = "articles", links = links }`；Step 2 -> 3 抓取 `links` 中的子文章详情并追加至 `details`，返回 `current_step = 3, total_steps = 3` |
| [hgamer.lua](../../../crates/core/resources/source-scripts/hgamer.lua) | `fallback_indexer`，按游戏补查 | 无 `collect_index` | `discover(ctx, rule, query, previous)`：搜索词条、验证游戏页 `identities`、抓取补丁页，直接返回 `{ details: Vec<SourceDetail>, identities, diagnostics, complete }` |

## 8. 分阶段实施与验收计划

| 阶段 | 实施范围 | 定向验证 |
| --- | --- | --- |
| P1：核心数据模型与存储缓存层重构 | 修改 `model.rs`、`mod.rs`、`index.rs`、`store.rs`：以 `SourceDetail` 替换 `PageFact`/`FactLink`/`DiscoveryPage`，重构 `SourceIndexEntry` 与 `DetailRecord`，递增 `SOURCE_INDEX_SCHEMA_VERSION` / `SOURCE_DETAIL_SCHEMA_VERSION` 至 `2`，更新 `normalize_entries`、`from_cache`、`from_discovery_index` 及版本化 JSON 读取 | `cargo test -p patchhub-core --test source_data_v2` 及 `sources::index::tests` |
| P2：Lua 执行器与候选生成统一 | 修改 `adapters/script.rs`、`adapters/mod.rs`、`identity.rs`、`registry.rs`、`matching.rs`：将 `resolve_page` 替换为 `resolve_detail`（含 Step 循环与前进性校验），`discover` 切换为 `details: Vec<SourceDetail>`，删除 `process_category`、`match_entry_rows`、`select_entries`、`candidate_from_index_match`，统一 `Index + Detail -> SourceCandidate` | `cargo test -p patchhub-core --test script_source_runtime --test script_source_pipeline --test script_source_fallback` |
| P3：五个内置 Lua 脚本迁移与夹具更新 | 迁移 `patchmoe.lua`、`kagura-cn.lua`、`kagura-en.lua`、`yozakura.lua`、`hgamer.lua`（移除冗余未使用的 `row_entry`/`kind` 模板代码），更新录制基线与单元测试夹具 | `cargo test -p patchhub-core --test source_update --test source_matching --test source_acceptance --test builtin_script_migration` 及 `cargo test -p patchhub-cli` |
| P4：格式、静态检查、过度设计复核与空内置导入验证 | 执行 `cargo fmt --all -- --check`、受影响包 `cargo clippy --all-targets -- -D warnings`，执行实现后的 `$overengineering-review` 代码复核，验证空内置构建下仅凭仓库 5 份单文件 Lua 导入恢复全部行为 | 双平台/本地定向测试、Clippy、空内置恢复测试与实机待验收项盘点 |

## 9. 过度设计审阅结论

已按 [overengineering-review](../../../.agents/skills/overengineering-review/SKILL.md) 对本设计方案完成静态审阅，结论为：**未发现需修改项（方案已收敛至最小必需设计）**。关键审阅记录如下：

1. **无宿主状态机或反射分派膨胀**：宿主仅维护 `current_step`、`total_steps` 两个整数与透传的 `step_data: Option<Value>`，通过单一 `resolve_detail(ctx, rule, state)` 入口循环推进；不在宿主引入 Step 枚举、多入口函数反射或跨任务协程恢复框架，具体步骤分支完全由脚本内部局部函数处理。
2. **彻底清理双轨结构与旧拓扑字段**：移除 `PageFact`、`FactLink`、`DiscoveryPage`、`kind`、`article_links`、`article_facts`、`next_page` 以及 `match_entry_rows` / `candidate_from_index_match` 双轨路径，Publisher（一步或N步）与 Fallback Indexer 均汇入同一个 `SourceDetail -> SourceCandidate` 构造函数。
3. **不设无依据的硬上限**：不对 `total_steps` 步数、`step_data` 大小或 `details` 数量设置人为上限；仅通过 `1 <= current_step <= total_steps` 与单步前进性校验（`next.current_step > state.current_step`）结合现有取消信号防止死循环。
4. **最小化的旧缓存淘汰路径**：利用 `SOURCE_INDEX_SCHEMA_VERSION = 2` 与 `SOURCE_DETAIL_SCHEMA_VERSION = 2` 在读取时将 `< 2` 的旧缓存直接视为空（`Ok(None)`），无需编写复杂的旧版 `PageFact` 结构体兼容转换器，同时避免旧缓存触发 `deny_unknown_fields` 阻断升级后的正常启动与更新。
