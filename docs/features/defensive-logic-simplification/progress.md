# 防御逻辑精简施工记录

设计与验收标准：[design.md](design.md)。

## 当前状态

**已实施并提交（e70799e、bc122fd、82e3d92、5cc6843、22ee515、7bb7604）；外部审查三项修复已复验；Steam Deck 实机部署与运行时/RPC 验证通过（WSL Linux 测试含 Unix 专属用例）；界面面板/全屏画面与实体手柄操作待人工确认。**S01–S19 全部完成（含配套收尾、P5 复核、审查响应与实机部署），记录见下文。

| 批次 | 项目 | 状态 |
| --- | --- | --- |
| P1 | S01、S02、S05–S08、S19 来源与设置 | 已实施；本地自动化通过（S05 为 Decky/Python 侧） |
| P2 | S03、S04、S16 清理与事实 | 已实施；本地与 WSL Linux 测试通过（含 100,001 条目慢测试实跑、160 层深目录与 Unix 专属用例）；SteamOS 实机未单独复跑 Unix 清理用例 |
| P3 | S09、S10、S17、S18 协议与调用生命周期 | 已实施；本地自动化通过（Rust/CLI/Windows/Decky/契约）；真实跨端传输待验证 |
| P4 | S11–S15 下载与格式解析 | 已实施；本地自动化通过；真实网络下载与实机待验证 |
| P5 | 最终路径审阅及文档收口 | 独立过度设计审阅完成并收尾（4 项）；外部审查响应三项修复完成（见文末）；文档更新 |

## 2026-10-17：建立独立精简计划

- 基于本次对话在 `80829ea` 上完成的静态审阅，整理初始7项、后续3组扩展范围和新增5项，共S01–S15。
- 记录必要路径、摘要/CRC、来源主机、安装互斥和实际解析边界的保留理由，区分可删局部分支与整类机制。
- 更正候选数量超限的影响：适配器保留未合并结果并降为partial，不是直接丢弃全部候选。
- 明确8 MiB响应限制和普通调用300秒总时限进入删除范围；来源字节门槛不得仅依历史数值保留，实际消费路径仍需施工核实。
- 保留清理深度、诊断条数、请求大小和解析内存等相邻机制的待核实状态，不把它们自动列为已确认可删。
- 创建设计、施工记录并更新功能索引。未修改生产代码、未提交、未构建、未发布、未操作设备。
- 文档验证：两份新文档的全部相对链接目标存在，S01–S15共15项且编号连续；功能索引同时链接设计和施工记录。业务自动化和实机验证：未执行。

## 待办

- 界面人工确认：Steam Deck 上插件 QAM 面板与全屏页的真实画面、实体手柄焦点/返回（用户在设备上操作；脚本点击未定位到面板入口，不冒充通过）。
- 实机/真实端到端仍待覆盖：Windows 宿主与 Decky 实机 >8 MiB 响应与大请求传输、真实网络无后缀下载、真实流式网络中途取消、真实 SF9.x 大样本、Decky 实机游戏安装（本轮部署不含游戏安装/卸载）。
- Unix 分支：WSL Linux 编译与测试已通过（`build-linux.sh`；core lib 415 passed 含 Unix 专属用例）；SteamOS 实机如需单独复跑 Unix 清理用例再补。
- 基线门禁修复已完成（见“基线门禁修复”）；如需复跑完整 CI（合约 check、打包等）按需执行。
- 本轮记录（含后续进度修复重新部署）已按用户指示提交。未验证部分不得声称实机验收或全量 CI 通过。

## 2026-10-17：完成原未决边界核查

