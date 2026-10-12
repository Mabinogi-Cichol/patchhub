# Rust 核心服务收敛重构施工记录

设计：[design.md](design.md)。P1 证据：[p1-validation.md](p1-validation.md)。P2 证据：[p2-validation.md](p2-validation.md)。P3 证据：[p3-validation.md](p3-validation.md)。P4 证据：[p4-validation.md](p4-validation.md)。基线审阅：[rust-core-consolidation-review.md](../cross-platform-installer-migration/rust-core-consolidation-review.md)。

当前状态：**P0 已冻结并提交；P1 扫描投影/受限取证/只读预览已实现，Windows/Linux 自动化、基准对照与独立复核通过；P2 签名/信任/终态及统一安装入口已实现，Windows/Linux 自动化与独立复核通过；P1/P2已本地提交；P3核心缓存/策略已实现，Windows/Linux自动化与独立复核通过；P4 actions/双端消费已实现、最终自动化通过、两项复核意见已修复；P3/P4 代码已本地提交 `19ffc8a`；P5–P6 待实现；双端最终产物、实机待验证。**

## 1. 用户目标与范围

用户要求按审阅优先级建立独立重构 feature，并清除本次识别的技术债。立项阶段授权和实际工作为编写文档；用户随后授权开始 P0 施工，范围为契约/行为/数据冻结、性能基线及静态复核；随后授权提交 P0 并开始 P1 实现，再授权继续 P2。覆盖扫描投影、签名编排/信任存储、缓存、动作 DTO、请求/协议，以及相关结果映射、共享 UI helper 和最终验收收口。

保留已完成安装器迁移的验收结论和历史记录；新 feature 的验证以变更后的最终树为准。初始化记录仅含文档；P0 增加隔离基准工具并已提交；P1/P2 生产代码已在本次用户授权下提交为 `e77d0a8`；文档独立提交，未部署。

## 2. 阶段状态

| 阶段 | 状态 | 已有证据 | 下一项交付 |
| --- | --- | --- | --- |
| 文档初始化 | 已完成 | 2026-10-11 design/progress、索引与总体进度关联 | P0 静态复核与契约冻结 |
| P0 契约/数据/性能基线 | 已冻结、静态复核通过 | [契约](p0-contract.md) / [真值表](p0-behavior.md) / [数据债项](p0-data-debt.md) / [基准](p0-baseline.md) / [复核](p0-review.md) | P1 扫描投影/取证核心化 |
| P1 扫描投影/检测/预览 | 已实现、自动化与独立复核通过 | [P1 验证](p1-validation.md)：Windows417/Linux422、只读子进程、基准计数/事实对照 | P2 已完成；P1/P2 代码已提交 `e77d0a8` |
| P2 signed/trust/outcome | 已实现、自动化与独立复核通过 | [P2 验证](p2-validation.md)：Windows465/Linux472、信任并发/硬链接、三路线、持久终态与恢复边界 | P3 缓存/清理策略；P1/P2 代码已提交 `e77d0a8` |
| P3 cache/cleanup policy | 已实现、自动化与独立复核通过 | [P3 验证](p3-validation.md)：Windows487/Linux494、双端与清理互斥 | P4 已完成；P3/P4 代码已提交 `19ffc8a` |
| P4 actions/双端消费 | 已实现、最终自动化通过、两项复核修复完成 | [P4 验证](p4-validation.md)：Windows502/Linux509、Tauri24、Decky71/TS65、过期快照与真实体积 | 代码已提交 `19ffc8a`；P5 正式 v4/生成契约；P6 最终独立 gate |
| P5 protocol v4/共享 helper | 待实现 | 无新实现证据 | 生成契约与混装拒绝、重复代码清理 |
| P6 最终清理/验收 | 待实现（前序未完成） | 无新验证证据 | D1–D10 关闭、双端矩阵/产物/实机/独立复核 |

阶段通过后记录具体文件、命令、结果与证据；保留历史事实，不以设计中的目标覆盖实际完成情况。

## 3. 2026-10-11 文档初始化

### 实际修改

