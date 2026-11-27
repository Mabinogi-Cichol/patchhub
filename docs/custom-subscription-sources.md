# 自定义订阅源开发与使用指南

PatchHub 的自定义订阅源采用**单文件自描述 Lua 脚本**格式：静态 `meta` 表声明起始页面、允许访问的域名和来源能力，入口函数负责抓取和解析网页内容。

用户无需手动填写复杂的规则表单——只需在 PatchHub 中点击 **复制 AI 提示词** 发给 AI 编程助手，由 AI 向用户确认来源类型与网址、现场核查目标网页并生成完整 Lua 脚本，用户复制整份脚本粘贴保存即可导入。请仅导入你信任的来源提供的脚本。

---

## 导入、编辑与管理来源

### 1. AI 辅助生成并导入（推荐）

1. 在 Windows 或 Steam Deck 的 PatchHub 中打开 **全局来源 → 导入脚本来源**。
2. 点击 **复制 AI 提示词**，将提示词粘贴发送给你的 AI 助手（提示词内容见 [AI 提示词模板](../crates/core/resources/ai-source-prompt.md)）。
3. AI 会主动向你询问**来源类型**、**订阅源网址**及**可选显示名称/补充说明**，分析目标站点后输出一份**完整 Lua 源码**。
4. 将 AI 输出的整段 Lua 源码粘贴到 **Lua 源码** 输入框，点击 **保存并导入**。

### 2. 宿主如何管理 ID、名称与内置来源

- **自动生成并锁定 `id`**：新建导入时不填写也不在脚本中声明 `id`；宿主自动根据 `meta.pages` 首个页面域名生成规范化小写 `id`（自动去掉前导 `www.` 并转为连字符 slug；若已存在同前缀来源则自动追加 `-2`、`-3`）。使用 **编辑脚本 → 保存并替换** 更新已有来源时，原 `id` 保持锁定不变。
- **本地显示名称与规则解耦**：`meta.name` 仅作为来源首次导入或未改名时的默认显示名称（省略时默认取首个域名）。你在界面上随时可以通过 **修改来源名称** 改名，这只会写入本地 `settings.names`，不会更改脚本或规则指纹，也不会使已有缓存失效。
- **内置来源只读保护**：无法通过替换覆盖内置来源（触发 `source_builtin_readonly`）。如需参考或二次定制内置来源，请打开对应卡片的 **查看脚本 → 复制 Lua 源码**，再通过 **导入脚本来源** 粘贴保存，宿主会自动分配新的非冲突 `id`。
- **更新、编辑、删除与清缓存**：
  - 导入或替换脚本时，宿主**仅在无网络、无文件 I/O 的沙箱中求值模块顶层以读取 `meta` 和检查入口**，不会发起网络请求或执行爬虫入口函数。
  - **索引源（`publisher`）**：保存后卡片启用开关默认开启，请在卡片点击 **手动更新 / 更新来源索引** 或点击 **全部更新**，再回到游戏详情页匹配补丁。
  - **补查源（`fallback_indexer`）**：启用来源并确认目标游戏已订阅该来源，对目标游戏触发重新匹配。补查源仅在所有索引源覆盖完整且没有可信命中时按游戏按需触发。
  - **调试期彻底清理缓存**：如需在调试脚本期间重跑全部抓取与详情解析，可在 **设置 → 缓存管理** 清理 **本地游戏数据缓存**（仅清理索引、详情与匹配缓存，不会删除已导入脚本、名称、更新间隔与订阅设置）。

---

## AI 代理编写脚本标准工作流

如果你是受托编写 PatchHub 订阅源脚本的 AI 编程助手，请严格按以下顺序工作：

1. **先提问收集必要信息**：优先调用可用的交互式提问工具向用户确认：
   - **来源类型**：索引源（`publisher`，站点有集中列表/目录/发布页，可定期建立索引）或 补查源（`fallback_indexer`，站内按游戏名/AppID 实时搜索，无法一次拉全量列表）；
   - **订阅源网址**：用户希望抓取的起始页面（如列表首页、补丁发布页或搜索页）；
   - **可选显示名称与补充说明**：期望展示在界面的名称、站点是否需要登录态/特殊筛选条件等。