- 新增[边界核查与最终决策](boundary-review.md)，逐项阅读源码、调用方、现有回归源码及P2/P5历史证据。
- S08确定删除本地来源数量/字节门槛；新增S16清理深度（显式栈）、S17诊断完整保留、S18普通业务请求上限、S19 HTML正文上限，共19项。
- 明确保留解析器当前受限内存/解释模型及终态后的诊断排空，写明触发条件、代码作用和证据；不宣称具体预算是格式理论最大值或最优容量。
- 前节“仍待核实”是建档时历史状态，由本节关闭；当前计划没有待实施时再决定删留的事项。
- 本轮未改生产代码，未运行业务测试、未重新执行历史复现、未操作设备。实施和功能验收仍为待完成。

## 2026-10-17：S01–S19 实施与自动化验证

- 基线 `80829ea`；改动在工作区尚未提交，未操作设备。S01–S19 按 P1–P4 分多条独立通道实施；生成契约（packages/contracts/generated、fixtures）与 `scripts/test-ui-contract.mjs` 同步更新。实施后由独立审阅（overengineering-review 技能）完成 P5 复核，收尾项已处理（见“P5 独立复核”）。全部改动就位后在冻结工作区执行整树验证（见文末）。
- 以下按批次记录实际删除范围、调用链核对、必要回归与结果；文档记录无法替代未执行的实机验收。

### P1 来源与设置（S01、S02、S05–S08、S19）

- S01：`identity.rs::validate_candidate` 删除列表最多64项、DLC条件最多32项、镜像最多64项判断，及 `invalid_discovery_mirrors` 错误串（全仓无其他产生点）；调用链核对确认 `adapters/mod.rs::discover` 无独立数量门槛，未改。保留非零 AppID、身份、URL、元素长度与实际格式检查。新增“超原门槛候选仍可合并/发布/持久化”测试（70 项列表/DLC/镜像）。
- S02：`registry.rs::set_game_source` 删除对完整 overrides 序列化测长后返回 `source_limit` 的分支；写入统一由 store 入口处理。新增 >256 KiB 合法设置可修改、持久化并重载的验证。
- S05（Decky）：`source_service.py` 删除桥接层 schema 验证后的重复 isinstance 过滤、静默 return/continue 与掩盖违约的空结构，内部违约改为显式 `ValueError`；保留协议允许的可选字段、`cached_discovery` 的 not_queried 正常快照与 `protocol_validation.validate_response` 真实校验（核对确认为真实契约校验，未精简）。新增 3 项测试。
- S06：`store.rs` 的 `revision()/settings()` 改为返回 `Result` 并沿现有路径传播锁错误（删除版本0/default_settings 兜底）；`run_if_revision`、`registry::reload` 同步调整；保留首次初始化默认设置。毒化锁测试确认返回 `source_state_busy` 且不改写磁盘设置。
- S07：复核全仓无引用后删除 `REFRESH_WORKER_TIMEOUT`，未引入替代定时器。
- S08：删除 `MAX_SOURCES`、`MAX_DISCOVERY_OUTPUT`(512 KiB)、`MAX_STATUS_BYTES`(4096)、`MAX_CACHE_BYTES`(4 MiB)、`MAX_DISCOVERY_INDEX_BYTES`、`MAX_SETTINGS_BYTES` 及对应全部固定拒绝：registry 数量门、commit 的 index/记录测长与“缩减 attempt.index 重编码”分支、validate 的 canonical 测长与 overrides ids 门、store 读写 max 参数、kagura index 超限诊断。保留文件身份/类型/链接检查、JSON 结构/schema、原子写入、共享索引复用与真实 IO 错误（`detection` 独立语义未动）。新增 8 MiB 原子写入、4097 项列表、6000 条共享索引重开等验证。
- S19：`http.rs::fetch` 删除 `take(MAX_HTML+1)` 改完整读取；`fetch_html` 删除原始/解码后两次长度拒绝及依赖旧错误串的死回退分支；删除 `MAX_HTML`。保留 HTTP/编码/挑战页/主机/逐跳重定向/取消检查。新增原始与解码后 >4 MiB 正文完整处理验证。
- 通道结果：`sources::` 153 passed；lib 403 passed/6 ignored；`source_snapshot` 5、`catalog` 8。遗留：reqwest 真实传输路径由离线替身覆盖，真实网络待验证。

