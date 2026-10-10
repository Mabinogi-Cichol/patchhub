# P0/P1 共享来源引擎 Rust 契约（实现规范）

设计总览：[design.md](design.md)。本文件是 P0 基线契约与 P1（规则 / 订阅 / 存储）的**实现规范**，由独立架构审阅产出并经主 Agent 用真实 Python 复核（golden 已实测）。P2/P3 只给出接口桩，不得在本轮实现中重定义本文件的冻结值。

状态：**待实现**。日期：2026-10-07（Asia/Shanghai）。

## 0. P0 冻结契约（P1 实现、P2/P3 消费，不得再改）

| 常量 | 值 | Python 名 / 出处 |
| --- | --- | --- |
| `MAX_HTML` | `4 * 1024 * 1024` (4 MiB) | `MAX_HTML` |
| `MAX_ENTRIES` | `4096` | `MAX_ENTRIES` |
| `MAX_SOURCES` | `32` | `MAX_SOURCES` |
| `MAX_DISCOVERY_INPUT` | `32 * 1024` (32 KiB) | `MAX_DISCOVERY_INPUT` |
| `MAX_DISCOVERY_OUTPUT` | `512 * 1024` (512 KiB) | `MAX_DISCOVERY_OUTPUT` |
| `MAX_OVERRIDES_BYTES` | `256 * 1024` | 硬编码 |
| `MAX_STATUS_BYTES` | `4096` | 硬编码 |
| `MAX_CACHE_BYTES` | `4 * 1024 * 1024` | 硬编码 |
| `MAX_CANDIDATES` | `128` | `len(candidates) > 128` |
| `MAX_MIRRORS_PERSIST` | `32` | `_validate_candidates` |
| `MAX_MIRRORS_WORKER` | `64` | `discovery.validate_candidate` |
| `MAX_EVIDENCE` | `64` | `identity_evidence/provenance` |
| `PARSER_VERSION` | `2` | `DISCOVERY_PARSER_VERSION` |
| `QUERY_VERSION` | `1` | `DISCOVERY_QUERY_VERSION` |
| `DEFAULT_RULES_SCHEMA` | `2` | `sources.json` |
| `STORE_SCHEMA_VERSION` | `1` | 新 `rust-v1` 文件 |
| `SOURCE_CACHE_TTL` | `24 * 3600` | `_refresh_one` |
| `DISCOVERY_SUCCESS_TTL` | `6 * 3600` | `cached_candidates` |
| `DISCOVERY_FALLBACK_TTL` | `24 * 3600` | fallback 且有 candidates |
| `DISCOVERY_INDEX_TTL` | `24 * 3600` | `_game_previous` |
| `OBSERVED_TTL` | `6 * 3600` | cache_status fresh/stale |
| `REFRESH_WORKER_TIMEOUT` | `180s` | `refresh` |
| `DISCOVERY_CONTEXT_DEADLINE` | `29s` | `RequestContext(deadline=+29)` |
| `DISCOVERY_CONTEXT_BUDGET` | `24` | `budget=24` |
| `OFFICIAL_SUBDEADLINE` | `6s` | `min(deadline, now+6)` |
| `OFFICIAL_SUBBUDGET` | `requests + 3` | `min(budget, requests+3)` |
| `FETCH_TIMEOUT` | `20s` | `_fetch` |
| `DETAIL_REQUEST_LIMIT` | `6` | `context.detail_requests >= 6` |
| `PAGINATION_SEEN_LIMIT` | `100` | `len(seen) < 100` |

维护适配器 id 白名单（`overrides.added` 只能引用）：

```text
BUILTIN_ADAPTERS = ["patchmoe", "table_html", "kagura_wordpress", "kagura_woocommerce", "hgamer_html"]
```

> `kagura_cn_html` / `kagura_en_html` 是 discovery.py 内部实现名，**不是** 规则 `adapter` 取值。
> discovery 适配器集合：`DISCOVERY_ADAPTERS = {kagura_wordpress, kagura_woocommerce, hgamer_html}`。

## 1. 模块布局 `crates/core/src/sources/`

```text
crates/core/src/sources/
├── mod.rs          P1  常量、错误码、re-export、Sources 门面、Clock
├── model.rs        P1  serde + JsonSchema 类型
├── validate.rs     P1  纯校验与 URL/host/名称原语（无 IO）
├── canonical.rs    P1  与 Python json.dumps 等价的 python_json + 哈希输入
├── registry.rs     P1  内存合并规则 + 订阅 + 来源缓存匹配
├── store.rs        P1  rust-v1 持久化、短锁、revision、导入
├── identity.rs     P2  候选 ID/fingerprint/merge/enrich（桩）
├── http.rs         P2  受限 fetcher trait/上下文（桩）
├── adapters/       P2  Adapter trait + patchmoe/table/kagura/hgamer（桩）
└── session.rs      P3  查源会话/队列/取消/发布检查（桩）
```

