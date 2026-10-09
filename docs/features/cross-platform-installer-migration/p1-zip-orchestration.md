# P1-3b-3 动态 publisher ZIP 编排设计稿

日期：2026-10-08（Asia/Shanghai）。状态：**设计完成、待实现**（只读分析 + 设计产出；不作为已实现结果）。转录自独立 @oracle 设计会话（ora-5）。

关联：[design.md](design.md) §4/§5.1/§5.2/§6、[p0-contracts.md](p0-contracts.md) §2/§3/§4.1/§5、[p1-host-contract.md](p1-host-contract.md) v1.2、[progress.md](progress.md) P1-3b。决策格式：【冻结值 / 理由 / 被否替代 / 迁移动作 / 验证方式】。

## 0. 背景、范围与不重开边界

### 0.1 已识别契约空隙

- p1-host-contract §1.1 只冻结了动态路由 `exe-selection-*`（EXE 内容 ID），**动态 publisher ZIP 的候选/选择表示、resolve 数据流与 `install_patch` 入口未冻结**；`preview`/`scan` 的动态投影亦未冻结。
- p0-contracts §2.1/§2.2 的选择/内容 ID 公式是 EXE 的；publisher ZIP 现行由 `publisher.py` 生成**每实例本地签名 catalog envelope**（`instance.json`/`private.key`/`signed.json`）并 `plan_install`，开发阶段 breaking change 后不再沿用该 envelope（design §2.3/§7.1）。
- p0-contracts §2.4 已冻结：**ZIP 的内容身份沿用 `install::recipe_identity`**（`(appid, release_id, language, artifact_sha256, [(source, destination, installed_sha256)])`），p1-host-contract §2.3 R2 已冻结：`content_id` 仅 EXE 动态路由产生。本稿**不重开**这两条，动态 ZIP 的终态身份据此推导。

### 0.2 本稿冻结范围

1. `zip-selection-*` 候选/选择表示与来源 candidate/source_record 映射、产生时机、终态 `release_id` 与内容身份。
2. `install_patch(selection_id)` 的 resolve 数据流与阻断错误码。
3. 单任务 orchestrator 状态机、与 `install.rs` 事务的调用序列、`requests.sqlite` 幂等、租约/撤销/取消接入点、`install_progress` 阶段映射。
4. `runtime.rs` 协议接线（动态分支判定顺序、scan/preview 投影、错误码清单）。
5. 自动化测试清单。
6. 回写 p0-contracts / p1-host-contract 的最小条目。

### 0.3 不重开/沿用

protocol v3、9 阶段事件、确定性编码（§1.1 `canonical_bytes`）、`installer/rust-v1/` schema、`install::Store` 事务算法与 `recipe_identity`、`authorization` 12 项重核、lease 15s/`PLAN_TTL≤900`、`exe-selection-*` 与签名 `release_id` 路由、`archive` 仅签名路由（UI 不传路径/配方/URL）。EXE worker 与 `analyzing_installer`/转换属 P2/P3，本稿仅冻结其路由占位。

---

## 1. 决策一：候选/选择表示

### 1.1 冻结值

**选择 ID（候选级，不可 apply）**

```
zip-selection-<hex(sha256(canonical_bytes(zip_candidate_binding)))[:24]>-<appid>
```

`zip_candidate_binding` 字段集（有序、显式）：

| 字段 | 来源（`sources::model::SourceCandidate` 字段） |
| --- | --- |
| `source_id` | `candidate.source_id` |
| `source_record_id` | `candidate.source_record_id`（= `canonical_url(source_page)`） |
| `game_appid` | `candidate.game_appid`（必须 `Some(appid)`，且 == ID 后缀 appid） |
| `variant` | `candidate.variant`（`base/dlc/demo/unknown`） |
| `languages` | `candidate.languages`（有序） |
| `version_identity` | `candidate.version_identity` |
| `source_page` | `candidate.source_page` |
| `requires_dlc` / `excludes_dlc` | `candidate.requires_dlc` / `candidate.excludes_dlc`（`null` 或有序列表） |
| `mirrors_zip` | `candidate.mirrors` 中 `format=="zip"` 的条目，按原顺序取 `url`（去 label） |

- 编码使用 p0-contracts §1.1 的确定性 JSON（键 UTF-8 字节序、无空格、`ensure_ascii=false`、禁浮点），**不新增编码版本**（新增消费者不改变字节契约）。
- `appid` 为正整数（`[1-9][0-9]*`）。
- **与 `exe-selection-*` 平行**：字段集与 §2.1 的 EXE `candidate_binding` 同源，仅前缀区分路由；同一 candidate 的 ZIP/EXE 两条 selection 共享摘要、前缀不同，互不冒充。
- **选择 ID 只代表候选，不授予 apply 权限**（design §5.1）。

**产生时机**

- 由**安装器投影**产生：`scan` 的 `installs[].patches[]` 与 `discover_game` 返回的 `patches[]`。`list_game_sources` 仍是来源面板订阅视图，**不产生 selection ID**（保持“candidate ≠ 安装许可”分离，避免 sources 面反向依赖 installer）。
- 投影读取 `Sources::registry().cached_discovery(game)` 的候选（**scan 不发网络请求**）；`status != not_queried` 且候选非空才投影。
- selection ID 在 `preview`/`install_patch` 时**重新推导**，不信任前端回传的候选列表。

**终态 `release_id` 与内容身份**

| 项 | 冻结值 |
| --- | --- |
| 终态 `release_id` | 来源 release id：`patchmoe-<appid>`（rule id `patchmoe`）否则 `source-<rule-id>-<appid>`（与 `registry::matches` 同构） |
| `content_id`（宿主 DTO） | `null`（R2 不重开；仅 EXE 产生） |
| 内容身份 | `install::recipe_identity(&release)`（p0-contracts §2.4），落 `plan.recipe_sha256` |
| 账本 `content_id`（内部） | 上述 `recipe_identity` 摘要（64 hex），作为绑定元组的“已解析内容” |

