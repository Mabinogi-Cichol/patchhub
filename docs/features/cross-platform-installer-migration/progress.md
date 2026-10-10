# 跨平台安装器迁移施工记录

设计：[design.md](design.md)。日期：2026-10-10（Asia/Shanghai）。状态：**P1 已实现/自动化通过；P2 主体与可得真实样本通过，三旧 profile 待验；P3 已实现/双平台自动化与隔离验证通过；P4 Windows 已实现，2303800 EXE 与 2743810 ZIP 真安装/重装及两款游戏内确认通过，后续修复与最终代码回归、隔离恢复/历史 RPC、便携包校验通过，P4 按调整后验收范围完成，用户已取消断网实测要求（[收尾验收](p4-acceptance.md)）；P5 已实现/正式部署并验收完成，ZIP/EXE 真安装、写入回滚及用户实体手柄/两款游戏内确认通过；P6 待执行；更早 Windows 版本未覆盖**。

## 当前状态

| 工作 | 状态 | 说明 |
| --- | --- | --- |
| 前序范围/安装器差异盘点 | 已完成只读核对 | 来源 P0–P6 完成；动态 ZIP/EXE 准备与授权仍为 Decky Python，Windows 只展示候选 |
| 设计与功能归档 | 已完成 | 独立设计/施工、索引与关联入口；文档检查见下文 |
| 来源迁移兼容遗留盘点与清理规划 | 已完成只读检查与设计；代码待清理 | 六类遗留、消费者依赖、阶段门槛已纳入设计第 7.3 节；不是已清理或已验证 |
| P0 契约/样本/平台隔离 | 契约冻结定稿并通过 Gate 1；Windows 原型完成；Linux/Deck 对照与 root 降权补测通过 | 契约 v0.5（encoding/ID/schema/锁/worker/限额/依赖冻结，Gate 1 终审通过）；Windows LPAC+Job 原型 31 项探针 27 pass；Linux bwrap 原型 22 项在 WSL2 与 SteamOS Deck 均为 21 pass/0 fail/1 info；P0-D 修订后非 root 回归 23 项与 WSL root 降权补测 26 项均 0 fail；cgroup/seccomp 与 P1+ 待执行 |
| P1 共享 ZIP 服务 | 已实现/自动化验证 | 获取/缓存/配方/新授权与新 schema；来源 Rust 编码、当前规则契约和直接初始化 |
| P2 Rust SF 与 worker | 主体已实现/双平台自动化与可得真实样本通过；三旧profile待验 | SF9.5–9.7 完整结构/AST/IR/固定worker/确定性转换；P3生产接线待实现 |
| P3 隔离/授权/任务 | 已实现/自动化与隔离验证；独立复核修复完成 | 生产启动器、EXE 编排、facts/空间/撤销/任务回收；双平台 release 证据与 [复核归档](p3-gate-review.md) |
| P4 Windows | 已实现/自动化与前序独立复核通过；两款真安装/生效与恢复 RPC 通过；调整后范围验收完成，断网实测已取消 | 2303800 EXE、动态/签名 ZIP、恢复/重装；移除旧核心来源降级（[实施记录](p4-windows.md) / [复核归档](p4-gate-review.md)） |
| P5 Decky | 已实现/正式部署/验收完成 | v3 薄桥接、新 schema 与旧安装保护；自动化、沙箱、ZIP/EXE 真安装及回滚、正式包/RPC/QAM/全屏通过，用户确认实体手柄与两款游戏生效（[P5 记录](p5-decky.md)） |
| P6 收口 | 待实现/待验收 | 单一生产策略、移除 Python SF 依赖、来源兼容遗留逐项检查、打包与文档 |

## 2026-10-08 设计制定

### 用户目标与本轮范围

用户说明 shared-source-discovery 已开发完成，要求撰写后续 feature“跨平台安装器迁移”。进一步明确动态 ZIP 配方、EXE 行为分析和安装授权也应迁入 Rust 核心。本轮只读核对和文档撰写；未实施代码、运行安装测试、下载补丁、连接设备、部署或创建 Git 提交。

### 实际核对

- 阅读共享来源设计/施工/索引，确认唯一 Rust 来源引擎及尚未迁出的 publisher ZIP/EXE 安装器。
- 核对 publisher、exe_generic/profiles/facts/installation/worker 及 tasks：XP3 映射、分实例签名 state、选择/内容 ID、撤销先于 discard、Linux bwrap 边界。
- 核对 install/download、CLI runtime、Windows 宿主/页面、共享 DTO 与契约；签名 ZIP 可复用，Windows 动态入口尚未开放。
- 沿用通用 EXE/一站式/Decky UX 决策：行为准入、无游戏白名单、一键安装、租约/结果、成功清理。
- 查阅微软 AppContainer/Job Object 官方文档，区分资源隔离与进程/资源控制；未做实验，原生隔离须 P0 实测。
- 核对四款本地游戏与三条旧 profile 是不同验收组，设计分别登记。
- 保留工作区既有来源 feature 与相关未提交修改；索引/前序设计只补充后续链接。

### 初版设计决策（兼容条款已由下方 breaking change 修订取代）

- 三项业务全部归 Rust 核心；SF 解析/转换由隔离固定 Rust worker 执行，核心核验授权并调用既有事务层，宿主只桥接。
- Windows 无 Python/WSL；隔离不足阻断对应 EXE。
- 保留旧 ID/收据，签名目录与动态授权分开，绑定内容/来源/策略/实例/现场条件。
- 新索引在 installer/rust-v1，首期复用旧 publisher 事务目录并聚合，先恢复未完成事务；不复制设备 state、不删旧数据。
- 核心统一一站式编排、幂等/撤销/恢复与跨 state 实例互斥；保留历史不复播修复。
- P0–P6 门槛，Windows 真实动态安装后切换 Decky，双端验收后移除重复生产 Python 策略。

### 本轮文档检查

- 新文档 37 个相对链接与关联文档新增 6 个引用均有效；代码块闭合、无尾随空白，设计/施工互链和索引均为待实现。git diff --check 通过，仅有仓库 LF/CRLF 提示。
- 关联 5 份文档全量扫描共 110 个相对链接，发现前序来源设计中 5 个指向已删除 Python 来源实现/测试的历史链接；它们属于既有迁移历史，本次未新增或改写。
- 实现测试/构建、真实网络、隔离、游戏写入、设备验收：未执行，全部待验证。
- 前序成绩仅为历史基线，不计本 feature 已通过。撰写期间重新核对前序最新记录：Decky 状态误报修复已正式部署并通过 RPC/真实页面，实体手柄待验收；新设计已同步该基线。

### 初版待办与风险（旧 schema/回退项已取消，以末节当前待办为准）

1. 冻结黄金 ID/ZIP/SF 覆盖/载荷/错误，登记脱敏样本与旧 state schema。
2. 实测 Windows 隔离资源权限、网络拒绝、进程回收/计量，锁定支持版本/依赖；不足时先改方案，不无隔离放行。
3. 冻结实例锁/journal 读协调、租约/幂等/撤销、回退条件，再实施。
4. 按设计执行双平台自动化、生产包与真安装；缺样本、手柄/生效确认明确待验证。

## 2026-10-08 开发阶段 breaking change 修订

用户明确接受旧缓存文件的 breaking change，并说明开发阶段无需考虑旧版本兼容。本轮按该决定修订设计、索引与总体计划，功能仍待实现。

### 实际修订

- 取消旧缓存/schema 解析、导入、ID 哈希复刻/映射、profile 注册表、旧本机签名/收据读取、双版本 RPC 与旧版本回退要求。
- 动态安装使用新的内部授权类型和 installer/rust-v1 状态；旧下载包、报告、normalized ZIP、临时配方/授权直接失效，安装时重新获取和分析。
- 三条旧 profile 的包只作为通用语义/载荷测试输入，不保留生产 profile 安装分支或兼容资源。
- 核心/两端宿主一起更新；新 ID/schema 黄金测试验证 Rust 确定性与双平台一致，验收不要求与旧 Python 字节/ID 相等。
- 旧任务切换前由旧引擎撤销/恢复，避免新核心承担旧 journal 解析；游戏/存档、订阅与未完成事务恢复材料保留。旧成功历史可留档，新版本无需读取；已有文件按当前检测识别，不能因无新收据自动视为未安装。
- 更新 P0/P1/P5、验收矩阵与收口条件，删除旧状态聚合/导入/回退和旧 profile 专用兼容检查。

### 本轮验证

- 文档复核通过：37 个相对链接有效，代码块闭合、无尾随空白；git diff --check 通过（仅仓库 LF/CRLF 提示）。现行设计已删除旧格式导入、ID 映射、旧 profile/收据读取和旧版本回退要求；施工记录保留初版历史并明确由本次修订取代。
- 本轮只改文档，未删除任何实际缓存、收据、事务或游戏文件；未改代码、部署或创建 Git 提交。

### 当前待办

1. 冻结新 ID/schema、缓存与恢复材料边界、内部授权/任务契约，不再冻结旧 schema 或回退接口。
2. 继续完成双平台隔离、跨进程互斥与 SF 通用语义门槛。
3. 实施旧任务收尾与新 schema 初始化，再按设计完成双端安装/恢复和生产包验收。

## 2026-10-08 来源兼容遗留并入安装器迁移

### 用户决策与本轮范围

用户确认 Publisher/EXE 后续也迁入 Rust，要求将来源兼容清理纳入本 feature 文档。主要清理随契约与消费者切换推进，作为本 feature 结项条件；不等安装器结项后另留清理任务。本轮只修改设计、施工记录与功能索引，代码及实际用户状态均未清理。

### 核对与实际修订

- 复用本会话源码/调用链只读检查：Rust 首次启动仍导入 Python 设置/cache/status/discovery/index；Decky 每次启动写 engine-switch；Python JSON/URL/哈希复刻、规则 schema 1 接受、Windows 旧核心降级及无用途参数/RPC 仍存在。Python 回退引擎本身已经删除，不能描述为仍有第二来源引擎。
- `matches/cached_candidates/_rules/builtins/validate_download` 仍服务当前 Publisher/EXE 安装器；安排 P5 随消费者迁 Rust 删除，避免提前重构临时 Python 安装接口。展示/订阅/事件薄桥接按新宿主需要保留。
- 设计第 7.3 节新增六类清理清单、代码入口、依赖、替代行为与实施门槛；P0 冻结契约，P1 清理导入/哈希/旧规则，P4 删除 Windows 旧核心降级，P5 切换消费者/启动方式，P6 检查全部遗留与产物。
- 修订第 5.1 节：来源 candidate/source_record、fingerprint/query_key/规则指纹与安装 ID 一起冻结 Rust 确定性编码；不要求复刻 Python 字节/ID，不维护旧 ID 映射。来源缓存可失效重查，当前 Rust 规则、启停与按 AppID 订阅保留。
- 补充完成定义、CLI 错误契约、实施阶段及自动化/实机验收；前序来源设计的现行兼容条款在 P6 同步更新，本期设计明确以新决策取代旧要求，历史施工证据不改写。
- 区分兼容残留与现行业务：`legacy_match` 是当前 ZIP 匹配门槛，HGamer fallback 是查询策略，保留行为；合法默认值、路径/大小限制、原子持久化、平台隔离能力及安装授权重核继续保留。

### 验证与当前待办

- 文档检查通过：设计、施工记录与功能索引共 80 个本地相对链接有效；代码块闭合、无尾随空白，阶段/索引保持待实现。git diff --check 通过，仅有仓库既有 LF/CRLF 提示；未跟踪功能文档另按全文检查链接/结构/空白。
- 功能仍为待实现；源码清理、测试/构建、缓存失效、设备/游戏写入与生产包验收均未执行。未创建 Git 提交。
- P0 冻结新来源/安装契约及设置保留边界；实施阶段按第 7.3 节逐项记录删除和回归证据，P6 未完成清单不得结项。

