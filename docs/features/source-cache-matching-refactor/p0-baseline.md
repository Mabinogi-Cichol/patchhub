# P0 基线测量与证据

- 状态：P0 实测记录（仅测量与证据，未修改生产逻辑，未提交 Git，未部署设备）。
- 上级设计：[design.md](design.md)；施工记录：[progress.md](progress.md)。
- 分支：`feature/source-cache-matching-refactor`，测量时 HEAD：`353548e`（`docs(sources): 制定来源缓存与游戏增量匹配重构计划`）。
- 测量日期：2026-10-19（Asia/Shanghai）。
- 新增测试：`crates/core/tests/p0_source_baseline.rs`。

本文严格区分两类内容：

- **已测事实**：由新增集成测试用真实 session/adapter 代码 + `FakeFetcher` 实际发出的请求计数，可复现；
- **静态推断**：只读代码得到的宿主侧调用次数与本地工作路径，未在 Deck/Windows 实机运行，因此没有耗时或内存数字。

## 1. 环境与复现

### 1.1 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 家庭版 中文版，`10.0.26300`（`Microsoft Windows NT 10.0.26300.0`） |
| Rust | `rustc 1.94.0 (4a4ef493e 2026-03-02)`，host `x86_64-pc-windows-msvc`，LLVM 21.1.8 |
| Cargo | `cargo 1.94.0 (85eff7c80 2026-01-15)` |
| Rust 工具链隔离 | `CARGO_HOME=<repo>\.artifacts\cargo-isolated\cargo`，`RUSTUP_HOME=<repo>\.artifacts\cargo-isolated\rustup`，并把 `%CARGO_HOME%\bin` 前置到 `PATH`（工具链不在系统 PATH） |
| 测试类型 | `crates/core` 集成测试（离线，无真实网络） |

### 1.2 测量时的仓库状态与复现命令

测量执行期间，同一分支的工作区正被 P1 数据/存储实现并行修改（`model.rs`/`store.rs` 等一度处于不可编译的中间状态）。为保证数字对应明确的代码版本，首次测量在 **HEAD `353548e` 的只读快照**中执行，额外只加入本测试文件；随后工作区恢复可编译，又在工作区原样复跑一次，**数字完全一致**（两次均 `1 passed`）。

两条命令均可用：

```powershell
$root = 'D:\Project Files\git\steam-hentai-patch-searcher'
$env:CARGO_HOME = Join-Path $root '.artifacts\cargo-isolated\cargo'
$env:RUSTUP_HOME = Join-Path $root '.artifacts\cargo-isolated\rustup'
$env:PATH = "$env:CARGO_HOME\bin;$env:PATH"

# 工作区直接复跑（当前分支工作区可编译时）
cargo test -p steam-patch-core --test p0_source_baseline -- --nocapture --test-threads=1
```

```powershell
# 首次测量所用的 HEAD 快照（工作区被并行修改时仍可复现）
$root = 'D:\Project Files\git\steam-hentai-patch-searcher'
git archive --format=zip HEAD -o .artifacts\p0-baseline-source.zip
Expand-Archive .artifacts\p0-baseline-source.zip .artifacts\p0-baseline-source
Copy-Item crates\core\tests\p0_source_baseline.rs .artifacts\p0-baseline-source\crates\core\tests\
$env:CARGO_TARGET_DIR = Join-Path $root '.artifacts\p0-baseline-target'
Set-Location .artifacts\p0-baseline-source
cargo test -p steam-patch-core --test p0_source_baseline -- --nocapture --test-threads=1
```

完整 stdout：HEAD 快照运行存于 `.artifacts/p0-baseline-test-head.log`，工作区复跑存于 `.artifacts/p0-baseline-test-worktree.log`。测试仅断言耗时 1.97s（快照）/ 2.25s（工作区，rustfmt 后复跑），不含编译。

## 2. 测量口径（必须与数字一起阅读）

