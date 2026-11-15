# 自定义订阅源使用指南

PatchHub 的自定义订阅源由一份 JSON 规则和一份 Lua 脚本组成。规则声明来源名称、起始页面和允许访问的域名，脚本负责解析网站内容。导入时需要分别粘贴这两份文件的完整内容。请使用可信作者提供的脚本。

## 导入来源

1. 在 Windows 打开 **全局来源**，或在 Steam Deck 的 PatchHub 插件中打开 **管理订阅源**。
2. 选择 **导入脚本来源**，将规则 JSON 和 Lua 源码粘贴到各自的输入框。
3. 选择 **保存并导入**。保存时会检查规则和脚本语法。卡片显示 **尚未执行** 时，可以按下一节验证实际运行结果。

编辑器里的 **查看脚本示例** 可以单独复制规则和源码，查看和复制都不会覆盖当前草稿。

## 更新来源并验证匹配

| 来源类型 | 导入后的验证方式 |
| --- | --- |
| 普通来源（`publisher`） | 在来源卡片上启用来源并执行 **手动更新**。更新成功后，对适用的游戏重新匹配并查看补丁候选。 |
| 补查来源（`fallback_indexer`） | 启用来源，确认目标游戏已订阅该来源，再重新匹配。只有普通来源覆盖完整且没有可信命中时，才会按游戏触发补查。 |

遇到错误时，先查看更新或匹配任务的诊断。语法错误通常可按 Lua 行号定位。运行错误需要检查请求结果和网页结构。返回格式错误时，先检查入口返回的字段、字段类型，以及空列表是否使用了 `ctx.array()`。

## 编辑或删除来源

- 选择 **编辑脚本**，修改后用 **保存并替换** 更新同一个来源。替换后，普通来源需要重新更新索引，再验证游戏匹配。
- 如需用另一个来源 ID，使用 **导入脚本来源**。新导入的 ID 必须唯一。
- 选择 **删除来源**可移除自定义来源。内置来源可通过 **查看脚本** 阅读实现。

## 规则怎么写

| 字段 | 填写方式 |
| --- | --- |
| `id` | 来源唯一标识，以小写英文字母开头，后续可用小写字母、数字和连字符，最多 48 个字符。 |
| `name` | 来源在界面上的显示名称。 |
| `enabled` | 布尔值，填 `true` 启用来源，填 `false` 禁用。 |
| `role` | 默认 `publisher`，更新时建立索引。`fallback_indexer` 在匹配游戏时按需补查。 |
| `pages` | 起始网页的完整 HTTPS 地址列表。站点首页也要保留路径斜杠，例如 `https://patches.vendor.example/`。 |
| `page_hosts` | 允许访问的页面域名列表。填写小写域名，每个子域名分别列出。 |
| `download_hosts` | 直接下载来源至少填写一个下载域名。补查来源填写 `[]` 或省略。 |
| `root_layout` | 直接下载来源填 `game_root`，表示安装以游戏根目录为基准。仅发现来源省略该字段。 |
| `discovery_only` | 默认为 `false`。仅提供发现结果时设为 `true`，补查来源必填 `true`。 |
| `adapter` | 自定义脚本填 `lua` 或省略。 |

## Lua 模块入口

Lua 文件返回一个表，表中的函数按名称供 PatchHub 调用。每次调用使用新的 Lua 环境，`rule` 是当前生效规则的副本，访问权限由宿主按原规则检查。保存时编译语法，入口及返回值在实际任务中验证。

| 入口 | 参数和返回值 |
| --- | --- |
| `collect_index(ctx, rule, previous)` | 普通来源更新时调用。返回 `{entries, pending, complete, etag?, modified?, not_modified?, diagnostics?}`。`previous` 提供上一份可用索引或更新进度中的 `entries`、`pending`、`complete`、`etag`、`modified`，没有记录时为 `ctx.null`。 |
| `resolve_page(ctx, rule, page)` | 索引包含 `product`、`article` 或 `category` 时实现。游戏匹配时接收详情页的绝对 URL，返回一个 `PageFact`。 |
| `discover(ctx, rule, query, previous)` | 补查来源匹配时调用。`query` 提供 `appid`、`name`、`name_hints`、`terms`、`force`，`previous.known_candidates` 提供已有候选。返回 `{pages, complete, identities?, diagnostics?}`。 |