**投影去重（签名优先）**：同一 appid 同时存在签名 `managed_zip` release 与动态 `publisher_zip` 候选且 `release_id` 相同时，**只投影签名 release**，抑制动态 selection（design §2.1“签名目录保留原信任链”；避免 UI 以 release_id 重复键）。

### 1.2 理由

- 复用 p0 §2.1 的候选级绑定与 §1.1 编码，使 ZIP/EXE 选择表示同源、易证；候选携带 `mirrors/variant/version_identity/requires_dlc`，足以完成 resolve 与 facts 期望映射。
- `mirrors_zip` 只绑 ZIP 镜像：ZIP 路由不应因无关 EXE 镜像或标签变化而失效（相较 §2.1 绑全量 mirrors 的**有意收窄**）。
- 终态身份按 §2.4 + R2 推导为“来源 release_id + `recipe_identity`”，**不引入新 ID 家族**，`content_id` 保持 null。

### 1.3 被否替代

- **另造 `zip-content-<hash>` 作为终态 release_id**：与 §2.4“ZIP 内容身份=recipe_identity”重复，且需新增内容编码契约，收益不足。
- **绑定 `SourceMatch`（来源缓存条目）而非 candidate**：`SourceMatch` 缺 `variant/version_identity/requires_dlc/mirrors`，无法表达内容更新判据，且与 EXE 选择输入不一致。
- **绑全量 mirrors**：无关镜像变化会误失效选择。
- **`list_game_sources` 产生 selection**：把安装投影混入来源面板，破坏权限分离。
- **允许动态 selection 与签名 release 同 release_id 并存投影**：UI/账本键冲突且弱化信任优先级。

### 1.4 迁移动作

- 新增 `crates/core/src/installer/resolve.rs`（或 `orchestrator.rs` 内子模块）：
  - `pub fn zip_selection_id(candidate: &SourceCandidate) -> Option<String>`（`game_appid` 有值且存在 ≥1 zip mirror 才返回）。
  - `pub fn resolve_zip(registry: &SourceRegistry, game: &GameQuery, selection_id: &str) -> Result<ResolvedZipCandidate, ResolveError>`（§2）。
- `runtime.rs` 的 `scan` 在现有 `catalog::matches` 投影后追加动态投影；`discover_game` 结果追加 `patches[]`。
- 扩展 `catalog::SupportLevel`：新增 `PublisherZip`（P1）；`PublisherExe` 预留给 P2/P3。`PatchMatch` 增 `selection_id: Option<String>`（`release_id` 保持既有）。
- `Sources` 暴露只读 `rule_fingerprint(rule)`（现 `session::rule_fingerprint` 私有）。

### 1.5 验证方式

- 稳定性：同输入跨次/跨进程 ID 一致；`mirrors_zip`/`languages`/`variant`/`version_identity`/`requires_dlc`/`excludes_dlc` 任一变化 → ID 变化（fixture 矩阵）。
- 前缀隔离：同 candidate 的 `zip-selection-*` 与 `exe-selection-*` 均合法且不互相路由。
- 无 zip mirror → 不产生 selection（`zip_selection_id` 返回 `None`）。
- 签名优先去重：注入同 `release_id` 的签名 release + 动态候选 → scan 只出现签名 patch。

---

## 2. 决策二：resolve 数据流

### 2.1 冻结值

`install_patch(selection_id)`（动态 ZIP）resolve 按序：

1. **选择解析**：`^zip-selection-[0-9a-f]{24}-[1-9][0-9]*$`；提取 `appid`。不匹配 → `invalid_selection_id`（见 §4 判定顺序）。
2. **实例**：`steam::scan(roots)` 找 `instance_id`；校验 `appid` 等于 selection 后缀、`availability=="installed"`、`buildid` 存在；否则 `instance_not_found`/`game_not_ready`/`build_not_verified`。
3. **来源装载**：`Sources::open(state_dir)`；取 `registry`、`settings_revision`；`game_query(instance)`。
4. **候选定位**：`registry.dynamic_zip_candidates(&game)`（discovery 候选 ∪ 索引来源投影候选；后者为 2026-10-09 修订）的候选逐个计算 `zip_selection_id`，匹配请求值。无命中 → `selection_not_found`（含候选漂移：重算摘要不同即视为未命中）。
5. **来源重核**（design §5.1）：rule 存在、`enabled`、`!discovery_only`、`game_subscribed(appid, source_id)`；`candidate.cache_status != Stale`。否则 `source_not_enabled`/`source_not_subscribed`/`source_requires_guidance`/`selection_stale`。
6. **下载 URL 解析**（`acquire.validate_download` 前置）：
   - `mirrors_zip` 按原顺序过滤（`allowed_url` 校验；非法镜像跳过）后规范化去重：该格式镜像为 0 → `zip_download_unavailable`；过滤后合法集合为空 → `invalid_download_url`；合法集合 basename 异构（≥2 种）→ `zip_download_ambiguous`；否则取首个合法 URL（**确定性择一**；2026-10-09 P4 实机验收修订，见 §2.3）。
   - `acquire::validate_download(url, rule.download_hosts)`：HTTPS/主机/端口 443/无凭据/`.zip` 路径；失败 → `invalid_download_url`。
7. **released 门（F1 裁决：动态 ZIP 路径移除 `source_not_released`）**：候选模型暂无可靠日期字段（`SourceCandidate.date` 缺失、`published_at` 恒 `None`），动态路径不承诺 `source_not_released`；**待候选日期字段补齐后另行引入**。未发布条目的过滤仍由 discovery/来源缓存负责，无日期证据视为已发布。
8. **facts 期望映射**（`FactsExpectation`）：

