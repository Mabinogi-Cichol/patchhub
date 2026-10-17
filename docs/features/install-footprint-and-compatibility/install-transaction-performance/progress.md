# 安装事务性能优化施工记录

关联：[设计文档](design.md) · [父功能施工记录](../progress.md) · [原始性能报告](../perf-analysis-20261017.md)。

当前状态：**D1–D4 已实施并通过本地自动化检查；D5 双平台实机验收待执行。**

| 阶段 | 状态 |
| --- | --- |
| EXE 无备份三步流程 | 按用户决定重写并已实施 |
| D1 解析与单会话流式接口 | 已实现；Windows 自动化通过，Linux 分支待编译/实机验证 |
| D2 无备份直接写入 | 已实现；自动化通过 |
| D3 任务结果、中断及重试 | 已实现；新路线不回滚，旧事务恢复保留 |
| D4 删除旧 EXE 转 ZIP 链路 | 已实现；旧 ZIP/事务恢复路径保留 |
| D5 双平台及性能验收 | Deck 覆盖安装通过；Windows、进程中断/重试、旧事务恢复实机与游戏内生效待验证；历史记录已重新取证 |

## 2026-10-17：报告复核与子功能规划

- 基线 `6a62687`，开始时工作区干净。读取父功能设计、既有施工记录、性能报告以及实际 apply、archive/stage 写入、备份、restore、journal 和 Steam 扫描执行路径。
- 确认 apply 和 restore 均逐文件调用全库扫描；提出复用目标 manifest 解析，保留实例检查时点，去除库枚举与名称解析。
- 修正报告候选方案的推理：二次文件 sync 中间存在权限修改；不同哈希点具有不同对象和时间窗口；当前恢复依赖先持久化 writing 意图，不能直接降低 SQLite 同步级别。
- 历史 1153 秒/815 秒为引用数据，本轮未连接设备，未重新运行探针；缺少原始归档，未验证微基准可复现性。未采纳 200–400 秒作为验收承诺。
- 新建设计与施工记录，在父功能和功能索引挂接。只修改文档，未修改生产代码、未部署、未提交 Git。`git diff --check` 通过；新增设计/施工记录中的全部 Markdown 相对链接目标存在，父功能与索引挂接目标已核对。

## 2026-10-17：归入父功能目录

- 按用户要求将本子功能移至父功能的 install-transaction-performance/ 目录，保留既有设计与记录，更新父功能、功能索引及子功能内部相对链接；状态仍为待实现。

## 2026-10-17：删除不必要的并发修改假设

- 用户明确安装期间 Steam 更新或外部修改无需考虑。撤回初稿中定向 manifest 接口、保留逐文件实例检查及调整取锁顺序的方案；P1 改为直接删除 apply/restore 循环内重复校验，沿用现有事务边界检查。
- 同步修订哈希评估依据、实施步骤与验收要求，不再以安装期间外部修改为设计依据。上方初稿记录保留为历史，以当前设计为准。
- 本轮仅修改设计与记录，生产代码未变，状态仍为待实现。

## 2026-10-17：重新取证并纠正哈希审阅依据

- 按用户授权，通过当前 Devkit 配对与已有可信主机公钥只读连接 Steam Deck，取回该游戏完整 job、任务、文件时间及 Loader 日志，详见[证据记录](evidence-20261017.md)。没有部署、重新安装或写盘探针。
- 确认两次成功事务 1044/1153 秒；留存的 4 次失败涉及来源查找、报告大小限制及 ZIP 元数据解析，没有该游戏哈希失败记录。精确文件 mtime 跨度约 785.25 秒，不能等同函数阶段计时。
- 撤回“对象不同，所以复读应默认保留”的推理。P3 已变为本次实施任务：删除正常路径原文件重复校验、rename 后及提交前复读，复制/解压所需摘要并入已有数据流；恢复识别摘要与正常路径复读分开处理。
- 保留旧记录为历史；当前方案以设计 P1–P4 为准。生产代码未修改，优化未实施。文档 diff 与相对链接检查通过。
## 2026-10-17：全部热点统一审阅

