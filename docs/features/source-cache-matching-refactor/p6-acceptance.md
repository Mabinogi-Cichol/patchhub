# P6 验收证据（自动化矩阵、真实网站、过度设计复核）

- 状态：P6 自动化验收完成；真实网站关键链路（Kagura 列表结构/多入口、HGamer live 搜索→身份→补丁详情）已完成并通过；双端实机、真实 Kagura 全量更新耗时/流量、真实旧状态迁移未执行。**实机验收前不合并 main**，理由见 §9。
- 修订记录：2026-10-19 真实网站修复后复核（HGamer `result_summary` 最小修复 + Kagura EN 多入口起爬），本节与 §2/§3/§4/§5/§6/§7/§8/§9 已按实测结果更新；原 HGamer“结构不匹配/阻塞”结论作废。
- 上级设计：[design.md](design.md)；施工记录：[progress.md](progress.md)；P0 基线：[p0-baseline.md](p0-baseline.md)；冻结契约：[p1-contract.md](p1-contract.md)。
- 分支：`feature/source-cache-matching-refactor`（非提交状态；HEAD `353548e`，P1–P5 改动均在工作区）。
- 测量/核验日期：2026-10-19（Asia/Shanghai）。
- 环境：Windows 11 `10.0.26300`、Rust `1.94.0`（`x86_64-pc-windows-msvc`）、Cargo `1.94.0`；工具链隔离在 `.artifacts/cargo-isolated/{cargo,rustup}`。

## 1. 新增/修改的验证代码

| 文件 | 内容 |
| --- | --- |
| `crates/core/tests/source_acceptance.rs` | **新增**，11 个测试，矩阵 §7.1–§7.9 的固定响应、请求轨迹与状态断言；`FakeFetcher` + 临时 state，另含可注入设置变更的 `HookedFetcher` |
| `crates/core/src/sources/update.rs` | **矩阵 8 最小绑定**：完整采集后、发布正式索引前重读持久化 settings；源已停用则保存为 progress 并返回 `source_not_enabled`，不发布索引/成功状态 |
| `crates/core/src/sources/matching.rs` | **矩阵 8 最小绑定**：`match_one` 开始时记录持久化 settings 修订，保存 `matches/<appid>.json` 前若修订变化且该游戏的 `input_key`（含启用/订阅快照）变化，则丢弃本次结果、保留上次已提交记录（无记录则保持 `pending`） |
| `crates/cli/src/runtime.rs` | 新增 2 个 CLI 测试：`scan`/`project_instances` 对已有来源状态只读；已持久化 match 记录不会让已移除游戏重新出现 |
| `apps/decky/backend/source_service.py` | 过度设计复核后把保留历史任务上限命名并说明依据（行为不变） |
| `crates/core/src/sources/store.rs` / `matching.rs` | 过度设计复核删除：未使用的 `SourceStore::run_if_revision`、未使用的 `MATCH_CONCURRENCY` |
| `crates/core/src/sources/adapters/hgamer.rs` | 真实网站修复（非本轮 P6 绑定改动，由 core-sources 完成）：`result_summary` 扫描含查询词且可解析计数的 `<p>`（当前站点把 `<p>「term」找到 N 筆結果</p>` 放在 h1 兄弟 section），找不到时回退整页文本；不解析 RSC |
| `crates/core/src/sources/adapters/kagura.rs` | 真实网站修复：`collect_index` 无 progress 时从 `rule.url` + `rule.pages` 全部入口起爬（canonical 去重），每个入口各自沿 `next_page` 分页 |
| `crates/core/tests/hgamer_live.rs` | **新增 live `#[ignore]` 测试**：`cargo test -p steam-patch-core --test hgamer_live -- --ignored --nocapture`；对真实 HGamer 执行搜索→身份→补丁详情，仅请求列表/详情元数据，不下载、不落盘 |
| `crates/core/tests/source_update.rs` | 新增 fixture 测试 `kagura_en_crawls_every_configured_entry_root`（双入口 + 第二入口分页；3 列表页、4 条目、complete=true、不抓详情） |

运行命令：

```powershell
cargo test -p steam-patch-core --test source_acceptance -- --nocapture --test-threads=1
cargo test -p steam-patch-cli --bin steam-patch sources_tests
cargo test -p steam-patch-core --test hgamer_live -- --ignored --nocapture --test-threads=1
```