1. **计数单位**：`FakeFetcher` 每记录一次 `Fetcher::fetch` 调用即计 1 次“HTTP 请求”。本次所有场景均为 `200` 响应、无重试、无重定向，因此“请求数 = `fetch` 调用次数”。若出现 429/5xx 重试或重定向，计数值会高于逻辑页面数（实现见 `crates/core/src/sources/http.rs:422-453`）。
2. **请求轨迹**：每个请求的完整 URL 由测试打印；断言同时校验索引页/详情页各自的请求次数（例如 `url_count`），不是只看总数。
3. **fixture 驱动**：Kagura 页面是测试内构造的最小 DOM（1 条索引 + 1 个详情页；S7 为 2 页 WooCommerce 列表），HGamer 是“0 结果”页，patch.moe 使用仓库既有真实裁剪 fixture `crates/core/tests/fixtures/sources/patchmoe-index.html`。fixture 只保证调用链走到真实解析分支，**不能代表真实站点页数与内容**。
4. **网络策略**：所有 `RequestContext` 使用 `Duration::ZERO` 间隔（与 core 单元测试一致），不测限速/等待；请求数与间隔无关。
5. **规则集合**：默认生产规则含 `kagura-cn`、`kagura-en`、`hgamer`（discovery）与 `patchmoe`（ZIP 索引，见 `crates/core/resources/sources.json`）。本次按“机制隔离”测：
   - S1–S4：仅 `kagura-cn`；
   - S5、S6：`kagura-cn` + `hgamer`；
   - S7：仅 `kagura-en`（WooCommerce，2 页）；
   - S8：仅 `patchmoe`。

   因此表中数字是“单来源机制”的请求数，不是一次生产扫描所有来源的总和。
6. **未测量**：真实网络耗时、DNS/TLS、真实 Kagura 实际分页数、HGamer 真实结果页、Deck/Windows 运行时行为。
7. **跨进程复用**：S2b、S3、S5b、S6、S7 强制段与 S8 重复刷新之前，测试都会从同一状态目录重新 `SourceRegistry::load`（模拟下一次 CLI `scan`/`refresh_sources` 调用），因此“0 请求/索引复用”证明的是持久化记录被复用，不是同一进程内的内存状态。

## 3. 基线表（已测事实）

`cargo test ... -- --nocapture` 打印的汇总表（原样摘录）：

```text
================ P0 BASELINE TABLE ================
scenario                                                   HTTP requests
S1 one game, first discover_game (kagura-cn)                          2
S2 ten games, first scan_sources (kagura-cn)                         11
S2b ten warm games, second scan_sources (force = false)               0
S3 warm matched game, cached query (kagura-cn)                        0
S3 warm matched game, force refresh (kagura-cn)                       1
S4 two games, same detail page (kagura-cn)                            3
S4 same detail URL twice inside one RequestContext                    1
S5 ten missing games, first scan (kagura-cn + hgamer)                11
S5b warm missing game, cached query                                   0
S6 ten warm missing games, forced scan                               20
S7 3 games, first scan (kagura-en, 2 pages)                           2
S7 3 warm games, forced scan (kagura-en, 2 pages)                     6
S8 patchmoe first refresh (ZIP index)                                 3
S8 patchmoe non-forced repeat (cached, no TTL)                        0
S8 patchmoe forced refresh with 304                                   1
S8 patchmoe forced full refresh                                       3
===================================================
```

逐场景完整 stdout 摘录（同一测试输出，已去掉编译日志）：

