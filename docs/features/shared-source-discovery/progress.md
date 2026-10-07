# 来源发现与订阅迁入共享 Rust 后端施工记录

设计：[design.md](design.md)。P0/P1 实现规范：[p0-p1-rust-design.md](p0-p1-rust-design.md)。P0 实机基线：[p0-baseline.md](p0-baseline.md)。状态：**P0–P3 已实现并自动化验证（待 P4 起实机验收）**。日期：2026-10-07（Asia/Shanghai）。

## 当前状态

| 工作 | 状态 | 说明 |
| --- | --- | --- |
| 两端来源能力差异定位 | 已完成只读核对 | Windows只用签名目录；Decky另有Python SourceRegistry，名称显示无法补齐动态查源 |
| 迁移设计与功能归档 | 已完成设计与文档复核 | 本目录、设计/施工互链、索引；链接检查和独立审阅已执行，迁移代码待实现 |
| P0 基线fixture及最终契约冻结 | 已完成 | 契约/golden 冻结；2303800 实机结果已核对并并入 golden；页面证据存忽略目录。见 [p0-baseline.md](p0-baseline.md) |
| P1–P3 Rust配置/cache/适配器/CLI会话 | 已完成（自动化验证） | P1 配置/存储 + P2 HTTP/identity/五适配器 + P3 会话/CLI 全部实现；core lib 126 + CLI 14 测试、clippy、改动范围 fmt、两端 typecheck+build 通过 |
| P4 Windows接入与验收 | 待实现、待验证 | 先Windows；既有中文展示验收不替代动态来源验收 |
| P5 Decky切换与消费者兼容 | 待实现、待验证 | 保留旧设置、receipt及publisher/EXE接口 |
| P6 双端收口 | 待实现、待验证 | Python来源逻辑退场及完整证据尚未执行 |

## 2026-10-07 计划制定

### 用户目标与本轮范围

用户要求把Decky专属的来源订阅、联网发现、缓存和身份核对迁入Rust，并作为新feature制定计划。本轮只读盘点、设计与文档检查，不实施功能、不部署或连接设备。

### 实际完成的调查

- 读取仓库AGENTS、来源指南、核心CLI/serve-task、Windows宿主、共享DTO、Decky scan/discover及旧安装消费者；以当前源码和已部署Windows结果确认边界。
- 复用前次Windows验收证据：AppID2303800保留英文name、中文localized_name，签名目录revision4中没有该AppID，patches=0、无discovery。记录位于[名称施工记录](../localized-game-names/progress.md)，不是新的源迁移验收。
- Luna只读盘点来源配置schema1/2、overrides enabled/added/games、cache层级、TTL、成功/attempt分离、检查点、查询key、并发过期和测试入口。默认shell最初无输出，切换明确的本机PowerShell只读通道后完成；未将最初失败算成源码验证。
- 当前自动发现仍主要使用manifest name和可选name_hints，未消费新增localized_name；计划将核心覆盖名纳入查询输入并使旧负缓存正确失效。
- 确认publisher/exe消费者直接依赖sources.matches、cached_candidates、_rules/builtins、game_subscribed和validate_download；计划包含薄RPC适配与消费者回归，不能仅删Python文件。

### 方案决策

- 单一Rust sources服务/默认规则；协议2增量方法与能力协商，来源事件与安装进度分离。
- 单独sources/rust-v1状态空间，先导入设置，旧cache重新校验后只作过期seed；旧Python文件留存，单引擎运行，回退保护后续用户选择。
- 保留现有静态catalog和安装层授权；Windows先实现来源发现/展示，Decky再切换。Windows动态ZIP/EXE自动安装迁移是后续独立feature。
- 以P0–P6门槛推进；真实设备、网站和安装能力分别记录验证，计划阶段不声称完成。

### 本轮验证