| 期望 | 取值 |
| --- | --- |
| `appid` | selection appid |
| `language` | 候选选中语言（`languages` 含 `zh-CN` 取之，否则首个）；为空时回退 rule.`language`，再回退现场选中语言（自洽，不因来源无语言而阻断）。**是否作为准入条件 = `rule.discovery_only`**：发布商来源（kagura）设门；索引投影来源（patchmoe，2026-10-09 修订）不设门；语言仍进 `recipe_identity` |
| `require_language` | 语言准入开关（2026-10-09 修订新增）：发布商 ZIP/EXE 为 `true`（默认），索引来源 ZIP 为 `false`；`false` 时语言不匹配仅记录 evidence、不阻断 |
| `requires_dlc` / `excludes_dlc` | `candidate.requires_dlc` / `excludes_dlc`（缺省空） |
| `verified_buildids` | `[instance.buildid]`（“本次现场 build 自证”，对齐 Python publisher 生成 `verified_buildids=[buildid]`；publisher ZIP 无人工 build 表） |
| `baselines` | 空（ZIP 无审核基线表；`original_sha256` 是现场快照，非原版认证，design §5.2） |

9. **阻断判定**：投影 blocker 集合须为空才继续（对齐 Python `if patch["blockers"]: raise`）：`source_not_enabled`/`source_not_subscribed`/`game_not_ready`/`build_not_verified`/`zip_download_unavailable`/`zip_download_ambiguous`（仅合法镜像 basename 异构；同名多域镜像确定性择一，2026-10-09 修订）/`managed_install_exists_or_needs_recovery`/`publisher_existing_transaction`（同实例已 committed 且 release_id 不同）/`archive_not_allowed_for_selection`。
10. **期望 → 下载请求**：`DownloadRequest { url, download_hosts: &rule.download_hosts, expected_sha256: None, expected_size: None }`（动态首 SHA 建立内容身份；来源若提供可靠摘要则另行核验，p0 §2.4，P1 候选暂不携带）。

**不可用/歧义阻断错误码**：`selection_not_found`、`selection_stale`、`source_not_enabled`、`source_not_subscribed`、`source_requires_guidance`、`zip_download_unavailable`、`zip_download_ambiguous`、`invalid_download_url`、`game_not_ready`、`build_not_verified`、`instance_not_found`。（`source_not_released` 已从动态 ZIP 路径移除；待候选日期字段补齐后另行引入，F1。）

### 2.2 理由

- 只在安装路径里做**网络无关的候选重核 + 一次动态下载**，不在 `install_patch` 隐式触发 `discover_game`（避免长网络/来源刷新混入任务）；陈旧选择明确要求 UI 刷新来源。
- 候选即来源真源，与 EXE 选择输入一致；DLC/语言来自候选条件，`requires_dlc/excludes_dlc` 未知不默认满足（`FactsExpectation` 空列表 = 无实际条件）。
- 语言准入按来源类别收窄（2026-10-09 裁决）：发布商来源（`discovery_only`）保留语言门（ZIP 无审核基线，语言为唯一变体前提，且与 EXE 路径一致）；索引投影来源（patchmoe）语言为页面归类、同 URL 多语言已在采集期合并且优先 zh-CN，不作准入门；语言仍进 `recipe_identity`（身份不随现场语言漂移）。对齐旧 `publisher.py`（无游戏语言门）。
- `buildid` 自证对齐 Python publisher，不引入 publisher ZIP 不存在的人工 build 门。

### 2.3 被否替代

- resolve 内联 `discover_game`/`refresh_sources`（网络进入安装任务，取消/租约复杂化）。
- 用 `SourceMatch`（来源缓存）做权威输入（信息不足）。
- 把 `verified_buildids` 设为可信 build 白名单（publisher ZIP 无该数据，且会误阻断）。
- ~~允许 `>1` 镜像自动择一（不确定来源，必须阻断）。~~ **2026-10-09 P4 实机验收修订撤销**：真实 kagura 来源对同一 artifact 提供多域同名镜像（均在 `download_hosts` 内），按原语义会误判歧义导致动态安装不可用；冻结改为「同候选同名多域镜像按 mirrors 原顺序确定性择一，内容身份由下载字节建立；仅合法镜像 basename 异构时阻断」（见 §2.1(6)）。其余被否项不变。
- 索引来源语言门维持（旧 `publisher.py` 无游戏语言门，属功能回退，2026-10-09 否决）；取消全部 ZIP 语言门（丢弃发布商来源真实防误装价值且与 EXE 不一致，过度）；「同 URL 多语言→现场语言」（采集期已合并为单候选，resolve 期证据不可得，成本高）。

### 2.4 迁移动作

- `resolve.rs` 实现上述函数；`ResolveError` 携带稳定 `code`。
- `runtime.rs` 删除对 `sources::session::resolve_source_record` 的潜在依赖路径；动态 ZIP 不再走 `source_for_release`（P6 删除项）。
- `Sources` 暴露 `cached_discovery` 只读句柄（已存在）。

### 2.5 验证方式

- 逐错误码 fixture：禁用/退订/无 zip 镜像/同名多镜像择一/异构多镜像阻断/主机越界/未来日期/未安装/无 buildid/候选漂移/缓存未查询。
- 语言/DLC 期望矩阵：`requires_dlc` 缺失 → `exe_dlc_required`；`excludes_dlc` 命中 → `exe_dlc_excluded`；**发布商来源**语言不符 → `exe_language_mismatch`，**索引来源**语言不符不阻断（均来自 `game_facts`，见 §3；2026-10-09 修订）。
- URL 校验：`http`/跨主机/带凭据/非 443/非 `.zip` → `invalid_download_url`。

---

## 3. 决策三：orchestrator 接口与单任务状态机

### 3.1 冻结值 —— 阶段/子阶段映射

ZIP 路由实际发出的 host 阶段（`operation::stage` 来源）：

