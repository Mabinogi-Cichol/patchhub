# 防御逻辑精简计划

日期：2026-10-17。状态：待实现。施工记录：[progress.md](progress.md)。

## 1. 目标与范围

汇总本次对当前代码的过度设计审阅，删除无依据的业务数量门槛、统一总时限、重复比较和吞错兜底，同时保留真实的文件、网络、并发及解析边界。审阅基线为 `80829ea`，当时工作区干净；以下为静态分析和实施决策，不代表运行时复现、自动化或实机验收通过。实施前按届时代码核对调用链，位置以文件和符号为准。

本计划承接 [安装磁盘占用与兼容性精简](../install-footprint-and-compatibility/design.md) 中 D08 等原则，记录其未清理完整的分支及本轮新增发现，不改写原施工历史。规则以 [AGENTS.md](../../../AGENTS.md) 为准。当前授权只建立计划，不修改生产代码、提交、发布或操作设备。

## 2. 产品和实施约束

- 支持完整、合法的输入和结果；不能用调大常量、恒真分支或新配置替代删除。
- 数值写在历史设计中、名为“资源预算”或使用字节单位，都不构成必要性证据。
- 需要多端同步修改是实施范围，不是保留限制的理由。协议响应大小与普通调用总时限已纳入删除范围。
- 保留真正的边界检查；内部沿用已验证契约，错误在发生位置传播。
- 不预建通用分页、流式框架、重试平台、状态机或设备预算配置。只有具体处理方式存在问题时才采用最小必要改动。
- 同步清理专属配置、错误分支、文案、注释和测试；保留验证业务契约的测试。共享错误码仅在确认无其他用途后删除。

## 3. 删除与精简清单

### S01：候选数量门槛残留

位置：[identity.rs](../../../crates/core/src/sources/identity.rs) `validate_candidate`；[adapters/mod.rs](../../../crates/core/src/sources/adapters/mod.rs) `discover`。

删除列表最多64项、DLC条件最多32项、镜像最多64项的判断。持久化层已移除部分重叠计数门槛，但适配器出口仍保留。真实路径是合并候选后校验，失败时保留未合并结果、附加诊断并降为partial，影响成功缓存更新；不是直接丢弃全部候选。保留非零AppID、身份、URL、实际格式检查。清理数量专属断言，验证超原门槛的合法候选仍可正常合并和发布。

### S02：订阅设置重复的256 KiB门槛

位置：[registry.rs](../../../crates/core/src/sources/registry.rs) `set_game_source`；[store.rs](../../../crates/core/src/sources/store.rs) `save_settings`。

删除每次订阅修改对完整overrides序列化测长后返回source_limit的分支。底层设置存储已有4 MiB边界，该独立低门槛会阻断合法订阅修改，甚至取消订阅。统一由存储入口处理写入；其字节预算依S08同步删除，不通过保留低阈值补救。验证超过256 KiB的合法设置仍可修改、持久化并重载。

### S03：缓存与备份清理100,000项门槛

位置：[cache.rs](../../../crates/core/src/installer/cache.rs) `Walk::visit`、两平台 `Node::names`；[backup.rs](../../../crates/core/src/installer/backup.rs) `Scan::visit`、`children`。

删除累计访问和单目录数量拒绝。单目录超过上限会在枚举返回前失败，重试无法推进删除。保留统计计数、真实算术溢出检查、归属/链接检查、锁及真实错误。深度128按S16改为显式栈后删除，诊断512项按S17配套删除。测试拆开计数门槛与溢出断言，保留后者；验证合法大目录可完整盘点、清理及再次盘点。

### S04：Steam DLC事实数量拒绝

位置：[facts.rs](../../../crates/core/src/installer/facts.rs) `compute_dlc`；[signed.rs](../../../crates/core/src/installer/signed.rs) `require_facts`。

删除4096个depot、128个DLC门槛。它们在清单及集合已读取分配后执行，无独立资源作用；签名安装的完整事实门会因此失败。通用EXE已按动作所需事实处理，不能误称所有通用EXE都会被同一门槛阻断。保留数字、对象结构及实际DLC条件验证。验证超原数量的完整事实提取与签名安装前置检查。

