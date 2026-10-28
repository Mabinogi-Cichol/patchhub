# 补丁语种选择与安装语言校验移除计划

状态：2026-10-29 已实现，相关自动化、Windows release 候选构建 / 协议冒烟及后续原生 UI 30 图通过；真实 Steam 语言切换、设备 / 游戏验收待执行。施工记录见 [progress.md](progress.md)，所属功能设计见 [design.md](design.md)。

调研日期：2026-10-28。代码基线：`main` 的 `cd71c219c374ba534f5f3aa4d9eba251b0cc8ed4`；开发分支：`fix/remove-patch-language-selection`。下文盘点保留开工前证据，实施与验证结果以施工记录为准。

## 1. 结论与统计口径

当前有 **3 处用户可操作的补丁语言选择控件**，均为“未指定 + 简体中文 / 繁体中文 / 英文 / 日文”。它们共用或连接到 2 种前端状态作用域、2 处展示排序、2 个匹配请求方法、1 个 CLI 参数和 2 种生产缓存摘要。

下表按职责归为 **13 类涉及点**，不是文件数量或互斥调用次数。用户选择、来源网页的语言和游戏安装语言是不同事实，不能通过搜索 `language` 后统一删除。移除第 1–8 类的用户偏好链路；2026-10-28 用户进一步确认第 11 类的安装语言事实与校验也删除。第 9、10、12 类保留；第 13 类按用户澄清调整为游戏名、封面等游戏信息跟随 Steam 界面语言，Flutter 应用界面 locale 不在本次调整范围。具体边界见第 4 节。

## 2. 当前工作流盘点

| 编号 | 涉及部分 | 当前行为与证据 | 推荐处理 |
| --- | --- | --- | --- |
| 1 | Windows：2 处选择控件 | 游戏补丁管理顶部“偏好的补丁语言”：`library_page.dart:771`；单游戏来源订阅页“语言偏好（本次会话）”：`source_pages.dart:432`。两处读写同一个 `AppModel.preferredLanguage`。 | 两处一起删除，保留重新匹配及来源开关。 |
| 2 | Decky：1 处选择控件 | 游戏补丁页“查找补丁”区：`index.tsx:455`；局部 `useState` 定义于第 288 行。 | 删除控件与局部状态，保留查询和取消。 |
| 3 | 前端状态：2 种作用域 | Windows 是 AppModel 内的会话状态，跨游戏共享；Decky 是游戏详情组件局部状态，按组件生命周期存在。两者均没有独立的语言偏好持久配置。 | 删除字段、setter、参数传递；不增加配置迁移。 |
| 4 | 前端排序：2 个实现 | Flutter `displayEntries` 与共享 TS `displayPatches` 对未投影为安装条目的 discovery 候选按语言偏好排序；不删除其他语种候选，安装条目另有本体 / DLC 等排序。 | 删除语言排序权重及函数参数；保留现有来源、类型排序和候选选择身份。 |
| 5 | 对外输入：2 个方法、1 个 CLI 参数 | `MatchGamesParams` / `RematchGameParams` 的 `preferred_language`；CLI `--preferred-language` 与 `CommandParams.preferred_language`。 | 从权威协议和 CLI 真正移除，不保留无效字段或隐藏开关。 |
| 6 | 宿主与服务转发 | Flutter 单游戏重新匹配、全库手动重新匹配携带偏好；Decky JSX → session → SourceApi → Python main → SourceService → Rust；Python `_language` / `LANGUAGE_PATTERN`、Rust `validate_query` 有专用校验。 | 沿整条调用链删参，连同专用校验；保留名称提示和 `force_details`。 |
| 7 | 匹配上下文与缓存：2 种生产摘要 | `QueryContext.preferred_language`；`sources/match.rs::input_key` 和 `matching.rs::fallback_query_key` 都纳入偏好。偏好变化可使旧匹配记录不再复用。 | 删除上下文成员和摘要输入，同步固定夹具；验证旧记录自然失效后新记录可复用。 |
| 8 | 旧兜底判断：1 个函数 | `identity.rs::needs_fallback` 按偏好过滤官方候选，但仓库调用检索仅找到其单元测试。当前主路径使用 `matching.rs::decide_fallback`。 | 删除无生产调用的旧函数及其专属测试，不把它描述为当前线上过滤逻辑。 |
| 9 | 来源网页语种采集 | patch.moe 从实际页面取 en / zh-CN / zh-TW 表单并 POST，合并相同下载 URL 的标题别名；HGamer 搜索路径固定 `/zh-CN/search`，不读取用户偏好；Kagura 国内 / 海外入口是来源配置。 | 保留。移除将改变来源覆盖，与取消用户选择不是同一项工作。 |
| 10 | 内部“优先简中”：6 处选择策略 | patch.moe 同 URL 归并、`matching.rs::select_entry`、ZIP `choose_language`、EXE `resolve_exe`、显式 mirror `manual_selection::resolve`、手动直链 `resolve_download`。位于 5 个文件，均独立于用户偏好。 | 本轮保留；详见第 4 节。 |
| 11 | 安装语言事实与校验：3 条入口 | 动态 ZIP、动态 EXE、签名目录安装均构造 `FactsExpectation`；`facts.rs::language_fact` 比较 appmanifest 的 UserConfig / MountedConfig 与预期，并可产生 `exe_language_mismatch`。ZIP / signed 使用整体事实有效性检查；当前 EXE 主编排只按实际文件条件选择 payload，语言失败不直接作为安装门禁。 | 用户已确认删除三条入口中的语言事实收集、预期与校验，以及专属错误和测试，见 P1b。 |
| 12 | 补丁语言元数据、展示与身份 | `SourceCandidate.languages`、`SourceIndexEntry.language`、SourceRule / Columns / catalog 的语言字段；双端补丁说明与下拉标签；候选 ID、selection ID、内容身份可绑定语言事实。 | 保留，使用户仍可分辨语言不同的下载项，安装 / 重装身份保持一致。 |
| 13 | 游戏信息本地化与应用界面语言 | core 已读取 Steam 界面语言解析本地化游戏名，Decky 使用 Steam 的 display_name / 图标 API；Windows 图片缓存仍固定优先 `_schinese`，默认图仅匹配无后缀或 `_schinese`；Flutter 应用界面 locale 固定 zh-CN。 | 游戏名、封面、横幅、缩略图等信息跟随 Steam 界面语言，修正 Windows 固定简中选图；保留用户自定义图优先级。Flutter 应用界面 locale / 业务文案保持现状，见 P2b。 |

