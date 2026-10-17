# 安装磁盘占用与兼容性精简施工记录

关联：[设计](design.md) · [代码审计](audit.md)。

## 当前状态

**2026-10-16 经用户明确授权恢复生产施工；D01–D10 删除已实施，并通过自动化检查。** 施工在当前未提交工作区基础上增量进行，保留其他功能改动，未整体 reset/checkout/覆盖文件。下方按日期记录的方案、回滚和验证均为对应阶段历史事实；本次新增成果与验证见文末“恢复生产施工与 D01–D10 实施”一节。

| 工作项 | 状态 |
| --- | --- |
| 初稿材料生命周期/兼容性审计 | 历史调研完成，旧决定已按新设计取代 |
| 增量空间、无stage、格式/手动兼容、任务通道等 | 工作区已有未提交实现；本轮补齐 P1/P3/P4 收敛项（容量模型、外部ZIP受管理化与缺料诊断、ZIP64 checked 转换），实机验收待执行 |
| EXE 无备份直接提取覆盖（子功能） | D1–D4 已实施并通过本地自动化；D5 双平台实机与性能待验收，详见子功能施工记录 |
| 删除优先复审及D01–D10范围、保留项、验收标准 | 已写入设计与审计 |
| D01–D10 直接删除实施 | 已实现；Rust 工作区测试与检查通过，详见文末 |
| 超大常量绕过（2GiB/512MiB 等）改为真实删除 | 已实现；真实大包/大 EXE 端到端路径待验证 |
| 新设计下自动化、真实大包、双平台实机容量/性能/恢复 | 自动化已执行；真实大包与双平台实机仍待验证 |

## 2026-10-15：调研与方案

- 读取“排查补丁选择信息过期”和“排查夏日狂想曲补丁安装失败”，筛选用户消息与最终总结；核对已有来源持久化、真正force、手动候选、本体+DLC及镜像修复，SF默认值限制移除和双平台/Deck既有记录。
- 基线HEAD为be099fa；工作区另有backup-cleanup未提交生产代码、测试、协议及文档。只读参考其接口和锁方向，未覆盖、回退、暂存或提交其改动；功能索引保留其当前内容。
- 静态确认ZIP prepare完整stage、全量原文件备份、逐成员.tmp写入、前缀恢复对stage依赖和transient终态清理边界。
- 静态确认EXE固定空闲门槛、完整预算重复检查、sandbox输入与artifact共享目录所有权，以及下载/format/report/worker多层2GiB限制、部分ZIP64能力、3GiB内存和CPU/墙钟预算。
- 澄清生产worker为整包mmap加流式载荷解码，不是整包read_to_end；512MiB条件文件摘要限制属于流式调用点门槛。手动unknown/demo内容ID已可表达，残留问题主要是后续事实聚合/构建/语言/DLC及来源能力范围。
- 建立审计编号，区分可删除人为门槛、须先补能力的限制、正面结构/一致性检查与历史已修复项。索引截断为静态风险，尚未复现真实候选丢失。
- 设计采用archive直接流式写入成员.tmp并校验rename；保留全部backup到终态、缓存租约和旧事务恢复。按阶段/卷计算未来增量，显式包含目标净增长和恢复scratch，附理论示例及待验证矩阵。

## 验证记录

- 已执行：只读Get-Content/rg/git status/git diff与两个聊天读取，核对当前代码和未提交范围。
- 工具限制：初期默认沙箱命令异常，采用自动审批后的只读PowerShell；中途一次审批服务额度错误使命令未执行，用户要求继续后重试成功。未将失败命令当作验证结果。
- 文档链接及diff检查：164个本地链接目标存在，代码行锚点均未越界；三个新文档无行尾空白/冲突标记；功能索引diff检查通过。本轮写入范围为三个新文档及索引新增行，另有忽略目录中的检索中间文件，不属于生产改动。索引原有backup-cleanup状态由另一聊天更新并保留。
- 未执行：Rust/前端/Python测试、构建、网络下载、EXE转换、设备部署、真实游戏写入、空间/内存采样。原因：本轮仅制定方案。

## 后续施工待办（当前执行入口）