2. **现场分析并静态核验目标站点**：以普通网页 Chat 为默认环境，无需安装或运行 PatchHub，也无需用户提供本机日志。
   - 检查入口、目录、相关分页和详情页，确认脚本使用的 CSS 选择器、链接属性、分页条件及下载域名。网页摘要或渲染后的纯文本不足以证明具体 HTML 结构，不得凭猜测编造选择器或主机名。
   - 选站内确实发布过补丁的 Steam 游戏，按脚本逐步核对条目来源、命中的标题、别名或 AppID，以及详情和下载链接的取得方式。按需详情须核对索引阶段已有足够命中线索：Core 命中后才调用 `resolve_detail`。站点分别发布本体和 DLC 补丁时，分别核对两条路径；确认相同规则适用于其他同类条目，样本专属数据未泄漏到脚本逻辑。
   - 导入成功、索引非空和网页静态核验分别代表不同的验证范围。
   - **人机验证或登录墙**：若发现站点返回 CAPTCHA、Turnstile、Cloudflare Challenge 或要求先在浏览器登录，严禁尝试绕过；应立即停止生成可用抓取逻辑或明确提醒用户该站点存在人机验证拦截，由用户自行确认访问能力。
3. **只编写一份自包含的完整 Lua 模块**：
   - 将规则声明放入静态 `meta` 表，不输出任何额外的 JSON 规则文本；
   - 根据源类型实现 `collect_index`（必要时搭配 `resolve_detail`、`match_appid`）或 `discover`；
   - 同类条目使用同一套站点规则，禁止写死具体游戏或 DLC 的名称、AppID、别名映射、专属详情链接，或按单个游戏设置分支、白名单和兜底表；人工核实的样本数据也只能用于验证。站点入口、通用页面结构及 `meta.page_hosts`、`meta.download_hosts` 域名权限按现有契约填写；
   - 游戏身份信息从运行时取得的来源页面或 API 提取；缺少 AppID 证据时留空，保留有出处的标题和别名。`entry.title` 和 `SourceDetail.title` 均参与匹配，须保留可匹配的游戏名及区分本体、DLC 所需的名称；补丁版本、日期和下载说明放在 `SourceDetail.text`，避免拼入匹配标题；
   - 空数组统一使用 `ctx.array()`，避免 Lua 空表 `{}` 序列化为 JSON 对象 `{}` 造成契约错误；
   - 顶层只返回声明表与纯常量，不在顶层执行网络调用、DOM 解析或大循环。
4. **交付结果**：网页证据足以支持实现时，输出一份可直接复制导入的完整 `lua` 代码块，在代码块外简短列出权限域名、核对过的页面和匹配路径；只做了网页静态核验时如实说明。若缺少决定实现是否可行的页面证据，具体指出缺少的页面、接口或结构，再请用户补充。

---

## `meta` 元数据表规范

Lua 模块返回的顶级表中必须包含静态对象表 `meta`。`meta` 字段受宿主严格校验，**不允许出现 `id` 或任何未定义字段**（`invalid_source_meta`）。