源码入口：

- Windows：[管理页](../../../apps/windows-flutter/lib/pages/library_page.dart)、[来源页](../../../apps/windows-flutter/lib/pages/source_pages.dart)、[AppModel](../../../apps/windows-flutter/lib/models/app_model.dart)、[展示逻辑](../../../apps/windows-flutter/lib/presentation/library_presentation.dart)。
- Decky：[页面与 RPC](../../../apps/decky/src/index.tsx)、[session](../../../apps/decky/src/session.ts)、[SourceApi](../../../apps/decky/src/source-tasks.ts)、[Python main](../../../apps/decky/main.py)、[SourceService](../../../apps/decky/backend/source_service.py)；[共享展示逻辑](../../../packages/ui/src/index.ts)。
- 协议与 CLI：[权威协议](../../../crates/protocol/src/lib.rs)、[CLI 参数](../../../crates/cli/src/main.rs)、[runtime](../../../crates/cli/src/runtime.rs)。
- 核心：[QueryContext](../../../crates/core/src/sources/model.rs)、[主匹配流程与兜底缓存](../../../crates/core/src/sources/matching.rs)、[匹配输入摘要](../../../crates/core/src/sources/match.rs)、[查询校验](../../../crates/core/src/sources/registry.rs)、[旧兜底函数](../../../crates/core/src/sources/identity.rs)。

### 2.1 实际执行路径及容易误判的地方

