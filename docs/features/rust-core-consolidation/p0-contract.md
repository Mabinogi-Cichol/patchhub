# P0 领域接口与 v4 wire 契约冻结

日期：2026-10-11。状态：**契约决策冻结；目标类型、生成器、v4 与新增锁均待实现**，以下签名不是现有 API 或已运行的代码。

关联：[设计](design.md)、[施工记录](progress.md)、[动作与终态真值表](p0-behavior.md)。本文冻结字段、接口、生成与拒绝边界；真值表冻结证据组合。后续必要变更须同步两份 P0 文档及设计。

## 1. 已核对当前源码

行号是本轮工作树的定位证据；重构后会变化，链接指向对应文件。

| 已证实事实 | 源码位置 | 提取决策 |
| --- | --- | --- |
| 手写 Params、16 方法与三任务方法 | [runtime.rs](../../../crates/cli/src/runtime.rs) L17–103；[main.rs](../../../crates/cli/src/main.rs) L19–44 | protocol 逐方法 Params；正式方法仍 16 |
| scan 组合目录/事务/来源；signed() 调用 remember_revision，scan/preview 共用其路径 | runtime.rs L389–408、L577–681 | 验证与记忆分开，投影/预览不写 revision |
| signed 身份优先；ZIP/EXE release 与 selection 分开；最近 committed 按 updated_at | runtime.rs L1505–1690、L1776–1802 | 移动算法，不把 superseded 当当前安装 |
| catalog 哈希有链接/尺寸检查；dynamic hash_target 将 open/read 失败当 None/missing | [catalog.rs](../../../crates/core/src/catalog.rs) L270–396；runtime.rs L1663–1707 | 唯一取证接口，读失败不等于不存在 |
| facts 有 nofollow、身份/变化检测、limit、取消 | [facts.rs](../../../crates/core/src/installer/facts.rs) L250–345 | 复用可靠读取保障 |
| signed 在 CLI，ZIP/EXE 已在 core | runtime.rs L1043–1470；[orchestrator.rs](../../../crates/core/src/installer/orchestrator.rs) L71–96、L472；[exe_orchestrator.rs](../../../crates/core/src/installer/exe_orchestrator.rs) L54–75 | 统一服务路由，CLI 删除 signed 业务流 |
| trust.sqlite 使用 FULL/5 秒 busy timeout/Immediate 二次 revision 检查 | runtime.rs L336–385 | 表、路径和原子防回退保留 |
| cleanup allowlist 已有，但 CLI 遍历/删除没有覆盖安装全生命周期的互斥 | runtime.rs L688–723；[store.rs](../../../crates/core/src/installer/store.rs) L164–219 | CacheService + 跨进程缓存锁 |
| outcome_json 平铺事务；cleanup.performed 由 state 推断；signed 失败含 contains("cancel") | runtime.rs L1125–1145、L1450–1468 | structured cause/disposition、真实独立 cleanup |
| 低层 prepare/apply 公开，文件注释要求调用者先认证目录 | [install.rs](../../../crates/core/src/install.rs) L1、L855–920、L1425–1430 | 正式库入口不能绕过认证；低层收为 crate 内 |
| Rust ID 用 bytes；Python _id 用字符数；Windows detail 加省略号 | main.rs L41–56；[bridge.py](../../../apps/decky/backend/bridge.py) L74–78；[Tauri main.rs](../../../apps/windows/src-tauri/src/main.rs) L226–240 | 共享字节约束/fixture，截断不加省略号 |
| contracts 只有参考 README/catalog schema；CLI schema 仅生成 CatalogPayload | [contracts](../../../packages/contracts/README.md)；[maintenance.rs](../../../crates/cli/src/maintenance.rs) L56–58 | 完整 wire 生成器；目录 schema/维护命令保留 |

Decky tasks.py 已使用精确取消错误码集合，不是 contains；需删除的是 CLI signed 的字符串推断以及各消费者重复业务映射。

## 2. 投影快照与只读接口

目标模块为 core::projection/detection/actions、installer::service/cache、trust::revision_store。core 不依赖 protocol。以下 Rust 省略 imports/derives，为实施规格。

```rust
pub struct ProjectionInputs<'a> {
    pub steam: &'a steam::ScanReport,       // 已有实例/Steam diagnostics；不重新发现库
    pub catalog: CatalogView<'a>,
    pub sources: Option<&'a SourceSnapshot>,
    pub transactions: &'a TransactionSnapshot,
    pub roots: &'a [PathBuf],
    pub now: u64,                          // 秒，整批统一时刻
    pub platform: Platform,
    pub capabilities: &'a HostCapabilities,
    pub cancel: &'a AtomicBool,
}
pub enum CatalogView<'a> {
    Verified(&'a VerifiedCatalog),
    Development(&'a Catalog),
    Unavailable { error: &'a DomainError },
}
pub struct ScanReport {                    // projection::ScanReport，与 steam::ScanReport 区分
    pub catalog_revision: u64,
    pub catalog_trust: CatalogTrust,
    pub catalog_error: Option<DomainError>,
    pub platform: Platform,
    pub roots: Vec<PathBuf>,
    pub settings_revision: Option<u64>,
    pub sources: Vec<SourceSummary>,
    pub installs: Vec<InstanceProjection>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}
pub struct InstanceProjection {
    pub game: GameInstall,
    pub match_status: MatchStatus,
    pub patches: Vec<ProjectedPatch>,
    pub sources: Vec<GameSourceSummary>,
    pub matches: Vec<SourceMatch>,
    pub discovery: Option<GameDiscovery>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}
pub fn scan(inputs: ProjectionInputs<'_>) -> Result<ScanReport, DomainError>;
pub fn project_instance(inputs: &ProjectionInputs<'_>, instance: &InstanceId)
    -> Result<InstanceProjection, DomainError>;
pub fn project_batch(inputs: &ProjectionInputs<'_>, instances: &[InstanceId])
    -> Result<Vec<InstanceProjection>, DomainError>;
pub fn preview(inputs: &ProjectionInputs<'_>, request: PreviewRequest)
    -> Result<InstallPreview, DomainError>;
```