- 相对链接、Markdown结构和状态一致性：最终36个新增/关联相对链接通过，代码块闭合，索引与设计/施工均保持待实现；两项审阅发现修订后由主Agent核对。git diff --check通过，仅有既有换行提示。
- 独立设计审阅：Sol完成只读审阅，发现回退后重入设置与默认扫描自动发现两项P2契约缺口；主Agent已补齐双向checkpoint/三方合并、冲突处理及默认scan→scan_sources/force/取消行为，并加入验收场景。
- Rust/Python/前端测试、构建、真实网络、CLI、客户端或设备部署：本轮未执行，代码未修改，不沿用以前结果冒充迁移验证。

### 待办与风险

- P0取得2303800当前Decky来源身份及最小脱敏fixture，冻结协议/模型/ID/TTL对象边界和资源限制。
- 锁定Rust HTML/Unicode依赖、跨进程查询发布与限速策略，证明双平台缓存原子写入和取消行为。
- 完成Rust实施及双方适配，才可移除Python HTML discovery worker；EXE worker及平台桥接保留。
- 无Git提交；保留工作区既有代码和其他feature文档修改。

### 独立审阅后的计划修订

- 不仅首次导入：每次切换记录活跃引擎、摘要和共同设置快照，Python回退期间的新选择在Rust重入时导入；双边同字段冲突不静默覆盖，中断/重复切换需幂等。
- 区分核心scan与宿主完整扫描：两端默认scan后自动启动scan_sources(force=false)，保持Decky自动查源；强制/单游戏查询、取消、revision和事件过期明确约定。
- 增加宿主内部get_source_policy，提供Rust核实的compiled/current规则与origin，兼容旧安装消费者读取；这些均是拟新增接口，不是已实现方法。
- 本轮委派2个子Agent：Luna/low只读盘点，Sol（gpt-6.1-sol）/high独立审阅；均仅接收任务说明，无完整历史，无Astra，未修改代码。

## 2026-10-07 P0/P1 契约冻结与 P1 骨架

### 实际完成

- 新增 P0/P1 实现规范 `p0-p1-rust-design.md`：冻结常量、精确 serde 类型、存储分层、ID/TTL/限制、测试矩阵、风险与依赖锁定。
- 新增跨语言 golden `crates/core/tests/fixtures/sources/identity-golden.json` 及可复现生成脚本 `gen_identity_golden.py`；三处哈希由真实 Python 3.10 实测，非推断。
- 新增默认规则资源 `crates/core/resources/sources.json`（schema 2，字节等同 `apps/decky/backend/sources.json`）。
- 新增 `crates/core/src/sources/{mod,model,validate,canonical}.rs`，`lib.rs` 注册 `pub mod sources`。`canonical.rs` 复刻 Python `json.dumps`（分隔符/码点排序/`ensure_ascii`），`candidate_id`/`query_key`/`fingerprint` 与 fixture 逐字节一致。
- 正在续跑 `store.rs`/`registry.rs` 及 §6 P1 其余用例（复用同一 fixer 会话）。

### 验证（本机 `.tools` 便携工具链 + `scripts/cargo.ps1`，cargo/rustc 不在 PATH）

- `cargo test -p steam-patch-core --no-fail-fast`：**通过**（lib 68 + 集成 catalog 8 / install 39 / steam_scan 6 / trust 3 / localized_game_names 4）。
- `cargo clippy -p steam-patch-core --all-targets`：**0 warning**。
- 改动范围 `rustfmt --check`（仅 `crates/core/src/sources/*.rs`）：通过；`tests/install.rs` 既有格式漂移不在本次范围，保持不动。
- 单测覆盖：`sources::canonical` golden、`sources::validate::*`、`sources::store::*`（原子写/替换/链接拒绝/短锁/revision/尺寸上限）、`sources::registry::*`（TTL/订阅/匹配/提交分层/共享索引/版本失效/query_key 负缓存失效）。
- 主 Agent 复核修正两处**测试期望**错误（非实现缺陷）：`filter_by_language` 对 `zh` 应为 0（对齐 `sources.py:883-886`）；停用 `hgamer` 后 `effective_discovery_rules` 非空（`kagura-cn`/`kagura-en` 仍启用）。另修正 `MAX_DISCOVERY_OUTPUT` 常量类型为 `u64` 以匹配存储 API。

### 待办与未执行