| 字段 | 类型 | 必填 | 默认值 | 说明与约束 |
| --- | --- | --- | --- | --- |
| `pages` | 字符串数组 | **是** | 无 | 起始网页的完整 HTTPS 地址列表，不能为空。**站点首页务必保留结尾斜杠**，例如 `"https://patches.vendor.example/"`。每个 URL 必须命中 `page_hosts`。 |
| `name` | 字符串 | 否 | `page_hosts[1]` | 来源在界面上的默认显示名称（去除首尾空白后非空，总长度 ≤ 128）。仅用于初始化本地显示名称。 |
| `page_hosts` | 字符串数组 | 否 | 从 `pages` 自动推导 | 允许通过 `ctx.get`/`ctx.post`/`ctx.form`/`ctx.browser_*` 及页面条目 `page` 字段访问的小写精确主机名白名单（不含协议和端口，子域名须逐一列出）。省略或传空表 `{}` 时宿主从 `pages` 自动提取；**如果列表页会跳转到不同子域名详情页（例如 `news.vendor.example`），必须显式列出所有涉及的域名**。 |
| `download_hosts` | 字符串数组 | 否 | `{}`（空） | 允许下载补丁文件或访问外部网盘镜像的主机名白名单（小写精确域名）。**补查源（`fallback_indexer`）必须省略或为空**；若索引源留空，表示仅提供网页或外部导航发现，不强制校验镜像域名。 |
| `priority` | 整数 | 否 | `100` | 取值范围 `0..=1000`，用于同条件候选排序参考。 |
| `official_title_match` | 布尔值 | 否 | `false` | 仅索引源可设为 `true`。**仅限官方发行商或制作组自营发布站启用**：当站点索引页未提供 Steam AppID、但页面标题与 Steam 游戏精确匹配时，允许将其作为官方身份确认。补查源或非官方搬运/聚合站必须保持 `false` 或省略。 |
| `role` | 字符串 | 否 | 按导出函数自动推导 | 取值 `"publisher"` 或 `"fallback_indexer"`。省略时：导出 `collect_index` 推导为 `"publisher"`；仅导出 `discover` 推导为 `"fallback_indexer"`；同时导出两个或均未导出时必须显式检查或报错（`script_source_contract_error`）。 |

> [!IMPORTANT]
> 已彻底废除且禁止在 `meta` 中出现的旧字段：`id`、`adapter`、`enabled`、`root_layout`、`discovery_only`、`columns`、`table_id`、`url`、`language`。若出现上述字段会直接报错 `invalid_source_meta`。

---

## Lua 模块入口与数据契约

Lua 文件须返回一个表，宿主按名称调用其中的入口函数。每次任务调用使用全新的 Lua 沙箱环境；传给入口函数的 `rule` 是当前生效规则的只读快照副本，访问控制仍以宿主保存的原规则为准。

| 入口函数 | 参数与返回值 |
| --- | --- |
| `collect_index(ctx, rule, previous)` | 索引源更新时调用。返回 `{entries, pending, complete, etag?, modified?, not_modified?, diagnostics?}`。`previous` 为上一份可用索引或断点进度中的 `{entries, pending, complete, etag, modified}`；首次更新无记录时为 `ctx.null`。 |
| `resolve_detail(ctx, rule, state)` | 索引条目的 `current_step < total_steps` 时，在游戏匹配命中该条目后按需调用。接收 `state = {title, aliases, page, appid, current_step, total_steps, step_data, details}`，推进抓取并返回 `{current_step, total_steps, step_data?, details, diagnostics?}`。 |
| `match_appid(ctx, rule, site_ids, steam)` | 可选。当条目或详情携带网页提取出的 `site_ids` 数组时由宿主调用，用以判断页面是否归属目标游戏（可核对 `steam.appid`、`steam.related_dlc` 等）。返回 `true` 确认、`false` 明确排除；省略时按同 AppID 自动正向匹配，无网页 AppID 时转由游戏名称与别名匹配。 |
| `discover(ctx, rule, query, previous)` | 补查源（`fallback_indexer`）匹配时调用。`query` 固定包含 `{appid, name, name_hints, terms, force}`（`appid` 为整数 Steam AppID；`name` 为规范游戏标题；`name_hints` 与 `terms` 为搜索词数组；`force` 为布尔值）。`previous` 包含 `{known_candidates}`。返回 `{details, complete, identities?, diagnostics?}`。 |

### `SourceIndexEntry` 与两种索引抓取模式

索引条目字段：`{title, page, appid?, aliases?, current_step?, total_steps?, step_data?, details?}`（其中 `current_step` 与 `total_steps` 默认均为 `1`）。

1. **一步到位型（适合列表页已含完整下载信息）**：
   - 在 `collect_index` 中设置 `current_step = 1`、`total_steps = 1`，直接填好 `details` 列表，无需实现 `resolve_detail`。
