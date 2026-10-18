# 来源缓存与游戏匹配重构施工记录

- 设计：[design.md](design.md)。
- 当前状态：P0 基线/契约、P1 数据/存储/迁移、P2 按源更新、P3 增量匹配、P4 双端与自动更新、P5 切换清理均已实现并通过自动化；P6 自动化验收与过度设计复核完成，真实网站关键链路（HGamer live、Kagura 列表结构与 EN 多入口）已通过；双端实机/安装链真实回归未执行，**实机验收前不合并 main**。P6 证据：[p6-acceptance.md](p6-acceptance.md)。
- 分支：`feature/source-cache-matching-refactor`。

## 2026-10-18：制定重构计划

- 从干净的 main（0721338）创建功能分支。
- 核对当前 Decky 扫描、Rust 来源调度/缓存/Kagura/HGamer、Windows 调用入口及既有持久缓存设计。
- 形成按源更新与本地游戏匹配分离、每源自动更新间隔、HGamer fallback、增量匹配、单游戏重新匹配、迁移及旧逻辑删除计划。
- 明确本次新需求取代旧设计中的“不提供定时更新”，但不恢复按年龄禁止使用缓存。
- 按 overengineering-review 完成计划自审：复用存储/事件/安装链；不引入新服务/数据库/通用框架；覆盖真实失败与取消语义。不是独立代码审查或实机验收。
- 本轮仅修改文档及功能索引；未实现功能、未运行功能测试、未提交 Git、未操作 Steam Deck。

## 阶段状态

| 阶段 | 状态 | 验证证据 |
| --- | --- | --- |
| P0 基线与契约细化 | 已完成（2026-10-19） | 实测基线表与逐项证据见 [p0-baseline.md](p0-baseline.md)；冻结数据/协议/调度契约见 [p1-contract.md](p1-contract.md)；双端改动测绘见 [p4-host-map.md](p4-host-map.md) |
| P1 数据与协议 | 数据/存储/迁移已实现并通过自动化；协议 v5 与宿主切换统一留到 P4 | `cargo test -p steam-patch-core` 通过；迁移/修订/调度纯函数测试 9 项；`contract-gen check` 通过（协议仍 v4，未生成 v5） |
| P2 按源更新 | 已实现并通过自动化 | `update_source` 单源更新、patchmoe/table/Kagura 列表索引、进度文件续跑、失败保留旧快照；`source_update` 17 项测试通过 |
| P3 增量匹配与 fallback | 已实现并通过自动化 | `matching.rs` 增量复用/按需详情/HGamer 六态/重匹配/来源重算；`source_matching` 21 项测试；core 全量 575 passed |
| P4 双端与自动更新 | 已完成（2026-10-19，自动化） | P4a 协议 v5+CLI+生成物；P4b Decky（backend 98 tests、frontend 61、typecheck/build/package）；P4c Windows（Rust 39 tests、status 12+17+8、typecheck/build）；调度与来源/匹配 UI 已实现，实机待验证 |
| P5 切换和清理 | 已完成（2026-10-19，自动化） | `session.rs` 与旧 cache/discovery 流程删除；`SourceRegistry` 改为 schema2 读模型（index/match/detail）；全量门禁通过（workspace tests、clippy -D warnings、contract check、fmt、双端 typecheck/测试） |
| P6 验收 | 自动化完成；真实网站关键链路（HGamer live、Kagura 列表结构与 EN 多入口）通过；实机待验证 | `crates/core/tests/source_acceptance.rs` 11 项、CLI 只读/移除游戏 2 项、`hgamer_live`（ignored，真实网络）与 Kagura 双入口 fixture 通过；Rust 门禁 705 passed；真实网站、偏差与合并建议见 [p6-acceptance.md](p6-acceptance.md) |

## 后续重点

- 核实 Kagura 列表覆盖与成本；索引匹配零网络，命中后只获取相关详情，缓存跨游戏共享。
- 固化自动更新控件；当前计划默认关闭，按宿主运行期间调度。
- 核对旧缓存覆盖范围、人工身份/DLC/镜像迁移与安装内容绑定。
- 实施后逐阶段补充实际修改、命令、结果和证据，不将本计划勾选为实现完成。

## 2026-10-19：采纳索引先行、详情按需

- 用户纠正全源详情预取方案：来源只更新游戏/补丁索引，游戏命中后获取相关详情。已同步修改数据模型、工作流、阶段门槛与验收要求。
- 普通扫描复用详情；手动重匹配刷新命中详情；来源更新后补充当前游戏库命中详情。游戏任务不遍历全站索引。
- 当前仅依据已读适配器确认能解析列表标题与链接；实时列表完整性/AppID/DLC 覆盖仍待 P0 验证。未执行网站采集或功能测试。