SourceSnapshot 是拥有数据的只读 registry 快照：settings_revision、规则/订阅、候选、来源状态、缓存 discovery，不含可写句柄。TransactionSnapshot 是同一 installer root 的事务及需要的 committed plan/读取诊断。构造器按状态边界只读加载，不初始化状态；缺库返回空来源，不吞掉已有损坏库的错误，损坏进入 diagnostics。VerifiedCatalog 内部封闭，不接受 caller "trusted=true"。

scan 按 steam.installs 顺序；batch 去重并保持首次出现顺序，空列表返回空，未知 ID 返回 instance_not_found。来源更新按 AppID 找出**所有库实例**再 batch；不重新扫描整个 Steam 库。同一刷新复用实例/事务/来源快照，generation/sequence/关闭/迟到事件留给宿主；哈希不做跨文件变化的长期无失效缓存。

ProjectedPatch 保留 release_id/source_id/candidate_id/selection_id/name/source_page/support_level/compatibility/detection，新增必需 actions；删除 download_available 及 UI auto_installable/reinstall_available/安装状态别名。preview 输出统一 instance_id、release_id/selection_id（可空）、catalog_revision、executable=false、blockers、changes、diagnostics；动态/signed 不再有不同 change DTO。ProposedChange 保留 path/action，baseline_matches/actual_sha256 缺证据为 null。preview 不下载、不建计划、不记忆 trust。

### 2.1 受限取证与诊断

```rust
pub struct EvidenceRequest<'a> {
    pub game: &'a GameInstall,
    pub relative_path: &'a RelativeGamePath,
    pub expected_sha256: &'a Sha256Digest,
    pub max_bytes: u64,
    pub cancel: &'a AtomicBool,
}
pub fn inspect(request: EvidenceRequest<'_>) -> FileEvidence;
pub struct FileEvidence {
    pub path: String,
    pub status: EvidenceStatus,
    pub actual_sha256: Option<String>,
    pub diagnostic: Option<EvidenceDiagnostic>,
}
```

RelativeGamePath::parse 沿用 catalog::relative_path，拒绝空/绝对/父目录/驱动器/ADS/非法分隔符；不接受绝对目标。检查 library/game 边界、所有祖先和最终路径 symlink/junction/reparse、普通文件、nofollow 打开、句柄/路径身份、修改时间/尺寸、累计读取限额。现有 manifest 1 MiB、EXE baseline 512 MiB 和 worker/安装限额保留；projection 指纹流式限额冻结为现有 space::MAX_ORIGINAL_BYTES（16 GiB），超限为 unknown。该投影限额是新增边界，P1 需验证大文件负向 fixture。尺寸不变而身份/修改时间变化仍拒绝。

| 事实 | status | diagnostic.code | actual_sha256 |
| --- | --- | --- | --- |
| 边界安全且文件确实不存在 | missing | 无 | null |
| 安全完整读取，hash 相同/不同 | match / mismatch | 无 | 已验证 hash |
| 权限/IO 失败 | unreadable_or_unsafe | evidence_read_failed | null |
| 非法相对路径/边界失败 | unreadable_or_unsafe | evidence_invalid_path / evidence_boundary_violation | null |
| 链接/reparse/非普通文件 | unreadable_or_unsafe | evidence_linked_path / evidence_not_regular_file | null |
| 超限/溢出/变化/取消 | unreadable_or_unsafe | evidence_size_limit / evidence_size_overflow / evidence_changed / evidence_cancelled | null |

整体 DetectionStatus 固定 unknown/not_detected/partial_or_conflict/installed_external/installed_managed；任何不可靠文件为 unknown，空证据不能因 all(empty) 判安装。missing 仅来自已验证安全边界的 NotFound。EvidenceDiagnostic={code,detail?}；ProjectionDiagnostic={code,instance_id?,path?,detail?}，path 只用受限相对路径（Steam 原扫描诊断保留原 path 事实）；不泄露文件内容。detail 限 2048 UTF-8 bytes。source/Steam 的自由诊断不得解释为安装权限。

## 3. 统一安装请求与依赖