- 按用户要求复核报告 §3.1–3.5 及全部候选方向；沿 Store::save/save_progress/load、validate_job、apply_files、restore、make_parents、cleanup_terminal 核对实际状态消费者，不再只修改哈希项。
- P2 扩展为一次文件同步、删除 rename 前临时目录同步、备份目录在阶段末统一同步；替代前版默认保留全部目录同步的决定。
- P4 纳入本次施工：正常路径删除逐文件 backed_up 和 installed 提交，恢复删除逐文件 restored 提交；复用已有阶段/终态保存。writing/restoring 意图仍先于变更持久化。正常逐项提交目标 3N → N，不降低 SQLite 同步级别、不引入批大小或新状态机。
- 备份实际数据保留一次以兑现取消/失败恢复；双次全量校验按 P3 删除。串行、路径边界和内存轮询分别给出保留理由；清理改按实际 archive/stage 来源避免不存在文件的探测，保留旧事务清理能力。
- 故障验证同步覆盖备份阶段和恢复重入、仅内存完成状态、终态保存及旧 journal；所有结论仍为方案，未实施或实测收益。之前 P1–P4 范围说明已被当前 P1–P6 取代，历史记录保留。
## 2026-10-17：按用户决定改为 EXE 直接提取安装

- 用户明确选择“解析 EXE→判断原文件备份→备份→直接提取到游戏目录→成功后删除备份”。当前设计已整体重写，替代旧 P1–P6 局部优化方案；前文保留为历史讨论，不再作为施工任务。
- 核对 sf::write_zip 与 audit::decode_payload：底层可向 writer 输出，现有 ZIP 是外层封装。现有 WorkerRuntime 仍采用完整 JSON 返回，需要实现单会话流式接口；尚未验证 Linux/Windows 接口。
- 新方案无中间 ZIP/stage、无完整临时目标副本、无逐文件数据库状态或恢复哈希；覆盖失败通过预先备份和整份清单恢复。备份就绪/成功记录与必要 I/O 持久化保留，旧事务按旧路径恢复。
- 仅修订文档，未修改生产代码、未部署或重新安装游戏。设计与施工记录链接检查及 diff 检查通过。
## 2026-10-17：用户取消备份环节

- 用户明确认为不需要备份。当前设计改为“解析 EXE→直接提取覆盖→完成”，取代上节五步方案；历史讨论保留。
- 同步移除新路线的备份、回滚、备份清理、恢复摘要及逐文件持久化状态，空间模型不再计入备份；保留旧事务和旧备份的原有处理，避免误删已有恢复材料。
- 失败/取消/中断保留已写入及可能半写文件，显示安装未完成，不假称已还原。重新安装从头执行；双端 UI、收据与重试消费者纳入 D3，Steam 验证提示不承诺清除补丁新增文件。
- 仅更新设计、索引与父功能说明，未改代码、未部署、未删除任何现有备份。文档链接及 diff 检查通过。

## 2026-10-18：D1–D4 实施与自动化验证

### 实施内容