1. 按设计P0优先交付可独立实施的D01–D10删除及必要回归；移除TTL、心跳和MIME等门槛不等待容量、无stage或大文件重构。实施前重核工作区与backup-cleanup接口，保留其他功能改动。
2. P2归入D06–D07，直接收窄实际条件检查并贯通双端；P5与D03–D04一起完成持续查询、结果发布、取消及失败终态，不另建续轮或重试平台。
3. 材料释放与固定阶段空间计算、无stage事务、大文件能力分别验收。无stage依赖恢复材料和缓存锁正确性；大文件开放依赖端到端格式/资源能力。容量计算不引入通用材料图或卷级预留服务。
4. D08逐项记录保留字节上限的保护对象、数值依据、检查位置及超限行为；未核实项标为待核实。补必要的截断/编码/extra复现和合法输入边界验证，不先建分页平台。
5. 按每项实际变更补充必要检查，分别记录已实现、自动化验证、实机验证与待验收。沿用有效历史验证，不将已有未提交实现或此前局部回滚认定为新方案整体完成。

## 2026-10-16：补充查询中断体验问题（仅方案）

以下保留当时方案记录；其中续轮、无进展暂停等决定已被后续删除优先设计取代，不作为当前施工指引。

- 用户指出最大查询时间等门槛经常令查询半途结束，纳入P0问题，设计第9节明确单轮预算只用于让出执行权，由共享任务自动续查。
- 核对29秒总上下文/24请求、官方首轮6秒/3额外请求；已有Kagura检查点，但两端单游戏调用未自动续查。外层RPC超时仍需现场日志定位，未将静态风险当成实际复现。
- 补充按来源公平调度、结果渐进展示、取消/恢复、退避与无进展暂停，以及两端/协议验收。优先独立推进查询任务闭环，不等待磁盘路线完工。
- 未改生产代码、未运行真实网络查询、未部署或提交Git。新增文档链接与空白检查见本次检查结果；旧164链接为上一轮检查历史，不代表本次链接总数。

## 2026-10-16：删除优先复审与局部回滚

### 产品决定

- 用户明确要求删除徒增复杂度的防御策略，先停止生产施工、仔细审查并更新设计；随后明确要求此前代码中该回滚的部分回滚。
- 撤销“保留短期授权并自动续期”的决定，审定D01–D10：短期授权/计划TTL、前台心跳、查询总预算及续轮、固定无进展/退避截止、大小推导总时限、全事实快照、全局revision否决、数量许可线、来源ID名单、MIME否决。新增明确的配套字段/线程/错误码/测试清理要求。
- 缓存锁和journal恢复引用没有时钟到期，不与前台lease混删。临时写入、备份、原文件变化检查和必要条件求值继续保留。取消额外开发逐成员释放stage过渡实现。

### 实际回滚范围

- `sources/http.rs`撤回本功能新增的progressive标志、renew_slice、跨轮Retry-After集合/全局等待以及为长轮次添加的响应缓存修改，恢复HEAD传输实现。
- `sources/session.rs`撤回discover_progressively、完成来源集合、语义快照hash、两轮暂停、300秒截止及专属测试；保留已接入的定向查询参数/现有任务通道，每游戏暂恢复调用既有discover_game并发出结果事件。
- format/worker撤回work_seconds、按payload大小推导预算和30/60分钟上限，保留checked算术、文件格式能力、mmap地址空间处理及输入材料释放。
- 候选size_of公式和对应129候选专属测试撤回；新增暂停/等待文案随机制撤回。保留大索引真实字节检查及显式超限传播。
- 检查发现前次移除ZipInstallInputs.now后，service调用仍残留该字段导致编译失败；仅删除这一处过时参数以恢复一致性。
- 这些回滚恢复了旧29秒/24请求、旧worker时限和128候选门槛，**不是新方案已经完成**。D01–D10仍列为待实施；不以恢复旧行为充当最终精简。
- 未整文件还原其他功能，不触碰backup-cleanup实现；磁盘占用、无stage、格式和手动选择等既有有效改动保留。自动授权续期命令此前被审批拒绝而未执行，因此没有实际续期代码可回滚。

### 本轮验证

- 首次来源测试与CLI/worker编译发现上述过时now参数，修正后复检通过。
- `cargo test --locked --offline -p steam-patch-core --lib sources::`：150通过、0失败、1忽略；日志`.artifacts/simplification-rollback-sources.log`。
- `cargo check --locked --offline -p steam-patch-cli -p steam-patch-installer-worker`：通过，包含core/protocol依赖；日志`.artifacts/simplification-rollback-check.log`。
- 文档相对链接目标、行尾空白和冲突标记检查通过；已确认新增续轮引擎/renew_slice/work_seconds不再存在。
- 不重跑整项目或设备验收；未部署、未提交Git，真实磁盘峰值、真实大文件和设备性能仍待验证。

