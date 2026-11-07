# 自定义订阅源使用指南

PatchHub 的自定义订阅源由一份 JSON 规则和一份 Lua 脚本组成。规则声明来源名称、起始页面和允许访问的域名，脚本负责解析网站内容。导入时需要分别粘贴这两份文件的完整内容。请使用可信作者提供的脚本。

## 导入来源

1. 在 Windows 打开 **全局来源**，或在 Steam Deck 的 PatchHub 插件中打开 **管理订阅源**。
2. 选择 **导入脚本来源**，将规则 JSON 和 Lua 源码粘贴到各自的输入框。客户端会自动提交脚本 API 版本。
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
| `role` | `publisher` 更新时建立索引，`fallback_indexer` 在匹配游戏时按需补查。 |
| `pages` | 起始网页的完整 HTTPS 地址列表。站点首页也要保留路径斜杠，例如 `https://patches.vendor.example/`。 |
| `page_hosts` | 允许访问的页面域名列表。填写小写域名，每个子域名分别列出。 |
| `download_hosts` | 允许下载补丁的域名列表。提供直接下载时，列出下载链接使用的域名。 |
| `root_layout` | 下面的直接下载示例使用 `game_root`，表示安装以游戏根目录为基准。补查来源省略该字段，并设置 `"discovery_only": true`。 |

普通来源的 `collect_index(ctx, rule, previous)` 返回条目列表 `entries`、待处理页面 `pending` 和完成标记 `complete`。列表直接提供下载链接时，可以返回下例中的 `entry` 条目。列表只提供详情页链接时，可返回 `product`、`article` 或 `category` 条目，再由 `resolve_page(ctx, rule, page)` 从命中的页面提取标题、Steam AppID 和镜像等事实。

补查来源实现 `discover(ctx, rule, query, previous)`。用 `query.appid` 或 `query.terms` 查询网站，返回 `pages`、`identities`、`complete` 和 `diagnostics`。每个 `pages` 条目包含 `fact`，可附 `reported_format` 和 `upstream_page`。每个身份条目包含有网页依据的 `appid`、`page` 和 `aliases`。确认查无结果时，列表可用 `ctx.array()` 返回空数组。网站访问失败或查询未完成时，应报告错误或返回 `complete = false`。

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
