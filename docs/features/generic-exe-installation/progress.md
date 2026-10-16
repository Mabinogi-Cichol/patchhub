# 通用 EXE 自动安装施工记录

设计：[design.md](design.md)。当前修订计划：[fix-plan.md](fix-plan.md)。调研：[research.md](research.md)。验收审计：[acceptance.md](acceptance.md)。

历史阶段状态：**实施与验收完成。SF9.5–9.7 按安装器行为准入，无新增游戏白名单；四款本地实际安装、重装/失败恢复/存档保护及真实写入取消通过，用户确认四款生效。最终 Decky 部署/RPC/真实页面、合成任务 UI、设备只读 worker 和用户实体手柄验收完成；证据边界见验收审计。**

2026-10-15 当前增量状态：**默认值限制移除已实现，自动化、双平台回归及 Steam Deck 部署/隔离 worker/RPC/真实页面验收通过；未写入实际游戏，历史验收不覆盖本次修订。**

## 任务状态

| 工作 | 状态 | 结果/待办 |
| --- | --- | --- |
| 现有代码与白名单限制调研 | 已完成 | 核对 registry、resolve、事实核验、worker、安装任务与 UI |
| 第一方资料与妃神发布页核验 | 已完成 | 页面能力与来源声明已核验；完整包/格式/行为验证另见 P0–P2 |
| 独立设计与修改方案 | 已编写 | 按行为授权、条件 IR、选择/内容身份、缓存、迁移、实施拆分与验收 |
| P0 样本二进制实验与协议固化 | 已实现并本地验证 | 三个官方完整 EXE 的对象图、行为 IR、选择/内容身份与 DTO 已接通；四款身份已核实 |
| P1 格式与元数据 | 已实现并本地验证 | 顺序解析完整对象图与受限 Lua AST，参数化条件 IR；未知行为阻断；35 项相关测试通过 |
| P2 分析服务与 worker | 已实现并本地验证 | analyze_behavior / convert_behavior 共用真实 bwrap；三个大型官方包转换摘要正确 |
| P3–P4 授权与产品流程 | 已实现并验收 | 动态来源、Rust 实际写入、重启路由；后端 EXE 运行 69 项，67 通过、2 跳过；前端 44 项通过 |
| P5 自动化与实机验收 | 已完成 | 四款本地矩阵与用户游戏内确认、Deck 部署/RPC/真实页面、合成任务 UI、只读 worker、用户手柄确认及清理完成 |
| 2026-10-14 安装器默认值限制移除 | 方案已修订，待实现 | [修复计划](fix-plan.md)；本轮仅文档，代码/自动化/正式 worker/部署与实机验收未执行 |

## 2026-10-07

- 用户明确仅调研和撰写方案，不直接施工。按“已支持安装器行为”定义范围，取消逐游戏/逐版本人工适配作为新安装的必要前提。
- 静态读取当前源码，确认生产 registry 只有三条 profile，worker 仍依赖人工语义批准与固定摘要；审计器保留未知字段和未解析事件，不能简单删除白名单守卫。
- 查阅 Indigo Rose、innoextract 和 NSIS 第一方/原作者资料；读取妃神会秘史官方发布及安装说明。未下载完整官方 EXE，浏览工具打开其一个二进制入口未成功；未运行任何安装器/脚本、未连接设备。sfextract 历史固定 commit 页面本轮未成功抓取。
- 新建本功能 `design.md`、`research.md`、`progress.md` 并更新索引。旧 EXE 设计和施工历史不改写；新方案保留旧记录兼容，同时遵循已实施一站式安装的成功后临时材料清理策略。
- 没有运行插件测试、构建、安装、部署或 Git 提交；只执行文档结构与差异检查，结果见下。

## 文档检查

- 新文档与功能索引共 45 条本地相对链接全部存在。
- `git diff --check` 通过；新文档未发现冲突标记或行尾空白。Git 的 LF/CRLF 提示不属于检查失败。
- `git status --short` 确认改动仅为本功能三份新文档与功能索引，未修改插件代码。
- 文档检查仅证明引用与编辑范围，不证明行为解析可行性或安装功能通过。

## 实施前重点

- 先用跨游戏样本实验确认序列化字段和事件可以完整解析，输出参数化行为模板及具体阻断原因；不得通过第四条游戏专属 profile 验收。
- 查明妃神当前包格式、Steam 身份、实际事件与条件。首部/页面信息不足以证明自动安装能力。
- 固化新选择/内容 ID、分析报告/条件 IR、缓存和 receipt 版本；核对旧协议迁移、已安装检测和内容更新重装边界。
- 设置扫描流量/时间和 AST 分析预算；避免全库自动下载大包，未完整检查的条目不显示为确定支持。

## 2026-10-07：本地验证 case 与平台分工补充

- 用户提供本地四款游戏：你妈真漂亮、妻子变美的理由、妃神会秘史、众人觊觎的女友，并明确可在后续开发过程中直接使用这些游戏验证。
- 更新设计 P0/P5 与验收矩阵：四款全部参与本机格式、条件、安装、重装和失败恢复验证；Steam Deck 仅为最后阶段，主要验收 Decky 前端交互，补充 Linux worker/RPC/打包集成证据。
- 明确本地 Windows 与生产 Linux 隔离环境的差异；实施先核查本机已有 WSL2/容器，生产 sandbox 不得关闭。开发验证不自动扩展为 Windows 客户端功能交付。
- 当前授权范围仍为方案维护：没有扫描游戏路径、执行样本、修改游戏、实现插件或连接 Steam Deck。


## 2026-10-07：施工授权与多版本格式验证

