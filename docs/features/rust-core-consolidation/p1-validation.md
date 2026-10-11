# P1 核心投影施工与验证

日期：2026-10-11。阶段：已实现，自动化与独立审查完成；性能对照见下表。仅完成 P1，P2–P6、双端最终产物与实机待实施。

设计：[design.md](design.md)。施工：[progress.md](progress.md)。冻结规格：[P0 契约](p0-contract.md)。对照：[P0 基准](p0-baseline.md)。

## 实际实现

- [projection.rs](../../../crates/core/src/projection.rs) 提供类型化 ProjectionInputs / CatalogView / HostCapabilities、完整 scan、单实例 project_instance、去重保序 project_batch、三类 preview。复用调用方的 Steam 事实，批量统一校验一次，不重新扫描库。
- [SourceSnapshot](../../../crates/core/src/sources/snapshot.rs) 只读捕获有效规则、订阅、来源状态、缓存、discovery 和共享 index，固定时刻并禁止后续磁盘回读。缺失来源根返回 None；损坏设置/状态/发现/index 明确报错，损坏缓存丢弃并保留诊断，不初始化或修复持久文件。
- [TransactionSnapshot](../../../crates/core/src/install.rs) 在同一只读 SQLite 事务捕获事务与 committed plans。保留最新损坏计划的可恢复收据身份及 overlay 最大时间，无法恢复身份则禁止旧计划证明 managed。仍读取既有 installer/rust-v1，不新增 schema、状态根或日志。
- [detection.rs](../../../crates/core/src/detection.rs) 是目录和动态收据的共用取证入口。相对路径/hash 类型校验，nofollow 逐组件打开并持有祖先句柄，拒绝链接/reparse/非文件，核对边界、路径/句柄身份、前后大小/mtime，64 KiB 流式读取、16 GiB 上限与取消。单次读取内复用已持有祖先，无跨调用证据缓存。
- 来源 JSON 也复用受限读取保障；cache/status/discovery/index 内部链接无法读取根外数据。
- 签名优先覆盖同 release 的 ZIP 与 EXE 候选，动态 release 身份及 ZIP enabled/subscription/stale 条件来自 [resolver](../../../crates/core/src/installer/resolve.rs)；EXE 复用现有 resolver。去重在取证前完成。
- [CLI](../../../crates/cli/src/runtime.rs) 删除原 dynamic_patches / dynamic_detection / hash_target / preview_route / committed_changes / dynamic_release_id，改为构造快照和宿主能力、调用核心及序列化。scan/preview 仅 verify_signed，安装仍显式 remember_revision，等待 P2 提取。
- 不可靠文件为 unknown，而不是 missing；空计划不能判为已安装。预览无法取证的 baseline_matches/actual_sha256 为 null，带 baseline_unverified 和诊断，区别已确认的 baseline_conflict。

## 自动化证据

| 检查 | 实际结果 | 证据 |
| --- | --- | --- |
| Windows core / CLI | 417 项通过、6 项既有测试 ignored，0 失败 | [日志](../../../.artifacts/rust-core-consolidation/p1/windows-tests.log) |
| WSL Ubuntu、非 root、Rust 1.94 core / CLI | 422 项通过、6 项既有测试 ignored，0 失败 | [日志](../../../.artifacts/rust-core-consolidation/p1/linux-tests.log) |
| fmt | 全 workspace fmt --check 通过 | [记录](../../../.artifacts/rust-core-consolidation/p1/final-checks.json) |
| Clippy | core/CLI --all-targets -D warnings 通过 | [日志](../../../.artifacts/rust-core-consolidation/p1/windows-clippy.log) |
| CLI JSONL 实际子进程 | 2 项通过：Unicode、多库同 AppID、证据/预览、只读与损坏状态可见 | [测试](../../../crates/cli/tests/projection_smoke.rs) |
| 核心公共 API | 投影14、来源快照5、目录8、取证 Windows5/Linux5（平台分支不同）及读取中变化/取消通过 | [投影](../../../crates/core/tests/projection.rs)、[来源](../../../crates/core/tests/source_snapshot.rs)、[取证](../../../crates/core/tests/detection.rs) |

命令为 scripts/cargo.ps1 test --offline -p steam-patch-core -p steam-patch-cli；Linux 使用已有 .tools/linux-cargo + .tools/linux-rustup 和独立 .local/p1-linux-target，未更换工具链或下载依赖。既有 ignored 项仍为手动 benchmark、真实 worker/网络或 fixture 生成入口，不计为通过。验证汇总：[validation-summary.json](../../../.artifacts/rust-core-consolidation/p1/validation-summary.json)。

所有写入安装/回滚检查只发生在临时测试目录。基准动态收据通过本地签名、隔离 ZIP 和实际 serve-task 生成；task stdin 保持打开，响应只接受一个终态。没有网络下载、设备连接、部署或用户游戏写入。