```text
P0 source baseline (fixture driven, FakeFetcher call count)

S1  one game, first discover_game (rules: kagura-cn)
    GET https://kagurafan.com/
    GET https://kagurafan.com/game/1/
    requests = 2
    index page requests = 1, detail page requests = 1
    candidates = 1, status = complete

S2  ten games, first scan_sources, force = false (rules: kagura-cn)
    Game 1 (appid 2000): 2 request(s)
    GET https://kagurafan.com/
    GET https://kagurafan.com/game/1/
    Game 2 (appid 2001): 1 request(s)
    GET https://kagurafan.com/game/2/
    ...
    Game 10 (appid 2009): 1 request(s)
    GET https://kagurafan.com/game/10/
    total requests = 11
    index page requests = 1, every detail page requested exactly once = true
    warm second scan (force = false): total requests = 0

S3  warm game: cached query then force refresh (rules: kagura-cn)
    cached query: requests = 0, status = cache_complete
    force refresh: requests = 1
    GET https://kagurafan.com/game/1/
    force refresh: index page requests = 0, status = complete

S4  two games, same source detail page (rules: kagura-cn)
    game 1 (appid 3001): 2 request(s): GET https://kagurafan.com/, GET https://kagurafan.com/game/shared/
    game 2 (appid 3002): 1 request(s): GET https://kagurafan.com/game/shared/
    total requests = 3, the same detail URL is requested 2 time(s)
    contrast: the same URL twice inside one RequestContext = 1 request(s)

S5  ten games absent from the index, first scan (rules: kagura-cn + hgamer)
    Missing 1 (appid 4000): 2 request(s): GET https://kagurafan.com/, GET https://hgamer.tw/zh-CN/search?q=Missing+1
    Missing 2 (appid 4001): 1 request(s): GET https://hgamer.tw/zh-CN/search?q=Missing+2
    ...
    Missing 10 (appid 4009): 1 request(s): GET https://hgamer.tw/zh-CN/search?q=Missing+10
    total requests = 11, kagura index requests = 1, hgamer search requests = 10
    warm missing game, cached query: requests = 0, status = cache_complete

S6  same ten warm missing games, forced scan (rules: kagura-cn + hgamer)
    Missing 1 (appid 4000): 2 request(s): GET https://kagurafan.com/, GET https://hgamer.tw/zh-CN/search?q=Missing+1
    ...
    Missing 10 (appid 4009): 2 request(s): GET https://kagurafan.com/, GET https://hgamer.tw/zh-CN/search?q=Missing+10
    total requests = 20, kagura index requests = 10, hgamer search requests = 10

S7  kagura_woocommerce pagination: first scan vs forced warm scan
    first scan game 1 (appid 5000): 2 request(s): GET https://www.kaguragames.com/patch/, GET https://www.kaguragames.com/patch/page/2/
    first scan game 2 (appid 5001): 0 request(s):
    first scan game 3 (appid 5002): 0 request(s):
    first scan total = 2, page 1 requests = 1, page 2 requests = 1
    forced scan game 1 (appid 5000): 2 request(s): GET https://www.kaguragames.com/patch/, GET https://www.kaguragames.com/patch/page/2/
    forced scan game 2 (appid 5001): 2 request(s): GET https://www.kaguragames.com/patch/, GET https://www.kaguragames.com/patch/page/2/
    forced scan game 3 (appid 5002): 2 request(s): GET https://www.kaguragames.com/patch/, GET https://www.kaguragames.com/patch/page/2/
    forced scan total = 6, page 1 requests = 3, page 2 requests = 3

S8  ZIP index refresh (rules: patchmoe)
    first refresh: requests = 3, code = source_refreshed, entries = 6
    non-forced repeat: requests = 0, code = source_cache_fresh
    forced conditional refresh (304): requests = 1, code = source_not_modified, entries = 6
    forced full refresh: requests = 3, code = source_refreshed, entries = 6
```

注：为控制篇幅，上面 S2/S5/S6 的逐游戏行用 `...` 省略了中间若干行相同形式的内容；完整逐行输出见日志文件，断言仍逐个校验每个游戏。

## 4. 每个数字对应的测试与代码路径（已测事实）

测试断言位置：`crates/core/tests/p0_source_baseline.rs`，单一测试 `p0_baseline_request_counts`（`313` 行），各场景注释行：S1 `318`、S2 `368`、S3 `447`、S4 `500`、S5 `577`、S6 `660`、S7 `712`、S8 `817`。