- 用户明确“开始施工”，并补充你妈真漂亮目录为 F:\SteamLibrary\steamapps\common\Kudo’s Kuppa；此前“仅方案”记录是历史授权状态，不再代表当前范围。
- `sf95_audit.py` 加入显式 9.5/9.6/9.7 布局选择、有界 MFC CString（含宽字符和长长度）读取；9.7 额外文件记录字节保留为待解释证据。默认审计/旧转换仍仅接受 9.5，不扩大旧 profile 许可。
- 新增 `sf9_metadata.py`：按 framing 解码前置 include 数组、脚本与选择器，诊断性读取类声明及事件。事件引用扫描不能替代完整对象图，报告固定标注覆盖不完整和 installable=false。
- `sf95_worker.py` 与后端 `exe_worker.py` 增加严格只读 analyze 协议，绑定包 SHA/长度，校验前后文件身份；保持离线、非特权 bwrap、进程取消和既有资源限制。不执行安装器/Lua/Wine，不输出规范化 ZIP，不访问游戏目录。
- 真实 Windows manifest 带有转义路径；修复只读 KV 解析器以接受双反斜杠/转义引号，同时继续拒绝未知及控制字符转义。
- Decky 与通用产物打包、SBOM 清单纳入新元数据模块及摘要。尚未构建/部署新插件。
- 分析单文件上限由 1 GiB 调整到 2 GiB：真实妃神 data.xp3 为 1,373,855,065 字节，众人 patch.xp3 为 1,296,894,503 字节。总展开 2 GiB、地址空间 3 GiB、180 秒 worker 上限保持。旧 profile 的 1 GiB 许可限制未改。

### 本地样本与证据

| case | AppID / build / 语言 | 实际来源与格式 | 本轮结果 |
| --- | --- | --- | --- |
| 你妈真漂亮 | 2743810 / 23866175 / english | 现有 patch.moe collector 对应官方 ZIP；不是 SF 样本 | 已核实目录/manifest/候选；ZIP 安装回归待执行 |
| 妻子变美的理由 | 3721970 / 21518806 / schinese | 官方 CN v1.01，SF9.7.0.0 | 完整 760,207,000 字节 EXE；data.xp3 914,931,059 字节；沙箱约 27.822 秒 |
| 妃神会秘史 | 2303800 / 10821988 / schinese | 官方 CN v4，SF9.5.2.0 | 完整 1,228,278,216 字节 EXE；data.xp3 1,373,855,065 字节；沙箱约 46.471 秒 |
| 众人觊觎的女友 | 3606360 / 20231092 / schinese | 官方 CN v1.00，SF9.6.0.1 | 完整 1,162,243,456 字节 EXE；patch.xp3 1,296,894,503 字节；沙箱约 44.918 秒 |

- 三个完整官方 EXE 均在本机 WSL Ubuntu 的非特权 bwrap 中完成 PE/版本、长度、受限解压、载荷 CRC 与 SHA 校验，分别诊断读取 59 个事件；均为 coverage.complete=false、installable=false、输出文件数 0。不能把这解释为完整事件覆盖或安装通过。
- EXE SHA-256：妻子 `67d258e9cad0f86649282862d2439fb4a796946f7a6182bcfe6880ce855182a8`；妃神 `7336c5c9876bf239811478001741862256610c77298c745d3bc035a8bcdd94f3`；众人 `6a1018168add626642036a70394d30562724aac645985bfefadb2f235c04f42d`。摘要仅用于本次内容绑定，未加入游戏许可名单。
- 忽略的本地 `.artifacts/generic-exe-local/real-worker-check.json` 保存摘要/时长；各完整分析报告及官方 EXE 留在本地实验目录，不将游戏资产提交 Git。
- 本机使用既有 WSL2 Ubuntu，并安装 bubblewrap 0.6.1 完成实际沙箱实验；未连接 Steam Deck。未运行安装器或游戏，未更改游戏目录、存档或原补丁。

### 自动化验证与未完成范围

- SF 审计/元数据/worker 测试 25 项通过；后端既有 EXE 测试阶段性 54 项通过（2 项跳过），新增分析协议 2 项通过，更新后的事实解析 11 项通过（1 项跳过）。跳过项与当前 Windows 平台能力有关，不算通过。
- 最终相关后端 EXE 测试共 57 项通过（2 项跳过）；本功能文档及索引 47 条本地链接检查通过。
- SBOM 1 项、产物打包 7 项通过；真实合成 SF9.7 worker 在 bwrap 中验证只读且无输出。差异检查通过。
- 尚未完成完整序列化对象图、覆盖/选择属性解释、受限 Lua AST 与行为 IR、来源条件动态授权、内容 ID/收据迁移、转换/RPC/UI 串联。
- 当前安装路径仍要求旧三条 profile，**新游戏自动安装尚未开放**。没有新增第四条游戏配置，也没有用检测到 SF 版本/读取到文件表直接授予安装权。
- 四款本地安装/重装/取消与故障恢复、游戏内生效、最终 Decky 前端验收均待执行。


### 施工期间产品约束修订

- 用户明确一站式自动安装，不还原 EXE 向导，要求跳过条款页。按当前授权移除新增的条款等待/RPC/同意按钮；不模拟用户同意。随后进一步明确条款信息无需用户理解：条款只作为内部解析证据，移除前端 DTO 中的条款字段和要求阅读/同意的提示。游戏文件、CRC、来源语言和 DLC 等真实安装条件继续自动核验。
- 本次界面约束检查：`test_exe_installation` 18 项运行通过（其中 1 项平台相关跳过），覆盖返回安装选项不含条款字段；Decky TypeScript 类型检查通过。此结果仅证明本次接口/界面调整，不代表四款游戏的完整安装或最终实机验收已完成。

### 持续施工目标与完整链路补齐

- 按用户要求建立持续执行目标，完成标准为通用行为准入、本地四案例安装及必要的重装/取消/失败恢复/存档保护验证、最后阶段的 Decky 前端验收；不会以格式检查通过结束目标。
- 新增 `sf9_graph.py`、`sf9_lua.py`、`sf9_behavior.py`：顺序消耗完整序列化图、解析所有 include/event AST，静态转换成只读文件存在/CRC 条件，不执行 Lua 或 EXE。三个真实版本样本均解析 59 个事件及 2 个 include，条件为各自包内的 Script.vdf CRC 与 Game.exe 存在；没有新增游戏专属 profile。
- 补上共享运行时标志/常量的事件间修改检查，以及所有进度阶段的动作检查；相应 AST/转换测试 8 项通过。
- 新增动态官方来源选择与内容身份，通用 worker 转换重新验证完整图和行为摘要；解析和转换工具加入产物/SBOM 清单，真实转换验证正在执行。
- 重启后持久化文件只用于找回来源，不授权安装；重新安装须重新分析且匹配原内容身份。相关后端测试 19 项运行通过（1 项平台相关跳过），包括路由不授予行为权限及来源变化阻断。Decky 类型检查通过。
- 本地 WSL Rust 核心只读扫描 F 盘真实库成功，返回目标游戏实例且无扫描诊断；为后续真实准备/写入验证建立本地 HOME，不连接 Steam Deck。
- 仍待：完整来源发现与转换/计划/应用串联、实际本地四案例矩阵、最终 Decky 验收。历史阶段记录中“仍要求旧 profile”等表述保留为当时事实，以当前任务表和本段为准。

