# 来源缓存与游戏匹配重构施工记录

- 设计：[design.md](design.md)。
- 当前状态：P0 基线/契约、P1 数据/存储/迁移、P2 按源更新、P3 增量匹配、P4 双端与自动更新、P5 切换清理均已实现并通过自动化；P6 自动化验收与过度设计复核完成，真实网站关键链路（HGamer live、Kagura 列表结构与 EN 多入口）已通过；Steam Deck 编译部署与 RPC/真实界面冒烟已通过（2026-10-19）；Windows 实机、完整实机矩阵与安装链真实回归仍待验证，**实机验收前不合并 main**。P6 证据：[p6-acceptance.md](p6-acceptance.md)。
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

## 2026-10-19：编译并部署至 Steam Deck

- 依据用户“计划已经完成，编译并部署至 steamdeck”的指示，在 `feature/source-cache-matching-refactor`（`431ec3c8bf1edd7d31f0bf61ad72fd45a9b38415`，部署前工作区干净）构建；沿用已有自动化验证，不重复运行测试。不合并、不提交、不发布版本。
- 当前 Devkit 选中设备 `steamdeck`；本轮 SSH 使用当前配对密钥与已有可信主机记录，独立保存 known_hosts。部署前 `list_tasks` 无非终态任务，插件目录 root 所有，Loader 为 v3.2.10。
- 构建：`npm run build:decky` 通过；当前源码归档上传隔离验证目录，设备已有 Rust 1.94.0 bookworm 容器执行 `cargo build --release --locked -p steam-patch-cli -p steam-patch-installer-worker --bin steam-patch --bin sf-worker` 通过（40.68s）。取回本轮 Linux ELF 至 `target/linux/release/`，`npm run package:plugin -w apps/decky` 通过；从当前 Cargo metadata 与锁文件生成 SBOM（749 components，无 license metadata missing），`scripts/package-artifacts.py --platform decky` 通过。
- 完整包 23 文件、7,182,341 字节；ZIP SHA-256：`099bda3579610a05f37b08815e0f6a9523b529b81366fe4d6b7a432a02a3c937`。上传后逐成员大小/哈希与整体 ZIP 哈希一致；通过 Steam 已有 Decky 连接调用 Loader 本地 ZIP 重装，严格核对安装提示 name/version/hash/type/request ID，再确认安装。
- 备份：完整旧插件、旧状态、Loader/本插件设置、逐文件清单及可安装 rollback.zip 已取回本地并核对 SHA-256。证据与回滚材料存放忽略目录 `.artifacts/source-cache-matching-deck-20261019/`，其中 `backup/` 保留首次备份。
- 部署核验：正式 23 文件全部匹配，CLI/worker 有执行位，runtime manifest 为 protocol 5 / worker protocol 1；Loader 服务 active、插件实例严格 1。实际认证 HTTP bundle SHA 与本轮构建/正式文件一致：`28752f0ff6342aa09e73b4f554e138a0df2c137de005c9a6f59f8f745d6e3772`。
- RPC：新版 `scan()` 返回 28 个真实游戏实例，`list_sources` 返回 4 个来源及 revision/complete/interval_hours 等 v5 字段，`source_task_status` 正常，`project_instances` 返回单实例投影，`list_transactions` 可读 23 条历史事务。最终安装/来源活动任务均为空。
- 真实旧状态迁移冒烟：原文件无缺失；原有状态文件仅 `sources/rust-v1/schema.json` 与 `settings.json` 哈希改变，其余原文件（含事务/收据）保持一致；已保存来源索引和匹配结果可读。这是该设备现有样本验证，不代表全部旧状态组合已验收。
- 真实界面：QAM 显示 7 款“有补丁”游戏；全屏来源页显示索引条目/修订、完整性、更新时间、自动更新关闭、更新间隔与手动更新按钮；HGamer 不显示定时更新设置。来源页顶部/底部与 VenusBlood FRONTIER International 游戏页已截图并检查，游戏页显示 HGamer 命中和“重新匹配”入口；DOM 滚动检查无横向溢出。未点击安装或实际来源更新/重匹配按钮。
- 收尾：Loader 安装改变的本插件排序记录已恢复原有成员关系，其他设置不回写；已复核恢复结果。关闭安装提示和本次菜单，返回 Steam 首页。远端本轮暂存目录清理与隧道关闭结果见同目录 cleanup.json。
- 设备诊断：扫描报告两个既存 Steam 库挂载路径不可用（`library_unavailable`）；当前可访问库扫描正常，未修改挂载配置。来源列表保留 HGamer 旧的 `source_refresh_failed` 尝试状态，本轮未发起来源网络刷新，不据此声称真实来源更新通过。
- 工具限制与已复核情况：沙箱启动被 Z: 映射盘凭据错误阻断，使用获准的沙箱外执行通道完成；一次未认证的 HTTP bundle CDP fetch 超时，改用设备 loopback 认证请求核对成功；恢复设置并导航时 CDP Promise collected，之后独立读取确认设置已恢复且页面回到首页。
- 验收边界：本轮完成编译、完整部署、文件/HTTP/RPC 与真实界面冒烟。实体手柄、来源真实更新/取消/调度/休眠、Windows 实机、完整迁移组合及安装链真实回归仍待验证，未将本轮部署等同于 P6 全矩阵验收，不合并 main。

## 2026-10-19：新增子 feature 计划——可折叠订阅源卡片