## 2026-10-08 P0 实施：契约冻结与 Windows 隔离原型

### 用户目标与本轮范围

用户要求开始实现本 feature，从 P0（契约/平台可行性）开始。本轮完成：新契约冻结设计（经三轮独立门禁评审）、Windows LPAC+Job 隔离原型与 31 项本机探针实测。未执行：Linux/Deck 隔离对照（需授权）、真实游戏/安装链路、P1 及后续阶段。

### 实际修改

- 新增 `crates/installer-worker/`（原型）：`model.rs` 跨平台数据模型（4 单测）、`stub.rs` 非 Windows stub、`src/win/{appcontainer,job,launch,probe_os,util}.rs`（LPAC profile/SID/ACL、Job 限额与回收、STARTUPINFOEX 启动器）、`src/bin/{probe_runner,probe_worker}.rs` 探针；根 `Cargo.toml` members 与 `Cargo.lock` 同步。
- 新增功能文档：`p0-contracts.md`（契约冻结 v0.5，含来源/安装 ID 与确定性编码、新状态 schema `installer/rust-v1/`、锁顺序、取消/租约、worker 协议与限额、规则 schema、依赖/许可、fixture 清单；宿主事件契约显式延后 P1）、`p0-sandbox-probe-windows.md`（探针证据）、`p0-gate1-review.md`（三轮门禁评审归档）。
- 同步 `design.md` §8 P0 行（宿主事件契约延后指向）。
- `.gitignore` 增加 `.slim/deepwork/`、新增 `.ignore`（deepwork 本地状态隔离，不入库）。

### 验证方式与结果

- 探针（本机 Windows，build 26300；2026-10-09 复核为 Win11 26H2 正式版）：31 项 = 27 pass / 0 fail / 1 未覆盖（CPU rate hard cap，宿主嵌套 Job 限制）/ 3 info；LPAC 身份、零 capability 无网络、越界读写/注册表拒绝、句柄白名单、进程树回收、内存与 JOB_TIME 限额、无控制台、中文长路径均通过。
- 构建：`cargo check --workspace`、`cargo test -p steam-patch-installer-worker`（4 passed）、`cargo metadata --locked` 通过（隔离工具链）。
- 门禁：独立 oracle 初审（条件放行）→ 15 条修订（v0.4）→ 定向复审（N1/N3 必修 + N2/N4/N5）→ 二轮修订（v0.5）→ 终审（attempt 3/3）**通过，无定稿前必修项**；结构扫描确认改动未越界（core/cli/apps 未动）。

### 未执行（保持待验证）

- Linux bwrap 对照原型与 Deck 实机（需用户授权）；Windows 标准 Win11 零售机版本矩阵（列为 P4 发布条件）；reparse/跨卷/杀软/profile 崩溃清理用例（P4 前）；环境块精确 allowlist、注册表键集、worker 目录实施冻结（P1）。
- 真实下载/安装/游戏内生效；三旧 profile 包的可得性（不在仓库，P2 前由用户提供或降级合成 fixture）。

### 待办

1. 已完成：用户确认 Windows 支持版本冻结为仅 Win11 x64（2026-10-08），P0 契约定稿。
2. 已完成：P0 阶段成果提交（b0ade70 文档 / 1ef5388 原型代码 / ef1671f 配置；未推送）。
3. 下一步（用户选择）：P0-D Linux/Deck 对照（bwrap 启动 Rust worker 原型 + 实机对照；Deck 操作按授权范围）。
4. P1 开工前置：冻结宿主 RPC 方法/事件/终态契约（p0-contracts §11）；P1 实施：来源编码替换/规则收紧/直接初始化、共享 ZIP 获取-配方-授权-状态路由。

## 2026-10-08 P0-D：Linux bwrap 原型与 SteamOS Deck 实机对照

### 用户目标与本轮范围

完成 P0-D Linux/Deck 对照：实现 Linux bwrap 启动器与探针并在 WSL2 验证，再在真实 SteamOS Deck 上运行同一探针。本节记录 Deck 实机验证与证据归档；实机未发现需针对 bwrap 0.12 修复的差异，**未修改 `crates/installer-worker` 代码、未创建 Git 提交**。

### 实际修改

- Linux 平台实现与探针（详见 [p0-sandbox-probe-linux.md](p0-sandbox-probe-linux.md) §3、§9）：`crates/installer-worker/src/linux/{mod,sys,launch,probe_os}.rs`、`model.rs` Linux rlimit 字段、`lib.rs` 平台 gating、`probe_worker.rs`/`probe_runner.rs` 的 `linux_impl`。未改 `Cargo.toml`/`Cargo.lock`（无新依赖），未改 `core`/`cli`/`apps`。
- `p0-sandbox-probe-linux.md` 本次追加第 10 节「SteamOS / Deck 实测」并同步首部范围声明与 §7 未覆盖表。

### 验证方式与结果

- WSL2 Ubuntu 22.04（bwrap 0.6.1，内核 6.18.40.1-WSL2）：22 项 = 21 pass / 0 fail / 1 info。
- SteamOS Deck（bwrap 0.12.0，内核 6.18.50-valve，deck uid=1000）：22 项 = 21 pass / 0 fail / 1 info，逐项结果与 WSL2 一致；越界读写/私钥读在真实 `/home/deck/...` 路径仍 `ENOENT`，回环/外网连接被拒。
- 上传与取回均逐文件核对 SHA-256 一致；远端唯一验证目录 `/home/deck/steam-patch-validation/p0d-20261008-033904/` 取回证据后已删除，未触碰插件/state/游戏目录。
- 证据：`p0-sandbox-probe-linux.md` §10；本机 `.artifacts/p0d-deck-20261008-033904/probe.json`（SHA-256 `cf3f110e2ac3f11ec8584c63b9150bf757b50a29e5aa1fbbeab713e620b8a8bb`）。

### 未执行（保持待验证）

- Deck 上 cgroup v2 委派与 seccomp 策略、真实 SF9 worker 与黄金产物、并发/跨用户、ARM64、正式超时值（wall-clock 180/240/480s、CPU 300s）、无 core dump 端到端证明、CEF/HTTP 与插件部署。

### P0-D 门禁修订（2026-10-08）

- 独立 oracle 初审（attempt 1/3）条件通过；已执行 remediation：M3 输出限额 fail-closed（stdout 16 MiB / stderr 64 KiB，超限终止并报 `exe_worker_response_limit`）；M1 root 宿主最小降权（setgroups→setgid→setuid）并补 WSL root 场景实测（26 pass / 0 fail / 1 info；负对照无 `--run-as` 时 worker uid=0，证明降权路径有效）；非 root 回归 23 pass / 0 fail / 1 info。
- M2（Linux 树级资源限额与 Windows Job 不对称）与评审 R1-R6 已登记（见 [p0-contracts.md](p0-contracts.md) §10、[p0-sandbox-probe-linux.md](p0-sandbox-probe-linux.md) §8）；`design.md` §4.3 已同步 root 降权补测状态。
- 未执行：Deck 上 root 场景（Decky Loader 实际形态）与 cgroup/seccomp 仍待 P1/P3；未提交 Git。

### 待办

1. root 降权生产化（Decky Loader root 形态、完整环境处理）与 cgroup v2 / RLIMIT_NPROC 树级限额在 P1/P3 落实。
2. 按 [p0-sandbox-probe-linux.md](p0-sandbox-probe-linux.md) §8 建议冻结 Linux 侧生产配置（待 cgroup/seccomp 评估）。
3. 已完成：P0-D 成果提交（bbd2edd / 14ab369）。

## 2026-10-08 P1 启动：宿主契约冻结与来源编码迁移

### 用户目标与本轮范围

用户确认启动 P1。前置（p0-contracts §11）要求先冻结宿主 RPC/方法/事件/终态契约，随后实施来源确定性编码迁移（P1-1）。本轮完成：宿主契约冻结稿（独立初审 + 定向复审通过）、来源编码迁移（独立评审通过）与评审收尾补全。未执行：P1-2（installer/rust-v1 骨架）、P1-3（共享 ZIP 服务）、两端宿主切换（P4/P5）。

### 实际修改

- 新增 `p1-host-contract.md`（v1.2 定稿）：`protocol_version=3` 方法白名单（安装器 8 + 来源 8；删除 `plan_install`/`start_install`/`fetch_artifact`/`migrate_state` 与 5 个来源旧方法）、`install_progress` 事件（9 阶段、平铺字段、递增 `sequence`、终态 `InstallOutcome`）、能力协商（`installer.links` 平台探测 + 错误 `detail`）、容量限制、幂等/租约/取消（15s + `renew` 控制行）、双端宿主适配与测试要求；新增 `p1-gate0-review.md`（初审 + 复审归档）。
- `crates/core/src/sources/`：`canonical_bytes` 取代 `python_json`（确定性 JSON：UTF-8 字节序键序、无空格、原样 UTF-8 仅转义必需字符、浮点/整数溢出拒绝）；`candidate_id`/`fingerprint`/`query_key`/`rule_fingerprint` 全部改走新编码；`SOURCES_ENCODING_VERSION=1` 入 `query_key` 并持久化进 discovery/index 记录（`encoding_version` 字段，缺失=0），`discovery_fresh`/`shared_index` 比对失效；`identity-golden.json`/`gen_identity_golden.py` 删除，新增 `encoding-golden.json`。
- `crates/cli/src/runtime.rs`：测试辅助同步 `query_key` 语义。
- `p0-contracts.md`：§1.1 端口行为修正为“原样保留（现行行为）”；§10/§11 回写宿主契约已冻结。

### 验证方式与结果

- 编码迁移：`cargo test -p steam-patch-core`（lib 149 passed / 1 ignored，集成 60 passed）、`cargo test -p steam-patch-cli` 19、`cargo check --workspace`、`cargo clippy -p steam-patch-core --all-targets`、`rustfmt --check`（改动文件）通过；旧编码记录失效、版本 stale、字段重排稳定、浮点/整数溢出拒绝、NFC-NFD 不归并、大小闸门用例通过。
- 宿主契约：独立 oracle 初审（MF1-MF6 必修 + N1-N7 登记）→ v1.1 → 定向复审（R1-R3 一行级折入）→ v1.2 定稿；编码迁移独立评审“通过（P1 基线）” → 收尾补全（`canonical_url` 文档纠错、fixture `:443`、3 个新测试）。
- 未执行：P1-2/P1-3、两端宿主切换（P4/P5）、实机协议验证（待 P4/P5）。

### P1-2/P1-3a 实施（2026-10-08 续）

- **P1-2 状态骨架**（`crates/core/src/installer/`）：`InstallerPaths`（schema/journal.lock/journal/requests/jobs/content/receipts/downloads/reports）、`init_atomic()` 原子幂等初始化（staging → sync → rename，`schema.json` 最后写、残留可重试）、可清/受保护边界 API、`instance_hash`（sha256 前 16 hex）、`requests.sqlite` 幂等账本骨架（`RequestBinding`/`RequestRecord`/`RequestLedger`）；`install.rs` 增加 `Store::open_at` 可配置 root（生产调用方未切换）；8 个新测试，core lib 157 passed。
- **P1-3a 协议外壳**（`crates/cli`）：`PROTOCOL_VERSION=3`（v1/v2 → `unsupported_protocol`；`error.detail` ≤2 KiB 截断；`request_id` 校验 1–128 字节/非空/无控制字符）；16 方法白名单（安装器 8 + 来源 8；9 个旧方法的 CLI 分支删除、核心内部实现保留）；`get_capabilities` §3.1 形状（links 探测留 TODO）；`cleanup_cache` 最小实现（downloads/ 清扫）；`install_progress` 平铺事件（阶段折叠在 CLI 层完成，`operation.rs` 保持与 HEAD 一致）+ `renew`/`cancel`/EOF 控制行；cli 25 测试通过。
- 验证：`cargo test -p steam-patch-core -p steam-patch-cli`（lib 157 + catalog 8 + install 39 + localized 4 + steam_scan 6 + trust 3 + cli 25，全绿）、`cargo check --workspace` 通过；全量复核确认 `operation.rs` 无 diff。
- TODO（已登记）：租约落盘（job 文档）、`installer.links` 隔离探针注入（P3/P4）、`cleanup_cache` 接入 installer store。