| 数字 | 场景/断言 | 实际执行路径 |
| --- | --- | --- |
| 2 | S1：`assert_eq!(fake.call_count(), 2)` | `session::discover_game`（`session.rs:74`）→ Kagura 构建索引：入口页 `kagura.rs:130` 的 `fetch_page`；命中详情：`kagura.rs:367` 的 `fetch_page`。`fetch_html` 的第二次 `fetch` 命中同一 `RequestContext` 的响应缓存，不再发请求（`http.rs:599-605`） |
| 11 | S2：`per_game[0].len()==2`、其余 `==1`、`total==11`、索引页仅 1 次 | 每游戏新建 `RequestContext`（`session.rs:620`），但索引在首游戏提交后持久化（`registry.rs:856-882`），后续游戏经 `registry.shared_index`（`registry.rs:749`）复用，仅取自身详情（`kagura.rs:117` 条件不成立 → 不再遍历索引） |
| 0 | S2b：暖缓存在 `force=false` 扫描时全部 0；S3 缓存查询 0；S5b 负缓存查询 0 | `needs_refresh`（`registry.rs:723-745`）判定无待刷新时 `discover_game` 直接返回缓存（`session.rs:110-117`），不发请求 |
| 1 | S3：已命中游戏的强制刷新只重取 1 个详情页，索引 0 次 | 已成功候选的 `source_page` 作为 `upstream_pages` 传入（`session.rs:141-148`），Kagura 因此产生 `direct` 页面（`kagura.rs:76-85`），跳过索引重建，只走详情（`kagura.rs:367`） |
| 3 / 2 | S4：两款游戏同详情页共 3 次请求，同一 URL 被请求 2 次；单个 `RequestContext` 内同 URL 两次仅 1 次 | 扫描对每个游戏新建 `RequestContext`（`session.rs:620`），响应缓存是 context 级（`http.rs:472-480`、`599-605`），故跨游戏不共享；同一 context 内第二次 GET 命中缓存 |
| 11 | S5：10 款未命中游戏首扫 = 1 次索引 + 10 次 HGamer 搜索 | 首游戏建索引（1 次），每个游戏 publisher 无候选时 fallback 逐游戏执行（`session.rs:172-210`），HGamer 每个查询词 1 次搜索（`hgamer.rs:56-72`） |
| 20 / 10 | S6：已缓存的 10 款未命中游戏强制扫描 = 20 次，其中索引页 10 次、HGamer 10 次 | 强制时 `force` 让已无候选的游戏重新压入入口页（`kagura.rs:106-107`），`records` 非空也会因 `pending` 非空再次遍历（`kagura.rs:117-130`）；`force` 时 fallback 无条件执行（`session.rs:172-176`） |
| 2 / 6 | S7：kagura-en 两页列表，3 款未命中游戏首扫仅 2 次（索引只建一次）；强制扫描每游戏 2 次共 6 次 | 索引建好后 `pending` 为空、`records` 非空 → 后续游戏跳过遍历（`kagura.rs:117`）；强制时每游戏重新压入入口页并沿 `rel=next` 继续翻页（`kagura.rs:106-107`、`240-242`、`454-459`）；`kagura_cn_html` 的 `seen.len() >= 2` 提前结束条件（`kagura.rs:251`）只适用于 CN 适配器，海外适配器没有该上限 |
| 3 / 0 / 1 / 3 | S8：patch.moe 首次 3 次（索引 GET + 2 个语言表单 POST）、非强制重复 0 次、强制且 304 时 1 次、强制全量 3 次 | `refresh_source` 对任意已缓存 `checked_at` 的非强制刷新直接返回 `source_cache_fresh`（`registry.rs:947-962`，无 TTL）；首次/强制走 `PatchmoeAdapter::collect`（`patchmoe.rs:21-164`：入口 GET + 每语言 POST）；304 分支 `patchmoe.rs:37-47` |