- **D1 解析与流式提取**：`installer-format` 新增 `audit_metadata`，只解析容器/元数据/行为表并按声明长度计入 `expanded_total`，不再为生成安装列表解码全部载荷；完整 `audit` 仅保留给冻结的容器 golden。`installer-worker` 新增 `extract_behavior` 描述符（成员路径、长度、CRC、偏移、压缩标志），按“JSON 成员头 + 原始字节 + JSON 尾帧 + done”向 stdout 输出；`sf_worker` 在提取失败时写 stderr 而不是把错误对象混入成员字节。
- **会话输入**：新增 `WorkerSession`，一次复制 EXE、worker 和平台配置，分析和提取共用；Windows/Linux launcher 增加有界 stdout 流通道，主机在工作线程消费并直接写入游戏文件，接收端停止时终止 sandbox。没有常驻双向协议或通用消息队列。
- **D2 直接写入**：新增 `installer/direct.rs` 流式帧解析与写入器，按成员打开/截断目标、创建父目录、保留已有权限、逐文件同步并汇总目录同步；失败/取消时停止并保留已写入和半写文件。新增 `SpaceBudget::for_direct`/`after_input_copy`，只计算沙箱输入副本、目标净增长和余量；进度来自提取字节数与完成文件数。
- **D3 任务记录与结果**：新增 `Transaction.direct`、计划记录 `begin_direct_at`、成功提交 `commit_direct_at`、失败记录 `fail_direct_at`；新增 `failed` 终态。中断的 direct 任务由 recovery 标记为 `failed`/`install_incomplete`，不恢复文件、不要求恢复材料，`ensure_no_active` 与请求查询不因未完成的 direct 任务阻止重新安装。取消使用 `cancelled_incomplete` 结果，双端文案明确“已写入的文件未恢复”，并提示可用 Steam 校验游戏文件；尚未开始写入目标的失败/取消保留原错误码或普通取消结果。
- **D4 删除旧链路**：删除 worker `convert_behavior`、`write_zip`/ZIP central-directory writer、`validate_artifact`、`retain_plan_archive_at`、EXE `authorization`/`condition` 写入、EXE 重装证据和对应 ignored 测试；普通 ZIP、签名 ZIP、旧 journal/备份恢复路径保留。`ContentBinding::publisher_exe` 仅作为旧授权文件读取兼容保留。
- **协议与客户端**：`TRANSACTION_STATES`/`TransactionState` 与 `OutcomeState` 增加 `failed`，`Transaction`/`InstallerOutcome` 增加 `direct`；重新生成 v4 contracts。共享 UI 增加 `install_incomplete`、`cancelled_incomplete`、`game_write_failed`、`exe_extract_failed` 文案和 `failed` 状态标签；Decky 失败结果优先使用完整安装结果文案，进度关闭提示不再声称一定恢复。

### 验证

- `cargo check --workspace --all-targets` 通过（Windows 宿主；Linux `cfg` 分支仅做静态审阅，未在本机编译）。
- `cargo test --workspace`：37 个测试二进制全部通过。新增/改写的自动化覆盖：元数据报告与冻结 golden、提取帧的逐字节/摘要校验、错误绑定、会话 staging 清理、直接计划/提交、无 backup/stage、直接目标拒绝非普通文件、direct 中断标记失败且可重试、帧跨块解析与错误 CRC 保留半写文件。
- `cargo run -p steam-patch-contract-gen -- check` 通过；`npm run typecheck`、`npm run test:contracts`、`npm test -w apps/decky`（70 通过）通过。
- 未执行：Steam Deck/Windows 实机安装、真实 3.53 GiB/12,141 文件 EXE 的耗时与峰值空间、进程强杀后的实际恢复提示、旧事务恢复的实机回归、游戏内生效确认。D5 仍待验收。

### 已知边界

- 分析/提取是同一 staging 会话上的两个固定 worker 动作，不是常驻单进程；提取动作开始前仍会读取一次完整 EXE 摘要。这样保留了现有一次性 worker/沙箱生命周期，避免新增双向会话协议；耗时收益仍来自取消全量载荷预解码、中间 ZIP 写读、备份和逐文件数据库提交。
- 重新安装不再读取旧收据/authorization 做原文件基线豁免；依赖原始字节的条件在部分安装后可能直接拒绝。此边界与设计 §4 一致，实机需验证常见补丁的重装路径。
- `failed` 任务的 UI 历史标签为“安装未完成”；旧事务仍按原状态显示与恢复。

## 2026-10-18：Steam Deck 部署与“冲突的意志”实机安装