- 新建本 feature 根目录 rust-core-consolidation，分别建立 design.md 和 progress.md 并互链。
- design.md 明确目标职责/依赖方向、P0–P6 顺序、协议 v4 整体切换、当前持久化数据保全、动作权威与 overwrite 行为、宿主自动清理策略、技术债关闭清单和验收矩阵。
- docs/features/README.md 增加本 feature，状态明确为待实现；docs/progress.md 添加总体摘要和本 feature 链接。
- 迁移后审阅保留原证据，追加后续立项关联；原跨平台安装器迁移 design/progress 及其已完成验证不改写。

### 核对与验证

- 静态核对：AGENTS.md、基线审阅、CLI Params/methods/capabilities/ID/detail、core InstallOperation、双端主按钮 operation、Decky 桥接/SourceService 与现有 CI 入口。
- 文档检查通过：新 feature 两份文档与基线审阅共 42 个本地链接有效；design/progress 互链、P0–P6 和 D1–D10 编号对应、索引/总体进度关联有效；git diff --check 与新文件空白检查通过。
- 本轮没有调用子 Agent：沿用上一轮三路只读审阅并由主 Agent核验本轮契约相关入口。
- 未执行：编译、测试、协议冒烟、性能基准、产物构建、Steam Deck 连接/部署、游戏写入和实机验收。
- 未创建 Git 提交。

## 4. P0 交付清单（2026-10-11 已完成）

- [x] 冻结 ProjectionInputs/ScanReport/单实例与批量投影 API、受限取证接口及诊断语义。
- [x] 冻结 InstallTarget/InstallRequest/InstallerOutcome/Disposition、signed/trust/cache 依赖接口。
- [x] 列出当前 v3 → v4 字段、方法/错误/事件与 capability 变化；保留正式有效的方法，删除过渡字段；定义生成流程与版本拒绝测试。
- [x] 冻结 actions 真值表：自动/本地输入、签名信任、来源、平台、事务与检测状态；确认主按钮保持 overwrite。
- [x] 冻结终态真值表：commit 优先、取消回滚、非取消回滚、needs_recovery、传输错误与 cleanup 失败。
- [x] 记录 installer/sources/trust/settings/task-history 路径和结构；识别未完成事务与切换保全步骤。
- [x] 记录扫描 fixture、耗时/哈希次数以及可得真实实例，明确新旧投影对照方法。
- [x] 将 D1–D10 映射到生产定义、消费者、测试和产物；明确临时过渡代码的删除阶段。
- [x] 完成设计静态复核；新增发现或产品决策变化先同步 design。

P0交付索引：[字段/API/v4表](p0-contract.md)、[动作/终态样本](p0-behavior.md)、[数据/债项](p0-data-debt.md)、[性能/体积](p0-baseline.md)、[wire静态样本](p0-wire-samples.json)、[复核](p0-review.md)。勾选表示冻结与基线已完成，不表示目标生产代码或v4运行验证已完成。

## 5. 技术债状态

| ID | 状态 | 完成证据 |
| --- | --- | --- |
| D1 CLI 扫描/预览业务 | 已实现、阶段自动化通过；P6 最终关闭 | 核心公共 API、CLI 只读适配与子进程；[P1](p1-validation.md) |
| D2 准入/身份/哈希重复规则 | P1/P2 规则与写入边界、P4 actions 已实现；正式请求契约待 P5 | 共用取证、来源身份/ZIP准入/EXE resolver；[P1](p1-validation.md) |
| D3 signed/trust 核心化 | 已实现、阶段自动化/复核通过；P6 最终关闭 | [P2](p2-validation.md)：统一入口、原始签名重核、SQLite 高水位并发 |
| D4 cache 服务与追加删除 | 已实现、阶段自动化/复核通过；P6最终关闭 | [P3](p3-validation.md)：核心安全回收/互斥/策略，旧遍历和追加删除移除 |
| D5 前端动作推导/派生字段 | 已实现、阶段自动化通过；P6 最终关闭 | [P4](p4-validation.md)：必需 actions，旧权限字段/helper 删除，双端直接消费 |
| D6 请求/错误/生成契约 | 待实现 | — |
| D7 安装结果终态映射 | core 已权威化；双端终态消费/删除重复映射待 P5 | [P2](p2-validation.md)：持久取消、恢复、清理诊断与账本失败重放 |
| D8 共享展示 helper | 待实现 | — |
| D9 过渡路径与废弃消费者 | 待实现 | — |
| D10 v4 冒烟/CI/文档产物 | v3 阶段冒烟已通过；v4/最终 CI/产物待实现 | [P1 子进程](../../../crates/cli/tests/projection_smoke.rs) |