### 4.1 对 design §2 表述的修正（已测事实）

- “**逐游戏触发普通源全站遍历**”在当前实现中**可由强制刷新路径复现**（S6：10 个游戏 10 次索引页；S7：每游戏重翻 2 页），但**普通首次扫描不是这样**：首游戏建索引一次，后续游戏复用持久化索引（S2/S5/S7 首扫索引页均只 1 次）。
- 普通首次扫描的线性增长来自两处，且性质不同：
  1. 每个命中游戏按需取自己的详情页（S2：1 请求/游戏）——这是 2026-10-19 修订后“详情按命中游戏获取”的预期行为；
  2. 每个未命中游戏各触发一次 HGamer fallback 搜索（S5：1 请求/游戏）——这是逐游戏联网的工作量。
- 因此 P2/P3 的验收重点应是：**强制刷新/自动更新按来源执行一次**（不得按游戏广播 force），以及**同一批游戏命中同一 URL 时共享响应**（S4 的 2 次 → 1 次）。

## 5. 静态核对：本地重复工作（静态推断，未实机运行）

以下均为只读代码核对，**不是耗时测量**；行号对应 HEAD `353548e`。

### 5.1 Decky 一次完整扫描/刷新周期：`bridge.scan()` = 3 次

调用链 `Plugin.scan()`（`apps/decky/main.py:103-133`）→ `ScanSession.run()`（`apps/decky/backend/scan_session.py:79-97`）：

| # | 位置 | 触发 | 结果 |
| --- | --- | --- | --- |
| 1 | `main.py:110` → `_read_report`（`main.py:96-101`） | 扫描开始 | `bridge.scan()` 1 次（`main.py:97`） |
| 2 | `main.py:117-123` 的 `refresh` 回调 → `_refresh_rows`（`main.py:82-94`） | `refresh_sources`（ZIP 源）之后 | `bridge.scan()` 1 次（`main.py:85`），并整库重投影（`main.py:87-92`） |
| 3 | `main.py:125-129` 的 `scan_sources` 回调 → `_refresh_rows` | 逐游戏 `scan_sources` 之后 | `bridge.scan()` 1 次（`main.py:85`），并整库重投影 |

- 该路径把 `project_on_update=False`（`main.py:131`），因此扫描期间不再逐事件投影；但每个扫描周期仍有 3 次整库 `scan`。
- `bridge.scan()` 即 `_call("scan", {})`（`apps/decky/backend/bridge.py:92-93`），不是缓存读。

### 5.2 `_refresh_rows` 是整库重复投影

`_refresh_rows`（`main.py:82-94`）对 `report["installs"]` 中**所有**安装实例逐行 `row.clear()/update()`（`main.py:88-92`），并在每次 ZIP 刷新后与每次来源扫描后各执行一次（`main.py:122`、`main.py:128`）。即一次刷新周期至少 2 次 O(安装数) 行重投影，加上 3 次 Rust `scan` 内部自身的整库投影。

### 5.3 `start_game_discovery` 会关闭扫描会话并再次整库 `scan`

`Plugin.start_game_discovery`（`main.py:161-184`）：

1. 关闭现有 `_scan_session`（`main.py:165-170`）——若用户正在整库扫描，会被取消；
2. `_read_report()`（`main.py:173`）→ `bridge.scan()` 1 次；
3. 新建 `ScanSession` 时未传 `project_on_update`，采用默认 `True`（`scan_session.py:14`）；Rust `scan_sources` 每发出一个 `source_update` 事件，`on_event` 都调用 `project(row)`（`scan_session.py:52-72`，其中 `67-68`）→ `_project_row`（`main.py:72-80`）→ 又一次 `bridge.scan()`。

单游戏查询时 Rust 只扫描该实例（`source_service.py:133-141` 传入 `instance_id`；`crates/cli/src/runtime.rs:906-934` 只收集该实例），事件数为 1，因此单游戏查询 = **2 次整库 `scan`**（读取报告 1 次 + 事件投影 1 次），并伴随一次旧扫描会话取消。