```rust
pub enum InstallTarget {
    SignedRelease { release_id: ReleaseId, archive: Option<LocalArchive> },
    PublisherZipSelection { selection_id: ZipSelectionId },
    PublisherExeSelection { selection_id: ExeSelectionId },
}
pub struct InstallRequest {
    pub request_id: RequestId,
    pub instance_id: InstanceId,
    pub operation: InstallOperation,
    pub target: InstallTarget,
    pub cleanup_policy: CleanupPolicy,
}
pub struct InstallerDeps<'a> {
    pub host: &'a ValidatedHostContext,
    pub catalog: &'a dyn TrustedCatalogProvider,
    pub revision_store: &'a dyn TrustRevisionStore,
    pub transport: &'a dyn Transport,
    pub worker: Option<&'a WorkerRuntime>,
    pub control: &'a dyn TaskLease,
    pub clock: &'a dyn Clock,
    pub emit: &'a mut dyn FnMut(Progress),
}
pub fn install(request: InstallRequest, deps: InstallerDeps<'_>)
    -> Result<InstallerOutcome, DomainError>;
pub(crate) struct SignedInstallInputs<'a> {
    pub game: &'a GameInstall,
    pub release_id: &'a ReleaseId,
    pub archive: Option<&'a LocalArchive>,
    pub operation: InstallOperation,
    pub request_id: &'a RequestId,
    pub now: u64,
    pub ttl_seconds: u64,
}
pub(crate) struct SignedInstallDeps<'a> {
    pub common: InstallerDeps<'a>,
    pub paths: &'a InstallerPaths,
}
pub(crate) fn install_signed(inputs: SignedInstallInputs<'_>, deps: SignedInstallDeps<'_>)
    -> Result<InstallerOutcome, DomainError>;
pub trait Clock { fn now_seconds(&self) -> Result<u64, DomainError>; }
pub trait TaskLease {
    fn cancelled(&self) -> bool;
    fn lease_active(&self, now_seconds: u64) -> bool;
    fn deadline_ms(&self) -> u64;
    fn cancellation_cause(&self) -> Option<CancellationCause>;
}
```

ID newtypes 字段私有，checked constructors 执行 UTF-8 bytes/control/prefix 校验；PreviewRequest={instance_id,target:PreviewTarget}，PreviewTarget 为 SignedRelease/PublisherZipSelection/PublisherExeSelection（不含 archive/operation）。LocalArchive 经宿主选择仍由核心检查普通文件/链接/大小/hash。selection 结构上不能带 archive。

ValidatedHostContext::new(state_dir,steam_roots,platform) 检查实际进程身份，不接受 caller 声明 UID；只读可记录身份，写入口必须 ensure_write_identity（非特权），core 不降权。状态 canonical future/no-links，与所有扫描游戏双向不重叠；library/game 从受信 Steam 根重读，不能信任 caller 提交配方/GameInstall 以写任意目标。上下文拥有唯一 state/InstallerPaths，禁止两者不一致。Clock/Transport/catalog provider/worker 是受信库依赖，不能从 RPC 注入。TrustedCatalogProvider 返回原签名 envelope+公钥供 core 验证，不能送已认证 release struct 绕过签名。正式构建 provider/worker 路径由宿主固定。

HostCapabilities={zip:LinkCapability,exe:LinkCapability}，LinkCapability={available,blockers:Vec<DomainError>}；snapshot 仅用于投影。执行时再探针 worker 隔离和固定二进制，不能信任过期 links；平台不得伪装。install 重新读取 manifest/build/language/DLC、source revision/规则/订阅/selection、信任有效期/撤销、lease/revocation、事务，并在 apply 前重核 authorization。

Signed 顺序：身份/边界/请求 → cache shared lock → RequestClaim → TaskControl → 当前签名验证+原子 revision 记忆 → release 准入 → begin_request 幂等绑定 → 本地 ZIP/下载 → recipe/content conflict → plan → facts/authorization → 应用前重核 → apply → record_terminal → 真实收尾 → 可选缓存清理。下载/分析不持 journal.lock。Install/Reinstall/Overwrite 正式语义保留；主按钮 overwrite；请求账本绑定实例/选择/release/operation/content，不因 v4 清空。

仅统一 install 和受相同安全上下文保护的 recover/discard_plan 是正式库写入口；低层 prepare/apply/裸 ZIP/EXE route 收为 pub(crate)，防止直接库调用绕过认证。只读不要求写身份且不创建状态；所有适用写入口（安装/恢复/discard/清理/revision记忆/来源写）执行实际身份/路径检查。core Linux 实际错误码 installer_host_requires_non_root，与宿主早拒绝 unprivileged_user_required 区分。

补充可直接实施的构造/依赖接口（目标规格，非现有实现）：