- 根据用户新增的页面分界与折叠需求，建立 [子 feature 设计](collapsible-source-cards/design.md) 与 [子施工记录](collapsible-source-cards/progress.md)，沿用当前父分支。
- 子项当前待实现；已列明产品契约、技术方案、分阶段实施及自动化/实机验收矩阵，并完成静态过度设计自审。
- 本轮仅编写计划与索引，未修改插件代码、未测试、未构建、未操作设备。父功能之前的部署结果不作为本子项通过依据。


## 2026-10-19：可折叠订阅源卡片子 feature 实施

- 子项 [设计](collapsible-source-cards/design.md) / [施工记录](collapsible-source-cards/progress.md)：C1–C3 已实现，前端 67 tests、typecheck/build/package 通过，过度设计静态自审完成。
- 已部署 Steam Deck，文件/HTTP/RPC 与单插件实例验证通过；真实页面确认默认收起、独立展开、缓存页往返保留、aria/DOM 焦点和 Enter/Space，长 URL/QAM 无溢出。首次备份与证据保留，设备暂存及隧道清理完成。
- Loader 重装期间前端导入超时，按技能确认空闲后重启 Steam JS 上下文恢复，最终状态已独立复核。详细过程与证据见子施工记录。
- 子项实体手柄、真实任务进度/取消仍待验收；未启动来源刷新或补丁安装，未提交、未合并。父功能其他遗留验收边界保持不变。

## 2026-10-19：VenusBlood FRONTIER 漏匹配与详情异常修复

- 专项记录：[vbf-matching-fix.md](vbf-matching-fix.md)。用户授权直接连接当前 Devkit 所选 Steam Deck 查看，设备 Loader 当前为 v3.3.0，插件实例可读，取证时安装任务全为终态、来源任务为空。
- 根因 1：真实 Loader 日志中 rematch_game 抛出 instance_required；session 同时传入 instance_id 与 appid，违反 Python SourceService 的互斥契约。修复为按安装实例选择，RPC 的 appid 传 null；重新发起来源任务时清除前次错误提示，新增失败后重试回归。
- 根因 2：设备 Kagura CN revision=1、complete=true 已有正确分类和 Venus Blood Frontier 文章；Steam 名称为 VenusBlood FRONTIER International。旧 HGamer 结果已确认 AppID 1189440 并给出该文章精确 upstream_page，matching 未使用该关联。修复为在已发布索引内按已确认 AppID 的精确上游页面定位，按既有详情流程解析，生成 verified_alias 候选并记录 verified_upstream 证据、保留 HGamer provenance。普通来源命中后隐藏 HGamer；没有放宽标题比较，也没有全站刷新或未入索引的上游遍历。
- 回归：source_matching **24 passed**（新增 3 项，包含旧 fallback、首次新补查、错误 AppID/未入索引页面、暖缓存、失败详情不重复业务请求）；source_acceptance **11 passed**；核心 lib/匹配测试 clippy -D warnings 通过；cargo fmt 通过；Decky **68 passed**、typecheck、build 通过；Linux release CLI 构建通过。未重复父功能完整门禁，宿主协议/生成物/Python 未因本修复改变。
- 真实设备隔离对比：分别复制正式 sources 状态到本轮隔离目录，旧/新 CLI 使用同一设备安装清单、来源索引与真实网站元数据；旧版为 fallback_match，仅 HGamer 网页候选；新版为 publisher_match，Kagura CN / AppID 1189440 / verified_alias，解析出中文 v5 的两条 EXE 下载镜像，diagnostics=[]。最终新 CLI SHA-256 为 0c3ac76ecc756336176e439c98440bec62bdf2f0c8f86804c0c7a90ff0e99d77。未下载或安装补丁，未替换正式插件，未改变正式匹配状态。
- 过度设计实现复核：只保留精确上游 URL、当前 AppID 与本体/DLC 一致的已知身份关联；新补查仅补充新增关联，不因同一旧证据再试失败详情。沿用原 HTTP 传输重试、索引、详情缓存、候选与六态，无新配置/状态/限制，未发现其他需修改项。
- 环境：Windows 沙箱启动被 Z: 凭据错误阻断，获准使用沙箱外命令；本机无 cargo，使用设备已有 Rust 1.94 bookworm 容器及编译缓存。临时容器内添加 rustfmt/clippy，未向 SteamOS 系统安装工具。一次测试夹具 503 触发既有传输重试，改用 404 隔离业务层重复解析断言，最终通过。
- 验收边界：代码和构建已完成；真实设备仅隔离状态/CLI 验证。正式插件部署、真实页面按钮回归、实体手柄与安装链未执行。无 Git 提交或合并，保留本轮开始已有的未提交缓存清理/卡片改动。证据、本轮 Linux CLI、原始对比响应存于忽略目录 .artifacts/vbf-matching-bug-20261019/。
- 收尾：最终正式 CLI 哈希与取证基线一致，正式 1189440 匹配记录 SHA-256 与本地取证副本完全一致。隔离证据归档 evidence.tar.gz 的本地/远端 SHA-256 均为 b03b76131f76cb79246468dfa671b3b97e7b47e20dbea96910b169b824769e5e；已取回并核对。已清理本轮远端隔离目录和两个停止的临时容器，关闭本轮 CEF SSH 隧道；保留原有共享构建缓存及其他验证目录。