### P1-3b-1 动态获取与 ZIP 配方（2026-10-08 续）

- `installer/acquire.rs`：动态 HTTPS 获取（每跳重核 + `download_hosts` 白名单、无凭据/端口 443/.zip 路径、Content-Type 与 ZIP 魔数校验）、内容寻址缓存（`downloads/<sha256>.zip` + `.partial` + `<sha256>.json` sidecar）、ETag/Last-Modified 条件请求（304 仅同身份完整包）、复用前重哈希、中断不发布、预期摘要/大小核验、进度与取消（`download_cancelled`）、8 GiB 上限。
- `installer/zip_recipe.rs`：publisher 配方移植（8192 条目 / 4 GiB 单文件 / 16 GiB 展开 / 压缩比 200；路径/设备名/链接/extra/大小写与 Unicode 冲突/目录-文件冲突等拒绝项与 Python 1:1；XP3 包装映射与歧义阻断；输出 `{source,destination,original_sha256,installed_sha256}`）。
- `installer/mod.rs` 注册两模块；未改 `download.rs`/`install.rs`/CLI/Python。
- 验证：core lib 177（+20 新测试，含获取/缓存/304/取消与配方拒绝矩阵）、集成 39/8/6/4/3 全绿；workspace check、clippy、fmt 通过。
- 已知偏差/注记：① `zip` crate 无 UTF-8 flag 访问器，名称编码用启发式（非 ASCII 无 flag 正确拒绝；malformed UTF-8 + flag 场景与 CPython 有差异）；② `downloads/` 新增 `<sha256>.json` sidecar（p0-contracts §3.1 已同步注明）；③ `load_cached` 按请求扫描 sidecar（预期规模下可接受）。

### P1-3b-2 实例事实与新内部授权（2026-10-08 续）

- `installer/facts.rs`：`GameFacts` 快照（ACF/manifest 身份、stateflags/buildid、Steam 语言、DLC 拥有/挂载、基线 sha256/crc32/存在性；`read_file` 描述符/路径身份前后校验与 `no_links`/`O_NOFOLLOW`；自实现 CRC-32 与 `binascii` 一致；失败码 `exe_manifest_*`/`exe_game_identity_changed`/`game_not_ready`/`build_not_verified`/`exe_language_mismatch`/`exe_dlc_*`/`exe_baseline_*`）。
- `installer/authorization.rs`：完整绑定（实例/路径、候选/来源 + settings revision、规则/条件、内容/转换产物 + 有序 mapping 摘要、plan/task/request、策略版本、facts 快照、期限/撤销）；`authorize`（TTL 1..=900、拒绝带失败 facts）+ `validate` 重核矩阵（12 项有序诊断：schema → 撤销 → 租约 → 过期 → 归属 → revision → 策略 → 候选 → 规则 → 条件 → 内容 → 事实）；持久化 `jobs/<plan_id>/authorization.json`（受保护），`revoked` 标记。
- `installer/mod.rs`/`store.rs` 注册与 `job()`/`authorization()` 路径辅助（`write_atomic` 提升 `pub(crate)`）。
- 验证：core lib 190（+13 新测试）、集成全绿、check/clippy/fmt 通过。
- 遗留：协议接线与编排（P1-3b-3）；`conditions_sha256`/`candidate_sha256`/`rule_fingerprint` 由调用方提供（条件求值器与来源绑定摘要为后续任务）；成功收据 schema 后续。

### P1-3b-3 动态 publisher ZIP 编排与协议接线（2026-10-08 续）

- 设计：`p1-zip-orchestration.md`（ora-5 设计：zip-selection 表示、resolve 数据流、orchestrator、协议接线、测试与回写清单）；契约回写已落 p0-contracts §2.5 等与 p1-host-contract 增量。
- 实施（fix-12）：`installer/resolve.rs`（`zip_selection_id`/`parse`、`ResolvedZipCandidate`、`resolve_zip` 与稳定错误码、released 门）；`installer/orchestrator.rs`（`install_zip` 单任务：connecting→acquire→recipe→facts→prepare→authorize→重核→apply→cleanup；幂等 `begin_request`/`content_conflict`/`record_plan`/`record_terminal`；取消/租约边界与计划后 discard）；`install.rs` `_at` API 族 + `PlannedChange.source` + `open_ro` 只读快照；`catalog::SupportLevel::{PublisherZip,PublisherExe}` + `PatchMatch.selection_id`；CLI：install_patch 路由分派（互斥→签名优先→zip/exe 前缀）、`install_zip_route`、签名路由迁新根/新账本/授权重核、preview 路由、scan 动态投影、`discard_plan`/`list_transactions`/`recover`/`cleanup_cache` 迁 installer 根、`LeaseState` + 看门狗。
- 验证：core lib 199 + 集成 60 + cli 28 全绿；workspace check、clippy、fmt（改动文件）通过；重点用例（阶段序列、幂等重放、计划前取消、路由矩阵、archive 拒绝、preview 只读、签名迁新根）。
- 实施注记（设计缺口裁决，待 P1 gate 审议；已同步设计稿 §9）：① `!discovery_only` 校正为仅 `enabled`；② `source_not_enabled`/`source_not_subscribed`/`selection_stale` 在 resolve 不可达（过滤后为 `selection_not_found`），仅 scan 投影保留；③ buildid 交接经 `ResolvedZipCandidate::expectation(buildid)`；④ 签名路由授权绑定为实施确定值（`source_id="signed"`、`settings_revision=catalog.revision`、release 派生摘要）——待冻结或收窄；⑤ `error.detail` 经内部 `\u{1}` 分隔（后续 typed error）；⑥ 空 journal 容忍与 `cleanup_cache` 保持旧形状。
- 测试补全（2026-10-08 续，fix-12）：设计 §5 清单 7 组已实现——resolve 逐码矩阵（按可达语义：`selection_not_found`/`selection_stale`/`zip_download_ambiguous`/`invalid_download_url`；不可达码以实际语义覆盖并文档化）、facts 语言/DLC/manifest 矩阵（均断言不写事务）、逐阶段取消（mid-download/planning 前/apply 中/迟到）+ 租约过期撤销、`recover_at` rolled_back/needs_recovery、动态重装三态、CLI 端到端 scan 投影（签名优先去重）/preview 只读、计划前失败可重试。测试：cli 32、core lib 219、集成 8/40/4/6/3 全绿；未改实现逻辑。

### P1 收尾清理（2026-10-08 续，fix-13）

- **删除 Python 状态导入**（`sources/store.rs` 符号级）：`import_python_state`/`import_cache`/`import_status`/`import_discovery`/`import_index`/`load|save_import_report`、`ImportReport`/`RawOverrides`、`StorePaths` 的 `python_*` 访问器、导入 staging/诊断辅助与专用测试全部移除；`SourceStore::open(state_dir)` 不再接收 `builtins`。
- **直接初始化**：`schema.json` 缺失时直接写 Rust 默认 `settings.json`（`STORE_SCHEMA_VERSION`、revision=1、空 overrides）+ 最后写 `schema.json`；不探测/不读取 Python `overrides/cache/status/discovery/index`。
- **规则 schema 仅 2**：`mod.rs` 门槛收紧为 `schema_version == 2`（1/0/3 → `invalid_sources_config`）；`validate_rule` 拒绝语义不变（publisher 必带 `root_layout=="game_root"`、discovery 必须缺失）；schema1 接受测试删除、新增拒绝与默认值用例。
- 验证：core lib 221 + 集成 8/40/4/6/3（=61）+ cli 32 全绿（fix-13 时点；fix-14 新增测试后终值为 core lib 223 / cli 33）；check/clippy/fmt 通过。
- 残留（P5）：Decky `source_service.py` 导入诊断/`import_report` 引用与 `test_source_import/engine/service.py`；`engine-switch`；`MAX_OVERRIDES_BYTES` 未用常量与 `installer/store.rs` doc 引用（后续清理）。

### P1 gate 初审与修订（2026-10-08 续）

- **独立初审**（独立 @oracle 会话 ora-6，报告归档 [p1-gate-review.md](p1-gate-review.md)）：结论 **P1 有条件放行进入 P2**——§8 自动化门槛证据充分；实机/真实写入属 P4/P5 待验。必修 F1–F6 + 低危 F7/F8（不静默延期）。后台独立跑测 `cargo test -p steam-patch-core -p steam-patch-cli` 全绿（exit=0）。
- **权威测试计数（P1 终态，以单次 `cargo test -p steam-patch-core -p steam-patch-cli` 为准）**：core lib **223** + 集成 **8/40/4/6/3 = 61** + cli **33**（fix-14 后终值）。此前各节按阶段记录的快照计数（如集成 60/58、cli 32）仅为历史，不再作为终态依据。
- **F1–F8 清单与处置**：

| 项 | 级别 | 问题 | 处置 | 归属 |
| --- | --- | --- | --- | --- |
| F1 | 高 | `source_not_released` 在动态 ZIP 路径不可达（契约承诺 vs 实现缺口） | 从动态 ZIP 契约移除并登记“待候选日期字段补齐后另行引入”（[p1-zip-orchestration.md](p1-zip-orchestration.md) §2.1 step 7/§4.4、[p1-host-contract.md](p1-host-contract.md) §1.1） | 文档本任务 |
| F2 | 中 | `install_patch` 可经 `serve`（非 serve-task）执行 | `serve` 拒绝任务面方法并返回 `task_required` | 代码 fix-14 / 文档标注 |
| F3 | 中 | 租约未落盘（仅内存 `LeaseState`） | 补做 `jobs/<plan_id>/lease.json`（`{deadline_ms, renewed_at}`，`write_atomic`） | 代码 fix-14 |
| F4 | 中 | 动态路由租约用固定 `now`（恒真） | 改实时检查，与签名路由对称 | 代码 fix-14 |
| F5 | 中 | 只读 `scan`/`preview` 通过独占 `journal.lock` 读 plan | 走只读快照（`open_ro`） | 代码 fix-14 |
| F6 | 中低 | 计划后失败形态不一致 / 幂等重放形态漂移 | 取消/租约过期/撤销 → `InstallOutcome`（`rolled_back`/`needs_recovery`）；其余计划后漂移失败保持 `ok:false+error` 并留终态事务（重放返回该终态，契约允许）——对齐 §2.3/§4.4 | 代码 fix-14 / 文档收窄 |
| F7 | 低 | `SourceStore::open` 在 `schema.json` 缺失但 `settings.json` 存在时覆盖用户设置 | 优先复用现有 settings | 代码 fix-14 |
| F8 | 低 | 语义/文档债（头注释、`exe-policy.json` 注释、`ManagedZip` 借用、无 fixture） | 随修复/后续清理 | 代码 fix-14 / 后续 |

- **文档修订（本任务）**：[p1-zip-orchestration.md](p1-zip-orchestration.md) §2.1/§4.1/§4.4/§6.2/§7/§9 同步（F1 移除、stale 更正、§4.1 step 2 收窄、签名路由绑定冻结值、F3/F4/F6 注记）；[p1-host-contract.md](p1-host-contract.md) §0.1/§1.1/§3.3/§5.1/§5.5/§8 同步（F2 门禁、错误码、绑定冻结、租约落盘）；本文件计数统一。只改文档，未改代码、未提交 Git。
- **修复归属**：代码 fix-14（另一任务）；文档修订本任务（归档报告记 fix-15，`.slim` 计划记 fix-16）。
- **后续**：定向复审 attempt 2/3；P2 主体（SF 对象图/AST/IR、worker 协议、LZMA2、确定性 ZIP）与动态 ZIP 路径低耦合，可并行推进。