2. **多步按需型（适合目录页仅列出游戏名与详情链接）**：
   - 在 `collect_index` 中设置 `current_step = 1`、`total_steps >= 2`、将 `details = ctx.array()` 留空，可在 `step_data` 保存分页状态或临时标识。
   - 索引阶段的标题、别名或 AppID 必须提供足够的命中线索；详情页中的身份信息不能代替命中前的索引线索。
   - 游戏匹配选中该条目后，宿主循环调用 `resolve_detail(ctx, rule, state)` 直到 `current_step == total_steps`（支持中途调大 `total_steps`，例如在主帖又发现子公告链接时从 `2` 调整为 `3`）。
   - **推进不变量与断点缓存**：每次返回必须严格满足 `next.current_step > state.current_step` 且 `1 <= next.current_step <= next.total_steps`，否则触发 `invalid_detail_step`。若某步返回携带 `diagnostics` 但尚未完结，宿主会保留进度供下次重试续跑。完成后结果持久化进 `DetailRecord`，其中 `detail.page ~= entry.page` 的子条目还会被宿主独立缓存以便跨入口复用。

### `SourceDetail`、`identities` 与校验红线

- **`SourceDetail`**：`{title, page, steam_ids?, aliases?, mirrors?, single_download_group?, text?, languages?, published_at?, body_links?, reported_format?, upstream_page?}`
  - `mirrors` 每项格式为 `{url, label?}`，宿主会依据 URL 自动判定压缩包（`zip`）、可执行程序（`exe`）或网页；同一补丁的多个分流地址若属于同一组，可设置 `single_download_group = true` 合并在同一个补丁候选中。
  - `reported_format` 若指定，必须为 `"unknown"`、`"zip"`、`"exe"`、`"web"` 之一。
  - `published_at` 若指定，必须为 `"YYYY-MM-DD"`、`"YYYY/M/D"` 或 `"COMING SOON"`（建议通过 `ctx.date` 或 `ctx.normalize_date` 生成）。
  - `body_links` 为页面正文相关的补充链接字符串数组；`upstream_page` 为上游原发布页（须通过通用导航 URL 检查）。
- **`discover` 中的 `identities`**：可选数组，每项 `{appid, page, aliases?}`，仅在网页上存在明确的 Steam 链接/ID 证据时输出（用于向身份核验层补充证据）。
- **必须遵守的校验红线（违反即报错）**：
  - `entry.title` 与 `detail.title` 去除空白后不能为空；
  - `entry.page`、`detail.page` 与 `pending` 每一项都必须属于 `page_hosts`；
  - `entry.appid` 不能为 `0`，`detail.steam_ids` 中不能包含 `0`；
  - 若 `meta.download_hosts` **非空**，则已完成条目的 `details` 及其中每项的 `mirrors` 都不得为空，且每个 `mirror.url` 必须通过 `download_hosts` 白名单校验；
  - 若 `complete = true`，则 `pending` 必须为空数组（`ctx.array()`）。

### 完成状态、304 与诊断返回

- 索引尚未抓完（如多分页）时，将剩余待抓绝对页面 URL 放入 `pending` 并返回 `complete = false`（下次增量继续或由任务继续推进）。
- 收到 HTTP 304 且 `previous` 为有效完整索引时，返回 `{not_modified = true, entries = ctx.array(), pending = ctx.array(), complete = true, etag = ..., modified = ...}`，宿主会自动复用 `previous.entries`。
- 补查源确认站内无对应游戏补丁时，返回 `{details = ctx.array(), complete = true}`。如网络异常或未完成查询，可直接抛 `error(...)` 或返回 `complete = false`。
- `diagnostics` 数组每项格式为 `{code, detail?}`（如 `{code = "source_http_404", detail = url}`）。

---

## Lua API 能力（`ctx`）

宿主函数统一通过点号调用（如 `ctx.get(url)`）；HTML DOM 节点方法统一通过冒号调用（如 `node:select("a[href]")`）。参数末尾 `?` 表示可省略。

### HTTP 请求、浏览器会话与 URL 处理

