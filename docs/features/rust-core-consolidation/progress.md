# Rust 核心服务收敛重构施工记录

设计：[design.md](design.md)。基线审阅：[rust-core-consolidation-review.md](../cross-platform-installer-migration/rust-core-consolidation-review.md)。

当前状态：**P0 冻结/静态复核通过：契约、行为真值表、数据/债项基线、Windows 实测与体积核对已完成，可进入 P1。P1–P6 待实现；生产 API、双平台产物、实机待验证。**

## 1. 用户目标与范围

用户要求按审阅优先级建立独立重构 feature，并清除本次识别的技术债。立项阶段授权和实际工作为编写文档；用户随后授权开始 P0 施工，范围为契约/行为/数据冻结、性能基线及静态复核。覆盖扫描投影、签名编排/信任存储、缓存、动作 DTO、请求/协议，以及相关结果映射、共享 UI helper 和最终验收收口。

保留已完成安装器迁移的验收结论和历史记录；新 feature 的验证以变更后的最终树为准。初始化记录仅含文档；本次 P0 增加隔离基准工具，不修改生产代码、不部署、不提交。

## 2. 阶段状态

| 阶段 | 状态 | 已有证据 | 下一项交付 |
| --- | --- | --- | --- |
| 文档初始化 | 已完成 | 2026-10-11 design/progress、索引与总体进度关联 | P0 静态复核与契约冻结 |
| P0 契约/数据/性能基线 | 已冻结、静态复核通过 | [契约](p0-contract.md) / [真值表](p0-behavior.md) / [数据债项](p0-data-debt.md) / [基准](p0-baseline.md) / [复核](p0-review.md) | P1 扫描投影/取证核心化 |
| P1 扫描投影/检测/预览 | 待实现 | 无新实现证据 | 核心服务与调用替换、阶段测试 |
| P2 signed/trust/outcome | 待实现 | 无新实现证据 | 核心统一安装 API、防回滚与终态验证 |
| P3 cache/cleanup policy | 待实现 | 无新实现证据 | 核心清理与保护/互斥验证 |
| P4 actions/双端消费 | 待实现 | 无新实现证据 | 必需动作 DTO、前端删除推导 |
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
| D1 CLI 扫描/预览业务 | 待实现 | — |
| D2 准入/身份/哈希重复规则 | 待实现 | — |
| D3 signed/trust 核心化 | 待实现 | — |
| D4 cache 服务与追加删除 | 待实现 | — |
| D5 前端动作推导/派生字段 | 待实现 | — |
| D6 请求/错误/生成契约 | 待实现 | — |
| D7 安装结果终态映射 | 待实现 | — |
| D8 共享展示 helper | 待实现 | — |
| D9 过渡路径与废弃消费者 | 待实现 | — |
| D10 v4 冒烟/CI/文档产物 | 待实现 | — |

关闭标准以 design 第 9 节为准，不能只凭删除某个符号声明关闭。

## 6. 验证状态与证据位置

| 验证 | 当前状态 | 实施后应记录 |
| --- | --- | --- |
| 文档结构/链接/差异 | P0 已通过 | 本次链接/阶段/债项/空白验证；文档及隔离基准工具，无生产源码变化 |
| 核心/CLI/protocol/worker 自动化 | 待验证 | Windows/Linux 的最终版本、命令与结果 |
| Tauri、Decky Python、TS | 待验证 | 两端协议消费与新增/迁移测试 |
| v4 子进程冒烟/契约一致性 | 待验证 | 固定 fixture、失败场景、生成无漂移 |
| 性能/哈希次数 | 当前 Windows debug 基线通过；新实现对照待验证 | 3固定fixture各6轮、3次计数等价、6次真实manifest；来源/动态计数与Linux待后续 |
| 双端包/SBOM/哈希 | 待验证 | 最终包、版本清单、allowlist、license 增量 |
| Windows 实机 | 待验证 | 实例、操作/事务、前后文件、恢复/清理 |
| Deck 正式部署/RPC/UI/控制器 | 待验证 | 备份、正式文件哈希、RPC、页面与用户确认 |
| 独立复核 | P0 通过；P6待验证 | 三项P2修正后定向复核无遗留；[P0复核](p0-review.md) |

证据保存在本 feature 附加文档或 .artifacts 的对应目录，progress 链接实际存在的结果。本轮未创建虚构验收或样本证据文件。

## 7. 遗留与阻断

P0 冻结/静态复核完成，无已知 P1 实施阻断。所有 P1–P6 生产代码任务与运行验证未执行。新增 cache.lock、16 GiB 取证上限、params.host、生成器和 Rust/TS/Python fixture 仍为待实现目标。

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