### 本地真实来源、转换与首次安装

- 本地三个完整官方 EXE 通过 `analyze_behavior` 与 `convert_behavior` 的真实 bwrap 调用；转换分别生成妻子 `data.xp3`（914931059 字节）、众人 `patch.xp3`（1296894503 字节）、妃神 `data.xp3`（1373855065 字节）。转换后的 SHA 与完整包内逐文件校验一致，记录在忽略目录 `.artifacts/generic-exe-local/real-convert-check.json`。
- 实际来源发现暴露 Ubuntu 22.04 Python 3.10 的 HTTP 308 兼容问题，已在带域名限制的发现跳转处理器中修复；没有扩大目的域名或协议范围。重新查询后，三个 EXE 官方来源均经索引身份交叉核验关联到实际 Steam AppID。新增跳转目的地测试通过。
- 使用仓库已有隔离 Rust 1.94 工具链构建当前 Linux release 核心；此前本地旧二进制仍返回协议 v1，现已更新为当前桥接要求的 v2。
- 用空 EXE profile 注册表、真实已发现来源、动态条件分析、真实沙箱转换、签名目录和 Rust 核心完成“妻子变美的理由”的实际安装。内容 ID 为 `exe-content-6185bea8e26300eb2e68272a-3721970`，持久化事务状态 `committed`；唯一补丁目标 `data.xp3` 的 SHA 为 `7aa1c726101a089be9f1955e62cb07963b416016badf95acfb74c6a5945f2564`。
- 原有非目标文件（含游戏目录存档）逐文件 SHA 未变化；核心创建的空 `.steam-patch-manager.lock` 单独核验。原文件备份位于忽略目录 `.artifacts/generic-exe-local/original-backups/3721970/`，安装证据为 `3721970-installed.json`；正常提交后按一站式策略清理插件临时缓存，来源关联保留。此结果证明实际写入/载荷/文件保护，不宣称已经验证游戏内画面。
- 补齐同进程及重启后重装都重新分析的入口，通用准备过程只下载一次，然后复用本次已分析且重新哈希核对的输入；合成来源到签名计划、apply 核验及重启后重装的集成测试通过。
- UI 对通用条目显示“安装时自动检查条件”，未检查时摘要不宣称“有可用补丁”；自动安装选项代替其自身重复的来源说明，错误消息不要求人工审核。Decky 44 项测试及类型检查通过；相关后端安装测试 20 项运行通过（1 项平台相关跳过），通用事实/发现测试 40 项通过。
- 下一步：其余本地案例安装，真实重装/取消及必要失败恢复，最后阶段 Decky 前端部署验收。目标继续执行，尚未完成。

### 四款本地实际安装与当前验收

- 四款均通过生产后端、签名目录与当前 Linux Rust 核心实际写入，终态均为 `committed`。三个 SF9 EXE 使用空 profile 注册表，通过相同对象图/AST/行为规则生成授权；没有新增 AppID/build/样本摘要许可配置。你妈真漂亮的官方来源实际是 ZIP，沿用官方 ZIP 流程验证回归。

| case | 实际载荷/内容 ID | 本地结果 |
| --- | --- | --- |
| 你妈真漂亮 | 官方 `Kudos_Kuppa_patch.zip`，根目录 `patch.xp3` | 通用包装层修复后重新实际提交；载荷摘要一致，非目标文件/存档不变 |
| 妻子变美的理由 | `exe-content-6185bea8e26300eb2e68272a-3721970`，替换 `data.xp3` | 实际提交；载荷摘要一致，非目标文件/存档不变 |
| 众人觊觎的女友 | `exe-content-68cf92d90800940825751c4a-3606360`，新增 `patch.xp3` | 实际提交；原 `data.xp3` 保留，其他文件/存档不变 |
| 妃神会秘史 | `exe-content-6dcc185a0fd1bf8c5a96312b-2303800`，替换 `data.xp3` | 实际提交；载荷摘要一致，非目标文件/存档不变 |

- 每款证据为忽略目录 `.artifacts/generic-exe-local/<appid>-installed.json`；原有目标文件的开发备份与初始文件清单位于 `original-backups/<appid>/`。成功后清理产品临时缓存并保留内容来源路由。未启动游戏，游戏内画面/实际补丁效果仍待单独验证。
- 对通用写入增加存档/用户数据路径阻断（save/saves/savedata/userdata/user_data 目录及 .sav/.save 文件）；这属于路径行为边界，不是游戏规则。对应行为测试 10 项通过，SF 审计/元数据/worker/行为合计 35 项通过。
- 在原内容上重装仍重新分析；修正旧选择 ID 与当前内容 ID 的 RPC 映射，取消/失败回滚后释放 EXE 授权租约。后端 EXE 测试运行 68 项，66 项通过、2 项平台相关跳过；前端 44 项及类型检查通过。新插件完整构建已成功。
- 真实 RPC 取消第一次尝试因另一个本地案例正在写入而在扫描阶段返回 `installer_busy`，没有进入重装写入；保留并发守卫，其他案例完成后串行重试。此次失败不算取消/回滚通过。
- 为本地大型样本设置明确 worker 预算：通用行为分析 240 秒墙钟、转换 480 秒墙钟、单进程 CPU 300 秒，地址空间仍为 3 GiB；旧 profile 转换墙钟仍 180 秒。真实设备的大包预算仍待验收，不由本机耗时推定。
- 真实 `start_patch_install_task` / `cancel_task` 验证完成：在妻子案例重装写入 21,954,560 字节后取消，经历 `writing → rolling_back → cancelled`，Rust 返回 `rolled_back`。全部游戏文件名及 SHA 与重装前完全一致；证据 `3721970-rpc-cancel.json`。只读进程可取消，Rust 写入进程通过协议接收取消并自行回滚，没有强杀写入进程。
- 重新构建完整 Decky 插件及 SBOM，727 个依赖组件都有许可元数据；SBOM/打包测试共 8 项通过。完整新包 `.artifacts/steam-patch-manager-decky.zip` 大小 4,647,893 字节，SHA-256 `d6a5730cf603a8eaa327421fca8ef6b5fd21020ea2eebc6b8d4e1e5610fb76ea`。尚未部署设备。

