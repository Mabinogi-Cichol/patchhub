# Rust 核心服务收敛重构

日期：2026-10-11（Asia/Shanghai）。状态：**P0 已冻结并提交；P1 扫描投影/取证/只读预览已实现，Windows/Linux 自动化、基准对照与独立复核通过；P2 签名/信任/终态与统一安装入口已实现，Windows/Linux 自动化和独立复核通过；P3–P6 待实现，双端最终产物与实机待验证**。

施工记录：[progress.md](progress.md)。基线：[迁移后代码审阅](../cross-platform-installer-migration/rust-core-consolidation-review.md)、[跨平台安装器迁移](../cross-platform-installer-migration/design.md)及其[完成记录](../cross-platform-installer-migration/progress.md)。

P0 交付：[契约与 v4 变化表](p0-contract.md)、[动作/终态真值表](p0-behavior.md)、[数据与债项映射](p0-data-debt.md)、[性能基线](p0-baseline.md)、[独立复核](p0-review.md)。P1 实现与阶段过渡详见 [P1 验证](p1-validation.md)；其余冻结目标仍待后续实现。

## 1. 目标与完成定义

将目前留在 CLI、客户端与平台桥接中的共享领域职责收敛到 steam-patch-core，以类型化公共契约连接 CLI、Windows 和 Decky。本 feature 覆盖审阅识别的五项整合，以及相关重复规则和过渡代码的最终清理。

完成时，核心可通过库 API 独立执行扫描投影、三类安装编排、信任 revision 管理、缓存清理和动作判定；CLI 是协议/执行适配器，宿主负责身份、进程与交互生命周期，UI 消费权威事实并负责呈现。Windows 与 Decky 使用同一套规则和契约。

本 feature 对第 9 节列出的技术债逐项清零。完成声明必须同时具备源码清理、契约一致性、自动化回归、双端产物和实机验收证据，不能将“新入口已接通但旧生产逻辑仍在”视为完成。

## 2. 范围与产品决策

1. 按既定优先级实施：扫描投影 → 签名安装编排与信任存储 → 缓存清理 → 动作可用性 → 公共请求/协议契约。P0 先冻结各阶段必需的接口边界，P6 收口清理和验收。
2. 安装行为保持已有验收语义。两端主按钮继续以 overwrite 操作发起任务；初次安装与“覆盖安装补丁”的标签由核心证据区分。Install/Reinstall/Overwrite 枚举保留各自正式语义，不能将 overwrite 按钮误改成旧保护型 install。
3. 保留签名目录优先、动态 ZIP/EXE 身份、来源订阅、语言/DLC 条件、平台能力、请求幂等、迟到取消时 commit 优先、写入取消回滚和旧收据 superseded 行为。
4. 持久化 installer/rust-v1、sources/rust-v1、trust.sqlite 与宿主设置/任务历史沿用现有数据。核心库提取不新增第二套状态根、信任数据库或事务日志，不清空缓存、历史或恢复资料。
5. 自动清理偏好继续由宿主保存并明确传入核心。Decky 保留现有开关与默认行为；Windows 本轮不新增设置 UI，传入 Disabled，保持当前行为。核心提供统一服务和显式策略，平台差异记录为产品边界。
6. 本 feature 最终使用协议 v4，同步更新 CLI、Tauri、Decky 桥接、客户端及指南；v3 请求/响应明确拒绝，不保留双协议执行或旧核心降级。变更原因是必需动作 DTO、结构化安装结果与共享校验契约，双端必须同步消费。持久化 schema 不随 wire 版本自动升级。
7. 临时迁移入口只允许存在于开发中的阶段；P6 删除。正式构建必须包含匹配的 CLI、worker、宿主和 UI，版本不匹配在任何写入前报 unsupported_protocol。
8. 不新增来源、安装格式、发行商策略、Windows 直链执行模式、网络监听服务或跨设备偏好同步。已登记的旧 profile 样本、旧 Windows 版本和 fake-IP 环境限制沿用历史记录，不能冒充本 feature 已验证范围。

## 3. 已核对基线与职责

