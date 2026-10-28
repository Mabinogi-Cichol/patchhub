# 补丁管理整合与流程精简施工记录

设计与施工计划见 [design.md](design.md)。

## 状态

已实现并本地提交。双端页面、核心直链输入和直接解析路径已完成；自动化、Windows 原生窗口和候选构建通过，真实游戏与 Steam Deck 验收待执行；尚未合并或推送。

| 阶段 | 状态 | 证据 / 退出条件 |
| --- | --- | --- |
| P0 冻结范围与建立基线 | 已完成 | 开工基线 `130a434`；工作区开始时干净 |
| P1 整合页面 | 已实现；Windows 原生矩阵通过 | 双端三栏默认补丁管理，保留下拉与游戏信息；Decky 实机待验 |
| P2 删除四类流程 | 已实现；相关自动化通过 | 可选 ZIP、直接解析、手动 URL、查询呈现 |
| P3 契约 / 文档 / 审阅 | 已完成；发现已处理 | 生成契约、双端桥接与现行说明已更新；见[实现审阅](review.md) |
| P4 自动化 / 原生 / 实机验证 | 自动化与 Windows 原生通过；真实安装 / 设备待验 | 未执行项单列，不以夹具代替实机 |

## 2026-10-28：定向调查与施工计划草案

- 用户要求制定施工计划：删除概览页签，并删除之前注释指出的四类多余状态判断 / 流程；不确定处直接询问，不自行编造。
- 工作区开始时干净，当前 `main` 为 `fbe3ce5`；从 `main` 创建 `fix/patch-management-flow-simplification`。本轮只写计划文档。
- 核对 Windows 游戏详情、概览、补丁管理、任务句柄、来源模型；核对 Rust 动作投影、安装服务、CLI EXE 探测和实际解析 worker，以及 Decky / 共享 UI 的本地输入消费者。
- 事实：普通来源与签名目录的本地 ZIP 能力不同；EXE 提前可用性探测与实际隔离执行均有源码路径；只有来源候选但未投影成安装条目时前端会显示待核对；持久查询结果状态与当前任务 running 不是同一概念。
- 已在对话中提出 Q1–Q5，涉及本地输入删除层级、EXE 删除层级、仅网页线索、查询错误处理和双端范围。未将任何默认选项视为用户答复。
- 依据 overengineering-review 技能对计划草案作专项检查：结论为关键依据待确认，不将互斥实现路线当作已确认方案。

## 计划阶段验证记录（历史）

- 产品测试、构建、原生界面、真实安装和设备检查：未执行；本轮为计划任务。
- 文档检查：`git diff --check` 通过；新增设计 / 施工记录的本地相对链接目标检查通过。最终工作区仅本次计划文档及功能索引改动。
- 计划定稿阶段没有修改产品代码，没有 Git 提交、推送、发布或设备操作；后续文档提交见下节记录。

## 2026-10-28：两轮答复与计划定稿

- 用户确认 Windows 和 Decky 都删除概览页签及相关流程。
- 本地 ZIP：删除强制文件输入，保留可选导入。
- EXE：提前探测和实际解析隔离一起删除，直接运行解析器。
- 来源：保留网页，明确未找到下载链接，新增用户手动输入 ZIP / EXE 直链；不支持网盘分享页 / 提取码。
- 查询：删除界面失败提示，结束停止动画、保留旧结果；无旧结果显示“暂无补丁结果”，保留重新查询，具体错误只记录日志。
- 补查到实际运行时依赖：Linux worker 固定 `/input/archive.exe`、Windows 相对输入路径；生产运行时还创建 worker / 输入副本并配置资源限制，计划已纳入直接启动和实际路径改造。
- 补查到手动来源选择目前只接受缓存 mirror；新增直链需贯通核心下载、安装目标、协议和双端宿主，不能只增加输入框。
- 更新设计的产品决策、技术方案、施工顺序、逐模块清单和验收矩阵；完成过度设计专项自审，未发现需新增配置、重试或其他独立框架的依据。