`lib.rs` 增加 `pub mod sources;`，核心不得依赖 Tauri / Decky / Python / 前端 appStore。

`validate.rs` 与 `canonical.rs` 是总览 layout 之外的两个 P1 模块，理由：校验原语需被 `registry`/`store`/P2 复用且无 IO；Python 兼容 JSON 序列化必须独立可 golden 测试。

### 1.1 门面与时钟

```rust
pub const PARSER_VERSION: u32 = 2;
pub const QUERY_VERSION: u32 = 1;
pub type Result<T> = std::io::Result<T>;

pub struct Sources { store: store::SourceStore, registry: registry::SourceRegistry }
impl Sources {
    pub fn open(state_dir: &Path) -> Result<Self>;              // include_str! 默认规则
    pub fn open_with_rules(state_dir: &Path, rules_json: &str) -> Result<Self>;
    pub fn registry(&self) -> &registry::SourceRegistry;
    pub fn registry_mut(&mut self) -> &mut registry::SourceRegistry;
    pub fn store(&self) -> &store::SourceStore;
    pub fn settings_revision(&self) -> u64;
}

pub trait Clock { fn now_unix(&self) -> u64; }
pub struct SystemClock;
pub struct FakeClock(AtomicU64);   // 测试 / fixture
```

## 2. 精确类型

输出/持久化类型字段名与 Python JSON 及 `packages/ui/src/index.ts` DTO **逐字一致**（snake_case，无需 rename）。`deny_unknown_fields` 用于规则与配置；持久化候选/envelope 的导入路径允许宽松解析（见 §4.5）。

### 2.1 规则

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceRole { Publisher, FallbackIndexer }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RootLayout { GameRoot }

/// 反序列化中间态：宽松字段类型，便于返回 Python 精确错误码。
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RawSourceRule {
    pub id: String,
    pub name: String,
    pub adapter: String,
    #[serde(default)] pub role: Option<String>,
    #[serde(default)] pub discovery_only: Option<bool>,
    #[serde(default)] pub priority: Option<i64>,
    pub enabled: bool,
    pub pages: Vec<String>,
    pub page_hosts: Vec<String>,
    #[serde(default)] pub download_hosts: Option<Vec<String>>,
    #[serde(default)] pub root_layout: Option<String>,
    #[serde(default)] pub columns: Option<Columns>,
    #[serde(default)] pub table_id: Option<String>,
    #[serde(default)] pub language: Option<String>,
    #[serde(default)] pub url: Option<String>,
}

/// 校验后规范态。字段名与 Python rule dict 一致。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRule {
    pub id: String,
    pub name: String,
    pub adapter: String,
    #[serde(default = "d_role")]     pub role: SourceRole,       // 默认 publisher
    #[serde(default)]                pub discovery_only: bool,   // 默认 false
    #[serde(default = "d_priority")] pub priority: u16,          // 默认 100
    pub enabled: bool,
    pub pages: Vec<String>,
    pub page_hosts: Vec<String>,
    #[serde(default)]                pub download_hosts: Vec<String>,
    #[serde(default)]                pub root_layout: Option<RootLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub columns: Option<Columns>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub table_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Columns {
    pub title: u8,
    pub download: u8,
    #[serde(default)] pub date: Option<u8>,
    #[serde(default)] pub language: Option<u8>,
    #[serde(default)] pub appid: Option<u8>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RulesConfig { pub schema_version: u32, pub sources: Vec<RawSourceRule> }
```

`validate_rule` 复刻 `_validate_rule` 顺序与错误码：`invalid_source_rule` / `invalid_source_id` / `invalid_source_name` / `unsupported_source_adapter` / `invalid_source_pages` / `invalid_source_hosts` / `invalid_source_host` / `invalid_source_url` / `invalid_source_columns` / `invalid_table_id`。

规则约束：`discovery_only==true` ⇒ `root_layout.is_none()` 且 `adapter ∈ DISCOVERY_ADAPTERS`；否则 `root_layout==GameRoot` 且 `role==Publisher`；`role==FallbackIndexer` ⇒ `discovery_only && download_hosts.is_empty()`。默认 `role=publisher`、`discovery_only=false`、`priority=100`；`enabled` 必填 bool。

### 2.2 订阅 / 设置 / 摘要

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    #[serde(default)] pub enabled: BTreeMap<String, bool>,
    #[serde(default)] pub added: Vec<SourceRule>,
    #[serde(default)] pub games: BTreeMap<String, BTreeMap<String, bool>>, // "<appid>" -> source_id -> subscribed
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingsFile {
    pub schema_version: u32,   // 1
    pub revision: u64,
    pub enabled: BTreeMap<String, bool>,
    pub added: Vec<SourceRule>,
    pub games: BTreeMap<String, BTreeMap<String, bool>>,
}

pub struct Settings { pub revision: u64, pub rules: Vec<SourceRule>, pub overrides: Overrides }
```