| 阶段 | 发出点 | 说明 |
| --- | --- | --- |
| `connecting` | orchestrator（解析后、首跳前） | 新增显式 emit（当前 acquire 不发） |
| `downloading` | `acquire.rs`（GET 200 后） | 字节随阶段可重置 |
| `validating` | `acquire.rs`（摘要/大小）+ `zip_recipe::build_recipe`（结构/展开）+ `install::prepare`（成员/落 stage） | 多段复用同一阶段名 |
| `analyzing_installer` / `extracting` | 不发出 | EXE 专用（P2/P3 worker 转换）；ZIP 无转换产物 |
| `planning` | `install::prepare`（save 前） | |
| `applying`（`substage` = `backing_up`/`writing`/`committing`） | `install::apply` | CLI `ProgressCursor` 已折叠 |
| `rolling_back` | `install::restore` | 期间忽略取消 |
| `cleanup` | orchestrator（终态清理/缓存） | `install::cleanup_terminal` 现不发阶段，登记为 P1 补齐点 |

`install_progress` 信封字段不变（p1-host-contract §2.2 平铺）；`selection_id` 对动态 ZIP 必填，签名路由可空。

### 3.2 冻结值 —— orchestrator API

新增 `crates/core/src/installer/orchestrator.rs`：

```rust
pub struct ZipInstallInputs<'a> {
    pub game: &'a steam::GameInstall,
    pub selection: &'a resolve::ResolvedZipCandidate, // owned 或借用
    pub operation: install::InstallOperation,
    pub request_id: &'a str,
    pub now: u64,
    pub ttl_seconds: u64, // <= PLAN_TTL_SECONDS
}

pub struct ZipInstallDeps<'a> {
    pub paths: &'a InstallerPaths,
    pub state_dir: &'a Path,          // 用于 Sources::open
    pub transport: &'a dyn acquire::Transport,
    pub cancel: &'a AtomicBool,
    pub lease_active: &'a dyn Fn(u64) -> bool, // now -> lease_active
}

/// 单任务：resolve(外部) → acquire → recipe → facts → authorize → plan → apply
pub fn install_zip(
    inputs: ZipInstallInputs<'_>,
    deps: ZipInstallDeps<'_>,
) -> Result<install::Transaction, ZipTaskError>;
```

- `install_zip` 在 `operation::run` 作用域内运行（`main.rs::run_task` 已包裹），使用 `operation::*` 发阶段；不自行 `operation::run`。
- `transport` 注入使编排可离线测试（复用 `acquire` 的传输接缝）。
- `resolve` 在 `runtime.rs` 调用（需要 `Sources`），把 `ResolvedZipCandidate` 传入，保持 `installer` 不依赖 CLI。

### 3.3 冻结值 —— 调用序列（与 install.rs 事务）

1. **幂等预检**（§3.4）：`RequestLedger::open(paths)`；`get(request_id)`。
2. **resolve**（无锁）：§2。
3. **acquire**（无锁，发 `connecting`）：`acquire_with(deps.transport, &DownloadRequest{...}, deps.paths, cancel, progress)` → `AcquiredArchive`。
4. **recipe**（无锁）：`zip_recipe::build_recipe(&acquired.path, &game.install_path, cancel)` → `Vec<FileMapping>`。
5. **facts**（无锁）：`facts::game_facts(game, &expectation, cancel)`；`!facts.is_valid()` → 返回首个 `FactFailure.code`（`exe_dlc_*`/`exe_language_mismatch`/`game_not_ready` 等），不写事务。
6. **构造动态 release**（内存，不落签名 catalog）：`PatchRelease{ patch_id=release_id, release_id=来源 release id, appid, name, version="0", language, category="publisher_zip", source_page, support_level=ManagedZip, platforms=[Windows,Steamos], verified_buildids=[buildid], detection=mappings→Fingerprint, recipe=Some(Recipe{artifact_sha256=acquired.sha256, files}), artifact=Some(Artifact{url=acquired.final_url, size, sha256, allowed_redirect_hosts=rule.download_hosts}) }`。
7. **plan + authorize + validate**（持 `journal.lock`，经 `Store::open_at`；本步内不跨锁调用 apply）：
   - `install::prepare_with_operation_at(paths.root(), game, &release, &acquired.path, 0 /*catalog_revision*/, operation)` → `InstallPlan`（内部完成 ZIP 复校、落 `.stage`、写 `planned` journal）；**`PlannedChange` 新增 `source: Option<String>`**（§3.6）。
   - `content = ContentBinding::publisher_zip(&acquired.sha256, acquired.size, &mappings)`；`identity = install::recipe_identity(&release)`。
   - `authorize(...)`（绑定按 §3.3，含 `candidate_sha256: candidate.fingerprint`、`rule_fingerprint`、`settings_revision`、`conditions_sha256`、`content`、`plan_id`、`request_id`、`facts`、TTL）；`authorization::write(paths, &auth)`。
   - **重核**：重开 `Sources`（只读比对 settings revision）、重读候选/facts、从 `plan.changes` 重建 `ContentBinding` → `authorization::validate(&auth, &CurrentState{...})`；任一失败 → `plan_failed` + 对应 `authorization_*` 码，`discard_plan_at` 撤销计划，不 apply。
8. **apply**（重新 `Store::open_at`）：`install::apply_at(paths.root(), &plan.plan_id)`（内部取 game lock、`check_revoked`、TTL、`ensure_no_active`、逐文件写入/恢复）。**不跨锁复用 Store**（apply 会自行 `try_lock_exclusive`，同进程二次加锁会失败）。
9. **终态**：构造 `InstallOutcome`（§3.7），回填账本，释放锁，返回单行终态。

**install.rs 最小 API 增补**：新增 `prepare_with_operation_at(root, ...)`、`apply_at(root, plan_id)`、`recover_at(root)`、`list_transactions_at(root)`、`discard_plan_at(root, plan_id)`、`get_plan_at(root, plan_id)`，内部委托 `Store::open_at`；`list_transactions_at`/`recover_at` 的读取路径按 p0 §4.1 走只读快照（`open_ro`，不取 `journal.lock` 独占）。