### P2 清理与事实（S03、S04、S16）

- S03：删除 `cache.rs` 的累计访问门槛 `MAX_ENTRIES=100_000` 与 Unix/Windows `Node::names` 单目录门槛、`backup.rs` 的 `LIMIT/DEPTH` 同类判限；保留统计计数、`checked_add` 溢出检查、归属/链接/锁与真实错误。计数门槛断言与溢出断言拆分；新增 160 层深目录测试；`#[ignore]` 慢测试本轮实跑（100,001 条目，约 96.5s）——完整盘点、清理、再次盘点为 0。
- S04：删除 `facts.rs::compute_dlc` 的 4096 depot / 128 DLC 拒绝；`signed.rs::require_facts` 复核确认无独立数量门（仅传播 `is_valid` 首个失败码），未改。新增 4097 depot+129 DLC、130 条 installeddlc 与 >128 DLC 签名安装前置检查验证。
- S16：删除遍历深度 128 拒绝，并把 cache/backup 递归遍历改为局部显式栈、后序删除（cache：`inspect_tree/inspect_children` 与 `Frame` 驱动的 `delete_subtree/process_child`，`Node::capture`/`Captured` 保留 Unix quarantine 句柄与 Windows 目录句柄及原名复核、失败恢复；backup：`open_material/count_file/MaterialFrame/finish_dir`）；未新增持久状态机。
- 通道结果：lib 403 passed；`cache` 2、`signed_install` 24、`installer_service` 6；慢测本轮实跑通过。遗留：Unix 分支本机无 Linux std 未编译，Unix 专属测试待 Linux/SteamOS 或 CI。

### P3 协议与调用生命周期（S09、S10、S17、S18）

- S18：删除 protocol `MAX_REQUEST_BYTES` 与整行拒绝、CLI `read_request` 上限与 `oversized` 参数、Windows 4 处 `MAX_REQUEST` 检查、contract-gen/生成物常量；保留 JSON Lines 分帧、逐方法 schema（含 roots≤32）、方法白名单与 ID 校验；worker 描述符与控制消息未动。
- S09：删除 protocol `MAX_RESPONSE_BYTES` 与 `bounded_json_line`（Capped/overflow/response_too_large），新增无上限 `json_line()`；CLI 删除 response_too_large 回退；Windows `read_all_bounded`→`read_all`、任务流无上限 `read_line`；Decky bridge 删除 StreamReader limit 与 response_too_large 路径。终态排空完整保留：Windows `INSTALL_DRAIN_TIMEOUT/BYTES`、`drain_install_terminal/drain_terminal_tail`（`read_bounded_line` 改用既有命名预算）；Decky `TERMINAL_DRAIN_SECONDS/BYTES/CHUNK`（`TERMINAL_DRAIN_BYTES` 显式 8 MiB，独立命名）。新增 >8 MiB 响应完整往返与坏行后继续处理验证（CLI/Windows/Decky）。
- S10：删除 `CoreBridge(timeout=300)` 与普通调用总时长 wait_for/杀进程路径及仅为此存在的转发参数；保留 shield、取消、读取故障、进程错误与终态排空。新增“无总时限参数”“长时间有进展调用不中断（替身）”验证。
- S17：protocol 删除 `MAX_DETAIL_BYTES`/`truncate_detail`/validate_bounds 的 detail 长度拒绝与 schema `x-max-utf8-bytes`（instance_id 上限保留）；CLI/Windows 不再裁剪 detail；contract-gen 删除 TS `truncateDetail` 与常量、fixtures 同步；Decky 删除 `protocol_validation.truncate_detail`、接收端长度拒绝与 `tasks.py` 唯一调用点；core 删除 cache/backup 512 条诊断上限（含省略覆盖末条）、`outcome.rs::bounded_detail`、`projection.rs` `DomainError` 截断、`install.rs::bounded_snapshot_message`。保留结构化 code、disposition/提交状态独立推导与统计计数；新增 >512 条诊断、>2048 字节多字节 detail、3 MiB 历史错误重放完整保留验证。
- 生成契约：`generate` 后 `check` 通过；同步修复基线漂移（HEAD 生成物缺 `ScanSourcesParams`，与 lib.rs `schema()` 不一致），复核确认为必要修复。
- 通道结果：protocol 2+3、CLI 47+1+2、Windows 33、`npm run test:contracts` 通过；Decky 后端 91 项通过（连续 6 次），`test_v4_bridge` 20 次复跑通过（continuous_tail 用例定时 patch 由 .02s 放宽至 .2s；基线同用例存在 1/20 调度竞态，排空代码未改）。