### ZIP 回归发现与修复：实际安装目标不可由提交成功代替

- 复核你妈真漂亮包内说明，要求将 `patch.xp3` 复制至游戏根。此前 ZIP 配方保留 `Kudos_Kuppa_patch/` 外层目录，事务虽然提交、摘要正确，游戏根却没有补丁。此前“四款实际安装”只证明文件事务；此 ZIP 案例不能按补丁安装生效验收，明确撤回其完成判断。
- 增加不依赖标题/AppID 的单层 XP3 补丁布局规则：现场根 `data.xp3` 与包内 `patch[数字].xp3` 均需实际 XP3 文件头，所有文件须位于唯一单层目录，其他成员只允许说明文档。满足才去掉包装层、仅安装载荷；未知混合成员/错误签名阻断。既有根布局、其他引擎及嵌套布局不猜测路径。
- 官方 ZIP 后端测试 15 项通过，包含任意包装目录名、原补丁摘要、不同引擎保持原路径、未知成员和损坏 XP3 拒绝。
- 续接确认上一轮恢复验收进程已退出，没有产生四款重装/恢复完成证据；先使用 Rust recover/discard 核对遗留事务。错误实验文件按原收据摘要核对后保留到忽略目录，旧状态整体留存；不删除旧证据，不触碰既有文件或存档，再以正确路径重做 ZIP 验收。

### 同内容重装失败回滚修复

- 真实 ZIP 暂存载荷故障注入暴露核心缺陷：现场目标内容与本次计划原 SHA/安装 SHA 完全相同，进度仍为 pending，未备份也未写入；回滚却把相同的已安装 SHA 当成需要恢复的修改，误报 `backup_missing_or_changed`，进入 `needs_recovery`。
- 只读 SQLite 记录确认两项进度均 pending、原/安装摘要均相同。修正 Rust `restore`：只有目标已经偏离本次原图像时，才要求用备份恢复；现有冲突/未知备份/临时文件校验继续执行。
- 新增真实核心回归 `same_content_reinstall_failure_before_backups_rolls_back`，首次安装成功、同内容重装首个 stage 损坏后必须 rolled_back，保留首次 committed 收据及原有完整载荷。该测试通过；完整安装事务回归及本地遗留恢复正在执行。

- 核心完整安装事务回归 39 项全部通过，新 Linux release 核心构建成功。实际遗留 `needs_recovery` 在修复后经 Rust recover 转为 `rolled_back`，没有改动目标内容；恢复证据保留于 `.artifacts/generic-exe-local/incorrect-kudo-layout/recovery.json`。
- 正确布局再次安装你妈真漂亮完成：唯一目标为游戏根 `patch.xp3`，840,228,417 字节，SHA `28ad37cd440726e353af622c98aa024f28a19bc77cd0963865c187245dcb0c6f`；事务 `6ba88a98-1c40-4f50-af8c-3157931544e3` 为 committed。说明文档不写入游戏目录；既有文件/存档完整保留，新证据覆盖当前 `2743810-installed.json`，错误历史证据另存。
- 四案例真实暂存损坏恢复与同内容重装矩阵再次串行运行，尚未计作通过。游戏内画面确认已向用户请求；本环境无法读取本地游戏画面，不能用文件摘要代替此项。

- 你妈真漂亮正确根布局的完整矩阵已通过：损坏暂存 `0.stage` 后 rolled_back，错误 staged_file_changed，全部文件与任务前一致；随后同内容重新计划并 committed，文件集合/摘要及存档保持完全一致。证据为 `2743810-fault-recovery.json` 与 `2743810-reinstall-recovery.json`。其余三个 EXE 的相同矩阵继续执行。
- 当前冻结的新包（包含本轮 Rust 和 ZIP 修复）大小 4,648,443 字节，SHA `e8d2b355bc78c223a7f0cb550ae070a5aa65729934cecf9648c82795c07e8632`；逐文件清单 38 项，Linux CLI 文件头确认为 ELF。100 条本地文档链接全部存在。先前包摘要保留为历史，不能用于本轮部署。

- 妻子案例的真实 stage 损坏阶段已通过：事务 a769e342-8534-4890-981e-4e63a4c95cf5 返回 staged_file_changed / rolled_back，文件集合及 SHA 与任务前完全一致，证据 `3721970-fault-recovery.json`。验收脚本随后发生缓存路径变量被返回的字符串覆盖的问题，未进入重装写入；脚本已修正，核对原 rolled_back 事务后续跑正常重装，已通过的你妈真漂亮矩阵不再重复。

### 用户游戏内验收与余下矩阵

- 用户实际启动四款本地游戏，明确反馈“都可以正常启动并生效”。记为用户实机确认，与开发侧载荷/事务验证分开；没有伪造本环境游戏画面或把文件复制代替此项。
- 妻子正常同内容重装实际已 committed（787b6914-66df-4c01-a9c0-86955777d2ea），唯一目标 data.xp3 的原/安装 SHA 相同。用户启动验证与文件比较重叠，新出现 savedata/steam_autocloud.vdf，使严格全目录相等断言失败；只读比对首次快照，所有原有非目标文件和存档均未变化，新增文件完整保留。此项证据明确记录外部新增，不声称期间文件集合完全一致；证据 `3721970-reinstall-recovery.json` 和 `3721970-post-game-launch-diff.json`。
- 已核实没有四款游戏的运行进程。妻子完成租约释放和成功清理；只续跑众人与妃神的故障/重装矩阵，额外落盘每项任务开始前的完整摘要，以便任何外部变化都可准确核对。

- 补齐旧适配未覆盖新 Steam 构建时的通用检查回退：当前候选重新核验来源并走行为分析，旧收据/旧 release 解析仍保持原绑定，禁用或退订来源仍不能安装。EXE 后端回归 69 项运行，67 项通过、2 项平台跳过。
- 包含此回退的最终候选包大小 4,648,559 字节，SHA `41c7536b5b0e25098da5acb837cd980d336dbdf08cd891e3f5c38fc25152bb12`；重新冻结 38 文件清单。此前摘要均为历史。