### 3.4 冻结值 —— requests.sqlite 幂等

绑定元组：`(instance_id, selection_id|release_id, operation, resolved content_id)`（动态 ZIP：`selection_id` 必填、`release_id` null、`content_id` = `recipe_identity` 摘要）。

算法（对齐 p1-host-contract §5.1 N6）：

1. `record` 不存在 → 插入绑定（`content_id` null），进入编排。
2. 存在且 `(instance_id, operation)` 或选择身份（`selection_id`/`release_id`）不符 → `request_id_binding_conflict`。
3. 存在且已有 `content_id`：与本次解析内容比对；不符 → `request_id_binding_conflict`。
4. 存在但尚未解析内容：若记录有 `plan_id` 且 `list_transactions_at` 返回该 plan 的终态 → 返回原终态（幂等重放）；无 `plan_id` 但同实例存在**非终态**事务（`planned/backing_up/applying/rolling_back/needs_recovery`）→ `install_request_in_progress_or_interrupted`；否则视为可继续（重新 resolve/acquire，安全，因计划前无写入）。
5. 计划建立后 `record` 回填 `plan_id`、`content_id`；终态后回填 `transaction_id/state/error`。
6. **计划建立前的失败不写 `error`**（保留绑定行，允许同 request_id 重试）；**同内容重装必须由宿主换新 request_id**（MF1），账本不做绑定派生。
7. 崩溃/中断：不二次写入；由 `recover`/`discard_plan` 收尾，或按 (4) 诊断。

### 3.5 冻结值 —— 租约/撤销/取消接入点

- **租约**：15s（p1-host-contract §5.2）。`renew` 更新共享 deadline；计划建立时写 `jobs/<plan_id>/lease.json`（`{deadline_ms, renewed_at}`，`write_atomic`），此后每个阶段边界若收到 renew 则刷新落盘。`lease_active(now) = now_ms <= deadline`，作为 `CurrentState.lease_active` 传入 `validate`。
- **撤销**：需要停任务时（用户 `cancel`、租约过期、EOF）：
  - 计划前：置 `cancel`，返回 `cancelled`（`ok:false`，无事务）。
  - 计划后：先持久撤销（`jobs/<plan_id>/revoked` + `sync_all`），再停获取；apply 未开始时用 `discard_plan_at`（revoke + `rolled_back` + 清 stage）；apply 中由 `apply` 的 `check_revoked`/`operation::check` 触发 `restore`。**提交成功优先于迟到取消**；恢复冲突 → `needs_recovery`。
- **EOF/父退出**：`main.rs::spawn_control_thread` 置 `cancel`；撤销逻辑同上（计划前仅停止；计划后落 `revoked`）。worker 进程组回收属 P3。
- **阶段边界检查**：每个阶段前后执行 `operation::check()` + 本地 `cancel` + `lease_active` 检查；下载/配方内部已按 chunk 检查取消。

### 3.6 冻结值 —— PlannedChange / ContentBinding 重核

- `install::PlannedChange` 新增 `#[serde(default)] pub source: Option<String>`；`prepare_with_operation` 落 `source=Some(mapping.source)`（旧 journal/None 容忍）。使 apply 前可从 `plan.changes` 重建 `ContentBinding::publisher_zip`（`source/path/installed_sha256`）并与授权比对。
- apply 前重读下载文件摘要，须等于 `authorization.content.artifact_sha256`（否则 `authorization_content_changed`/`download_hash_mismatch`）。

### 3.7 冻结值 —— InstallOutcome（ZIP）

`state` ∈ `committed|superseded|rolled_back|needs_recovery`；`release_id` = 来源 release id；`selection_id` = `zip-selection-*`；`content_id` = `null`（R2）；`plan_id`/`transaction_id` 来自事务；`recovery.required` 在 `needs_recovery` 时为 true；`cleanup` 反映 `cleanup_terminal`。计划前失败与下载期取消返回 `ok:false + error{code}`（§2.3），不产生 `InstallOutcome`。

### 3.8 理由

- 网络/解析不持 `journal.lock`（p0 §4.1）；plan/apply 各自短持锁，避免同进程二次 `try_lock` 失败，同时由 `journal.lock`（prepare）+ game lock（apply）保证单写者。
- 授权在 plan 后、apply 前生成并立即重核，覆盖 settings revision/候选/规则/条件/内容/facts 漂移（p0 §4.1 两次比对）。
- 持久撤销先于停任务、提交优先、恢复冲突显式上报，全部沿用已冻结语义。

### 3.9 被否替代

- 计划前后跨锁持有单个 `Store`（apply 会二次加锁失败）。
- 计划前失败写 `error` 并短路重试（用户可安全重试的动作被永久钉死）。
- 用内存标志代替 `revoked` 文件（崩溃/新进程失效）。
- 让 orchestrator 生成本地签名 catalog envelope（用户已接受 breaking change，design §2.3）。
- apply 前复用内存 `release` 而不重读（无法证明下载/现场未变）。

### 3.10 迁移动作

- 新增 `orchestrator.rs`、`resolve.rs`；`installer/mod.rs` 注册与 re-export。
- `install.rs` 增 `_at` 系列 + `PlannedChange.source`；`prepare_with_operation` 落 source。
- `main.rs::run_task` 把 lease 句柄传入 install 任务（新增 `Arc<LeaseState>`），并新增租约过期看门狗置 `cancel`。

### 3.11 验证方式

- 阶段序列：合成任务断言 `connecting→downloading→validating→planning→applying(+substage)→cleanup`，`sequence` 严格递增、阶段重置字节。
- 调用序列：prepare 后 journal 有 `planned`；authorize 写 `jobs/<plan_id>/authorization.json`；apply 终态一致。
- 幂等：§5 用例。
- 取消/租约：§5 用例。

---

## 4. 决策四：协议接线

### 4.1 冻结值 —— `runtime::install_patch` 动态分支判定顺序