1. Windows 改偏好会通知页面，立即改变 discovery 候选排序；单游戏重新匹配和全库手动重新匹配会传值。扫描后自动处理 pending 游戏的 `match_games` 不带该值。
2. Decky 改偏好会重新渲染排序，单游戏重新查询传值；全库入口 `session.matchGames(null)` 使用缺省空偏好，没有全库语言选择控件。
3. 主匹配流程接受偏好并写入摘要，但 `decide_fallback` 依赖发布源是否已命中、覆盖是否完整等事实。HGamer 通过名字 / 名称提示查找，不按该偏好改变搜索网页语种。不能声称删偏好会直接停止某个语种的网络请求或恢复被过滤的候选。
4. 不同下载 URL 对应不同补丁时，当前 `select_entry` 先判定多 URL 歧义，再选相同 URL 的简中代表行。取消语言偏好不会自动解决多 URL 选择，也不能改成静默取第一项。
5. 当前语言偏好不直接进入安装请求；安装器使用所选候选 / 来源声明及 Steam manifest。仅隐藏选择器不能解除语言不匹配错误；本计划已按用户补充要求单独加入实际安装事实与门禁的删除。

## 3. 推荐移除范围与实施顺序

目标流程：选择游戏 → 自动展示已匹配补丁 → 有多个结果时选择具体补丁 / 下载项 → 安装。需要时直接重新匹配，不再先选择偏好语言；安装不再比较 Steam 游戏语言与补丁语言，也不因 UserConfig / MountedConfig 语言缺失或冲突而拒绝；仍可看到补丁自身的语言说明。

### P1：同步删除输入契约与核心偏好依赖

- 从两个 Params、CLI 帮助 / 解析、`CommandParams` / JSON 解码及四处 runtime 转发中删除偏好。
- 从核心 match_games / rematch_game / rematch_source 等签名、内部转发、`QueryContext`、查询校验、两种生产摘要中删除对应参数。
- 删除仅服务该输入的 `valid_language_code`。删除无生产调用的 `needs_fallback` 及其专属测试；保留当前实际使用的 `decide_fallback`。
- 同步更新 `sources/canonical.rs` 的查询示例及 encoding golden、source_matching / hgamer_live 和 adapter 测试里的 QueryContext 初始化。候选语言身份和 URL 编码算法不变，不因此整体提高来源 encoding、store 或索引 schema 版本。

### P1b：删除安装语言事实与校验（用户已确认）

- 从 `FactsExpectation` 删除 `language`、`require_language` 及其专属默认值函数；从 `GameFacts` 删除 `language` 成员，从 failures / all_failed / game_facts 等构造与汇总路径删除语言分支；删除 `LanguageFacts` 类型及 `installer/mod.rs` 导出。
- 删除 `facts.rs` 中的 `language_fact`、`current_language`、`expected_language` 和语言映射 `LANGUAGES`。不保留“永远通过”的语言事实、`require_language=false` 常量或可重新开启的配置。
- 动态 ZIP / EXE 的 expectation 不再传语言；ZIP orchestrator 删除空语言时补读 Steam manifest 的逻辑；签名目录 expectation 删除当前语言读取与 release 语言兜底，并清理因此失去用途的形参。
- 清理 `ResolvedZipCandidate::conditions_sha256` 中的语言条件成员。该方法当前无生产调用，不能将其误认为 ZIP 实际内容身份的计算路径。
- 删除 `exe_language_mismatch` 的生产错误分支及共享 UI 专属文案。清理只服务语言事实的单元测试、类型初始化、exe_report 夹具和注释；混合验证语言、版本、DLC 的测试仅删除语言断言，保留其余业务验证。
- 保留 manifest 的通用解析、游戏身份、运行状态、build、DLC、基线、真实文件条件和失败传播；EXE 的文件条件仍决定 payload。保留所选补丁的来源语言元数据与内容身份，不再将这些字段传给安装事实校验。
- 所有三条入口连同显式 mirror / 手动直链路径统一按上述范围清理，不能只处理默认自动安装。

### P2：同步删除双端入口与转发

- Windows 删除两个控件、AppModel 字段 / setter、重新匹配请求字段及所有 displayEntries 调用中的参数。管理页保留重新匹配按钮与运行中动画；订阅页保留来源开关和重新匹配。
- Decky 删除一个控件、useState、displayPatches 参数，更新 session / SourceApi / callable 元组及 Python 两层服务签名。注意 `matchGames` 的 `forceDetails` 原来位于 language 之后，必须同步前移，防止位置参数错位。
- 删除 `_language`、`LANGUAGE_PATTERN` 和专属非法语言测试，保留通用请求 / 名称 / 实例校验。
- 共享 TS 和 Flutter 只删语言排序权重，不改候选分组、mirror 选择、本体 / DLC 顺序、选择身份和 fallback 展示规则。