- 众人案例完整故障/重装矩阵通过：stage 损坏返回 staged_file_changed / rolled_back，随后重新分析、转换、计划并 committed；文件集合和全部 SHA 与任务前完全一致，存档未变化。证据 `3606360-fault-recovery.json`、`3606360-reinstall-recovery.json`。最后剩妃神案例的同组本地验证。

- 妃神案例完整故障/重装矩阵通过：stage 损坏后 rolled_back，重新分析转换并 committed；文件集合及全部 SHA 与任务前完全一致，存档未变化。证据 `2303800-fault-recovery.json`、`2303800-reinstall-recovery.json`。四款本地验收完成，正式进入最后阶段的 Steam Deck 前端验收。

### 最终 Decky 验收：设备身份待核实

- 本地四款全部完成实际安装、同内容重装及 stage 损坏恢复；真实 RPC 写入取消完成，用户明确确认四款正常启动且补丁生效。当前代码和最终候选包已准备完成。
- 首次最终设备只读连接被 SSH host-key 校验阻止：历史 `192.168.31.186` 保存的 ED25519 指纹为 `SHA256:b77rklEC2scaeGfv0qeSbl7JCWPizDbzZ6APlCDF9So`，本次返回 `SHA256:1eQWToCx6mKr5lz/YPXgwit390H7XTkWcpR56L0mJx8`。核对原可信记录后确认不匹配；没有接受新密钥、关闭校验、连接成功或写入设备。
- 已请求用户提供当前 Deck 地址及本机指纹/重装情况；身份确认前仅停止设备部署，保留全部本地结果。最终 Decky 备份/部署、真实 RPC、屏幕和实体手柄验收仍待完成，不宣告目标完成。此问题是 SSH 身份校验失败，非自动审批拒绝。

- 续接核对本机全部 known_hosts 可信指纹，没有本次新指纹的匹配记录；未更改信任或尝试其他设备。已按设计 7 项门槛整理 `acceptance.md`，直接核对四款当前安装/恢复/重装 JSON、空 profile 注册表和真实取消证据，明确区分本地通过与最终设备待验收。

- 第三轮阻塞复核：对既有地址再次使用 StrictHostKeyChecking=yes 的只读身份连接，仍返回同一新指纹并被 host-key 校验拒绝。当前可信记录没有更新，用户尚未提供当前地址/本机指纹；剩余设备验收无法安全推进。目标进入设备身份阻塞，代码、本地证据和冻结包完整保留；不是完成、取消或用户请求暂停。身份确认后继续原完整目标。

- 用户反馈当前 Devkit 可正常连接后，进一步读取当前官方客户端源码和只读本机 API：正在运行的 Devkit 选中 steamdeck/192.168.31.186/login deck，配对密钥路径一致；官方客户端的 SSH 默认 AutoAddPolicy/StrictHostKeyChecking=no，因此正常连接并不更新命令行原可信指纹。已获取公开新密钥并准备本次独立 known_hosts.proposed，未修改全局记录或部署；等待用户确认该选中设备就是当前目标 Deck。Windows 自带 keyscan 不兼容设备 KEX，改用本机现有 Git OpenSSH 只读获取成功。

### 最终 Decky 部署与真实界面验收

- 用户已明确确认 Devkit 选中设备与本次公开指纹并授权继续验收；使用本轮独立 known_hosts 和 StrictHostKeyChecking=yes 成功连接，未修改全局旧可信记录。此前身份阻塞已解除。相应排查流程写入项目部署技能，已有确认不重复询问。
- 首次有效备份已取回本机并逐包核验：旧插件、外部状态、Loader 设置及完整回滚 ZIP。完整新包通过 Loader 当前正式安装接口完成替换及后端重启，正式插件 39 文件逐一大小/SHA 匹配、Linux CLI 可执行、Loader active。先前 38 文件清单仅覆盖构建输出，补上打包产生的 SBOM 后为 39；ZIP 字节和冻结 SHA 未变化，没有把漏项清单当成完整验收。
- 真实 HTTP bundle 为 95,733 字节，SHA `fefd5f43e63293a52d087a847bc55a1af1e114d442dfc9e23f71c8f65da5d9b4`；实际挂载组件对应新实现。唯一插件实例；真实扫描 38 实例，2 个终态任务、8 个既有事务，旧 VBF 收据可读且侧栏显示已安装。安装接口移除本插件排序后已恢复原位置，其他插件/来源选择未改变。
- 妃神中文/英文 EXE 通过实际 reviews RPC 返回动态 check_required，profile_id 为空、blockers 为空；真实安装页显示“安装时自动检查条件”“自动提取并安装”和安装按钮，没有条款界面。本轮没有点击安装或写入 Deck 游戏。
- 真实 QAM 有补丁筛选显示 6 游戏；全屏概览/操作记录/订阅源页面及安装页已检查。CSS 视口 854×534，页面 scrollHeight 1160/clientHeight 534，底部 scrollTop 626，安装按钮完整可见，根宽度无横向溢出。顶部/底部及返回重进截图已读取查看。
- 返回重进第一次自动化尝试在导航未完成且 QAM 停留通知标签时失败，未算通过；重新读取真实 DOM、选择 Decky 标签，再点击妃神后正确进入 AppID 2303800。筛选及 6 游戏列表保持。
- 剩余：实体手柄 A/B/焦点/滚动、实机任务进度与取消展示/运行中重进、设备大型 worker 资源预算、验收后的临时目录与隧道清理。已向用户请求实体手柄反馈及大包只读上传授权。自动审批拒绝约 1.2 GB 官方样本上传，理由是既有授权未明确包含此次载荷传输；没有换通道上传或执行。只读验证脚本已在本地准备，等待明确授权。
- 本轮证据/备份保留于忽略目录 `.artifacts/generic-exe-deck-20261007-1333/`；当前目标仍为验收中，未宣告完成。

### 最终补证与收尾