1. **参数校验**：`instance_id` 必填（`instance_required`）；`selection_id` 与 `release_id` **互斥**（同给 → `invalid_request`）；二者皆缺 → `selection_or_release_required`；`operation` ∈ `install|reinstall`（否则 `invalid_operation`，R3）。
2. **签名路由（优先）**：`release_id` 存在 → 现有签名 catalog 路径（`signed()` + `ready_release()`），迁移到 `installer/rust-v1` 根 + 新账本 + 授权（**收窄为“根/账本迁移 + facts/撤销/租约重核”**；签名路由授权绑定实施值见 §9 第 4 条），`archive` 仍仅此路由有效。
3. **动态路由**：`selection_id` 存在 →
   - 前缀 `zip-selection-` → 动态 ZIP orchestrator（§2/§3）。
   - 前缀 `exe-selection-` → P2/P3；P1 返回 `exe_route_unimplemented`（`links.exe.available=false` 已表达不可用）。
   - 其他前缀/格式 → `invalid_selection_id`。
4. **候选不冒充 release**（design §5.1）：任何 `candidate-*`、`zip-selection-*` 等选择身份不得作为 `release_id` 路由。
5. **archive 限制**：动态 selection + `archive` → `archive_not_allowed_for_selection`（核心侧校验普通文件/`.zip`/`no_links`/≤8 GiB 仅签名路由适用）。

### 4.2 冻结值 —— scan / preview 投影

- `scan.installs[].patches[]` 每条：`release_id`、`selection_id?`、`support_level`、`download_available`、`blockers[]`、`detection`、`compatibility`。
  - 动态 ZIP：`support_level="publisher_zip"`、`selection_id=zip-selection-*`、`release_id=来源 release id`、`download_available = (候选有 zip 镜像 && 来源启用/订阅 && released && 未安装阻断)`、`blockers` 来自 §2.1(9)、`detection` 由**最近 committed 事务**（同实例同 release_id）的 plan 目的地重哈希得出，否则 `unknown`。
  - EXE 候选：`support_level="publisher_exe"`，P1 追加 blocker `exe_sandbox_unavailable`（能力未实现），不产生可安装。
  - 签名 managed_zip 沿用现有投影；同 `release_id` 时不重复投影动态 selection（§1.1）。
- `preview{instance_id, selection_id?|release_id?}`：
  - `zip-selection-*`：只读、**不下载**；返回 `{instance_id, selection_id, release_id: 来源 release id, executable:false, blockers, changes:[]}`；若有最近 committed 事务，`changes` 由其 plan 列出（`path`/`action`），否则空。
  - `release_id`：现有 catalog preview（`executable:false`）。
  - `exe-selection-*`：P2/P3；P1 返回 `executable:false` + blocker `exe_route_unimplemented`。

### 4.3 冻结值 —— 迁移到 installer 根

- `list_transactions`/`recover`/`discard_plan`/`cleanup_cache`/`install_patch` 统一改用 `InstallerPaths::new(state_dir)`：`init_atomic`（幂等）后 `*_at(paths.root())`；`journal.sqlite` 存在性改为 `paths.journal()`。
- `cleanup_cache` 改用 `paths.list_cache_entries()`（仅 `content/`、`downloads/`、`reports/`，不删 active lease/receipt/journal/`.stage`/`.backup`/`revoked`）。

### 4.4 错误码清单（动态 ZIP）

| 阶段 | `error.code`（宿主） | 备注 |
| --- | --- | --- |
| 请求 | `invalid_request`、`selection_or_release_required`、`invalid_selection_id`、`invalid_operation`、`archive_not_allowed_for_selection` | |
| resolve | `instance_not_found`、`game_not_ready`、`build_not_verified`、`selection_not_found`、`selection_stale`、`source_not_enabled`、`source_not_subscribed`、`source_requires_guidance`、`zip_download_unavailable`、`zip_download_ambiguous`、`invalid_download_url` | 直通稳定码；`zip_download_ambiguous` 收窄为合法镜像 basename 异构（2026-10-09 修订）；`source_not_released` 已移除，见下表注 |
| 下载 | `download_cancelled`、`download_hash_mismatch`、`download_size_limit` 直通；其余传输/HTTP/非 ZIP/空包归 `download_failed`（`detail` 带内部码） | |
| 配方/分析 | `archive_entry_limit`、`archive_too_large` 直通；`zip_*`/`unsafe_relative_path`/`empty_recipe` 归 `analysis_failed`（`detail` 内部码） | |
| 事实 | `exe_dlc_required`、`exe_dlc_excluded`、`exe_language_mismatch`（仅发布商来源；2026-10-09 修订）、`exe_manifest_*`、`game_not_ready` | 来自 `game_facts` |
| 计划 | `plan_failed`、`plan_has_blockers`、`insufficient_space`、`installer_busy`、`game_busy`、`managed_install_exists_or_needs_recovery`、`reinstall_content_identity_changed`、`reinstall_unowned_or_incomplete`、`plan_expired` | |
| 授权 | `authorization_*`（12 项）、`authorization_expired`、`authorization_revoked`、`authorization_lease_inactive` | |
| 路由占位 | `exe_route_unimplemented`（P1 临时，P3 移除） | 新增，需回写 |

> 注（F1）：`source_not_released` 已从动态 ZIP 路径错误码表移除——动态 ZIP 暂无此门（候选模型无可靠日期字段）；**待候选日期字段补齐后另行引入**。

### 4.5 被否替代

- 让前端携带候选/配方/URL（扩大攻击面，design §2.1）。
- 动态 `exe-selection-*` 在 P1 静默成功或走签名路径（冒充 release）。
- `preview` 动态 ZIP 下载（违反只读、不下载）。
- 保留旧顶层 `install_requests.sqlite` 读写（p0 §3.1 明确替代）。

### 4.6 验证方式

