# 内置 Lua 单文件与订阅源表单重构计划

状态见[施工记录](progress.md)：五份单文件 Lua、双端具名字段表单、Steam 本体/DLC 匹配、YOZAKURA 文章地址与 Lua 驱动浏览器传输均已实现，并通过双平台自动化、真实网站、Deck 实机、空内置发行形态恢复与 Windows 原生窗口导入验证；实体手柄/软键盘/剪贴板与真实补丁安装按用户指示不列入本轮验收。同日新增的“关联 DLC 匹配归类（`variant = dlc` 与 `requires_dlc` 自动绑定）及本地 `appmanifest` 已安装 DLC 状态检测与适用条件展示”已完成代码实现与双端定向自动化验证，实机与真实 Steam 库展示**待验收**。对应[功能设计](design.md)。2026-11-16 静态调研基线为 `main@6c83576`。

## 交付与当前链路

每个内置来源在仓库有一份完整的 Lua。客户端查看和复制的是随包执行的同一份文本。Windows 和 Decky 使用相同字段进行导入、编辑和只读查看，规则 JSON 继续用于仓库配置及应用内部传输。清空内置规则和脚本映射后，两端仍可用普通导入入口，从仓库 Lua 原文和对应规则恢复五个来源。脚本也能换来源 ID 运行。

| 路径 | 调研基线行为 |
| --- | --- |
| [构建](../../../crates/core/build.rs)与[规则](../../../crates/core/resources/sources.json) | `bundled_scripts` 按历史 adapter 声明文件数组。构建先读 `common.lua`，再给每个片段加一个 LF 后拼接。两个 Kagura 入口分别只有 `return kagura(false)` 与 `return kagura(true)`，运行所需代码来自 `common.lua` 和 `kagura.lua`。 |
| [绑定](../../../crates/core/src/sources/builtin.rs)、[注册表](../../../crates/core/src/sources/registry.rs)与[执行](../../../crates/core/src/sources/adapters/mod.rs) | `validate_rule` 与设置合并按 adapter 绑定 API 1 和源码摘要。注册表加载有效规则及源码，索引更新、详情解析和补查通过统一的 Lua 执行器取用用户源码或随包源码。匹配独立合并规则时也会绑定摘要。 |
| [脚本文档与导入 RPC](../../../crates/cli/src/runtime.rs)及[脚本校验](../../../crates/core/src/sources/script.rs) | `get_script_source` 从只读快照取规则、源码与摘要，未初始化状态时直接读随包定义。返回规则统一为 `adapter=lua`。`import_script_source` 接受 `{rule, api_version, code, replace}`，保存时仅编译 Lua 语法，实际更新或匹配才执行模块和入口。 |
| [Windows](../../../apps/windows-flutter/lib/pages/script_source_editor.dart)与[Decky](../../../apps/decky/src/script-source-editor.tsx) | 两端都显示规则 JSON 文本框、Lua 文本框和一份内嵌示例。Windows 将解析后的对象传给 core，Decky 经 [Python 入口](../../../apps/decky/main.py)把字符串反序列化为对象后传给 core。来源卡片根据 `origin` 决定查看或编辑与删除。 |

## 单文件打包与兼容