- 用户明确反馈 Deck 实体手柄 A/B、焦点/滚动和重新进入“操作正常”，完成实体操作验收；此前 DevTools 检查与此反馈分别保留。
- 用户明确允许上传官方大包进行只读验证后才执行。已部署生产 worker 以 uid 1000 在 bwrap 隔离中分析/转换；输入 1,228,278,216 字节，分析 28.48 秒/转换 55.98 秒，分别小于 240/480 秒墙钟预算。行为摘要与本地一致；载荷 data.xp3 为 1,373,855,065 字节，SHA `98ea8336e208709c166fe87bd13bbf644a2c94f5633f09e9f18e74012f641d4f`，与本地一致。没有执行 EXE/Lua、读取游戏或调用 Rust 写入。
- 实际生产前端以明确标记的合成任务验证进度、关闭窗口、退出重进、取消恢复阶段与终态关闭。首轮夹具误把 SDK 字符串参数当数组，未知合成 task ID 请求失败但真实任务无变化；另一次从首页捕获路由的夹具错误未挂载目标页。这些尝试不计作通过。修正夹具后重新完整执行，所有前端断言通过；恢复原 API 后真实任务仍是原来的 completed/cancelled 两条。合成数据不作为下载速度或文件回滚证据。
- 独立重新读取远端规范化 ZIP 和 data.xp3 验证 SHA，全部本地备份与远端摘要一致。确认精确绝对路径、无链接且不涉及插件/state/游戏后，清理本轮唯一远端目录，共 2,618,818,691 字节；恢复 Steam 首页/菜单，停止唯一匹配本轮的 SSH 隧道，本地首次备份及证据保留。
- 完成审计直接读取四款 installed/fault/reinstall JSON 的实际终态与保护断言、真实 RPC cancelled/文件恢复、39 文件清单、完整备份 SHA、实际 worker 结果、真实/合成 UI 与清理记录；全部本期要求有对应范围的证据。测试结果沿用本开发 session 已通过记录，本轮仅新增验证及文档，无产品代码修改，不重复跑已通过的全套测试。
- 最终状态：本功能实施与验收完成，未提交 Git；按未知安装器行为具体阻断的边界保留，不宣称支持所有 EXE。

## 2026-10-07 列表状态口径修订

- 用户反馈：妃神会秘史（AppID 2303800）列表显示“找到补丁来源”，但详情可自动安装，与旧文案“找到补丁来源表示工具暂不支持自动安装”矛盾。核对后确认该游戏为无逐游戏审核配方的通用 EXE，`check_required` 是“安装时自动检查条件”，详情安装入口可用。
- 用户确认方案 A：详情可自动安装的通用 EXE 应在列表显示“有可用补丁”。据此修订前端汇总判定，未改后端能力判定与安装流水线。
- 代码改动：`packages/ui/src/index.ts` 的 `installSummary` 去掉 `compatibility !== "check_required"` 门槛，改为凡 `canAutoInstall`（无 blockers、有下载能力、非 HGamer）即“有可用补丁”；`compatibility` 保留给详情显示。HGamer fallback、存在 blockers 或 reviews 为 guidance（不进入 patches）仍归“找到补丁来源”。第 107 条施工记录中“未检查时摘要不宣称‘有可用补丁’”的口径由本条目取代。
- 测试：`apps/decky/tests/install.test.ts` 更新 `check_required` 用例为“有可用补丁”，并新增 HGamer、blocker 仍为“找到补丁来源”的断言。
- 文档：本功能 `design.md` 第 5.2 与第 8 节追加 2026-10-07 修订说明；`simplified-install-flow` 的状态说明与 README 同步。

### 2026-10-07 部署与实机验证（列表口径修订）

- 构建：`npm run build:decky`。前端 `apps/decky/dist/index.js` 97,720 字节 SHA-256 `31707dd84f6bd1e0bd4262dcb8f6a6a39fa93d7c3923b907b4b0a23db0d2ed09`，`index.js.map` 1,583,748 字节 SHA-256 `5adff7e66c55f97c7ce64051f86f96ef4e2b8ca0f0814519e5ab57bc843704e7`。`npm run test -w apps/decky` 51/51、`npm run typecheck -w apps/decky` 通过。
- 设备与授权：沿用已确认的 Devkit 设备 `steamdeck`（`deck@192.168.31.186`），独立 `known_hosts` + `StrictHostKeyChecking=yes`。设备时间 2026-10-07 16:5x。
- 部署前门槛：`list_tasks` 3 条全部终态（completed/cancelled），插件实例 1。插件根 `/home/deck/homebrew/plugins/steam-patch-manager` 为 `root:root 755`，`dist/` 为 `deck:deck` 可写，故采用前端增量替换：上传到 `/home/deck/steam-patch-validation/statuslabel-20261007-165048/`，逐文件 SHA 核对后原子 `mv` 进 `dist/`；旧 `dist/index.js`（`80e7cdb5…`）与 map（`21b851e9…`）备份到 `.artifacts/statuslabel-deck-20261007-165048/`。
- 重载：`DeckyPluginLoader.importPlugin('Steam 补丁管理','0.1.0')`；实例严格为 1，`hasPlugin=true`，version 0.1.0。实际 HTTP bundle `http://127.0.0.1:1337/plugins/Steam%20%E8%A1%A5%E4%B8%81%E7%AE%A1%E7%90%86/dist/index.js` 97,720 字节 SHA-256 与本地构建一致。
- RPC：`scan(false)` 返回 40 实例；AppID 2303800 行 `patches`：kagura-cn EXE（`publisher_exe`、`auto_installable=true`、`blockers=[]`、`compatibility=check_required`）与 kagura-en EXE（`exe_language_mismatch` 阻断），按新判定归“有可用补丁”。
- 真实界面：QAM 游戏列表显示“妃神会秘史 有可用补丁”；全屏概览“补丁状态：有可用补丁”；补丁管理页仍显示兼容性“安装时自动检查条件”、安装方式“自动提取并安装”与“安装补丁”按钮。截图 `qam.png`、`main_overview2.png`。
- 未点击真实下载/安装；游戏文件未写入。实体手柄 A/B/焦点、真实下载/取消/恢复仍待验收。部署后 `list_tasks` 仍为 3 条全终态。
- 清理：删除远端暂存目录 `statuslabel-20261007-165048`；正式 `dist/index.js` 复核为新 SHA。本地首次备份与证据保留于 `.artifacts/statuslabel-deck-20261007-165048/`。