- P0 仍缺 Decky 2303800 实时结果与最小脱敏 HTML fixture（设备可用但本轮尚未操作）。
- P2（HTTP/适配器/identity）与 P3（CLI 会话/事件）未开始。

## 2026-10-07 P2 网络层

### 实际完成

- 接入 §8 锁定依赖：workspace 与 core 增加 `url`、`encoding_rs`、`scraper`、`unicode-normalization`、`caseless`、`unicode-general-category`；reqwest 启用 `charset,gzip,deflate`。
- 新增 `crates/core/src/sources/http.rs`：`Fetcher` trait + `FetchResponse`、`ReqwestFetcher`（不自动重定向、受限 body、charset 解码）、离线 `FakeFetcher`、`RequestContext`（预算/截止/按主机限速/重试/Retry-After/重定向逐跳重核/取消），错误码对齐 `discovery.py`。
- `download.rs` 客户端显式 `.no_gzip().no_deflate()`：启用 crate 级 gzip 后，安装包 `Content-Length`/大小校验语义保持不变。
- 主 Agent 修复 http 编译/测试问题：`const` 禁用 `from_secs_f64`；测试内 `let fake = fake();` 遮蔽导致的 `E0618`（helper 更名 `new_fake`）；移除冗余闭包。

### 验证

- `cargo test -p steam-patch-core`：**通过**（lib 87 + 集成 catalog 8 / install 39 / steam_scan 6 / trust 3 / localized_game_names 4）。
- `cargo clippy -p steam-patch-core --all-targets`：**0 warning**；改动范围 `rustfmt --check` 通过。

### P2 完成补充（identity / 适配器）

- `identity.rs`：候选模型（`build_candidates`）、合并/富化/`needs_fallback`、OpenCC 简繁转换；`validate::normal`/`discovery_normal` 升级为真实 NFKC + full casefold + Letter/Number 过滤；OpenCC 表复制到 `crates/core/resources/opencc/`。
- 适配器：`adapters/{mod,dom,patchmoe,table,kagura,hgamer}.rs` 五个适配器 + `discover` 派发；`patchmoe`/`table` 为 ZIP 索引采集，`kagura_wordpress`/`kagura_woocommerce`/`hgamer_html` 为发现适配器。
- 主 Agent 修复适配器编译/测试问题：`SourceCache` 增加 `Default`；patchmoe 多余 `.copied()`；`ElementRef` 为 `Copy`；空字符串比较。
- 验证：`cargo test -p steam-patch-core` **通过**（lib 116 + 集成全部）；clippy 0 warning；改动范围 fmt 通过。
- 遗留：`patchmoe` 因 `RequestContext` 无 POST/body 支持改用 GET 近似，真实本地化路径未验证；真实网络/设备语义核对留待 P2 收口或 P4。

### 待办

- P3（CLI 方法/能力协商/会话事件/scan 投影）未开始。

## 2026-10-07 P3 会话与 CLI

### 实际完成

- 新增 `crates/core/src/sources/session.rs`：`discover_game`（输入校验、有效发现规则、单一共享预算 24/29s、fresh 6h+版本短路、`force`、compare-and-publish）、`scan_sources`（批量、按 AppID 去重、进度/更新事件、取消）、`refresh_sources`（ZIP 索引、24h TTL、失败保留 cache）、`capabilities`、`source_policy`、`resolve_source_record`。
- `Sources` 门面新增对应调用与可注入 fetcher；registry/store 增补 `previous_record`/`commit_if_revision`/`refresh_source`/`source_origin`。
- CLI `crates/cli/src/{runtime,main}.rs`：`METHODS` 增加 11 个来源方法、`Params` 扩展（`source_id`/`enabled`/`subscribed`/`rule`/`name_hints`/`preferred_language`/`force`/`generation`/`record_id`）、`scan` 离线投影 `sources`/`settings_revision`/逐实例 `discovery`、`serve-task` 流式 `source_progress`/`source_update` 事件（`request_id` + 递增 `sequence`），`HELP`/flags 更新。
- 共享 DTO `packages/ui/src/index.ts`：修正 “Decky 专属” 注释（核心拥有发现能力），补齐候选字段（`version_identity`/`published_at`/`patch_updated_at`/`fingerprint`/`related_index_pages`、`mirrors[].label` 可空）。
- 主 Agent 修复会话层 `contexts` 可变性与未用导入、eager-clone lint；对 CLI 改动范围执行 rustfmt。