### 构建与部署

- 在 Deck 上用 rootless podman 容器 `rust:1.94.0-bookworm`（daocloud 镜像）按 `Cargo.lock` release 构建，源码、工具链与 target 均放在 `/home/deck/steam-patch-build`，未修改 SteamOS 系统分区。产物：`steam-patch` 14,200,256 B SHA-256 `c7d146a9…e7025c4`，`sf-worker` 1,731,424 B SHA-256 `8ec7ba72…a55b902`；Linux 分支实际编译并运行通过。
- `npm run package:plugin -w apps/decky` 生成 23 文件插件包；本地 ZIP `steam-patch-manager-decky.zip` 7,121,511 B SHA-256 `e115122f…9303bb7`。经 Loader 3.2.10 现有 `DeckyBackend` 连接调用 `utilities/install_plugin` + `confirm_plugin_install`（`file://` 本地 ZIP，reinstall）部署，未使用 sudo、未裸替换插件目录。
- 部署后 `plugin_loader` active，插件实例 1，正式目录 `bin/steam-patch`/`bin/sf-worker` 哈希与新构建一致；Loader 实际返回的 `dist/index.js` 116,648 B SHA-256 `ae1cac7e…4bb3155` 与本地构建一致。旧插件目录备份 SHA-256 `221df156…da2ad2d`（7,235,191 B）已取回本地；远端暂存、构建目录和本轮容器镜像已清理。

### 安装结果（AppID 1011940）

- 任务 `7560b1a3-df04-45fa-a2ec-76d8ca5b0432`，操作 `overwrite`，选择 `exe-selection-80cf34a5e3a1b699c9669979-1011940`；结果 `committed`/`completed`，`error=null`，`direct=true`，`has_replacements=true`。
- 用户点击到任务完成（含下载/分析/提取）：**530.3 秒（8 分 50 秒）**。分阶段：下载约 0–428 秒（EXE 3,788,037,984 B，约 9 MB/s，受当时网络爬升影响）；`analyzing_installer` 约 428–479 秒；`extracting` 约 479–530 秒。
- 安装事务（`begin_direct_at` 的 `created_at` → 提交 `updated_at`）：**49 秒**（1792256463 → 1792256512），完成 **12,141 个文件 / 3,794,744,100 B**。历史同一游戏旧路径 apply 为 1153 秒，本轮事务约为其 **1/23.5**；总时长由旧路径约 31 分钟降为 8 分 50 秒，差异主要来自下载与网络条件，不能直接归因于安装算法。
- 新任务目录为空：无 `*.backup`、`*.stage`、`payload.zip`、`authorization.json`、`condition.json`；journal 中 `archive_source=null`、`created_dirs=[]`、`progress` 全为 `installed`、12,141 个 `installed_sha256` 均存在、`original_sha256` 与 `source` 均为 0。下载缓存清理报告 `removed_bytes=3,820,045,071`，缓存目录为空。
- 游戏目录仍为 13,722 个文件、4.2 GB；12,141 个文件 mtime 晚于安装开始；仅保留正常的 `.steam-patch-manager.lock`，无成员临时文件。安装后 `scan(false)` 仍为 `installed_managed`。
- 第一次尝试在下载 226 MB 时因初始低速取消，任务记为 cancelled，未写入游戏；未计入上表。第二次完整运行成功。

### 待验证

- Windows 真实 EXE 覆盖/新增/混合、取消/失败/进程中断、失败后重试、旧事务恢复实机回归；Deck 上进程强杀后的恢复提示与重试；游戏内实际生效（本轮未启动游戏）；实体手柄/全屏视觉验收。首次下载速度受网络影响，未重复测多轮网络基线。

证据目录：`.artifacts/install-rewrite-deck-20261018/`（任务/轮询 JSON、journal 摘要、部署与安装前后清单、备份、Loader 设置、post-install 输出）。