```rust
pub struct TrustedCatalogMaterial { pub envelope: Vec<u8>, pub keys: TrustKeys }
pub trait TrustedCatalogProvider {
    fn load(&self) -> Result<TrustedCatalogMaterial, DomainError>;
}
impl ValidatedHostContext {
    pub fn new(state_dir: &Path, steam_roots: &[PathBuf], platform: Platform)
        -> Result<Self, DomainError>;
    pub fn ensure_write_identity(&self) -> Result<(), DomainError>;
}
impl SourceSnapshot {
    pub fn load_ro(state_dir: &Path, steam: &steam::ScanReport)
        -> Result<Option<Self>, DomainError>;
}
impl TransactionSnapshot {
    pub fn load_ro(state_dir: &Path, steam: &steam::ScanReport)
        -> Result<Self, DomainError>; // 不存在为空；路径/plan读取诊断保留
}
pub fn recover(host: &ValidatedHostContext) -> Result<Vec<Transaction>, DomainError>;
pub fn discard_plan(host: &ValidatedHostContext, plan: &PlanId)
    -> Result<Transaction, DomainError>;
```

provider 原 envelope 大小仍由 core trust::read_bounded/from_bytes 限额验证，keys 校验及验签在 core，不由 provider 自报可信。snapshot loader 先检查状态与已扫描实例边界/no-links，transaction loader 只读取 installer/rust-v1，绝不创建/初始化。外部可自行构造只读投影输入，不会获得写权限；正式写入口不接受投影 GameInstall 作为目标。

### 3.1 TrustRevisionStore

```rust
pub trait TrustRevisionStore {
    fn highest_revision(&self) -> Result<u64, DomainError>; // 缺失为0；不建目录/DB
    fn remember_verified_revision(&self, catalog: &VerifiedCatalog)
        -> Result<(), DomainError>;                       // 不接受任意 u64 伪装验证结果
}
pub fn verify_catalog(provider: &dyn TrustedCatalogProvider, store: &dyn TrustRevisionStore,
    now: u64) -> Result<VerifiedCatalog, DomainError>;      // 校验，不 remember
```

生产 SqliteTrustRevisionStore 固定 state_dir/trust.sqlite、catalog_revision(id=1,revision)，FULL/Immediate/5秒 busy timeout/i64范围与事务内二次读取不变。验证时最高 revision 失败是 catalog_revision_rollback；并发 remember 回退仍 catalog_rollback，保留两个现有稳定代码。scan/preview 从不 remember；安装显式记忆，apply 前用当前 highest_revision 再核验。测试内存 store 只用于 fixture；正式宿主不能选无持久/第二DB实现。

## 4. 缓存与结构化结果

```rust
pub enum CleanupPolicy { Disabled, AfterCommit }
pub struct CacheService<'a> { /* validated host；私有字段 */ }
impl CacheService<'_> {
    pub fn inspect(&self) -> Result<CacheInventory, DomainError>;
    pub fn cleanup(&self) -> Result<CleanupResult, DomainError>;
}
pub struct CleanupResult {
    pub status: CleanupStatus,
    pub performed: bool,
    pub removed_bytes: u64,
    pub failures: Vec<CleanupFailure>,
}
pub enum CleanupStatus { NotRequested, Completed, Partial, Failed, Busy }
pub struct CleanupFailure { pub code: CleanupErrorCode, pub detail: Option<String> }
```

CacheInventory={total_bytes:u64,entries:u64,diagnostics:Vec<CleanupFailure>}，只读 inspect 不建状态/锁文件。新 cache.lock 固定 installer/rust-v1/cache.lock，自身受保护；安装/恢复全生命周期 shared、删除 exclusive，获取顺序 cache → request/task owner → journal。AfterCommit 真实事务收尾后释放本任务 shared，再尝试 exclusive；其他任务占用为 Busy cleanup，不能改变 committed/死锁升级。手动冲突返回 installer_busy。Decky 显式传既有开关，Windows Disabled，偏好文件留宿主。

allowlist 仅 content/downloads/reports；tasks 用已有 owner.lock/lease/终态安全回收，未知/恢复 task 不删。runtime/jobs/receipts/journal/requests和sidecars/schema/locks/authorization/revoked/stage/backup、sources/trust/settings/history 不删。不跟随链接/不接受 caller 删除路径，删除前核对身份，计数 checked_add；removed_bytes 只算确认删除，部分失败 Partial+failures，不能把预计字节当成功。前置不安全返回 DomainError；服务执行后的部分失败返回 CleanupResult，ok:true 表示调用完成，UI 看 status。

InstallerOutcome 保持 v3 事务字段平铺：transaction_id、plan_id、instance_id、selection_id、content_id、release_id、state、error（原事务字符串）、created_at、updated_at、has_replacements；新增必需 disposition、cause（可空）、recovery（可空）、cleanup（必需）。无事务不产生 outcome；身份来自事务/验证路线，不靠 release 前缀猜 content。历史缺 selection/content 保持 null。

| 类型 | 固定字段/值 |
| --- | --- |
| Disposition | completed / cancelled / failed（与宿主现有任务三终态一致） |
| FailureCause | kind=user_cancelled / lease_expired / domain_failure；code:string；detail?:string；无失败为 null |
| RecoveryRequirement | required=true；code?:string；detail?:string；无需恢复为 null |
| CancellationCause | user_cancelled / lease_expired；stdin EOF=user_cancelled；失联传输错误留宿主 |
| CleanupErrorCode | cleanup_failed / cleanup_linked_path / cleanup_unknown_path / cleanup_size_overflow / installer_busy |