### 待办

1. 已完成：P1 全部实现与清理（P1-0/P1-1/P1-2/P1-3a/P1-3b-1..3/收尾清理）。
2. **P1 gate**：独立初审完成（**有条件放行进入 P2**，报告归档 [p1-gate-review.md](p1-gate-review.md)）；必修 F1–F6（+F7/F8）已按归属分流——代码 fix-14（另一任务）、文档修订（本任务，见上节）；定向复审 attempt 2/3 待执行。
3. P2 前置：SF fixture 冻结（三条旧 profile 包可得性待用户提供）、`installer-format`/`installer-worker` 开工。

## 2026-10-08 P2 Rust SF与固定worker

用户说明P1开发完成，要求继续P2。沿用P1已有测试与修订记录，不重复P1验收。实施边界见 [p2-rust-sf.md](p2-rust-sf.md)。

### 实际实现

- 新增 `crates/installer-format`：完整PE/resource manifest/overlay/证书、受限raw LZMA2、SF9.5–9.7文件表/未知字段证据与完整CArchive对象图、展示文本编码、Lua AST与静态行为IR。库无宿主/网络/游戏文件访问或脚本执行依赖。
- `installer-worker` 新增固定 `sf-worker`，严格动作/键/摘要/大小描述符、只读映射、文件身份重核、完整元数据/IR/载荷绑定、流式确定性ZIP、create_new partial及同卷原子无覆盖发布。分析始终不授予安装权限。生产CLI/宿主EXE路由仍未接线（P3）。
- 新增黄金采集与开发隔离对照脚本、LPAC/bwrap example。合成fixture不含真实发布方代码/二进制；真实EXE只在固定worker沙箱中解析，宿主只复制/哈希。Python仅为开发黄金工具，不是Rust测试或Windows产品运行时。
- [独立审查及修订](p2-gate-review.md)：字符串/XML namespace内存放大、精确整数/浮点比较、冻结全局作用域、循环表/大DAG、全局别名性能均修复并有回归。真实SF9.5发现的5种literal模式双重转义修复后完整报告一致。
- 收尾发现CPython ZIP64 central/EOCD阈值为2^31−1；补齐近2GiB文件/条目偏移的central extra及结束记录，新增9组阈值等于/超过、组合extra与Unicode的独立header黄金。以最终构建重新执行真实矩阵；旧构建证据保留为历史。
- P0契约/设计/功能索引同步P2实现边界、额外保守限额与依赖；保留前序P0/P1历史和未执行项。

### 自动化与构建

- 首轮Windows本机：`cargo test --locked -p steam-patch-installer-format -p steam-patch-installer-worker` **51 passed / 0 failed**，分组 format lib2 + container11 + golden6 + graph7 + semantics14 =40，worker lib6 + integration5 =11（含4项已有model测试）。
- 首轮Linux/WSL2：同一测试命令 **51 passed / 0 failed**，冻结黄金和新增拒绝/限额矩阵通过。6个黄金测试组共75个输入：AST21、behavior20、graph13、inventory9、audit6、完整report6；worker再验证6个跨版本/中英文路径规范化产物。
- 收尾ZIP64边界修复后，两端定向 `cargo test --locked -p steam-patch-installer-worker` **12 passed / 0 failed**（lib7 + integration5），新增1项测试读取9组独立CPython header黄金。最终P2合计 **52项**（未改动format40项沿用上轮通过记录 + worker12项）；不分配2GiB载荷，不重复未受影响的P1测试。
- `cargo check --locked --workspace` 通过；Windows新两crate `clippy --all-targets -- -D warnings` 通过；所有P2改动Rust文件 `rustfmt --check` 通过。
- Linux `installer-format --all-targets`及worker新增lib/bin/test/example目标严格clippy通过。Linux全worker `--all-targets -D warnings`仅触发旧 `probe_worker.rs:693` 的 `slow_vector_initialization`：原有探针刻意try_reserve捕获分配失败，不为清告警改动P0逻辑；该旧探针不计作P2新代码检查失败。
- Windows/Linux release固定worker与隔离example构建通过；体积Windows **1,398,272字节**、Linux **1,758,856字节**。完整便携包集成/体积验收留P3/P4/P6。
- 最终worker SHA-256：Windows `2c48291bbdbb84ba337d13dc15ad31b37ae5fcd8587e9693df0929b54b759066`；Linux `c02f853c4ec294d9f5b0d818f92e31e61d5338a9d8e6bc42df909ce862f9c163`。收尾真实矩阵重新运行，并逐一核对所复制二进制与这些最终构建一致；旧构建的结果保留为历史证据。
- `cargo metadata --locked`与`build-sbom.py`通过，744组件、许可元数据缺失0；新lzma-rust2 0.21.0 / quick-xml 0.38.4 / memmap2 0.9.11和两个crate均入SBOM，快照 `.artifacts/p2/dependencies.cdx.json`。原sfextract MIT布局参考与commit保留。

### 真实离线样本对照

三条旧profile二进制当前仓库缺失；本次采用现有registry外三份真实EXE，分别覆盖9.5/9.6/9.7。不下载、不执行官方EXE/Lua、未写游戏。

- **Windows本机LPAC（build26300；2026-10-09 复核为 Win11 26H2 正式版，本机证据即正式版证据）**：三版本中文路径合成转换3项 + 真实分析/转换6项 = **9/9通过**。零capability、Job单进程/3GiB内存/480s、固定参数与干净环境。完整分析report逐字段一致，元数据/行为/文件清单与摘要一致，转换产物按冻结的CPython 3.12 ZIP规则重建基准后一致。
- 最终Windows证据 `.artifacts/p2/windows-1791473791831110600-summary.json`；SHA-256 `317af9f0f454a922b12a70e787a35703194f90357396e831d0b10ad904b208d5`。
- 历史初轮双平台亦9/9通过：Windows `.artifacts/p2/windows-1791471990092010900-summary.json`（SHA-256 `e2be58171998bc277cbcf5fd17f205bfa276831ced4dbeb1d4a98e4250bb79cc`）；Linux `.artifacts/p2/linux-1791472393558943840-summary.json`（SHA-256 `4b9edad15de5531cb44097e01609e3b01cda186a48d7ffe0bc81753bed0510a5`）。收尾ZIP64边界修复后重新运行矩阵；最终两端摘要与初轮及彼此逐项一致，历史汇总保留在 `.artifacts/p2/pre-zip64-cross-platform-parity.json`。
- **Linux/WSL2 bwrap**：最终构建三版本合成转换3项 + 真实分析/转换6项 = **9/9通过**；完整report逐字段、行为/元数据/载荷摘要和规范化ZIP全部与冻结Python及Windows结果一致。WSL对照不代替P5 Deck实机。
- 最终Linux证据 `.artifacts/p2/linux-1791473908752499778-summary.json`；SHA-256 `42fc0bc94dcbb7d4fba95452e32994ae97f070cbbbf7b96d5e65a56e5d505107`。
- 两端最终9项摘要逐项交叉复核，18个case所复制worker逐一匹配最终构建；汇总 `.artifacts/p2/cross-platform-parity.json`，SHA-256 `559ecd5047efeb41a8d272f88f9ee55dc5f1562d7a8687bd01c649dd48237279`。

| AppID / 版本 | EXE SHA-256 | behavior SHA-256 | 冻结基准规范化ZIP SHA-256 |
| --- | --- | --- | --- |
| 2303800 / SF9.5 | 7336c5c9876bf239811478001741862256610c77298c745d3bc035a8bcdd94f3 | 7fbc67758d3af82da8d89d3c72601ebe71208bb2a80508caa84ff9c69e6258d4 | e7e232c192f06cad4cb7c5d2a13de67f4eb90834f615d5000aae8240f47fab89 |
| 3606360 / SF9.6 | 6a1018168add626642036a70394d30562724aac645985bfefadb2f235c04f42d | 9f363cd16df9669aa0f92b33f5e1b271b0f5c8d54a2c684f9d7c8d1af979bb12 | 6d8c5d929fae5c40e526b99067639f1240f064257eac47b60b99b22368e25b3b |
| 3721970 / SF9.7 | 67d258e9cad0f86649282862d2439fb4a796946f7a6182bcfe6880ce855182a8 | 8225c2a67934c1271a1c0bd3254cf76d0a8ce8e6ec13bc8596c79a3e2256cd36 | 24c2fbca3d2781aa8e165a607b7ffd9b5df7a63fac054db6495f28ea15c0c936 |

历史normalized ZIP使用Python 3.10的local/central header版本，与冻结的CPython 3.12 force_zip64不同，第一次整包比对因此失败；未误判为载荷差异。先按冻结报告核验历史ZIP清单/各载荷SHA/大小，再以冻结的CPython 3.12 ZipInfo/force_zip64重新编码。开发oracle显式固定ZIP64版本/大小占位，在Python 3.10.12与3.12.10下分别验证6/6合成黄金一致。最终Rust与重建基准的整包摘要一致；不执行真实EXE的Python解析。`--reuse-run`只复用同worker SHA、同描述符、成功退出且再次核对报告/产物的本次结果；未拿历史P1验证冒充P2结果。

### 当前结论与待验

P2主体已实现，双平台自动化与可得SF9.5/9.6/9.7三份真实样本分析/转换均通过；独立审查阻断与收尾ZIP64边界均已修复。三条旧profile真实包回归仍待样本，不宣称完整P2真实样本门槛已结项。P3生产启动器/授权/facts/空间/撤销/生命周期接线、P4 Windows真安装与标准版本矩阵、P5 Decky切换、P6收口仍待执行。脚本语法、功能文档146个本地链接与新增文本空白检查通过；`git diff --check`通过。未连接Deck、未部署、未安装游戏补丁、未创建Git提交。


## 2026-10-09 P2 Git 提交归档

- 按用户“提交GIT”要求创建本地提交，代码与文档分开；代码提交 `d88d223`（`feat(installer): 实现 P2 Rust SF 解析与固定 worker`），包含解析库、固定worker、合成黄金/回归和开发隔离对照工具。
- 本次文档提交归档P2设计、独立审查、契约增量、施工记录和功能索引；保留2026-10-08各轮验证与未提交时的历史事实。
- 沿用本session已有双平台52项相关测试、各9/9隔离对照与构建记录，未重复运行。提交前工作区/暂存区空白、文件范围及合成黄金检查通过；未纳入真实安装包、构建产物或 `.artifacts/` 验收材料。
- 三旧profile回归和P3–P6/两端实机状态未改变；未推送、打tag或改写历史。


## 2026-10-09 — P3 开发启动（待验证）

- 实施范围：[P3 生产运行时](p3-runtime.md)：固定隔离 worker 生产启动器、EXE 来源选择/内容身份、下载公共 IP 连接绑定、严格报告与现场条件核验、空间预算、任务租约及持久撤销、CLI 能力探测与 EXE 接线。
- 使用 Sol 子 Agent 分担启动器、纯报告/条件/空间与独立安全复核；不使用 Astra。
- 此时仅进入开发，不表示实现/自动化/实机验收完成。P4 Windows 版本矩阵/真实安装及 P5 Decky 部署不在本次操作范围。

## 2026-10-09 P3 实施、复核修复与双平台证据

### 用户目标与本轮范围