关闭标准以 design 第 9 节为准，不能只凭删除某个符号声明关闭。

## 6. 验证状态与证据位置

| 验证 | 当前状态 | 实施后应记录 |
| --- | --- | --- |
| 文档结构/链接/差异 | P0–P4 已通过 | [P4 最终检查](../../../.artifacts/rust-core-consolidation/p4/final-checks.json) |
| 核心/CLI/protocol/worker 自动化 | P4 core/CLI Windows502、Linux509 通过；新 protocol/最终 worker 待后续 | [P4 汇总](../../../.artifacts/rust-core-consolidation/p4/validation-summary.json) |
| Tauri、Decky Python、TS | P4 Windows24、Decky71通过/2skipped、TS65及双端类型检查通过 | [P4](p4-validation.md)；最终v4双端待P5/P6 |
| v4 子进程冒烟/契约一致性 | 待验证 | 固定 fixture、失败场景、生成无漂移 |
| 性能/哈希次数 | P0/P1 Windows debug 对照完成；哈希计数一致，取证耗时上升；Linux/发行包待后续 | [P1 对照](p1-validation.md#性能对照)，新增动态收据32次哈希 |
| 双端包/SBOM/哈希 | 待验证 | 最终包、版本清单、allowlist、license 增量 |
| Windows 实机 | 待验证 | 实例、操作/事务、前后文件、恢复/清理 |
| Deck 正式部署/RPC/UI/控制器 | 待验证 | 备份、正式文件哈希、RPC、页面与用户确认 |
| 独立复核 | P0/P1/P2 通过；P6待验证 | P2 安全与终态边界闭环，无遗留 finding；[P2](p2-validation.md#独立复核与修复) |

证据保存在本 feature 附加文档或 .artifacts 的对应目录，progress 链接实际存在的结果。本轮未创建虚构验收或样本证据文件。

## 7. 遗留与阻断

P1–P4 已完成阶段实现与自动化，无已知 P5 实施阻断。受限取证16 GiB上限已实现；带文件取证的 debug 扫描慢45%–61%，保留安全检查并登记最终平台性能验证。cache.lock 与 actions 已完成；params.host、生成器、v4 Rust/TS/Python fixture 仍待 P5，最终产物/实机及全范围独立 gate 待 P6。过渡字段及协议边界见 [P1记录](p1-validation.md#阶段过渡与未执行项)。

已有旧 profile 样本、旧 Windows 版本和 fake-IP 限制保留在原功能记录。本 feature 新发现的必要问题进入本记录并保持未关闭；范围外问题单独登记。最终清理清单和必要验收未闭环时，不声明 feature 完成。

## 8. 2026-10-11 P0 施工与冻结

用户授权开始 P0，使用 multi-agent 技能并行制作三个互斥文档，第四路只读独立复核。主 Agent完成基准、体积核对、文档集成与修正。没有新增未消费的生产类型，core/CLI/两端运行逻辑未改；没有连接设备、部署、游戏写入或提交。

### 实际交付

- p0-contract.md：精确 projection/取证、统一 install/signed/trust/cache/outcome/actions 接口，直接库调用边界，16方法的v4参数/结果/事件/capabilities/错误与生成流程。保留历史10个事务state读取和数据schema；新安装结果三 disposition。
- p0-behavior.md：自动/本地/平台/来源/检测/收据/三 operation 与 overwrite 主按钮真值表，commit优先/typed取消/恢复/cleanup/传输终态样本。修正本地ZIP能力 gate、unknown 不误禁初装、recovery nullable、partial cleanup真实统计。
- p0-data-debt.md：唯一数据根与表/JSON结构、未完事务及整体切换保全、D1–D10生产定义/消费者/测试/删除阶段；旧marker是无业务作用的历史资料，不当当前保护规则。
- scripts/benchmark-projection.py 与 p0-baseline.md：可重复隔离 fixture、实际 hash attempts/completions/bytes、plain/counted响应等价与源码SHA不变、归一化快照、本机真实清单可得性；计数只在临时源码副本，构建目标独立于正式target。
- p0-wire-samples.json、p0-review.md：目标DTO静态样本与字节上限核对、独立问题及关闭证据。

### 实际验证

- `python scripts/benchmark-projection.py --output .artifacts/rust-core-consolidation/p0-projection-baseline.json --real-root 'D:\Program Files (x86)\Steam'` 通过。隔离plain/counted debug编译，18次合成serve、3次计数对照、6次真实manifest scan。两次准备失败（未复制内嵌resources/trust公钥）修复后最终通过。
- Windows warm median：500实例无patch 0.324238s；24实例全匹配 0.668940s（96/96hash，25,165,824B）；混合证据 0.373439s（96尝试/48完成，12,582,912B）；真实36清单 0.044995s。debug进程总耗时、非冷缓存，dynamic计数0且未覆盖来源/committed plan，不冒充后续优化结论。
- 真实清单三样本均installed：2303800 build10821988、2743810 build23866175、3137110 build16717597。未读installer/source/trust状态或哈希实际游戏payload；旧实机仅历史基线。
- 新请求adapter定向结构检查通过：v3参数保留、v4六字段host、请求超限进程前拒绝、完整8MiB行接受/8MiB+1拒绝。使用subprocess double，不是v4 runtime通过。
- v4静态 UTF-8 JSON+LF体积：请求384B，三outcome509/572/633B；24实例report45,155/42,880B。500×4patch满detail约19.2MB会超限，冻结明确response_too_large；不增限额。独立reviewer另核对请求/outcome体积一致。
- 独立reviewer初审3项P2（unknown、recovery/partial cleanup、v4请求adapter）均修正，定向复核通过，无遗留具体finding，可放行P1。详细 [复核记录](p0-review.md)。
- `python -m py_compile scripts/benchmark-projection.py`、JSON解析、Markdown本地链接、阶段/债项编号、EOF/空白与 `git diff --check` 完成检查；[检查结果](../../../.artifacts/rust-core-consolidation/p0-doc-check.json)。证据在本 feature 附加文档和 .artifacts/rust-core-consolidation/。

### 待实施/待验证

P1先迁 projection/受限取证/preview，再按P2–P6推进。新增领域API、v4真实子进程fixture与生成、source/动态hash新旧对照、trust/cache竞态、Linux/release、双端包/SBOM/最终实机均未执行。D1–D10仍待实现，不因P0文档冻结关闭技术债。

## 9. 2026-10-11 提交 P0 并完成 P1 阶段

用户授权“提交，然后开始 P1 施工”。P0 已分成三个本地提交：`3e8bbd2`（归档整合审阅）、`5e2fb04`（隔离基准工具）、`07c5a5a`（P0 冻结文档）。提交前检查暂存差异与空白，通过；沿用已完成的 P0 验证，不重复运行。

P1 使用 multi-agent 并行实现：Sol/high 负责共享受限文件取证与核心投影，Luna/medium 负责来源只读快照与 resolver 共享规则；主 Agent负责 CLI 适配与集成。生产协议仍为 v3，P4/P5 切换 actions/v4；这一阶段不部署混合产物。

- [x] 核心投影、单实例/批量与预览 API。
- [x] 来源/事务只读快照、统一时刻与显式宿主能力。
- [x] 安全取证共用，区分缺失与无法读取。
- [x] CLI 删除私有投影/检测/预览实现，扫描不更新 trust revision。
- [x] 直接核心 API 与 CLI 子进程验证、P0 性能对照。
- [x] 独立审查与文档同步。


### P1 实际结果

- 新 core projection/detection、来源和事务只读快照；CLI 删除私有投影/检测/预览并保持 v3 适配。损坏/链接/变化证据返回 unknown；预览不可靠基线为 null。具体文件、边界和过渡见 [P1 验证](p1-validation.md)。
- Windows 417、Linux 422 项测试通过，均6项既有 ignored；fmt全workspace、Windows core/CLI Clippy -D warnings、真实CLI子进程2项通过。来源/事务不初始化或修复，扫描不更新信任revision。
- 隔离基准4组各6轮：原3组除新增 diagnostics 外与P0事实一致，哈希次数/字节一致；动态收据32/32次、8 MiB。带取证的两组耗时增加45.5%/61.2%，不声明性能提升；冷缓存/Linux/release/实机待最终验收。
- 独立审查4项P2已修复并回归：相对状态根、来源内部链接、损坏overlay最新时间、未知预览基线；祖先句柄复用定向安全复核通过，无遗留具体finding。
- 本阶段共4个子Agent：取证和投影2路 Sol/high、来源1路 Luna/medium、独立复核1路 Sol/high；主Agent集成、修复、运行与文档。
- P1尚未提交；没有连接设备、部署或改动用户游戏/状态。P2–P6不因P1阶段通过而宣告完成。

## 10. 2026-10-11 至 2026-10-12 P2 阶段完成

用户授权“继续P2”。复用P0冻结规格，保留P1未提交改动。三路Sol/high分别负责签名编排、SQLite信任revision、结构化终态/取消持久原因；主Agent统一宿主验证、安装入口及CLI适配，第四路 reviewer 只读安全复核。已完成 P2 运行验证与文档收口；不连接设备、不部署、不提交。

- [x] 核心签名编排及apply前重新验签/revision/准入核验。
- [x] 唯一trust.sqlite服务与只读验证、Immediate二次revision检查。
- [x] 三类统一安装入口、安全宿主上下文与公开写入口收口。
- [x] 结构化终态、持久typed取消/租约及幂等重放。
- [x] CLI删除私有signed/trust流程，保持开发期v3适配。
- [x] Windows/Linux回归、负向边界、独立审查与证据同步。


### P2 实际结果

- 三路线统一核心 API、受检查身份与宿主写入边界；CLI 删除私有 signed/trust/outcome 实现。trust.sqlite 与 installer/rust-v1 原路径/schema 保持，旧事务身份优先读取；新请求稳定 UUID 支持双回填失败 journal 重放。
- 明确用户取消/租约原因、commit 与 needs_recovery 优先；普通失败、授权撤销、迟到取消及清理诊断不混淆。公开恢复/丢弃在写锁内校验实际计划，恢复全批先校验，内部失败仅恢复自身计划。
- Windows 465、Linux 472 项通过，均6项既有 ignored；fmt、Windows all-targets Clippy -D warnings、独立只读复核通过。实际 Linux root 身份另运行2项拒绝写入验证通过。完整日志、负向样本及未执行范围见 [P2 验证](p2-validation.md)。
- P0 冻结终态在 2026-10-12 作受限修订：允许 writer 固定非空 cleanup_pending 后缀，机器码仍精确匹配，诊断独立保留；非致命收尾不改变已确认回滚事实。复核发现的写边界、幂等、取消和清理问题均已修复回归，无已知遗留 finding。
- 本阶段共4个 Sol/high 子 Agent（3路 specialist、1路 reviewer，fork none）；主 Agent 集成、修复、测试与文档。P1/P2 尚未提交；P3 cache/AfterCommit、P4/P5 双端 actions/v4、P6 最终产物和实机仍待实施。


## 11. 2026-10-12 提交 P1/P2

用户明确要求“提交”。使用项目 steam-patch-git-commit 技能检查暂存区与实际范围；P1/P2 同属本 feature，完整实现共用文件连续变更，功能代码与文档拆为两次本地提交。

- `e77d0a8`：refactor(rust-core): 收敛 P1 投影与 P2 安装核心服务。33 个 Git 变更条目（测试移动识别为 rename），含必要自动化与隔离基准工具；不含文档、忽略的验收产物或用户库材料。
- 文档另行提交：P1/P2 验证、设计/施工记录、P0 受限终态修订、索引和因测试移动更新的历史验收链接。
- 沿用本 session 最终 Windows465/Linux472、fmt/Clippy、实际 root 拒绝与独立复核；提交前核对全部源码 SHA-256 无漂移、暂存内容与已验证工作树一致，diff --cached --check 通过，不重复运行测试。
- 本次仅本地提交，不推送、不部署；P3–P6 与双端最终产物/实机仍待实施。第9、10节保留实现阶段尚未提交的历史事实。


## 12. 2026-10-12 P3 阶段完成

用户授权“继续P3”。P1/P2 已提交，复用冻结缓存契约：核心 CacheService、共享/独占跨进程缓存锁、AfterCommit 策略与双端薄适配。第一轮 Sol/high 分别实现安全缓存回收及缓存锁，Luna/medium 适配 Decky；主 Agent 集成统一服务、CLI 与验证。

- [x] 核心缓存只读盘点及受保护边界。
- [x] 安装/恢复/丢弃共享锁与清理独占锁。
- [x] 仅新提交触发 AfterCommit，清理失败独立呈现。
- [x] Decky 策略及结果适配，Windows 默认 Disabled。
- [x] Windows/Linux 自动化、独立安全复核与证据。

实际实现与完整边界见[P3验证](p3-validation.md)、修正与复核见[P3复核](p3-review.md)。核心只读盘点/安全删除、持久cache.lock与Unix目录双锁、独立安全gate、可信终态tasks回收、任务自身句柄收尾已落地。AfterCommit只新commit、重放不重复，清理失败/忙碌独立呈现；Decky偏好进入核心，Windows显式Disabled。

- 最终Windows487/Linux494通过，均6项既有ignored；Windows宿主23、Decky后端69通过/2skipped、TS63与类型检查、fmt/Clippy通过。实际root负向1项通过。
- 四个子Agent，3路实现+1路独立安全review，主Agent集成修复/运行；发现的policy遗漏、owner重开、Unix锁inode分裂、旧自身终态按路径删除均闭环。Windows目录共享兼容回归已修正，最后完整测试通过，源码哈希无漂移。
- 本阶段不部署、不触碰真实用户游戏/设备、不提交。持久schema/偏好保持；v3policy开发期可选，P5收口严格v4。D4自动化阶段通过，待P6最终关闭；P4–P6仍待实施。

## 13. 2026-10-12 P4 施工开始

用户授权继续 P4。保留未提交 P3 代码与记录；新增核心 actions、共享 UI 类型和 Windows/Decky 直接消费。协议正式切换、生成器与共享终态 helper 留待 P5；最终产物和实机留待 P6。施工前差异保存在 `.artifacts/rust-core-consolidation/p4/pre-p4.diff`。当前实现与验证尚未完成，未提交、未部署。

## 14. 2026-10-12 P4 实现与验证完成

- 核心必需动作 DTO、来源/信任/实例/链路/收据事实、稳定理由和主按钮事实已实现；双端取消旧字段/能力推导，按 actions 发起默认或显式本地 ZIP，保留只读预览。
- 收紧实际 Reinstall prepare/apply 的 owned/intact 条件；损坏文件走 Overwrite。扫描后撤销/build/文件/订阅/来源/needs_recovery 的 stale snapshot 回归通过。现有 recovery 互斥原本有效，补明确码和跨 patch 样本。
- 最终矩阵：Windows core/CLI 502、Linux509（各6 ignored），Tauri24，Decky后端71通过/2 skipped，TS65；Windows10状态用例/13动作断言、双端类型、fmt/Clippy通过。源码与日志证据见 [P4验证](p4-validation.md)。
- 四路 multi-agent 完成实现与只读复核意见收集；复核两项 P2 已由主 Agent修复/回归闭环，最终全范围 reviewer 报告因审批服务用量中断未收到，不记为 findings[]，详见 [审阅](p4-review.md)。审批恢复后本地修复与最终验证已完成，无当前执行阻断。
- 实际500实例1/4 patch序列化为4,787,585/18,533,585 bytes；保留8MiB宿主限额、真实async行读取超限归一化。P5仍须生产者有界输出/后续请求/v4终态闭环，最终产物/实机与独立 gate留待P6。
- 保留未提交P3，P4也未提交、未部署；没有操作真实游戏或设备。

## 15. 2026-10-12 P3/P4 本地提交

用户授权“提交”，使用仓库 steam-patch-git-commit 技能，按功能代码与文档拆分。

- 代码提交 `19ffc8a`：核心缓存清理/共享锁/AfterCommit、必需 actions/执行重核、双端薄适配与必要测试，共 42 个文件。
- 文档独立提交：设计/施工记录、P3/P4 验证与审阅、功能索引、总体进度与开发期契约说明。
- 沿用本 session 最终矩阵；提交前及代码提交后核对 199 个源码 SHA-256 无漂移，暂存完整差异与 diff --cached --check 通过，不重复运行测试。
- 本次仅本地提交，未推送、未部署。P5/P6、双端最终产物与实机待实施；P4 最终全范围独立 gate 仍待 P6。第 12–14 节保留实施时尚未提交的历史事实。