- **索引条目**需要 `title`、`page`、`kind`，可附 `appid`、`aliases`、`language`、`date`。`kind = "entry"` 的条目直接提供下载地址，必须填 `url`。其他类型的 `page` 指向待解析的页面。
- **页面事实 `PageFact`** 需要 `title`、`page`、`kind`。类型可为 `product`、`article` 或 `category`。可提供 `steam_ids`、`aliases`、`text`、`published_at`、`body_links` 和 `mirrors`。镜像项填写 `{url, label?}`，格式由宿主从 URL 判断。同一补丁的替代下载链接可用 `single_download_group = true` 保留在一个候选中。分类页还可提供 `article_links`（每项为 `{title, page}`）、`article_facts`（内嵌页面事实）和 `next_page`。
- **补查结果**的 `pages` 每项为 `{fact, reported_format?, upstream_page?}`。`reported_format` 接受 `unknown`、`zip`、`exe`、`web`。身份项为 `{appid, page, aliases?}`，AppID 和别名应有网页证据。PatchHub 根据这些事实生成候选。
- **完成状态**由 `complete` 表示。索引还有待抓取的绝对页面 URL 时，放入 `pending` 并返回 `complete = false`。收到 HTTP 304 且已有完整索引时，可返回 `not_modified = true` 并保留原有条件请求字段。补查确认无结果时，返回 `pages = ctx.array(), complete = true`。请求失败或查询尚未完成时，抛出错误或返回 `complete = false`。诊断项为 `{code, detail?}`。

## Lua API 能力一览

宿主函数用点调用，例如 `ctx.get(url)`。HTML 节点的方法用冒号调用，例如 `node:select("a[href]")`。参数后的 `?` 表示可省略。

### HTTP 请求和 URL

| 能力 | API | 参数和返回值 |
| --- | --- | --- |
| GET 与条件请求 | `ctx.get(url, validators?)` | 可传 `{etag?, modified?}`，也可将已有索引传作条件字段来源。 |
| POST 文本 | `ctx.post(url, content_type, body)` | 传入内容类型与文本正文，例如向 JSON 接口发送 `ctx.json_encode(data)`。 |
| POST 表单 | `ctx.form(url, fields)` | 传入 `{{"name", "value"}, ...}`。宿主进行表单 URL 编码，保留字段顺序和重复字段。空表单用 `ctx.array()`。 |
| 拼接 URL | `ctx.url_join(base, reference)` | 用绝对基址解析相对引用，返回完整 URL。无效参数会报错。 |
| 规范化页面 URL | `ctx.canonical_url(url)` | 返回用于页面识别的字符串，通常会去掉查询参数和片段。请求需要查询参数或下载需要签名时，保留原始 URL。 |
| 读取 URL 部件 | `ctx.url_parts(url)` | 返回 `{host, path}`。无效 URL 会报错。 |
| 检查导航 URL | `ctx.navigation_url(url)` | 返回布尔值，表示 URL 是否符合通用导航地址规则。访问页面时仍检查 `page_hosts`。 |

三个请求 API 都返回 `{status, text, url, etag?, modified?}`。其中 `status` 是 HTTP 状态码，`text` 是解码后的正文，`url` 是重定向后的最终地址。脚本按站点语义处理状态码，例如对 304 复用索引。请求及其重定向受 `page_hosts` 约束，Cookie 由宿主请求上下文管理，连接失败等请求错误会抛出 Lua 错误。

### HTML 解析

| 能力 | API | 返回值 |
| --- | --- | --- |
| 解析 HTML | `ctx.html(text)` | HTML 根元素。 |
| 查询后代元素 | `node:select(css)` | 匹配元素的 Lua 序列。无匹配时为空序列，CSS 选择器无效时抛出错误。 |
| 提取文本 | `node:text()` | 拼接后的文本，连续空白折叠为一个空格，首尾空白去除。 |
| 读取属性 | `node:attr(name)` | 属性字符串，缺失时为 `nil`。 |
| 读取标签名 | `node:tag()` | 标签名字符串，例如 `"a"`。 |
| 遍历父子元素 | `node:parent()`、`node:children()` | 父元素或 `nil`，以及直接子元素序列。 |

### 游戏身份、数据转换与任务状态