| 当前代码 | 待提取职责 | 最终所有者 |
| --- | --- | --- |
| [CLI runtime.rs](../../../crates/cli/src/runtime.rs)：扫描组合、dynamic_patches/dynamic_detection/preview_route/committed_changes | 完整扫描投影、身份关联、文件取证与只读预览 | core projection/detection 服务 |
| 同文件 install_signed_route/inner、signed/ready_release、highest_revision/remember_revision | 签名 ZIP 业务流程、准入、防回滚存储 | core installer 与 trust store |
| 同文件 cleanup_cache/directory_size；[installer/store.rs](../../../crates/core/src/installer/store.rs) | 允许清理区域、遍历/删除/统计、结果 | core installer cache |
| [共享 UI](../../../packages/ui/src/index.ts) patchInstallable；[Windows 页面](../../../apps/windows/src/main.tsx) projectScan；[Decky 页面](../../../apps/decky/src/index.tsx) projectPatch/按钮条件 | 平台能力、来源、收据与检测组合成可发起动作 | core actions |
| [CLI main.rs](../../../crates/cli/src/main.rs)、[Tauri main.rs](../../../apps/windows/src-tauri/src/main.rs)、[Decky bridge.py](../../../apps/decky/backend/bridge.py) | 请求/错误/事件 DTO、ID 和字段约束 | 共享 protocol crate 与生成契约 |
| [Decky tasks.py](../../../apps/decky/backend/tasks.py) 与客户端安装结果处理 | 安装结果的稳定终态分类 | core outcome；宿主维护任务记录 |
| [Decky discovery.ts](../../../apps/decky/src/discovery.ts)、[Windows sources.ts](../../../apps/windows/src/sources.ts)及任务进度展示 | 来源 URL guard、公共状态谓词与格式 | 共享 TS UI |

模块名是目标职责名称；具体文件拆分和签名在 P0 冻结。提取时移动现有成熟算法和有价值测试，避免为每个阶段重写底层安装器。

## 4. 目标架构与依赖方向

- steam-patch-core：类型化领域请求/结果、projection、detection、actions、installer 三种编排、trust revision store、cache 和 outcome。
- 新建 steam-patch-protocol（目标目录 crates/protocol）：版本、方法白名单、wire DTO、错误和事件格式、ID/字节上限、领域请求转换。它依赖 core；core 不依赖 protocol、CLI、Tauri 或 Python。
- CLI：解析命令行和受限 JSONL，确定受信配置/平台/worker 环境，将请求转换为核心 API，转发进度及结果。
- Tauri/Decky：宿主目录与降权身份、固定二进制、进程控制、renew/cancel/EOF、有界读取和事件派发。
- packages/contracts：由 Rust wire 类型导出并纳入版本管理的 JSON Schema/TS 类型、固定协议 fixture；生成结果必须可再现，CI 验证无漂移。
- packages/ui：使用生成 DTO，保留中文文案、颜色、排序、搜索、进度格式和展示级 URL guard，消除双端相同 helper。

核心接受显式依赖：可信状态根、平台能力、固定 worker runtime、Transport、取消/租约回调、时钟、信任公钥/目录及只读 Steam 实例事实。调用方提供环境，核心校验路径、准入、授权和写入边界；RPC 参数不允许指定任意 worker/binary 或注入环境。

core 公共入口必须具备状态根不重叠游戏、链接路径和非特权身份等适用检查，不能以“CLI 已校验”作为直接库调用的前提。平台专属能力实现继续复用既有隔离 worker crate。

## 5. 各模块技术方案

### 5.1 扫描投影与受限文件取证

提取类型化 ProjectionInputs/ScanReport、完整扫描与单实例/批量投影入口。输入包含实例、已验证/开发目录、来源快照、事务、统一时刻和本机能力；返回当前实例的补丁、来源、发现结果、检测证据与诊断。

