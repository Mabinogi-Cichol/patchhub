# 跨平台安装器迁移施工记录

设计：[design.md](design.md)。日期：2026-10-08（Asia/Shanghai）。状态：**P0 实施中：契约冻结与 Windows 隔离原型已通过 Gate 1（见 [p0-contracts.md](p0-contracts.md)、[p0-sandbox-probe-windows.md](p0-sandbox-probe-windows.md)）；Linux 对照与 P1+ 待执行**。

## 当前状态

| 工作 | 状态 | 说明 |
| --- | --- | --- |
| 前序范围/安装器差异盘点 | 已完成只读核对 | 来源 P0–P6 完成；动态 ZIP/EXE 准备与授权仍为 Decky Python，Windows 只展示候选 |
| 设计与功能归档 | 已完成 | 独立设计/施工、索引与关联入口；文档检查见下文 |
| 来源迁移兼容遗留盘点与清理规划 | 已完成只读检查与设计；代码待清理 | 六类遗留、消费者依赖、阶段门槛已纳入设计第 7.3 节；不是已清理或已验证 |
| P0 契约/样本/平台隔离 | 契约冻结定稿并通过 Gate 1；Windows 原型完成；Linux 对照待授权 | 契约 v0.5（encoding/ID/schema/锁/worker/限额/依赖冻结，Gate 1 终审通过）；Windows LPAC+Job 原型 31 项探针 27 pass；Linux/Deck 对照、版本矩阵与 P1+ 待执行 |
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
2. 进行中：P0 阶段成果提交（用户要求，按文档/代码/配置拆分）。
3. 下一步（用户选择）：P0-D Linux/Deck 对照（bwrap 启动 Rust worker 原型 + 实机对照；Deck 操作按授权范围）。
4. P1 开工前置：冻结宿主 RPC 方法/事件/终态契约（p0-contracts §11）；P1 实施：来源编码替换/规则收紧/直接初始化、共享 ZIP 获取-配方-授权-状态路由。