### 验证

- `cargo test -p steam-patch-core -p steam-patch-cli`：**通过**（core lib 126 + 集成 catalog 8 / install 39 / steam_scan 6 / trust 3 / localized_game_names 4；CLI 14）。
- `cargo clippy -p steam-patch-core -p steam-patch-cli --all-targets`：**0 warning**。
- 改动范围 `rustfmt --check`（`crates/core/src/sources`、`crates/cli/src`）：通过。
- `npm run typecheck`（apps/windows + apps/decky）：**通过**。
- `npm run build:windows`（tsc + vite）与 `npm run build:decky`（rollup）：**均通过**。

### 待办

- P4（Windows 接入与真实验收）与 P5（Decky 切换）未开始；P0 的 Decky 2303800 实时结果与最小脱敏 HTML fixture 待取。
- `patchmoe` 真实本地化 POST 路径未验证（当前 `RequestContext` 无 POST/body，改用 GET 近似）。

## 2026-10-07 P0 实机基线与 fixture

- 设备核对：Devkit 配对 `deck@192.168.31.186`，`plugin_loader` active，插件目录 `root:root 755`；主机密钥变化经用户确认后，本轮采用新指纹写入独立 `known_hosts`（系统旧记录保留未改）。
- 核对 2303800「妃神会秘史 / My Life as a Cult Leader」真实发现：`kagura-cn`/`kagura-en`/`hgamer` 各有 1 个 `discover_only` 候选，`match_basis` 为 `verified_alias`/`verified_alias`/`appid`，均带 hgamer 身份出处；`status` 中 `hgamer=source_refresh_failed`。
- 三条真实 `candidate_id`（`candidate-21d97c05e6cf1324b0cea5c7` / `candidate-d04331eb1335da62bedec536` / `candidate-625cfade8b8786da9e58842c`）由 Python 算法**逐字节复现**，并入 `crates/core/tests/fixtures/sources/identity-golden.json`（`origin=decky-2303800-real`）；Rust `canonical_matches_python_golden` 通过。
- 官方页面抓取到 `.artifacts/`（仅记录 bytes + sha256，正文不入库）。
- 详见 [p0-baseline.md](p0-baseline.md)。
- 未入库：设备状态库全量、完整网页正文、凭证、补丁资产。
- 遗留：页面级在线语义与真实下载/安装验收属 P4/P5；`patchmoe`/`kagura` POST/表单路径未验证。

## 2026-10-07 P4 Windows 接入（进行中）

### 已完成

- `apps/windows/src-tauri/src/main.rs`（fix-2）：新增来源命令面 `source_capabilities`/`list_sources`/`list_game_sources`/`set_source_enabled`/`set_game_source`/`discover_game`/`refresh_sources`/`add_source`/`get_source_policy`/`resolve_source_record`；流式 `start_source_scan`（`serve-task`，转发 `source_progress`/`source_update` 事件）、`source_scan_status`、`cancel_source_scan`；扩展 `CallArguments` 与纯函数 `build_params` 校验；9 个内联测试。
- 验证：`cargo check -p steam-patch-windows` 通过；`cargo test -p steam-patch-windows` **9 passed**。

### 实机验收（Windows，2026-10-07）—— 通过