## 2026-10-16：计划审阅后的范围收紧（仅文档）

- 将容量模型限定为固定阶段纯计算及既有锁/串行约束，不新增通用材料图、持久化容量状态或卷级预留服务。
- D08要求说明保留字节上限的保护对象、数值依据、检查位置和超限行为；现有512 KiB/4 MiB策略值仍待施工核实，不能直接认定为合理边界。
- 明确查询失败终态：失败来源结束，其他来源继续；所有来源结束后任务结束并保留结果和错误，不因存在错误而一直运行。
- 更新P0/P2/P5依赖关系及当前待办；独立删除不等待容量或格式重构，无stage与大文件能力分别验收。保留此前回滚与测试历史，修正续轮仍“待精简”的过时状态。
- 本次仅修改设计、施工记录、审计说明和功能索引；未修改生产代码、运行代码测试、部署或提交Git。已检查四份文档的199个本地链接目标，均存在；行尾空白、冲突标记和索引diff检查通过。

## 2026-10-16：恢复生产施工与 D01–D10 删除实施

### 授权与工作区

- 用户明确授权恢复生产代码施工；设计第7节“暂停施工”为此前阶段状态，据此在本节记录恢复。此前“仅收紧文档、不改生产代码”的限制按用户本次指示解除。
- 施工在当前未提交工作区上增量进行：先核对 `git status`/diff，确认已有实现与其他功能改动（含 backup-cleanup）并全部保留；未整体 reset/checkout/覆盖文件，未提交或推送 Git、未发布、未部署设备。
- 删除优先原则执行：直接删除无独立作用的防御机制及其专属参数、字段、定时器、线程、状态文件、错误码、UI文案和旧测试；不以超大常量、始终返回 true、自动续期、例外表或新框架绕过旧限制。

### 已实施删除（D01–D10）