## 2026-10-28：按最终范围调整分支类型

- 用户指出本任务更适合 feature。最终范围包含新增手动 ZIP / EXE 下载链接输入与双端安装流程接入，属于功能开发，不能仅因包含旧流程删除就归类为 fix。
- 将当前分支从 `fix/patch-management-flow-simplification` 改名为 `feature/patch-management-flow-simplification`；仍基于原 `main` 基线，文件改动和提交历史保留。上文创建 fix 分支的记录保留为历史事实。
- 同步设计中的任务类型和开发分支。未修改产品代码，未提交或推送。

## 2026-10-28：提交计划文档

- 用户明确要求提交 Git；本次提交范围为设计与施工计划、本施工记录和功能索引，使用 `docs` 类型提交，所在开发分支仍为 `feature/patch-management-flow-simplification`。
- 沿用已完成的文档相对链接检查，提交前检查工作区及暂存区 diff；不重复执行产品测试。
- 仅创建本地提交，不推送、合并或发布；功能保持待实现、待验证。

## 待办

- 真实游戏安装 / 游戏内生效、Steam Deck 部署与实体手柄验收；当前不自行操作设备。
- 本轮实现已创建本地提交，见下节提交记录；尚未合并或推送。

## 2026-10-28：开始施工与实现

- 用户要求“开始施工”。当前分支 `feature/patch-management-flow-simplification`，开工提交 `130a434`，开始时工作区干净。
- Windows 删除概览及候选卡片路径，默认进入补丁管理；迁移重新匹配、语言偏好、未完成操作入口与游戏路径；仅网页候选进入同一补丁下拉，并可输入直链。
- Decky 默认面板改为 patches，保留 patches / records / sources 三栏；迁移游戏信息、查询控制、语言与记录入口，保留原生下拉和可选 ZIP。
- 核心新增 `manual_download: {source_id, candidate_id, url}` 输入，与 release / selection / archive 目标互斥。重读真实游戏和候选，绑定实际 URL 后下载一次，按文件内容进入 ZIP / EXE 安装；自动来源规则和手动输入规则分开处理。
- EXE worker 改为普通子进程，接收实际输入路径；删除 LPAC / AppContainer / ACL / bwrap、probe 二进制和仅服务隔离的副本、资源上限。保留流式输出、解析器内容验证和实际取消回收。
- 默认输入不再回退为本地 ZIP；现有可选导入能力、文件选择和 `archive` 协议继续保留。
- 查询运行时使用实际任务动画；终态错误仅记录日志；无结果显示“暂无补丁结果”。保留结果时尊重当前来源启用 / 订阅事实，不恢复已撤销来源的旧动作。
- 同步生成 TypeScript / Python / JSON schema、双端宿主和 Flutter 内置 schema，调整协议夹具及相关测试。
- 多 Agent 实施分工：worker crate、共享核心 / 协议、Decky 三个写入范围互不重叠；主 Agent 负责 Flutter 和集成；另有只读独立审阅。

### 独立审阅发现与处理

1. 下载器原阻塞 I/O 不及时响应取消，且缺省总超时会误伤合法慢下载：改为任务内可取消异步网络 I/O，复用同步 Transport / Read 接口，不增加总运行时限或遗留后台网络线程。真实本地服务的响应头 / 正文停滞取消测试通过。
2. 失败查询旧结果合并可能恢复退订 / 停用来源：双端按 incoming sources 的 enabled / subscribed 过滤；空来源数组同样权威，不恢复缺少来源身份的旧签名条目；非空新安装投影优先。
3. 手动链接任务原先强制关闭 Decky 自动清理：已恢复沿用现有用户设置。
4. Flutter 全局来源任务列表可能再次展示游戏查询错误：过滤终态查询任务，同时阻止 TaskSummary 为查询渲染错误或诊断折叠；Decky 同步过滤终态任务。
5. Flutter 将局部 `match_games` 误认为全库查询：改为按实际 instances 判断，增加实际子进程查询 / 取消测试中的局部范围断言，其他游戏不再出现查询动画或禁用重新匹配。

