# 「冲突的意志」安装事务性能分析报告（AppID 1011940）

日期：2026-10-17（Asia/Shanghai）。状态：**诊断完成；修复方案未设计、未实施。** 本报告为性能诊断与证据归档，不改变当前代码与验收结论。

关联：[设计](design.md) · [施工记录](progress.md) · [代码审计](audit.md)。前次同游戏实机安装记录见施工记录“2026-10-16：大包 EXE 实机验证与报告/ZIP64 修复”。

## 1. 结论摘要

用户报告在 Steam Deck 上安装“冲突的意志”（Ideology in Friction）补丁需 20~30 分钟，且下载只占小部分。诊断确认：

- 总时长由两段构成：**下载/分析/转换 ~12 分钟 + apply 安装事务 1153s（19.2 分钟）**；热点在 apply 事务。
- apply 事务时间的构成（设备侧证据）：校验+全量备份 ~265s、**写盘 12,141 个文件 815s（67ms/文件）**、提交+清理 ~70s。
- 根因是安装事务中一系列 **按文件数线性执行的重复操作**：写循环内逐文件全库扫描、每文件全同步持久化（SQLite `synchronous=FULL` ×2 + fsync 4~5 次）、同一文件 3~5 次全量 SHA-256，以及 apply 前全量备份与 commit 前二次全量校验。
- Deck 微基准显示单文件合成成本 ~9ms（空载），实测 67ms/文件，差额来自 appinfo.vdf 真实解析、ZIP member 读取与持续脏页负载下 fsync 等待放大。

## 2. 实机现场还原

对象：2026-10-17 的一次成功安装。任务 `8d0c735d-db68-49ef-a855-1331d7667e6b`，计划 `196549ee-bfa6-8c4d-beac-2b7f485cf1fd`；最终 `state=committed`、`disposition=completed`、`error=null`。

补丁与规模：

- 源：kagura-cn EXE；下载包 3,788,037,984 B（3.53 GiB）。
- 规范化载荷（payload.zip）：3,796,758,922 B；`plan.changes` **12,141 个文件**（全部为替换已有文件）。
- 安装后游戏目录：13,722 个文件、4.37 GB；被替换文件平均 ~312 KB（中位 191 KB，最大 1.05 GB）。

时间线（设备本地时间 CST；证据：journal `transaction.created_at/updated_at`、job 目录文件 mtime、游戏目录 12,141 个文件 mtime 分桶）：

| 阶段 | 区间 | 时长 | 证据 |
| --- | --- | --- | --- |
| 下载 + 分析 + 转换 | ~02:43 → 02:55:31 | ~12 分钟（下载约 350s，取 10-16 同包实测） | request lock mtime 02:43、plan.created_at 02:55:31 |
| apply：校验 + 全量备份（`validating`/`backing_up`） | 02:55:31 → ~03:00 | ~265s | 首个写盘文件 mtime 03:00 |
| **apply：写盘（`writing` 循环）** | ~03:00 → ~03:13:35 | **815s（67ms/文件，14.2 文件/秒）** | 12,141 个文件 mtime 按分钟分桶（474/1087/984/846/858/850/869/870/870/877/928/1011/1020/597） |
| apply：提交 + 清理（删除 ~3.82 GB） | ~03:13:35 → 03:14:44+ | ~70s | transaction.updated_at 03:14:44 |
| **apply 事务总计** | 02:55:31 → 03:14:44 | **1153s = 19.2 分钟** | journal 时间戳 |
| **用户感知总计** | ~02:43 → ~03:15 | **~31 分钟** | 与“20~30 分钟”一致 |

对照：2026-10-16 同一游戏上一次成功安装（计划 `cf01c69e`）事务 1044s（17.4 分钟），施工记录记载总 1354s（下载 ~350s、分析 ~60–120s、转换 ~120s、校验 ~200s、写盘 ~800s）。两轮高度一致，写盘稳定在 ~800s。

## 3. 热点明细（按代价排序）

除注明外，行号均指 `crates/core/src/install.rs` 当前工作区版本。

### 3.1 写循环内逐文件全库扫描（实现缺陷）

- `apply_files` 写循环（:2023–2062）每个文件调用 `validate_install`（:2027）。
- `validate_install`（:826–854）内部执行 `steam::scan(&[library])`（:843）：canonicalize 路径、`read_dir` 各库 `steamapps`、解析 20 个 appmanifest、读取 `config.vdf`，并在名字解析时**全量读取与遍历解析 2.5 MB 的 `appcache/appinfo.vdf`**（steam_metadata.rs:198–206 → appinfo.rs:37）。
- 即 12,141 次完整 Steam 库扫描。该检查在 `apply_at` 入口（:1878）已经执行过一次，语义上无逐文件重复的必要（同一计划、同一游戏锁持有中，:1879）。
- 设备实测 I/O 部分（无 VDF 解析、page cache 命中）中位 0.85ms/次；真实含解析成本更高。这是纯实现层的 O(N) 放大。