- **D01 短期授权与计划TTL**：删除 `PLAN_TTL_SECONDS`、`InstallPlan.expires_at` 及 journal TTL 校验、`apply_at` 的 `plan_expired` 闸门；`authorize`/`validate`/`validate_stored` 删除 TTL/到期/`expires_at`/`ttl_seconds`/`lease_active`，`CurrentState.lease_active` 一并删除。保留并复用 `authorization.json` 的内容/归属记录（EXE 重装映射 `exe_orchestrator::reinstall_evidence` 与 `ReinstallEvidence::from_committed` 仍读取其 `content.mappings`），未另建重复快照。签名catalog发布有效期未动。
- **D02 前台心跳租约**：删除 CLI `LEASE_TTL_MILLIS`/`LeaseState`/`spawn_lease_watchdog`/`renew` 分支与 `no_lease_deadline`/`always_active`；core `TaskLease` 收敛为 `cancelled()`+`cancellation_cause()`，删除 `lease_active`/`deadline_ms`/`write_lease`/`lease.json`/阶段租约复查与 `LeaseExpired` 生产分支；Windows 删除 `RENEW_INTERVAL`/`spawn_renew_forwarder*` 与续期测试；Decky 删除 `RENEW_SECONDS`、`LEASE_SECONDS`、续期写入与 `_watch_lease`/`_expire_lease`。`cache.rs` 的陈旧任务回收改用既有 `owner.lock` 独占锁作存活信号 + 既有 journal 终态证明，删除 `lease.json` 依赖，仍保留无终态证明的任务目录。保留明确 cancel、stdin EOF、子进程收束/排空与 journal 崩溃恢复。
- **D03 查询总预算**：删除 `DISCOVERY_CONTEXT_DEADLINE`（29s）、`DISCOVERY_CONTEXT_BUDGET`（24）、`OFFICIAL_SUBDEADLINE`（6s）、`OFFICIAL_SUBBUDGET_ADDEND`（+3）及 `official_subbudget`、死常量 `FETCH_TIMEOUT`/`DETAIL_REQUEST_LIMIT`/`PAGINATION_SEEN_LIMIT`；`RequestContext` 删除 `budget`/`deadline`/`requests`/`detail_requests` 及预算/截止检查，保留单请求超时、限速、重定向、重试、`Retry-After`、取消与去重。kagura 去除预算 `break`，以真实分页游标+`seen`/`visited` 去重收敛；hgamer 去除详情请求上限。session 去除官方子预算交换。
- **D04 无进展/长退避截止**：复核确认 `stalled`/`retry_wait`/整轮摘要此前已回滚且不存在，未重新引入。
- **D05 解析/转换总时长许可线**：删除 `installer-format` `Decoder.deadline`（硬编码180s）与 `audit_timeout` 检查；删除 worker 按动作的 180/240/480s 墙钟与 300s CPU 预算、Linux `HARD_CEILING`（3600s）与 Windows 300s 阀；保留 probe 启动/无响应短时限、sandbox/内存/地址空间/文件大小边界、取消与真实进程故障处理，并保留累计字节/扩展 `MAX_METADATA`/`MAX_DICTIONARY`/`MAX_MANIFEST_DEPTH` 资源边界与 checked 算术。
- **D06 全事实快照相等与非必需事实失败**：删除 `validate` 的 `facts != current.facts` 全量快照相等与 `authorize`/`exe_report`/`exe_orchestrator` 的 `failures()` 整体否决；保留归属、内容、conditions、candidate、rule、policy 等真实绑定，以及 signed 配方声明的 verified buildid 等明确要求与目标原态复核。手动路线的 buildid/语言/DLC 存在性门槛此前已由 `is_manual()`/`!manual` 收敛。
- **D07 全局设置revision否决**：session 去除按全局 `settings_revision` 的 stale 丢弃与 `discovery.status="stale"`，改为正常 commit；保留 store 短锁/版本号乐观并发（`with_short_lock`/`bump_revision`/`run_if_revision`）与任务 generation 身份。
- **D08 重叠数量许可线与死常量**：删除死常量 `MAX_DISCOVERY_INPUT`/`MAX_MIRRORS_WORKER`/`SOURCE_CACHE_TTL`/`DISCOVERY_SUCCESS_TTL`/`DISCOVERY_FALLBACK_TTL`/`DISCOVERY_INDEX_TTL`/`OBSERVED_TTL`；删除与 `MAX_DISCOVERY_OUTPUT`（512 KiB，真实持久化字节边界）重叠且无独立依据的 `MAX_CANDIDATES`/`MAX_MIRRORS_PERSIST`/`MAX_EVIDENCE` 数量拒绝，以及 `validate_entries` 的 `MAX_ENTRIES` 上限（保留 `is_empty`）。SF 侧 `MAX_ENTRIES=u16::MAX` 为真实字段宽度，按设计§6.2保留并新增注释；保留 `MAX_DISCOVERY_OUTPUT`/`MAX_CACHE_BYTES`/`MAX_DISCOVERY_INDEX_BYTES`/`MAX_STATUS_BYTES`/`MAX_SOURCES`/`MAX_HTML` 并注明保护对象与位置。
- **D09 手动EXE来源名称准入**：删除 `manual_selection.rs`/`exe_resolve.rs` 中按 `kagura-cn`/`kagura-en`/`builtin` 的来源名单门槛；保留当前来源启用/订阅、下载主机校验、隔离解析与实际支持判断（由解析/必要动作决定），自动推荐条件与手动准入分开。
- **D10 下载 MIME 否决**：删除 `acquire.rs` 仅凭 `text/html`/`application/json` 提前返回 `download_not_zip` 的分支；保留 Content-Type 诊断、`copy_verified` 魔数/大小/摘要检查与真实格式解析（真实错误页仍因内容非支持格式失败）。

### 超大常量绕过改为真实删除（设计§6.1）

- 将此前以巨值“删除”的许可线改为真实删除，移除全部 `i64::MAX`/`u64::MAX` 哨兵：`installer-format` `MAX_EXE`/`MAX_FILE`/`MAX_TOTAL`、worker `MAX_EXE`/`MAX_ARTIFACT`（`file_size_bytes` 取消固定上限，依赖真实内存/地址空间/磁盘）、core `MAX_EXE_BYTES`（EXE 无格式大小上限，依赖真实空间检查）、`MAX_PAYLOAD_BYTES`、`MAX_DIGEST_BYTES`（`digest_file` 去掉上限参数，固定缓冲流式到 EOF + checked 算术；`DEFAULT_MAX_BYTES` 取消默认上限）。保留并核对真实边界与 checked 转换。

### 配套清理

- 删除 `map_download_error` 中已无生产者的 `download_size_limit` 映射；更新 `installer/store.rs` 中仍提及 `lease.json` 的注释；删除断言 `exe_facts_failed` 的已忽略测试。
- 旧持久化兼容：`outcome.rs` 仍将历史 `lease_expired`/`installer_lease_expired` 读为 `UserCancelled`，不影响旧事务恢复；不恢复授权/租约生命周期。
- 相关错误码/UI文案/协议生成物：`FailureKind` 去除 `lease_expired`，重新生成 `packages/contracts/generated/*`；UI 去除 `installer_lease_expired`/`plan_expired` 等失效文案；保留签名catalog的 `catalog_expired`。

