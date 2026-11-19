# 单文件自描述 Lua 与面向 AI 的极简脚本导入

状态：**已实现（本地自动化验证通过，Windows / Steam Deck 实机待验收）**。施工记录见 [progress.md](progress.md)。开发分支：`feature/simplified-script-source-import`（基于 `main@9289242` 创建）。

## 1. 背景与现状问题

当前自定义订阅源导入弹窗要求用户除粘贴 Lua 源码外，还需手动填写或切换十余个表单字段（来源 ID、规则名称、角色、仅发现、优先级、启用、起始页面、页面域名、下载域名、起始 URL、语言、表格 ID、表格列），对普通用户门槛过高，也不利于配合 AI 编码工具实现“生成即导入”。

核查 [model.rs](../../../crates/core/src/sources/model.rs)、[validate.rs](../../../crates/core/src/sources/validate.rs)、[script_source_editor.dart](../../../apps/windows-flutter/lib/pages/script_source_editor.dart) 与 [script-source-editor.tsx](../../../apps/decky/src/script-source-editor.tsx) 后，现有 15 个规则/表单字段的真实职责与冗余来源如下表所示：

| 分类 | 现状字段 | 代码核查事实 | 简化方案 |
| --- | --- | --- | --- |
| **历史废弃（直接删除）** | `columns`、`table_id`、`url`、`language`、`root_layout` | `columns`/`table_id` 为已退役声明式 `table_html` 遗留，生产路径 0 处读取；5 个内置源均未配置 `url` 或 `language`（仅 3 份内置 Lua 中各含 1 行 `rule.url` 兼容，以及 [resolve.rs](../../../crates/core/src/installer/resolve.rs) 与 [exe_resolve.rs](../../../crates/core/src/installer/exe_resolve.rs) 各有 1 处 `rule.language` 兜底）；安装器仅支持 `game_root` 一种布局，不因 `root_layout` 分支 | 全部从规则模型、校验与双端表单移除；清理内置脚本中 3 处 `rule.url` 兼容、安装器 `rule.language` 兜底及对应旧测试夹具 |
| **宿主维护（用户零填写）** | `id`、`name`、`enabled` | `id` 仅作内部身份键与存储键；`settings.json` 已有独立的本地名称缓存 `names` 与启停表 `enabled`，表单里的“规则名称”与卡片改名形成双轨；新建脚本通常立即使用，启停已在卡片直接控制 | `id` 由 core 自动分配并锁定；显示名称统一收归本地 `settings.names` 维护（脱离 `rule_fingerprint`，改名不失效缓存）；新建默认启用，启停沿用来源卡片 |
| **脚本自描述（内聚到 Lua `meta`）** | `role`、`pages`、`page_hosts`、`download_hosts`、`priority`、`discovery_only` | 这些字段本质上是目标站点的固有解析属性与宿主访问边界；同时 `discovery_only` 命名已经失真（`kagura-cn`、`kagura-en`、`yozakura` 均为 `discovery_only: true` 却全部支持一键下载，真正决定能否直链下载的是 `download_hosts`）；[identity.rs](../../../crates/core/src/sources/identity.rs) 仅用 `!rule.discovery_only` 判断是否启用纯 Steam 标题总表信任与名称分割 | 全部收敛进 Lua 脚本导出的静态 `meta` 表；`page_hosts` 可由 `pages` 自动推导、`role` 可按导出函数自动推断；用含义明确的可选布尔项 `official_title_match`（默认 `false`）替代 `discovery_only` |
| **内部固定（无需展示）** | `adapter` | 用户导入统一为 `"lua"`；内置规则仅保留历史 adapter 映射以定位随包脚本 | 保持内部字段语义，不暴露给用户或导入参数 |

## 2. 目标、范围与产品决策

### 2.1 目标工作流

将自定义订阅源构建升级为“面向 AI + 零配置粘贴”闭环：