### 3.2 每文件全同步持久化：SQLite FULL ×2 + fsync 4~5 次（设计取舍叠加实现重复）

写循环每文件触发的持久化操作：

| 操作 | 位置 | 说明 |
| --- | --- | --- |
| `save_progress`（"writing"） | :2030 → :598–608 | `INSERT … ON CONFLICT`，连接级 `PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL`（:508） |
| `write_archive_member` 内 `sync_all` + `sync_dir` | :1728、:1733 | 临时文件落盘 + 父目录 fsync |
| 循环内 `sync_all(temp)`（第二次） | :2050 | 与 :1728 重复刷同一文件 |
| `sync_dir(dest.parent())` | :2055 | rename 后目录 fsync |
| `save_progress`（"installed"） | :2060 | 第二次全同步 INSERT |

- 合计每文件 **2 次 FULL 同步 SQLite 提交 + 4~5 次 fsync**，×12,141 = ~24,000 次全同步提交 + ~50,000 次 fsync。
- `backing_up` 循环（:1995–2015）同样：`copy_checked` 内 `sync_all`+`sync_dir`（:1805、:1810–1814）+ `save_progress`（:2013）。
- 探针实测 SQLite FULL 提交 4.15ms/次；fsync 家族在空载合成 2.53ms/组。持续脏页负载下（安装期间连续写入 GB 级数据）等待时间进一步放大。
- 定性：逐文件崩溃安全粒度是设计取舍；但同一临时文件两轮 `sync_all`+`sync_dir`（:1728/:1733 与 :2050/:2055 之间）属于可合并的实现重复。

### 3.3 每文件多次全量 SHA-256 与重复读取（实现缺陷）

写循环中对每个文件的全量哈希/读取（不含循环外）：

1. `hash_at(temp)`（:1730，`write_archive_member` 内）；
2. `hash_at(dest)` 读旧文件（:2043）；
3. `hash_at(dest)` 读新文件（:2056）；
4. `backing_up` 循环里 `copy_checked` 对备份副本的哈希校验（:1807）。

加上事务级的两轮全量校验（§3.4），同一批 3.8 GB 数据被完整读取与哈希至少 5~6 轮。单文件平均 312 KB 的哈希本身很快（探针 0.20ms/191KB），但读放大叠加在 fsync 压力上共同拉低写盘吞吐（有效 ~4.7 MB/s）。

### 3.4 apply 前全量备份与双次全量校验（设计取舍）

- `apply_at` 前全量校验 `verify_all(&job, false)`（:1945）：哈希全部 12,141 个旧文件。
- `backing_up`：把 12,141 个旧文件全量复制到 `jobs/<plan_id>/` 备份（每文件读+写+哈希）。
- commit 前再 `verify_all(&job, true)`（:2065）：哈希全部 12,141 个新文件。
- 另有 `open_payload_archive`（:1680–1705）在 apply 开头对 3.79 GB payload.zip 做一次全量 SHA-256（:1695–1699）与 preflight。

以上为事务/恢复语义的设计决定（全量备份 + 双端校验），但随文件数线性膨胀；结合 §3.2/§3.3 构成 apply 事务的主要时间。

### 3.5 其他观察

- 全流程单线程串行：写循环、备份循环、哈希均无并发。
- `make_parents`（:1818–1838）逐级检查/创建目录，多数命中已存在路径，成本低；新建目录时 `save_directory` + `sync_dir`（:1830–1832）。
- 文件路径安全检查（`target`/`safe_path`/`temporary`）每文件多次 `symlink_metadata`，量级小，未单独计时。
- 前端 750ms `task_status` 轮询只读 Python 内存任务快照，不产生子进程；Rust 侧进度事件按 100ms 节流，均非热点。

## 4. 量化证据（Deck 微基准）

方法：在设备（SteamOS，eMMC 内置盘，`/home/deck`）上运行只读探针 `perf_probe.py`，复刻 `steam::scan` 的 I/O 序列、journal 表结构与 PRAGMA、以及逐文件写路径；n=100（合成 n=50），空载、page cache 命中。脚本与完整输出见 §7。

| 操作 | 中位延迟 |
| --- | --- |
| `steam::scan` 等价 I/O（不含 VDF 解析） | 0.85 ms |
| SQLite 全同步 INSERT（`journal_mode=DELETE` + `synchronous=FULL`） | 4.15 ms |
| 写 312 KB + fsync + rename + 目录 fsync | 2.53 ms |
| SHA-256（191 KB） | 0.20 ms |
| 合成单文件骨架（scan + 2×INSERT + 写/fsync + 2×hash） | **9.15 ms** |