### S05：Decky已验证结果的重复类型兜底

位置：[source_service.py](../../../apps/decky/backend/source_service.py) `prime`、`_apply_summaries`；[protocol_validation.py](../../../apps/decky/backend/protocol_validation.py) `validate_response`。

桥接层已按具体方法验证响应。删除内部必需字段的重复isinstance过滤、静默return/continue及替代空结构，避免违约变为旧快照或缺少结果。保留协议允许的可选字段处理，不移除真实RPC边界校验。验证正常快照与可选字段；内部违约应显式失败，不伪装为正常空结果。

### S06：设置锁错误伪装成默认值

位置：[store.rs](../../../crates/core/src/sources/store.rs) `revision`、`settings`、`save_settings`及其调用方。

删除锁获取失败返回版本0或default_settings的兜底，沿现有Result传播真实错误。首次初始化默认设置仍保留。验证锁错误不能生成正常默认快照或写入默认设置，正常首次初始化和设置持久化不回归。

### S07：无调用刷新超时常量

位置：[sources/mod.rs](../../../crates/core/src/sources/mod.rs) `REFRESH_WORKER_TIMEOUT`。

基线全仓Rust引用检索仅有定义。实施时复核后删除180秒常量及失效说明，不引入替代定时器。通过引用检查和编译验证即可，不为死常量单独造测试。

### S08：来源数量与字节预算

位置：[sources/mod.rs](../../../crates/core/src/sources/mod.rs)、[registry.rs](../../../crates/core/src/sources/registry.rs)、[validate.rs](../../../crates/core/src/sources/validate.rs)、[store.rs](../../../crates/core/src/sources/store.rs)、[http.rs](../../../crates/core/src/sources/http.rs)。

删除MAX_SOURCES=32、512 KiB候选/发现结果、4 MiB缓存/索引/设置及4096字节status/schema的固定拒绝，覆盖读取、序列化、持久化与适配器生成入口。删除仅为测长而重复编码、缩减attempt.index的分支及专属错误/测试；保留完整共享索引、文件身份/类型、JSON结构和原子写入。当前检查部分发生在分配后，现有数值无独立瓶颈依据，删留决定已完成，详见[边界核查](boundary-review.md)。验证完整大结果写入、读取、更新和发布。HTML另按S19处理。

### S09：8 MiB协议响应硬拒绝

位置：[protocol/lib.rs](../../../crates/protocol/src/lib.rs) `bounded_json_line`；[cli/main.rs](../../../crates/cli/src/main.rs) `encoded_line`；[bridge.py](../../../apps/decky/backend/bridge.py)；[Windows宿主](../../../apps/windows/src-tauri/src/main.rs)及生成契约。

取消固定响应大小拒绝。已有[投影验证记录](../rust-core-consolidation/p4-validation.md)记录500实例、每实例4补丁的合法DTO合成结果超过8 MiB；这是历史合成证据，不是本轮实机复现。当前CLI会将完整结果替换成response_too_large，JSON Lines分帧不要求该常量。

同步处理序列化、Decky StreamReader读取边界、Windows接收端、契约生成源与生成物、测试。检查MAX_RESPONSE_BYTES被终态排空等机制复用的地方：取消正常响应大小拒绝不等于取消已有终态后的诊断排空策略。请求大小与诊断截断已分别按S18/S17决定配套删除；终态排空独立保留。验证超8 MiB合法响应能完整跨两端传输；保留非法JSON、错误关联ID、错误类型和进程退出处理。不得截断业务数据，也不预建分页框架。

### S10：普通调用300秒统一总超时

位置：[bridge.py](../../../apps/decky/backend/bridge.py) `CoreBridge.__init__`、`_exchange_call`、`call_sync`及调用方。