结果：`source_acceptance` **11 passed / 0 failed**；CLI `sources_tests` **23 passed / 0 failed**；live HGamer **通过**（两次真实查询，见 §3/§5）。完整 stdout 见 `.artifacts/p6-acceptance.log`、`.artifacts/p6-hgamer-live.log`。

## 2. 验收矩阵逐项证据

| # | 设计 §7 要求 | 自动化证据（测试） | 关键实测数字 | 状态 |
| --- | --- | --- | --- | --- |
| 1 | 普通源初始化一次后 1/10/多游戏共享快照；新增游戏不新增索引请求，只取命中详情；已有详情复用 | `matrix1_one_update_snapshot_is_shared_and_index_requests_stay_constant` | `update_source` 1 请求（仅列表页）；1 游戏 1 详情请求；10 游戏 9 请求（首游戏已缓存）；重启后 0 请求；10 游戏存在时再次更新仍 1 请求、revision 不变 | 已自动化 |
| 2 | 两游戏同详情页同批只 1 请求；游戏数不放大索引遍历 | `matrix2_two_games_share_a_detail_page_and_requests_follow_hit_pages` | 10 游戏命中 3 个不同详情页 → 3 请求（每 URL 1 次）；匹配不重写索引文件 | 已自动化 |
| 3 | 新游戏只匹配自身；重启保留；同 AppID 多实例不重复 HGamer，实例状态独立 | `matrix3_new_game_matches_itself_and_duplicate_appids_query_fallback_once` | 新增 1 游戏 = 1 请求（仅新详情）；重启 0；同 AppID 两实例 = 1 次 fallback 查询、1 条匹配结果（instance_id 取首个实例） | 已自动化 |
| 4 | 普通源命中 / 仅 HGamer / 完整无命中 / 源未初始化 / 部分更新 / 网络失败 / HGamer 禁用显示正确；普通源后来命中隐藏 HGamer | `matrix4_six_states_and_disabled_or_incomplete_semantics`、`matrix4_a_later_publisher_match_hides_the_previous_fallback_candidates`；真实链路 `hgamer_live` | publisher_match 0 网络；fallback 命中 2 请求；完整无命中 1 请求；partial 覆盖 0 请求且 `not_queried`；禁用 HGamer → `no_match` + `disabled` 且 0 请求；网络失败 → `incomplete` + 2 条诊断（偏差见 §4）；迁移记录 `failed` 正确显示；普通源后命中 → `publisher_match` 且旧 HGamer 候选不再显示。真实 HGamer：3/5 逻辑请求、两次 status=complete、identities=1、diagnostics=[]；完整无结果与 2 候选两种结果均符合语义 | 已自动化（真实 HGamer live 已修复并通过） |
| 5 | 可重匹配已命中/无补丁；不更新索引；刷新命中详情；确认未命中可重查 HGamer；其他来源任务不被取消 | `matrix5_rematch_refreshes_hits_and_rechecks_fallback_without_touching_index` | 命中重匹配 1 详情请求，索引文件字节不变；无补丁重匹配 2 请求（搜索+详情）；`rematch_source` 0 请求、目标源 revision 2、其他源记录/索引原样 | 已自动化 |
| 6 | 指定源更新只访问该源列表；详情补充是后续阶段；关闭/到期/间隔/重启/失败/重复点击无补跑风暴 | `matrix6_update_is_list_only_and_match_details_are_a_separate_phase`；`kagura_en_crawls_every_configured_entry_root`；宿主调度测试（`apps/decky/backend/test_scheduler.py`、Windows `select_due_sources` 测试）；核心 `schedule.rs` 单测 | 更新轨迹仅 `https://kagurafan.com/`（无 `/article/`、`/category/`）；匹配轨迹仅详情页；Kagura EN 双入口：3 列表页/4 条目/complete=true、不抓详情；新修订重算只覆盖调用方传入的当前库/订阅游戏（未传入游戏保留旧 revision） | 已自动化 + 真实列表结构核验；真实 Kagura 全量更新耗时未跑；实机休眠恢复未验证 |
| 7 | 失败/取消保留最后成功快照；完整更新反映撤下条目；缓存年龄不阻止匹配/人工选择；迁移覆盖旧数据 | `matrix7_failure_cancel_removal_and_cache_age`；迁移见 `crates/core/tests/source_data_v2.rs` 9 项 | 传输失败 → `source_offline_cached`，索引字节不变（revision 1/entries 6）；取消 → `task_cancelled`，旧快照不变，progress 保留 2 条；撤下条目 → revision 2、entries 1；`checked_at=1` 的旧索引+旧详情仍 0 请求命中且 `dynamic_zip_candidates` 非空 | 已自动化（真实用户旧状态样本未验证） |
| 8 | 更新/匹配进行中停用源、退订、移除游戏后旧结果不回写；与安装并行无锁冲突/输入保持绑定 | `matrix8_update_does_not_publish_when_source_is_disabled_mid_task`、`matrix8_match_discards_a_result_when_settings_changed_mid_task`；CLI `scan_does_not_reintroduce_a_removed_game_from_a_persisted_match`；安装并行/内容绑定沿用既有安装契约测试 | 更新中停用：3 请求后返回 `source_not_enabled`，无正式索引/成功状态，progress 保留 6 条；首次匹配中停用：结果 `pending` 且不落盘；重匹配中退订：`matches/<appid>.json` 字节不变、返回上次 `publisher_match`；无关设置（他源 interval）变化仍正常落盘；移除游戏后 scan 不因旧 match 记录重新出现 | 已自动化（新增最小绑定）；安装并行实机未验证 |
| 9 | 匹配更新不触发整库 scan/全库哈希；记录冷/暖/新游戏/重匹配/来源更新请求数；单行投影不整库哈希 | `matrix9_warm_match_is_offline_and_reads_do_not_write_state`；CLI `scan_and_project_instances_do_not_modify_existing_source_state`；既有 CLI `scan_reads_the_persisted_match_summary` / `project_instances_projects_requested_rows_and_reports_missing_instances` | 暖匹配 0 请求；`scan`/`project_instances` 前后状态目录文件哈希完全一致（6 文件）；`scan` 路径仅 1 次 `steam::scan`（`crates/cli/src/runtime.rs:526` 静态核对），单行投影走 `project_batch`（`runtime.rs:649-658`），仅对请求实例做投影/动态检测哈希 | 已自动化 + 静态核对；未测耗时 |
| 10 | Windows/Decky 真实设置、进度、取消、离线、重启、手柄；安装链回归 | 宿主单元/组件测试见 §7；实机未执行 | — | **待实机** |