对照实机写盘 67 ms/文件（815s/12,141）：差额来自 appinfo.vdf 真实解析（探针未模拟）、ZIP member 读取、`backing_up` 与 `verify_all` 阶段，以及负载态下 fsync 等待放大 2~5 倍。环境数据：20 个 appmanifest（共 17,691 B）、appinfo.vdf 2,588,618 B、4 条 library 记录（含未挂载 SD 卡路径）。

## 5. 问题定性

| 项 | 定性 | 说明 |
| --- | --- | --- |
| 循环内 `validate_install`→`steam::scan` | 实现缺陷 | 本应事务级一次；逐文件重复无额外语义 |
| 同文件两轮 `sync_all`+`sync_dir` | 实现缺陷 | `write_archive_member` 与循环体重复刷盘 |
| 同一文件 3~5 次全量哈希 | 实现缺陷 | 部分校验点可合并或利用已持有摘要 |
| 每文件 SQLite FULL + fsync 的崩溃安全粒度 | 设计取舍 | 语义需要，但粒度与文件数线性放大；可评估批量/降级同步而保留恢复语义 |
| apply 前全量备份 + 双次全量校验 | 设计取舍 | 恢复与幂等语义需要；可评估流式/单遍实现 |
| 全流程单线程 | 设计现状 | 非缺陷，但限制了磁盘/CPU 并行度 |

总体：**用户感知的 20~30 分钟中，约 19 分钟是 apply 事务；其中主要放大来自实现层每文件重复操作，其次为逐文件同步持久化的设计粒度。** 修好实现层两项与合并 fsync 即可显著缩短写盘阶段（粗估 815s 可减至 200~400s 区间，取决于批量持久化取舍）；彻底优化需在事务设计层评估。

## 6. 候选修复方向（未设计、未评审，不作为结论）

1. 将写循环内 `validate_install` 移至循环外（事务入口已有一次，可直接删除循环内调用；若需防游戏变更，改为低频抽查或按目录批检）。
2. 合并同一临时文件的两轮 `sync_all`+`sync_dir`（保留 rename 后目录 fsync）。
3. `save_progress` 改为批量/降低同步级别（例如阶段末一次 FULL、期间普通提交），保留崩溃恢复语义等价性评估。
4. 减少重复哈希：利用写前已校验的摘要，删除可推导的重复 `hash_at` 点。
5. 备份与提交前校验评估流式单遍实现，避免多轮 3.8 GB 级读取。

以上方向需另行设计并走本项目设计与验收流程（含过度设计审阅），本报告不实施、不承诺。

## 7. 证据、脚本与复现

证据归档：`.artifacts/install-perf-diagnosis-20261017/`（忽略目录，本地保留）

- `summary.md`：诊断汇总（本报告依据的完整版）。
- 设备取证副本：`journal.sqlite`、`tasks.json`、`condition-196549ee.json`、`authorization-196549ee.json`。
- 分析脚本：`dump_schema.py`、`list_jobs.py`、`inspect_job.py`、`analyze_timeline.py`、`check_progress_tables.py`、`find_task.py`、`timestamps.py`。
- 设备探针：`perf_probe.py`（输出已记录；远端探针目录 `/home/deck/steam-patch-validation/perf-probe-20261017/` 已清理）。
- 连接与设备核对沿用本轮 Devkit 配对（`deck@192.168.31.186`，known_hosts 位于 `.artifacts/defensive-logic-deck-20261017/known_hosts`）；本轮仅只读取证与探针，未部署、未修改设备上任何插件/游戏文件。

复现要点：`journal.sqlite` 的 `jobs.document` 含 `transaction.created_at/updated_at` 与 `plan.changes`；游戏目录文件 mtime 分桶命令：

```bash
find "/home/deck/.local/share/Steam/steamapps/common/Ideology in Friction" -type f \
  -newermt "2026-10-17 02:40" -printf "%TH:%TM\n" | sort | uniq -c
```

## 8. 边界与未验证项

- 未用 strace/perf 对真实安装做系统调用级采样；写盘 815s 的构成（fsync 等待 vs 哈希 vs 扫描）为代码证据 + 微基准推断，未逐项实测。
- `appinfo.vdf` 解析的单次成本未单独计时（探针只覆盖 I/O 序列）。
- 02:55:31→03:00 的 ~265s 未细分 `verify_all(false)` 与 `backing_up` 各自耗时。
- 备份阶段的 3.82 GB 清理耗时含在 ~70s 内，未单独分解。
- 微基准为**空载**测量；负载下放大倍数（2~5 倍）为区间估计，非精确值。
- 未修改任何代码；修复收益估算为粗估区间，未经实施验证。