| 能力 | API | 参数与返回值 |
| --- | --- | --- |
| GET 与条件请求 | `ctx.get(url, validators?)` | `validators` 可传 `{etag?, modified?}` 或直接传入 `previous`。 |
| POST 文本 | `ctx.post(url, content_type, body)` | 传入 `Content-Type`（如 `"application/json"`）与正文字符串（常配合 `ctx.json_encode(data)`）。 |
| POST 表单 | `ctx.form(url, fields)` | 传入键值二元组数组 `{{"k", "v"}, ...}`，保留顺序及同名多值；空表单传 `ctx.array()`。 |
| 浏览器会话读页 | `ctx.browser_available`、`ctx.browser_get(url)`、`ctx.browser_snapshot(url)` | 当宿主提供浏览器上下文时可用（用于需要 Cookie/JS 的站点）。`browser_snapshot` 读取当前页面文档并保留表单控件现值而不重新导航。均受 `page_hosts` 限制。 |
| 浏览器会话发送请求 | `ctx.browser_post(url, content_type, body)`、`ctx.browser_form(url, fields)` | 在当前浏览器上下文中同源发送 POST/表单请求，表单字段格式同 `ctx.form`。 |
| 拼接 URL | `ctx.url_join(base, reference)` | 按基址解析相对链接，返回完整绝对 URL；参数非法会抛出 Lua 错误。 |
| 规范化 URL | `ctx.canonical_url(url)` | 去除用于展示的锚点与常见无状态参数；**下载直链、签名 URL 或分页参数请务必保留原 URL**。 |
| 拆解 URL 部件 | `ctx.url_parts(url)` | 返回 `{host, path, query}` 表；无效 URL 抛出错误。 |
| 校验导航 URL | `ctx.navigation_url(url)` | 返回布尔值，判断 URL 是否符合通用安全 HTTP/HTTPS 导航地址规则。 |

上述 5 个 HTTP / 浏览器请求 API 均返回 `{status, text, url, etag?, modified?}`（`url` 为重定向后的最终地址）。所有请求及重定向均受 `page_hosts` 白名单检查；Cookie 在同一任务上下文内由宿主自动维护；底层传输异常直接抛出 Lua 错误，业务 HTTP 状态码（如 304 / 404 / 429）由脚本判断处理。

### HTML 解析

| 能力 | API | 返回值 |
| --- | --- | --- |
| 解析文档 | `ctx.html(text)` | 返回文档根 DOM 节点。 |
| CSS 选择器查询 | `node:select(css)` | 返回匹配节点数组（无匹配返回空数组；CSS 选择器语法非法抛错）。 |
| 读取清洗文本 | `node:text()` | 拼接后代文本，连续空白折叠为单个空格并去除首尾空白。 |
| 读取属性 | `node:attr(name)` | 返回属性值字符串，缺失返回 `nil`。 |
| 标签名与节点遍历 | `node:tag()`、`node:parent()`、`node:children()` | 返回小写标签名、父节点（或 `nil`）、直接子元素节点数组。 |

### 游戏识别、数据转换与任务状态

| 能力 | API | 参数与返回值 |
| --- | --- | --- |
| 提取 Steam AppID | `ctx.steam_ids(links)` | 传入链接 URL 数组，自动识别 Steam 商店 `/app/<id>` 与 `/widget/<id>`，返回去重整数 AppID 数组。 |
| 标题与别名整理 | `ctx.game_title(text)`、`ctx.aliases(title, blocks)` | 从标题或公告文本块提取干净的游戏名称与别名数组。 |
| 日期提取与校验 | `ctx.date(text)`、`ctx.normalize_date(text)` | 前者从自由文本提取规范日期（找不到返回 `nil`）；后者校验斜杠日期或 `COMING SOON`。 |
| 完整词匹配 | `ctx.word_contains(text, word)` | Unicode 词边界完整匹配，ASCII 字母忽略大小写，返回布尔值。 |
| JSON 编解码 | `ctx.json_decode(text)`、`ctx.json_encode(value)` | JSON 文本与 Lua 值互转。 |
| 空数组与 JSON null | `ctx.array()`、`ctx.null` | **至关重要**：`ctx.array()` 标记该表为数组（空时序列化为 `[]`）；裸表 `{}` 默认序列化为 `{}` 对象。`ctx.null` 表示 JSON `null`，判空用 `val == ctx.null`。 |
| 进度反馈与取消 | `ctx.progress(message)`、`ctx.check_cancel()` | 汇报阶段文字进度；在较长的分页循环中调用 `ctx.check_cancel()` 可立即响应用户取消。 |

