# 跨平台安装器迁移施工记录

设计：[design.md](design.md)。日期：2026-10-08（Asia/Shanghai）。状态：**P1 进行中：宿主契约已冻结（[p1-host-contract.md](p1-host-contract.md) v1.2，两轮独立评审通过）、来源确定性编码迁移已完成并通过评审（149+60 测试）；P1-2/P1-3 与两端宿主切换待执行**。

## 当前状态

| 工作 | 状态 | 说明 |
| --- | --- | --- |
| 前序范围/安装器差异盘点 | 已完成只读核对 | 来源 P0–P6 完成；动态 ZIP/EXE 准备与授权仍为 Decky Python，Windows 只展示候选 |
| 设计与功能归档 | 已完成 | 独立设计/施工、索引与关联入口；文档检查见下文 |
| 来源迁移兼容遗留盘点与清理规划 | 已完成只读检查与设计；代码待清理 | 六类遗留、消费者依赖、阶段门槛已纳入设计第 7.3 节；不是已清理或已验证 |
| P0 契约/样本/平台隔离 | 契约冻结定稿并通过 Gate 1；Windows 原型完成；Linux/Deck 对照与 root 降权补测通过 | 契约 v0.5（encoding/ID/schema/锁/worker/限额/依赖冻结，Gate 1 终审通过）；Windows LPAC+Job 原型 31 项探针 27 pass；Linux bwrap 原型 22 项在 WSL2 与 SteamOS Deck 均为 21 pass/0 fail/1 info；P0-D 修订后非 root 回归 23 项与 WSL root 降权补测 26 项均 0 fail；cgroup/seccomp 与 P1+ 待执行 |
| P1 共享 ZIP 服务 | 待实现 | 获取/缓存/配方/新授权与新 schema；来源 Rust 编码、当前规则契约和直接初始化 |
| P2 Rust SF 与 worker | 待实现 | SF9.5–9.7 完整结构/AST/IR/转换 |
| P3 隔离/授权/任务 | 待实现 | 双平台启动、facts/撤销/互斥/进度/恢复 |
| P4 Windows | 待实现/待实机验证 | 2303800 EXE、动态/签名 ZIP、恢复/重装；移除旧核心来源降级 |
| P5 Decky | 待实现/待实机验证 | 旧任务收尾、新 schema 初始化、删除安装来源适配/引擎检查点、薄桥接与正式包/RPC/QAM/全屏/手柄 |
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

- 探针（本机 Windows，非标准 build 26300）：31 项 = 27 pass / 0 fail / 1 未覆盖（CPU rate hard cap，宿主嵌套 Job 限制）/ 3 info；LPAC 身份、零 capability 无网络、越界读写/注册表拒绝、句柄白名单、进程树回收、内存与 JOB_TIME 限额、无控制台、中文长路径均通过。
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
- 测试缺口（P1-3b-3 门槛未满，补测进行中）：resolve 逐码矩阵（按可达语义）、facts 语言/DLC 动态矩阵、逐阶段取消/租约过期、recover `needs_recovery`（动态）、动态重装错误、CLI 端到端 scan/preview（注入 transport）。

### 待办

1. 已完成：P1-2 / P1-3a / P1-3b-1 / P1-3b-2 / P1-3b-3（本轮分批提交）。
2. 进行中：P1-3b-3 测试缺口补全（设计 §5 门槛）；随后 P1 gate。
3. `import_python_state`/`import_index` 删除与规则 schema 收紧（P1 后续块）。
4. 提交：本轮按模块分批（installer / 协议 / 文档）。