### P4 下载与格式解析（S11–S15）

- S11：`validate.rs::allowed_url` 下载分支删除 `.zip` 后缀判断（fragment 规则保留）；`acquire.rs::ArchiveKind::url` 删除 `.exe` 检查；同步删除 `resolve.rs::MirrorKind::validate` 的镜像后缀准入以保持生产选择入口一致。保留主机/HTTPS/端口/逐跳重核/响应终址核对/内容头与 ZIP/PE/SF 实际解析。新增无后缀端点用例（有效 ZIP 下载、重定向、HTML 内容仍 `download_not_zip`、MZ 走 EXE profile、越权主机仍失败）。
- S12：删除 `load_cached` 末尾预期 SHA 与实际 SHA 的恒假重复比较；保留重哈希与前面两项比较。
- S13/S14/S15：`audit.rs` 删除 `package_count_limit`(256)、`resource_entry_limit`(256)、`count <= MAX_ENTRIES` 恒真比较与 `MAX_ENTRIES` 常量；`installer-worker/sf.rs` 删除冗余 `payload.len() <= MAX_ENTRIES`（核实为与 manifest 子集校验重复，ZIP 计数按 u16 饱和/ZIP64）；`scripts/sf95_audit.py` 同步删除 package/resource 门槛，并依据历史证据（8192 系真实样本从 4096 上调的策略值，跨平台设计列为待取消低固定门）删除脚本侧 `MAX_ENTRIES=8192`，与 Rust 对齐为按实际 u16 计数、仅保留 `count>0`。补 300 包引用、257 资源项且 manifest 有效正向验证；截断/无效引用回归保留。
- 通道结果：installer-format、installer-worker 测试通过；`test_sf95_audit` 18、`test_sf9_behavior` 10。

### 配套收尾

- `packages/ui/src/index.ts` 删除已无生产者的死映射：`discovery_html_too_large`、`discovery_output_resource_limit`、`discovery_index_resource_limit`、`discovery_index_resource_limit_incomplete`、`request_too_large`、`archive_too_large`、`exe_manifest_limit`；保留活码 `resource_limit`、`discovery_index_incomplete` 与通用兜底。`evidence_size_limit` 仍有真实生产者（detection 的 digest 预算），保留。
- `packages/contracts/README.md` 更新为当前契约描述（JSON Lines 无固定整行上限；detail 完整不截断）。
- 残留审计（源码范围）：被删常量与错误码（MAX_REQUEST_BYTES、MAX_RESPONSE_BYTES、MAX_DETAIL_BYTES、MAX_SOURCES、MAX_DISCOVERY_OUTPUT、MAX_STATUS_BYTES、MAX_CACHE_BYTES、MAX_HTML、REFRESH_WORKER_TIMEOUT、source_limit、truncate_detail、package_count_limit 等）全仓无残留引用；应保留项（sf9_lua/lua.rs/exe_report.rs 的 96 层、keyvalues 32 层、resource_limit、终态排空）均在。
- `apps/decky/plugin-output/**` 为 gitignore 构建产物，无需同步。

