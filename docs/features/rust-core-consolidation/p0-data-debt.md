# P0 数据保全基线与技术债关闭映射

设计：[design.md](design.md)。施工记录：[progress.md](progress.md)。历史基线：[迁移记录](../cross-platform-installer-migration/progress.md)、[P4 Windows 验收](../cross-platform-installer-migration/p4-acceptance.md)、[P5 Decky 验收](../cross-platform-installer-migration/p5-decky.md)。

日期：2026-10-11（Asia/Shanghai）。状态：**P0 静态基线与债项映射；无生产代码修改、数据迁移、测试、部署或实机操作。下列实机样本是既有迁移证据，不计为本 feature 新验证。**

## 1. 数据保全不变量

核心提取沿用宿主传入的唯一 `state_dir`。不得另设 core 状态根、复制数据库/JSON、启动第二套事务日志或信任库，也不得把宿主任务历史并入 installer schema。Windows 与 Decky 各自使用本机根；不共享或互相复制数据。

### 路径和责任

| 数据 | Decky | Windows | 所有者与要求 |
| --- | --- | --- | --- |
| 安装器 | `/home/<user>/.local/share/steam-patch-manager/installer/rust-v1/` | `%APPDATA%\io.github.steampatch.manager\installer\rust-v1\` | core；保留所有状态和恢复材料。Windows 根来自 Tauri `app_data_dir()`，identifier `io.github.steampatch.manager`。 |
| 来源/订阅 | `<state>/sources/rust-v1/` | `<app_data_dir>/sources/rust-v1/` | core SourceStore；保留 settings/status/revision。受 query/encoding 版本影响的缓存可按既有迁移契约失效重查，不重置用户设置。 |
| 签名 revision | `<state>/trust.sqlite` | `<app_data_dir>/trust.sqlite` | 当前 CLI；保留 `catalog_revision`。迁入 TrustRevisionStore 仍使用原文件/表。公钥和 signed catalog 是只读宿主配置，不在此 DB。 |
| 自动清理偏好 | `<state>/settings.json` | 当前无设置，固定传 Disabled | Decky backend Settings，schema_version=4、`auto_cleanup: bool`，默认 true。继续由宿主读取并显式传策略。 |
| 任务历史 | `<state>/host-v3/tasks.json` | 当前 Tauri 进程内任务/事件状态，无持久历史文件 | Decky TaskManager，schema_version=1，至多 64 条；重启把非终态历史标为 `failed/task_interrupted`。这不代表安装 journal 已恢复。 |
| 宿主状态目录 | `<state>/host-v3/` | Tauri app data 根、固定 packaged binary | Decky `state_host` 固定路径、链接、owner/权限检查；Tauri 固定 binary、进程、renew/cancel/EOF、事件派发属于合法宿主职责，不列删除债项。 |

Decky `main.py` 固定根为 `/home/<user>/.local/share/steam-patch-manager`，`prepare_host_state()` 只准备根与 `host-v3`，不递归修复 ownership。Windows `common_params()` 通过 `app.path().app_data_dir()` 注入 state 根。Windows 没有 Decky 式文件历史；不得为对称而新增第二套持久任务根。

### 安装器和数据库结构

`InstallerPaths::new(state_dir)` 唯一安装根是 `state_dir/installer/rust-v1/`：

| 路径/schema | 现有含义 | 保全边界 |
| --- | --- | --- |
| `schema.json` | schema v1 完成标记；原子初始化最后写入 | 标记缺失不等同目录可删。 |
| `journal.lock` | 跨进程安装互斥锁 | 不删除锁绕过 busy/恢复。 |
| `journal.sqlite` | SQLite `journal_mode=DELETE`、`synchronous=FULL`、FK 开启。`jobs(id TEXT PRIMARY KEY, document TEXT NOT NULL)` 是事务/计划 JSON 真相源；`job_progress(job_id,item,state,updated_at)` 与 `job_directories(job_id,item,path)` 是增量恢复 overlay，FK cascade。旧 job 可用 document 内嵌进度。 | 一致备份 DB 与可能存在的 journal 文件；不改写历史 transaction/document。 |
| `requests.sqlite` | `requests` 表：request_id PK、instance_id、selection_id/release_id、operation、content_id、transaction_id、plan_id、state/error、created_at/updated_at；DELETE/FULL/FK。 | 幂等与重放账本，不重置，也不以宿主 task_id 取代 request_id。 |
| `jobs/<plan_id>/` | 授权、lease/revoked、计划、`.stage`/`.backup` 等恢复材料 | 受保护；不能当缓存清理。 |
| `receipts/<instance_hash>/` | `init_atomic` 创建的保留目录；当前源码未发现向此目录写独立 receipt JSON。instance_hash helper 为 instance_id SHA-256 前 16 位小写 hex。 | 目录若存在保留；当前 committed 成功事实存在 journal Job/Transaction，不得误说已有独立 JSON receipt。 |
| `content/`, `downloads/`, `reports/` | 唯一允许 CacheService 清理的缓存/报告根 | 由统一服务限路径、链接、活动任务检查后清理。 |
| `tasks/<uuid>/` | 临时 task/lease/revoked/owner.lock；终态回收、崩溃后检查锁的 stale sweep | 不属于通用清理根；不能删仍持锁任务。 |
| `preexisting-installations.json`（如存在） | 当前 CLI scan/install 不读取；有效格式与坏格式都不影响 scan 或三种 install route（`legacy_marker_does_not_block_scan_or_install_routes`）。 | 若文件存在，备份时原样保留，避免丢弃未知用户资料；不得把它称为当前有效保护语义，也不得以它替代 journal/source/trust 基线。 |

journal 三表在 `crates/core/src/install.rs::Store::open_at` 初始化；请求表在 `crates/core/src/installer/requests.rs::ensure_requests_db` 建立。`sources/rust-v1/` 有 `schema.json`、`settings.json`（schema/revision/enabled/added/games）、`settings.lock`、`cache/<source>.json`、`status/<source>.json`、`discovery/<source>/<appid>.json`、`discovery-index/<source>.json`。SourceStore 在 marker 缺失时仍复用完整 settings（已有 F7 修复），原子写入并保留 settings revision。

`trust.sqlite` 当前只有 `catalog_revision(id INTEGER PRIMARY KEY, revision INTEGER NOT NULL)`；CLI 读取 id=1 的 revision，写事务再查全表最大 revision 并对 id=1 upsert，连接设置 FULL。验签、过期/撤销和 release/platform/build/support 准入在 CLI `signed`/`ready_release`，验签类型在 core `trust.rs`。新 store 必须保留现有 Immediate 写事务内二次 revision 检查，避免提取时退化成 SELECT 后写入竞态。持久化 JSON 版本核对：journal 的 `jobs.document` 是 `{plan, transaction, progress, created_dirs, uninstall_requested, transient_recovery, progress_format}`；`InstallPlan`/`Transaction`/`Job` 当前都没有 `schema_version` 字段，`progress_format` 是 overlay 格式标记（缺省 0），不是整体 schema 版本。`tasks/<uuid>/task.json` 有 `schema_version: 1`。当前代码未找到单独写入 `receipts/<instance_hash>/` 的 receipt JSON；成功事实仍在 journal committed Job/Transaction 中，因此没有独立 receipt JSON schema 版本。`jobs/<plan_id>/authorization.json` 是另一种受保护授权记录，其 `schema_version: 1`、`installer_schema_version: 1`，不能误当安装 receipt。

### 未完事务识别与升级/回滚步骤

journal job document/overlay 是未完成事务唯一权威来源。`recover_at` 会处理写入/回滚中断及卸载意图；失败持久化为 `needs_recovery` 并保留冲突材料。`planned`、`rolled_back`、`uninstalled`、`superseded` 和无卸载意图的 `committed` 被跳过。Decky task history 重启改为 `task_interrupted` 只是 UI 历史状态，不可以替代 journal recovery。

1. 停止接收新安装任务；备份匹配的宿主/CLI/worker、state 根清单和 SHA-256。SQLite 备份须一致包含主 DB 与可能存在的 journal 文件。
2. 用当前匹配版本只读 `list_transactions`，执行既有 recover；逐条确认活动事务回到终态。若仍有 `needs_recovery`，保留现场及 stage/backup，诊断完毕前不切换。
3. 不删除或重建 `installer/rust-v1`、`sources/rust-v1`；不允许新旧引擎并发写同一 state/游戏。匹配宿主、CLI、worker、UI 和 v4 契约作为一组切换。握手/字段校验失败时禁止写入。
4. 切换后核对 state 根、schema/表读取、source settings/revision、trust revision、task history、receipt/transaction 以及活动事务为空；保留回滚备份。
5. 回滚用匹配旧产物；新版本若产生事务，先用新版本恢复并停任务，核实 state 后再切换。禁止旧备份覆盖升级后更新的 journal/receipt/history；发现不明状态则停止写入。

Decky state、Windows state 彼此独立，不应由切换步骤互相同步。

## 2. 现存真实样本与取消证据

这些是迁移记录和仓库 artifacts 支持的历史事实。此轮未连接设备，不能证明现场仍可复测。新架构的受影响链路需用匹配产物重验；历史结果不得登记为本 feature 新验证。

| 样本 | 现存记录 | 可复用边界 |
| --- | --- | --- |
| 2743810 ZIP | P4 Windows 真实下载/安装/覆盖及用户确认游戏内生效；P4 acceptance 记录 `real-cached-scan` 和 history，仓库有对应 `.artifacts/p4-acceptance/run-20261009/` 及 `.artifacts/generic-exe-local/2743810-*`。 | 历史基线。投影/身份可复测；需重新确认输入可读性，历史实机不算新架构验收。 |
| 2303800 EXE | P4 Windows GUI 真实下载、隔离分析、转换、安装、游戏内确认；`.artifacts/p4-windows/windows-real-sf.json` 和 `.artifacts/generic-exe-local/2303800-*` 是已有材料。 | worker 格式结果可作旧黄金基线；核心调用链须新验证。 |
| 3137110 大 ZIP | 2026-10-11 Deck 修复记录 6.78 GB 获取/安装、9 个目标摘要匹配、非目标原文件不变及复扫 installed_managed；记录在迁移 progress“续二”及 `.artifacts/decky-install-fix-20261010/` / `.artifacts/installer-fix-20261011/`。 | 现存的是历史现场结果；未确认下载包当前仍可重跑。可引用大包基线，不能宣称本 feature 实机验证。 |
| Windows 计划前取消 | 迁移 progress 记载 2303800 取消后无新事务、游戏文件不变；是旧迁移验收。 | 可作下载/计划前取消基线，不能代替写入回滚。 |
| Deck 写入期取消 | 迁移 progress“续二”及 `.artifacts/p5-decky/night-write-cancel-*`、`.artifacts/installer-fix-20261011/cancel-poll-*` 记录真实写入期取消、任务 cancelled、事务 rolled_back、文件恢复、存档完好。 | 历史实机基线；P2 commit-wins/rollback 和 P6 仍须按新匹配版本验证。 |

当前 Windows Steam 根只读核对已确认：注册表根 `D:/Program Files (x86)/Steam`，其中 libraryfolders.vdf 列出 `F:/SteamLibrary`，目标 AppID 2743810、2303800、3137110 均在库中。主 Agent已通过无 state/catalog 的基准工具确认真实36实例清单，三目标均installed：2303800 build10821988、2743810 build23866175、3137110 build16717597，详见 [P0基线](p0-baseline.md)。该清单不证明可复用补丁缓存或可写入现场。Deck 设备/游戏样本仍是历史迁移证据。仓库中未确认 3137110 原始 ZIP 当前可重跑；现有材料支持的是历史安装/摘要事实。

## 3. D1–D10 映射

当前均为**待实现**。实现 owner 为对应阶段执行者；P6 的删除审查由独立复核者确认。表内测试和产物是现有候选/历史定位，不表示本轮已运行。

| ID | 当前定义/消费者 → 目标所有者 | 测试与产物基线 | 负责人阶段；清理完成证据 |
| --- | --- | --- | --- |
| D1 扫描/投影/检测/预览 | CLI `runtime.rs::{dynamic_patches,dynamic_detection,preview_route,committed_changes}` 与结果拼装 → core projection；CLI 编解码；Decky `_project_row/_refresh_rows`、Windows scan 消费报告。 | CLI `scan_projects_*`、`preview_selection_is_read_only...`；core catalog/steam_scan tests；P4 cached scan 只作旧样本。 | P1 实现，P6 删除。旧私有函数/拼装无生产消费者；scan/preview 生产链只调用 core；固定投影 fixture 与双端消费证据。 |
| D2 blocker、release identity、hash 重复 | CLI dynamic blocker、`dynamic_release_id`、`hash_target` 与 core `resolve::source_release_id`/blocker 和 `catalog::hash_file` 重复；消费者为 CLI scan 与 resolver。 | `resolve.rs` stale/subscription tests、core catalog path/hash tests、CLI dynamic selection/signed priority tests；补同输入一致与 linked/boundary/file-change 负例。 | P1，P6 删除。CLI blocker/id/hash 副本无生产消费者；共享规则/取证各一个实现并有一致性测试。 |
| D3 signed/trust | CLI `highest_revision`/`remember_revision`/`signed`/`ready_release`/`install_signed_route(_inner)` → core signed service 与 TrustRevisionStore，CLI 提供环境、传输、公钥。 | CLI `install_flow_tests` signed/幂等/失败清理；`core/tests/trust.rs`；core orchestrator rollback/late-cancel/revision tests。补直接 API 防回滚竞态。 | P2，P6 删除。CLI signed 业务和 trust DDL 消失；同一 `trust.sqlite/catalog_revision` 被 core 使用，事务内 revision 重检有证据。 |
| D4 cache/cleanup | CLI `cleanup_cache`/`directory_size`、Decky `main.py` committed 后二次删除/合并结果 → core 唯一 CacheService+CleanupPolicy。宿主仍保存偏好并传策略。 | CLI `cleanup_cache_sweeps_cleanable_roots_and_protects_recovery_material`、Decky `test_cache_cleanup.py`、core store/task tests；补活动安装互斥、链接、部分失败不改 commit。 | P3，P6 删除。旧遍历/删除和 Decky 追加删除无消费者；jobs/receipts/requests/tasks 保护和互斥测试。 |
| D5 安装动作派生 | `packages/ui::patchInstallable/canAutoInstall`、Windows `projectScan`、Decky `projectPatch/available/installed` 及 `auto_installable/reinstall_available` → core 必需 PatchActions；UI 管 busy/选中/文案。 | Decky `tests/install.test.ts`、Windows `scripts/test-game-status.mjs`、CLI scan/action fixtures 迁移至 core truth table；涵盖 signed/ZIP/EXE links、来源/事务/detection、manual input、overwrite。 | P4，P6 删除。旧字段/helper 无生产读写，双端直接读同一 action DTO，core action truth table 通过。 |
| D6 重复 wire/ID/detail | CLI main/runtime、Tauri、Decky bridge 手写 v3 DTO、方法/ID/detail/互斥校验 → `crates/protocol` v4 + contracts 生成 schema/TS + Python fixture。 | CLI protocol tests、Tauri `validate_response/build_install_request`、Decky `test_v3_bridge.py`；补生成无漂移、UTF-8 byte bound、v3 拒绝、未知字段和乱序/重复终态。 | P5，P6 删除。契约唯一来源、生成无 diff、Rust/TS/Python fixtures 一致，生产包无 v3 fallback。 |
| D7 事务到任务终态重复映射 | Decky `TaskManager._run` 按 committed/rolled_back/error 映射；CLI 按 state/error 组装；宿主记录任务/传输错 → core InstallerOutcome.disposition，宿主只保存历史与生命周期。 | Decky `test_tasks.py`、CLI `outcome_reports_needs_recovery_explicitly`、core late commit/write rollback tests、Windows task stream tests；共享 commit/cancel/other rollback/needs_recovery/transport/cleanup fixture。 | P2 定义、P5 DTO、P6 删除。Python/CLI 不按字符串推状态；两宿主 outcome fixture 一致，journal 原始事务与 cleanup 结果独立保留。 |
| D8 共享 URL/state/progress helper | Decky `src/discovery.ts`、Windows `src/sources.ts` URL guard；双端 source predicate/progress formatting → shared TS UI helper。下载网络许可仍独立验证。 | Decky discovery/install/task-progress tests、Windows source/npm status tests；共同 fixture 覆盖危险/正常 scheme、错误理由、格式边界。 | P5，P6 删除。重复实现删除并双端 import shared helper；不能以浏览器 guard 代替下载安全。 |
| D9 临时入口/fallback/旧夹具 | 阶段 wrapper、过渡路由/兼容桥需逐项登记；有效 state 读取与宿主权限/进程职责不属于待删债；`preexisting-installations.json` 当前不参与生产行为，若存在仅为避免丢弃未知资料而保全。 | 搜索 legacy/fallback、旧方法名/v3 DTO、迁移入口与测试 fixture；审阅最终源代码和 package allowlist。 | 临时项仅由 P6 owner 删除。每项有删除证据；正式包只有 v4；installer/source/trust/settings/history/recovery 仍可读，无第二状态根或旧 core fallback。 |
| D10 v4 smoke/CI/文档/产物 | CLI 当前 JSONL v3、CI、contracts README、指南/索引/SBOM → protocol v4 fixture-backed subprocess smoke、生成漂移 CI 与同步文档/最终包。 | CLI scan→preview/stream tests、Decky `test_v3_bridge.py`、Tauri RPC tests 是旧起点；P4/P5 artifacts 仅历史产物。 | P5 建链、P6 验收。可复跑 v4 smoke/CI；双端包中 CLI/worker/host/UI 匹配；索引、指南、SBOM 一致；独立复核无差异。 |

P6 最终删除审查应基于生产源码和实际包清单，不能只用文本零匹配。合法宿主进程/权限/固定 binary/RPC 生命周期、Decky 设置和任务历史、来源 settings/status、事务恢复及其他有效用户数据读取必须保留；`preexisting-installations.json` 即使存在也只按未知资料备份保全，不代表当前读取语义。关闭证据至少包括每项旧消费者扫描、测试报告、双端产物文件清单/哈希、受保护恢复目录复核。

## 4. 静态自检范围与限制

阅读了 AGENTS、design/progress、worker/result protocol、迁移审阅/验收文档和相关 core/CLI/Decky/Windows 源码。路径/schema依据 `installer/store.rs`、`requests.rs`、`install.rs`、`sources/store.rs`、`runtime.rs`、`trust.rs`、Decky `state_host.py/plugin_settings.py/tasks.py/main.py` 和 Tauri `common_params()`/config。样本依据迁移 progress/P4/P5 记录及已有 artifacts 清单，没有重放或连接设备。

本次完成了 Markdown 本地链接静态检查（5/5 目标存在）；未执行测试、编译、CLI smoke、性能基准或部署。该文档完成的是静态数据与债项映射；P0 其余 API/DTO、actions/终态真值表与性能基线已由主任务整合并经独立复核冻结，详见 [施工记录](progress.md)。