用户说明 P3 施工进行到一半，要求继续完成剩余任务。本轮在既有工作区实现基础上：确认编译与测试状态、补齐独立复核修复（M1/M2/R1）、完成双平台真实样本与隔离证据、归档复核并回写功能文档；并裁定 P3 验收自动化必须使用 release worker。未执行：P4 Windows 真安装、P5 Decky 部署、三条旧 profile 真包回归（样本不在仓库）。未创建 Git 提交。

### 实际实现（P3 生产运行时与 EXE 编排）

- **worker 生产启动器**（`crates/installer-worker/src/production.rs`）：受信宿主注入的固定 `WorkerRuntime`（exe/stage/run_as 均不由 RPC 提供）；`probe()` 真实执行固定 worker（inert 无效输入必须以 `not_pe` 失败）；每次调用私有 staging（`owner.lock`/`owner.json`、跨进程锁、崩溃残留回收）；`copy_bound` 身份核验后复制输入，只读绑定；派发 descriptor 严格解析（动作/摘要/大小/载荷清单，16 MiB 上限）；超时 analyze 180s / analyze_behavior 240s / convert_behavior 480s、内存 3 GiB、单进程、CPU 300s、输出 16 MiB/64 KiB fail-closed、取消轮询、父退出回收；响应二次校验（binding、路径/动作不可被响应篡改）。
- **双平台启动器生产化**：Windows LPAC+Job（`PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY`、`JOB_LIST`、句柄白名单与 PipeHandles Drop、`WAIT_TIMEOUT` 处理、profile 清理）；Linux bwrap（`run_sandboxed_cancellable`、seccomp 单进程过滤器 deny `clone/fork/vfork/clone3`、root 宿主强制显式非 root `run_as`、`PR_SET_PDEATHSIG`、`env_clear`、`OVERFLOW_IO`）。
- **core EXE 编排**（`installer/{exe_orchestrator,exe_report,exe_resolve,space,task}.rs`）：来源缓存重算 selection 与解析；EXE 获取（download_hosts 白名单、公共 IP 解析并 `resolve_to_addrs` 绑定连接、`no_proxy`、MZ 魔数）；报告严格校验（exe sha/size、metadata/behavior/有序载荷摘要、`installable=false` 不授权、未知动作/字段/数值/深度拒绝）；静态条件求值（`GameFacts` + 重装证据，仅 owned 且当前 SHA 匹配可复用原条件）；空间预算（下载/转换/stage/原文件恢复，同卷合并，`insufficient_space`）；任务控制（acquire 前建立任务 job；撤销/租约先持久 `revoked`/`lease.json` 再触发有效取消）；apply 前重核（fresh resolve + facts + condition + report 读回 + artifact 重哈希）。
- **CLI 接线**（`crates/cli/src/runtime.rs`）：`install_patch` 按 `exe-selection-` 前缀路由；`get_capabilities` 返回真实沙箱可用性；scan/preview EXE 投影；`cleanup_cache` 接入任务清扫。
- **获取扩展**（`installer/acquire.rs`）：`acquire_exe_with`（2 GiB、MZ、`.exe` 路径）与公共 IP 策略测试。

### 独立复核与修复

[独立复核](p3-gate-review.md)（只读 oracle）：**有条件放行**。必修 M1（任务目录无回收、报告写入受保护 `jobs/`）、M2（只读路径写 installer root 且逐候选起沙箱）；契约偏离 R1（EXE 投影 release_id 冒充选择身份）。修复：
- M1：任务运行态迁移 `tasks/<uuid>/`（owner.lock 排它锁、终态回收、`cleanup_stale_tasks` 崩溃残留清扫、cleanup_cache 接入）；`report.json`/`space.json` 改落 `reports/<content_id>/`。
- M2：`exe_sandbox_availability` 进程内缓存 + OS 临时 staging，只读路径不写 installer state；安装路由保持状态根 staging 与运行时重探。
- R1：EXE 投影 `release_id` 改 `dynamic_release_id(source_id, appid)` 并去重，`selection_id` 仍为路由句柄。
- R2/R3/R5/R6/R7/R8 建议项登记（[p3-gate-review.md](p3-gate-review.md) §3），不阻塞 P3。

### 验证方式与结果

- **修复后自动化（最终代码）**：Windows `cargo test -p steam-patch-core -p steam-patch-cli` → core lib 249 passed / 4 ignored，集成 catalog 8 / install 40 / localized 4 / steam_scan 6 / trust 3 全绿，cli 36 passed；WSL2（Ubuntu 22.04）core lib 250 passed / 4 ignored、cli 36 passed。两平台 `cargo clippy -p steam-patch-core -p steam-patch-cli -p steam-patch-installer-worker --all-targets -- -D warnings` 通过；本次改动文件 rustfmt 通过（`apps/windows/src-tauri/src/main.rs` 的既有格式差异为 HEAD 遗留，非本次引入）。
- **双平台 ignored 生产测试（release worker）**：`precancel_persists_revocation_without_worker_or_game_writes`、`sandbox_to_authorization_install_retry_reinstall_and_external_edit`（沙箱→报告→授权→安装→幂等→重装→外部修改拒绝）、`production_available_real_samples_admitted_and_converted` 在 Windows 与 WSL2 全通过（Windows 957s / WSL 1040s，含 3 份真实样本）。
- **真实样本产物摘要**（双平台一致且与 P2 冻结基准 3/3 全等）：`.artifacts/p3-production/windows-real-sf.json`、`.artifacts/p3-production/linux-real-sf.json`——2303800 `e7e232c1…`（1,373,855,199）、3606360 `6d8c5d92…`（1,296,894,639）、3721970 `24c2fbca…`（914,931,193）。
- **隔离对照**（`.artifacts/p3-production/{windows,linux,linux-root}-controls.json`，P3 早期会话产出）：17/18/17 项全 pass（stage 权限失败、canary、子进程拒绝、只读输入、禁网、env 清理、fd 泄漏、取消、超输出、父退出回收、janitor 保留/回收、超时回收）。
- **失败定位与裁定**：debug worker 无法在 300s CPU 限额内完成 1.2 GiB 样本转换（Windows 311s 后 `0xC0000142`、WSL `exit=137`）；release worker 59s 成功、摘要一致。P3 验收命令固定 release worker（`SF_WORKER_TEST_BIN`；`core-linux-check.sh`/`run-linux-check.sh` 已改为构建并使用 release）。

### 未执行（保持待验证）

- Windows 标准 Win11 零售机 LPAC/版本矩阵（P4 发布条件）；SteamOS Deck 实机部署与插件集成（P5）；三条旧 profile 真实包回归（样本不在仓库，沿用 P2 待验记录）。
- P0 登记的 Deck root 形态（Decky Loader 降权链路）与 cgroup v2/seccomp 生产配置在实机上的最终验证（P5）。

### 待办

1. P4：Windows 页面动态安装接线（传 `selection_id`，不再以投影 `release_id` 作为动态路由句柄）与动态 ZIP/EXE 实机路径。
2. P5：Decky 宿主降权、任务收尾、新 schema 初始化与部署验证。
3. 提交：按用户明确要求另行执行（代码/文档分提交）。

## 2026-10-09 P4 Windows 接入实施、复核与修复

### 用户目标与本轮范围

用户要求推进 P4（[design.md](design.md) §8）：Windows 动态列表/详情、一键安装/重装、历史与便携包；移除旧核心来源降级。本轮完成宿主 protocol v3 接线、前端动态安装打通、core 收口（R3/R6/H2）、契约 README 与 Windows runner，并完成独立复核（初审→修复→定向复审通过）。未执行：2303800 真实下载/安装实机验收、便携包本地构建（本轮进行中）。标准 Win11 零售机矩阵经 2026-10-09 核验由本机（Win11 26H2 正式版）证据覆盖。未创建 Git 提交。

### 实际实现

- **Windows Tauri 宿主**（`apps/windows/src-tauri/src/main.rs`）：协议 2→3（信封/校验/任务）；`CallArguments`/`start_install` 增 `selection_id`（与 `release_id` 互斥、`archive` 仅签名路由）；`install_progress` 平铺解析 + `(request_id, sequence)` 去重（终态后迟到事件忽略、第二个终态行 `invalid_core_response`）；`renew` 3s 转发（安装与来源任务）与 `cancel`/EOF；删除 `MigrateState`/`GetSourcePolicy`/`ResolveSourceRecord` 命令与启动迁移；能力命令更名 `get_capabilities`；错误 `detail` 截断 ≤2KiB。测试 9→20。
- **前端与共享 DTO**（`packages/ui/src/index.ts`、`apps/windows/src/main.tsx`、`sources.ts`）：`Patch.selection_id`、`Transaction.selection_id/content_id/has_replacements`、`Preview.selection_id`；新增 `patchInstallable`（N1：`installer.links.*.available`，不用 `formats`）与 `splitError`；动态路由传 `selectionId`（不以投影 `release_id` 作句柄）；删除 `ensureSourcesSupported` 降级与"（未支持）"文案；`Capabilities` 改 §3.1 形状；9 阶段 + substage 文案。
- **core 收口**：R6 `resolve_exe` 对 demo/unknown 变体返回 `exe_candidate_variant_unsupported`（投影仍生成、blocker 呈现）；R3 `exe_report` 对 `script_inventory`/`certificate`/`diagnostics` 嵌套键严格校验；H2 `Transaction` 增 `selection_id`/`content_id` 并经 `prepare_with_binding_at` 随计划持久化（动态 ZIP/EXE；签名 `None`），修复动态 EXE 重装判据。
- **契约/工具**：`packages/contracts/README.md` 升 v3；新增 `scripts/run-windows-check.ps1`（release worker + 真实样本 ignored 测试）；`p1-host-contract.md` §2.4/§2.8 统一终态后迟到事件语义。

### 验证方式与结果

- 回归（最终树）：core lib **255 passed / 4 ignored**、cli **37**、集成 8/40/4/6/3、windows 宿主 **20**；clippy `-D warnings`、rustfmt、前端 `tsc` + `vite build` 通过。
- Windows runner（release worker）：ignored 生产测试 3/3 通过（2303800/3606360/3721970，905.76s），产物摘要与 P2 冻结基准一致；证据 `.artifacts/p4-windows/{windows-check.log,windows-real-sf.json}`。
- 独立复核：初审**有条件放行**（H1 来源任务未续租、H2 事务归属缺失、M3 待证据）；修复后定向复审**通过**。归档 [p4-gate-review.md](p4-gate-review.md)，实施记录 [p4-windows.md](p4-windows.md)。
- 证据溯源：runner 日志（core 251 pass）早于 H2 的 4 个绑定测试（当前 255 pass）；因 H2 不改 `exe_report` 校验路径且 ignored 测试不建事务，M3 结论不受影响；复审已独立验证最终树全绿。

### 实机验收（2026-10-09）

- **2303800 真安装/生效（通过）**：GUI 动态列表/详情 → kagura-cn 中文候选（多镜像确定性择一修复后）→ 真实下载 1,228,278,216 字节（下载包 SHA-256 `7336c5c9…` 与官方样本一致）→ 隔离分析（LPAC worker）→ 转换 → 写入 `data.xp3`（1,373,855,065）→ 用户游戏内确认生效。
- **同内容重装（通过）**：事务 `superseded` + 新 `committed`，`selection_id`/`content_id` 落账；**取消（通过）**：计划前取消返回 `cancelled`、无新事务、游戏文件未动；**进度/历史/清理（通过）**：下载/分析进度展示、事务历史、`tasks/` 回收为 0、报告落 `reports/<content_id>/`、无 stage/backup 残留。
- **验收发现与处置**：① kagura-cn 多镜像被旧歧义阻断误伤 → oracle 裁决「确定性择一」（ZIP 契约修订 + EXE 实现修复 + 测试更新，见 [p4-gate-review.md](p4-gate-review.md) §8）；② 本机 TUN/fake-IP 代理与公共地址校验冲突（`download_private_address`）→ 临时关闭 TUN 后通过，代理环境支持登记待决策；③ 取消文案 `reason("cancelled")` 缺口 → 已修复。
- **残余（登记）**：动态 ZIP 实机路径（无匹配目标时待样本）；GUI 断网重开历史/离线项；更早 Win11 版本矩阵。
- R2 附加条件已随 kagura-cn 中文安装满足。
- **便携包（通过）**：最终 zip SHA-256 `c4a159cab477dec9f965875837725e3fcbaf8341d4a7307edd78001ce1238155`（8,496,461 字节，含 `sf-worker.exe`）；三件套哈希与 `target/release` 一致；便携 GUI 启动正常、`get_capabilities` smoke `installer.links.exe.available=true`。