### P5 独立复核（overengineering-review 技能）

- 结论：“建议修改（整体落实到位，仅少量收尾残留）”；未发现用新阈值/兜底/框架替代被删旧机制；保留清单（路径/身份/链接、摘要、主机许可、互斥锁、解析边界、终态排空）完整；生成契约一致、无悬空常量。收尾项已全部处理：
  1. `detection::read_bounded` 的 `u64::MAX` 哨兵与 `before.size > max` 恒假判断删除（含唯一调用方与注释），保留增长/身份/变化检测；`read_digest_with` 的真实预算保留。
  2. `packages/ui` 的 `exe_manifest_limit` 死映射删除（S04 后无生产者）。
  3. `cache.rs::process_child` 的 task 白名单 depth 语义补注释（说明与删除前递归的等价前提；无行为改动）。
  4. S19 的“读取中可取消”表述与实现对齐：正文为单次完整读取、取消沿用请求/操作级检查（与基线一致），已修正 design.md 与 boundary-review.md 措辞，不新增分块取消机制。
- 复核确认的必要项：显式栈的 `Captured/MaterialEntry` 抽象属必要重构；生成契约补 `ScanSourcesParams` 属必要漂移修复。历史功能文档（如 cross-platform 的 p1-host-contract）仍记载旧上限，属历史记录未改写；当前契约以 packages/contracts 与代码为准。

### 整树验证（全部改动就位后，Windows 本机）

- 通过：`cargo run --locked -p steam-patch-contract-gen -- check`；`cargo test --locked -p steam-patch-protocol`；`cargo test -p steam-patch-core -p steam-patch-cli --locked`（core lib 403 passed / 6 ignored，全部集成套件 0 failed；P5 收尾修复后复跑仍 403 passed / 0 failed）；`cargo test -p steam-patch-installer-format -p steam-patch-installer-worker --locked`；`cargo test -p steam-patch-windows --locked`（33 passed）；`cargo build -p steam-patch-core -p steam-patch-cli --locked`。
- 通过：Python 后端 91 项（skipped=2）；scripts：`test_sf95_audit` 18、`test_sf95_worker` 5、`test_package_artifacts` 7、`test_build_sbom` 1、`test_install_decky` 30；`npm run test:contracts`、`npm run typecheck`、`npm run test -w apps/decky`、`node --test scripts/test-install-decky-bridge.mjs`。
- 基线既有门禁问题（非本 feature 引入，已按用户指示随本轮修复，详见“基线门禁修复”）：fmt 漂移曾约 20 处、涉及约 15 个文件（可 blame 至基线前提交）；CI clippy 曾因 `installer-worker/sf.rs` 的 `count as u64`（d88d2235）失败 2 处；`test_decky_bootstrap` 曾因 README 列表缩进（e8aef7d）失败 1 项。

### 基线门禁修复（随用户指示，2026-10-17）

- `cargo fmt --all` 全量对齐（含 catalog.rs、install_tests.rs、authorization.rs、exe_orchestrator.rs、exe_resolve.rs、orchestrator.rs、signed.rs、task.rs、hgamer.rs、session.rs、installer-worker launcher、windows main.rs 等基线漂移文件；feature 文件逻辑未动），`cargo fmt --all -- --check` 干净。
- clippy 机械修复：`installer-worker/sf.rs` 两处 `count as u64` → `count`；修复后暴露的 3 处机械告警一并处理（cache.rs `field_reassign_with_default` 改结构体更新语法、install_tests.rs `needless_borrow`、signed_install.rs `single_element_loop`），均行为不变；CI clippy 命令通过。
- `scripts/test_decky_bootstrap.py` 放宽 `read_command()` 正则以容忍 README 列表缩进，保持精确命令/shlex/长度断言；测试通过（Linux 专属 8 项按设计 skip），两 README 仍解析出同一命令。
- 以上与 feature 属不同变更集，提交时建议按仓库规则分开。