committed 永远 completed；needs_recovery → failed + state=needs_recovery + recovery.required=true，明确恢复需求；仅取消/租约且恢复确认 rolled_back 才 cancelled，其余 rolled_back 为 failed。撤销/准入失效属 domain_failure；清理不能改 disposition。计划前失败仍 ok:false，计划前取消 code=cancelled，宿主按精确 code 记取消；传输失联不能假造事务，之后 recover/history 取得事实。详见真值表。

精确 outcome 类型（目标规格；Option在wire为null，FailureCause.detail/RecoveryRequirement.code/detail可省略，表中事务字段保持既有类型）：

```rust
pub struct InstallerOutcome {
    pub transaction_id: String,
    pub plan_id: String,
    pub instance_id: String,
    pub selection_id: Option<String>,
    pub content_id: Option<String>,
    pub release_id: String,
    pub state: TransactionState,
    pub error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub has_replacements: bool,
    pub disposition: Disposition,
    pub cause: Option<FailureCause>,
    pub recovery: Option<RecoveryRequirement>,
    pub cleanup: CleanupResult,
}
```

幂等重放必须得到相同事务事实与终态：RequestLedger现有记录只有Transaction/error，不新增持久cause列。新取消在确认回滚后将typed cause的规范码写入已有Transaction.error与ledger.error（user_cancelled写cancelled；lease_expired写lease_expired）再record_terminal；正常失败保留原诊断事实。读历史/重放时仅error精确等于cancelled/task_cancelled/lease_expired转换取消原因，不能contains；committed先completed，needs_recovery先failed+required，其他rolled_back含null error保守failed。新任务当前TaskLease可提供实时cause，但不允许覆盖持久committed/needs_recovery或把缺证据的旧记录猜成cancelled。cleanup重放不再自动删除，返回NotRequested/removed_bytes=0；disposition与事务事实稳定，清理是本次调用独立副作用事实。

P2 修订（2026-10-12）：上述精确三码规则允许事务 writer 已有的固定非空后缀 `; cleanup_pending:<诊断>`。仅先分离该固定语法再精确比对三码；`from_code` 本身仍严格，不接受其他自由详情或 contains。原 Transaction.error/ledger.error 完整保留，typed cause.code 是精确机器码，清理诊断进入有界 cause.detail；普通领域失败也保持其机器码与清理详情分离。此修订不增加 schema，不改变 commit / needs_recovery 优先级，目的为保证非致命收尾不改变取消/失败事实；实际验证见 [P2](p2-validation.md)。

## 5. 权威动作 DTO

```rust
pub struct PatchActions {
    pub automatic: ActionAvailability,
    pub local_archive: ActionAvailability,
    pub reinstall: ActionAvailability,
    pub overwrite: ActionAvailability,
    pub primary: PrimaryAction,
}
pub struct ActionAvailability { pub available: bool, pub blockers: Vec<ActionBlocker> }
pub struct ActionBlocker { pub code: ActionBlockerCode, pub detail: Option<String> }
pub struct PrimaryAction {
    pub operation: InstallOperation, // 恒 Overwrite
    pub label: PrimaryLabel,        // Install | Overwrite
    pub input: InstallInputKind,    // Automatic | LocalArchive | Unavailable
}
```

automatic 是无需本地文件的 overwrite 发起；local_archive 仅签名 ZIP；reinstall 保留严格 owned/content 条件；overwrite 为当前 target overwrite 可发起（automatic/local_archive 任一）。primary input 优先 automatic，其次 local_archive，否则 unavailable；enable 消费对应 availability。label 按收据/当前文件证据，与下载可用性无关；已有/部分管理或外装证据为 overwrite，否则 install，operation 都 overwrite。UI 管 busy/可见/选择，不得修改 actions；最终执行仍重核。

ActionBlockerCode 固定集合：unsigned_catalog、catalog_unavailable、catalog_expired、catalog_revision_rollback、catalog_rollback、release_revoked、release_not_matched、platform_not_verified、game_not_ready、build_not_verified、source_requires_guidance、artifact_requires_manual_download、recipe_missing、source_not_enabled、source_not_subscribed、exe_source_disabled、selection_not_found、invalid_selection_id、selection_stale、zip_download_unavailable、zip_download_ambiguous、exe_download_ambiguous、invalid_download_url、exe_candidate_ambiguous、exe_candidate_variant_unsupported、exe_sandbox_unavailable、installer_host_requires_non_root、unprivileged_user_required、reinstall_unowned_or_incomplete、reinstall_content_identity_changed、needs_recovery、local_archive_signed_only。新增投影理由仅 unsigned_catalog/catalog_unavailable/local_archive_signed_only；其他沿用准入/resolver，不创造 platform_unsupported/link_unsupported 同义码。

签名本地可绕过 artifact 缺失/远端 download 不可用，**仍必须 ZIP links/身份、签名 recipe/hash/准入**。unknown 不自动阻断成熟安全 overwrite，match 也不能越过来源/信任/平台条件。EXE 发起可用不等于写授权，preview 的 exe_check_required 仅预览诊断。blockers 去重、规则顺序稳定，available 等价 blockers 空。活动任务 busy 属宿主交互与 core 锁竞争，不能用过期投影 busy 替代互斥。当前实例存在 needs_recovery 时，四种写动作均 unavailable + needs_recovery；清理/恢复材料不得借 overwrite 清除。