### 未执行（保持待验证）

- 动态 ZIP 实机路径（待样本）；GUI 断网重开的历史/离线核对。
- Windows 版本矩阵：更早 Win11 版本（23H2/24H2 等）未覆盖，登记为残余（本机 26H2 正式版已覆盖）。

### 待办

1. P4 收尾：最终便携包重建与便携运行验证已完成（zip SHA-256 `c4a159cab477dec9f965875837725e3fcbaf8341d4a7307edd78001ce1238155`；便携 GUI 启动与 `get_capabilities` smoke 通过；宿主 `18DFDEB4…` / CLI `2E942D02…` / worker `F4B27AD2…`）。
2. P5：Decky 宿主降权、任务收尾、新 schema 初始化与部署验证。
3. 登记项：S1（`managed_zip` 未校验 `links.zip.available` → P5）、S2/S3（前端 hgamer 硬编码、runner 变量名 → P6）、S4（`loadCapabilities` 缓存，可接受）、S5（更早 Win11 版本矩阵）、**S6（待决策：TUN/fake-IP 代理环境支持）**、S7（取消文案已补）。
4. 提交：按用户明确要求另行执行（代码/文档分提交）。

## 2026-10-09 P4 后续：用户报告问题的诊断与修复（patchmoe 动态 ZIP / cmd 弹窗）

### 用户报告
1. 「你妈真漂亮」（AppID 2743810）无法自动安装（旧 Decky 可装；patch.moe ZIP `Kudos_Kuppa_patch.zip`），"重新查找"有 warning。
2. Windows GUI 每次查询弹出 cmd 窗口。

### 诊断（只读核对 + 现场抓取归档）
- **patchmoe 索引解析 bug**：`parse_rows(table_id=None)` 只解析文档第一个 `<table>`；patch.moe 页面含 `tbl new`（3 条"最近更新"）+ `tbl regular`（111 条完整列表），产品仅入库 3 条（临时 state 复现 `entry_count=3`）。旧 Python `_Page` 收集所有顶层 table，属移植退化（href 尾随换行已排除——`row_entry` 已 trim）。
- **安装接线缺口**：patchmoe（非 `discovery_only` 索引来源）的 `matches`/`SourceMatch` 在 Rust 无安装消费者（旧 `publisher.py` 正是消费它）；动态 ZIP 候选仅来自 discovery 候选。
- **语言门误伤**：候选语言 zh-CN 与游戏 acf `UserConfig/MountedConfig.language=english` 不符 → `exe_language_mismatch` 必然阻断（旧 Python publisher 路径无游戏语言门，属新引擎能力回退）。
- **宿主 refresh 通道 bug**：GUI「刷新来源索引」经一次性 `serve` 调用 `refresh_sources`（协议服务面为 serve-task 方法）被核心拒绝（`method requires the serve-task control channel`）。
- **cmd 弹窗**：宿主 spawn sidecar 未设 `CREATE_NO_WINDOW`（GUI 子系统进程 spawn 控制台程序弹窗）。
- kagura 两条诊断（`discovery_pagination_cycle`、`discovery_index_incomplete`）与 patchmoe 无关，单独登记（见文末）。

### 独立裁决
- **patchmoe 缺口（ora-4）**：方案 B2——把已校验索引条目经确定性 `SourceMatch → SourceCandidate` 投影并入 `dynamic_zip_candidates`（复用 `zip-selection-*`/resolve/orchestrator/授权/配方全链路），不改 `discovery_only` 语义；先修全表解析。契约修订：p0-contracts §2.5、p1-zip-orchestration §2.1(4)/§5.1、design §7.3。
- **语言门（ora-5）**：方案 d——`FactsExpectation.require_language`（serde 默认 true）；发布商来源（`rule.discovery_only`）保留语言门，索引来源不设门；语言仍进 `recipe_identity`（身份不随现场语言漂移）。契约修订：p1-zip-orchestration §2.1(8)/§2.2/§2.3/§2.5/§4.4/§5.1。

### 修复实现
- **解析（fix-12）**：`parse_rows` 改为收集所有顶层 table（嵌套表按节点去重、`table_id` 行为不变）；新增合成/裁剪 fixture 与 4 项测试；临时 state 实机刷新复验 **3 → 160 条**（含 `Kudos_Kuppa_patch.zip`、zh-CN、title 你妈真漂亮）。
- **接线（fix-13）**：`registry::{index_candidates, dynamic_zip_candidates}`（确定性映射：canonical candidate_id、`mirrors=[zip]`、`version_identity="page-unknown"`、`COMING SOON` 过滤、cache-first 去重）；`resolve_zip` 与 `cli::dynamic_patches` 候选源切换；测试（映射确定性、patchmoe resolve、scan 投影）。
- **语言门（fix-14）**：`require_language` 开关与两处 failure 收口（evidence 始终记录）；ZIP expectation `= rule.discovery_only`；EXE/签名显式 true；测试含 `index_source_language_is_not_a_gate`（patchmoe english manifest → committed；kagura → 仍 mismatch）。
- **宿主 refresh 通道（fix-15）**：`refresh_sources` 改 serve-task 模式（复用 `SourceScans` 槽、renew 生命周期、`read_refresh_stream` 去重 reader）；23 项宿主测试。
- **cmd 弹窗（fix-11）**：`core_command` + `CREATE_NO_WINDOW`（3 处 spawn 统一）。

### 实机复验（2026-10-09，GUI 便携版）
- 索引：用户 state `refresh_sources(force)` → patchmoe **160 条**；Kudo 条目（`你妈真漂亮`/`zh-CN`/2026-06-23/`https://download.patch.moe/Kudos_Kuppa_patch.zip`）。
- 投影：重新扫描后 2743810 显示「有可用补丁」；详情 `ZIP · 默认解压至游戏根目录`、`zip-selection-560d6cae838993b26db4caa6-2743810`、「安装补丁」可用（语言门不再阻断）。
- 安装：真实下载 83,964,941 字节 → 解压写入游戏根 **`patch.xp3`（840,228,417）**；原 `data.xp3`/`thekuppa.exe` 未动；事务 `committed | release_id=patchmoe-2743810 | selection_id=zip-selection-560d6cae… | content_id=None`。
- 重装：`superseded` + 新 `committed`（「重新安装完成」）。
- cmd 弹窗：serve-task 进程 `MainWindowHandle=0`、无控制台窗口。
- 证据截图：`.artifacts/p4-windows/gui-2743810-detail.png`、`gui-2743810-install-done2.png`、`gui-2743810-reinstall-done.png` 等。

### 未执行/登记
- 游戏内确认（用户）；便携包最终重建（含 fix-15 宿主，进行中）。
- **kagura 来源诊断登记**（与 patchmoe 无关，单独排查）：`discovery_pagination_cycle`（kagura-en「来源页的跳转链接重复，查询已停止」）与 `discovery_index_incomplete`（kagura-cn「来源目录尚未查完」）可能降低 kagura 候选覆盖；归入来源质量专项（随 P5/P6 评估）。

## 2026-10-09 P4 收尾验收与 Windows 游戏列表状态修复

用户要求继续完成验收，并指出 Windows 游戏列表不应突出“已找到来源”，应突出“有可用补丁、已安装补丁、未找到补丁”。本轮实际修复、检查、证据边界与待验详见 [p4-acceptance.md](p4-acceptance.md)。

- 完成游戏列表主补丁标签；真实 EXE 检测补上同实例/选择绑定与实际文件摘要；恢复结果区分成功/无待办与 needs_recovery。
- 最终后续修复基线回归：core lib 272 pass/5 ignored、CLI 38、Windows 23、集成 8/40/4/6/3；新增检测回归后 CLI 39 pass；主状态 10/10、clippy/TS/build 通过。
- 实际 RPC：独立合成库中 committed 历史跨进程一致；中断恢复 rolled_back/重复空；外部修改 needs_recovery/改动与原始备份保留；存档 sentinel 全保持。真实游戏仅只读扫描，两款均 installed_managed。
- 用户明确确认 2743810 游戏内生效；关闭其游戏内确认待办。
- 最终便携包重建/解压/三件套哈希/从包内 get_capabilities 全通过：8,513,897 字节，SHA-256 `e7204378b006ae8723dcd6e69949d7d36ad8e7878a5b19ca4c0468080d68f0f4`。前节“重建进行中”由此关闭。
- 浏览器验证生产前端列表与恢复提示通过，桥接为真实 RPC 快照模拟，单列不计原生实机。
- Windows GUI 控制运行时两次初始化超时；原生 GUI 断网重开/事务历史/恢复按钮结果已请用户协助，仍待确认。未关闭整机网络；合成 RPC 无目录/来源依赖不能替代物理断网验收。
- P4 当前摘要、设计阶段描述和功能索引已按实际证据同步；历史施工事实保留。未操作 Deck、未再次写入真实游戏、未提交 Git。

## 2026-10-10 P4 验收范围调整与结项

- 用户明确“不需要测断网”，取消此前原生 GUI 断网重开及相关人工核对要求；不再等待此前人工验收请求。
- P4 按调整后的验收范围完成：沿用两款真安装/重装/取消与游戏内确认、最终自动化、隔离恢复/冲突与存档保护、历史 RPC、生产前端浏览器展示和最新便携包校验记录。未执行的断网项标为用户取消，不写成通过。
- 同步当前状态、设计 P4 门槛、验收报告和功能索引；保留 2026-10-09 的历史待验与工具超时事实。更早 Windows 版本、TUN/fake-IP 决策及 P5/P6 登记项不变。
- 本轮仅修改文档，未重跑已通过的代码/实机测试，未重打便携包，未创建 Git 提交。

## 2026-10-10 P4 Git 归档

- 用户明确要求先提交 P4；代码与测试已提交为 `e1fc1d3`（`feat(windows): 接入共享安装器并完成 P4 验收修复`），设计、契约与验收文档另作独立提交。
- 提交前完整审查暂存差异并通过 `git diff --cached --check`；沿用本 session 已完成的自动化与实机验收记录，未重复运行测试。
- 提交范围仅含 P4 代码、测试、配套脚本及文档；`.artifacts/`、安装包、截图与真实游戏文件不纳入 Git。仅创建本地提交，未推送、未启动 P5。

## 2026-10-10 P5 开始施工

- 用户要求开始 P5。P4 已结项，沿用此前验证，不重复运行未修改范围。
- 当前工作：Decky 协议 v3 薄桥接、一站式共享 Rust 安装与能力门控、租约/取消/终态、非 root 身份与宿主状态、新生产包固定 Rust worker；切换前旧事务检查由部署工具负责，不加入生产旧格式兼容读取。
- 详细边界与验收：[p5-decky.md](p5-decky.md)。本节为开始记录，不表示实现或验收通过。
- 本地沙箱进程无法返回输出；只读/构建操作使用经自动审批的终端。Devkit 当前目标与新 SSH 指纹已由用户确认，后续按项目部署技能检查任务、备份、哈希、运行时。

## 2026-10-10 P5 实现、正式部署与实装验收