1. 用户在 PatchHub 双端（Windows / Decky）点击 **导入脚本来源** 中的 **复制 AI 提示词**，将其发送给任意 AI 编码 Agent 工具；
2. AI 根据精简提示词先拉取仓库中的权威规范 [docs/custom-subscription-sources.md](../../custom-subscription-sources.md)，再调用提问工具向用户确认 **来源类型（索引源 / 补查源）** 与 **订阅源网址**（以及可选的站点名或特殊访问说明）；
3. AI 实地探查网页结构、校验下载域名与 Steam AppID 证据，严格按规范文档生成符合单文件契约的自包含 Lua 脚本；
4. 用户仅需复制粘贴这一段 Lua 源码，点击 **保存并导入** 即可完成全部配置。

### 2.2 产品决策记录

- **短提示词 + 仓库权威规范单一真相源**：客户端复制的 AI 提示词仅保留精炼的 Agent 执行流程指令（拉取规范 → 提问收集参数 → 实地探查网页 → 输出单文件 Lua 与域名核对摘要），将具体的 `meta` 字段表、Lua 入口契约、`ctx` API 速查、校验红线与参考示例全部外链指向 GitHub 仓库的 [docs/custom-subscription-sources.md](../../custom-subscription-sources.md)（Raw 地址：`https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/docs/custom-subscription-sources.md`，网页地址：`https://github.com/Mabinogi-Cichol/patchhub/blob/main/docs/custom-subscription-sources.md`）。
- **将 [docs/custom-subscription-sources.md](../../custom-subscription-sources.md) 纳入核心重构范围**：该文档在本 Feature 中重新定位为 **面向人类与 AI Agent 双重消费的权威单文件 Lua 规范**，彻底消除客户端内置长提示词与仓库文档双份维护、内容漂移及不发版无法更新规范的问题。
- **一步粘贴直接导入**：不额外增设“解析预览”前置步骤或多余 RPC；粘贴 Lua 后直接点击 **保存并导入**，由 core 一次完成沙箱解析、校验、分配 ID、写入默认本地名称与持久化，导入后可在来源卡片展开详情中核对生效的 `page_hosts` 与 `download_hosts`。
- **完整闭环一次性交付**：本功能在同一开发分支内连贯完成“核心规则与存储精简 + 单文件 `meta` 契约 + 精简 AI 提示词内置与权威规范重构 + 双端极简界面”，不拆分成过渡半成品。
- **遵守宿主与脚本职责边界**：严格遵循 [AGENTS.md](../../../AGENTS.md#订阅源脚本与应用宿主的职责) 与空内置可恢复契约，所有站点行为继续完全保留在单文件 Lua 内，清空内置来源后仅凭粘贴 5 份内置 `.lua` 原文（零手填表单字段）即可恢复全部内置来源行为。

## 3. 核心数据模型与单文件 Lua 契约

### 3.1 单文件 Lua `meta` 规范

每一份完整 Lua 模块除导出入口函数（`collect_index` / `resolve_detail` / `match_appid` / `discover`）外，声明一张静态只读配置表 `meta`：

```lua
return {
    meta = {
        name = "示例补丁站",                              -- 可选：导入时的默认显示名（省略则取首个 page_hosts）
        role = "publisher",                              -- 可选："publisher" 或 "fallback_indexer"，省略时按导出函数推断
        pages = { "https://patches.example.com/list" },  -- 必填：起始 HTTPS 页面列表
        page_hosts = { "patches.example.com" },          -- 可选：允许访问的页面域名白名单（省略时自动从 pages 提取去重）
        download_hosts = { "dl.example.com" },           -- 可选：允许托管下载的域名白名单（默认为空；补查源须为空）
        -- priority = 100,                               -- 可选：同条件候选排序优先级（0..=1000，默认 100）
        -- official_title_match = false,                 -- 可选：默认 false；仅类似 Shiravune 的官方 Steam 标题总表开启
    },

    collect_index = function(ctx, rule, previous)
        -- ...
    end,
}
```

字段约束与推导规则如下：

| `meta` 字段 | 类型 | 必填与默认值 | 校验与推导规则 |
| --- | --- | --- | --- |
| `name` | `string` | 可选；默认取 `page_hosts[1]` | 仅用于新建导入时初始化本地显示名称；去首尾空白后非空、最多 128 字节（缺省回退首个域名） |
| `role` | `string` | 可选；按导出函数自动推断 | 仅允许 `"publisher"` 或 `"fallback_indexer"`；若导出 `collect_index` 且未导出 `discover` 则推断 `"publisher"`，反之推断 `"fallback_indexer"`；同时导出或均未导出且未显式声明时报错；显式声明时必须与对应必需入口函数匹配 |
| `pages` | `string[]` | **必填**，至少 1 项 | 保留原始顺序与查询参数；每项必须通过 HTTPS 与 `page_hosts` 校验 |
| `page_hosts` | `string[]` | 可选；省略或为空时由 `pages` 推导 | 自动推导时提取每个 `page` 的小写 hostname 并保序去重；所有域名须通过 [validate.rs](../../../crates/core/src/sources/validate.rs) 的 `host()` 合法性检查（禁止 IP、`.local`、`localhost`）并覆盖 `pages` 中全部主机 |
| `download_hosts` | `string[]` | 可选；默认 `[]` | 每项须通过 `host()` 检查；`role == "fallback_indexer"` 时必须为空；`collect_index` 返回的直链一步到位条目（`current_step == total_steps`）若存在镜像，仍按 `download_hosts` 校验直链下载权限（非空即允许直链校验，不再由 `!discovery_only` 反推） |
| `priority` | `integer` | 可选；默认 `100` | 允许范围 `0..=1000` |
| `official_title_match` | `boolean` | 可选；默认 `false` | 替代原 `!discovery_only` 在 [identity.rs](../../../crates/core/src/sources/identity.rs) 中仅剩的语义：控制无网页 Steam AppID 时是否按官方标题（及 `split_mode`）升级为 `MatchBasis::OfficialTitle` 并附加 `"steam"` 平台标识 |

### 3.2 精简后的 `SourceRule` 与 `SourceSummary`

- **`SourceRule`**（[model.rs](../../../crates/core/src/sources/model.rs)）：
  - 移除 `name`、`columns`、`table_id`、`url`、`language`、`root_layout`、`discovery_only`。
  - 保留并精简为：`id`、`adapter`、`role`、`official_title_match`、`priority`、`enabled`、`pages`、`page_hosts`、`download_hosts`、`script`。
  - 注：执行 Lua 回调（`collect_index(ctx, rule, ...)`、`resolve_detail(ctx, rule, ...)`、`discover(ctx, rule, ...)`）时传给脚本的 `rule` 为精简后的 `SourceRule` 序列化对象。
  - 若有基于旧 `DISCOVERY_ADAPTERS`（[build.rs](../../../crates/core/build.rs)、[mod.rs](../../../crates/core/src/sources/mod.rs)）的硬编码列表判断，随 `discovery_only` 一并清理。
- **`SourceSummary`**：
  - 将 `name: String` 提升为 `SourceSummary` 顶层显式字段（而非依赖扁平化的 `rule.name`），[list_sources](../../../crates/core/src/sources/registry.rs) / `list_game_sources` 对外 JSON 结构仍保留顶层 `"name"` 字段，Windows 与 Decky 卡片、游戏订阅页无感兼容。
  - [registry.rs](../../../crates/core/src/sources/registry.rs) 中的 `source_match_from_candidate` 不再输出无意义的 `root_layout` 或由统一默认 `"game_root"` / `None`（按实际消费需求精简 `SourceMatch`）。

### 3.3 自动 ID 维护与本地名称缓存

- **自动 ID 生成**（[registry.rs](../../../crates/core/src/sources/registry.rs)）：
  - 内置来源保留固定 ID：`patchmoe`、`kagura-cn`、`kagura-en`、`yozakura`、`hgamer`。
  - 新建自定义来源（`source_id` 为 `None`）时，core 根据 `page_hosts[0]` 规范化生成满足 `valid_source_id`（`[a-z][a-z0-9-]{0,47}`）的 slug（例如 `patches.example.com` → `patches-example-com`，若首字符非小写字母则前置 `s-`，超长按字节截断并去除尾连字符）；若该 ID 与现有内置或已添加来源冲突，自动追加 `-2`、`-3` 等递增后缀直到唯一。
  - 编辑替换既有自定义来源（`source_id` 为 `Some(id)`）时，永久锁定原 `source_id`；若传入内置 ID 且不在 `settings.added` 覆盖中，继续返回 `source_builtin_readonly`。
- **显示名称独立本地缓存**：
  - 复用 `settings.json` 中的 `names: BTreeMap<String, String>` 与既有的 `set_source_name` 接口：
    - 内置源默认显示名称从其随包 Lua `meta.name` 派生，若 `settings.names` 中存在用户覆盖则优先使用用户名称；
    - 新建自定义源时，core 将最终选定的默认名称（`meta.name` 或 `page_hosts[0]`）写入 `settings.names[source_id]`；
    - 编辑替换自定义源时，保留 `settings.names[source_id]` 中的用户名称（仅当历史数据缺失该键时补写 `meta.name`）；
    - 用户通过卡片 **保存名称** 修改名称仅更新 `settings.names`，由于 `name` 不再属于 `SourceRule`，改名彻底脱离 `rule_fingerprint`，不再触发任何缓存或匹配失效；
    - 删除自定义源时，`SettingsFile::clear_source_overrides` 同步清除 `settings.names[source_id]`。

### 3.4 内置五源与 `sources.json` 统一收口

- 将 `meta` 表直接写入仓库 5 份内置脚本（[patchmoe.lua](../../../crates/core/resources/source-scripts/patchmoe.lua)、[kagura-cn.lua](../../../crates/core/resources/source-scripts/kagura-cn.lua)、[kagura-en.lua](../../../crates/core/resources/source-scripts/kagura-en.lua)、[yozakura.lua](../../../crates/core/resources/source-scripts/yozakura.lua)、[hgamer.lua](../../../crates/core/resources/source-scripts/hgamer.lua)），并清理 [kagura-cn.lua](../../../crates/core/resources/source-scripts/kagura-cn.lua)、[kagura-en.lua](../../../crates/core/resources/source-scripts/kagura-en.lua)、[hgamer.lua](../../../crates/core/resources/source-scripts/hgamer.lua) 中各自 1 处多余的 `rule.url` 分支。
- 其中 [patchmoe.lua](../../../crates/core/resources/source-scripts/patchmoe.lua) 的 `meta` 显式声明 `official_title_match = true`，其余四源默认 `false`。
- [sources.json](../../../crates/core/resources/sources.json) 瘦身为内置注册清单（`schema_version: 2`、`bundled_scripts` 映射及每项仅保留 `id`、`adapter`、`enabled`）；[build.rs](../../../crates/core/build.rs) / [builtin.rs](../../../crates/core/src/sources/builtin.rs) 在编译或加载期直接由各内置 Lua 的 `meta` 派生 `SourceRule` 与默认显示名称，消除两份配置重复与漂移风险。

## 4. 导入期沙箱解析与安全校验边界

在 [script.rs](../../../crates/core/src/sources/script.rs)（配合 [adapters/script.rs](../../../crates/core/src/sources/adapters/script.rs)）新增只读元数据提取函数 `parse_script(source_label, api_version, code)`：

1. **零权限只读 Lua 5.4 沙箱**：
   - 使用与运行期相同的纯计算安全子集（`_VERSION`、`assert`、`error`、`ipairs`、`next`、`pairs`、`select`、`tonumber`、`tostring`、`type`、`string`（无 `string.dump`）、`table`、`math`、`utf8`、`pcall`、`xpcall`），**不注入 `ctx` 网络、DOM 或任何 IO 函数**。
2. **顶层初始化指令限流**：
   - 通过 `mlua` 的 `HookTriggers::every_nth_instruction` 为**模块加载并返回顶层表**这一同步导入步骤设置指令步数保护，防止恶意或笔误的顶层死循环卡死同步 RPC / UI 线程；该限制仅用于 `meta` 提取阶段，运行期 `collect_index` / `resolve_detail` / `discover` 仍保持不设指令上限、仅按取消信号退出的策略。
3. **结构与入口完整性校验**：
   - 校验模块必须返回一个表；
   - 提取 `exports.meta` 并反序列化、校验字段类型、拒绝未知键、规范化 `pages`/`page_hosts`/`download_hosts`/`priority`/`official_title_match`；
   - 依据 `role`（或自动推断出的 `role`）验证对应入口函数（`collect_index` 或 `discover`）存在且类型为 Lua `function`，并在此时校验可选项 `resolve_detail` / `match_appid` 若存在也必须为 `function`；
   - 计算源码规范 `sha256`，生成 `ScriptDescriptor`。
4. **安全边界保留**：
   - 继续保留全部既有访问安全约束：`pages` 仅允许 `https://`、无内嵌凭据、默认端口、主机在 `page_hosts` 内；`page_hosts` 与 `download_hosts` 均拒绝 IP 字面量、`localhost`、`.local` 及非法域名。

## 5. 旧版设置与缓存兼容策略

- **设置 Schema 升级**（[mod.rs](../../../crates/core/src/sources/mod.rs)、[store.rs](../../../crates/core/src/sources/store.rs)）：
  - 将 `SOURCE_SETTINGS_SCHEMA_VERSION` 由 `3` 升级为 `4`（`index/`、`details/`、`matches/` 的磁盘缓存 schema 保持不变）。
  - 读取 `schema_version` 为 `1..=3` 的旧 `settings.json` 时，通过宽松迁移反序列化：
    1. 剥离 `added` 条目中的废弃字段（`columns`、`table_id`、`url`、`language`、`root_layout`）；
    2. 将原 `!discovery_only && role == "publisher"` 映射为 `official_title_match = true`（其余为 `false`）；
    3. 若旧条目带有非空 `name` 且 `settings.names` 中尚未为该 `id` 设置覆盖名，则将该 `name` 迁移写入 `settings.names[id]`；
    4. 升级后写回 `schema_version = 4`（只读快照路径 [snapshot.rs](../../../crates/core/src/sources/snapshot.rs) / `open_readonly` 在内存中执行同一转换，不写磁盘）。
- **历史自定义脚本兼容**：
  - 升级前已持久化在 `settings.added` + `settings.script_sources` 中的旧自定义 Lua（代码内不含 `meta`）直接沿用已迁移的 `SourceRule` 照常运行、更新与匹配；
  - 当用户对该来源执行 **编辑脚本 → 保存并替换** 时，要求源码补齐 `meta` 块（保存校验不通过时返回明确错误码并保留用户草稿），不在后端做脆弱的 Lua AST 或文本自动改写。
- **规则指纹与缓存自然刷新**：
  - 内置 5 源及已升级自定义源因精简规则结构及新增 `meta` 文本导致 `rule_fingerprint` 与 `sha256` 更新，[index.rs](../../../crates/core/src/sources/index.rs) 与 [matching.rs](../../../crates/core/src/sources/matching.rs) 既有的 `parser_fingerprint` 机制会自动将旧索引与旧匹配判定为不兼容并等待刷新，无需额外清理代码或提升 `SOURCE_INDEX_SCHEMA_VERSION`。

## 6. 协议契约、AI 提示词与双端极简界面

### 6.1 协议与桥接精简

涉及 [crates/protocol/src/lib.rs](../../../crates/protocol/src/lib.rs)、[runtime.rs](../../../crates/cli/src/runtime.rs)、[main.py](../../../apps/decky/main.py) 与 [source_service.py](../../../apps/decky/backend/source_service.py)（改后重新运行 `patchhub-contract-gen` 同步 `packages/contracts/generated/`）：

- **`import_script_source`**：
  - 请求参数 `ImportScriptSourceParams` 精简为：`{ source_id?: string | null, api_version?: u32, code: string }`（`api_version` 默认 `1`；`source_id` 为 `null`/省略表示新建并自动分配 ID；提供 `source_id` 表示编辑并替换对应自定义来源）。
  - 返回 `ImportedScriptSource` / `AddSourceResult`：`{ rule: SourceRule, name: string, settings_revision: number }`（其中 `rule.id` 为实际分配或锁定的 ID，`name` 为生效的本机显示名称）。
- **`get_script_source`**：
  - 返回 `ScriptSourceDocument`：`{ source_id: string, name: string, rule: SourceRule, api_version: number, code: string, sha256: string }`，直接携带解析后的当前本地名称 `name` 与精简规则供编辑/只读弹窗展示摘要。
- **AI 提示词读取接口**：
  - 新增单一权威提示词资源 `crates/core/resources/ai-source-prompt.md`（编译内嵌到 core/CLI）；
  - 新增只读协议方法 `get_script_source_prompt`（返回 `{ prompt: string }`），由 Windows 与 Decky 直接读取并复制到剪贴板（Decky 通过既有桥接暴露对应方法），保证唯一真实来源、双端零重复文案。

### 6.2 面向 AI 的提示词设计（`crates/core/resources/ai-source-prompt.md`）

为大幅压缩复制体积并保持规范单一来源，内置提示词仅保留**约 15–25 行的极简四步流程指令**，不重复内嵌长篇 API 表格与示例，确保主流 AI Agent（Antigravity、Claude Code、Codex、Cursor 等）遵循以下工作流：

1. **读取仓库权威规范**：先拉取并遵循 GitHub 仓库的 [docs/custom-subscription-sources.md](../../custom-subscription-sources.md)：
   - 优先读取 Raw 地址：`https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/docs/custom-subscription-sources.md`
   - 备用网页地址：`https://github.com/Mabinogi-Cichol/patchhub/blob/main/docs/custom-subscription-sources.md`
   - 若当前运行环境无法联网或抓取失败，提示用户提供该文档内容或本地路径后再继续编写。
2. **强制首轮提问**：优先调用交互式提问工具（如 `ask_question` / `AskUserQuestion`；若当前环境无提问工具则首轮直接发问并等待回复），向用户确认两项必答与一项可选信息：
   - ① 订阅源类型：**索引源（`publisher`）**（站内有完整补丁列表/目录/RSS/REST API）还是 **补查源（`fallback_indexer`）**（按游戏名/关键词站内搜索）；
   - ② 订阅源入口网址（完整 `https://...` URL）；
   - ③ 可选：期望的显示名称或站点特殊说明（如是否需登录、年龄确认、语言筛选等）。
3. **实地探查网页与编写**：用工具抓取/分析目标站点的真实 DOM、分页、Steam AppID 证据与下载域名，严格按文档中的单文件自描述 `meta` 与 Lua 契约编写（严禁凭猜测编造 CSS 选择器或域名）。
4. **交付输出格式**：最终仅输出一份可直接整段复制粘贴导入的完整 ` ```lua ... ``` ` 代码块，并在代码块外简要列出脚本声明的 `page_hosts` 与 `download_hosts` 供用户核对（无任何额外手工配置表）。

### 6.3 双端极简界面与权威规范文档重构

- **Windows（[script_source_editor.dart](../../../apps/windows-flutter/lib/pages/script_source_editor.dart)、[source_pages.dart](../../../apps/windows-flutter/lib/pages/source_pages.dart)）与 Decky（[script-source-editor.tsx](../../../apps/decky/src/script-source-editor.tsx)、[index.tsx](../../../apps/decky/src/index.tsx)）**：
  - **新建导入（`target == null`）**：移除全部规则表单控件；界面仅保留：
    1. 简短安全与使用提示 + **复制 AI 提示词** 按钮（点击拉取 `get_script_source_prompt` 写入系统剪贴板并反馈“已复制 AI 提示词”）；
    2. 唯一的 **Lua 源码** 多行等宽文本框；
    3. 错误提示区域（原样呈现 core 返回的格式/语法/域名校验错误并保留草稿）与 **取消 / 保存并导入** 按钮。
  - **编辑自定义脚本（`target != null && !readOnly`）**：顶部只读展示当前来源摘要（来源 ID、显示名、来源类型、允许访问域名、允许下载域名）与唯一的 **Lua 源码** 文本框，按钮为 **取消 / 保存并替换 \<id\>**。
  - **查看内置脚本（`readOnly == true`）**：顶部只读展示内置来源摘要与域名边界，提供完整只读 **Lua 源码** 与 **复制 Lua 源码** 按钮。
  - **全局来源卡片展开详情**：补充展示当前来源的允许访问域名（`page_hosts`）与允许下载域名（`download_hosts`），让安全边界一眼可见；名称修改继续沿用卡片内的 **来源名称 + 保存名称**。
- **权威规范文档重构（[custom-subscription-sources.md](../../custom-subscription-sources.md)，纳入本次核心交付）**：
  - 彻底清除所有旧版 JSON 规则、多字段导入表单及废弃字段（`id`、`root_layout`、`discovery_only`、`table_id`、`columns`、`url`、`language`）说明；
  - 将文档重整为面向人类与 AI Agent 双重消费的单一权威规范，结构包含：
    1. 极简导入流程与面向 AI 提示词生成入口说明；
    2. 单文件自描述 `meta` 表字段、默认值与自动推导规则（`name`/`role`/`pages`/`page_hosts`/`download_hosts`/`priority`/`official_title_match`）；
    3. Lua 模块入口契约（`collect_index`、`resolve_detail` Step 推进不变量与断点续传缓存、`match_appid`、`discover`）；
    4. `SourceIndexEntry` 与 `SourceDetail` 结构及校验红线；
    5. 全量 `ctx` API 速查与 Lua 5.4 沙箱限制；
    6. 三类可直接参考的单文件 Lua 示例（一步到位索引源、多步按需索引源、补查源骨架）与五份内置生产脚本链接；
    7. 调试检查要点与“本地游戏数据缓存”清理说明（整合工作区既有的有效修订）。

## 7. 分阶段实施拆分与验收标准

| 阶段 | 实施范围 | 定向自动化验证 |
| --- | --- | --- |
| **P0：分支与立项文档** | 基于 `main@9289242` 创建 `feature/simplified-script-source-import`；新建本 `design.md` 与 [progress.md](progress.md)；更新 [功能索引](../README.md) | 核对分支、工作区状态与文档相对链接 |
| **P1：核心规则精简、Lua `meta` 解析、自动 ID 与名称缓存** | 精简 `SourceRule`/`SourceSummary` 与 `validate_rule`；在 `script.rs` 实现只读沙箱 `meta` 解析与顶层初始化限流；在 `registry.rs` 实现自动 slug ID 生成与 `settings.names` 解耦；在 `store.rs` 实现 schema `<= 3` 向 `4` 迁移；5 份内置 Lua 内嵌 `meta` 并瘦身 `sources.json`/`build.rs`；清理 `identity.rs`/`resolve.rs`/`exe_resolve.rs` 旧字段引用 | `cargo test -p patchhub-core --lib sources:: -- --skip real_transport` 及集成测试 `script_source_import`、`script_source_runtime`、`script_source_pipeline`、`script_source_fallback`、`source_data_v2`、`source_update`、`source_matching`、`source_acceptance`、`builtin_scripts`、`builtin_script_migration`、`builtin_script_regression`、`projection`、`installer_service` |
| **P2：协议契约、CLI 与 Decky 后端** | 精简 `crates/protocol` 中 `ImportScriptSourceParams`/`ScriptSourceDocument` 并新增 `get_script_source_prompt`；运行 `patchhub-contract-gen` 更新 `packages/contracts/generated/`；更新 CLI `runtime.rs` 与 Decky `main.py`/`source_service.py` | `cargo test -p patchhub-protocol`、`cargo test -p patchhub-cli --bins --test script_sources_smoke --test projection_smoke`；Decky Python 单元测试（`test_bridge.py`、`test_source_service.py`、`test_source_tasks.py`） |
| **P3：精简 AI 提示词与权威规范文档重构** | 新增极简版 `crates/core/resources/ai-source-prompt.md`（指向 GitHub 仓库 `docs/custom-subscription-sources.md`）并嵌入构建；重构 [custom-subscription-sources.md](../../custom-subscription-sources.md) 为面向人类与 AI Agent 的单文件自描述 Lua 权威规范（吸收并整合工作区已有关于 `priority`、多步 `resolve_detail` 与缓存清理的有效修改） | 核对提示词中的 GitHub Raw/网页 URL 与仓库实际路径完全一致；核对 `docs/custom-subscription-sources.md` 中的字段、入口签名、`ctx` API 与核心运行时 100% 一致，且文内示例均可通过核心导入解析校验 |
| **P4：Windows 与 Decky 双端极简界面** | 重构 Windows `ScriptSourceEditor` / `GlobalSourcesPage` 与 Decky `ScriptSourceEditor` / `SourcesPage`，接入「复制 AI 提示词」、纯 Lua 导入/编辑/查看及卡片域名展示 | Windows `flutter test`（重点覆盖 `script_source_editor_test.dart`、`empty_builtin_import_test.dart` 及来源卡片测试）；Decky `npm run typecheck` / 前端单测（`script-source-editor.test.ts`、`game-manager.test.ts`） |
| **P5：空内置恢复、静态检查与过度设计复核** | 验证空内置构建下仅粘贴 5 份 `.lua` 原文（零手填字段）恢复全量来源与行为；验证旧 `settings.json` 升级保留自定义来源与本地名称；执行 `cargo fmt --all -- --check` 与受影响包 Clippy（`-D warnings`）；按 [overengineering-review](../../../.agents/skills/overengineering-review/SKILL.md) 完成实现后代码审阅并回填 [progress.md](progress.md) | 全部定向自动化 + 静态检查 + 过度设计审阅闭环；记录受沙箱限制用例及 Windows / Steam Deck 实机待验收项 |

## 8. 方案级过度设计审阅结论

已按 [overengineering-review](../../../.agents/skills/overengineering-review/SKILL.md) 对本设计方案完成预审，结论为 **未发现多余复杂度**，关键边界取舍如下：

1. **彻底下线旧多字段表单，不维持双轨导入**：直接以单文件自包含 Lua 取代旧的 13 项表单输入，不在 UI 或协议层同时保留两套导入入口。
2. **不做 Lua AST 自动改写器**：历史 schema `<= 3` 自定义来源通过设置反序列化平滑迁移后即可继续运行，仅在用户主动编辑保存时校验 `meta` 存在性，避免引入易误改代码的字符串/语法树重写逻辑。
3. **复用既有 `settings.names`，不另建名称存储子系统**：只将 `name` 从 `SourceRule` 剥离到 `SourceSummary` 读模型，直接利用已有 `settings.names` 与 `set_source_name` 完成新建落盘、重命名与删除清理。
4. **仅保留有明确依据的防御边界**：模块 `meta` 解析处于同步导入请求路径，使用 `mlua` 指令钩子防止顶层死循环卡死 UI/RPC 有明确故障依据；不对脚本字符长度、规则条目数或运行期抓包耗时设置任何无依据的硬上限。
5. **提示词外链单一规范文档，避免内置长文本与仓库文档重复维护**：客户端仅内置极简流程指令并指向 GitHub 仓库的 [docs/custom-subscription-sources.md](../../custom-subscription-sources.md)，既压缩复制体积与 AI 上下文占用，又保证后续文档或示例修订无需重复同步客户端资源。
