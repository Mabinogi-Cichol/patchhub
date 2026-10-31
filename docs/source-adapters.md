# 源适配器与订阅

来源能力已迁入共享 Rust 核心并作为单一来源引擎（P0–P5 已在 Windows 与 Decky 实机验证，P6 收口）：[设计](features/shared-source-discovery/design.md)、[施工记录](features/shared-source-discovery/progress.md)、[P0 基线](features/shared-source-discovery/p0-baseline.md)、[P5 契约](features/shared-source-discovery/p5-decky-design.md)。下文描述现行共享核心入口与用户可见行为。自定义 Lua 来源现已提供双端导入/编辑，见[脚本适配器设计](features/script-source-adapters/design.md)与[施工记录](features/script-source-adapters/progress.md)；四个内置来源及通用表格已全部使用随核心内嵌的 Lua 脚本。

工具订阅网站，不订阅固定的游戏清单。工作单位是“来源”而不是“游戏”：每次来源更新只遍历该来源自己的列表/目录页并发布共享索引；游戏匹配先读本地索引，命中后才按需获取相关详情。普通源索引不会因为新增游戏、重启或手动重匹配而重新遍历。只有具备 ZIP/EXE 安装能力的匹配结果进入安装流程；仅相似标题的候选仍需人工选择。

## 配置和状态

默认订阅由核心内置 `crates/core/resources/sources.json`（编译进 CLI，单一来源）。用户启用状态、新增源、逐游戏订阅与每源自动更新间隔写入核心命名空间 `~/.local/share/patchhub/sources/rust-v1/`：

```text
sources/rust-v1/
  schema.json                 命名空间 schema 2 完成标记
  settings.json               设置 schema 3：启用/新增/名称/订阅/intervals/script_sources
  status/<source>.json        最近尝试结果与最近成功时间
  index/<source>.json         正式按源索引快照
  index-progress/<source>.json 未完成更新的续跑进度（成功后删除）
  details/<source>/<key>.json 按规范 URL 共享的页面事实缓存
  matches/<appid>.json        按游戏身份保存的匹配结果
```

首次初始化直接写入 Rust 默认状态；旧 `sources/overrides.json` 与旧 `cache/`、`discovery/`、`discovery-index/` 只在版本升级时读取一次并迁移，之后不再读写（旧文件保留，不删除）。迁移失败保留原数据并报告，不会静默变成空结果。缓存年龄不阻止匹配、人工选择或安装。

## 来源索引更新

- `update_source(source_id, force?)` 只更新指定来源，不接受游戏列表；请求数与已安装游戏数无关。界面手动更新与宿主自动更新共用该入口。
- 只有完整成功的结果才替换 `index/<source>.json`；失败、部分成功或取消保留旧正式快照，并把已抓条目与剩余分页写入 `index-progress/<source>.json`，下次更新从该队列续跑。
- `force` 忽略条件请求；非 force 且存在正式快照时，patch.moe/通用表格源可携带 `If-None-Match`/`If-Modified-Since`，304 只更新检查时间、不改内容修订。
- 内容修订 `revision` 只在条目、完整性或续跑队列实际变化时递增；仅更新时间或校验器变化不触发游戏重匹配。
- Kagura 来源更新只遍历列表/目录分页，保存文章/产品/分类链接，不抓取详情或分类页内容；分页失败页留在进度队列中，由取消/续跑控制，不设任意页数截断。
- HGamer 是补查来源，不做全站更新；`update_source` 对 fallback 源返回 `source_not_indexable`。

## 增量匹配与多来源补查

- `match_games`（可指定实例/AppID、`force_details?`）与 `rematch_game` 复用已发布索引和持久化详情：本地按 AppID 优先、完整标题/别名其次筛选；只对命中条目获取详情，详情按来源+规范 URL 跨游戏共享。
- 匹配输入（游戏名、本地化名、名称提示、相关源订阅）未变且来源修订未变时直接读取 `matches/<appid>.json`，不联网。补丁语言偏好已移除，旧摘要沿既有匹配流程重算。
- 结果状态为六态：待匹配、普通源命中、补查来源命中、完整无结果、未完成/失败；索引或详情不完整时不冒充“确认无补丁”。
- 补查来源（HGamer 与自定义脚本）仅在无普通源可信命中且所有启用的普通源覆盖完整时按游戏查询。遍历全部有效来源并聚合，单源失败保留其他来源的可信候选与完整诊断；所有源完整且为空才报告无结果。普通源命中优先显示并隐藏补查结果；普通源未初始化/不完整时显示“未完成”，不声称补查已查无结果。
- 手动重匹配（含已命中、无补丁、失败游戏）只重算该游戏：复用普通源索引、刷新命中详情，必要时重新查询全部有效补查来源；普通源索引更新后只重算受影响来源与当前库游戏，不更新未安装游戏的详情。