### 未验证/遗留（保持待验证）

- Unix 分支编译与专属测试（本机仅 Windows std；cache/backup 显式栈的 Unix quarantine/链接路径、launcher 等）。
- 真实 Windows 宿主与 Decky 实机跨端 >8 MiB 响应、大请求传输；真实网络无后缀端点下载；真实 SF9.x 大包样本；Decky/SteamOS 实机安装。
- `freeze_sf_rust_fixtures.py` 未实际执行（需真实 fixture 输入）。

## 2026-10-17：外部审查响应（GPT，三项 P2）

外部审查针对已提交的 e70799e/bc122fd/82e3d92 提出 3 项问题，均已核实为真并修复后提交：

- **缓存盘点句柄随目录宽度增长**（S16 改写引入）：`inspect_tree` 曾把每个同级子目录的已打开句柄压入工作列表，句柄数随目录宽度增长（审查者在 Linux 以 fd 上限 256、300 个同级目录复现 47 次 EMFILE 与盘点遗漏）；改为保存枚举进度的深度优先帧栈（每帧一个打开目录句柄与游标），任一时刻打开句柄数 ≤ 当前深度，visit/record/protected/单条失败继续等语义与改写前等价。新增 `wide_and_deep_cache_tree_is_fully_inventoried`（300 同级 + 150 层链，entries=752、bytes=301、无诊断）。
- **Unix 捕获 rename 后丢失恢复**（S16 改写引入）：`Node::capture` 在 `renameat` 成功后，`q.child(item)` 或 `same(...)` 失败会经 `?` 直接返回，原条目滞留 `.cleanup-*/item` 且隔离目录不清理；改为 rename 成功后所有失败统一执行现有 `restore` 与 quarantine 清理，与改写前递归版逐路径等价；Windows 捕获无 quarantine，无需恢复。
- **正文读取期间不可取消**（S19 决策遗留）：`ReqwestFetcher::fetch` 的 `read_to_end` 无取消检查，持续流式响应下取消需等整页读完；改为 `read_body_checked` 64 KiB 分块、逐块 `operation::check()`（不恢复大小上限与总时限，传输错误仍映射 `discovery_network_error`，取消错误原样传播）；CLI `stream_sources_task` 以 `operation::with_cancel` 安装取消作用域，使 scan/refresh 读取与任务取消共享同一标志。新增 3 个合成读取测试（>8 MiB 完整读取、激活未取消作用域、读中取消返回 `task_cancelled` 且无部分正文外泄）。

验证：本机 `cargo fmt --all --check`、CI clippy 命令、`cargo test -p steam-patch-core`（lib 407 passed/6 ignored 与全部集成套件）、`cargo test -p steam-patch-cli` 全部通过；独立审阅（静态逐路径核对）结论“通过，无必须修改项”，并确认 Unix 恢复修复消除的是 HEAD 上的真实缺陷。待验证：Unix 分支编译与 Unix 捕获用例（需 Linux/CI，建议 `cargo test -p steam-patch-core installer::cache`）、真实流式网络中途取消时序、CLI serve-task 端到端取消事件序列。

## 2026-10-17：Steam Deck 实机部署与验证

按 [decky-deploy-verify](../../../.agents/skills/decky-deploy-verify/SKILL.md) 流程把 S01–S19 与审查修复部署至当前 Devkit 配对设备并完成运行时/RPC 验证；界面面板与手柄操作留待人工确认（未冒充通过）。