## 2026-10-14：夏日 DLC 故障取证与计划修订

- 前一轮按用户授权通过当前 Devkit 配对和已有可信独立 known_hosts 只读连接设备。任务记录确认两次夏日 DLC 安装/覆盖在 analyzing_installer 失败，下载缓存 EXE/元数据摘要与历史样本一致；未进行正式安装、重载或部署。
- 纯静态、仅驻留内存的 Python 参考诊断逐项定位尾字节必须为 0、runtime_folder 必须精确为 Archive 和条件子路径误用 basename 校验。诊断中调整三处假设后完整图/行为可解析；不等同于正式 Rust worker 或实机安装通过。原始证据在忽略目录 `.artifacts/summer-memories-diagnosis-20261014/`。
- 用户进一步明确移除多余防御性策略，随后要求先修改计划、不要施工。中断前调用只有只读命令；本轮编辑前 git status --short 为空，确认没有代码改动。
- 新建 [fix-plan.md](fix-plan.md)，将旧“先逐字段审核，再扩展允许取值”路线改为删除样本默认值准入，按结构读取配置、按真实载荷选择与提取操作处理。范围覆盖项目/界面/卸载、属性/时间戳、附带配置、名称启发式及路径条件，不局限于夏日三处错误。
- 同步设计当前决策和功能索引，保留既有历史验收事实。仅进行文档差异与相对链接检查；代码、测试、构建、正式 worker 分析/转换、设备连接、部署与游戏写入均未在本次计划修订中执行。
- 后续状态：限制清单与代码调整待实现；参数变化回归、正式 Rust 样本矩阵与实机验收待验证。用户尚未授权本轮施工，不能按已修复汇报。

- 本轮文档检查：4 份计划/设计/施工/索引文档的 83 条相对链接均存在；git diff --check 通过。新增任务表链接复核存在。检查仅证明文档引用与编辑范围，不证明插件修复完成。

## 2026-10-15：默认值限制移除施工

- 用户明确授权“开始施工”后实施，共享 Rust 格式/行为、报告、编排及 worker 同步修改；无逐游戏分支、摘要许可表或新增用户确认流程。
- graph/audit 移除尾字节、普通项目/界面/卸载配置、文件原始属性等样本值断言，读取保留配置；behavior 移除 Archive 精确匹配、属性/时间戳/opaque 默认字节、空 shortcuts/icon/font、save 文件名启发式与固定事件组合准入。完整依据见 [限制处理清单](policy-inventory.md)。
- 只读 exists/CRC 按完整相对路径拆分目录与叶文件名。实际 package Install 默认值及安装前脚本修改、每成员 condition 生成选择条件；多包归属按 OR。Disabled/Enabled 不混淆，后置/卸载/返回帮助事件不改变安装成员。
- 图保留事件 lifecycle，并按启动、安装前、安装中准备与安装后的顺序编译；仅安装前 OnNext 贡献全局 gate。Global Functions 只定义一次。显示进度屏幕配置尚未映射的行为覆盖边界见设计第 9 节。
- 报告/核心/worker 使用 v2 decoder/policy 和 file_conditions；一次观测所有条件引用，选中成员后生成内容身份与 ZIP。转换请求中的子集必须来自同一完整报告，重复或篡改成员拒绝。
- 同内容重装从倒序事务中找到 EXE/meta/size 匹配的可信记录，使用原始条件事实选择成员；当前现场文件与选中内容身份继续核对。旧策略版本可重分析同一原始包，不继承旧任意许可布尔值。
- 新增参数变动、生命周期、组件状态、多包 OR、子路径、逐成员条件、选择子集实际 ZIP、同内容重装/外部修改回归。独立 Python 参考生成 golden，原始输入、载荷和 ZIP 基线保持；研究脚本没有进入生产引擎。
- 独立复核使用 multi-agent 技能要求的 1 名只读 reviewer（gpt-6.1-sol / high）。复核发现的选择时序、卸载事件、重装查找与重复观测问题均已修正；最终静态复核无新增可操作问题。测试和真实转换由主 Agent 执行。
- 验证与候选包：最终源码回归与候选包核对均通过，见下节实际结果。证据保存在忽略目录 `.artifacts/summer-memories-diagnosis-20261014/`。未提交 Git、未部署、未执行实际游戏安装。

### 最终源码自动化与候选包

- Windows：`cargo test --locked -j 1 -p steam-patch-core -p steam-patch-installer-format -p steam-patch-installer-worker`，29 个 suite 共 543 passed / 0 failed / 7 ignored。首次并行编译曾在 rustc 退出 1 时未给出诊断，串行重试完整通过；首次失败不计作通过。日志 `final-stable-windows-tests.log`、`final-stable-windows-tests-retry.log`。
- Linux：同源码格式/worker 共 66 passed，核心 exe_report 10 passed，均 0 failed；日志 `final-linux-format-tests.log`、`final-linux-core-tests.log`。
- Windows 正式 release worker 的 LPAC 集成：`sandbox_to_authorization_install_retry_reinstall_and_external_edit --ignored --exact` 单独执行通过（1 passed）；使用临时合成游戏实例覆盖隔离分析/转换、授权安装、重试、同内容重装和外部修改拒绝，不作为真实游戏安装证据。日志 `final-stable-lpac.log`。
- Windows/Linux CLI、worker、隔离验证 runner 的最终 release 构建通过；日志 `final-stable-windows-build.log`、`final-linux-build.log`。Rust fmt 检查、git diff --check 和 93 条相对文档链接检查通过。golden JSON 最后仅排序键，值未变。
- 候选包 `.artifacts/summer-memories-diagnosis-20261014/candidate-20261015/steam-patch-manager-decky.zip`：7,228,118 字节，SHA-256 `f2a7ce9248f0358c211783f7ba5de0b357169d4684e954b7b642e1f717b0a3a2`，24 个 ZIP 成员。打包入口检查当前 SBOM/后端/协议；独立 ZIP 检查 CRC、runtime.json 摘要及两个 ELF 的 755 权限。没有覆盖历史候选 ZIP。
- Linux CLI SHA-256 `4a8c6d874aae030d29acaac0f75e36dfe810e517c5e21b70b3ed0475a155bded`；sf-worker `0b8299570d0f599f62074462adc0e5504a8d69679300aa4eca0241febe9c4636`。前端与 Python host 无修改。
- 夏日最终正式隔离分析/转换双平台通过，5,874 个成员全部以独立 Python ZipFile 核对路径、大小、CRC 与 SHA。规范化 ZIP 双平台相同 SHA `7b20d4bc35fb66e6dabe746b7ceea48fadebf8491a7a396ba8e7119434c75bfe`；全局条件仍为 Game.exe 存在及 Script.vdf CRC32=2002373744。其余三款（2303800/SF9.5、3721970/SF9.7、3606360/SF9.6）最终源码双平台矩阵均通过；分析的全部载荷和条件与独立参考报告一致，转换 ZIP 每个成员路径/大小/CRC/SHA 完整核对，四款双平台产物 SHA 均相同。汇总 `windows-samples.json`、`linux-samples.json`、`final-verification-summary.json`，日志 `final-stable-windows-samples.log`、`final-stable-linux-samples.log`。
- 本次没有重载、部署、修改 Deck 状态或写入真实游戏；实际安装、生效及设备平台性能仍待本次修复验收。