## 2026-10-19：P0 基线与 P1 契约/存储实现

- P0 实测（fixture 驱动，FakeFetcher 计数，Rust 1.94.0 / Windows 11）：1 游戏首次 2 请求；10 游戏首扫 11 请求（索引 1 + 详情 10）；暖缓存 0；force 刷新已命中游戏 1；同批两款游戏同详情页请求 2 次；10 未命中首扫 11（索引 1 + HGamer 10），强制 20（索引逐游戏重翻）；patch.moe 首次 3 / 304 1 / 强制 3。完整表、口径、行号与静态核对见 [p0-baseline.md](p0-baseline.md)。
- 修正原计划表述：普通首扫并不会逐游戏重建普通源索引（首游戏构建、其余复用持久化索引）；逐游戏全站遍历只在 force 路径复现。宿主整库重复投影仍是真实成本（Decky 一次周期 3 次整库 scan、单游戏查询另 2 次）。
- 冻结 [p1-contract.md](p1-contract.md)：schema 2 目录（index/details/matches/status/settings.intervals）、内容修订语义、input_key 与增量条件、六态匹配结果、迁移规则、协议 v5 方法面与事件、调度语义、删除范围。
- P1 实现（未提交）：`sources/model.rs` 新增持久化类型；`store.rs` 支持 index/details/matches、settings v1→v2、幂等迁移（失败保留旧文件并报 `source_migration_failed`）；新增 `index.rs/match.rs/schedule.rs` 纯函数；`crates/core/tests/source_data_v2.rs` 9 项测试通过；`crates/core/tests/p0_source_baseline.rs` 基线测试通过。
- P4 只读测绘完成（`p4-host-map.md`）：确认协议 v5 必须与双端宿主、生成物、打包脚本同一切换单元，P1–P3 不提前升 v5、不删旧方法。
- 未验证：真实网站分页与耗时、Deck/Windows 实机、游戏内生效；迁移只用合成样本验证，未用真实用户旧状态。

## 2026-10-19：P2 按源更新与 P3 增量匹配实现

- P2（`sources/update.rs` + 适配器）：每源一次更新、无游戏参数；正式 `index/<source>.json` 只在完整成功时替换，失败/取消写 `index-progress/<source>.json` 续跑；patchmoe/table 复用原 ZIP 收集并映射索引条目、支持条件请求；Kagura 只遍历列表/目录分页（不抓详情），失败页留 pending，取消保存进度；`DetailRecord` 改为来源拥有的 `PageFact`（query 无关），新增 `resolve_page` 供匹配按需抓取；HGamer 不提供全站索引（`source_not_indexable`）。`crates/core/tests/source_update.rs` 17 项通过。
- P3（`sources/matching.rs`）：本地索引筛选（appid 优先、标题/别名其次）→ 命中项按需详情 → `identity::build_candidates` 生成候选；详情缓存按来源+规范 URL 跨游戏共享；HGamer fallback 仅在无普通源可信命中且全部启用普通源覆盖完整时按游戏查询，普通源命中优先显示、失败/不完整不冒充无补丁；六态结果持久化到 `matches/<appid>.json`；`match_games`/`rematch_game`/`rematch_source`/`game_match_summary`/`source_revisions` 提供 P4 接口；名称提示与语言偏好进入 input_key/fallback query_key。`crates/core/tests/source_matching.rs` 21 项通过；core 全量 575 passed / 0 failed。
- P4a 启动：协议 v5（20 方法、4 类事件）、CLI 执行面与契约生成物切换；宿主切换紧随其后。
- 未验证：Kagura 真实站点分页覆盖、HGamer 真实查询、详情页真实结构、双端网络行为；仅 fixture 驱动自动化。

## 2026-10-19：P4 协议 v5 与双端切换