- 来源身份拼法、启用/订阅/stale 条件复用核心 resolver/registry 的公共判定；扫描与 resolve 可以因阶段目的返回不同信息，但相同条件的理由代码只能有一个规则实现。
- 签名 release 冲突优先，动态 selection 与 content/release 区分；最新 committed 关联保持原规则，superseded 不冒充当前安装。
- 提取 catalog 和动态检测共用的受限文件取证：相对路径、库/游戏边界、symlink/junction/reparse、文件类型、大小与读取期间变化。区分 missing、mismatch、unknown/读取失败；不得将无法读取误写为“已匹配”。
- scan/preview 为只读业务入口，信任 revision 的持久更新通过显式 trust 服务执行；不以只读预览触发游戏写入。
- 来源事件完成后按受影响实例批量投影；保留完整刷新入口。同一快照复用已有实例/事务事实，不重复扫描整个 Steam 库或为每条 UI 事件重哈希所有游戏。
- P0 登记现有投影耗时与哈希次数；基准使用固定 fixture 和可得真实实例。优化不得依靠跨文件变更的未失效哈希缓存。

generation、旧事件去重、扫描关闭、来源发现的异步时序继续由宿主会话管理；来源匹配/去重算法继续使用已有 Rust 引擎。

### 5.2 三种安装入口与签名目录信任

提取 SignedInstallInputs/Deps 和签名编排；复用现有 ZIP/EXE 编排、TaskControl、RequestLedger、授权和事务核心。类型化 InstallTarget 区分 SignedRelease、PublisherZipSelection、PublisherExeSelection，核心统一路由。

- signed 路线完整迁移 request claim → 幂等绑定 → 下载/本地 ZIP → 计划 → facts/授权 → 应用前重核 → apply → record_terminal 与失败撤销/恢复。
- release 与实例/AppID、平台、build、支持等级、撤销/有效期由核心准入；内嵌公钥和受信文件选择由宿主配置层提供。
- TrustRevisionStore 沿用 trust.sqlite，保留 FULL 持久性和 Immediate 写事务内的二次 revision 检查，禁止读后写竞态允许回滚。
- InstallerOutcome 包含原事务/身份/错误/恢复事实，以及稳定 disposition（三值 completed/cancelled/failed）；needs_recovery 是事务状态，以 recovery.required 明确呈现，宿主 task 仍记录 failed。Committed → completed；取消或租约过期导致的 rolled_back → cancelled；其余失败 → failed；needs_recovery 显式携带恢复需求。迟到取消不能把 committed 改为 cancelled。
- 清理失败独立记录；失败处理仍优先保留真实事务结果和恢复信息，不依靠字符串包含 cancel 推导终态。
- 核心直接调用者必须使用相同入口和准入保障。CLI 保留序列化和宿主事件适配，不保留私有签名业务流。

P2 实施补充（2026-10-12）：正式写入使用 service::ValidatedHostContext 与受检查身份，裸 prepare/apply/recover/discard 只供 crate 内部；公开恢复/丢弃在 journal 写锁内校验实际计划，批量先全量校验，失败恢复只处理自身 plan。新请求使用域分隔的稳定 UUID，旧 plan_id 与 schema 保持；双回填失败可由原 journal 绑定验证重放。固定非空 `; cleanup_pending:<诊断>` 是唯一可伴随规范取消码的持久后缀，cause.code/detail 分离；planned 丢弃和应用回滚的非致命收尾不改变真实终态。AfterCommit 在 P3 前明确拒绝。具体实现与证据见 [P2 验证](p2-validation.md)。

### 5.3 缓存与安装后清理

提取 CacheService、CleanupPolicy（Disabled/AfterCommit）和 CleanupResult。手动清理与提交后清理调用同一服务，自动策略在统一安装入口中处理；Decky main.py 不再追加第二次删除或改写业务结果。

- 仅清理 InstallerPaths 明确列出的 content/downloads/reports 与可安全回收的遗留任务，不扩展到未知 state 子目录。
- 保护收据、请求账本、日志、授权、活动任务/锁、撤销标记及恢复需要的 stage/backup。沿用成熟事务收尾；缓存服务不得借机删除事务备份。
- 清理与安装互斥由核心锁/任务事实保障，不能仅信任前端 busy；实例并发或后台进程也不能删除活动任务的输入/输出。
- 拒绝链接逃逸和未知路径；统计采用受限遍历、溢出检查，部分删除明确报告已删除字节及失败理由。
- committed 后清理失败保持安装成功，并提示可重试清理；rolled_back/cancelled/needs_recovery 不自动清理恢复材料。
- Disabled 不删除下载缓存；手动清理结果相同。宿主偏好文件继续保存原路径与结构，设置无需迁入 Rust。