### 本轮验证

- `cargo test --locked --offline --workspace --no-fail-fast`：37 个测试目标全部 `ok`，0 失败，0 编译错误，0 panic；日志 `.artifacts/install-footprint-d01-d10-tests.log`（core lib 387 通过 0 失败；cli/windows/installer-format/installer-worker/各集成目标均通过）。
- `cargo check --locked --offline --workspace`：通过，无警告。
- 分阶段验证：sources 层 150 通过（D03/D04/D07）；installer:: 146 通过（D01/D02）；installer-format/worker 全绿（D05 与边界删除）；Decky `python -m unittest` 86 通过 2 跳过（D02）。
- 工具环境：本机 cargo 不在 PATH，使用项目本地 `.tools/cargo/bin/cargo.exe` 并设 `CARGO_HOME`/`RUSTUP_HOME`；`--offline` 依赖已就绪。所有读取与命令均在沙盒外执行。

### 待验证与遗留

- 真实大包/大 EXE 端到端：删除 `MAX_EXE`/`MAX_TOTAL`/`DEFAULT_MAX_BYTES` 后，流式审计/摘要/转换的大文件路径仅有合成/黄金夹具覆盖，未用多 GiB 输入实测；`file_size_bytes=None` 后 worker 产物大小仅由地址空间、真实磁盘与取消约束，未做设备验证。
- 容量与无stage：设计 P1/P3 的固定阶段增量预算、分析/转换输入释放、旧事务最小恢复兼容仍为“已有未提交实现待验收”，未在本轮重验；真实峰值/内存/耗时未采样。
- 双平台实机：Windows/Steam Deck 的真实安装、恢复、崩溃扫尾与性能未执行；Decky 后端 Python 测试通过但未做设备运行。
- D08 保留的字节上限仅补充注释依据；512 KiB/4 MiB 等仍未以真实大来源/大索引复现确认，未宣称兼容性验收完成。
- 未提交或推送 Git、未发布、未部署设备。

## 2026-10-18：EXE 无备份直接提取覆盖 D1–D4 实施（摘要）

- 子功能按[设计](install-transaction-performance/design.md)完成重写：解析 EXE 元数据后，同一 staging 会话直接流式提取所选成员到游戏目标；不再生成中间 ZIP/stage、不备份、不写逐文件数据库状态，也不做成功前全量复读。
- 自动化：`cargo check --workspace --all-targets`、`cargo test --workspace`、contract-gen check、`npm run typecheck`、`npm run test:contracts`、Decky 70 项测试通过。Linux release 已在 Deck 的 podman 容器中实际编译并运行。
- 实机：2026-10-18 在 Steam Deck 部署新插件（Loader 本地 ZIP，实例 1），用 AppID 1011940“冲突的意志”执行 overwrite：任务总 530.3 秒（下载约 428 秒、分析约 51 秒、提取+提交约 51 秒），事务 apply 49 秒完成 12,141 文件 / 3.794 GB；无 backup/stage/payload，下载缓存清理 3.82 GB。详见[子功能施工记录](install-transaction-performance/progress.md)。
- 未验证：Windows 实机、强杀中断与重试、旧事务恢复实机回归、游戏内生效与手柄/全屏视觉验收。

## 2026-10-16：P1/P3/P4 未完成项收敛

在设计授权的 P0/D01–D10 之后，继续收敛 P1（容量/材料释放）、P3（无stage 兼容）、P4（大文件/ZIP64）三个此前“已有未提交实现待验收”的方向中可离线完成的具体缺口。

### P1 容量模型

- 删除仅测试使用的旧 `SpaceBudget::new`（旧 `D+2P+O` 形态），避免两套并行生产公式；测试迁移到 `for_conversion`/`ZipSpaceBudget`。
- EXE `for_conversion` 同卷峰值改为设计§5.2 的 `max(2D+Z, D+Z+O+T)`：补回安装阶段仍保留的缓存下载 `D`；conversion 项为沙箱输入副本共存的 `2D+Z`（checked_add）。
- `Z` 作为**预测**的 STORED 打包开销（命名常量 `EXE_NORMALIZED_OVERHEAD_BYTES`，非 P、非余量）并注明依据；`state_bytes` 取 `Z + D + max(D,O) + M`（缓存D+沙箱D+Z 与 缓存D+Z+备份O 的峰值；备份位于 `jobs/<plan>/*.backup` 状态卷）。
- 下载后 EXE 门槛由未说明的固定 32 MiB 改为文档化的单一边距 `SPACE_MARGIN_BYTES`，保留真实空间检查，未恢复 2 GiB 门槛。
- 真实磁盘峰值、内存与耗时仍未实测；`Z` 无更紧的上界（`for_conversion` 只有成员大小）。