- 最终状态：本次代码与文档施工完成，自动化、独立复核、四款双平台正式隔离 worker 回归和候选包验证通过；设备部署与实际游戏安装/生效验收待执行，未提交 Git。

## 2026-10-15：新版 Steam Deck 部署与验收

- 用户要求“部署至steamdeck并验收”后使用项目 decky-deploy-verify 技能完成备份、正式 Loader ZIP 更新、后端正常重启、24 文件/实际 HTTP 摘要、唯一实例、RPC、设备实际隔离分析/转换及真实 QAM/全屏页面验证。详细验收与明确边界见 [本轮部署验收](deployment-20261015.md)。
- 夏日 DLC 在已部署 sf-worker 的真实 bwrap 环境以 uid 1000 分析 3.156 秒、转换 3.578 秒，全部 5,874 文件逐项独立核对；ZIP 与 Windows/Linux 本机相同，原项目选项错误未出现。
- 完整第一次备份、回滚包和设备证据已取回并核对；外部状态没有异常变更。仅自然来源缓存更新，23 条旧终态任务/14 条事务与 Loader 设置保留。唯一远端暂存和本轮 SSH 隧道已清理，用户原路由已恢复。
- 当前状态：用户明确选择“只部署插件并验收运行与界面”，本次按确认范围完成；实体手柄未验证，实际夏日安装/覆盖和游戏内生效不在本轮范围，未写游戏。下述 Git 提交前保留本阶段未提交的历史事实。

## 2026-10-15：Git 归档

- 用户要求提交目前为止的改动。安装器修复与必要测试已独立提交 `439408d`（fix(installer): 移除安装器默认值准入并修复夏日补丁解析）；本功能设计/取证/施工与部署验收另作独立文档提交，备份全清计划属于另一功能。沿用本 session 已通过验证，没有新增产品代码或重复测试。

## 2026-10-17：分析与提取阶段进度显示修复（Deck 实机反馈）

- 背景：用户在新版 CLI 的实机 EXE 安装中反馈"分析""提取"阶段进度恒为 0%（进入"核对"后恢复）。核对部署前后多轮实机记录（10-08~10-10、10-16 同一 3.79 GB 包）与本次：两阶段 `bytes_done` 全程为 0，属实现以来的既有显示行为，非本轮部署引入。这两个阶段的工作（安装包行为分析、载荷转换）在沙箱 worker 内一次性执行；worker 协议只返回最终结果、没有增量进度通道，核心此前仅上报阶段名与总量，UI 按"已处理/总量"得到恒 0% 的确定进度条。
- 修复：按既有"未知 total 不伪造百分比"约定，`exe_orchestrator.rs` 的 `analyzing_installer` 与 `extracting` 阶段不再上报 total（`operation::stage(name, None, None)`）；Decky/Windows 前端对未知 total 显示不确定进度动画（与 planning/cleanup 一致）。真实百分比仍不可得（worker 无进度通道），本修复只消除假 0%。
- 验证（Windows）：`cargo fmt --all --check` 通过；`cargo test -p steam-patch-core --lib` 407 passed / 0 failed / 6 ignored；`cargo test -p steam-patch-cli` 全部通过（含集成套件）。
- 验证（Linux，WSL `scripts/build-linux.sh`）：core 415 passed / 0 failed / 6 ignored；CLI 47+1+2 passed；release 产物 `steam-patch` 14,306,728 B `dc0c6d95…`（`sf-worker` 未变 `28b64f4d…`）。
- 打包：`package:plugin` + `package-artifacts.py --platform decky`；ZIP 7,231,185 B sha256 `bcd725d7…`，24 成员；ZIP 内两个 ELF 哈希/755 权限与 runtime.json 摘要核对通过；与线上清单相比仅 `bin/steam-patch` 与 `bin/runtime.json` 变化（`dist/index.js` 保持 `996223c2…`）。
- 部署：远端暂存 `/home/deck/steam-patch-validation/exe-progress-stage-fix-20261017/`（上传哈希一致）→ Loader `utilities/install_plugin` + `confirm_plugin_install`（prompt 参数 `steam-patch-manager` / `dev` / 本轮 hash / type 1 核对一致）；正式目录 24/24 与包内清单全等、`bin/*` 权限保持、`plugin_loader` active、实例 1 / 版本 0.1.0；HTTP bundle（URL 编码插件名路由）200、115,789 B、sha `996223c2…`；RPC `list_tasks` 31 全终态、`list_transactions` 18、`scan(false)` 28 实例 / 4 来源；Loader pluginOrder/disabled/frozen/hidden 前后一致；远端暂存已清理、隧道已关闭。
- 关联实机证据：修复部署前，用户在新版 CLI 上完成一单 3.53 GiB EXE 安装（task `8d0c735d-…`，终态 completed/cleanup），分析→提取→核对→写入→清理全流程实机跑通。
- 待验证：修复后的不确定进度显示待下一次实机 EXE 安装确认（用户操作）；Windows 宿主同项。证据在忽略目录 `.artifacts/defensive-logic-deck-20261017/`（stagefix 相关文件与本轮 ZIP 副本、`verify-zip-stagefix.py`、`compare-stagefix.py`）。