### 5.4 权威动作 DTO

核心在同一 projection 快照中输出必需 PatchActions。包含自动安装、本地 ZIP 导入、覆盖/重装的可用性、阻断理由和主按钮的操作/标签类别；具体字段在 P0 契约表冻结。

- 自动下载可用性组合签名信任、来源/订阅、selection、artifact、实例、检测与本机 links；本地导入只能进入 signed 路线，按本地输入准入，不把下载不可用当成同一个阻断。
- 主按钮操作为 overwrite，标签类别 install/overwrite 来自核心收据与文件证据；是否 busy/选中/可见由 UI 管理。
- 对相同可信快照两端动作事实一致；平台隔离/身份差异允许有具体理由。EXE 的可发起检查不等于已获写入授权。
- UI 直接消费 actions，不再拼 committedKeys、installedStatuses、support_level 或 source_id 推导权限。
- 删除 auto_installable/reinstall_available 等旧客户端派生字段及 patchInstallable/canAutoInstall 的权威判定分支；展示摘要改为消费 actions 和正式检测状态。仅有一个正式 DTO，无新旧字段双读。
- 执行安装时重新读取来源 revision、facts、信任、租约和事务，防止 stale actions 绕过重核。

P0 冻结 actions.automatic/local_archive/reinstall/overwrite/primary，替代旧 download_available 及 UI 派生权限字段；源候选原始 artifact/download 事实仍保留。详见 [目标字段](p0-contract.md#5-权威动作-dto)。

### 5.5 公共请求、结果与展示 helper

领域请求在 core 类型中表达，wire 校验在 protocol 集中维护；Tauri/Python 的早期拒绝只作为适配，核心仍执行权威检查。

- release_id/selection_id 必须且只能有一个；selection 禁止本地 archive；操作枚举与 prefix/selection 身份由核心验证。
- request_id 非空、UTF-8 不超过 128 bytes、无控制字符；其他业务 ID 保持 256 bytes 上限。Python 使用编码后字节数，避免与 Rust 长度含义不同。
- error.detail 不超过 2048 UTF-8 bytes，在字符边界截断，不附加省略号；是否在 UI 展示省略提示属于呈现。
- 保留 64 KiB 请求、8 MiB 响应的受限读取边界（完整 JSONL 行，包含 LF）以及 request_id/sequence 对应关系；新 DTO 的体积在 P0 核对，禁止静默增大上限。
- 协议版本、方法表、阶段/错误/结果 DTO 以 Rust 为源；导出的 schema/TS 类型及 Python 协议 fixture 纳入 CI。生成工具不得成为插件运行时依赖。Python 的方法表、版本和字段校验消费生成契约/固定 fixture；避免以生成 TS 类型代替 Python 消费者的一致性检查。
- 宿主任务终态消费 disposition；异常传输错误仍由宿主记录，任务 ID、历史、租约触发和异步取消生命周期不迁入 core。
- 来源 URL guard、公共来源状态谓词和阶段/百分比格式统一到 packages/ui；保留平台交互差异。浏览器链接打开与下载网络许可分别验证，不能互相代替。

P0 已冻结补充：逐方法 wire params 的受信配置统一为必需 host 六字段，operation 与宿主 cleanup_policy 必需，外层 request_id 为唯一幂等 ID；不保留旧配置字段双读。取证新增 16 GiB 流式上限（P1 已实现）；installer/rust-v1 新 cache.lock 统一 shared 安装/恢复与 exclusive 删除，自动清理不得锁升级。幂等重放从原 Transaction/ledger.error 的精确原因码重建 disposition，不增加持久 schema。具体接口/字段/生成器与 source 枚举以 [P0 契约](p0-contract.md) 为准；P1 已实现扫描/检测/预览，P2 已实现签名/信任/终态与写入口，P3–P5 尚待实施。

## 6. 实施阶段与交付门槛

| 阶段 | 目标和顺序 | 依赖 | 阶段通过条件 |
| --- | --- | --- | --- |
| P0 | 契约/债项/性能与数据基线冻结 | 本设计 | 冻结目标类型、状态/动作真值表、v4 变化表、直接 API 边界与数据路径；完成静态复核 |
| P1 | 扫描投影、检测与预览核心化（高） | P0 | CLI 只调用核心投影；resolver/检测共用规则；固定输入语义和取证边界通过 |
| P2 | 签名编排、统一安装结果与 trust store（高） | P0、P1 的公共事实类型 | signed/ZIP/EXE 核心入口、幂等/取消/恢复/防回滚验证通过；旧 signed 私有流程删除 |
| P3 | 核心缓存与明确自动策略（中） | P2 | 清理服务及互斥/保护验证通过；Decky 追加删除移除；平台偏好明确 |
| P4 | 核心 actions 与双端消费（中） | P1–P3 | 动作真值表、平台差异、过期重核通过；前端不再计算安装权限 |
| P5 | v4 protocol、生成契约与共享展示 helper（较低） | P0 的边界、P1–P4 稳定结果 | Rust/TS/Python 契约一致；匹配版本接通；所有旧分支/重复 helper 删除 |
| P6 | 清理、双端回归、打包、实机与独立复核 | P1–P5 | 第 9 节全部债项关闭，完整验收证据齐备，无阶段过渡生产代码 |

P0 采用文档与静态样本冻结目标类型，暂不新增未消费的生产类型；P5 负责整体契约收口，避免前期接口随意扩散。阶段之间允许内部过渡，过渡清单要进入 progress，不能提前部署混合契约产物。每阶段可独立提交/复核，文档与代码分开提交；只有 P6 完成才声明本 feature 完成。

## 7. 验收矩阵

| 层级 | 必须覆盖 | 证据/执行时机 |
| --- | --- | --- |
| 结构 | core 不依赖 CLI/protocol/Tauri/Python；CLI 无领域实现；债项消费者全部接入；生成契约无漂移 | 各阶段静态调用链；P6 最终复核 |
| 扫描 | signed 优先、ZIP/EXE 身份、已装/外装/部分/缺失/不可读、superseded、多库同 AppID、source stale/订阅变化 | core 固定 fixture；读路径链接/尺寸变化负向用例 |
| 信任/安装 | 签名有效期/撤销、revision 并发防回滚、三路线 install/reinstall/overwrite、request 冲突/幂等、写入取消回滚、needs_recovery、commit 优先 | core 直接 API 与 CLI 真实子进程；复用现有测试并按新增边界补充 |
| 缓存 | 自动策略两值、手动清理、活动任务冲突、保护目录、链接/部分失败、统计、清理失败不改 committed | temp fixture 与核心锁竞争；双端 RPC |
| 动作/契约 | signed/unsigned、ZIP/EXE link、纯手动输入、事务/检测组合、过期 actions 重核、Unicode ID/detail、超限/未知字段/混装版本/乱序事件 | 固定真值表；Rust/TS/Python 共享协议 fixture |
| 自动化 | fmt、clippy、core/cli/protocol/worker、Windows 宿主、Decky Python、双端 typecheck/build、TS 与打包/SBOM测试 | Windows + Linux/WSL2；扩展现有 CI，不在本次文档阶段执行 |
| 产物 | 双端最终包、固定 worker、运行文件 allowlist、CLI/UI/宿主版本配套、哈希清单与 SBOM | P6 release 构建和解包核验；核对许可证/依赖增量 |
| 实机 | 两端扫描/发现/actions/历史/恢复/清理；ZIP/EXE 真安装、覆盖、写入取消回滚与无关文件保全；Deck 控制器交互 | 最终匹配产物部署后；记录实例、版本、事务、前后文件证据及用户确认 |

增加当前协议的 fixture-backed CLI 子进程冒烟，使用临时状态与受控游戏 fixture 验证完整调用链；替代历史已删除 smoke 工具留下的 v3 空缺，最终工具只覆盖 v4。安装/回滚测试的写入只发生在明确隔离 fixture 或授权实机实例。

实机样本优先沿用已完成迁移的 2743810 ZIP、2303800 EXE 和 3137110 大包，以及写入取消样本；实际可得情况与基线证据在 P0 冻结。worker 格式解析未改时沿用既有 golden/真实样本，不为核心服务提取重做所有旧 profile 实测。

既有验证可作为迁移基线；发生代码移动、契约变更或删除后的最终树需要相应新证据。避免仅镜像实现的测试，优先保留跨边界行为、竞态、负向输入与真实协议验证。所有未执行项在 progress 标待验证。

## 8. 数据保全与双端切换

P0 记录现有目录/表/schema、设置、任务历史和未完成事务的读取要求。保持已有计划/收据/请求账本的含义；新增 outcome/actions 为运行结果与投影，不要求重写历史事务来供 UI 使用。

最终升级前完成或恢复活动事务、备份插件/宿主产物与状态、核对哈希并整体替换。启动先做 v4 握手和字段校验，失败时停留在可诊断状态，禁止旧新组合执行写入；恢复旧产物时使用匹配备份，避免回滚覆盖升级期间新事务。

立项阶段仅编写文档；2026-10-11 用户随后授权开始 P0，新增冻结文档和隔离基准，并只读确认本机 Steam 清单。用户随后授权提交 P0 并实施 P1，实际写入只限临时测试/基准目录，扫描/预览保持只读。不连接 Steam Deck、不部署、不修改用户游戏或宿主持久化状态。实际部署按 [项目部署技能](../../../.agents/skills/decky-deploy-verify/SKILL.md) 和执行时的用户授权范围进行。

## 9. 技术债关闭清单

| ID | 必须关闭的项 | 完成证据 |
| --- | --- | --- |
| D1 | CLI 内扫描/动态投影/检测/预览业务实现 | 核心 API + CLI 调用链；原私有实现删除 |
| D2 | CLI 与 resolver 的来源 blocker、release 身份拼法及多套文件哈希规则 | 公共规则/取证入口，固定输入与路径边界测试 |
| D3 | CLI 内 signed 编排和 trust.sqlite 管理 | 核心 signed/trust store；直接库调用与并发防回滚证据 |
| D4 | CLI 清理实现、Decky 安装后追加删除和缺少核心互斥 | 唯一 CacheService、显式策略与保护/冲突测试 |
| D5 | 前端安装权限/覆盖证据组合与旧派生字段 | 必需 actions，双端消费；旧 helper/字段无生产消费者 |
| D6 | 手写重复 wire DTO、ID 长度差异、detail 截断差异 | 唯一 protocol、生成契约、共享 fixture/CI |
| D7 | Python/客户端重复事务到任务终态映射 | 核心 disposition 与 commit/取消/恢复真值表 |
| D8 | 双端相同 URL guard/状态谓词/进度格式 | 共享 TS helper 与平台调用点；不复制网络许可策略 |
| D9 | 阶段临时入口、双协议/旧核心降级、废弃 import/测试夹具 | v4 唯一路径，最终源码和产物审阅；有效数据读取保留 |
| D10 | 当前协议缺少可重复子进程冒烟及新架构文档漂移 | v4 冒烟、CI、指南/架构/索引/SBOM 同步 |

D1–D10 在 progress 分别记录“待实现/已实现/自动化通过/待实机/已关闭”和证据。最终不得用“后续优化”关闭已列项。无法完成的必要项保持 feature 未完成，明确阻断原因。

合法宿主职责、现有生产数据读取与历史文档事实不属于待删除重复代码。对范围外的新问题单独登记，不以“大扫除”扩大本 feature。

## 10. 风险与处置

- 提取时把成熟规则改成另一套算法：先固化行为矩阵，移动现有实现，单独审阅任何语义修订。
- 扫描导致重复大文件哈希：P0 记录次数/耗时；P1 复用快照并批量投影；验收报告数据差异和原因。
- 核心入口绕过宿主校验：核心验证关键身份/路径/准入，直接 API 负向测试作为门槛。
- wire 版本与状态 schema 混淆：v4 同步产物切换，持久化结构沿用；如实施发现必须修改 schema，先修订设计与数据保全方案。
- 将动作快照当授权或 cleanup 改变事务结果：执行时重核；独立结果字段与竞态/失败测试。
- 最终只完成新实现却保留旧消费者：P6 对 D1–D10 做源码、调用链、打包内容和独立复核，未闭环不放行。