- 修复（fix-3 / fix-4，经 ora-2 根因）：hgamer 身份作用域 `h2.parent.parent`、`localized_name` 进入查询、`_discovery_sync` 富化二次遍历；`http.rs` cookie jar + POST body + 逐跳 host 校验；`patchmoe.rs` 表单 POST。
- `cargo test -p steam-patch-core -p steam-patch-cli`：core lib **137** + CLI **14** + 集成全绿；clippy **0 warning**；改动范围 fmt 通过。
- `refresh_sources(force)`：patchmoe `source_refreshed`，entry_count **3**（此前失败/0）。
- `discover_game(force=true)`（Windows 实机）：status **complete**、diagnostics **空**、**3 个身份核实候选**，candidate_id 与 Decky 实机**逐字节一致**：
  - kagura-cn `candidate-21d97c05e6cf1324b0cea5c7`（`verified_alias` / `4:…CN Patch v4.exe`）
  - kagura-en `candidate-d04331eb1335da62bedec536`（`verified_alias` / `3:…EN Patch v3.exe`）
  - hgamer `candidate-625cfade8b8786da9e58842c`（`appid` / `steam_appid`）
- 缓存读取：`discover_game(force=false)` → `cache_complete`、3 候选。
- 源选择：停用 hgamer → revision 2 → 强制查询仅剩 kagura-cn/kagura-en 且 status complete；恢复 → revision 3。
- `list_sources`：4 源 enabled、`source_refreshed`；`get_source_policy(kagura-cn)`：origin=builtin、adapter=kagura_wordpress、fingerprint。
- `scan`：33 个安装实例，2303800 正确，静态安装路径未改动。

### GUI 视觉验收（Windows，2026-10-08）—— 通过

- 启动 Tauri 应用（dev）并截图（`.artifacts/p4-windows-gui/`）：列表 33 个安装实例 + 来源 chip（`部分来源`/`已找到来源`）；目录版本 4、信任「已验证签名」。
- 搜索 2303800 → 单条「妃神会秘史 · 已找到来源」；详情渲染候选 tab（含「来源」标签）与候选卡片、能力提示（不据此安装、无安装入口）、`查找来源` 卡片（语言选择/重新查找/诊断）。
- 已验证候选正确渲染「身份已核实 / 按已核实别名匹配」（例：Kagura Games 海外候选）；`查找来源` 显示「共 N 条候选，其中 M 条身份已核实」。
- 来源管理抽屉：全局源分组（发行商/索引）、状态与条数、启用开关、刷新列表/刷新来源索引/强制刷新索引、设置版本。
- 观察（与 Python 一致）：默认 `force=false` 扫描在富化二次遍历失败时保留 title 候选；显式「重新查找」得 `verified_alias`；缓存命中后 GUI 直接显示已核实。
- 说明：GUI 交互采用窗口坐标自动化点击 + `PrintWindow` 截图，是渲染/交互核对，不替代人工端到端体验测试；截图位于忽略目录。

### 未执行

- 便携 release 包（NSIS）未构建。
- 完整跨平台动态安装（Windows 装补丁）仍属后续独立 feature。

## 2026-10-08 P5 Decky 切换到 Rust 引擎（离线完成，待实机部署验证）

### 已完成

- **Rust 增补**（fix-5）：`list_source_matches`/`get_source_state`/`get_source_settings` 方法；`scan` 逐行 `matches`；`list_sources` 带 `origin`；`Params.appid`（`set_game_source`/`list_game_sources`/`list_source_matches` 支持 appid）；首次导入 `ImportReport` 持久化 `rust-v1/import-report.json`；能力表登记新方法。
- **Decky 适配层**（fix-6）：`bridge.py` 来源方法 + `serve-task` 受控 + `source_progress`/`source_update` 事件转发 + `call_sync`；新增 `source_service.py`（`SourceService`，鸭子类型等价 `SourceRegistry`）；新增 `source_engine.py`（引擎选择/`engine-switch.json`/设置导出）；`plugin_settings.py` schema 3（`source_engine`）；`main.py` 经 `open_source_engine` 构造 + `prime` + 事件驱动 `ScanSession`；RPC 名称/形状不变；`publisher`/`exe_*` 零改动；Python 引擎保留为回退。

### 验证（离线）