### P3 无stage 缺口

- §4.3：外部本地 ZIP 不再持久化任意绝对路径。新增 `install::adopt_external_plan_archive_at`，在授权校验后、apply 前把调用方提供的本地 ZIP 复制一次到受管理的 `jobs/<plan_id>/payload.zip`，复核精确 size 与完整 SHA-256 后写入 journal（`managed=true`）；损坏/缺失返回 `archive_source_changed`/`archive_source_missing`，不留部分副本，不改用户原文件；下载缓存来源不复制。
- §4.6：`open_payload_archive` 在记录路径与受管理回退路径都不可用时返回命名 `archive_source_missing:<path>`，与 `archive_source_changed` 区分，不再抛裸 `NotFound`。
- `ArchiveSource.managed` 由死字段改为参与缓存保留判定（受管理计划归档随自身 journal 释放，非受管理的共享下载缓存才按缓存规则保留）。
- §4.2：成员名作为稳定定位并注明理由（prepare 已做重名冲突检查、名称与成员 1:1），未新增持久化索引；未做破坏性 journal schema 变更。
- 新增 4 个测试：外部ZIP受管理化并在删除原文件后 apply/commit、删除原文件后的受管理副本前缀恢复、缺失受管理归档的命名诊断、篡改后拒绝。

### P4 ZIP64/大文件

- `installer-worker/src/sf.rs`：`crc32` 改 `u32::try_from`、`payload_offset` 改 `usize::try_from`（checked，溢出报错，不截断）。
- `installer/core/exe_report.rs` `payload_bytes` 改 checked 求和（溢出报错，不回绕）；两处调用点同步传播错误。
- `zip_metadata.rs`：为保留的 `MAX_METADATA_BYTES`（64 MiB）与 `RECORD_ESTIMATE`（1024）补齐保护对象、数值依据（字节预算而非数量线）、检查位置（`exceeds_budget`←`preflight`/`inspect`）与超限行为（显式 `zip_metadata_limit`，不静默截断）注释。
- fixture：新增 `size-and-offset-extras-beyond-32bit`（size/offset 均 >2^32 且压缩大小非零）与 writer 侧 `count>=65535` 触发 ZIP64 EOCD 的用例；同步更新生成脚本 `scripts/freeze_sf_zip64_fixture.py`，重新生成与 fixture **逐字节一致**（10 个用例）。
- 真实 >4 GiB 端到端（磁盘上成员确实位于 4 GiB 之后）**未做**：可信复现需要分配 >4 GiB 磁盘，覆盖仍停留在 writer 组帧层，标记待验证。

### 配套清理与验证

- 删除 UI 中已无 Rust 生产者的 `exe_original_size_limit` 文案。
- `cargo test --locked --offline --workspace --no-fail-fast`：37 个目标全 `ok`，0 失败 / 0 编译错误 / 0 panic；日志 `.artifacts/install-footprint-p1-p4-tests.log`。
- `cargo check --locked --offline --workspace`：通过。
- 前端/契约：`node --experimental-strip-types --test apps/decky/tests/*.test.ts` 70 通过 0 失败；`npm run typecheck`（Windows + Decky `tsc --noEmit`）通过；`npm run test:contracts` 通过。
- 仍未执行：真实大包/大 EXE 端到端、真实磁盘峰值采样、P4 的整包 mmap 与 3 GiB 地址空间窗口映射（设计留待平台内存语义实测）。未再提交或推送 Git。

## 2026-10-16：Steam Deck 实机部署与旧 journal 兼容修复