合并（复刻 `_reload`）：`added_ids = {added.id}`；`rules = builtins.filter(id ∉ added_ids) ++ added`；`≤32` 且 id 唯一，否则 `invalid_source_overrides`；对每个 rule，若 `overrides.enabled` 有该 id，覆盖 `rule.enabled`。缺 `games` == 全部默认订阅（**显式 `{}` 亦然**）。

```rust
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)] // 仅输出，不 derive Deserialize
pub struct SourceSummary {
    #[serde(flatten)] pub rule: SourceRule,
    pub status: String, pub entry_count: u64,
    pub cached_at: Option<u64>, pub checked_at: Option<u64>,
}
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct GameSourceSummary {
    #[serde(flatten)] pub source: SourceSummary, pub subscribed: bool, pub matched: bool,
}
```

### 2.3 来源缓存 / 状态

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceEntry {
    pub title: String,
    #[serde(default)] pub aliases: Vec<String>,
    pub url: String,
    pub source_page: String,
    pub language: String,
    #[serde(default)] pub date: Option<String>,   // "YYYY/M/D" | "COMING SOON"
    #[serde(default)] pub appid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceCache {
    pub entries: Vec<SourceEntry>,
    #[serde(default)] pub etag: Option<String>,
    #[serde(default)] pub modified: Option<String>,
    #[serde(default)] pub cached_at: Option<u64>,
    #[serde(default)] pub checked_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceStatus {
    pub checked_at: u64,
    #[serde(default)] pub cached_at: Option<u64>,
    #[serde(default)] pub entry_count: u64,
    pub code: String,
}
```

### 2.4 候选 / 记录（与 TS DTO 对齐）

```rust
#[serde(rename_all = "snake_case")] pub enum Variant { Base, Dlc, Demo, Unknown }
#[serde(rename_all = "snake_case")] pub enum MatchBasis { Appid, VerifiedAlias, OfficialTitle, Title, None }
#[serde(rename_all = "snake_case")] pub enum Capability { DiscoverOnly }
#[serde(rename_all = "snake_case")] pub enum ArtifactFormat { Zip, Exe, Web, Unknown, Mixed }
#[serde(rename_all = "snake_case")] pub enum CacheStatus { Fresh, Cached, Stale }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mirror { pub url: String, #[serde(default)] pub label: Option<String>, #[serde(default)] pub format: String }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdentityEvidence { pub kind: String, #[serde(default)] pub appid: Option<u32>, #[serde(default)] pub page: Option<String> }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Provenance { pub source_id: String, pub page: String }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceCandidate {
    pub candidate_id: String,        // candidate-[0-9a-f]{24}
    pub source_id: String,
    pub source_record_id: String,    // canonical_url(source_page)
    pub game_appid: Option<u32>,     // 恒在，可 null
    pub parent_appid: Option<u32>,   // 恒在，可 null
    #[serde(default)] pub aliases: Vec<String>,
    pub game_title: String,
    pub patch_title: String,
    pub variant: Variant,
    pub requires_dlc: Option<Vec<u32>>,  // 恒在，可 null
    pub excludes_dlc: Option<Vec<u32>>,  // 恒在，可 null
    #[serde(default)] pub platforms: Vec<String>,
    pub patch_kind: String,          // "adult_content"
    #[serde(default)] pub languages: Vec<String>,
    pub version_raw: Option<String>,
    #[serde(default)] pub version_evidence: Vec<String>,
    pub version_identity: String,
    pub published_at: Option<String>,
    pub patch_updated_at: Option<String>,
    pub source_page: String,
    pub upstream_page: Option<String>,
    #[serde(default)] pub mirrors: Vec<Mirror>,
    pub format_reported: ArtifactFormat,
    pub format_observed: ArtifactFormat,
    pub capability: Capability,
    #[serde(default)] pub identity_evidence: Vec<IdentityEvidence>,
    #[serde(default)] pub provenance: Vec<Provenance>,
    pub match_basis: MatchBasis,
    pub observed_at: String,         // ISO8601 字符串
    pub cache_status: CacheStatus,
    #[serde(default)] pub warnings: Vec<String>,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub related_index_pages: Vec<Provenance>,
}
```

**字段存在性契约**：`_candidate` 恒写入上表除 `related_index_pages` 外全部键（null 用 `Option` 序列化为 `null`）；`related_index_pages` 仅在 sources.py 合并阶段出现，故用 `skip_serializing_if`。`source_name` **不在持久化候选中**，由展示层注入。

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    #[serde(default)] pub source_id: Option<String>,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub detail: Option<String>,
}
```