- P4a（协议/CLI/生成物）：`PROTOCOL_VERSION=5`，方法 20（来源 9 + 安装器 11，新增 `project_instances/set_source_update_interval/update_source/match_games/rematch_game`，删除旧 3 方法）；4 类事件 `source_progress/source_update/match_progress/match_update`；capabilities 7 项源版本；`SourceSummary` 新增 revision/complete/interval_hours/next_due_at；`update_source` 完成后按修订重算受影响游戏，`match_games`/`rematch_game` 接通 matching 增量接口；`scan` 固定一次 steam 扫描并用持久化匹配记录覆盖 discovery；契约由 generator 重新生成 v5（删除 v4 生成物）。
- P4b（Decky）：scan 只 1 次 bridge.scan、不再刷新来源/逐游戏发现；按源/匹配键任务注册表（同键去重、可取消、4 类事件消费）；新增 update/match/rematch/task_status/cancel/interval/project_instances RPC；启动立即检查 + 60s 到期调度（enabled publisher、interval>0、next_due_at）；来源页手动更新/间隔预设/最近成功/最近尝试/进度/覆盖，游戏页“重新匹配”与六态/HGamer 文案；`scan_session.py` 删除；`packages/ui` 集中六态/间隔/fallback 共享函数。验收：backend 93 tests OK、Decky typecheck/61 frontend tests/build/package 全通过。
- 未验证：双端实机联调、真实网络任务、Deck 休眠恢复调度；wire `GameMatch` 无 fallback 子状态，UI 仅按顶层六态呈现。

## 2026-10-19：P5 切换和清理（自动化完成）

- Rust：删除 `sources/session.rs` 与旧 `DiscoveryRecord/Success/Attempt`、`needs_refresh`、按游戏 `discovery/` 写入、`cache/` 运行时读写；`SourceRegistry` 改为 schema-2 只读模型（index/match/detail/settings），旧 `cache/`、`discovery-index/`、`discovery/` 只由一次性迁移读取。`https`/适配器保留并改为 `collect_index` + `resolve_page` 两个入口。
- 协议/生成物：协议 v5（20 方法、4 类事件、7 项源版本）由 generator 重新生成，v4 生成物删除；旧 `refresh_sources`/`scan_sources`/`discover_game` 调用返回 `unsupported_method`（CLI/Decky/Windows/protocol fixtures 均有断言）。
- Decky：删除 `scan_session.py` 与整库 `_project_row`/`_refresh_rows` 回填、`start_game_discovery` 关闭扫描会话、旧 RPC 转发；来源工作改为独立任务流。Windows：删除旧 `refresh_sources`/`scan_sources`/`discover_game` 命令与共享 source 槽位旧语义，改为来源任务注册表。
- 验证（P5 阶段记录，P6 复核时重跑）：workspace tests、clippy -D warnings、contract-gen check、fmt、双端 typecheck/测试见 P6 段；P5 自身无新增未验证功能，仅真实网站/实机项顺延到 P6。

## 2026-10-19：P6 验收（自动化 + 真实网站 + 过度设计复核）