本次重构遵循 [AGENTS.md 的「订阅源脚本与应用宿主的职责」](../../../AGENTS.md#订阅源脚本与应用宿主的职责)。

调研基线的 `table.lua` 是旧声明式来源的解析入口。`common.lua` 中的 `rows`、`row_entry` 也被 Shiravune 调用，剔除旧来源后仍由 `patchmoe.lua` 的局部代码处理列号、ZIP 和待定日期等站点解析约定。[Shiravune](../../../crates/core/resources/source-scripts/patchmoe.lua)

1. 将五个内置来源的现有拼接成品按原 UTF-8/LF 字节分别落入 `patchmoe.lua`、`kagura-cn.lua`、`kagura-en.lua`、`yozakura.lua`、`hgamer.lua`。两份 Kagura 各自带完整局部函数和原有模式选择。机械落盘阶段以等字节和等行为为验收条件，后续有意调整解析或匹配时沿用已有缓存失效规则。
2. 把 `sources.json` 的 `bundled_scripts` 改为 adapter 到**单一文件名**的映射。沿用 `build.rs` 的输入变更跟踪、默认配置生成、缺失映射或文件时报构建错误的路径，直接嵌入完整文本。移除拼接逻辑及失去引用的 `common.lua`、`kagura.lua`。运行时查找、摘要绑定以及有状态和无状态的文档读取继续使用同一份文本。更新仍引用旧文件的文档链接，把旧拼接描述标为调研基线或更新到新契约。
3. 保留五个内置规则的 adapter 值以稳定整份规则哈希。退役旧声明式表格来源，删除 `table.lua`、`table_html` 和 `table` 映射，以及只服务旧 `table_html` 的[校验](../../../crates/core/src/sources/validate.rs)。旧设置解码后按 `settings.added` 中的 adapter 移除原 `table_html` / `table` 条目，沿用来源删除的设置清理语义，随下一次已有设置写入提交。只读加载应用相同的有效规则过滤。旧条目停止更新、匹配和提供安装选择。其余内置及现有 `adapter=lua` 来源照常加载，Lua 可选规则字段继续用于现有脚本。更新用户指南时注明旧表格来源已退役。
4. 保留 [设置格式](../../../crates/core/src/sources/model.rs)和已有设置事务。`settings.added` 按 ID 覆盖随包定义，`adapter=lua` 的现有用户源码继续来自 `settings.script_sources`，包括早期同 ID 覆盖。删除这类覆盖恢复随包定义。全局名称覆盖、启用、订阅、间隔和 revision 沿现有语义读写，现有显式 `0` 间隔继续表示停用周期更新。新建用 `replace=false`，同 ID 冲突由 core 拒绝；编辑锁定 ID，用 `replace=true`，按原事务继承有效启用状态。内置的新覆盖仍由 core 拒绝。
5. 将等字节和相同生效规则作为升级回归条件。[索引兼容](../../../crates/core/src/sources/index.rs)依赖整份规则身份（含 adapter、API 版本、源码摘要），[匹配](../../../crates/core/src/sources/matching.rs)还依赖索引 revision、候选解析指纹和补查来源集合指纹，详情按来源、页面、索引 revision 复用，[安装解析](../../../crates/core/src/installer/resolve.rs)也绑定规则指纹。保持原 ID、规则和摘要可继续读到同一份缓存及安装选择。后续真实脚本或规则变更沿用已有的失效、更新和重匹配路径。自定义副本以新 ID 建立自己的索引、详情与匹配记录。

从当前 `build.rs` 输入以同样的 LF 拼接规则静态复算所得基线如下。Shiravune 用当前分支数据作为比较依据。

| 完整文件 | 字节 | SHA-256 |
| --- | ---: | --- |
| `patchmoe.lua` | 12175 | `78f516492c3d9d56dc91021448f7c8a95d29c84b2f53da60fb718d5cc444b314` |
| `kagura-cn.lua` | 14756 | `a56a58db73530a6109df921336d39450746f9f60c03223251ecaa0a3d28e9994` |
| `kagura-en.lua` | 14755 | `aca5d2b41d10e99307092e25606db6dca2072b012284e99bd9658e027a7b64b3` |
| `yozakura.lua` | 9185 | `2d46ceabe261ec7a0909cf36ee15ad7d96f10987c41b7b8613bd138e7ea29144` |
| `hgamer.lua` | 15124 | `be0602dec767bd5bc23721741b4c665c14b29c0b6542a3c7811695a73e1b5ac2` |

### Shiravune 的浏览器传输

调研基线中 [Windows](../../../apps/windows-flutter/lib/host/core_task.dart)与 [Decky](../../../apps/decky/backend/source_service.py) 只对 `patchmoe` ID 启动浏览器，随包 JS会识别页面状态、挑选语言表单并提交。

1. `patchmoe.lua` 识别验证页并决定何时请求浏览器传输。普通 HTTP 和浏览器两条路径共用 Lua 现有的语言表单选择、字段读取、POST 构造和结果解析。页面要求用户本人确认时，由 Lua 识别并向用户报告该状态。
2. Windows 和 Decky 只提供通用浏览器传输，在同一会话内逐次执行 Lua 指定的请求，每次把响应交回同一来源任务。页面脚本动态填写表单时，响应应向 Lua 提供页面实时的字段状态，供后续请求读取。导航前及后续请求、跳转和回传响应都按当前来源规则核对域名。取消沿用现有任务语义并结束浏览器会话。施工时结合现有浏览器路径核实所需最小请求与响应契约。
3. 双端打通后，**删除 `shiravune-browser.js`**。同时删除 Windows 和 Decky 的 JS 加载、资源声明及打包复制和清单引用，移除按 `patchmoe` ID 启动浏览器的宿主分支，以及仅服务旧 JS 或 ID 分支的测试和夹具。通用传输、来源权限、取消及需要本人确认的用例改为覆盖新路径。[Windows 资源声明](../../../apps/windows-flutter/pubspec.yaml) · [Decky 打包](../../../apps/decky/scripts/package-plugin.mjs) · [发行物清单](../../../scripts/package-artifacts.py)

### YOZAKURA 的详情地址

沿用现有 `page` 字段。`yozakura.lua` 从 WordPress 的 `post.link` 确定文章地址，由脚本保留文章标识 `p` 并决定是否清除跟踪参数。索引中的 `page` 用于请求详情，详情结果沿用同一文章地址。Core 继续校验 URL 和来源域名权限，保留 Lua 为文章地址选定的查询参数。删除 `canonical_url` 中按 `yozakuragames.com` 保留 `p` 的站点特判。[现有 YOZAKURA Lua](../../../crates/core/resources/source-scripts/yozakura.lua) · [URL 规范化](../../../crates/core/src/sources/canonical.rs)

| 核对位置 | 实施要求 |
| --- | --- |
| [索引归一化](../../../crates/core/src/sources/index.rs)和[更新](../../../crates/core/src/sources/update.rs) | `normalize_entry` 当前会重新规范化 `entry.page`。保留脚本选定的页面查询参数，检查索引去重及续跑页归一化是否再次清掉文章标识。 |
| [详情队列和请求](../../../crates/core/src/sources/matching.rs) | 核对详情入队、内嵌文章去重、分页、`resolve_page` 调用前和上游页面比较的规范化。同一篇文章贯穿这些步骤使用同一 URL 身份，实际请求带上各自的 `?p=`。 |
| [详情缓存](../../../crates/core/src/sources/store.rs) | 当前读写均从 `canonical_url(page)` 计算 `detail_key`。读写对同一文章得到相同键，`/?p=150` 与 `/?p=161` 使用不同键。 |
| [候选身份](../../../crates/core/src/sources/identity.rs)与[索引候选投影](../../../crates/core/src/sources/registry.rs) | 核对候选生成时对页面的再次规范化，保证 `source_record_id` 与候选身份使用同一个保留查询参数的来源页面。 |

验收使用 `/?p=150` 和 `/?p=161` 验证索引地址独立、两次实际 GET 带对应参数、详情缓存各自复用、候选各自关联正确文章。用原 ID 和新 ID 导入同一份 Lua，核对除来源身份外的结果一致。同时复核其它来源已有去重与缓存身份。改动范围以保留来源 Lua 选定的页面查询参数为准，下载 URL 和其它调用点继续沿用各自现有语义。

## 本体与 DLC 匹配

### 用户可见结果与职责

- **保留 `appinfo.vdf` 全量关联 DLC 发现**：继续从 `appcache/appinfo.vdf` 的 `extended.listofdlc` 获取本体关联的所有 DLC（不论本地是否已安装）及其 `appid` 与多语言名称（如 `1390150` → `夏日狂想曲+`），将本体名及其全部关联 DLC 名都作为同一游戏的候选匹配名；未安装的关联 DLC 依然参与补丁搜索并展示在本体游戏的「补丁选择」列表中，由用户自行选定。
- **DLC 补丁归类与绑定（已实现，实机待验收）**：当补丁候选是通过匹配关联 DLC 的名称或关联 DLC 的 AppID 命中时，直接将该候选判定为 `variant = dlc`，并自动绑定对应 DLC 的 AppID（`requires_dlc = [matched_dlc.appid]`），替代原先依赖标题是否包含 `+`、`dlc`、`plus` 等关键词猜测 `variant` 的做法；标题关键词仅在未命中已知 DLC 名称/AppID 时作为兜底。
- **本地已安装 DLC 状态检测与前端“适用条件”展示（已实现，实机待验收）**：在 `steam::scan` 解析游戏本体的 `steamapps/appmanifest_<appid>.acf` 时，顺带读取 `AppState.InstalledDepots` 下各 depot 的 `dlcappid`（及 `InstalledDLC`），识别哪些关联 DLC 在本地已实际勾选安装。本地安装状态仅用于前端适用条件提示，不筛除候选或阻断安装：
  - 若 `requires_dlc` 对应的 DLC 在本地 `appmanifest` 中已检测到安装：前端“适用条件”展示为已安装状态（例如 `适用条件：扩展 / DLC · 需要 DLC 夏日狂想曲+ (1390150)（已安装）`，无名称时为 `需要 DLC 1390150（已安装）`），不再显示“（拥有及安装状态待确认）”。
  - 若通过 `appinfo.vdf` 匹配到但本地 `appmanifest` 未检测到安装：保留候选展示，并明确标注未检测到安装（例如 `需要 DLC 夏日狂想曲+ (1390150)（未检测到安装）`），提醒用户核对后再安装。
- Core 提供 Steam 名称、AppID、关联 DLC 关系与本地已安装 DLC 状态，统一汇总与校验候选。来源 Lua 决定是否从网页提取 AppID、选哪个链接、是否采信、如何与 Steam 侧 AppID 比较。名称匹配可复用 core 的通用比较能力，网页 AppID 的取舍和纠错由单个来源负责。
- Kagura 和 Patchmoe 的目标路径可用名称匹配。侧会话抽查的 Kagura 国内本体与 DLC 补丁正文通过作品名和站内链接互指，样本中未见 Steam 商店链接。Patchmoe 的[生产解析](../../../crates/core/resources/source-scripts/patchmoe.lua)只读取标题、日期与下载列。当前没有仅标注 DLC AppID 的补丁页样本，该情形不列为本次必需交付。
- YOZAKURA 提供了网页 AppID 证据的实样。文章 161 的商店链接正确指向 `4370020`，另一作品的文章 150 也错指至 `4370020`，现有 Lua 会在来源端纠正为 `4376090`。继续由这里决定如何采信并与 Steam 侧 AppID 比较。两篇文章的 URL 处理见[详情地址方案](#yozakura-的详情地址)。

### 实现边界与既有路径

1. **Steam 关联 DLC 名称与本地已安装状态输入**：[appinfo.rs](../../../crates/core/src/appinfo.rs) 与 [steam_metadata.rs](../../../crates/core/src/steam_metadata.rs) 继续从 `appinfo.vdf` 读取本体 `extended.listofdlc` 的全量关联 DLC `appid` 与多语言名称并传入直接索引匹配、详情匹配及需要按名称查询的 Lua（`merged_hints` 不截断关联 DLC 名称；关系或名称变化时重算匹配缓存）。新增的本地安装状态检测在 [steam.rs](../../../crates/core/src/steam.rs) 的 `parse_manifest` 中顺带完成：解析本体 `steamapps/appmanifest_<appid>.acf` 时提取 `AppState.InstalledDepots` 下各 depot 的 `dlcappid`（及 `InstalledDLC`），随 `GameInstall` 扫描结果传递给双端前端。[现有匹配输入](../../../crates/core/src/sources/matching.rs)
2. **网页 AppID 决策与 DLC 归类绑定**：在来源匹配阶段把 Steam 事实交给 Lua，保留 API v1 现有脚本入口的读取兼容，删除了 `identity.rs` 对原始网页 AppID 不包含当前 AppID 的全局硬拒绝以及绑定 `kagura-cn`、`1227890`、`1390150` 的 DLC 特判；只有来源明确采信并比对的证据才参与 AppID 判断，保留最终候选的游戏身份、来源归属及 URL 校验。归类与绑定规则已落点在 [matching.rs](../../../crates/core/src/sources/matching.rs)、[identity.rs](../../../crates/core/src/sources/identity.rs) 与 [registry.rs](../../../crates/core/src/sources/registry.rs)：列表匹配（`select_entries` / `match_entry_rows` / `candidate_from_index_match`）与详情/补查匹配（`candidates_from_fact` / `build_candidates`）已实现识别候选是否由关联 DLC 名称或关联 DLC AppID 命中；若是，则直接设置 `variant = Variant::Dlc` 与 `requires_dlc = Some(vec![matched_dlc.appid])`，仅在未命中已知关联 DLC 时回退到 `identity::variant` 的标题关键词判断。[候选生成与站点特判](../../../crates/core/src/sources/identity.rs) · [Lua 执行入口](../../../crates/core/src/sources/adapters/script.rs)
3. **多候选投影与双端“适用条件”展示**：将名称命中的不同有效补丁全部投影到同一游戏，复用现有「补丁选择」及安装时针对已选候选的验证（已存在的相同下载归并逻辑照常使用）。双端前端（Windows [library_presentation.dart](../../../apps/windows-flutter/lib/presentation/library_presentation.dart) 与 Decky [discovery.ts](../../../apps/decky/src/discovery.ts)）的 `candidateConditions` 结合当前游戏的关联 DLC 名称与本地 `appmanifest` 已安装状态，将 `requires_dlc` 格式化为 `需要 DLC <名称> (<appid>)（已安装）` 或 `需要 DLC <名称> (<appid>)（未检测到安装）`（未知名称时退化为 `<appid>`），取代“（拥有及安装状态待确认）”。[列表匹配](../../../crates/core/src/sources/matching.rs) · [安装解析](../../../crates/core/src/installer/resolve.rs)

## 双端字段与保存

以下字段以 [RawSourceRule](../../../crates/core/src/sources/model.rs) 与 [validate_rule](../../../crates/core/src/sources/validate.rs) 为准。单端的导入、编辑、查看共享一份字段表单，页面与域名列表可按行输入，保留顺序。预填来自 `get_script_source.rule`，Lua 预填来自 `code`。只读页提供复制完整 Lua；用户也可直接复制仓库文件。编辑中失败保留已填字段和源码并显示现有 core 错误。

| 具名字段 | 合同与取值 |
| --- | --- |
| 来源 ID `id`、规则名称 `name` | 都必填。ID 小写英文字母开头，后续允许小写英文字母、数字和连字符，最多 48 字符。名称去首尾空白后须非空，原值最多 128 UTF-8 字节。编辑锁定 ID，卡片显示名的覆盖独立于规则名称。 |
| 角色 `role` | `publisher`（core 默认）或 `fallback_indexer`。前者可更新索引，后者按游戏补查。 |
| 仅发现 `discovery_only`、安装布局 `root_layout` | core 默认关闭仅发现。关闭时必须为 `publisher` 且布局为唯一值 `game_root`。开启时布局为空。`fallback_indexer` 必须仅发现。表单可按组合直接显示对应布局。 |
| 优先级 `priority`、启用 `enabled` | 优先级 core 默认 `100`，范围 `0..=1000`。`enabled` 是必填布尔值，core 无缺省值；计划新建表单默认勾选，编辑显示现有有效状态，启停沿用来源卡片。 |
| 起始页面 `pages`、页面域名 `page_hosts` | 都必填且至少一项，保留 `pages` 的路径、查询串和次序。现有 core 按 HTTPS 和页面域名白名单校验。 |
| 下载域名 `download_hosts` | core 缺省为空。可安装规则至少一项；仅发现的发行商允许为空；`fallback_indexer` 必须为空。 |
| 起始 URL `url`、语言 `language`、表格 ID `table_id` | 可选字符串，无全局默认。Kagura 可额外从 `url` 入队，HGamer 可用它替代 `pages[0]`，实际请求仍由宿主校验。自定义 Lua 按自身解析逻辑读取 `language` 和 `table_id`。 |
| 表格列 `columns` | 可选对象，提供时需要 `title`、`download`，另可设 `date`、`language`、`appid`。索引从零开始，按现有 `u8` 合同接受 `0..=255`。保留第 0 列和历史第 22 列用例，空字段与零值分别处理。 |
| Lua 源码 | 现有 Lua API v1。新建使用该版本，编辑沿用文档版本；摘要由 core 计算。adapter 使用现有隐式 `lua` 或显式 `lua`。版本、摘要与 adapter 无需用户填写。 |

选填值按文档回填，保留未设置、空列表、零值和现有显式字符串的语义。可选字符串允许清除为未设置，也允许保留或填写显式空串。打开后直接保存不得丢失或重置 `columns`、`table_id`、`language`、`url`。客户端只处理表单值到现有字段类型的转换，规则组合、域名、ID、编译及持久化错误由 core 判定。Windows 提交现有对象，Decky 在 RPC 边界序列化给现有桥接，复用 v5 DTO、API v1 和设置 schema 3。

移除双端“查看脚本示例”和维护的示例常量。将[用户指南](../../custom-subscription-sources.md)改为从内置查看、复制 Lua、填写表单并更新或匹配的实际操作，链接到随包文件。现有测试依赖的示例可保留为测试历史或夹具，相关 UI 测试和复制导入验收使用真实随包文件。

## 施工与定向验收

| 顺序 | 实施 | 验收 |
| --- | --- | --- |
| 1 | 机械落盘完整 Lua，切换到单文件嵌入，沿用五源适配器绑定 | 扩充 `builtin_scripts` 遍历五源，比对仓库文件和两条 RPC 读取路径返回的原文与摘要，并核对执行使用的脚本描述和只读无写入。运行 `builtin_script_migration`、`builtin_script_regression`、`source_acceptance` 等既有夹具，覆盖 Shiravune 条件更新与表单请求、Kagura 国内列表/分类/详情和海外多入口/分页/详情、YOZAKURA 索引与镜像、HGamer 按游戏补查。删除只验证旧表格解析器的用例，仍验证通用索引、匹配和安装合同的用例改用现有 Lua 测试夹具。 |
| 2 | 退役声明式表格来源，回归旧状态与导入，再接双端表单 | 用同时含旧表格来源、内置来源和 Lua 自定义来源的状态验证升级。确认旧表格条目在只读视图与可写流程均退出来源列表，旧缓存无法提供候选或安装选择，其余来源及设置可读写。回归早期同 ID Lua 覆盖、同 ID 替换、保存失败及真实脚本变更后的缓存失效。沿用 `script_source_import`、`script_source_pipeline`、`script_source_fallback` 与 CLI `script_sources_smoke`。Windows 在 `script_source_editor_test.dart` / `script_source_native_test.dart`、Decky 在 `script-source-editor.test.ts` / 桥接与来源服务用例检查具名字段、请求体、只读、错误后草稿、选填字段往返。 |
| 3 | 核实 Steam 本体/DLC 数据入口，实现名称、AppID、DLC 归类绑定、本地安装状态检测与适用条件展示 | 覆盖 `appinfo.vdf` 多语言关联 DLC 名称与全量 `extended.listofdlc` 读取，以《夏日狂想曲：乡间的难忘回忆》与《夏日狂想曲+》等验证本体及关联 DLC 各自命中并同时出现在本体的「补丁选择」，以及 Patchmoe 列表两个名称命中两条不同下载均进入候选。关联 DLC 归类绑定、`appmanifest_<appid>.acf` 本地已安装 DLC 检测及双端适用条件展示已完成代码实现与双端定向自动化验收（待实机与真实 Steam 库展示验收）。 |
| 4 | 将网页身份和详情地址的站点判断留在来源 Lua，接通 Shiravune 通用传输并删除旧 JS | YOZAKURA 用文章 161 的正确链接及文章 150 的已知错链作对照，分别匹配 `4370020` 和 `4376090`，验证索引、实际 GET、详情缓存与候选四处均保留各自的 `?p=`。用原 ID 和新 ID 比较同一 Lua 的候选与证据，来源 ID 及派生标识随副本变化。双端使用受控页面动态更新表单字段，验证 Lua 选择三个语言表单，POST 读取当前字段值，逐次请求复用同一浏览器会话并最终发布索引。分别确认不允许的域名被拒绝、取消会结束会话、需要本人确认时返回该状态。核对两端产物与加载路径均已移除旧 JS。 |
| 5 | 以实际发行形态验证空内置构建及双端复制导入 | 用 `sources=[]`、`bundled_scripts={}` 构建 Windows 和 Decky，启动干净状态并确认初始来源为空。分别从仓库复制五份真实 Lua，参考仓库规则在普通导入表单填写原来源 ID 和规则，保存后重新打开核对源码与字段。四个发行商分别更新并验证候选和详情（按入口适用），HGamer 在没有发行商可信候选的游戏上验证按游戏补查，Shiravune 跑过受控浏览器路径。再以新 ID 导入两份 Kagura，核对同一规则与 Lua 得到相同匹配结果，包括本体与 DLC 的同屏选择。用含内置来源的普通构建另验只读复制与实际内嵌源码一致。记录 Windows 粘贴和 Deck 游戏模式剪贴板、软键盘及手柄焦点。 |

Rust 修改按仓库规则执行 `cargo fmt --all -- --check`、受影响包 Clippy（`-D warnings`）和定向测试；Decky 执行类型检查与对应前后端测试，Windows 执行对应 Flutter 测试。真实站点受阻时记录具体诊断，并保留离线夹具结果作为确定性证据，实机项继续待验收。本文保留调研基线与验收条件，实施与验证结果记录在[施工记录](progress.md)。

## 过度设计审阅

**方案静态审阅完成，已补充关联 DLC 归类绑定与本地安装状态检测审阅结论。** 按 `overengineering-review` 静态走查 `main@6c83576` 的构建 → 摘要与分派 → 双端表单与设置 → 索引、详情和候选投影。下列结论针对本轮方案。

- 以 `sources.json` 的单文件映射复用现有分派，空内置验收直接清空现有配置。机械拆包阶段保留脚本原字节和 adapter 以维持规则哈希。后续脚本和匹配语义有意变化时重新验证缓存。旧表格解析器和两条映射一起删除，旧条目只在读取设置及既有写入事务中剔除。
- 具名表单只转换现有 DTO，沿用原子写入和 core 校验。复制入口使用随包 Lua。组合 JSON 文件、额外模板副本、第二套规则 schema 或迁移服务均超出当前路径所需。
- DLC 匹配、归类与本地状态检测均基于已有真实数据边界：全量关联 DLC 发现继续复用 `appinfo.vdf`，本地已安装 DLC 状态顺带从 `steam::scan` 已读取的 `appmanifest_<appid>.acf` 提取，无额外磁盘 I/O 或外部接口；`variant = dlc` 与 `requires_dlc` 直接由命中关联 DLC 名称/AppID 的事实派生，替代按内置 ID 的硬编码特判与纯标题关键词臆测；未检测到安装仅用于前端“适用条件”提醒，不筛除候选、不自动选补丁也不阻断安装，仅有 DLC AppID 的补丁页处理无本次需求。
- YOZAKURA 复用 `page` 表达文章地址。索引、详情访问、缓存键和候选身份需一并保持 Lua 选定的参数，这些环节目前都可能再次调用会删查询参数的规范化函数。现有规则、接口和缓存结构可承载该地址。
- Shiravune 复用双端已有浏览器传输能力，站点判断和表单请求集中到 `patchmoe.lua`。逐次传输和实时字段读取直接服务于 Lua 根据当前页面状态构造下一次请求。沿用任务取消和来源域名约束，删除旧 JS 与宿主特判后再验收无内置注册的导入路径。待核实双端 Steam 关系读取、最小 Lua 匹配接口与浏览器请求回传方式，实施后的表单、空内置构建和实机均需验收。