## 3. 新流程请求数/扫描数记录

fixture 驱动场景（FakeFetcher）：

| 场景 | HTTP 请求数 | 扫描次数/说明 |
| --- | --- | --- |
| `update_source`（Kagura CN 单列表页，无游戏参与） | 1 | 0 次 Steam 扫描（更新不读游戏库） |
| `update_source`（patch.moe：入口 + 2 语言页） | 3 | 同上 |
| `update_source`（Kagura EN 双入口 fixture：入口1 + 入口2 + 入口2 分页） | 3 | 同上；complete=true、4 条目、不抓详情 |
| 匹配 1 个命中游戏（详情未缓存） | 1（详情） | 0 网络索引遍历 |
| 匹配 10 个命中游戏（首游戏已缓存） | 9 | 0 索引请求 |
| 匹配 10 个游戏命中 3 个不同详情页 | 3 | 每 URL 1 次 |
| 暖匹配 / 重启后匹配 | 0 | 仅读持久化记录与共享详情 |
| 重匹配已命中游戏 | 1（详情） | 索引 0 请求 |
| 重匹配无补丁游戏 | 2（搜索+补丁详情） | 索引 0 请求 |
| `rematch_source`（entry 来源） | 0 | 索引文件不写 |
| 来源更新中停用源（3 请求后） | 3 | 不发布 |
| CLI `scan` | 0（无来源网络） | `steam::scan` 1 次；`projection::scan` 整库投影；来源快照为只读 |
| CLI `project_instances` | 0 | `steam::scan` 1 次；`projection::project_batch` 仅请求实例；动态检测只检查这些实例的变更文件 |

真实网络（HGamer `hgamer_live`，逻辑请求数 = `RequestContext.visited`，含每 host 1s 节流）：