### 已执行的阶段验证

- Windows / WSL worker：各 18 项测试通过；两平台 all-target Clippy 与 worker 格式检查通过。包含真实解析器小样本分析 / 流式提取、实际路径、无效输入、运行中取消和消费者退出后的进程回收。
- 核心初轮原生单元：CLI 57、core 372 通过（5 项原有忽略）；projection 集成 24、signed install 21 通过。取消 I/O 改造后定向 acquire 13 项及真实 service 停滞连接取消测试通过。
- 协议 2 项单元与 3 项契约夹具通过；生成契约已更新。
- Decky Python 后端 111 项通过，3 项原有跳过；前端初轮 78 项通过，后续审阅修复的 session / 页面定向 27 项通过；typecheck / Rollup 构建通过，最后集成状态仍待终检。
- Flutter 初轮静态检查通过；首轮定向 46 项通过、1 项旧选择器测试定位失败，定位已更新。完整回归暴露旧展开入口、生成 schema 同步和新测试进程等待问题，正在修正 / 重跑，不计为通过。
- 上述原生权限重跑用于解决沙盒 telemetry / 文件锁限制，没有修改产品锁或吞掉真实失败。

### 最终验证与证据

- Flutter：`flutter --no-version-check analyze --no-pub` 无问题；完整 `flutter test --no-pub --reporter expanded` **245 项通过、1 项原有跳过**。证据 `.artifacts/patch-management-flow/flutter-analyze.log`、`flutter-tests-final.log`。新增验证覆盖手动 URL 参数 / 目标互斥、三页默认与空态路径、查询停止 / 取消、全局错误不泄漏、来源退订和局部查询范围。
- Windows 原生：`integration_test/modern_visual_test.dart -d windows` 通过；深浅主题、1440×900 / 1180×760 / 850×560，最小尺寸文字放大 1.5，**30 张截图**；游戏路径和技术详情展开、三栏与全局来源切换、更新弹窗、最大化 / 还原和关闭路径通过。已人工查看暗色普通尺寸与浅色最小尺寸截图。证据 `native-ui.log`、`native-ui/result.json`、`native-ui/metrics.json`。使用夹具数据，不等于真实安装验证。
- Decky：最终前端 **80/80**，typecheck / Rollup build 通过；共享 UI 契约和 Flutter 展示夹具检查通过。证据 `decky-tests-final.log`。页面测试使用实际 JSX 和原生控件替身，不等于游戏模式 / 手柄验证。
- Rust 最终：`cargo clippy --offline --workspace --all-targets -- -D warnings`、`cargo fmt --all --check` 通过；v5 契约生成检查通过。证据 `cargo-clippy.log`、`cargo-fmt-check.log`、`contractgen-check.log`。
- Linux / WSL：core / protocol / CLI 的 locked check 通过；手动安装服务 **3/3**、acquire **13/13**、协议 **2 项单元 + 3 项契约夹具**通过。证据 `linux-core-check.log`、`linux-manual-download-tests.log`、`linux-acquire-tests.log`、`linux-protocol-tests.log`。worker 两平台测试结果见阶段记录。
- Windows release：core / worker locked offline release 构建通过；Flutter Windows release 构建通过，放入本轮 core / worker，形成工作区本地候选。目录 `apps/windows-flutter/build/windows/x64/runner/Release`；三个 EXE 的哈希记录于 `.artifacts/patch-management-flow/windows-candidate.json`；构建证据 `windows-rust-release.log`、`windows-flutter-release.log`。未制作发行附件或发布。
- 文档：新增 / 修改的 13 个文档链接目标存在；工作区 diff 空白检查通过。历史 P0 文档已有的退役 Tauri 代码链接不计入本次新增链接验证。
- 以上为实现阶段验证记录；当时尚未提交实现代码，未合并 main、推送、发布或部署 Steam Deck。后续本地提交见末节。

### 尚未执行的验收