---

## Lua 5.4 沙箱环境与错误处理

- **可用标准库**：`string`（不含 `string.dump`）、`table`、`math`、`utf8`，以及基础函数 `assert`、`error`、`ipairs`、`next`、`pairs`、`select`、`tonumber`、`tostring`、`type`、`pcall`、`xpcall`、`_VERSION`、`_G`。
- **禁用能力**：不提供 `io`、`os`、`debug`、`package`、`dofile`、`loadfile`、模块加载、文件读写、子进程或原生补丁安装 API。
- **顶层初始化指令预算**：保存或加载脚本时，宿主执行模块顶层代码的指令数上限为 **1,000,000 条**（`script_source_init_limit_exceeded`）。请勿在模块顶层编写大量计算或循环，所有请求与循环必须放在入口函数内部。
- **数组与整数规范**：返回数组下标从 `1` 开始连续排列，不可在同一表中混用数组下标和字符串键；超大站点 ID（超出有符号 64 位整数）应转为字符串保存；不要直接将 DOM 节点放进返回值。
- **常见错误码速查**：
  - `invalid_source_meta`：缺少 `meta`、存在未知字段、`pages` 非 HTTPS/空，或补查源填写了 `download_hosts`/`official_title_match`。
  - `script_source_init_limit_exceeded`：顶层初始化代码触发指令预算上限。
  - `script_source_contract_error`：未导出对应入口函数，或返回的 JSON 结构/数组表类型不合法（例如忘了用 `ctx.array()`）。
  - `invalid_detail_step`：`resolve_detail` 返回的 `current_step` 未严格递增或超出 `total_steps`。

---

## 完整单文件示例

以下三个示例均可完整复制并直接通过 PatchHub 的 **导入脚本来源** 保存（测试时请把示例域名替换为实际站点）。

### 示例 1：一步到位型索引源（列表页直接提供补丁下载）

```lua
return {
    meta = {
        name = "示例补丁发布站",
        pages = { "https://patches.vendor.example/patches/" },
        page_hosts = { "patches.vendor.example" },
        download_hosts = { "downloads.vendor.example" },
        priority = 100,
    },

    collect_index = function(ctx, rule, previous)
        local response = ctx.get(rule.pages[1], previous)
        if response.status == 304 and previous ~= ctx.null and previous.complete then
            return {
                not_modified = true,
                entries = ctx.array(),
                pending = ctx.array(),
                complete = true,
                etag = response.etag or previous.etag,
                modified = response.modified or previous.modified,
            }
        end
        if response.status ~= 200 then
            error("source_http_" .. response.status)
        end

        local entries = ctx.array()
        local root = ctx.html(response.text)
        for _, row in ipairs(root:select("tr[data-appid]")) do
            ctx.check_cancel()
            local title_node = row:select(".title")[1]
            local link_node = row:select("a[href]")[1]
            local appid = tonumber(row:attr("data-appid"))
            if not title_node or not link_node or not appid or appid == 0 then
                error("missing_row_fields")
            end

            local item_title = ctx.game_title(title_node:text())
            local download_url = ctx.url_join(response.url, link_node:attr("href"))
            entries[#entries + 1] = {
                title = item_title,
                appid = appid,
                page = response.url,
                current_step = 1,
                total_steps = 1,
                details = {
                    {
                        title = item_title,
                        page = response.url,
                        steam_ids = { appid },
                        mirrors = { { url = download_url, label = "直接下载" } },
                    },
                },
            }
        end

        return {
            entries = entries,
            pending = ctx.array(),
            complete = true,
            etag = response.etag,
            modified = response.modified,
        }
    end,
}
```

### 示例 2：多步按需型索引源（列表页 + 详情页按需抓取）