| 案例 | 逻辑请求数 | 请求轨迹 | 结果 |
| --- | --- | --- | --- |
| appid 1250180 `Re:從H開始的異世界生活` | 3（2 搜索 + 1 游戏详情） | `/zh-CN/search?q=...`（zh-CN 与 zh-TW 词形）→ `/zh-TW/games/runaway-demon-bride` | status=complete、candidates=0、identities=1、diagnostics=[]（有游戏条目但未链接补丁 = 完整无结果） |
| appid 1227890 `夏日狂想曲：乡间的难忘回忆` | 5（2 搜索 + 1 游戏详情 + 2 补丁详情） | 2 搜索 → `/zh-TW/games/summermemories` → `/zh-TW/patches/summer-memories`、`/zh-TW/patches/summer-memories-dlc` | status=complete、candidates=2、identities=1、diagnostics=[]；两候选 `match_basis=appid`、`game_appid=1227890`，mirror 为 kagurafan 页面（`web`） |

本机复跑耗时（含 1s/host 节流）：`elapsed_ms=4121` 与 `6096`；这是目前唯一记录到的真实 HGamer 端到端耗时，不能外推为全部来源的更新耗时。

## 4. 实现与设计的偏差（需 Lead 复核，已按现状记录）

1. **`failed` 六态在实时匹配中不可达**：p1-contract §7.1 第 5 条“全部失败 → failed”。当前 `derive_game_status`（`sources/match.rs:66-119`）在“普通源详情失败 + fallback 失败”时给出 `incomplete`（来源状态 `detail_incomplete`），因为来源失败被建模为“未完成”而非“失败”；`failed` 只由迁移记录（`SourceMatchStatus::DetailFailed` + `FallbackMatchStatus::Failed`）产生。P6 按现有行为断言，未改状态机（超出本轮允许改动范围）。需产品决定：接受 `incomplete` 语义并修订契约，或后续单列修复。
2. **`SourceStatus` 落盘形状**：契约 §4 期望 status 含 `revision/complete`；实现持久化 `checked_at/cached_at/entry_count/code`，`revision/complete` 由 `SourceIndex` 得出并经 `SourceSummary`/`update_source` 结果暴露（`model.rs:261-268`、`registry.rs:205-254`）。语义一致，字段位置不同。
3. **`DetailRecord` 形状**：契约 §6 写 `candidates/diagnostics`；实现按“索引与详情分层”存 `fact: PageFact`（query 无关的页面事实）+ `index_revision/complete`，候选在匹配时由 `identity::build_candidates` 生成（`model.rs:644-700`、`matching.rs:1042-1148`）。这是 2026-10-19 §9 修订的实现落点。
4. **新增 `source_not_enabled` 错误码**：矩阵 8 绑定时，来源在网络期间被停用则更新任务返回该错误（不发布索引）；UI 已有 `source_not_enabled` 文案（`packages/ui/src/index.ts:111`），不是新协议字段。
5. **Kagura 海外补充入口（已修复）**：原实现 `collect_index` 只读取 `pages.first()`，未消费 `resources/sources.json` 中 `kagura-en.pages` 的第二入口 `product-category/patches/patch/`。修复后无 progress 时从 `rule.url` + `rule.pages` 全部入口起爬（canonical 去重），每个入口各自沿 `next_page` 分页；fixture 测试 `kagura_en_crawls_every_configured_entry_root` 断言 3 个列表页、4 个条目、complete=true、不抓详情。真实站点全量更新仍未执行（成本/耗时见 §8）。
6. **HGamer 真实站点 DOM 契约（已修复）**：原适配器要求 h1 父节点文本含查询词与“找到 N 笔”计数，与当前站点不符。修复后 `result_summary` 扫描含查询词且可解析计数的 `<p>`（当前站点把 `<p>「term」找到 N 筆結果</p>` 放在 h1 兄弟 section），找不到时回退整页文本；不解析 RSC/客户端脚本。真实网络 live 测试已通过（§3/§5），原“HGamer 阻塞/需产品决策”结论作废。
7. **`GameMatch` wire 无 fallback 子状态**：宿主只显示顶层六态；HGamer 的 `disabled/not_queried/failed` 不随游戏行下发（进度记录已记，P6 未改协议）。

## 5. 真实网站只读核验（2026-10-19，结构/计数，不保存页面内容）

方法：`GET` 列表/搜索页（UA `SteamPatchDiscovery/1`），仅在内存统计标签/链接/分页/头部；对提供校验器的页面用 `If-Modified-Since` 复核 304。后续 HGamer live 测试会请求搜索/游戏/补丁**元数据详情页**（与原生产 fallback 相同），但**不下载任何制品、不保存页面内容**。Http 请求计数为逻辑请求（`RequestContext.visited`）。