## 自动更新调度

- `set_source_update_interval(source_id, interval_hours)` 设置每源间隔（0 = 关闭；控件预设关闭/6h/12h/24h/3d/7d/14d，协议接受任意小时数）。默认全部关闭。
- 自动更新只在宿主运行期间执行：Decky 插件运行期间、Windows 应用运行期间；不安装系统服务，也不持久化计时器。启动/恢复后检查到期项，每源最多补一次，不补跑每个错过周期。
- `next_due_at = status.checked_at + interval_hours * 3600`；失败后下一次到期从本次尝试结束时间起算，不立即重试。禁用源不参与自动更新；所有补查来源均不参与全站自动更新。
- 同源手动与自动更新并发时宿主返回现有任务，不叠加重复工作；不同源可以并行/排队。

## 添加表格网站

在 Windows“全局来源 → 高级：添加来源规则”或 Decky 全局来源的“添加源”中填入 JSON 规则。Windows 标题旁的“示例”和 Decky 的“查看规则示例”均可查看并复制模板，不会覆盖正在编辑的规则。以下使用占位网站，需要替换为真实发行商域名及实际列布局：

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

列编号从 0 开始。标题与下载列必填；AppID、发布日期和语言列可选。没有语言列可使用规则的 `language`，默认为 en。只查询指定 HTTPS 页面、只下载指定域名的 ZIP；重定向也受该源域名列表约束。明确 AppID 优先于名字；没有 AppID 才使用完整标题与明确别名。单个源匹配多个不同 ZIP 且无法消歧时不自动选择。不同源的结果分开呈现，不共享允许下载的域名。

## 网站专用适配器

表格配置不能替代所有网站逻辑。自定义来源可使用下节的 Lua 适配器。四个内置来源与通用表格均由随包 Lua 脚本提供，旧 Rust 网站模块已退役；脚本错误直接报告。Lua 执行器内嵌在共享核心，两端不依赖外部 Lua/Python 解释器。

- **列表索引采集器**：patch.moe 与通用表格适配器读取列表页（支持条件请求），产出 `SourceIndexEntry`（title/aliases/page/kind/appid/url/language/date）；条目源为 `kind="entry"`。Kagura 适配器把列表分页解析为 `article/product/category` 链接。
- **详情解析**：Kagura 命中项由匹配任务调用 `resolve_page` 解析单页，生成 query 无关的 `PageFact`（标题、Steam AppID、镜像、正文、发布时间、分类内联文章等），写入按来源+规范 URL 共享的详情缓存。
- **HGamer 补查**：按游戏查询词搜索，使用 Steam AppID/有出处别名核实身份，返回候选与上游链接。
- **release_id**：patch.moe 使用 `patchmoe-<appid>`，其他源使用 `source-<source-id>-<appid>`。索引声明 ZIP 而官方链接为 EXE 时显示冲突，两者均不得扩展下载白名单。

新增适配器需用实际页面结构验证列表、详情、身份、语言、域名跳转与完整性；自定义脚本可直接导入，不必修改默认源配置。

## 导入与编辑 Lua 来源

在 Windows“全局来源”或 Decky“管理订阅源”点击“导入脚本来源”，分别粘贴规则 JSON、Lua API 版本（当前为 **1**）和 Lua 源码。“查看脚本示例”只展示可复制模板，不会替换草稿或自动添加来源。可下载/维护[规则示例](features/script-source-adapters/examples/publisher/source.json)和[源码示例](features/script-source-adapters/examples/publisher/adapter.lua)，具体字段与示例网页见[示例说明](features/script-source-adapters/examples/README.md)。Windows 文件选择和 Decky 远程文件浏览不属于当前入口；两端均可粘贴完整文本。