### P2b：游戏名、封面等游戏信息跟随 Steam 语言

- 用户已澄清“跟随 Steam”指游戏信息本地化。本次不修改 `MaterialApp.locale`、supportedLocales 或应用业务文案，不增加应用语言设置。
- 游戏名沿用 core `steam_metadata::names_for_root` 的 Steam 界面语言解析及既有默认名称回退；Decky 保留 Steam appStore 的 `display_name` 与图标 API。补丁语言偏好和单游戏 appmanifest 安装语言均不参与这些展示选择。
- Windows `SteamArtworkStore` 删除固定 `_schinese` 排序，以及只接受无语言后缀 / `_schinese` 的匹配。按当前 Steam 界面语言匹配相应文件后缀；同类图片缺少该语言版本时使用无语言后缀的默认图片，再沿既有图片类型回退；没有可用图时保持现有占位。
- 保留当前账号的自定义封面 / hero / header 优先；仅影响 Steam 默认缓存图的语言选择，不改用其他账号图片，不新增图片下载流程。
- 语言输入来自图片所属 Steam 根的界面语言，复用现有 config.vdf 优先、同根注册表补充的 Rust 读取结果。实施采用 `scan.steam_languages` 按规范化 Steam 根输出语言，AppModel 的扫描快照经 LibraryPage 向图片读取器传值；`steamArtworkContext` 保留 root / activeUser，无需新增原生解析。不能以单游戏 `name_language` 替代：缺少该语种本地化名称时它会为空，封面仍应按 Steam 界面语言选取。
- 沿既有刷新入口更新名字并清理图片上下文 / AppID 缓存，使 Steam 切换语言后的刷新采用新语言；不新增后台监听或轮询。Steam 语言不可读取时使用默认名称 / 默认图片，不固定猜测简中。

相关代码：[Steam 名称元数据](../../../crates/core/src/steam_metadata.rs)、[Windows 图片读取](../../../apps/windows-flutter/lib/host/steam_artwork.dart)、[图片宿主上下文](../../../apps/windows-flutter/windows/runner/flutter_window.cpp)、[Decky 游戏信息](../../../apps/decky/src/steam.ts)。

P1、P1b、P2、P2b 是本轮施工顺序，完成前不生成供用户使用的半套候选。

### P3：契约产物与文档同步

- 以 Rust 为唯一权威，执行 `cargo run --locked -p patchhub-contract-gen -- generate` 更新 TS / Python / JSON Schema 和 v5 cases，随后同步 Flutter 内置 `assets/protocol-v5.schema.json`；不手改生成类型。
- 删除字段会改变请求接受范围，`request_validation_version` 从 1 调整为 2；新增 scan 的 Steam 根语言输出，`projection_version` 从 1 调整为 2。同步 CLI capabilities、Flutter `validateCapabilities`、生成 CONTRACTS 与相关夹具；actions / outcome 仍为 1。沿用 v5 envelope 和生成路径。
- 同步 `packages/contracts/README.md`、`docs/source-adapters.md` 的当前接口 / 缓存描述与相关当前设计。历史发布说明和施工记录保留事实，通过后续记录说明已移除。
- 本次不支持新旧宿主混装，不引入接受后忽略字段的兼容分支。新客户端、Python 桥、core 和内置 schema 随同一候选交付；旧请求按已有未知字段规则拒绝，CLI 旧参数按已有非法参数路径处理。

### P4：缓存与验证

- 两种摘要删掉该成员后，包含旧成员（包括 null）的旧摘要自然不匹配；沿现有 `record_reusable` 路径重算匹配，不增加全盘清缓存、偏好配置迁移或双版本读取器。
- 保留来源索引、详情事实、下载文件和安装记录。移除的是 Steam 安装语言事实；签名收据 / selection / content 身份中的补丁来源语言仍保留。ZIP `mapping_identity` 与 EXE `content_id` 的语言参数来自所选补丁，并不依赖 `GameFacts.language`，本轮不改其算法，也不新增身份迁移。
- 用真实持久记录回归证明“旧摘要 → 一次重算 → 新摘要稳定复用”，并覆盖此前有偏好和无偏好的记录。查询失败、取消与旧结果保留继续遵循现有流程。