## 6. v3 → v4 正式 wire

唯一 protocol_version=4；v1/v2/v3/未来未知版本拒绝 unsupported_protocol，不双读/不降级。信封仍 {protocol_version,request_id,method,params} 与 {protocol_version,request_id,ok,result|error}；逐方法 Params/结构 DTO deny_unknown_fields。诊断 code 可未知但不驱动授权。

v4每个 params 必需 host:HostInputs，HostInputs={roots:PathBuf[],catalog:PathBuf|null,signed_catalog:PathBuf|null,trust_keys:PathBuf|null,state_dir:PathBuf|null,platform:Platform}，六字段均必需（不可用为null），与表中业务字段并列；这些由宿主固定注入、UI 不得传。v3 params 顶层六配置移入 host，旧顶层字段拒绝，不设双读。CLI 不接受 worker/env/recipe/URL；roots<=32 保留。wire target 保留 release_id XOR selection_id，经转换得到 InstallTarget，**不再并存第二 target 对象**。删除 params.request_id，外层 ID 是唯一幂等 ID；operation 改必需，避免缺省 Install；cleanup_policy 是宿主注入必需 disabled/after_commit，UI 不写偏好。

| 正式方法 | 通道 / v4 业务 params | v3 → v4 result与行为 |
| --- | --- | --- |
| get_capabilities | serve / 无 | 必需 contracts 子对象；现有版本/links保留 |
| scan | serve / 无 | ScanReport/actions/诊断；删 download_available/旧UI派生；不记忆 revision |
| preview | serve / instance_id + release_id XOR selection_id | 统一 InstallPreview/nullable change evidence/diagnostics；不记忆 revision |
| install_patch | serve-task / instance_id、target两字段、operation；signed可 archive | outcome disposition/cause/真实cleanup；仅外层ID；宿主 cleanup_policy |
| list_transactions | serve / 无 | Transaction[] 保留，历史可选身份 null；不补写历史 outcome |
| recover | serve / 无 | Transaction[] 保留；相同安全上下文/cache shared lock |
| discard_plan | serve / plan_id | Transaction 保留；相同安全上下文/锁 |
| cleanup_cache | serve / 无 | {performed,removed_bytes} → CleanupResult |
| list_sources | serve / 无 | SourceSummary[] 保留、生成类型 |
| list_game_sources | serve / instance_id或appid（同时给instance优先） | GameSourceSummary[]保留，未安装AppID订阅读取保留 |
| set_source_enabled | serve / source_id、enabled | {settings_revision,source_id,enabled}保留 |
| set_game_source | serve / source_id、subscribed、instance_id或appid | {settings_revision,source_id,subscribed}保留，instance优先 |
| add_source | serve / rule:RawSourceRule | {settings_revision,rule:SourceRule}保留；Raw允许验证型宽松输入 |
| refresh_sources | serve-task / force默认false | {settings_revision,statuses}保留；生成事件union |
| scan_sources | serve-task / force默认false | {settings_revision,results}保留；宿主按受影响实例批量投影 |
| discover_game | serve / instance_id、name_hints默认[]、preferred_language?、force默认false、generation? | {instance_id,appid,generation,settings_revision,status,candidates,diagnostics}保留 |

删除方法保持 unsupported_method：plan_install/start_install/fetch_artifact/migrate_state/get_source_state/get_source_settings/list_source_matches/get_source_policy/resolve_source_record，及任何未列方法。renew/cancel 非JSON控制行，15秒TTL/EOF持久撤销与worker组清理、一进程一任务保留；serve拒绝三任务方法 task_required。

### 6.1 事件与枚举

| 事件 | v4字段/变化 |
| --- | --- |
| install_progress | 平铺 event/version/request_id/sequence/instance_id/selection_id/stage/substage/bytes_done/total_bytes/files_done/total_files/current_file/archive_bytes；仅version与生成校验变化，未知总量null |
| source_progress | event/version/request_id/sequence/completed/total/running/settings_revision，形状保留 |
| source_update（游戏） | event/version/request_id/sequence/appid/index/instance_id/settings_revision/discovery；按 discovery 判别union，形状保留 |
| source_update（来源刷新） | event/version/request_id/sequence/source_id/settings_revision/status；按 source_id 判别，不混游戏字段 |

sequence每任务从0严格递增、全部事件共用；request_id匹配。重复同序列丢弃，回退新事件不应用；非法首序列/结构/版本拒绝，terminal后不应用事件；无字段缺失猜测v3兼容。