- 按 [decky-deploy-verify](../../../.agents/skills/decky-deploy-verify/SKILL.md) 流程部署完整插件到 `deck@192.168.31.186`（SteamOS holo）。设备基线：`plugin_loader` active、插件目录 `root:root 755`、`list_tasks` 25 条全终态；本轮 host key 记录于 `.artifacts/install-footprint-deck-20261016-031616/known_hosts`。
- 构建：前端 `npm run build:decky`；Linux CLI/worker 用仓库内 WSL 工具链 `scripts/build-linux.sh` 重建（18 个 Linux 测试通过），产物 `target/linux/release/{steam-patch,sf-worker}`；打包 `steam-patch-manager-decky.zip`（首版 `06c0a1bc…`，修复版 `1cb0564c…`）。
- 部署方式：Loader 本地 ZIP 安装（`utilities/install_plugin` + `utilities/confirm_plugin_install`），正式目录 24 个文件哈希与清单一致，`bin/*` 执行位保留。
- **实机发现回归**：新核心读取设备上已有的 `installer/rust-v1/journal.sqlite`（16 条含 `plan.expires_at`）与旧 `authorization.json`（含 `expires_at`/`lease_active`）时报 `journal_invalid`，插件 `main.py::_initialize` 的 `bridge.recover()` 启动失败。
- **修复（设计“旧持久化字段仅作最小兼容读取”）**：`InstallPlan` 加回只读 `expires_at`，`Authorization` 加回只读 `expires_at`/`lease_active`，均 `#[serde(default, skip_serializing)]`，不恢复 TTL/心跳逻辑；补齐三处构造点；新增兼容回归测试 `legacy_plan_document_with_expiry_loads_read_only`、`legacy_expiry_and_lease_fields_load_read_only`。
- 修复后重新部署：`recover()` 正常，日志无 `journal_invalid`/Traceback；`list_tasks` 25 条全终态、`scan(false)` 返回 28 条真实实例（诊断仅为未挂载 SD 卡的 `library_unavailable`）；插件实例 1，`plugin_loader` active。
- 验证：Windows `cargo test -p steam-patch-core` 394 通过 0 失败；Linux `build-linux.sh` 测试 18 通过；Decky TS 70 通过、`typecheck`、`test:contracts` 通过。
- 待验证：真实屏幕（QAM/全屏）截图未取到（CDP 对 `QuickAccess_uid2` 截图超时），仅完成文件/RPC/实例验证，故界面视觉与实体手柄交互标为待验证；真实游戏补丁安装/卸载不在本轮范围。
- 远端暂存目录已按脚本核验后清理，仅保留本轮 `steam-patch-validation/<round>/`；本地保留备份与证据于 `.artifacts/install-footprint-deck-20261016-031616/`。

## 2026-10-16：大包 EXE 实机验证与报告/ZIP64 修复

- 验证对象：AppID 1011940「冲突的意志 / Ideology in Friction」的 kagura-cn EXE 补丁，包 **3,788,037,984 B（3.53 GiB）**、内含 **12,141** 个文件。
- 逐层实测暴露并修复三处真实缺陷（均为既有实现中的无依据上限或错误，按“精简设计/防御性逻辑”直接删除或修正，不放宽阈值）：
  1. **报告体积上限**：worker `installer-format::audit` 的 `report_size_limit`（报告 < 16 MiB）直接删除；连带删除 worker 响应编码 `LimitedVec` 的 `MAX_MESSAGE` 上限、core `exe_report::MAX_REPORT_BYTES` 的读取/写入/校验上限，以及冗余的 `bounded_json` 递归预校验（serde_json 已限深，字段类型由后续读取校验）。
  2. **沙箱 stdout 捕获上限**：Linux/Windows 启动器的 16 MiB stdout 上限移除（仅保留 stderr 64 KiB 上限）；报告随合法文件表增长，由 worker 沙箱内存上限与真实文档大小约束。
  3. **ZIP64 读取错误**：CPython（及本写入器，字节冻结对齐 CPython 3.12）仅在值溢出 32 位时饱和经典 EOCD 字段，而中心目录偏移落在 `[2 GiB, 4 GiB)` 时经典偏移保持原值却仍写 ZIP64 EOCD。读取器原先仅按 `0xFFFF/0xFFFFFFFF` 判定 ZIP64，导致这类合法 ZIP64 包被误判为 `zip_metadata_invalid`。改为同时依据经典 EOCD 前的 ZIP64 locator 判定；新增回归测试 `accepts_zip64_with_unsaturated_classic_fields`。
- 顺带删除死常量 `MAX_BASELINE_BYTES`（512 MiB，无调用点）。
- 实机结果：完整安装**成功** — `state=committed`、`disposition=completed`、`error=null`，提交后清理 ~3.82 GB；游戏目录 **1.5 GB → 4.2 GB**，无 `.backup`/`.tmp` 残留，`downloads/`、`runtime/` 均已清空。总耗时 ~1354s（下载 ~350s、分析 ~60–120s、转换 ~120s、校验 ~200s、写盘 ~800s）。
- 验证：Windows `cargo test --workspace` 37 目标全绿；Linux `build-linux.sh` 测试 18 通过。
- 待验证：游戏实际启动/运行未执行；本次 3.53 GiB 包在整包 mmap + `size+3 GiB` 地址空间下通过，但更大的单成员/偏移（F02 窗口映射）仍未覆盖；真实磁盘峰值未采样。