- 路由矩阵：`release_id`/`zip-selection-*`/`exe-selection-*`/未知前缀/互斥/皆缺 → 预期码。
- 动态 selection + `archive` → `archive_not_allowed_for_selection`。
- `get_capabilities.methods` 仍为 16，方法集合不变。
- scan/preview 形状与 §1/§4.2 一致；`list_transactions`/`recover` 使用新根。

---

## 5. 决策五：测试要求（自动化，合成 fixture）

### 5.1 冻结值 —— 用例清单

1. **选择身份**：`zip_selection_id` 稳定性与字段敏感矩阵（`mirrors_zip`/languages/variant/version_identity/dlc）；同一 candidate 的 ZIP/EXE 前缀隔离；无 zip 镜像返回 None；索引来源（patchmoe）投影候选的 `candidate_id`/`fingerprint`/`zip_selection_id` 确定性（2026-10-09 修订）。
2. **resolve**：§2.5 全部错误码；禁用/退订/未查询缓存/未来日期；同名多镜像确定性择一与异构镜像阻断；主机越界；候选漂移。
3. **获取/配方**（复用现有 `acquire`/`zip_recipe` fixture 合成）：正常下载→内容寻址缓存→304 复用；不同字节独立身份；XP3 包装/穿越/压缩比/条目上限拒绝。
4. **facts 期望**：语言/DLC/build 映射；`requires_dlc` 缺失、`excludes_dlc` 命中、发布商来源语言不符、manifest 缺失 → 对应码且不写事务；索引来源语言不符不阻断（2026-10-09 修订）。
5. **编排状态机**：合成游戏根 + 合成 ZIP（`tempdir`），注入 `FakeTransport`，断言阶段序列、`sequence` 严格递增、终态一次、`InstallOutcome` 字段（release_id/selection_id/content_id=null）。
6. **幂等**：同 `request_id` 同绑定重放返回同 `transaction_id`；异 `operation`/选择 → `request_id_binding_conflict`；`content_id` 回填；崩溃窗口（绑定无 plan + 孤儿 `planned` 事务）→ `install_request_in_progress_or_interrupted`；同内容重装**换新 request_id** → 新事务。
7. **取消/租约**：逐阶段取消（connecting/downloading/validating/planning/applying）；计划前取消无事务（`cancelled`）；计划后取消 `rolled_back`；apply 中取消恢复；迟到取消不覆盖 `committed`；不续租 15s → 撤销（写 `revoked`）；`renew` 保持授权；EOF 等价 cancel。
8. **恢复/重装**：注入中断 job → `recover_at` 得 `rolled_back`/`needs_recovery`；`needs_recovery` 显式上报；同内容重装 `committed`；新字节/语言/DLC → `reinstall_content_identity_changed`；无主 → `reinstall_unowned_or_incomplete`。
9. **授权重核**：plan 后篡改 settings revision / 候选指纹 / 规则指纹 / 条件 / 下载文件 / facts → 对应 `authorization_*` 且不 apply。
10. **协议**：§4.6；`preview` 动态 ZIP 不触网、`changes` 空或来自 committed plan；scan 投影字段齐全 + 签名优先去重。
11. **边界**：`archive` 动态拒绝；`state_overlaps_game`；`linked_state_path`；请求 >64 KiB；`PlannedChange.source` 旧 journal（None）容忍。

### 5.2 验证方式

- `cargo test -p steam-patch-core -p steam-patch-cli`、`clippy`、`fmt`；fixture 文本/JSON 入库，二进制合成不提交（p0 §9）。
- 不比较设备路径/时钟；不执行真实安装器。

---

## 6. 决策六：契约回写清单（最小条目）

### 6.1 p0-contracts.md

| 位置 | 追加内容 |
| --- | --- |
| 新增 §2.5（置于 §2.4 后） | 动态 publisher ZIP 选择 ID：`zip-selection-<hex24>-<appid>`、`zip_candidate_binding` 字段集（含 `mirrors_zip` 收窄说明）、编码用 §1.1、不可 apply。标题格式【冻结值/理由/被否替代/迁移动作/验证方式】。 |
| §2.2 | 一句交叉引用：动态 ZIP 终态 `release_id` = 来源 release id（`patchmoe-<appid>`/`source-<id>-<appid>`），`content_id` 不产生（R2）；内容身份见 §2.4 `recipe_identity`。 |
| §3.1 | 补注：`requests.sqlite.content_id` 对动态 ZIP 存 `recipe_identity` 摘要（内部解析内容）。 |
| §9（F6） | 增 ZIP 选择/内容 ID 变化矩阵 fixture（字段变化 → ID 变化）。 |
| §10 | 增行：动态 ZIP release_id/内容身份已冻结（指向本稿）。 |

### 6.2 p1-host-contract.md

| 位置 | 追加内容 |
| --- | --- |
| §1.1 A（动态路由说明） | 扩展为：签名 `release_id` / `zip-selection-*` / `exe-selection-*`；判定顺序（互斥→签名→前缀分派）；`archive` 仍仅签名路由。 |
| §1.1（错误码） | `invalid_selection_id`、`selection_not_found`、`selection_stale`、`zip_download_unavailable`、`zip_download_ambiguous`、`exe_route_unimplemented`（P1 临时）；`source_not_released` 动态 ZIP 无此门（待候选日期字段补齐后另行引入，F1）。 |
| §2.2 | `install_progress.selection_id` 对动态 ZIP 必填说明。 |
| §2.3 | `InstallOutcome`：动态 ZIP `release_id`=来源 release id、`content_id`=null。 |
| §5.1 | 账本对动态 ZIP 的绑定：`selection_id` + 内部 `content_id`=`recipe_identity`；计划前失败不写 `error` 的说明。 |
| §7.1 | 增测试条目：路由矩阵、动态 ZIP 幂等/取消/恢复/重装、`stage/substage` 映射。 |
| §8 待决策表 | 增行：动态 ZIP 编排（本稿已冻结，指向 `p1-zip-orchestration.md`）。 |

### 6.3 格式与位置