- 已完成 Decky 协议 v3 薄宿主与 selection_id/task/租约/取消/终态桥接、当前来源直接初始化、批量扫描投影、非 root 与固定新状态目录。生产包固定 CLI/worker、六模块白名单、许可/SBOM，旧 Python 安装消费者和切换模块退出部署。
- 切换前旧任务/事务终态与磁盘一致性通过，停用旧插件后发布三旧 committed AppID 的写保护；新运行时不读旧日志，不把未知改动当作原版。用户确认新 SSH 指纹，正式 Loader 安装、21 文件哈希、HTTP 新 bundle、实例 1/服务 active、真实 v3 能力与 scan/RPC、设置/56旧文件保留检查通过。
- 自动化：Python 159（5平台跳过）与 Linux 所有权4；Decky最终63；两端TS/build；Linux core274+集成64（5忽略）、最终CLI41；SBOM1/打包6/切换6；最终clippy通过。独立复核问题闭合，见 [p5-gate-review.md](p5-gate-review.md)。
- 实机生产沙箱18项通过；安装进程UID/GID1000。QAM/全屏真实页面与截图核对通过，实体手柄仍待验，不能用DevTools点击/RPC取消替代。
- 用户进一步授权两款游戏实装，完整游戏首次备份已取回并验哈希。真实ZIP发现首帧空阶段协议问题，已补真实遥测回归并正式重新部署；原失败现场保留。下载取消终态通过，游戏7182原文件0变化。
- ZIP继续完整实装并在 writing/1文件/4823213 bytes 请求取消，核心进入 rolling_back 后真实终态 rolled_back；全文件/额外残留复核通过（7182 原文件0变化/额外0），recover 返回空；完整重装随后 committed，50 文件摘要全匹配，复扫托管已安装。
- Summer Memories DLC CN 2.02.1 EXE下载/沙箱分析到达，安全拒绝未支持的CSetupFileData尾字段（5874记录全1），与旧人工profile样本一致；通用Rust与Python均不支持此值，保持拒绝，未写游戏，不把旧profile授权或摘要当作新安装许可。
- 分项实施、哈希、备份、RPC、截图与明确待验见 [p5-decky.md](p5-decky.md)。P5未结项，P6仍待执行；未创建Git提交。

## 2026-10-10 P5 EXE 真安装与最终收尾

- 用户扩展授权《妃神会秘史》简中 v4 EXE 实装，完整首次备份取回/验 SHA 后通过页面启动。真实沙箱分析、提取、内容授权、写入与 committed 通过；1,373,855,065 bytes 载荷摘要匹配，11 原文件仅计划内 1 文件变化，无计划外载荷，固定空锁文件单列。
- ZIP/EXE 最终均 installed_managed，recover=[]，自动清理后事务仍可读。完整成功安装与写入取消回滚均有独立实机证据。Summer DLC 格式仍安全拒绝，不以其他 EXE 通过掩盖其支持边界。
- 补充修复跨游戏终态反馈：成功/失败/取消/恢复与异步晚到提示绑定原实例；新增3回归，最终前端63项、TS与build通过。最终包正式更新、21文件/HTTP bundle与状态保留复核通过。
- QAM/全屏底部、记录、缓存和订阅源页面真实截图/滚动核对通过，重开不复播历史弹窗。首次备份与16远端证据文件已取回并验哈希，确认无活动任务后清理本轮远端暂存及本轮SSH隧道。
- 仍待实体手柄与两款游戏运行/内容生效人工确认，已发起核对；P5 保持待人工验收，P6 未启动，未创建Git提交。详细记录见 [p5-decky.md](p5-decky.md)。

## 2026-10-10 P5 用户实机确认与结项

- 用户明确回复“实体手柄与两款游戏均通过”，确认本插件 QAM/全屏页面焦点、A/B 返回，以及《夜勤病栋》《妃神会秘史》能运行且补丁生效；关闭前节人工待验项。
- P5 按本轮范围验收完成：沿用已完成的代码回归、独立复核、正式部署与哈希、生产沙箱、ZIP/EXE 真安装、写入取消回滚、文件/状态保留与页面验收证据。用户确认不代替 Summer DLC 未支持格式或缓存复用验证，这两项事实仍保留。
- 同步当前摘要、设计状态、功能索引与 [P5 记录](p5-decky.md)，保留历史施工和待验事实。仅更新文档，未重跑测试、部署或安装；完整备份继续保留，P6 未启动，未创建 Git 提交。

## 2026-10-10 P5 Git 归档

- 用户明确要求提交 GIT；P5 代码、测试与构建/打包/切换工具已本地提交为 `1a81469`（`feat(decky): 接入共享 Rust 安装器并完成 P5 验收修复`），共 49 文件。设计、验收记录与 README 另作独立文档提交。
- 提交前审查暂存范围及差异，通过 `git diff --cached --check`；沿用本 session 已完成的自动化、正式部署、真安装/回滚、文件摘要和用户实体手柄/两款游戏内确认，未重复测试或部署。
- `.artifacts/` 中完整备份、安装包、截图、日志与真实库材料不纳入 Git，继续本地保留。仅创建本地提交，不推送、不改写历史；P6 未启动。

## 2026-10-10 Decky 运行问题调研：覆盖安装与 SACRIFICE VILLAINS ZIP（修复待实现）

设计：[design.md](design.md) 第 10 节；修复方案：[fix-plan.md](fix-plan.md)。本轮用户最初要求只调研和写方案，随后明确授权在自己的 Steam Deck 测试 SACRIFICE VILLAINS 安装。未修改功能代码、构建、部署或创建 Git 提交。

### 已完成调研

- 源码追踪切换工具 → `preexisting-installations.json` → CLI 扫描/安装门 → 共享 UI。确认 P5 新增保护按实例/AppID 阻断，没有实际载荷核验或用户解除手段；当前正式保护包含 1788100/2743810（以及 1189440），本轮实际扫描两款均命中。
- 正式 RPC 原失败记录显示 3137110 完整下载 6,780,570,933 bytes 后 `analysis_failed / zip_expansion_limit`。只读缓存目录检查：11 条目、9 文件，总展开 6,885,849,408；最大 `update1.ypf` 为 6,073,754,660 bytes，确定超过单文件 4 GiB，其余包/总展开/数量/压缩比均未超现有限额。
- 旧 Python 与 Rust 均含相同大小上限和 XP3 包装规则；不能把 ZIP 上限和 YPF 映射缺口归为迁移新引入。当前游戏无 data.xp3，包中唯一外层为 `sacrifice_villains_fullpatch/pac/`；只放宽上限会保留错误外层。真实 YPF/ASF 首部与游戏 pac 布局支持去包装映射，尚未做修复后生效验证。
- 用户进一步指出固定大小/数量限制过多；方案改为实际磁盘峰值、流式处理及有依据的元数据资源预算，不以再设较高单文件常量作为最终解决方式。修改面覆盖 ZIP 获取、配方、事务、恢复与安装后检测，SF EXE 格式能力独立。

### 本轮实机失败复现与数据保护

- 当前 Devkit 与可信 SSH 记录匹配；正式插件实例 1、Loader active、UID/GID 1000、无活动任务。正式 CLI SHA `cf14ba30b9411b3175a08914de3cd470e2f582f3aefff62bad59c0ee1f6b95c4`。
- 游戏先完整备份并取回验哈希：19 文件、183,185,706 bytes；本地 `game-baseline.tar` SHA `ebfa718afe2d30f55ce1fbeac7fe45f5ad74c52662fa05ff12565c56628de541`。原始证据与备份保留于忽略目录 `.artifacts/decky-install-fix-20261010/`。
- 首次 RPC 测试遗漏前台续租，终态 `download_cancelled`；登记测试控制错误，不归为用户原问题。补足每秒 task_status 续租后正式重试，任务 `706ccd07-9625-4109-bc88-e98bf57214af` 终态 `failed / analysis_failed / zip_expansion_limit`，复用旧完整缓存走正常获取/重核，不是新增全量下载。
- 前后全量 SHA 复核通过：19 文件全部未变、新增文件 0、3137110 事务 0、无错误包装目录；复现结束无活动任务。未安装新载荷，不标记为补丁成功或生效。
- 本地备份取回并验哈希后，仅删除本轮远端验证目录的两份备份材料，关闭本轮 18081 SSH 隧道；本地完整备份和 RPC/缓存结构证据保留，未触碰其他用户会话。

### 产出与待办

- 创建专项 fix 文档，同步设计最新目标及功能索引；保留此前 P5 历史施工/验收结论。
- **待实现**：F1 一键覆盖及操作契约、F2 ZIP 资源策略、F3 YPF/pac 映射、关联错误/缓存进度；**待验证**：新增自动化与数量/内存压测、正式部署、两款旧保护游戏及 3137110 修复后实装/覆盖/取消回滚、游戏内生效、共享 Windows 回归。（2026-10-11 晚：以上全部完成并通过验收，见本文件“续”与“续二”节。）
- 本轮只做调研与当前正式版失败复现，未重跑无改动的既有测试，未修改/删除保护文件、数据库、缓存包或游戏文件。


## 2026-10-11 覆盖安装与 ZIP 运行问题开始修复

用户要求开始修复，按 [fix-plan.md](fix-plan.md) 推进。当前代码已完成主要实现并通过首轮自动化；Decky 正式构建/部署与真实游戏安装仍在进行，不能视为功能结项。

- **F1 覆盖安装（实现完成，集成验证中）**：CLI、Decky/Windows UI、请求操作与事务新增 `overwrite`；正常扫描与安装不再读取旧 marker，切换工具不再发布阻断名单。终态旧记录可被新覆盖原子标为 superseded；中断事务仍阻止冲突操作。EXE 覆盖按现场条件分析，只有历史 `reinstall` 使用旧收据证据。
- **F2 ZIP 资源策略（实现完成，集成验证中）**：动态/签名 ZIP 下载和缓存哈希按实际长度流式处理，已知长度按下载卷检查空间；EXE 的 2 GiB worker 边界保留。普通 ZIP 去掉 4/8/16 GiB、8192 entries 与 200 倍压缩比拒绝；事务 stage/hash/备份/恢复与安装后指纹同步使用实际长度，按状态卷/游戏卷的峰值空间预算预检。64 MiB 中央目录/计划元数据预算与路径、CRC、字节数、取消检查保留。
- **F3 YPF 映射（实现完成，合成测试通过）**：按唯一包装层和游戏现有 `pac/` 布局去掉一层外壳，保留 `pac/`，支持载荷中经验证的 YPF 与 ASF 文件；冲突、路径安全及 XP3 旧规则继续检查。
- **安全与规模审查**：ZIP 6 会在候选 ZIP 目录解析失败时回退到较早 EOCD。预检现扫描并预算解析器可选择的候选目录，新增 65,000 条早期目录回退回归；完整 6.78 GB ZIP 的预检额外扫描成本仍待实测。普通小文件压测：10,000 文件 recipe 21.4 秒、JSON 1.64 MB、取消响应 92 ms；50,000 文件 recipe 106.0 秒、JSON 8.28 MB、取消响应 453 ms。采样工作集 46.8 MB，仅为过程采样值，非已确认峰值。
- **逐文件日志**：新事务用独立 SQLite 增量表持久化每个文件/目录的恢复意图，写入前 FULL 同步；旧 journal 默认沿用原内嵌进度格式。整份事务保存与增量清理同一 SQLite savepoint，读取基文档和增量使用单条快照查询。用户明确授权接入并验证；孤立 50,000 文件 JSON/SQLite 原型的原子回滚、读取快照、历史文档兼容通过。完整并发/崩溃边界回归正在补齐。
- **已知自动化**：46 项核心安装事务集成测试已通过；下载、目录检测、元数据/recipe、Decky/切换脚本与 TS 检查由对应工作流报告通过。新增日志 overlay 中断写入/回滚、损坏索引拒绝、旧 journal 无 overlay 表读取及模拟提交失败后恢复证据保留等用例包含在上述46项内，全部通过。
- **本轮限制**：未执行真实 6 GB 文件提取；真实 Deck RPC 复验、插件部署、三款目标游戏实装/覆盖/取消回滚、文件摘要/游戏内生效与 Windows 安装器实装均待验证。设备基线和完整 3137110 游戏备份仍保存在 `.artifacts/decky-install-fix-20261010/`；本轮前后尚未写入任何游戏内容。（2026-10-11 晚：上述项均已执行完毕，见“续”与“续二”节；6.78 GB 真实校验/提取随 3137110 安装完成。）
- 用户明确禁止 Astra；实现与只读复核使用 Sol 路线。此前文档批量编辑曾被自动审批拒绝，已按反馈拆分处理；后续日志改动取得用户明确授权。无 Git 提交。