删除普通调用默认300秒总时长失败和由此触发的杀进程路径，清理仅服务该策略的参数、字段和测试。正常扫描/哈希缓慢不代表故障。受控安装任务目前默认无此总时限，不应写成此次才取消安装超时。

已核实生产调用无额外显式timeout值；删除仅转发该配置的参数。连接失败、读取故障、取消及终态排空语义保留。保留实际进程/协议错误传播与必要取消，不引入新的任意时长看门狗。验证长时间有进展调用不会因总时长中断，进程异常和用户取消仍能到达明确结果；测试用可控时钟或替身，不实际等待300秒。

### S11：以URL后缀代替下载格式准入

位置：[acquire.rs](../../../crates/core/src/installer/acquire.rs) `validate_download`、`ArchiveKind::url`、重定向；[validate.rs](../../../crates/core/src/sources/validate.rs) `allowed_url`及相关选择入口。

删除ZIP路径必须以.zip、EXE必须以.exe结尾的准入门槛，覆盖初始链接及重定向。允许已许可主机上的无后缀下载端点；保留主机许可、HTTPS、逐跳重核、实际文件头与ZIP/PE/SF解析。来源发现可用后缀辅助识别，但不能把它作为下载内容的证明；不因此猜测未知格式自动可安装。验证无后缀有效内容及重定向可用，错误内容和越权主机仍失败。

### S12：缓存摘要的不可达重复比较

位置：[acquire.rs](../../../crates/core/src/installer/acquire.rs) `load_cached`末尾。

前面已保证预期SHA（若提供）等于元数据SHA，且实际文件SHA等于元数据SHA。删除末尾再次比较预期SHA与实际SHA的不可达失败分支。保留实际文件重哈希及前面两项比较，沿用缓存篡改测试，不为恒假分支制造测试。

### S13：SF包引用256项门槛

位置：[audit.rs](../../../crates/installer-format/src/audit.rs) `file_table`；[Python审计脚本](../../../scripts/sf95_audit.py)。

packages_count来自u16，但额外拒绝大于256。删除package_count_limit判断；按实际计数读取，保留字符串编码、输入范围和引用语义检查。该路径由graph::decode调用。同步审计脚本，补超过256个合法引用的验证，保留截断/无效引用回归。

### S14：PE资源目录256项门槛

位置：[audit.rs](../../../crates/installer-format/src/audit.rs) `resource_directory`及manifest查找；[Python审计脚本](../../../scripts/sf95_audit.py)。

计数为两个u16字段之和，读取已限定于资源区，manifest查找按固定层级执行。删除resource_entry_limit=256拒绝，避免其他合法资源较多时在找到manifest前失败。保留实际范围、偏移、节点类型及manifest歧义检查。补资源目录超过256项且manifest有效的验证，并保留截断资源区测试。

### S15：u16计数的恒真上界比较

位置：[audit.rs](../../../crates/installer-format/src/audit.rs) `file_table`、`MAX_ENTRIES`。

count来自u16，MAX_ENTRIES等于u16::MAX，比较恒真。删除count <= MAX_ENTRIES及确认无其他用途的常量，保留count > 0业务要求。保留现有8193条声明但数据不足时报截断、空表拒绝测试，不再重复测试语言类型上界。

### S16：清理深度128

删除缓存/备份遍历深度128拒绝，同时将递归遍历改为局部显式栈、后序删除，保留节点身份、Unix捕获及失败恢复行为。不得裸删门槛后继续依赖任意调用栈深度；不得新增持久状态机。验证深于128层的合法目录及真实OS错误。依据和具体路径见[边界核查第3节](boundary-review.md)。归入P2。

### S17：诊断条数和文本截断

取消缓存/备份512条诊断上限，以及core/协议/宿主2048字节detail截断和接收端限制。与S09同步，保留完整错误、结构化控制码、独立终态推导；不改写历史数据。验证超512条、长UTF-8诊断和9 MiB历史错误重放。依据见[边界核查第4节](boundary-review.md)。归入P3。

### S18：64 KiB业务请求上限