### P5：验收与收尾

- 双端界面不再出现三处语言选择；用户能重新查询，并从全部实际结果中选择补丁 / mirror，语言说明仍可读。
- 两个匹配方法生成的请求不再包含 `preferred_language`；CLI 帮助和参数不再接受 `--preferred-language`；仓库生产代码不再保留该状态或语言排序分支。
- 三条安装入口不再采集或比较 UserConfig / MountedConfig 的语言，`FactsExpectation` / `GameFacts` 无语言成员，生产代码与界面无 `exe_language_mismatch` 专属分支。Steam 游戏语言与补丁标签不同、语言未填写或两处语言配置冲突，均不触发语言拒绝；实际非语言错误仍正常返回。
- 游戏信息按 Steam 界面语言展示，Windows 默认封面不再固定优先简中；切换 Steam 语言并刷新后名字与图片同步重新读取。应用界面的 locale / 中文业务文案保持现状。
- 更新当前 feature 的设计与施工记录。2026-10-29 用户已要求 Git 提交，语种代码和必要测试为一个修复提交，文档另行提交；后续关窗优化保持独立提交，具体记录见施工记录。

## 4. 保留项与已确认的安装校验移除边界

### 4.1 内部简中优先

6 处策略均不是用户选择控件。前两处选择同 URL 的展示 / 归并代表行；后四处从候选的语言列表选择一个值。当前实现将其用于安装预期及内容身份；P1b 删除安装语言预期后，该值仍用于补丁内容身份。

本轮保留。若还要消除内部简中优先，需要另行确定“同包多语言应记录单一语言还是完整语言集合”，并联动候选展示、内容身份、重装及旧记录处理；不能简单将 `find(zh-CN)` 换成 `first()` 后宣称已消除语种语义。

相关代码：[patchmoe](../../../crates/core/src/sources/adapters/patchmoe.rs)、[select_entry](../../../crates/core/src/sources/matching.rs)、[ZIP resolve](../../../crates/core/src/installer/resolve.rs)、[EXE resolve](../../../crates/core/src/installer/exe_resolve.rs)、[manual_selection](../../../crates/core/src/installer/manual_selection.rs)。

### 4.2 安装语言事实与校验：已确认删除

现有动态 ZIP 的 `require_language` 在 `ResolvedZipCandidate::expectation` 中受来源 `discovery_only`、语言是否为空及是否手动选择控制；动态 EXE 的 expectation 也会生成语言失败事实，但当前 `exe_orchestrator` 不做整体 `is_valid` 门禁，`exe_report` 使用实际文件条件，不能概括成“所有 EXE 都因语言不符阻断”。本轮三条路径都移除语言事实及相关残留。

现有签名目录安装取当前 manifest 语言，无法读取时使用 release 语言，同时要求语言事实有效；还可能因 UserConfig / MountedConfig 冲突失败。本轮删除这段读取、兜底和比较，保留其 AppID / build / 文件等事实校验。

用户已明确要求“这个也删除”。执行范围为 P1b 的真实删除，不只把开关改为 false，也不只删除前端提示。所选补丁的语言说明、候选 / selection / content 身份以及 Steam 本地化游戏名保持原用途。源码已按此实施，验证结果见施工记录。

相关代码：[语言事实](../../../crates/core/src/installer/facts.rs)、[ZIP expectation](../../../crates/core/src/installer/resolve.rs)、[EXE expectation](../../../crates/core/src/installer/exe_resolve.rs)、[签名目录](../../../crates/core/src/installer/signed.rs)、[安装编排](../../../crates/core/src/installer/orchestrator.rs)。

### 4.3 游戏信息与应用界面的边界

“Flutter 固定 zh-CN”描述应用界面的 locale，不代表游戏名和封面必须简中。用户已澄清本次只要求游戏信息跟随 Steam 界面语言；原先保留固定简中封面优先的建议由 P2b 替代，应用界面语言不调整。

此处的 Steam 界面语言不属于 P1b 删除的 appmanifest 游戏安装语言。来源网页采集语种、补丁内容语言也保留原有业务语义，不能为统一展示而改写这些事实。