- `cargo test -p steam-patch-core -p steam-patch-cli`：core **140** + CLI **19** + 集成全绿；clippy 0 warning；改动范围 fmt 通过。
- `python -m unittest discover -s apps/decky/backend -p 'test_*.py'`：**255 passed（3 skipped）**（含保留的 `test_exe_*`/`test_scan_projection`/`test_acquisition`/`test_discovery_rpc` 与回退 `test_sources`/`test_discovery*`）。
- 真实二进制集成探针：`SourceService.initialize()`（rust 引擎/revision 3/4 规则）→ `bridge.scan()`+`prime()`（33 实例）→ `cached_candidates(2303800)`=`verified_alias`/`verified_alias`/`appid`、`cached_discovery=cache_complete`、`game_subscribed`、`validate_download` 均正确（`matches=[]` 符合预期：该游戏无 patchmoe ZIP 条目）。

### 未执行（待实机）

- Decky 设备部署与实机验证：Linux CLI 构建 → `package:plugin` → 通过 [decky-deploy-verify](../../../.agents/skills/decky-deploy-verify/SKILL.md) 部署/重载 → RPC/QAM 核对（旧设置/来源/安装入口与记录保留；订阅关闭后已安装记录可见；`sources.matches`/`cached_candidates`/规则/`validate_download` 不再自行联网/匹配）。
- `apps/decky/plugin-output/` 为旧构建产物，需由打包流程重新生成。

### 2026-10-08 Decky 部署进展（部分完成，待人工重启）

- **Linux CLI**：经 WSL + `.tools/linux-cargo/linux-rustup` 构建 `target/linux/release/steam-patch`（ELF x86-64，11,856,056 bytes）。
- **打包**：重生成 SBOM（`cargo metadata`→`build-sbom.py`，739 组件）后 `package:plugin` 生成 `apps/decky/plugin-output`；`package-artifacts.py` 产出 `.artifacts/steam-patch-manager-decky.zip`（5,974,553 bytes，sha256 `6a038b5bf35e316185170b09da3be27171bc171cfff0fefe8f2acdbd2579954a`）。
- **设备**：`deck@192.168.31.186`；`plugin_loader` active；插件目录 `root:root 755`、内容 `deck:deck`。旧插件已备份（staging `old-plugin.tar.gz`），新 ZIP 上传且设备 SHA-256 一致。
- **部署**：增量替换 backend/bin/dist/顶层文件（除 root 的 `plugin.json`，其哈希未变）；逐文件 SHA-256 校验一致，`bin/steam-patch` 已为新版并含执行位。
- **受阻（已解决）**：首次部署后 Decky Loader 处于「插件已加载且拒绝重载」陈旧状态——结束插件后端进程后未自动重启；`loader/reload_plugin` 调用不返回，`restartWebhelper`/`importPlugin` 无效；无免密 sudo。用户重启设备后恢复。

### 2026-10-08 Decky 实机验证（通过）

- 重启后插件后端以新代码启动（PID 1585），首次即以 Rust 引擎运行，设备上生成 `rust-v1/{schema,settings,import-report,engine-switch}.json`。
- **一次性导入**：`settings.json` 复刻旧 `overrides.json`（enabled patchmoe/hgamer；games 1227890/1788100/3239270）；`import-report.json` = settings 1 / cache 1 / discovery 116 / index 2，1 条 `kagura-cn invalid_discovery_cache` 诊断；`engine-switch.json` active_engine=rust、switch_seq=1。
- **RPC（经 Steam SharedJSContext 现有连接，非新建 /ws）**：
  - `list_sources` 返回 Rust 序列化 4 源（含 `origin:"builtin"`、状态、entry_count）。
  - `list_tasks` 旧安装记录保留（如 1189440 的 committed 记录）。
  - `scan(false)` 返回真实库结构；2303800 `match_status="matched"`、`patches=0`、`sources_len=4`、discovery `cache_complete`，3 个身份核实候选（kagura-cn/en `verified_alias`、hgamer `appid`，candidate_id 与 Windows/设备一致）。
- **结论**：P5 门槛满足——旧设置/来源/安装入口与记录保留；来源、候选、规则与状态改由 Rust 引擎提供，Python 不再自行联网/匹配；`publisher`/`exe_*` 消费接口零改动。
- 未截图：QAM/全屏视觉（本轮前端未改，以 RPC + 设备状态为证据）；实体手柄单列待验证。

### 说明

