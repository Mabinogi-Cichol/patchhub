# 来源发现与订阅迁入共享 Rust 后端施工记录

设计：[design.md](design.md)。状态：**迁移方案（待实现）**。日期：2026-10-07（Asia/Shanghai）。

## 当前状态

| 工作 | 状态 | 说明 |
| --- | --- | --- |
| 两端来源能力差异定位 | 已完成只读核对 | Windows只用签名目录；Decky另有Python SourceRegistry，名称显示无法补齐动态查源 |
| 迁移设计与功能归档 | 已完成设计与文档复核 | 本目录、设计/施工互链、索引；链接检查和独立审阅已执行，迁移代码待实现 |
| P0 基线fixture及最终契约冻结 | 待执行 | 未取得本轮Decky实时来源结果，未创建HTML/旧状态迁移fixture |
| P1–P3 Rust配置/cache/适配器/CLI会话 | 待实现 | 没有迁移代码 |
| P4 Windows接入与验收 | 待实现、待验证 | 先Windows；既有中文展示验收不替代动态来源验收 |
| P5 Decky切换与消费者兼容 | 待实现、待验证 | 保留旧设置、receipt及publisher/EXE接口 |
| P6 双端收口 | 待实现、待验证 | Python来源逻辑退场及完整证据尚未执行 |

## 2026-10-07 计划制定

### 用户目标与本轮范围

用户要求把Decky专属的来源订阅、联网发现、缓存和身份核对迁入Rust，并作为新feature制定计划。本轮只读盘点、设计与文档检查，不实施功能、不部署或连接设备。

### 实际完成的调查

- 读取仓库AGENTS、来源指南、核心CLI/serve-task、Windows宿主、共享DTO、Decky scan/discover及旧安装消费者；以当前源码和已部署Windows结果确认边界。
- 复用前次Windows验收证据：AppID2303800保留英文name、中文localized_name，签名目录revision4中没有该AppID，patches=0、无discovery。记录位于[名称施工记录](../localized-game-names/progress.md)，不是新的源迁移验收。
- Luna只读盘点来源配置schema1/2、overrides enabled/added/games、cache层级、TTL、成功/attempt分离、检查点、查询key、并发过期和测试入口。默认shell最初无输出，切换明确的本机PowerShell只读通道后完成；未将最初失败算成源码验证。
- 当前自动发现仍主要使用manifest name和可选name_hints，未消费新增localized_name；计划将核心覆盖名纳入查询输入并使旧负缓存正确失效。
- 确认publisher/exe消费者直接依赖sources.matches、cached_candidates、_rules/builtins、game_subscribed和validate_download；计划包含薄RPC适配与消费者回归，不能仅删Python文件。

### 方案决策

- 单一Rust sources服务/默认规则；协议2增量方法与能力协商，来源事件与安装进度分离。
- 单独sources/rust-v1状态空间，先导入设置，旧cache重新校验后只作过期seed；旧Python文件留存，单引擎运行，回退保护后续用户选择。
- 保留现有静态catalog和安装层授权；Windows先实现来源发现/展示，Decky再切换。Windows动态ZIP/EXE自动安装迁移是后续独立feature。
- 以P0–P6门槛推进；真实设备、网站和安装能力分别记录验证，计划阶段不声称完成。

### 本轮验证

- 相对链接、Markdown结构和状态一致性：最终36个新增/关联相对链接通过，代码块闭合，索引与设计/施工均保持待实现；两项审阅发现修订后由主Agent核对。git diff --check通过，仅有既有换行提示。
- 独立设计审阅：Sol完成只读审阅，发现回退后重入设置与默认扫描自动发现两项P2契约缺口；主Agent已补齐双向checkpoint/三方合并、冲突处理及默认scan→scan_sources/force/取消行为，并加入验收场景。
- Rust/Python/前端测试、构建、真实网络、CLI、客户端或设备部署：本轮未执行，代码未修改，不沿用以前结果冒充迁移验证。

### 待办与风险

- P0取得2303800当前Decky来源身份及最小脱敏fixture，冻结协议/模型/ID/TTL对象边界和资源限制。
- 锁定Rust HTML/Unicode依赖、跨进程查询发布与限速策略，证明双平台缓存原子写入和取消行为。
- 完成Rust实施及双方适配，才可移除Python HTML discovery worker；EXE worker及平台桥接保留。
- 无Git提交；保留工作区既有代码和其他feature文档修改。

### 独立审阅后的计划修订

- 不仅首次导入：每次切换记录活跃引擎、摘要和共同设置快照，Python回退期间的新选择在Rust重入时导入；双边同字段冲突不静默覆盖，中断/重复切换需幂等。
- 区分核心scan与宿主完整扫描：两端默认scan后自动启动scan_sources(force=false)，保持Decky自动查源；强制/单游戏查询、取消、revision和事件过期明确约定。
- 增加宿主内部get_source_policy，提供Rust核实的compiled/current规则与origin，兼容旧安装消费者读取；这些均是拟新增接口，不是已实现方法。
- 本轮委派2个子Agent：Luna/low只读盘点，Sol（gpt-6.1-sol）/high独立审阅；均仅接收任务说明，无完整历史，无Astra，未修改代码。

## 2026-10-07 Git 归档

- 用户后续授权提交 Git；迁移设计、施工记录及关联索引按独立文档提交归档，不包含功能实现代码。
- 本地化名称代码、Windows 扫描锁修复及各自文档分别提交；其他既有修改保留在工作区。迁移阶段状态仍为待实现，验收项仍待执行。