### 5.4 `_project_row` 确实用全库 `scan` 取单行

`_project_row`（`main.py:72-80`）：先 `report = await self.bridge.scan()`（`main.py:74`，整库扫描+投影），再线性查找单个 `instance_id`（`main.py:76-80`）。同一逻辑也用于 `Plugin.discover_game`（`main.py:193-208`，投影在 `main.py:206`）——**每次单游戏发现结束后都有 1 次整库 `scan`**。

### 5.5 Windows 侧对等调用（`core_call("scan")`）

`apps/windows/src/main.tsx`：

| 流程 | `core_call("scan")` 次数 | 行号 |
| --- | --- | --- |
| 初始/手动整库刷新 `refresh()` | 1 次（随后自动 `startSourceScan(false)`） | `main.tsx:324-344`，`scan` 在 `330`，自动扫描在 `343` |
| 整库来源扫描期间 | 每个 `source_update` 事件 1 次 | `main.tsx:176-222`，`scan` 在 `199` |
| 整库来源扫描结束后 | 1 次 | `main.tsx:250-279`，`scan` 在 `263` |
| 单游戏“重新查找”（强制） | 2 次（1 个事件 + 1 次结束投影） | 按钮 `main.tsx:545` 传 `force=true` → `findGame` `297-307` → `startSourceScan` `250-279` |

即一次包含 N 个安装游戏（N 个 `source_update`）的 Windows 整库来源扫描共有 **N+2 次整库 `scan`**；Tauri 侧 `start_source_scan` 只是把一次 `scan_sources` 作为 `serve-task` 子进程运行并把事件转发给前端（`apps/windows/src-tauri/src/main.rs:1324-1438`，事件转发 `952-974`）。`refresh_sources` 为独立来源任务命令（`main.rs:537-644`），其事件不带 `discovery`，前端不会为它额外 `scan`（`main.tsx:194` 条件 `"discovery" in payload`）。

### 5.6 每次 `scan` 本身的本地工作（Rust 侧）

`crates/cli/src/runtime.rs` 中 `scan` 与 `scan_sources` 都会重新执行：

- `steam::scan(&roots)`（`runtime.rs:504`，`scan_sources_impl` 亦在 `runtime.rs:904` 单独执行一次）——重新遍历 Steam 库清单；
- `SourceSnapshot::load(state, &queries, now)`（`runtime.rs:584-591`）——为**每个游戏**预载 discovery 记录与索引（`sources/snapshot.rs:32-79`，`74` 调 `preload_snapshot_games`）；
- `projection::scan(inputs)`（`runtime.rs:657`，实现 `crates/core/src/projection.rs:304`）——整库投影；其中动态来源选中项会对每个 change 文件做证据检查（`projection.rs:840-905`，`879` 调 `detection::inspect_file`），实际读取并计算 SHA-256（`crates/core/src/detection.rs:106-149`、`428-489`）。

因此宿主每多一次 `scan`，就多一次“库清单 + 每游戏来源快照 + 整库投影（以及可能存在的安装文件哈希）”。本 P0 只记录调用次数，不测耗时/哈希字节数。

### 5.7 静态核对小结

- Decky 一次扫描/刷新周期 `bridge.scan()` = **3 次**；单游戏 `start_game_discovery` = **2 次整库 `scan`** 且取消旧扫描会话；`discover_game` RPC = **1 次整库 `scan`**。
- Windows 整库来源扫描 = **N+2 次整库 `scan`**（N 为安装游戏数）；单游戏强制重新查找 = **2 次**。
- 两侧都存在“用全库 `scan` 取单行”的路径（Decky `_project_row`，Windows 结束/事件后 `setScan(fresh)`）。

## 6. 已测事实 vs 静态推断