```lua
return {
    meta = {
        name = "示例多步游戏目录",
        pages = { "https://catalog.vendor.example/games/" },
        download_hosts = { "cdn.vendor.example" },
    },

    collect_index = function(ctx, rule, previous)
        local entries = ctx.array()
        local response = ctx.get(rule.pages[1])
        if response.status ~= 200 then
            error("source_http_" .. response.status)
        end

        local root = ctx.html(response.text)
        for _, card in ipairs(root:select("a.game-card[href]")) do
            local title = ctx.game_title(card:text())
            local detail_url = ctx.url_join(response.url, card:attr("href"))
            if title ~= "" then
                entries[#entries + 1] = {
                    title = title,
                    page = detail_url,
                    current_step = 1,
                    total_steps = 2,
                    details = ctx.array(),
                }
            end
        end

        return {
            entries = entries,
            pending = ctx.array(),
            complete = true,
        }
    end,

    resolve_detail = function(ctx, rule, state)
        local response = ctx.get(state.page)
        if response.status ~= 200 then
            error("detail_http_" .. response.status)
        end

        local root = ctx.html(response.text)
        local steam_links = ctx.array()
        for _, anchor in ipairs(root:select("a[href*='store.steampowered.com']")) do
            steam_links[#steam_links + 1] = anchor:attr("href")
        end

        local dl = root:select("a.download-patch[href]")[1]
        if not dl then
            error("missing_download_link")
        end

        return {
            current_step = state.current_step + 1,
            total_steps = state.total_steps,
            details = {
                {
                    title = state.title,
                    page = response.url,
                    steam_ids = ctx.steam_ids(steam_links),
                    mirrors = { { url = ctx.url_join(response.url, dl:attr("href")) } },
                },
            },
        }
    end,
}
```

### 示例 3：按需补查源（实时按游戏查询）

```lua
return {
    meta = {
        name = "示例补查站",
        role = "fallback_indexer",
        pages = { "https://search.vendor.example/" },
        page_hosts = { "search.vendor.example" },
    },

    discover = function(ctx, rule, query, previous)
        local term = query.terms[1] or query.name
        local response = ctx.form("https://search.vendor.example/search", {
            { "q", term },
        })
        if response.status ~= 200 then
            error("search_http_" .. response.status)
        end

        local details = ctx.array()
        local identities = ctx.array()
        local root = ctx.html(response.text)
        for _, item in ipairs(root:select("article.post")) do
            local title_node = item:select("h2 a[href]")[1]
            if title_node then
                local page_url = ctx.url_join(response.url, title_node:attr("href"))
                local steam_links = ctx.array()
                for _, link in ipairs(item:select("a[href]")) do
                    steam_links[#steam_links + 1] = link:attr("href")
                end
                local steam_ids = ctx.steam_ids(steam_links)
                local mirrors = ctx.array()
                for _, dl in ipairs(item:select("a.mirror[href]")) do
                    mirrors[#mirrors + 1] = { url = ctx.url_join(response.url, dl:attr("href")) }
                end
                details[#details + 1] = {
                    title = ctx.game_title(title_node:text()),
                    page = page_url,
                    steam_ids = steam_ids,
                    mirrors = mirrors,
                    reported_format = "web",
                }
                for _, id in ipairs(steam_ids) do
                    identities[#identities + 1] = { appid = id, page = page_url }
                end
            end
        end

        return {
            details = details,
            identities = identities,
            complete = true,
        }
    end,
}
```

---

## 内置脚本与源码参考

- [仓库默认内置配置](../crates/core/resources/sources.json)
- 内置来源完整实现：[Shiravune](../crates/core/resources/source-scripts/patchmoe.lua)、[Kagura 国内](../crates/core/resources/source-scripts/kagura-cn.lua)、[Kagura 海外](../crates/core/resources/source-scripts/kagura-en.lua)、[YOZAKURA](../crates/core/resources/source-scripts/yozakura.lua)、[HGamer 补查源](../crates/core/resources/source-scripts/hgamer.lua)
- [AI 提示词模板](../crates/core/resources/ai-source-prompt.md) 与 [Lua 宿主运行时](../crates/core/src/sources/adapters/script.rs)