### 2.5 发现记录 / 索引 envelope

`success` 与 `attempt` 字段集不同，用两个结构体精确表达：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverySuccess {
    pub status: String,          // complete|skipped
    pub candidates: Vec<SourceCandidate>,
    #[serde(default)] pub diagnostics: Vec<Diagnostic>,
    pub checked_at: u64,
    #[serde(default)] pub cached_at: Option<u64>,
    pub query_key: String,
    pub parser_version: u32,
    pub query_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryAttempt {
    pub status: String,          // complete|partial|failed|skipped
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub candidates: Vec<SourceCandidate>,
    #[serde(default)] pub diagnostics: Vec<Diagnostic>,
    pub checked_at: u64,
    #[serde(default)] pub cached_at: Option<u64>,
    pub query_key: String,
    pub parser_version: u32,
    pub query_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub index: Option<DiscoveryIndex>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub checkpoint: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub queries: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub query_cache: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryRecord {
    pub schema_version: u32,     // 1
    pub source_id: String,
    pub appid: u32,
    #[serde(default)] pub success: Option<DiscoverySuccess>,
    #[serde(default)] pub attempt: Option<DiscoveryAttempt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexRecord { pub title: String, pub page: String, pub kind: String } // product|article|category

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryIndex {
    pub parser_version: u32,
    #[serde(default)] pub records: Vec<IndexRecord>,
    #[serde(default)] pub pending: Vec<String>,
    #[serde(default)] pub checked_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryIndexRecord { pub schema_version: u32, pub checked_at: u64, pub index: DiscoveryIndex }
```

### 2.6 查询上下文 / 游戏输入

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GameQuery {
    pub appid: u32,
    pub name: String,
    #[serde(default)] pub localized_name: Option<String>,
    #[serde(default)] pub instance_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryContext {
    pub appid: u32,
    pub name: String,
    #[serde(default)] pub name_hints: Vec<String>,       // ≤3 × 256
    #[serde(default)] pub preferred_language: Option<String>,
    pub legacy_match: bool,
    pub force: bool,
    #[serde(default)] pub source_context: Vec<String>,
    pub parser_version: u32,   // 2
    pub query_version: u32,    // 1
    pub settings_revision: u64,
    #[serde(default)] pub generation: Option<u64>,
}
```

输入边界（复刻 `discover_game`）：`appid ∈ [1, 9999999999]`；`name` 非空 `≤1024`；`hints ≤3` 每个非空 `≤256`；`preferred_language` 匹配 `^[a-zA-Z]{2,3}(?:-[a-zA-Z]{2,4})?$`。

错误码统一为 `io::Error::new(ErrorKind::InvalidData, code)`，P1 全集：

```text
invalid_source_rule  invalid_source_id  invalid_source_name  unsupported_source_adapter
invalid_source_pages invalid_source_hosts invalid_source_host invalid_source_url
invalid_source_columns invalid_table_id  empty_or_oversized_source invalid_source_entry
invalid_source_language invalid_source_appid invalid_source_date invalid_source_cache
invalid_source_overrides source_limit unknown_source invalid_appid
source_exists_or_adapter_unavailable invalid_adapter
invalid_discovery_candidates oversized_discovery_candidates invalid_discovery_candidate
invalid_discovery_identity invalid_discovery_index invalid_discovery_cache source_not_enabled
linked_source_state source_state_busy invalid_sources_config
```

## 3. 默认规则资源 `crates/core/resources/sources.json`

新建；P1 期间 Python 侧 `apps/decky/backend/sources.json` 保留至 P6。编译期内联：

```rust
pub const DEFAULT_RULES_JSON: &str = include_str!("../resources/sources.json");
```

内容与现值完全一致（`schema_version: 2`，4 条来源）：patchmoe / kagura-cn / kagura-en / hgamer，字段逐字等同 `apps/decky/backend/sources.json`。

- 版本化：`schema_version ∈ {1,2}`。schema 1 只含 publisher 型规则，由 `validate_rule` 默认值补全（`discovery_only=false`、`root_layout=game_root`、`role=publisher`）。
- `add_source` 在 `_validate_rule` 后检查 `adapter ∈ BUILTIN_ADAPTERS`，否则 `source_exists_or_adapter_unavailable`；`added` 中 id 与内置冲突时移除内置、保留用户定义。

## 4. 存储设计（rust-v1）

### 4.1 命名空间

```text
<state_dir>/sources/            # Python 旧目录（只读导入；不双写）
│   overrides.json  cache/<id>.json  status/<id>.json
│   discovery/<id>/<appid>.json  discovery-index/<id>.json
└── rust-v1/                    # Rust 唯一可写命名空间
    ├── schema.json             # {"schema_version":1,"created_at":...}
    ├── settings.json           # SettingsFile（含持久 revision）
    ├── settings.lock           # 短锁（独立于 journal.lock）
    ├── engine-switch.json      # 双引擎切换检查点
    ├── conflicts/<seq>.json    # 重入冲突留存
    ├── cache/<id>.json  status/<id>.json
    └── discovery/<id>/<appid>.json  discovery-index/<id>.json
```

> 2026-10-11 P6 收口：树中 `engine-switch.json`、`conflicts/<seq>.json` 属双引擎切换残留，已被 [跨平台安装器迁移设计](../cross-platform-installer-migration/design.md) §7.3 第 2 行取代：单一 Rust 引擎不再读写这些文件。原文保留为 P1 冻结契约记录。

- **不复用 `journal.lock`**：来源子系统用独立 `rust-v1/settings.lock`；网络期间不持任何锁，避免 Windows 扫描文件占用回归。
- `sources/` 与 `rust-v1/` 均做链接/reparse 检查（Windows `FILE_ATTRIBUTE_REPARSE_POINT=0x400`），风格沿用 `catalog.rs::linked` / `download.rs::no_link`。

### 4.2 原子写原语

`write_atomic(path, bytes, max)`：`>max` → `file_too_large`；父链 `no_links`；`create_dir_all` + 目录 0o700；目标已存在则拒绝 linked/reparse/非普通（`linked_source_state`）；同目录 `.{name}.{uuid}.tmp` 以 `create_new` 打开、0o600、write + `sync_all`、`fs::rename` 原子替换（禁止先删后写）、失败清理 tmp、unix `sync_dir`。Windows 覆盖可能被杀软/句柄占用 → 有限重试（≤5 × 20ms）后 `source_state_busy`，绝不移除旧文件。

`read_bounded(path, max)`：不存在 → `None`；拒绝 linked/reparse/非普通；`>max` → `file_too_large`。

### 4.3 短锁 + 持久 revision + 发布检查

```rust
pub fn with_short_lock<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T>;  // try_lock，2s 超时 → source_state_busy
pub fn bump_revision(&self) -> Result<u64>;                                     // 短锁内 load → +1 → save
```

- **revision 语义变化（有意，R6）**：Python 为内存计数、重建归 0；Rust 必须持久、单调、原子递增。宿主视为不透明 `u64`，仅用于“捕获—重核—发布”。
- **发布检查**：会话/任务开始捕获 revision + 规则指纹；写缓存前在短锁内核对 `current_revision == captured`，否则丢弃（P3 消费，P1 提供原语）。
- **分页合并短锁**：`save_index` 在短锁内先读后写；仅当新 `parser_version==2 && checked_at >= existing` 才整体替换；`records` 按 `page` 去重、`pending` 去重、`records ≤ 4096`。

### 4.4 分层提交规则（复刻 `_run_discovery`）

`commit(source_id, appid, result, previous, key, clock)`：

1. 构造 `attempt`：`status/candidates/diagnostics/checked_at=now/query_key=key/parser_version=2/query_version=1`；`cached_at = now if complete else previous.success.cached_at`；`index/checkpoint/queries/query_cache` 依次取 `result → previous.attempt → previous.success`。
2. `success = attempt` 当 `status==complete || (skipped && previous.success.is_none())`，否则 `previous.success`；`success` 剔除 `index/checkpoint/queries/query_cache`。
3. `attempt.candidates` 仅 `status==partial` 保留。
4. 整条记录序列化 `> MAX_DISCOVERY_OUTPUT` → 跳过写入。
5. `index` 存在则写 `discovery-index/<id>.json`（`checked_at = result.index.checked_at || existing.checked_at || attempt.checked_at`），超 512 KiB 跳过。
6. 写 `status/<id>.json`：`source_refreshed | source_partial | source_offline_cached（有 success.candidates） | source_refresh_failed`。

不变量：**只有完整成功才替换 `success`**；partial/failed 只更新 `attempt` 并保留 success/检查点；**完整空结果**才写负缓存。

### 4.5 导入算法（Python overrides + 旧 cache 作 stale seed）

`ImportReport { imported_settings, imported_cache, imported_discovery, imported_index, diagnostics, revision }`。

1. 无 `rust-v1/schema.json` → 创建命名空间与标记，进入导入。
2. 读 Python `sources/overrides.json`（≤256 KiB、no_links）；`validate_overrides` 复刻 `_reload`；写 `SettingsFile { revision: 1, ... }`。**不原地改 Python 文件**。
3. 每个非 `discovery_only` 规则：读旧 `cache/<id>.json`（≤4 MiB）→ `validate_entries` 逐条校验；通过才作 **stale seed** 导入，保留原 `cached_at/checked_at`（**不伪造 fresh**）；否则诊断 `invalid_source_cache` 并跳过（保留原文件）。
4. 旧 `status/<id>.json`（≤4096）结构合法才导入，不提升为 complete。
5. 旧 `discovery/<id>/<appid>.json`（≤512 KiB）：校验 `schema_version==1/appid/source_id` + 候选逐条 `validate_candidates`；保留 provenance 与 observed_at；**降级 checked_at 使其过期**（触发重查）；无法确认 → 诊断 `invalid_discovery_cache`，不导入。
6. 旧 `discovery-index/<id>.json`：schema/parser_version 合法才导入，`checked_at` 保留但受 24h TTL。
7. 幂等：`schema.json` 存在即跳过整段（除 `--force-import`）。先写 `rust-v1/.import-<uuid>/` staging，全部成功再原子 rename 发布 + 完成标记；中断可重入。

### 4.6 双引擎切换检查点（P1 冻结数据模型）

`engine-switch.json`：

```json
{
  "schema_version": 1,
  "active_engine": "rust",
  "switch_seq": 3,
  "rust":   { "revision": 7, "settings_hash": "..." },
  "python": { "settings_hash": "...", "captured_at": 0 },
  "last_common": { "settings": { "enabled": {}, "added": [], "games": {} }, "hash": "...", "captured_at": 0 }
}
```

**每次切换**（非仅首次）记录 `active_engine`、`switch_seq`、双方摘要/版本、上次共同快照。重入：暂停 Python 任务 → 重读 Python overrides 与摘要 → 仅导入相对 `last_common` 变化的字段（含显式关闭/删除），未变字段保留 Rust 值 → 双方同字段/同 id 各自改动则留存 `conflicts/<seq>.json` 并报告，**禁止按墙钟静默覆盖** → 成功则原子发布设置 + `bump_revision` + 更新 `last_common`/`switch_seq+1`；失败继续原引擎，不删 Rust 状态、不覆写旧 cache。Rust→Python 回退前单独导出 Rust 设置并验证旧格式映射。

> 2026-10-11 P6 收口：本节 §4.6 双引擎切换检查点（`engine-switch.json` schema、每次切换摘要、冲突留存、Rust→Python 回退）整体被 [跨平台安装器迁移设计](../cross-platform-installer-migration/design.md) §7.3 第 2 行取代：单一 Rust 引擎，无引擎选择/切换/共同快照/旧引擎回退。原文保留为 P1 冻结契约记录。

## 5. ID / TTL / 限制 golden

### 5.1 python_json 与哈希输入（跨语言逐字节一致）

必须复刻 Python `json.dumps`：
- 默认分隔符 **`, ` / `: `**（`serde_json::to_string` 紧凑无空格，**不可直接用**）。
- `sort_keys` 时键按 **Unicode 码点**排序；`ensure_ascii=false` 时非 ASCII 原样 UTF-8。
- `None→null`、`bool→true/false`、整数十进制；字符串转义仅 `" \ \b \f \n \r \t` 与 `<0x20` 的 `\uXXXX`；`/` 不转义。
- 无浮点参与哈希。

| 输入 | 算法 |
| --- | --- |
| `candidate_id` | `"candidate-" + sha256(python_json(identity, sort_keys=false, ensure_ascii=false))[:24]`；`identity = [source_id, canonical_url(page), variant, languages_list, version_identity]`（有序）；`version_identity = (version or "page-unknown") + (":"+filename if filename else "")` |
| `fingerprint` | `sha256(python_json(候选dict 去 fingerprint/observed_at/cache_status, sort_keys=true, ensure_ascii=false)).hexdigest()`，键集 = Python 落盘时刻的键集 |
| `query_key` | `sha256(python_json({appid,name,name_hints,preferred_language,legacy_match,source_context,parser_version,query_version}, sort_keys=true, ensure_ascii=false)).hexdigest()` |

`canonical_url(u) = urlunsplit(scheme, netloc.lower(), path, "", "")`（丢 query/fragment，host 小写）。

**已实测 golden**（真实 Python 计算，见 `crates/core/tests/fixtures/sources/identity-golden.json`）：

```text
candidate_id(["official","https://publisher.example/game/","base",["en"],"page-unknown"])
  = candidate-f7f112b4599f264ff4eb69be
candidate_id(["kagura-cn","https://kagurafan.com/game/","dlc",["zh-CN"],"v1.2.3:patch_v1.2.3.zip"])
  = candidate-99461727558091c8fc6e5527
query_key(appid=42,name="A Game",hints=[],preferred=None,legacy=False,source_context=["official"])
  = a870a97d65b5391ec4a31ae7ddd9854c2fa745077f20e248128a3249ae4a60b4
fingerprint(hgamer base candidate, appid 42, v1 ZIP)
  = 39bc76a4c3eb68a5a4730a8f556f8ed4e7a6ee935d45cc20cc44141fc4022a9f
```

fixture 内含完整 payload 字符串与 canonical_url 期望，Rust 单测逐字节比对。

### 5.2 release_id

| 条件 | 格式 | 例 |
| --- | --- | --- |
| `source_id == "patchmoe"` | `patchmoe-<appid>` | `patchmoe-42` |
| 其它 publisher | `source-<source_id>-<appid>` | `source-second-42` |

`source_for_release` 前缀匹配 + 余部 `^[1-9][0-9]*$`；`discovery_only` 不参与。

### 5.3 TTL

| 层级 | TTL | 备注 |
| --- | --- | --- |
| 来源 cache refresh | 24h | `force` 绕过 |
| discovery success | 6h | cache_status `cached`/`stale` |
| discovery fallback（有 candidates） | 24h | `_run_discovery` |
| shared index | 24h | 需 `schema_version==1 && parser_version==2` |
| observed 新鲜度 | 6h | 展示 |
| freshness 还需 `parser_version==2 && query_version==1` | — | 版本变化即失效 |

`is_fresh(checked_at, ttl) = now >= checked_at && now - checked_at < ttl`（时钟回退 → 过期）。

### 5.4 限制总表

输入：`pages 1..=5`；`page_hosts 1..=8`；`download_hosts 0..=8`（仅 discovery_only 允许 0，fallback 必须空）；`columns 0..=20`；`name ≤128`；`hints ≤3×256`；`game.name ≤1024`；`candidates ≤128`；`mirrors ≤32`（持久）/`≤64`（worker）；`aliases/languages/warnings/version_evidence ≤64×1024`；`requires_dlc/excludes_dlc ≤32`；`identity_evidence/provenance ≤64`；`appid ≤9999999999`；`games` 键 `^[1-9][0-9]{0,9}$`；`set_game_source` 总和 ≤256 KiB。
文件：`MAX_HTML 4MiB`、`overrides 256KiB`、`status 4096B`、`cache 4MiB`、`discovery 512KiB`、`index 512KiB`。
运输（P3）：请求行 64 KiB、结果行 8 MiB、worker stdout 256 KiB / stderr 4 KiB。

## 6. P1 测试矩阵

命名 `sources::<module>::tests::<name>`；用 `FakeClock` + 直接 `SourceStore` 写 seed，不触网。

**来自 `test_sources.py`**：`registry_reads_seeded_cache_and_separates_sources`；`source_cache_round_trips`；`source_cache_ttl_24h_and_force`；`matches_refuses_multiple_distinct_urls`；`failed_refresh_preserves_cache_bytes`；`host_and_allowed_url_reject_hostile`；`validate_entries_rejects_bad_date_and_appid`；`add_source_and_set_enabled_persist_across_reopen`；`game_subscription_scoped_and_global_disable_wins`；`game_subscription_input_boundaries_and_missing_games`。

**来自 `test_discovery_registry.py`**：`builtin_rules_validate_and_keep_roles`；`discovery_commit_layers_success_and_attempt`；`negative_cache_invalidated_by_query_key_change`；`validate_candidates_rejects_claimed_match_without_appid`；`failed_attempt_preserves_success_and_pending_index`；`partial_attempt_does_not_shadow_success_candidate`；`toggle_filters_candidates_and_bumps_revision`；`stale_revision_never_publishes`（P1 提供 `bump_revision`+短锁，P3 驱动）；`validate_candidates_rejects_capability_foreign_appid_and_bad_urls`；`query_context_boundaries`；`shared_index_reused_and_expires_24h`；`partial_status_not_hidden_by_unqueried`；`parser_or_query_version_change_marks_stale`；`custom_rule_shadows_builtin_and_preserves_selection`；`read_paths_create_no_state`；`legacy_match_language_filter`。

**额外强制用例**：`rules_config_accepts_schema1` / `..._schema2_and_rejects_unknown_version`；`graph_missing_games_means_all_subscribed`；`reentry_imports_only_changed_fields` / `reentry_conflict_preserved_not_overwritten`；`atomic_write_recovery_on_crash` / `atomic_replace_existing_file`；`reject_symlink_and_non_regular_state`；`enforce_size_and_entry_caps`；`clock_rollback_expires_conservatively`；`short_lock_serializes_revision_bumps`；`canonical_matches_python_golden`（读 fixture 逐字节比对）。

## 7. 开放决策 / 风险

- **R1**：Python `json.dumps` 格式（`", "`/`": "`、码点排序、`ensure_ascii` 差异；`fingerprint` 落盘重算用 `ensure_ascii=False`）。必须实现 `python_json` 并逐字节比对 golden，不得用 serde_json 默认。
- **R2**：规则/配置 strict（安全边界）；导入遇未知键 **保留原文件 + 诊断**，不崩溃。待产品确认。
- **R3**：`enabled` 必填，保持 `RawSourceRule.enabled` 必填。
- **R4**：限制不一致：持久化用 sources（mirrors 32 / 文本 1024），worker 内部用 discovery（64 / 2048）。
- **R5**：两套 `_normal`：`sources._normal` 先 `+→plus` 再仅留 L/N；`discovery._normal` 保留 `+`。匹配用 sources 版，候选标题相关性用 discovery 版。
- **R6**：`settings_revision` 由内存计数改为持久单调。
- **R7**：`source_for_release` 前缀歧义：保持原算法 + golden 覆盖 `patchmoe` 特例。
- **R8**：缺 `games` 或显式 `{}` 均 == 全订阅，不得解释为全取消。
- **R9**：导入旧 success/index 不得作 fresh/complete；清除/降级 `checked_at` 作为 seed。降级方式待确认。
- **R10**：`preferred_language`/`legacy_match`：`casefold().replace("_","-")`，无 `-` 时前缀匹配；测 `zh-CN`/`zh_cn`/`ja`。
- **R11**：TTL 用墙钟、deadline 用 `Instant`；`Instant` 不作持久时间戳；时钟回退保守过期。
- **R12**：Windows `fs::rename` 覆盖需有限重试，禁 remove-then-write。
- **R13**：候选字段存在性：`published_at/patch_updated_at/upstream_page/version_raw/requires_dlc/excludes_dlc` 恒在（null），`related_index_pages` 仅合并后出现；`fingerprint` 键集由 Python 落盘时刻决定。
- **R14**：`observed_at` 用 `String`（ISO8601）。
- **R15**：`list_sources.status` 优先级：diagnostics > status 文件 > 默认 `cached`/`not_refreshed`。
- **R16**：双引擎快照口径由 `engine-switch.json` + `conflicts/<seq>.json` 冻结。

> 2026-10-11 P6 收口：R16 已被 [跨平台安装器迁移设计](../cross-platform-installer-migration/design.md) §7.3 第 2 行取代——不再有双引擎快照；其余风险项继续有效。原文保留为 P1 冻结契约记录。

## 8. 依赖锁定

**P1：零新增依赖**（仅 `serde` / `serde_json` / `schemars` / `sha2` / `fs2` / `uuid`）。`canonical.rs::python_json` 自行实现，不引入额外 JSON 库。URL 拆分 P1 用 std 手写 `split_url`（严格复刻 `urlsplit`）。

**P2 新增依赖（经独立调研锁定；P1 不安装）**：

```toml
# workspace [workspace.dependencies]
scraper                  = { version = "0.27", default-features = false, features = ["errors"] } # ISC，纯 Rust
unicode-normalization    = "0.1"   # NFKC
caseless                 = "0.2"   # 完整 casefold（等价 Python str.casefold）
unicode-general-category = "1"     # 仅保留 Letter/Number
url                      = "2.5"   # 规范化与 HTTP 解耦
encoding_rs              = "0.8"   # <meta charset> 嗅探（GBK/Big5）
reqwest                  = { version = "0.12", default-features = false, features = ["blocking", "rustls-tls", "charset", "gzip", "deflate"] }
```

- 简繁转换沿用 `apps/decky/backend/vendor/opencc/{STCharacters,TSCharacters}.txt`（Apache-2.0），用 `OnceLock<HashMap<char,char>>` 加载，不新增 crate。
- 排除：`lol_html`（4 MiB 上限内流式过度）、`zhconv`（GPL 表）、`opencc-*`（C 依赖）、`ferrous-opencc`（过重）、`unicode-casefold`（停维护）。
- `scraper` 需 rustc ≥1.85（当前 1.94 OK）；`default-features=false` 去掉 `getopts` CLI。

## 9. 落地顺序

1. `model.rs` + `validate.rs` + `canonical.rs`（含 golden 测试）→ 2. `store.rs`（原子写/短锁/revision/导入）→ 3. `registry.rs`（合并/匹配/分层读取）→ 4. `mod.rs` 门面 + `lib.rs` 注册 + `resources/sources.json` → 5. P1 测试矩阵。P2 从 `identity.rs`/`http.rs`/`adapters/` 桩开始。