- 沿用各文件既有【冻结值/理由/被否替代/迁移动作/验证方式】体例；不新建平铺文档（AGENTS.md 规则），本稿落入 `docs/features/cross-platform-installer-migration/`，并在 `progress.md` 记录。

---

## 7. 风险、待决与与现有模块的差异

1. **release_id 命名空间复用**：动态 ZIP 与签名 managed_zip 共享 `patchmoe-<appid>`/`source-<id>-<appid>`；以“签名优先去重”消除投影与 UI 键冲突；账本以 `selection_id` 区分路由。需在实现与测试中固化。
2. **候选日期/`_released`**：`SourceCandidate` 现无 `date`，`published_at` 恒 `None`；F1 裁决后本稿动态路径**不设 `source_not_released` 门**（已从契约移除），`candidate` 日期字段补齐后另行引入（不改变现有 resolved 语义）。
3. **`conditions_sha256`**：条件求值器未落地；本稿用候选条件 `{requires_dlc, excludes_dlc, language, verified_buildids}` 的确定性摘要占位（progress 已登记“由调用方提供”）。
4. **锁与并发**：多任务并发由 `serve-task` 单任务 + `journal.lock`（prepare）+ game lock（apply）+ `ensure_no_active` 约束；`planned` 孤儿计划由 `recover`/`discard_plan`/`cleanup` 处理（登记 P1 收尾）。
5. **`cleanup` 阶段**：`install::cleanup_terminal` 现不发阶段，P1 补齐（编排在 apply 后显式 emit）。
6. **签名路由同迁新根**：`install_patch`/`list_transactions`/`recover`/`discard_plan`/`cleanup_cache` 一并迁 `installer/rust-v1`，避免双根。
7. **EXE 占位**：`exe-selection-*` 在 P1 明确 `exe_route_unimplemented`，P3 移除该码并接 worker。

---

## 8. 验收标准 / 放行判断

- 设计自洽：不重开 P0/P1 已冻结决策；选择/内容身份与 §2.4/R2 一致；与 `acquire/zip_recipe/facts/authorization/store/install` 现有 API 兼容（仅新增 `_at` 与 `PlannedChange.source`）。
- UI/前端不传路径/配方/任意 URL；`archive` 仅签名路由。
- §5 自动化用例全绿为 P1-3b-3 实现门槛；实机（Windows/Decky）协议与真实安装验收随 P4/P5，保持待验证。
- 回写条目按 §6 落到 p0-contracts/p1-host-contract，并同步 `progress.md`/功能索引。

---

## 9. 实施注记（2026-10-08，fix-12；P1 gate 初审裁决已折入 fix-14/fix-15）

1. §2.1(5) `!discovery_only` 实施校正为仅 `enabled`（候选来自 discovery 规则；“publisher 规则携带 download_hosts”）。
2. §2.1(5) 的 `source_not_enabled`/`source_not_subscribed` 在 `resolve_zip` 路径不可达（`effective_discovery_rules` 已过滤候选），实际表现为 `selection_not_found`（含候选漂移）；两码保留于 scan 投影 blockers。**`selection_stale` 实际可达**：来源缓存条目 `cache_status == Stale` 时 `resolve_zip` 返回 `selection_stale`（测试 `stale_discovery_yields_selection_stale`），§2.1(5)/§2.1(6)/§4.4 的该码为生产路径真实返回（原注记第 2 条关于 stale 的“不可达”表述作废）。
3. §1.4 与 §2.1(8) 的 buildid 交接经 `ResolvedZipCandidate::expectation(buildid)`/`conditions_sha256(buildid)`（orchestrator 计算）。
4. **签名路由授权绑定冻结为实施确定值（r6 §9.4 裁决）**：`source_id="signed"`、`settings_revision=catalog.revision`、release 派生 `candidate_sha256`/`rule_fingerprint`、现场 manifest 语言；§4.1 step 2 的“同 §3”已收窄为“根/账本迁移 + facts/撤销/租约重核”。
5. `error.detail` 暂经内部 `\u{1}` 分隔传递（`main.rs::respond` 拆分）；建议后续 typed error。
6. `recover_at`/`list_transactions_at` 将空 journal（无 `jobs` 表）视为空；`cleanup_cache` 保持 `{performed, removed_bytes}` 形状。
7. **F1 裁决（r6）：`source_not_released` 从动态 ZIP 路径契约移除。** 候选模型暂无可靠日期字段（`SourceCandidate.date` 缺失、`published_at` 恒 `None`），released 门在动态路径无稳定来源可依；§2.1 step 7 与 §4.4 错误码表已同步标注“待候选日期字段补齐后另行引入”。未发布条目的过滤仍由 discovery/来源缓存负责，不变更 resolve 语义。
8. **F3 裁决（fix-14）：租约落盘已补做。** 计划建立时写 `jobs/<plan_id>/lease.json`（`{deadline_ms, renewed_at}`，`write_atomic`），阶段边界续租刷新；不再仅内存 `LeaseState`（§3.5、p1-host-contract §5.5/§8 同步）。
9. **F4 裁决（fix-14）：动态路由租约改为实时检查。** `install_zip_route` 不再用单次固定 `now`（原 `lease_active(fixed_now)` 恒真），改取实时时间，与签名路由对称；真实过期仍叠加看门狗（§3.5）。
10. **F6 裁决（fix-14）：计划后取消/租约过期/撤销的形态统一对齐 §2.3。** 该三类经恢复返回 `InstallOutcome`（恢复成功 `rolled_back`、恢复冲突 `needs_recovery`、提交已成功 `committed`）；**其余计划后漂移失败**（`authorization_candidate_changed`/`authorization_content_changed`/`authorization_facts_changed`/`authorization_expired` 等）保持 `ok:false + error{code}`（§4.4），并已留下 `rolled_back` 终态事务——同 `request_id` 重放返回该终态（契约允许）（§3.7、p1-host-contract §2.3）。