| 结论 | 类别 | 依据 |
| --- | --- | --- |
| 1 游戏首扫 = 2 请求（索引 1 + 详情 1） | 已测事实 | S1 |
| 10 款命中游戏首扫 = 11 请求，索引仅 1 次，其余为详情 | 已测事实 | S2 |
| 缓存后第二次普通查询 = 0 请求（正/负缓存均 0） | 已测事实 | S2b/S3/S5b |
| 已命中游戏强制刷新 = 1（只重取详情，索引复用） | 已测事实 | S3 |
| 同批两游戏命中同一详情页 = 该 URL 请求 2 次；同一 `RequestContext` 内为 1 次 | 已测事实 | S4 |
| 10 款未命中游戏首扫 = 1 次索引 + 10 次 HGamer | 已测事实 | S5 |
| 10 款未命中游戏强制扫描 = 10 次索引 + 10 次 HGamer（逐游戏重遍历索引） | 已测事实 | S6 |
| 海外适配器 2 页 fixture：首扫索引一次；强制时每游戏重翻 2 页 | 已测事实 | S7 |
| patch.moe：首次 3 / 非强制 0 / 304 条件刷新 1 / 强制 3 | 已测事实 | S8 |
| Decky 一次周期 3 次 `bridge.scan`、单游戏 2 次 | 静态推断 | §5.1、§5.3；未运行 Decky |
| Windows 整库扫描 N+2 次 `core_call("scan")` | 静态推断 | §5.5；未运行 Windows 应用 |
| 每次 `scan` 都做库清单 + 来源快照 + 整库投影（可能含文件哈希） | 静态推断 | §5.6；未测耗时 |
| 真实站点每游戏/每次更新的实际请求量与耗时 | 未测 | fixture 不能代表真实分页与详情数 |

## 7. 局限与未验证项

1. **fixture ≠ 真实站点**：Kagura 索引/详情为最小 DOM；真实 `kagurafan.com` 首次更新会翻多少页、`kaguragames.com` 列表有多少分页、详情页是否含 AppID/DLC 关系，均未测（design §8 已列为待验证）。
2. **未测真实网络耗时**：本文只有请求计数，没有秒数、流量、并发或限速数据；不得据此宣称性能提升幅度。
3. **未执行 Deck/Windows 实机**：§5 的调用次数来自代码核对；未验证宿主实际 RPC 次数、界面轮询、取消时序与资源占用。
4. **规则集合收敛**：为隔离机制，测试按来源拆分规则；一次生产扫描会叠加 `kagura-cn` + `kagura-en` + `hgamer` + `patchmoe`，其总请求数不是表中数字简单相加（还取决于每游戏命中情况与真实分页）。
5. **HGamer 查询词**：`identity::query_terms` 最多产生 3 个词（`crates/core/src/sources/identity.rs:66-95`），中文/繁体转换名可能使每游戏 fallback 请求数 >1；本次用 ASCII 名（1 词/游戏），未测多词情形。
6. **降级/失败路径**：未测量异常、超时、429/5xx 重试、取消与部分成功下的请求数；这些路径的请求数会高于本文正常路径（重试按 `http.rs` 会额外计次）。
7. **测量时的工作区状态**：首次测量在 HEAD `353548e` 快照中完成；当前分支工作区在测量结束时已恢复可编译，工作区复跑数字与快照一致（§1.2）。若并行修改再次使工作区不可编译，按 §1.2 的快照方式复现。

## 8. 对后续阶段的可验证输入

- P2 完成门槛应能复用本测试并断言：来源更新请求数与已安装游戏数无关；同一轮相同 URL 只请求一次；强制刷新按来源执行而不是每游戏广播 force。
- P3 完成门槛应能断言：索引命中后的详情请求只发生在命中游戏；单游戏重匹配不重新遍历普通源索引；同一批游戏命中同一 URL 时请求数不随游戏数增长。
- P4 完成门槛应能用本文件的“静态推断”行号做对照：宿主单次扫描的 `scan` 次数从 3（Decky）/N+2（Windows）降到固定次数，且单行投影不再调用整库 `scan`。