| 固定枚举 | 唯一snake_case值 |
| --- | --- |
| Method | 上述16方法 |
| Platform / SupportLevel | windows/steamos；discover_only/manual_download/managed_zip/publisher_zip/publisher_exe/unsupported |
| InstallOperation / CleanupPolicy | install/reinstall/overwrite；disabled/after_commit |
| CatalogTrust / MatchStatus | signed/unsigned_development/unavailable；matched/no_catalog_match |
| DetectionStatus / EvidenceStatus | unknown/not_detected/partial_or_conflict/installed_external/installed_managed；match/mismatch/missing/unreadable_or_unsafe |
| PrimaryLabel / InstallInputKind | install/overwrite；automatic/local_archive/unavailable |
| Disposition / CauseKind / CleanupStatus | §4集合 |
| InstallStage / Substage | connecting/downloading/validating/analyzing_installer/extracting/planning/applying/rolling_back/cleanup；null/backing_up/writing/committing（后3仅applying） |
| TransactionState列表/恢复 | planned/backing_up/applying/committed/superseded/uninstalling/rolling_back/rolled_back/uninstalled/needs_recovery（现有持久有效10值） |
| 新install outcome state | committed/rolled_back/needs_recovery，不产生uninstalled/superseded |
| 来源现有枚举 | SourceRole=publisher/fallback_indexer；RootLayout=game_root；Variant=base/dlc/demo/unknown；MatchBasis=appid/verified_alias/official_title/title/none；Capability=discover_only；ArtifactFormat=zip/exe/web/unknown/mixed；CacheStatus=fresh/cached/stale |

历史 uninstalled/uninstalling **读取保留**不恢复旧uninstall操作；持久未知state报 installer_schema_unsupported保留文件。来源 discovery/status、Steam availability/kind 当前string继续生成string，不凭空缩减状态；它们不能决定权限，P5迁移时保留所有现有生产状态fixture。

### 6.2 capabilities 每字段变化

| 字段 | v3 → v4 |
| --- | --- |
| protocol_version / methods | 3→4；原installer8+sources8同顺序无重复 |
| installer.schema_version/selection_version/content_version/adapter_version/policy_schema_version/report_schema_version | 全保持1，持久/worker版本不随wire升级 |
| installer.formats / strategies | zip/exe=supported；managed_zip/publisher_zip/publisher_exe保留 |
| installer.links.zip/exe | available+blockers[{code,detail?}]保留；ZIP身份/EXE隔离按真实本机探针 |
| sources.discovery_schema_version/adapter_version/parser_version/query_version/methods | 当前1/1/2/1/8方法保留，导出源码常量 |
| 新增必需 contracts | {projection_version:1,actions_version:1,outcome_version:1,request_validation_version:1} |

宿主先握手再允许写：版本 mismatch=unsupported_protocol；methods/必需contracts缺失或不匹配=capability_mismatch；installer/worker schema不兼容=installer_schema_unsupported。每请求CLI仍在初始化/状态写入前验证version。不保留静态扫描旧核心降级/新旧字段双读。

### 6.3 错误与限额变化

| 正式错误/类别 | v3 → v4决策 |
| --- | --- |
| unsupported_protocol/unsupported_method/task_required | 代码保留；支持4、白名单16 |
| invalid_request/invalid_request_id/request_too_large | 保留；未知字段/缺operation/类型/UTF-8 bytes/control统一拒绝 |
| capability_mismatch/installer_schema_unsupported/invalid_core_response | 宿主拒绝保留，统一detail，不伪装空扫描 |
| instance_required/instance_not_found/selection_or_release_required/invalid_selection_id/archive_not_allowed_for_selection/invalid_operation/invalid_id | 保留现有路由/早拒绝代码，统一权威校验 |
| installer_busy/request_id_binding_conflict/cancelled/installer_lease_expired/needs_recovery | 保留；已形成事务用cause/recovery+disposition，无事务仍error信封 |
| cleanup_failed | 保留；部分删除诊断进入failures，前置拒绝仍error；新增§4安全诊断码 |
| 其他核心准入/下载/worker/来源/事实/授权错误 | 当前稳定code原样保留，不批量改名；DomainError={code,detail?}替代String分隔符 |
| 新 response_too_large | 序列化行超8MiB前拒绝，不截JSON/裁actions/静默增限 |
| 新 evidence_* / 新action理由 | §2.1/§5，仅投影诊断/动作理由，不当任务终态 |

DomainError.code 继续string以保留现有解析/安全/worker诊断，**不是授权enum**；封闭集合仅正式控制码、actions/结果/事件enum。每种生产错误通过现有负向fixture搬迁，不用generic failed或contains("cancel")。code非空、<=128 UTF-8 bytes、ASCII[a-z0-9_]；自由IO文本改detail，未知合法code原样展示而不授权。上表覆盖每类正式error的迁移，非枚举自由诊断不做伪完整清单。

当前自由字符串与稳定错误码必须区分；“原样保留”仅指原有机器码，不指含路径/SQL/解析说明的整串。已核对 install.rs L146、L1045–1052、L1171、L1419、L1578、L1668：

| 当前动态文本 | v4规范化 |
| --- | --- |
| journal_error:<SQL诊断> | code=journal_error，detail=原完整底层诊断（截断） |
| invalid_zip:<解析诊断> / zip_member:<解析诊断> | code=invalid_zip / zip_member，detail=对应原诊断 |
| file_changed:<相对路径> / recovery_conflict:<相对路径> | code=file_changed / recovery_conflict，detail=原相对路径说明 |
| baseline_conflict:<目标路径>（当前plan blocker，不一定外层error） | 外层以plan_has_blockers/plan_failed及detail保留计划诊断；预览blockers保留诊断字符串；如转DomainError则code=baseline_conflict、detail=路径 |
| 任意无法识别的IO自由文本 | code=io_error、detail=原文本；原已分类plan_failed/exe_analysis_failed等继续用原分类，不再把任意文本塞code |