- 真实远程补丁 ZIP / EXE 安装、取消后的真实游戏文件结果、游戏内生效。
- Steam Deck 部署、真实游戏模式页面、实体手柄与设备上的实际安装。
- 安装版跨版本更新和发行附件核验不属于本次本地候选构建。

## 2026-10-28：候选版本元数据补正

- 新 EXE 的文件版本为 0.4.0+1，但候选目录遗留的 `app-version.json` 和本地更新清单仍为 0.3.0，导致界面 / 客户端更新读取旧值。
- 使用现有生成器刷新当前候选，并修复预览脚本提前返回遗漏元数据的问题。详见[构建入口施工记录](../platform-build-entrypoints/progress.md#2026-10-28预览残留旧版本元数据修复)。这次不修改已确认的功能范围或版本号。
- 刷新后 `app-version.json` / `update-manifest.json` 为 0.4.0，EXE 为 0.4.0+1；隔离预览脚本夹具验证通过。已经运行的旧进程未被关闭，需重启候选后重读元数据。

## 2026-10-28：提交实现与验收记录

- 用户要求先提交 Git；保持开发分支 `feature/patch-management-flow-simplification`。
- 功能代码与必要测试提交：`c552199`（`feat(patch-management): 整合双端补丁管理并精简安装与查询流程`）。
- 预览版本修复独立提交为 `118ba12`；构建文档另行归档，不混入功能代码提交。
- 功能设计、施工记录、审阅和相关文档引用单独提交；提交前核对已有验证证据、文件清单、工作区与暂存区空白检查，沿用上文验证结果，不重复执行产品测试。
- 只创建本地提交，未合并 main、推送、发布或操作设备；真实游戏安装和 Steam Deck 实机验收仍待执行。

## 2026-10-28：补丁语种选择盘点与移除方案

- 状态：调研与方案完成，**待实现、待验证**。专项方案见[补丁语种选择盘点与移除计划](language-selection-removal-plan.md)，所属[设计](design.md)。
- 从干净 `main`（`cd71c21`）创建 `fix/remove-patch-language-selection`，本轮只编写计划文档。
- 静态盘点确认 3 处可操作控件（Windows 2、Decky 1），以及状态、排序、RPC / CLI、宿主桥接、查询上下文、两种生产缓存摘要和无生产调用的旧兜底函数。
- 将来源多语言采集、6 处内部简中优先、安装语言事实 / 门禁、补丁语言元数据和 Steam 本地化展示单列，推荐保留，明确扩大删除范围的影响。
- 沿真实调用核实：当前 `decide_fallback` 不使用旧 `needs_fallback`；HGamer 固定搜索路径不随用户偏好改变；偏好不是安装请求的语言参数。
- 已按 overengineering-review 审阅方案，采用直接删链路和旧摘要自然失效，不增加兼容开关、配置迁移或整库清理。
- 本轮验证仅为源码调用检索、文档链接目标和 diff 空白检查；未执行产品测试、构建、原生 UI 或 Steam Deck 验收。P1–P5 全部待执行，未提交 Git、合并、推送或发布。

### 用户补充：安装语言事实与校验也删除

- 用户在了解“安装语言事实与校验：3 条入口”的用途后明确要求“这个也删除”。原先建议保留该项已被本次决定替代；[专项计划](language-selection-removal-plan.md) 增加 P1b，状态仍为待实现。
- 删除范围包括 ZIP / EXE / signed 的语言预期、事实类型 / 成员、Steam 游戏语言读取、比较和兜底、`exe_language_mismatch` 分支 / 文案及专属测试；不采用固定关闭开关或始终通过的空壳事实。
- 补充核实 EXE 主编排没有整体事实有效性门禁，实际以文件条件选择 payload；计划区分语言失败事实与实际阻断，仍清理无用途的语言收集逻辑。
- 补丁来源语言仍用于展示与内容身份。ZIP mapping_identity / EXE content_id 使用所选补丁语言，不依赖 GameFacts.language，因此本轮不改内容身份算法或安装记录格式。保留版本、DLC、基线和真实文件条件。
- 同步修订验收矩阵：覆盖三条入口的语言不符、缺失和配置冲突不再触发语言拒绝，以及非语言校验 / 旧安装记录仍有效；以上均待执行。
- 本次只更新计划、设计和索引；文档链接与 diff 空白检查通过，未修改功能代码或运行产品测试。

### 用户澄清：游戏信息跟随 Steam 语言

- 用户明确“跟随 Steam”指游戏名、游戏封面等信息，不是修改 Flutter 应用界面 locale。专项计划第 13 类与 P2b 已按此修订，保持待实现 / 待验证。
- 游戏名已使用 Steam 界面语言解析；Decky 已使用 Steam display_name / 图标 API，沿用现有路径。Windows SteamArtworkStore 仍固定优先并限定匹配 `_schinese`，需要改为按当前 Steam 界面语言选默认缓存图，保留自定义图优先及默认图片回退。
- 游戏名的 name_language 可能因翻译缺失而为空，不能将其作为图片语言的唯一来源；计划要求传入图片所属 Steam 根的界面语言，刷新时同步清理语言 / 图片缓存，不新增后台监听或下载。
- appmanifest 安装语言事实的删除与 Steam 界面语言读取分开；补丁语言元数据和内容身份仍保持原语义。
- 本次只修改文档；相对链接与空白检查通过，未修改代码或执行产品验收。

## 2026-10-29：语种流程移除与 Steam 游戏信息语言实施

状态：P1 / P1b / P2 / P2b / P3 / P4 已实施并完成相关自动化；P5 候选构建与协议冒烟通过，原生界面启动未完成、真实游戏 / Steam Deck 验收待执行。专项[设计与移除计划](language-selection-removal-plan.md)及所属[设计](design.md)已同步；分支 `fix/remove-patch-language-selection`，基线 `cd71c21`，未提交、合并、推送或发布。

### 实际变更

- Windows 两处及 Decky 一处语言选择控件、会话状态、候选排序权重全部删除；名称提示、来源开关、查询 / 取消和补丁 / 下载项选择保留。
- `preferred_language` 从两个协议 Params、CLI、Rust 匹配调用链、Flutter 与 Decky / Python 宿主移除；同步前移 Decky forceDetails，删除专属语言格式校验、旧 needs_fallback 及其测试。
- 匹配 input_key / fallback_query_key 不再含偏好。旧摘要自然失效，沿原路径重算；持久回归覆盖旧值 null / zh-CN，以及仅兜底摘要过期，证明重算后稳定复用且索引未清除。
- 删除 LanguageFacts、GameFacts.language、FactsExpectation 的语言 / 开关、current_language、映射 / 比较和兜底、exe_language_mismatch 分支及文案。ZIP、EXE、signed 全部接入无语言事实的预期；版本、DLC、身份、基线及实际文件条件保留。调整混合测试，删除旧专属断言，新增实际 ZIP / signed 安装与 EXE 共用事实路径回归。
- `scan.steam_languages` 返回规范化 Steam 根到界面语言的映射，与游戏名共享 Rust 已有 config / 同根注册表读取结果。Windows 图片按所属根的 Steam 语言选取缓存图，无对应图时取默认图；保留自定义图优先，沿原刷新失效图片缓存；name_language 为空也不丢失选图语言。
- v5 信封保留，projection_version / request_validation_version 改为 2，其余契约版本为 1；Rust 生成的 TS / Python / schema / cases、Flutter 内置 schema 和宿主能力检查同步更新。
- 补丁语言元数据、简中代表行策略及 candidate / selection / content 身份算法保留；应用界面 locale 与中文文案保持原范围，清理安装错误中已不适用的“检查游戏语言”建议。

### 验证与证据

- Flutter 静态分析无问题；完整回归 **246 项通过、1 项原有跳过**。随后新增同根语言接线断言，图片定向 **6/6** 通过，覆盖英 / 日 / 简中、缺少名称翻译、默认图、自定义图、刷新和 Windows 规范化根路径。日志 `.artifacts/language-removal/flutter-analyze.log`、`flutter-tests.log`、`artwork-tests.log`。
- Decky 前端 **80/80**、typecheck、Rollup build 通过；Python 后端 **111 项运行、3 项原有跳过，其余通过**。共享 UI 契约与 Flutter 展示夹具一致性通过；日志 `python-tests.log`、`decky-build.log`。
- Rust：核心单元 **370 项通过、5 项原有忽略**；新增 EXE 事实回归单独 **1/1** 通过。CLI 单元 **57/57** 及两个 smoke 套件通过；投影 **24/24**、signed 安装 **22/22** 通过。日志 `rust-tests.log`、`exe-facts-test.log`。整组执行在一条遗留语言摘要断言处停止；该断言已清理后，source_data_v2 **11/11**、source_matching **24/24**、localized_game_names **5/5** 定向通过，见 `rust-focused-tests.log`，不将未执行的其余套件记为通过。
- 协议 **2 项单元 + 3 项夹具**通过；encoding golden 已按 Rust 生成器刷新；contract-gen check、workspace all-target Clippy（`-D warnings`）、Rust fmt 和 diff 空白检查通过。日志 `protocol-tests.log`、`contract-check.log`、`rust-clippy.log`。首次 Rust 锁权限和 Flutter 遥测目录错误按正常权限重跑；未修改产品锁。
- Windows Rust / Flutter **release 构建通过**，本地候选位于 `apps/windows-flutter/build/windows/x64/runner/Release`；已同步新 core 与现有 worker，刷新 app-version / update-manifest，三个 EXE 哈希见 `windows-candidate-hashes.json`。候选真实 JSONL 冒烟证明 scan 输出 Steam 界面语言、两个旧语言请求和 CLI 参数均拒绝，见 `candidate-smoke.json`。构建日志 `windows-rust-release.log`、`windows-flutter-release.log`。
- 原生 UI 矩阵：Debug 原生构建通过，但 Flutter 报 `Error waiting for a debug connection` / `Unable to start the app on the device`，未产生截图，**不计为通过**。随后使用 computer-use 检查未发现 PatchHub 错误窗口，直接启动测试候选超时，复查仍无窗口；未据此认定单实例、权限或业务代码为根因。日志 `native-ui.log`。
- 按 overengineering-review 沿实际调用与差异复核，无新增默认语种 / 兼容开关 / 监听 / 迁移器；失败与取消继续走既有路径。Python 首轮暴露误删通用 re 导入，已恢复并完整重跑；ZIP 新夹具的响应域名与来源不符已修正，未改产品域名校验。

### 待验收

- Windows 原生 UI 成功启动后的矩阵与真实 Steam 语言切换展示。
- 真实远程 ZIP / EXE 安装、语言不同情况下游戏内生效、Steam Deck 部署 / 游戏模式 / 实体手柄。
- 本地候选验证不代表正式安装版、线上 CI 或发行验收；未操作 Steam 设备。

2026-10-29 后续补记：关窗优化阶段已在当前代码上完成 Windows 原生深浅主题 / 三种尺寸的 30 图矩阵，先前“原生 UI 未能完成启动”的记录保留为历史失败，本轮矩阵通过。详见[关窗施工记录](../windows-visual-polish/progress.md#原生验证补记)。实际 Steam 语言切换、真实安装和 Steam Deck 仍待验收。

### 2026-10-29 本地提交

- 用户要求“提交 GIT”；语种流程代码与必要测试提交为 `2b409bb`（`fix(language): 移除补丁语种偏好与安装语言校验并跟随Steam展示游戏信息`）。
- 施工期间为后续关窗优化切换到从 main 创建的 `fix/windows-idle-close-latency`，本轮待提交内容均保留并在该分支分别提交；文档与代码拆分，功能索引按对应行暂存。
- 沿用本 session 已有验证，提交前核对文件清单、工作区 / 暂存区空白和文档链接；未重复执行产品测试。真实游戏、Steam 语言切换与 Steam Deck 仍待验收。
- 仅本地提交，未合并 main、推送、打 tag 或发布。
