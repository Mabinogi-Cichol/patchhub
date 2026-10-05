# 源适配器与订阅

工具订阅网站，不订阅固定的游戏清单。Decky 扫描先读取成功缓存，选择游戏后按需查询启用的网站。只有具备 ZIP 安装能力的记录能进入共同的根目录安装流程；发现候选单独提供来源引导。

## 配置和状态

默认订阅位于 `apps/decky/backend/sources.json`，只包含网站规则。用户启用状态和新增源保存到 `~/.local/share/steam-patch-manager/sources/overrides.json`；成功查询结果缓存到同目录的 `cache/`。缓存只来自运行时成功查询，不附带当前游戏列表。显式刷新更新旧 ZIP 来源，游戏详情中的“重新查找 / 查看更多来源”更新该游戏发现记录；查询失败保留上次成功缓存并标明状态，结构失效不会覆盖有效缓存。停用源不会删除安装事务、备份或卸载能力。

patch.moe 专用适配器在已确认年龄的官方入口获取页面，读取官网语言表单的实际参数并提交，然后解析标题、日期、下载链接。只用完整标题、明确双语标题或实际相同 ZIP 地址关联的别名匹配；不按排列位置猜测，也不由英文文件名拼接中文链接。`+` 与普通版有区别。已公布但尚未发行的条目不能准备安装。非 ZIP 需要另行适配，当前不会执行网页或包内安装程序。

游戏详情中的“订阅源管理”选择此游戏使用哪些已配置的网站，不跳转全局设置。默认允许所有源，由全局启用状态决定是否参与匹配；每个游戏可单独关闭或重新打开某源。选择按 AppID 存储在 `overrides.json` 的 `games` 对象（例如 `{"games":{"42":{"patchmoe":false}}}`），同游戏在不同 Steam 库中共享选择，其他游戏不受影响。扫描返回各安装实例的 `sources` 列表，包含 `subscribed`（此游戏是否选择）和 `matched`（缓存中是否有无歧义匹配），即使未选择仍能看到和重新选择该源。准备安装也遵守源选择。网站的全局启用状态和新增规则从主面板“全局设置”修改。

## 添加表格网站

在 Decky“管理订阅的补丁源”中填入 JSON 规则。例如，以下是测试用占位网站，需要替换为真实发行商域名及实际列布局：

```json
{
  "id": "vendor",
  "name": "发行商补丁站",
  "adapter": "table_html",
  "enabled": true,
  "pages": ["https://patches.vendor.example/patches/"],
  "page_hosts": ["patches.vendor.example"],
  "download_hosts": ["downloads.vendor.example"],
  "root_layout": "game_root",
  "table_id": "patches",
  "columns": {"title": 0, "appid": 1, "download": 2, "date": 3, "language": 4}
}
```

列编号从 0 开始。标题与下载列必填；AppID、发布日期和语言列可选。没有语言列可使用规则的 `language`，默认为 en。只查询指定 HTTPS 页面、只下载指定域名的 ZIP；重定向也受该源的域名列表约束。明确 AppID 优先于名字；没有 AppID 才使用完整标题与明确别名。单个源匹配多个不同 ZIP 且无法消歧时不自动选择。不同源的结果分开呈现，不共享允许下载的域名。

## 网站专用适配器

表格配置不能替代所有网站逻辑。分页、搜索 API、产品详情页、语言表单等网站需要自己的适配器。后端在 `SourceRegistry` 初始化路径中通过 `register_adapter(id, collector)` 注册受维护的 Python 实现（插件与系统解析进程使用同一份注册代码）；规则只引用适配器名称，不下载或执行远端解析代码。

采集器接收 `(rule, previous_cache, force)`，返回 `(entries, etag, last_modified, unchanged)`。每条记录包含 `title`、`aliases`、`url`、`language`、`date`、`source_page`，可包含明确 `appid`。注册表统一验证记录及下载域名，执行源隔离的匹配，并产生 `release_id`：patch.moe 保留 `patchmoe-<appid>`，其他源使用 `source-<source-id>-<appid>`。新适配器需用真实页面结构及新增游戏、重复名称、失败缓存、语言、域名跳转等测试验证，随后在默认源配置中引用。

目前已实现 patch.moe 专用规则、通用表格规则、Kagura 国内 WordPress、海外 WooCommerce 和 HGamer 索引适配器。后面三者使用独立的 `SourceCandidate` / `discover_only` 路径，位于 `backend/discovery.py`，不遵循 ZIP 采集器返回格式。v2 规则区分 `publisher` 与 `fallback_indexer`，升级保留旧启停、逐游戏订阅和自定义规则。其他网站仍需各自配置或开发专用适配器，不能仅粘贴任意网址就自动适配。

Kagura 双站分别保留语言、本体 / DLC、原始版本、镜像和格式；未知条件不自动选择。匹配不足时 HGamer 接收选中游戏的检索名称，Steam AppID / 有出处别名用于核实身份，标题线索仍待确认。索引声明 ZIP 而官方链接为 EXE 时显示冲突，两者均不得扩展下载白名单或成为安装 release ID。

发现缓存按来源和 AppID 分片，保存成功结果与最近尝试：官方索引 24 小时、详情 6 小时、HGamer 正结果 24 小时、完整零结果 6 小时；失败 / partial 不写无补丁负缓存。解析版本、名称、来源选择影响缓存键；强制查询绕过新鲜期，停用来源既不查询也不显示普通候选。单游戏预算 30 秒，归档分页可部分完成并保留检查点。具体契约和验收见 [功能设计](features/kagura-hgamer-fallback/design.md)及[施工记录](features/kagura-hgamer-fallback/progress.md)。

## 安装与验证范围

安装默认保留包内相对路径到游戏根目录。获取时核对本地缓存哈希并向服务器重新校验：有 ETag / Last-Modified 时使用条件 GET，服务器返回 304 才复用已验证包；没有验证器则重新获取。之后固定 ZIP 哈希、当前 build 和原文件哈希，生成本机签名计划，由既有核心完成安全预检、确认、备份、提交与卸载。不同源、不同 Steam 库实例的计划与事务隔离；已有其他源的活跃安装阻止再次覆盖，以保留原始备份。

运行时查询遵守网站的实际可访问性。403、登录、验证码或网站结构变化需要相应处理，会显示查询失败而不会伪造匹配。真实网络验收与本地模拟页面测试需要分别报告。

Decky 的精简打包 Python 不含完整 HTML 解析模块。插件在该环境通过固定的 `/usr/bin/python3 -I` 工作进程解析官网，隔离打包运行库路径；工作进程只执行随插件部署的适配器。下载仍验证 HTTPS 证书，使用 SteamOS 系统证书库。