优先在产生失败的core位置构建DomainError，P5删除String分隔符；开发过渡桥仅允许上表精确前缀解析并登记删除，不做任意“冒号前就是code”的推断。历史Transaction.error仍原串保留，不强制ASCII、不重写数据库；新cause/recovery/cleanup/error信封使用结构化code/detail。io_error是新增通用IO控制码，不能遮掉已有稳定细分错误。

请求64KiB、响应/事件8MiB、detail2048 UTF-8 bytes；v4统一按完整行bytes计数（包含一个末尾LF delimiter），限额数值不增；CLI main.rs L165–168和Decky bridge.py L211已计LF。Windows普通call-core有payload/行边界差异，P5统一严格整行限制，不增加一字节接受范围。request_id=1..128bytes，其他业务ID=1..256bytes，无Unicode控制字符（Rust char::is_control/Python category Cc）。Python拒bool冒充整数；sequence/u64非负范围内。detail在字符边界截断不追加省略号，cause/recovery/cleanup/blocker detail同限额。JSON Schema maxLength不能替代bytes校验。TS number使用安全整数范围，超过Number.MAX_SAFE_INTEGER报invalid_core_response不舍入；合法真实体积限额远低于此值。

新增actions增大响应：本文**未测真实序列化体积**；P1/P4/P5用最大fixture检查8MiB，超限明确报错；本轮不声称载荷/性能通过。

## 7. 生成、CI与版本拒绝门槛

唯一源是新crates/protocol：版本/方法/逐方法Params/验证/事件/response/capabilities、显式domain转换；core不依赖protocol。沿用schemars导JSON Schema，新增构建时ts-rs导TS（SBOM/license必须更新）。独立workspace binary steam-patch-contract-gen（目标crates/contract-gen）生成；不可加入插件运行allowlist。

目标输出入Git：packages/contracts/generated/v4.schema.json、protocol.ts、protocol.py（纯数据：version/methods/stages/limits/contracts），fixtures/v4/*.json。Python运行时消费打包protocol.py、不运行生成器；bytes/ID/detail helper通过相同fixture校验；不能手写第二method/stage表。Tauri直接依赖protocol，TS导入生成类型，packages/ui不保留手写权威DTO。fixed fixtures覆盖每方法成功/失败、三事件和source_update两变体、三disposition及needs_recovery、Unicode/未知字段/阶段/超限/乱序/版本。

冻结实施命令：
- cargo run --locked -p steam-patch-contract-gen -- generate：确定性UTF-8/LF、排序、无时间/绝对路径。
- cargo run --locked -p steam-patch-contract-gen -- check：临时生成逐字节对比，漂移非零退出。
- cargo test --locked -p steam-patch-protocol：Rust fixtures；现有Decky Python测试与双端TS test/typecheck/build消费同fixtures。
- 现有 [CI](../../../.github/workflows/ci.yml) core/windows/decky jobs增加check，生成后无diff；packager加入Python常量/v4manifest，排除生成binary。

P1/P2先实现domain；开发期旧Params conversion只在适配层登记删除，不存在第二正式真值实现。P5整体切v4，同步CLI/Tauri/Decky/UI/contracts/指南/测试/包，删除手写wire/String separator/旧权限字段/fallback。P6源码检索区分历史文档与生产v3/旧方法/helper；历史持久defaults继续读。

必须验证：v3请求到v4 CLI无状态/DB/游戏写；v3 response/event到v4宿主在后续写前unsupported_protocol；缺contracts/方法/worker schema mismatch；旧UI缺actions报contract_mismatch不猜download_available；错request_id/乱序/terminal后事件；精确64KiB/8MiB完整行（含LF）与超出一字节的边界、超限serve下一合法请求仍可处理；最终匹配包固定CLI/worker真实JSONL冒烟。生成器/check/fixture此时均待实现，不能把目标命令记为通过。

持久路径固定installer/rust-v1、sources/rust-v1、state_dir/trust.sqlite；新增cache.lock只保护，不建第二store/schema。settings/task-history沿宿主现有路径，详见数据基线。升级不重写历史事务/收据/请求绑定，不清空缓存/恢复材料；wire v4不升级installer schema=1。

## 8. 本轮与后续验证

本轮仅本文，不修改生产、不连接设备、不安装/迁移用户状态。静态核对源码方法/事件/schema/限额/store与目标接口；链接/diff结果交施工记录。P0无可编译实现：编译、生成/fixture、锁竞态、性能/响应体积、双端产物与实机全部待验证。

实施门槛：直接库路径/身份/准入负向；scan/preview缺目录不创建且revision不变；并发trust防回退；cache安装锁竞争/partial字节；三路线同终态；Rust/TS/Python字节/字段/事件一致；混装写前拒绝。未实现/未验证项不能宣称feature完成。