## 2026-10-17：安装事务性能诊断（冲突的意志）

- 用户报告同一游戏（AppID 1011940）安装体感 20~30 分钟、下载占比小。诊断对象为设备上 2026-10-17 02:43–03:15 的一次成功安装（任务 `8d0c735d` / 计划 `196549ee`，12,141 个文件替换，payload 3.80 GB）。
- 时间线还原：下载+分析+转换 ~12 分钟；apply 事务 1153s（19.2 分钟）= 校验+全量备份 ~265s + 写盘 815s（67ms/文件）+ 提交/清理 ~70s。与 2026-10-16 记录的 ~1354s 分解一致（写盘两轮均稳定在 ~800s）。
- 热点（按代价排序）：写循环内逐文件 `steam::scan` 全库扫描（`install.rs:2027`→`:843`）；每文件 2 次 `synchronous=FULL` SQLite 提交 + 4~5 次 fsync；同一文件 3~5 次全量 SHA-256；另有 apply 前全量备份与双次全量校验。Deck 微基准与设备 mtime 证据见报告。
- 本轮为只读诊断：未修改任何代码、未部署、未提交 Git。完整报告：[perf-analysis-20261017.md](perf-analysis-20261017.md)；证据与脚本归档 `.artifacts/install-perf-diagnosis-20261017/`。修复方向仅列为候选，待另行设计评审后进入施工。

## 2026-10-17：性能报告复核与子功能规划

- 新建[安装事务性能优化设计](install-transaction-performance/design.md)及[施工记录](install-transaction-performance/progress.md)，状态为待实现。先定向校验目标 manifest，再合并权限设置与文件同步；保留逐文件持久化与恢复语义。
- 报告中的二次同步、哈希重复及收益估计需要按执行路径区分，详见子功能设计 §2。本轮未找到历史取证目录，设备时长仅引用原报告；未修改生产代码或执行设备操作。
- 后续按用户要求排除安装期间 Steam 更新或外部修改场景，子功能 P1 已改为删除循环内重复实例校验，撤回定向 manifest 接口方案；以子功能当前设计为准。

- 再次按用户授权连接 Steam Deck，重新取得历史安装记录；子功能已将正常路径重复哈希删除/合并纳入 P3，详见[取证记录](install-transaction-performance/evidence-20261017.md)与[当前设计](install-transaction-performance/design.md)。优化仍待实施。

- 全热点复核后，子功能扩展为 P1–P6：同时处理扫描、文件/目录同步、哈希、journal 提交和清理；备份数据、串行及路径检查的保留理由已逐项说明。见[当前设计](install-transaction-performance/design.md)，仍待实现。

- 用户随后决定取消 EXE 中间 ZIP，按五步直接提取安装。子功能设计已整体重写为 D1–D5，替代先前 P1–P6；涉及的 EXE 空间与恢复约定以[新设计](install-transaction-performance/design.md)为准，尚未实施。

- 用户进一步取消备份：子功能现为无备份直接覆盖，失败/取消保留部分结果、不回滚。设计、空间模型、双端状态及重试验收已同步修订；生产代码与现有备份未改动。

## 2026-10-18：ZIP 独立精简计划

- 按用户要求新增 [ZIP 无备份直接安装设计](zip-direct-installation/design.md)与[施工记录](zip-direct-installation/progress.md)，独立于 EXE 子功能。覆盖普通来源、签名目录及本地 ZIP，复用现有 direct 接口，状态为待实现。
- 本轮只规划和检查文档链接，未改生产代码、部署或提交。

## 2026-10-18：ZIP 直接安装 Z1–Z4 实施

- 按 [ZIP 子功能施工记录](zip-direct-installation/progress.md) 完成普通、签名与本地 ZIP 的直接覆盖安装；新事务不再创建 backup/stage/authorization，旧备份事务仍按 `recover_checked_at` 恢复。
- 验证：`cargo test --offline --workspace` 全部通过；`cargo clippy ... -D warnings`、`cargo fmt --all --check`、contract-gen `check` 通过。Windows/Steam Deck 实机、取消/中断与同包性能测量待执行。