| 站点 | HTTP | 校验器/条件请求 | 列表结构实测 | AppID | DLC/分类入口 | 结论 |
| --- | --- | --- | --- | --- | --- | --- |
| Kagura CN `https://kagurafan.com/` | 200 | `Last-Modified`（2026-10-18 15:40:50 GMT）、`Cache-Control: max-age=0`；条件 GET 返回 **304** | 10 个 `article`/`entry-title`；253 个 `/category/` 链接；分页链接含 `/page/2/` 与 `/page/24/`，`class=next` 1 个且 href=`/page/2/`；无 `rel=next`；3 个 `.zip` 链接、3 个 `DLC` 相关词 | 列表页 0 个 Steam app 链接 → **不提供 AppID** | 有分类入口（253 链接）；DLC 仅能从标题文本判断 | 适配器要素齐备；**真实更新至少 24 个列表页请求**（首页 + 2..24），首次全量成本远高于 fixture |
| Kagura EN `https://www.kaguragames.com/patch/` | 200 | `Last-Modified`（2026-10-16 15:12:53 GMT）；条件 GET **304** | 228 个 `product type-product` 卡/228 个产品标题；无分页链接（`rel=next`/`/page/N` 均 0）；4 个 form；32 个 DLC 相关词 | 0 个 Steam app 链接 → **不提供 AppID** | 产品卡/分类页存在；DLC 需详情页/标题判断 | 适配器要素齐备；**单页即 228 条，无分页**，一次更新 1 个请求（不含详情） |
| Kagura EN 补充 `https://www.kaguragames.com/product-category/patches/patch/` | 200 | `Last-Modified`（2026-10-18 18:37:24 GMT） | 每页 21 卡；分页 2/3/4/9/10/11 页（`rel=next` 存在） | 0 | 分类目录 | **已消费（修复后）**：`collect_index` 无 progress 时从 `rule.url` + `rule.pages` 全部入口起爬（canonical 去重），每个入口各自分页；fixture `kagura_en_crawls_every_configured_entry_root` 验证 3 列表页/4 条目/complete=true/不抓详情。真实全量更新与两入口合并后的最终条目数未跑 |
| patch.moe `https://www.patch.moe/?flag=1` | 200 | **无 ETag/Last-Modified**，`Cache-Control: no-store, no-cache, must-revalidate` → **不支持条件请求** | 3 个 `form_en/form_zh_cn/form_zh_tw` 语言表单；116 行 `tr`、114 个 `.zip` 链接、126 个 DLC 链接标签；无分页 | 0 个 Steam app 链接 → **不提供 AppID** | 语言表单 + 分类列（列表只有 title/release/patch） | 适配器要素齐备（3 请求/次全量）；无变更信号 → 每次完整重读 |
| HGamer `https://hgamer.tw/zh-CN/search?q=test` | 308 → `/zh-TW/search?q=test` → 200 | 无 ETag/Last-Modified；`Cache-Control: private, no-store` | 158 KB HTML：`self.__next_f.push` **39** 次（Next.js RSC/客户端渲染）；`h2` 3 个、分组 2 个；`/zh-TW/games/` 10 链接、`/zh-TW/patches/` 16 链接；无分页、无分类。站点把 `<p>「term」找到 N 筆結果</p>` 放在 h1 兄弟 section，而非 h1 父节点 | 搜索列表 HTML 0 个 Steam app 链接（身份在游戏详情页） | 无分类/分页入口 | **结构已适配（修复后）**：`result_summary` 扫描含查询词且可解析计数的 `<p>`，找不到回退整页文本；不解析 RSC。真实 live 结果见 §3（3/5 逻辑请求、complete、identities=1、diagnostics=[]） |
| HGamer live 1250180 | 200（308 跳转后） | 无校验器 | 2 搜索 + 1 游戏详情；`status=complete`、`candidates=0`、`identities=1`、`diagnostics=[]` | 游戏详情提供 Steam 身份 | — | **通过**：HGamer 有游戏条目但未链接补丁 = 完整无结果，不是结构失败 |
| HGamer live 1227890 | 200（308 跳转后） | 无校验器 | 2 搜索 + 1 游戏详情 + 2 补丁详情；`status=complete`、`candidates=2`、`identities=1`、`diagnostics=[]`；两候选 `match_basis=appid`、`game_appid=1227890`、mirror 指向 kagurafan（`web`） | 身份来自游戏详情 | 补丁详情经镜像链接给出上游页面 | **通过**：搜索→身份→补丁详情真实链路跑通 |
| HGamer `https://hgamer.tw/zh-CN/patches` | 200（737 KB） | 无校验器 | 未做逐项统计（非索引入口，仅确认可达） | — | — | 可达 |