- 设备与安全：Devkit 当前配对 `deck@192.168.31.186`；WSL `ssh-keyscan` 三类主机密钥与本轮 `known_hosts`（沿用上轮已确认记录）完全一致；部署前 `list_tasks` 30 项全部终态（completed 13 / failed 11 / cancelled 6），无进行中任务。
- 构建：前端 `npm run build:decky` 重建 dist；Linux 侧经 `scripts/build-linux.sh`（WSL，仓库本地工具链）先测后建——core lib **415 passed / 0 failed / 6 ignored**（含 Unix 专属用例，解除 P2“Unix 分支待编译/验证”在测试层面的限制）、CLI 47+1+2 passed；产出 `target/linux/release/{steam-patch,sf-worker}` 为新 ELF（14,306,704 / 1,800,272 B）。
- 打包：`package:plugin` + `package-artifacts.py --platform decky`；ZIP `steam-patch-manager-decky.zip` sha256 `8ba1dd3b…`（7,231,393 B），24 个成员与本地清单一致。
- 备份：旧插件全量 tar.gz sha256 `75cac1f7…`（24 文件）取回本地 `.artifacts/defensive-logic-deck-20261017/backup/` 并核对，旧清单/大小表同目录。
- 部署：经 Loader `utilities/install_plugin` + `confirm_plugin_install`（prompt 参数核对：`steam-patch-manager` / `dev` / request_id / hash 与本轮 ZIP 一致）；正式目录 24/24 文件哈希与清单全等、`bin/*` 保持执行位、`plugin_loader` active、后端进程重启。
- 运行时：插件实例 1、版本 0.1.0；HTTP bundle 返回 200、115,789 B、sha256 `996223c2…` 与本轮 `dist/index.js` 一致。注：Loader 的 dist 路由以 plugin.json `name`（URL 编码）为键，用插件目录名请求会 500，属查询方式问题而非插件缺陷。
- RPC：`list_tasks` 全终态；`list_transactions` 17；`scan(false)` 返回真实结构（28 安装实例 / 4 来源）——新版 CLI 在 SteamOS 实际执行。
- 界面：QAM 已打开并取得整屏截图；插件面板与全屏页真实画面、实体手柄焦点/返回由用户在设备上人工确认。
- 清理：远端暂存目录 `/home/deck/steam-patch-validation/defensive-logic-deck-20261017/` 已删除；CEF 隧道已关闭（18080 无监听）；本地保留备份与全部证据（`deployed-manifest.sha256`、install/runtime/RPC JSON、qam.png 等）。
- 未覆盖：SteamOS 实机单独复跑 Unix 专属清理用例；真实流式网络中途取消；>8 MiB 跨端传输；游戏安装/卸载不在本轮范围。

### 2026-10-17 后续：进度显示修复的重新部署

- 实机反馈 EXE 安装"分析/提取"阶段进度恒 0%；核查为 EXE 编排实现以来的既有显示行为（部署前 10-08~10-10、10-16 轮实机记录一致），非 S01–S19 回归。修复仅动 `exe_orchestrator.rs` 两处 stage 调用（不再上报 total，前端显示不确定进度），详见 [generic-exe-installation 施工记录](../generic-exe-installation/progress.md) 2026-10-17 条目。
- 重新构建并按同一流程 Loader ZIP 部署（ZIP 7,231,185 B `bcd725d7…`；相对线上仅 `bin/steam-patch` + `bin/runtime.json` 变化）。正式目录 24/24 与新清单全等；实例 1 / 0.1.0、`plugin_loader` active；HTTP bundle 200（`996223c2…` 未变）；RPC `scan(false)` 28 实例 / 4 来源、`list_tasks` 31 全终态、`list_transactions` 18；Loader 设置前后一致；远端暂存已清理、隧道已关闭。
- 用户在该构建上完成一单 3.53 GiB EXE 实机安装（task `8d0c735d-…`，终态 completed/cleanup）：安装、核对、写入、清理全阶段实机完整跑通，上述"游戏安装不在本轮范围"的限制由本次实机安装覆盖。
- 仍待验证：修复后的分析/提取不确定进度显示需下一次实机安装确认；Unix 专属用例复跑、真实流式网络中途取消、>8 MiB 跨端传输保持原状态。证据：`.artifacts/defensive-logic-deck-20261017/`（stagefix 文件集）。
