# 源适配器与订阅

来源能力已迁入共享 Rust 核心并作为单一来源引擎（P0–P5 已在 Windows 与 Decky 实机验证，P6 收口）：[设计](features/shared-source-discovery/design.md)、[施工记录](features/shared-source-discovery/progress.md)、[P0 基线](features/shared-source-discovery/p0-baseline.md)、[P5 契约](features/shared-source-discovery/p5-decky-design.md)。下文描述现行 Rust 入口与用户可见行为。

工具订阅网站，不订阅固定的游戏清单。工作单位是“来源”而不是“游戏”：每次来源更新只遍历该来源自己的列表/目录页并发布共享索引；游戏匹配先读本地索引，命中后才按需获取相关详情。普通源索引不会因为新增游戏、重启或手动重匹配而重新遍历。只有具备 ZIP/EXE 安装能力的匹配结果进入安装流程；仅相似标题的候选仍需人工选择。

## 配置和状态

默认订阅由核心内置 `crates/core/resources/sources.json`（编译进 CLI，单一来源）。用户启用状态、新增源、逐游戏订阅与每源自动更新间隔写入核心命名空间 `~/.local/share/steam-patch-manager/sources/rust-v1/`：

```text
sources/rust-v1/
  schema.json                 存储 schema 2 完成标记
  settings.json               启用/新增/逐游戏订阅/intervals
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

## 增量匹配与 HGamer 补查

- `match_games`（可指定实例/AppID、`force_details?`）与 `rematch_game` 复用已发布索引和持久化详情：本地按 AppID 优先、完整标题/别名其次筛选；只对命中条目获取详情，详情按来源+规范 URL 跨游戏共享。
- 匹配输入（游戏名、本地化名、名称提示、语言偏好、相关源订阅）未变且来源修订未变时直接读取 `matches/<appid>.json`，不联网。
- 结果状态为六态：待匹配、普通源命中、HGamer 命中、完整无结果、未完成/失败；索引或详情不完整时不冒充“确认无补丁”。
- HGamer 仅在无普通源可信命中且所有启用的普通源覆盖完整时按游戏查询（查询结果复用）；普通源命中优先显示并隐藏 HGamer；普通源未初始化/不完整时显示“未完成”，不声称 HGamer 查无结果。
- 手动重匹配（含已命中、无补丁、失败游戏）只重算该游戏：复用普通源索引、刷新命中详情，必要时重新查询 HGamer；普通源索引更新后只重算受影响来源与当前库游戏，不更新未安装游戏的详情。

## 自动更新调度

- `set_source_update_interval(source_id, interval_hours)` 设置每源间隔（0 = 关闭；控件预设关闭/6h/12h/24h/3d/7d/14d，协议接受任意小时数）。默认全部关闭。
- 自动更新只在宿主运行期间执行：Decky 插件运行期间、Windows 应用运行期间；不安装系统服务，也不持久化计时器。启动/恢复后检查到期项，每源最多补一次，不补跑每个错过周期。
- `next_due_at = status.checked_at + interval_hours * 3600`；失败后下一次到期从本次尝试结束时间起算，不立即重试。禁用源不参与自动更新；HGamer 不参与全站自动更新。
- 同源手动与自动更新并发时宿主返回现有任务，不叠加重复工作；不同源可以并行/排队。

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

列编号从 0 开始。标题与下载列必填；AppID、发布日期和语言列可选。没有语言列可使用规则的 `language`，默认为 en。只查询指定 HTTPS 页面、只下载指定域名的 ZIP；重定向也受该源域名列表约束。明确 AppID 优先于名字；没有 AppID 才使用完整标题与明确别名。单个源匹配多个不同 ZIP 且无法消歧时不自动选择。不同源的结果分开呈现，不共享允许下载的域名。

## 网站专用适配器

表格配置不能替代所有网站逻辑。分页、语言表单、产品/分类详情等需要自己的适配器。适配器由核心以受维护的 Rust 实现注册（`crates/core/src/sources/adapters/` 的 `patchmoe.rs`、`table.rs`、`kagura.rs`、`hgamer.rs`）；规则只引用适配器名称，不下载或执行远端解析代码，核心不依赖 Python 运行时。

- **列表索引采集器**：patch.moe 与通用表格适配器读取列表页（支持条件请求），产出 `SourceIndexEntry`（title/aliases/page/kind/appid/url/language/date）；条目源为 `kind="entry"`。Kagura 适配器把列表分页解析为 `article/product/category` 链接。
- **详情解析**：Kagura 命中项由匹配任务调用 `resolve_page` 解析单页，生成 query 无关的 `PageFact`（标题、Steam AppID、镜像、正文、发布时间、分类内联文章等），写入按来源+规范 URL 共享的详情缓存。
- **HGamer 补查**：按游戏查询词搜索，使用 Steam AppID/有出处别名核实身份，返回候选与上游链接。
- **release_id**：patch.moe 使用 `patchmoe-<appid>`，其他源使用 `source-<source-id>-<appid>`。索引声明 ZIP 而官方链接为 EXE 时显示冲突，两者均不得扩展下载白名单。

新增适配器需用真实页面结构及新增游戏、重复名称、失败缓存、语言、域名跳转、分页续跑等测试验证，随后在默认源配置中引用。

## 安装与验证范围

安装默认保留包内相对路径到游戏根目录。获取时核对本地缓存哈希并向服务器重新校验：有 ETag / Last-Modified 时使用条件 GET，服务器返回 304 才复用已验证包；没有验证器则重新获取。之后固定 ZIP/EXE 哈希、当前 build 和原文件哈希，生成本机签名计划，由既有核心完成安全预检、确认、备份、提交与卸载。不同源、不同 Steam 库实例的计划与事务隔离；已有其他源的活跃安装阻止再次覆盖，以保留原始备份。

运行时查询遵守网站的实际可访问性。403、登录、验证码或网站结构变化显示查询失败或索引不完整，不会伪造匹配。真实网络验收与本地模拟页面测试分别报告。

网页解析由核心 Rust CLI 完成（随插件打包为 `bin/steam-patch`），插件运行时不再包含 Python HTML 解析模块，也不需要 `/usr/bin/python3` 解析工作进程。下载仍验证 HTTPS 证书，使用系统证书库。