同步删除协议、CLI和两端宿主对普通业务请求整行的大小拒绝，保留JSON分帧、逐方法结构和身份授权边界。不同通道的worker描述符和控制消息不随此删除。验证合法大请求及坏请求后的继续处理。依据见[边界核查第5节](boundary-review.md)。归入P3。

### S19：HTML 4 MiB读取/解码门槛

删除ReqwestFetcher读取take截断及fetch_html原始/解码后大小拒绝，完整读取并解析正文；正文保持单次整体读取，取消沿用请求/操作级检查，不预建分块取消或流式DOM框架。保留HTTP/编码/挑战页/主机/重定向检查。验证原始或解码后超4 MiB的合法正文完整处理。依据见[边界核查第2节](boundary-review.md)。归入P1。

## 4. 明确保留的边界

| 类别 | 保留理由与范围 | 本轮可删部分 |
| --- | --- | --- |
| 路径防护 | 相对目标、目录归属、链接/重解析点、路径穿越与实际文件身份决定写入/删除位置；不存在仅靠内容摘要替代的关系 | 本轮未确认可直接删除的具体路径防护分支 |
| 摘要/CRC | 实际数据与预期内容、包记录校验值的比对；缓存重哈希、安装前后身份与恢复证据 | S12重复比较 |
| 来源主机 | 网络请求目标许可与重定向逐跳约束；哈希不约束请求发往何处 | S11后缀门槛 |
| 安装互斥 | 安装共享缓存锁与清理独占锁避免使用中删除；日志锁约束事务写入，职责不同 | 不整类删除、不合并成缺少跨进程效力的UI状态 |
| 解析边界 | 实际输入长度、偏移、格式字段、编码、节点和引用结构、压缩流完整性、算术溢出 | S13–S15人为门槛或恒真比较 |

## 5. 边界核查已完成

完整依据见[边界核查与最终决策](boundary-review.md)。清理深度、诊断截断、请求大小、HTML读取及来源本地字节门槛已明确删除/改写；解析器当前内存、递归、符号展开预算与终态后的诊断排空明确保留。没有留给实施阶段的删留判断。验证仍待实现后执行，不能把设计决策完成写成测试通过。

## 6. 实施拆分

| 批次 | 内容 | 完成条件 |
| --- | --- | --- |
| P1 来源与设置 | S01、S02、S05–S08、S19 | 已验证数据直达使用方；超原数量/大小合法结果不被旧分支拒绝；设置错误正常传播 |
| P2 清理与事实 | S03、S04、S16 | 大目录可完成清理，完整DLC事实可提取，锁和归属回归通过 |
| P3 协议与调用生命周期 | S09、S10、S17、S18 | 生成契约、CLI、Windows、Decky一致；大响应完整传输、长任务不中断、真实故障可结束 |
| P4 下载与格式解析 | S11–S15 | 无后缀合法下载和大资源/包引用可用，内容/主机/范围检查继续有效 |
| P5 收口 | 全部 | 沿生产路径复核无遗留门槛、死配置或吞错；更新设计/施工记录与索引，分别标注自动化和实机结果 |

批次只是组织顺序，不要求新增调度或抽象。各独立修复按仓库规则拆分提交；文档与代码分开，当前不执行提交。

## 7. 验收与证据

每项施工记录应包含实际删除范围、调用链复核、必要回归及结果。沿用有效历史记录；新增行为、失败或未解决问题才补检查。不为死常量/恒真判断添加镜像实现的测试。

至少验证来源完整发布、设置持久化与失败传播、大目录清理、事实提取、两端超8 MiB响应、普通长任务及取消、无后缀下载、SF/PE大计数。相应路径/主机/摘要/CRC/并发/截断输入回归必须保留。先用本地合成输入验证确定行为；真实Windows/Decky传输及实机安装如未执行，明确待验证，不以单元测试替代。设备操作另按当时授权范围执行。

完成后使用[过度设计审阅技能](../../../.agents/skills/overengineering-review/SKILL.md)沿最终调用路径复核，确认没有用新阈值、兜底或框架替代旧机制。