## 独立审查闭环

Sol/high 只读审查发现 4 项 P2，均已修复并增加回归：

1. 相对 state_dir 的空 ancestor 被误判缺失：入口统一 absolute，原相对路径 fixture 可捕获。
2. 来源内部目录链接：共用祖先 nofollow/身份保护，四个内部目录的 Windows junction / Linux symlink 负向通过。链接缓存保持丢弃+诊断语义。
3. 损坏 overlay 丢失最新更新时间：同一个 SQLite 快照恢复最大时间；新30/旧20/base10 回归保持 unknown。
4. 预览未知基线误报 false/conflict：核心返回 null 与取证诊断。

额外修正：scan 保留 Steam 原始 instance_id，即使长 Unicode 路径生成超过256字节也不改写持久身份；核心精确查已有 Steam 事实。wire 请求仍按当前 v3 检查，统一字节契约留待 P5。Unix symlink fixture 用 remove_file，Windows junction 用 remove_dir。

四项修复与单次祖先句柄复用的定向安全复核均通过，无遗留可操作问题。独立审查只提供静态结论，运行证据由主 Agent执行。

## 性能对照

| Windows debug fixture | P0 warm median | P1 warm median | 变化 | 哈希尝试/完成/字节 |
| --- | --- | --- | --- | --- |
| manifest-only | 0.324238s | 0.299593s | -7.6% | 0/0/0 B（相同） |
| recipe-all-match | 0.668940s | 0.973186s | +45.5% | 96/96/25,165,824 B（相同） |
| recipe-mixed-evidence | 0.373439s | 0.602034s | +61.2% | 96/48/12,582,912 B（相同） |
| dynamic-managed（新增） | 未覆盖 | 0.429012s | 无旧对照 | 32/32/8,388,608 B |

命令：python scripts/benchmark-projection.py --output .artifacts/rust-core-consolidation/p1/projection-benchmark.json --runs 6。4 个 fixture 各6轮，首轮单列，后5轮取中位数；普通与计数二进制4组响应全部等价，生产源码 SHA-256 不变。仅隔离副本插桩；动态收据为8实例、2库、4 AppID、每项4文件，实际写入隔离安装状态后扫描。

[P1 完整结果](../../../.artifacts/rust-core-consolidation/p1/projection-benchmark.json)、[归一化快照](../../../.artifacts/rust-core-consolidation/p1/projection-benchmark-snapshots.json)、[P0/P1 对照](../../../.artifacts/rust-core-consolidation/p1/projection-comparison.json)、[执行日志](../../../.artifacts/rust-core-consolidation/p1/benchmark.log)。三组原 fixture 删除唯一新增的实例 diagnostics 字段后，与 P0 扫描事实完全一致。混合 fixture 为非普通文件增加相对路径诊断，响应体因此由35,524 B增至45,052 B；其余两组仅增加空诊断字段。

哈希次数与读取字节未增长；500清单略快，带取证扫描比P0慢45.5%/61.2%。新增逐组件 nofollow 打开、祖先句柄保持、路径及读取前后身份/大小/mtime 检查带来成本，单次复用祖先已降低重复打开开销，仍保留完整安全检查。本轮没有证明整体性能提升，不以少哈希冒充耗时优化。两次基准是同机不同时段的 debug 进程总耗时，含启动/序列化，未测冷缓存，不代表发行包或实机性能；Linux性能和本机真实36清单的P1重测未执行，留在最终验收矩阵。

## 阶段过渡与未执行项

- P1 使用核心 PatchMatch，保留 download_available 与 v3 来源字段的 optional 省略；P4 引入必需 actions，P5 删除旧字段/手写 wire 适配。
- ProjectionDiagnostic 暂用有界 message，CLI catalog_error 暂序列化成 v3 字符串；preview 已统一三态 changes，P5 收口正式生成 DTO。
- 业务请求 ID 暂为已按入口/精确事实校验的 String；checked wire newtypes 与128/256字节契约由 P5统一。不得改写现有收据身份。
- signed 安装/trust revision、缓存清理、UI 权限组合、双端桥接与协议版本尚未迁移；属于 P2–P5，不视为本阶段完成。
- 当前仍为 v3 开发树；未部署、未构建双端最终发行包、未执行实机验收。Linux Clippy、两端最终矩阵和产物检查留待 P6。


## 提交更新（2026-10-12）

用户授权后，P1/P2 的同功能连续实现已统一本地提交 `e77d0a8`，验证与契约文档独立提交。上文保留 P1 阶段完成时的验证和未提交事实；后续 P2 证据见 [p2-validation.md](p2-validation.md)。未推送、未部署。