补充观察：

- Kagura 两站的 `Last-Modified` + 304 证明**列表页支持条件请求**；当前 PG 实现只把 etag/modified 用于表格源（patch.moe/Kagura 走 `collect_index`，`IndexOutcome` 的 etag/modified 为 `None`），**未利用 Kagura 的条件请求**（`update.rs` 会把它用于表格源；P6 未改）。这与 design §4.1“无可靠信号时重新读取”一致，但属于可优化项而非缺陷。
- patch.moe 无校验器：每次更新 3 个请求（入口+2 语言表单），与 P0 基线一致。
- HGamer 根因与修复：新站点把搜索摘要 `<p>「term」找到 N 筆結果</p>` 放在 h1 的兄弟 section；`hgamer.rs::result_summary` 改为先扫描含查询词的 `<p>`（可解析计数），再回退整页文本，不解析 RSC。live 测试两次真实查询均 `complete` + `identities=1` + 空 diagnostics，分别覆盖“完整无结果”和“2 个 appid 候选 + web 镜像”两种结果。
- Kagura EN 双入口已由 `collect_index` 消费并通过 fixture 验证；但“两个入口合并后的真实条目数/是否全集”和“真实 Kagura 全量更新的页数/耗时/流量”仍未执行，不能写成已完整覆盖。
- 本次核验与 live 测试均未保存页面内容、未下载制品；live 请求属于生产 fallback 的元数据链路，不代表安装或下载可用性。

## 6. 过度设计复核（按 `overengineering-review` 技能执行）

审阅对象：本轮 P1–P5 新增的来源索引/详情缓存/匹配/调度/任务绑定机制及双端宿主适配。基线：工作区改动 + 现有测试；只读代码 + 已运行测试，未做实机。结论：**发现 2 项无依据代码已删除，1 项硬限制补充依据，未发现其他需删除的机制**。

已删除：

1. `SourceStore::run_if_revision`（`crates/core/src/sources/store.rs`）：无任何调用方；P1 为“全局 settings revision 否决发布”预留的泛化原语。P6 矩阵 8 需要的是“重读 settings 后判断源是否仍启用 / 该游戏 input_key 是否变化”，用全局 revision 会把他源 interval 等他源变化误判为丢弃（阻断正常结果），因此改用 `load_settings` + `input_key` 的定向绑定；未使用的泛化原语直接删除。
2. `MATCH_CONCURRENCY`（`crates/core/src/sources/matching.rs`）：仅注释引用、无代码使用；串行执行是当前实现的直接形态，不需要一个未消费的常量。删除并把模块文档改为直接描述。

保留并补充依据：

3. `apps/decky/backend/source_service.py` 的已完成任务历史上限（原硬编码 `64`）：真实场景是插件长期运行、前端 `source_task_status()` 轮询全部任务快照，快照可携带候选列表；不设上限会无界增长。改为命名常量 `RETAINED_FINISHED_TASKS` 并注明依据，行为不变。Windows 侧对应 `RETAINED_FINISHED_TASKS = 16` 已有同类注释，保留。
4. 失败/取消保存 progress、传输错误与结构错误分流、迁移失败保留旧文件、来源更新期间不持写锁、详情/索引 schema 版本校验：均有契约条款或已复现故障语义（部分更新不能冒充完整、坏页面不能删旧缓存、旧数据迁移不能丢身份），保留。
5. 双端调度（60s tick、`next_due_at` 派生、活动任务去重、启动补一次）：对应 design §3.1/§10 的真实需求（不补跑、不立刻重试、同源不叠加），保留。
6. 未新增通用爬虫/队列/重试框架、无限重试、缓存年龄禁令或按耗时的硬截断；HTTP 每请求 2 次尝试沿用既有策略（真实网络错误语义），未扩大。

待核实（不影响本轮机制删留判断）：真实 Kagura 全量更新的页数/耗时/流量；真实用户旧状态迁移样本。原 HGamer DOM 契约与 Kagura 多入口两个 P6 真实网站发现已闭环（§4.5/§4.6、§5），不再列为待核实。