## 5. 验证标准（实际结果见施工记录）

| 层级 | 必要验证 | 通过标准 |
| --- | --- | --- |
| Rust 与契约 | protocol fixtures、CLI 参数 / capabilities、core source_matching；契约生成 check、格式与相关目标静态检查 | 无偏好输入的两个方法成功；旧字段 / CLI 参数拒绝；新旧缓存回归通过；能力版本一致。 |
| Decky Python | SourceService 请求构造 / 入参测试及协议能力校验 | match_games 的 force_details 不错位；rematch_game 的名称提示与实例仍正确。 |
| 双端展示 | Flutter 现有展示 / 页面 / 请求测试，Decky game-manager / session 与共享 UI 契约 | 三处控件消失；多语言候选仍完整、顺序稳定、选中身份不漂移，重新查询 / 取消可用。 |
| 编译与生成 | Flutter analyze、Decky typecheck / build、`npm run test:contracts`、contract-gen check | 所有新签名、内置 schema、生成字段与 capabilities 一致。 |
| 安装语言移除 | ZIP / EXE / signed 及其自动 / 显式选择入口；使用非语言条件有效、Steam 与补丁语言不同、语言缺失、UserConfig / MountedConfig 语言冲突的夹具 | 不产生语言拒绝，正常完成相应安装阶段；不保留语言事实空壳；检查真实入口而非仅断言字段消失。 |
| 保留边界 | 沿用并运行受修改影响的 patch.moe 采集、语言身份、build / DLC / 文件条件与收据回归 | 多语言采集、不同 URL 歧义、非语言条件拒绝与既有补丁内容身份保持有效；旧安装记录可读取。 |
| 游戏信息语言 | 同一 AppID 同时存在简中 / 英文 / 日文缓存图；当前语言名称缺失；默认图回退、自定义图、语言切换后的刷新 | 名字按 Steam 语言或既有默认名展示；图片取当前语种，缺失时取默认图，自定义图仍优先；不因 name_language 为空选错图；应用 locale 保持现状。 |
| 界面验收 | Windows 原生候选；Decky 页面 / 手柄与设备验证按后续授权执行 | 重新匹配控制区布局收紧；来源订阅、补丁 / mirror 选择、语言说明可用。未执行不得写为实机通过。 |

优先沿现有测试入口调整有效业务断言；删除专门维护旧偏好排序 / 安装语言事实与门禁的测试，不保留空壳断言。新增回归集中在参数位置、持久缓存、候选选择身份及三条真实安装路径，不为纯控件删除堆叠同义测试。

## 6. 过度设计审阅

依据仓库 AGENTS.md 和 overengineering-review 技能，沿入口 → 状态 → RPC → 匹配摘要 → 候选展示 → 安装事实进行了静态核查。

结论：可以直接删除偏好链路与用户已确认的安装语言事实 / 校验；不需要新增默认语种、兼容开关、语言探测器、偏好迁移器或整库缓存重置。游戏信息复用 Steam 界面语言及现有刷新路径，按实际缓存图片匹配，不扩展为应用翻译、图片下载服务或后台监听。保留来源语言、补丁内容身份和其余安装事实，避免将安装校验删除扩大为来源元数据或身份算法重构。已分别核对旧兜底函数与当前 EXE 的实际调用，未将无生产效果的分支误写为现有门禁。

上文为方案阶段审阅结论；实现阶段已沿实际差异再次核对删除范围，自动化、候选构建及未执行验收分别记录在施工记录中。

2026-10-29 实现复核：三个控件及其状态、排序、参数、专属校验和旧兜底函数已删除；三条安装入口不再构造语言事实或执行语言门禁，非语言事实和文件条件继续使用原路径。生产中的 `preferred_language` 已清空，剩余引用仅用于拒绝旧请求及重算旧缓存的回归。Steam 界面语言由现有 Rust 元数据读取器统一输出，按根传给图片层；未新增应用语言选项、翻译、下载、监听、迁移器或缓存清理策略。补丁候选 / selection / 内容身份中的语言未变。未发现需进一步修改的过度设计项；原生 UI 启动失败的原因未确认，不属于已通过验收。