- added 规则遮蔽 builtin id 时，`builtins` 以 origin 判定，`exe_profiles.rule` 可能返回 `exe_source_disabled`（原 `exe_source_changed`），两者均拒绝安装，属可接受语义微差。

## 2026-10-08 P6 收口

### 已完成

- **移除 Python 来源引擎**：删除 `apps/decky/backend/{sources.py,discovery.py,sources.json,vendor/opencc}` 及无用的 `fixtures/discovery`；新增 `source_primitives.py`（`safe_url`/`allowed_url`/`_normal`/`_split_names`）供保留消费者使用；`exe_generic`/`exe_profiles`/`exe_installation` 改从 `source_primitives` 导入。
- **单一引擎**：`source_engine.py` 去掉 Python 回退（仅 `rust`，非 rust 记 `invalid_source_engine`）；`plugin_settings.SOURCE_ENGINES=("rust",)`；`main.py` 移除 `SourceRegistry` 再导出与 `sources.json` 参数。
- **默认规则单一来源**：核心 `crates/core/resources/sources.json`；插件不再打包 `backend/sources.json`、Python 引擎或 OpenCC 副本。
- **测试**：删除覆盖已迁出引擎的 `test_sources.py`/`test_discovery.py`/`test_discovery_registry.py`；重指向 `test_publisher`/`test_acquisition`/`test_source_import`/`test_source_engine`/`test_settings`。
- **文档**：`docs/source-adapters.md`、`docs/development-plan.md` 更新为实际 Rust 入口；新增 [p5-decky-design.md](p5-decky-design.md)。

### 验证（双端离线/CLI 矩阵）

- `cargo test -p steam-patch-core -p steam-patch-cli`：core **140** + CLI **19** + 集成全绿；`clippy` 0 warning；改动范围 fmt 通过。
- `python -m unittest discover -s apps/decky/backend -p 'test_*.py'`：**175 passed（3 skipped）**；残留 `sources`/`discovery` 导入为 **0**。
- 前端 `npm run typecheck`（两应用）、`build:windows`、`build:decky`：通过。
- 打包产物 `apps/decky/plugin-output`：无 `sources.py`/`discovery.py`/`sources.json`/`vendor`；保留 `exe_worker`；`bin/steam-patch` 为 Linux 新版。

### 仍未迁移的安装器逻辑（明确列出）

- Decky `publisher.py`（publisher ZIP 下载/配方/事务）与 `exe_*`（EXE 提取、`exe_worker` 沙箱、`ExeProfiles`/`ExeInstallation`、能力评估）仍为 Python，属后续独立 feature（将下载/能力评估/静态提取迁入跨平台核心）。本次仅迁移来源发现与订阅。
- Windows 端动态发现的 ZIP/EXE 安装能力未迁移（仅来源展示与安装能力提示）。

### 2026-10-08 Decky 重部署（P6 包）—— 通过

- 增量部署 P6 包：`plugin.json` 未变、逐文件 SHA-256 一致；设备上 `sources.py`/`discovery.py`/`sources.json`/`vendor` 已移除，`source_service/source_engine/source_primitives` 存在。
- 重启设备后新后端启动（PID 1578）：`list_sources` 返回 Rust 序列化来源（含 `origin`）；`scan(false)` 返回 27 个实例，2303800 `match_status=matched`，候选含 kagura-cn `verified_alias`、kagura-en `verified_alias`、hgamer `appid`。**P6 单一引擎在实机生效。**
- 部署经验（已记入 [decky-deploy-verify](../../../.agents/skills/decky-deploy-verify/references/runtime-validation.md)）：本会话以裸文件替换部署、且 P5 曾手工结束后端进程，触发 Loader「已加载、拒绝重载」陈旧状态，只有重启 `plugin_loader`/设备才清掉；后续应走 Loader 正常安装/重载流程。

## 2026-10-07 Git 归档

- 用户后续授权提交 Git；迁移设计、施工记录及关联索引按独立文档提交归档，不包含功能实现代码。
- 本地化名称代码、Windows 扫描锁修复及各自文档分别提交；其他既有修改保留在工作区。迁移阶段状态仍为待实现，验收项仍待执行。