## 7. 完整验证命令与结果

首轮 P6（矩阵 8 绑定 + 复核删除 + 文档改动）与真实网站修复后均已重跑；下表为**真实网站修复后最终代码**的 Rust 门禁与受影响测试结果：

| 命令 | 结果 |
| --- | --- |
| `cargo test --workspace` | **通过**：42 个测试套件汇总 **705 passed / 0 failed**（core 单元 369 passed / 5 ignored；`hgamer_live` 为 ignored live 测试） |
| `cargo clippy --workspace --all-targets -- -D warnings` | **通过**（0 warning） |
| `cargo run -p steam-patch-contract-gen -- check` | **通过**：`v5 contracts check: ok` |
| `cargo fmt --all -- --check` | **通过** |
| `cargo test -p steam-patch-core --test source_acceptance -- --nocapture --test-threads=1` | **通过**：11 passed；输出见 `.artifacts/p6-acceptance.log` |
| `cargo test -p steam-patch-cli --bin steam-patch sources_tests` | **通过**：23 passed（含新增 2 项） |
| `cargo test -p steam-patch-core --test source_update kagura_en_crawls_every_configured_entry_root` | **通过**：1 passed（Kagura EN 双入口 fixture） |
| `cargo test -p steam-patch-core --lib hgamer::tests::result_summary` | **通过**：2 passed（兄弟 section 摘要 + 整页文本回退） |
| `cargo test -p steam-patch-core --test hgamer_live -- --ignored --nocapture --test-threads=1` | **通过**：真实网络两次查询（§3）；输出见 `.artifacts/p6-hgamer-live.log` |

首轮 P6 记录（宿主/协议未变化，真实网站修复只改 core 适配器，未重跑但沿用有效结果）：`npm run typecheck` 通过、`npm test -w apps/decky` 61 passed、Decky backend 98 OK（2 skipped）、`npm run test:status -w apps/windows` 通过、`npm run build:decky`/`npm run build:windows` 通过。core-sources 亦在修复后执行过同组全量门禁（workspace/clippy/fmt/contract-gen）并报告通过；本轮独立重跑结果与之一致。

说明：`cargo test --workspace` 首轮后台运行曾因终端管道的 transient 退出码记录为 1，但日志内所有套件均为 `ok`；随后以显式 `$LASTEXITCODE` 重跑得到 0，上表为显式退出码结果。

## 8. 未验证项

- HGamer：搜索→身份→补丁元数据链路已通过 live 测试（§3/§5）；**未验证** HGamer 补丁的实际下载/安装可用性（live 测试只读元数据，不下载制品）。
- Kagura 真实全量更新的实际请求数/耗时/流量（静态数出 CN 分页至 24 页、EN 两入口 11 页；未执行真实全量更新）；真实详情页结构、AppID/DLC 关系的全量覆盖未验证。
- patch.moe 真实站点 age gate/cookie 流程在无会话/有会话下的差异（两次 GET 表单数不同：3 vs 0，说明依赖会话/入口参数）。
- Decky / Windows 实机：真实设置/进度/取消、休眠恢复调度、手柄入口、来源任务与安装并行、离线展示。
- 迁移使用真实用户旧状态样本（当前仅合成样本）；安装链真实回归未执行。
- 真实耗时：仅有 HGamer live 两次端到端 4.1s/6.1s（含 1s/host 节流）；Kagura/patch.moe 的真实更新耗时、流量、内存未测。

## 9. 合并建议

- **仍建议实机验收后再合并 main（除非用户明确要求）**：自动化矩阵（11 项 + CLI 2 项）与真实网站关键链路（HGamer live 搜索→身份→补丁详情、Kagura 列表结构与 EN 多入口）均已通过；但双端实机矩阵（设置/进度/取消/休眠恢复调度/手柄/离线）、真实 Kagura 全量更新耗时/流量、真实旧状态迁移和安装链真机回归未执行。
- 下一步：在 Deck + Windows 实机执行矩阵 §7.10 与真实网络任务；补真实 Kagura 全量更新成本与真实旧状态迁移样本；完成后评估合并。
- 若用户明确要求先合入，应把上述未验证项作为合入后的验收清单/已知限制记录，不能声称 P6 全部完成。