- 规则的 `adapter` 使用 `lua`，`id` 唯一，`role` 为 `publisher`（列表索引）或 `fallback_indexer`（按游戏补查）。`pages/page_hosts/download_hosts/root_layout` 等仍遵守普通来源合同；域名不是由脚本自行扩大权限。
- 只导入你信任的脚本。脚本在后续更新/匹配任务中运行，可按规则访问网站；核心不向它开放文件系统、进程、环境变量或安装器。脚本仍可报告错误网页事实，允许的宿主能力不代表作者可信或已经完成恶意代码资源隔离。
- “保存并导入”只验证规则、API 版本及 Lua 文本语法，**不执行顶层代码，也不验证网站可用性或入口返回结果**。语法错误保留完整诊断及当前输入。保存成功后显示“尚未执行”：发行商来源点击“手动更新”，然后匹配游戏；补查来源启用并订阅后，在普通来源覆盖完整且无可信发行商命中时参与重新匹配。
- 来源卡片显示 Lua、API 版本和完整源码 SHA-256。点击“编辑脚本”显式从核心读取规则与源码，修改后点击“保存并替换 <来源 ID>”。编辑器将替换绑定到该 ID；要创建另一个 ID，使用新导入入口。新导入碰到已有 ID 会报错，不会自动覆盖。
- 替换保留用户名称覆盖、启用、游戏订阅及更新间隔。规则、源码与摘要由核心一次事务保存；摘要按收到的原始 UTF-8 字节计算，不由前端指定。两端刷新来源与游戏投影，清除旧脚本结果；刷新失败明确显示“已保存，但刷新失败”，不会伪装成未保存。
- 脚本变化会使受影响索引、详情、匹配和旧安装选择失效。保留旧缓存文件不代表可继续授权旧结果；需要重新更新/匹配。普通扫描和读取源码不运行 Lua、不联网、不升级状态文件。
- 内置来源可直接“编辑脚本”；读取不执行或联网，保存需显式替换同 ID 来源。用户覆盖保留名称、启用、订阅和间隔，发行更新不会覆盖已保存源码；未编辑的内置脚本随核心更新，无需额外导入。

两端传输完整源码，不复用旧 Decky `add_source` 的 32768 字符检查，不另设源码/报告截断。编辑器的可视行数只影响滚动区域，不限制文本长度。JSONL 仍为 v5，能力表新增 `import_script_source/get_script_source` 与 `sources.script_api_version=1`；旧核心/客户端组合会明确报能力不兼容，应配套升级。设置文件已是 schema 3，旧二进制降级读取不在支持范围内。

### 编写与调试

脚本返回一个入口表，详见 [Lua API 与错误合同](features/script-source-adapters/p0-contract.md)：

| 入口 | 何时需要 | 返回事实 |
| --- | --- | --- |
| `collect_index(ctx, rule, previous)` | `publisher` 必须 | `{entries, pending, complete}`；直接下载条目用 `kind="entry"`，包含标题、来源页、AppID 或可核实名字、下载 URL；详情链接条目可用 `product/article/category` |
| `resolve_page(ctx, rule, page)` | 列表产生非 `entry` 条目时 | `PageFact`，提供标题、页面、Steam ID、别名、镜像等；核心生成并验证候选 |
| `discover(ctx, rule, query, previous)` | `fallback_indexer` 必须 | `{pages, identities, complete, diagnostics}`，返回页面/身份事实，不能直接授权安装候选 |

`ctx.get/post/form` 复用核心 HTTP、编码与取消；`ctx.html` 和 `node:select/text/attr/parent` 读取 DOM；`ctx.url_join` 处理相对链接；`ctx.array()` 表示空数组，`ctx.null` 表示 JSON null。分页/表单逻辑由脚本实现，完成后如实报告 `complete`；确有未完成页面时保留 `pending`。用户取消沿同一任务生效，不通过固定总时长停止正常工作。

调试顺序为保存语法检查 → 手动更新/重新匹配 → 查看任务完整诊断。`script_source_syntax_error` 是编译错误，`script_source_runtime_error` 是执行异常，`script_source_contract_error` 是返回字段/事实不符合合同；`script_source_digest_mismatch` 表示存储源码与描述不一致，`unsupported_script_api_version` 表示 API 不支持。保留来源 ID、入口及可取得的 Lua 行号，不把失败当作无补丁。示例域名是占位值，样例保存或离线夹具通过不等于真实网站执行通过。

## 安装与验证范围

安装默认保留包内相对路径到游戏根目录。获取时核对本地缓存哈希并向服务器重新校验：有 ETag / Last-Modified 时使用条件 GET，服务器返回 304 才复用已验证包；没有验证器则重新获取。之后固定 ZIP/EXE 哈希、当前 build 和原文件哈希，生成本机签名计划，由既有核心完成安全预检、确认、备份、提交与卸载。不同源、不同 Steam 库实例的计划与事务隔离；已有其他源的活跃安装阻止再次覆盖，以保留原始备份。

运行时查询遵守网站的实际可访问性。403、登录、验证码或网站结构变化显示查询失败或索引不完整，不会伪造匹配。真实网络验收与本地模拟页面测试分别报告。

网页解析由核心 CLI 内嵌的 Lua 执行器完成（随插件打包为 `bin/patchhub-core`），插件运行时不再包含 Python HTML 解析模块，也不需要 `/usr/bin/python3` 解析工作进程。下载仍验证 HTTPS 证书，使用系统证书库。