| 能力 | API | 参数和返回值 |
| --- | --- | --- |
| 提取 Steam AppID | `ctx.steam_ids(links)` | 接收 URL 字符串数组，从 Steam 商店的 `/app/` 和 `/widget/` 链接提取去重后的 AppID 数组。 |
| 提取名称和别名 | `ctx.game_title(text)`、`ctx.aliases(title, blocks)` | 前者从文本提取游戏标题，后者从标题及文本块数组提取别名数组。 |
| 提取或规范化日期 | `ctx.date(text)`、`ctx.normalize_date(text)` | 前者从文本中提取日期，找不到时返回 `nil`。后者校验斜杠分隔的索引日期或 `COMING SOON`，并返回规范化字符串。 |
| 匹配完整词 | `ctx.word_contains(text, word)` | 返回布尔值。ASCII 字母忽略大小写，Unicode 字母、数字及下划线参与词边界判断。 |
| 读写 JSON | `ctx.json_decode(text)`、`ctx.json_encode(value)` | 分别将 JSON 文本解码为 Lua 值，或将 Lua 值编码为 JSON 字符串。 |
| 表示空数组和空值 | `ctx.array()`、`ctx.null` | 前者创建可追加元素的数组表，空时编码为 `[]`。空表 `{}` 编码为 `{}`。后者表示 JSON `null`，判断时用 `value == ctx.null`。 |
| 报告进度 | `ctx.progress(message)` | 向当前任务发送一条进度消息。 |
| 检查取消 | `ctx.check_cancel()` | 任务已取消时抛出错误，可在分页循环中显式调用。 |

索引、页面事实和 JSON 转换共用值规则。数组从索引 `1` 开始且连续，对象使用字符串键。同一张表混用两种键、循环引用或使用非有限数值会报错。Lua 整数为有符号 64 位，网站标识超出该范围时应使用字符串。DOM 节点用来读取页面，返回结果时应先提取所需字段。

脚本运行在 Lua 5.4 环境中，可用 `string`（除 `string.dump`）、`table`、`math`、`utf8`，以及 `assert`、`error`、`ipairs`、`next`、`pairs`、`select`、`tonumber`、`tostring`、`type`、`pcall`、`xpcall`、`_VERSION` 和 `_G`。HTTP 走上述宿主 API，HTML 按响应文本解析。运行时没有浏览器 JavaScript、文件或进程访问、外部模块加载及补丁安装 API。

更多可阅读的实现见[详情页示例](features/script-source-adapters/examples/product-pages/adapter.lua)、[HGamer 补查脚本](../crates/core/resources/source-scripts/hgamer.lua)和 [Rust 运行时](../crates/core/src/sources/adapters/script.rs)。

## 完整示例：从 HTML 表格导入补丁

假设补丁列表每行有 Steam AppID、游戏标题和下载链接，页面结构如下。

```html
<table>
  <tr data-appid="123">
    <td class="title">示例游戏</td>
    <td><a href="https://downloads.vendor.example/game.zip">下载补丁</a></td>
  </tr>
</table>
```

### 规则 JSON

```json
{
  "id": "my-script-publisher",
  "name": "示例补丁列表",
  "enabled": true,
  "role": "publisher",
  "pages": ["https://patches.vendor.example/patches/"],
  "page_hosts": ["patches.vendor.example"],
  "download_hosts": ["downloads.vendor.example"],
  "root_layout": "game_root"
}
```

### Lua 源码

```lua
return {
    collect_index = function(ctx, rule, previous)
        local entries = ctx.array()

        for _, page in ipairs(rule.pages) do
            local response = ctx.get(page)
            if response.status ~= 200 then
                error("source_http_" .. response.status)
            end

            local root = ctx.html(response.text)
            for _, row in ipairs(root:select("tr[data-appid]")) do
                local title = row:select(".title")[1]
                local link = row:select("a[href]")[1]
                if not title or not link then
                    error("missing_title_or_download")
                end

                local appid = tonumber(row:attr("data-appid"))
                if not appid then
                    error("invalid_appid")
                end

                entries[#entries + 1] = {
                    title = title:text(),
                    appid = appid,
                    page = response.url,
                    kind = "entry",
                    url = ctx.url_join(response.url, link:attr("href"))
                }
            end
        end

        return {entries = entries, pending = ctx.array(), complete = true}
    end
}
```

将规则中的占位地址和域名替换为实际站点，再按网页结构修改 Lua 中的 CSS 选择器。脚本用 `ctx.get` 请求页面，用 `ctx.html` 解析 HTML，并用 `ctx.url_join` 将相对下载路径转换为完整地址。逐行提取的 AppID 必须来自页面中的 Steam 游戏标识，示例中的 `123` 仅用于演示。

把两段完整文本分别粘贴到导入编辑器，保存后执行手动更新。更新成功后，选择补丁列表里确实包含的已安装游戏重新匹配，核对补丁标题、来源页和下载地址。如果更新后没有预期条目，先核对实际响应是否包含表格，以及选择器能否选中数据行。