## 2026-10-11 续：双平台全量回归、独立复核与收口修复

### 双平台全量回归（本轮实测）

- Windows：`cargo test -p steam-patch-core -p steam-patch-cli` → core lib 287、catalog 8、install 集成 46、localized 4、steam_scan 6、trust 3、CLI 41，0 失败；宿主 `cargo test -p steam-patch-windows` 23，0 失败；`npm run typecheck` 与两端前端构建通过。
- WSL2 Linux：同一套件 → core lib 288、catalog 9、install 集成 48（含 2 个 `#[cfg(unix)]`）、localized 4、steam_scan 6、trust 3、CLI 41，0 失败；release 构建（`steam-patch` + `sf-worker`）成功。
- Decky：Python `unittest discover` 159（5 平台跳过）全绿；TS 63 全绿（发现并同步 `task-progress-view.test.ts` 的 `validating` 文案期望为“检查已下载补丁”——Decky 与 Windows 实现早已为新文案，符合 fix-plan §6）；`scripts/test_decky_cutover.py` 5、`test_package_artifacts.py`、`test_build_sbom.py` 通过。
- 前节“46 项”为当时快照；`tests/install.rs` 完整集合为 **48 项**（Windows 运行 46，另 2 项 `#[cfg(unix)]` 仅 Linux 运行），本轮两端均已全量运行并 0 失败。

### 独立复核与收口

- [fix-gate-review.md](fix-gate-review.md)：独立 oracle 只读复核（对照 zip 6 源码验证 EOCD 回退 fail-closed）结论**有条件放行、无阻断**；I1/I2 文档矛盾按本轮修订，I3（6.78 GB 预检开销与动态路径潜在双次预检）保留为部署后实测，A1/A3/A4 登记为可接受权衡。
- **复核收口修复（已完成）**：`validate_metadata_budget` 改为逐条 `state.len()+32` 与 load 读取路径同口径预留，避免预检低估导致写入成功后才在恢复阶段拒绝；u64 溢出错误码改报 `zip_size_overflow`（原 `zip_expansion_limit`）；`map_recipe_error` 移除不再产生的 `archive_entry_limit/archive_too_large` 直通；删除无生产消费者的 `installer::preexisting` 模块与声明（CLI“marker 存在/损坏不阻断”回归保留）；Windows 完成文案按按钮语义区分“覆盖安装补丁/安装补丁”；另修复工作区引入的 `zip_recipe.rs:418` clippy `useless_conversion`（原会使 `-D warnings` 门禁失败）。
- **收口后复跑**：core lib 286（-1 为删除的 preexisting 单测）、install 集成 46、CLI 41、宿主 23、typecheck、Windows 前端构建、clippy `-D warnings`、rustfmt 全通过；Linux 复跑与 release 重建见下节。
- 复核跟进项：Task 1 未新增“近预算上限”集成用例（fixture 构造成本高，按复核许可改为注释说明）；如需强制覆盖另行安排。

### 正式构建与部署

- 发布产物冻结：Linux release `steam-patch` SHA-256 `a49299b8d8e57ee21e1140f29ed2fe00770a179edbfb112b96f14942d678f911`（13,399,752 bytes）；`sf-worker` 与正式版一致 `570300b3…`（F1–F3 未改动 worker）；Decky 包 `steam-patch-manager-decky.zip` 6,852,177 bytes `5545e84b…`（收口修复后按版本重建，见下）。
- 设备基线（只读核对）：Devkit 配对设备与上轮用户确认指纹一致（`SHA256:1eQWToCx…`）；无活动任务；三款目标游戏现场确认；3137110 安装前 19 文件 / 183,185,706 bytes 基线已存档；6.78 GB 缓存 ZIP 完好；旧插件完整备份已取回校验（`ee8ddd1d…`）。
- 详细部署与实机结果见下节（进行中）。

## 2026-10-11 续二：收口修复后重新构建与正式部署

### 构建与包（已完成）

- 收口修复后 Linux 复跑：core lib 287、install 集成 48、CLI 41，0 失败；release 重建 `steam-patch` SHA-256 `7904876e6569a761441cfd3cd98a3f6b35978474abe4dbe121929e80f027e2c4`（13,403,072 bytes），`sf-worker` 不变 `570300b3…`。
- 全量包 `steam-patch-manager-decky.zip` 6,855,032 bytes，SHA-256 `cfbf65a41b8ef388267ddb36d01b416dd67a94763c7483e1140c80874657553d`；plugin-output 清单含新 CLI 与 `bin/runtime.json` 更新哈希。

### 正式部署（已完成）

- 设备与指纹：Devkit 配对设备 `deck@192.168.31.186`；本轮 known_hosts 公钥与上轮用户确认指纹一致（`SHA256:1eQWToCx…`）；部署前 `tasks.json` 无活动任务、Loader `plugin_loader` active。
- 旧版备份：`plugin-backup-20261011-0110.tar.gz`（6,837,031 bytes，SHA-256 `ee8ddd1d…`）已取回本地校验。
- 上传：`/home/deck/steam-patch-validation/installer-fix-20261011-0110/steam-patch-manager-decky.zip`，远端 SHA-256 与本地一致。
- Loader v3.2.10 本地 ZIP 安装：`utilities/install_plugin`（versionLabel `dev`、installType 2）→ prompt 校验 name/version/hash/type 一致 → `confirm_plugin_install`（requestId `1791652502.556158`）。
- 安装后核对：正式目录 21 文件 SHA-256 与清单逐项一致（CLI `7904876e…`、worker `570300b3…`、`dist/index.js` `08b040d0…`、SBOM `6415c1b6…`）；CLI/worker 具备执行位；`plugin_loader` active，插件实例 1、`hasPlugin=true`；后端进程于 01:15 重启为新版本；HTTP bundle 实测字节 SHA-256 `08b040d0…` 与构建一致。
- 安装完成时 Loader 确认对话框（modal）因 API 侧确认未随 UI 关闭；参照 P5 方式经 React fiber 调用 `closeModal()` 安全关闭（不触发 `cancel_plugin_install`，避免已消费 requestId 的 KeyError），截图确认页面恢复。

### 实机验证（已完成，用户游戏内生效已确认）

- `scan(false)`：28 个实例；3137110 候选 `zip-selection-31d6b3f6b3e978c50b500c59-3137110` `available=true`、blockers 空；2743810/1788100 的旧保护 blocker 消失、候选可直接安装。
- **3137110 安装（通过）**：从全屏页面"安装补丁"按钮点击发起（任务 `802ee199-b9d3-485a-8ff7-e925606ff243`）；`validating` 阶段 UI 显示"检查已下载补丁"（6.78 GB 缓存重哈希实测约 70 秒），随后 planning → applying → committed（事务 `75b03b41-4d98-4be4-8e8d-e7ace3f9aa0b`），全程约 2 分 40 秒；成功后按 `auto_cleanup` 自动清理下载缓存（6,780,571,275 bytes）。I3 复核登记的中央目录预检开销实测未形成可感知延迟（全流程含两次预检仅约 160 秒），不再优化。
- **3137110 文件核对（通过）**：安装后全量 28 文件快照对比基线（19 文件/183,185,706 bytes）——9 个目标全部正确（`pac/bn.ypf` 替换为 `32a2ab67…`、`pac/update1.ypf` 6,073,754,660 bytes 及 `op.ypf`、`SV_NORMAL_ED_*`、`SV_TRUE_ED_*` 共 8 个新增，与包内 9 文件一一对应）；其余基线文件零变化；原 `SV_NOMAL_ED_en.ypf` 保留未动；无 `sacrifice_villains_fullpatch/` 外层目录；仅固定空锁文件 `.steam-patch-manager.lock` 单列。复扫 `installed_managed`，9 个目标逐一 `match`。
- **2743810 覆盖（通过）**：页面"安装补丁"按钮可直接发起、无核验阻断（任务 `9d1c9fb1-…`，下载 839,649,041 bytes）；committed（事务 `d3ac85a9-74e9-4797-b57f-70638925f3ad`）；覆盖后全量 diff 为空（服务器补丁与已装内容相同，字节一致），非目标文件与锁文件零变化。
- **1788100 覆盖（通过）**：同样直接发起（任务 `1170dc5e-…`，下载 813,370,375 bytes，写入 2 文件/2.1 GB）；committed（事务 `298451e0-76e9-468c-883e-413d82d301ee`）；全量 diff 为空，7 个存档文件保全。
- **写入阶段取消回滚（通过）**：Saimin 再次覆盖由页面发起（按钮实机文本已为"覆盖安装补丁"）；取消守护在 `applying/writing` 写 `pac/update1.ypf`（64 KiB）时调用 `cancel_task`（任务 `f0ab6e3c-e8f6-4d1f-8bfb-958193492d32` → `cancelling`）；终态 `cancelled`/`rolled_back`（事务 `f9aa66eb-…` `rolled_back`）；取消后全量 diff 为空（文件恢复至本次开始前）、存档完好；上一成功事务 `298451e0-…` 保持 committed。
- **复扫（通过）**：3137110/2743810/1788100 三款均 `installed_managed`、无 blockers。
- **清理**：旧插件备份 `ee8ddd1d…` 与部署包 `cfbf65a4…` 已取回本地校验；远端本轮暂存目录删除（历史轮次目录未动）；本轮 18080 隧道关闭；此前遗留的 18081 隧道一并清理。
- **用户确认（通过）**：SACRIFICE VILLAINS 启动后补丁正常生效（用户 2026-10-11 回复确认）；其余两款为同内容覆盖，与用户既有体验一致。

### Windows 共享事务回归（已完成）

- 收口修复后重建 Windows 前端与 release 宿主/CLI/worker；使用与 Windows 宿主相同的 state 目录（`%APPDATA%\io.github.steampatch.manager`）经 `steam-patch.exe serve`/`serve-task` 驱动共享事务；绑定同一判据：`scan` 确认 2743810 `installed_managed` 且候选 `zip-selection-560d6cae838993b26db4caa6-2743810` 可用。
- 首次重跑被本机 fake-IP 代理阻断（`download_failed/download_private_address`，域名解析为 198.18.0.0/15 保留段；S6 登记环境项，非本次修复缺陷，核心按设计直连并要求公共地址）；用户临时关闭 TUN 后通过。
- **覆盖安装（通过）**：事务 `91dc4cdf-8f42-44a3-be13-c2ef17d57b68` committed（阶段 connecting → validating(缓存重哈希) → planning → extracting → backing_up → writing → committing → cleanup，写入 `patch.xp3` 840,228,417 bytes）；**旧 committed `b361c6f5-e177-4e66-90a7-92458b2a79bb` 被原子标记 `superseded`**，历史 `78620697…` 保持 superseded——本轮修复的 superseded 提交语义在 Windows 实机路径确认。
- **文件核对（通过）**：与安装前 10 文件基线全量 diff 为空（同内容覆盖；`patch.xp3` 字节不变，`data.xp3`、`plugin/`、`thekuppa.exe`、锁文件零变化）。