- 新增 `crates/core/tests/source_acceptance.rs`：11 个测试覆盖 design §7.1–§7.9（更新快照共享、共享详情、增量/重启/重复 AppID、六态与 fallback 隐藏、重匹配、更新/详情阶段分离、失败/取消/撤下/缓存年龄、提交绑定、暖读只读与请求数）。`crates/cli/src/runtime.rs` 新增 2 个测试：`scan`/`project_instances` 对已有来源状态只读；持久化 match 记录不会让已移除游戏重新出现。命令与结果：`cargo test -p steam-patch-core --test source_acceptance -- --nocapture` **11 passed**；`cargo test -p steam-patch-cli --bin steam-patch sources_tests` **23 passed**。
- 矩阵第 8 项最小绑定（core 业务逻辑唯一改动，已报告）：`update_source` 发布前重读 settings，源已停用则只存 progress 并返回 `source_not_enabled`；`matching::match_one` 保存前比较 settings 修订与重算 `input_key`，变化则丢弃本次结果并保留上次记录。测试覆盖停用/退订/无关设置变化/移除游戏；未改状态机、协议字段或生成物。
- 真实网站只读核验（结构/计数，未保存页面内容；首轮未抓详情）：Kagura CN 分页至 24 页、10 article/253 分类链接、Last-Modified+304；Kagura EN `/patch/` 228 产品卡、单页、Last-Modified+304；patch.moe 无 ETag/Last-Modified（no-store）、3 语言表单、114 ZIP。详见 [p6-acceptance.md](p6-acceptance.md) §5。真实网站遗留问题（HGamer DOM、Kagura EN 第二入口）已由 core-sources 修复并在下方“P6 真实网站修复后复核”更新。
- 过度设计复核（`overengineering-review`）：删除无调用方的 `SourceStore::run_if_revision` 与未使用的 `MATCH_CONCURRENCY`；把 Decky 已完成任务历史上限命名并补充依据（长期运行 + UI 轮询任务快照，行为不变）；其余机制（progress 续跑、传输/结构错误分流、迁移、调度、任务绑定）有契约或真实触发条件，保留并记录理由。结论：未发现其他需删除项。
- 实现偏差（已记录到 [p1-contract.md](p1-contract.md) §13 与 [design.md](design.md) §10）：`failed` 实时不可达（显示 `incomplete` + 诊断）；`SourceStatus` 不落盘 revision/complete；`DetailRecord` 存 `PageFact`；矩阵 8 新增 `source_not_enabled`；wire `GameMatch` 无 fallback 子状态。Kagura EN 第二入口与 HGamer DOM 偏差已在“P6 真实网站修复后复核”中闭环。
- 首轮最终门禁（P6 首次重跑）：`cargo test --workspace` **通过**（41 套件汇总 702 passed / 0 failed；core 单元 367 passed / 5 ignored）；`cargo clippy --workspace --all-targets -- -D warnings` **通过**（0 warning）；`cargo run -p steam-patch-contract-gen -- check` **通过**（`v5 contracts check: ok`）；`cargo fmt --all -- --check` **通过**；`npm run typecheck` **通过**；`npm test -w apps/decky` **61 passed / 0 failed**；`python -m unittest discover -s apps/decky/backend -p "test_*.py"` **98 tests OK（2 skipped）**；`npm run test:status -w apps/windows` **通过**（12 game-status + 8 auto-match + 共享六态/间隔/覆盖 + v5 生成物）；`npm run build:decky` / `npm run build:windows` **通过**；`source_acceptance` 11 passed、CLI `sources_tests` 23 passed。日志见 `.artifacts/p6-*.log`。
- 未验证/遗留：Deck/Windows 实机设置/进度/取消/调度/手柄与安装链真实回归；迁移真实用户旧状态样本；真实 Kagura 全量更新耗时/流量。**结论：自动化 + 真实网站关键链路通过、实机验收前不合并 main**（详见 p6-acceptance §9）。

## 2026-10-19：P6 真实网站修复后复核（core-sources 修复，本记录复跑核验）

- HGamer 真实站点修复（core-sources，`adapters/hgamer.rs`）：根因是新站点把 `<p>「term」找到 N 筆結果</p>` 放在 h1 的兄弟 section；`result_summary` 改为扫描含查询词且可解析计数的 `<p>`，找不到回退整页文本，不解析 RSC。新增 `crates/core/tests/hgamer_live.rs`（`#[ignore]`，只读元数据、不下载/不落盘）。本记录复跑 `cargo test -p steam-patch-core --test hgamer_live -- --ignored --nocapture` **通过**：appid 1250180 为 3 逻辑请求、status=complete、candidates=0、identities=1、diagnostics=[]（完整无结果）；appid 1227890 为 5 逻辑请求、complete、candidates=2、identities=1，两候选 `match_basis=appid`、mirror 为 kagurafan（web）；耗时 4.1s/6.1s（含 1s/host 节流）。原“HGamer 阻塞/需产品决策”结论作废。
- Kagura EN 双入口修复（core-sources，`adapters/kagura.rs`）：无 progress 时从 `rule.url` + `rule.pages` 全部入口起爬（canonical 去重），每个入口各自 `next_page` 分页；新增 fixture `kagura_en_crawls_every_configured_entry_root`（3 列表页、4 条目、complete=true、不抓详情）。本记录复跑该测试 **通过**；真实 Kagura 全量更新仍未执行。
- 修复后复核门禁（Rust；宿主/协议/生成物未变，沿用首轮结果）：`cargo test --workspace` **通过**（42 套件汇总 **705 passed / 0 failed**；core 单元 369 passed / 5 ignored，含新增 2 项 HGamer 单测）；`cargo clippy --workspace --all-targets -- -D warnings` **通过**；`cargo run -p steam-patch-contract-gen -- check` **通过**（`v5 contracts check: ok`）；`cargo fmt --all -- --check` **通过**；`hgamer_live`（ignored）**通过**；`kagura_en_crawls_every_configured_entry_root` **通过**。日志见 `.artifacts/p6-*-after-fix.log`、`.artifacts/p6-hgamer-live.log`。
- 更新结论：自动化矩阵 + 真实网站关键链路（HGamer live、Kagura 列表结构与多入口）通过；剩余未验证为双端实机矩阵、安装链真机回归、真实 Kagura 全量更新耗时/流量、真实旧状态迁移。**建议实机验收后再合并 main（除非用户明确要求）**。
