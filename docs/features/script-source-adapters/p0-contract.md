# P0：Lua API v1 与接入合同

[设计](design.md) · [施工记录](progress.md) · [迁移矩阵](migration-matrix.md)

状态：**P0 原型验证通过，P1 已实现核心导入/读取；执行分派与客户端入口待后续阶段，当前客户端尚不能导入或运行脚本。** 原型位于 [script_runtime_prototype.rs](../../../crates/core/tests/script_runtime_prototype.rs) 及其 [测试宿主](../../../crates/core/tests/support/script_runtime.rs)，没有注册到生产适配器。

## 已验证的运行时决策

- 固定 `mlua = 0.12.2`，使用 `lua54/vendored/serialize`，不启用 `send/async/module`。锁定的 `mlua-sys 0.13.0` / `lua-src 551.0.2` 实际编译 Lua **5.4.9**；Rust 1.94 的 Windows MSVC、WSL Ubuntu x86_64 均能编译并运行同一组测试。
- P0 时依赖仅放在 core 的 `dev-dependencies`；P1 已将 `mlua` 移为正常依赖用于语法检查，原型也复用同一编译检查。`ego-tree 0.11` 仍为开发依赖，是已有 `scraper` 树的节点类型，不是另一套 DOM 引擎。P2 再将测试宿主的执行能力移入产品模块，不能复制成两套长期维护的执行器。
- 一次调用一个 VM。显式开放 `assert/error/ipairs/next/pairs/select/tonumber/tostring/type`、受控 `pcall/xpcall`、`string/table/math/utf8` 与 `_VERSION/_G`。去掉 `string.dump`；没有 `io/os/package/debug/ffi/require/dofile/loadfile/load/coroutine/collectgarbage/print`。
- 代码只以 `ChunkMode::Text` 编译。导入只编译，不调用生成的函数；顶层报错和无限循环均不会在语法检查时执行。入口在实际任务中从模块返回表读取，缺失入口明确失败。
- `scope` 借用当前 `RequestContext`，作用域退出后回调不可再调用；不延长引用生命周期、不增加线程共享网络栈。DOM 节点以 `Rc<Html> + NodeId` 保持原文档存活，不跨 VM 保存句柄，不依赖自引用或 `unsafe`。

## 取消合同

模块顶层和入口执行都挂 hook，检查本任务 `Arc<AtomicBool>`；原型每 10,000 条 VM 指令采样一次，这是取消响应参数，不是指令总量限制。

**不能仅靠 hook 报错或 hook yield。** P0 曾验证 hook yield 能退出普通受保护循环，但实际构造的 `string.gsub` C 回调内嵌套 `pcall` 循环不能让出。该实验被中断，未保留成正常产品路径。

最终方案使用 hook 报错，同时开放包装后的 `pcall/xpcall`：调用私有原始函数前后检查取消。原始函数只在宿主闭包中保存，脚本无法取得；取消时外层保护调用也必须继续向上传播，不能返回假成功。Rust 在模块/入口返回及 DTO 转换后再次检查。普通 Lua 的多返回值、错误返回与 `xpcall` 处理器语义已有回归。

验证包括顶层无限循环、`pcall/xpcall` 循环、C 回调及错误处理器中的嵌套循环、捕获网络取消后立即返回成功及继续循环。网络沿用 `operation::with_cancel` 的同一标志。

纯 C 长调用仍不能在内部被 VM hook 打断。64 MiB `string.rep` 最终一轮实测 Windows 185 ms、WSL 483 ms，返回时均拒绝发布结果，Lua 保留分配约 64 MiB。该测量只说明这次输入的成本，**不是取消延迟上限或内存隔离证明**；没有据此增加内存、脚本大小或总时长限额。

## 脚本模块与入口

模块返回普通表：

```lua
return {
    collect_index = function(ctx, rule, previous) ... end,
    resolve_page = function(ctx, rule, page) ... end,
    discover = function(ctx, rule, query, previous) ... end,
}
```

发行商必须实现 `collect_index`；产生非 `entry` 索引条目时必须实现 `resolve_page`。补查源必须实现 `discover`，不运行全站更新。`rule` 是当前任务捕获的有效规则副本；修改脚本侧副本不改变 Rust 的权限或设置。

| 入口 | 输入与输出合同 |
| --- | --- |
| `collect_index` | `previous` 为已有索引/进度中的 `entries/pending/complete/etag/modified`，无记录时为空值；返回 `{entries, pending, complete, etag?, modified?, not_modified?, diagnostics?}`。条目复用 `SourceIndexEntry`；时间、修订和源 ID 由核心生成。`not_modified=true` 必须已有同解析身份的正式索引。 |
| `resolve_page` | 输入绝对页面 URL；返回既有 `PageFact` 字段：`title/page/kind/steam_ids/aliases/mirrors/text/published_at/body_links/article_links/article_facts/next_page`。不得返回候选授权或安装对象。 |
| `discover` | `query` 提供 `appid/name/name_hints/terms/force`；`terms` 由公共身份算法生成。`previous` 提供已有候选的事实副本。返回 `{pages, identities, complete, diagnostics}`；`pages` 每项为 `{fact: PageFact, reported_format?, upstream_page?}`，`identities` 每项为 `{appid, page, aliases}`。核心仍调用公共身份、格式、候选生成和校验逻辑，不接受脚本直接返回 `SourceCandidate`。P4 验证此入口，P0 未声称 HGamer 已迁移。 |

`diagnostics` 每项只含 `code/detail?`；宿主绑定 `source_id`。空数组、缺省可选项按字段合同处理。脚本异常没有返回输出时不保存虚构的新续跑状态，保留既有正式索引和进度。返回明确的部分结果时才使用 `pending`；不保存 Lua 栈或局部变量。

## ctx 与 DOM

所有 `ctx` 函数使用点调用；DOM 节点使用冒号调用。HTTP 返回 `{status, text, url, etag?, modified?}`，`url` 为重定向后的地址，文本由现有编码解码器生成。

| API | 参数及语义 |
| --- | --- |
| `ctx.get(url, validators?)` | `validators` 含 `etag?/modified?`；沿用条件请求、页面域名、Cookie、重定向与取消。允许脚本处理 304 等响应；HTTP 状态不会自动伪装成成功 HTML。 |
| `ctx.post(url, content_type, body)` | POST 文本正文；相同域名与网络策略。 |
| `ctx.form(url, fields)` | `fields` 为 `{{name, value}, ...}`，保留重复表单字段及次序；宿主执行 URL 编码。 |
| `ctx.html(text)` | 返回文档根节点。 |
| `node:select(css)` | 后代元素序列，CSS 无效即报错，不返回空成功。 |
| `node:text()/attr(name)/tag()/parent()` | 合并空白文本、可选属性、标签、可选父节点；不存在的属性/父节点是 `nil`。 |
| `ctx.url_join(base, reference)` | 相对 URL 合并；执行请求时仍由捕获的规则检查域名，不信任脚本传入的 hosts。 |
| `ctx.json_decode(text)/json_encode(value)` | JSON 与普通值转换，见下述值合同。 |
| `ctx.null / ctx.array()` | JSON null 哨兵 / 显式空数组；普通 `{}` 表示空对象。 |
| `ctx.progress(message)/check_cancel()` | 消息/取消检查；来源 ID、任务与设置版本由宿主绑定。 |

`ctx` 不提供任意请求头、文件路径、环境变量、浏览器凭据、原生指针或安装器对象。后续确有站点需要补充 API 时先以真实样例修改合同。

## 值与错误边界

Lua 整数为有符号 64 位，Steam `u32` ID 可精确往返。JSON 整数大于 `i64::MAX` 必须明确报转换错误，不能先转浮点损失精度；网站此类 ID 应使用字符串。浮点值必须有限；不能把无穷值编码成 null。核心的修订、选择和安装授权不通过 Lua 数值往返。

P0 发现 `mlua` 默认转换可能忽略数组表的字符串键，而 `detect_mixed_tables` 会将空对象识别为数组。宿主在边界检查表结构，再做类型转换：数组必须连续，混合键、循环、错误字段类型及非有限数明确报错。该检查对应实际序列化损失，不是限制合法条目数。JSON 空数组以 metatable 保留；对象、数组、中文、emoji、null、`u32::MAX/i64::MAX` 均有往返测试。上游 JSON/serde 的递归深度能力仍是底层解析器约束，不能据此声称支持任意深度树。

P2 的外部错误固定为 `script_source_syntax_error`、`script_source_runtime_error`、`script_source_contract_error`、`script_source_digest_mismatch`、`unsupported_script_api_version`；`detail` 包含来源 ID、入口及底层 Lua 文件/行号和原因。取消统一为现有 `task_cancelled`。P0 测试宿主仍直接返回底层 `mlua::Error`，生产错误封装须在 P2 落实。

## P1/P3 持久化与 P5 传输决策

接口参数：`import_script_source {rule: RawSourceRule, api_version: 1, code: UTF-8 string, replace: false}`；成功复用 `{rule: SourceRule, settings_revision}` 形状。`get_script_source {source_id}` 返回 `{rule: RawSourceRule, api_version, code, sha256}`，作者规则不含由核心生成的描述。导入者不能指定摘要。

`SourceRule.script` 是可选 `{api_version, sha256}`，未脚本化的规则省略字段。`sha256` 为源码原始 UTF-8 字节摘要，不改换行或自动格式化。设置增加默认空的 `script_sources: source_id -> code`；规则、源码、摘要、revision 在锁内以一次完整文件替换提交，成功后才更换内存快照。现有 `apply_overrides → bump_revision` 是分开更新内存/落盘，不能直接当作完整设置事务。

P1 已选定设置文件 schema **3**，命名空间 marker 保持 **2**，索引/详情/匹配文件不变。读写打开一次升级旧设置；只读快照不升级、不运行脚本。已验证旧缓存文件保留、写失败不改设置字节或内存，以及多个 writer 不覆盖新脚本。现有来源 schema-2 规则 JSON 与来源 ID 保持。旧二进制不能读取 v3 设置；P5 的能力校验和升级说明必须明确此边界。

P3 至少给索引增加默认缺失的 `parser_fingerprint`，只在有效来源已脚本化时写入，值为包含脚本描述的规则解析标识；旧无标识索引不能授权脚本来源。`content_revision` 必须比较此标识，之后复用现有 `index_revision` 传播到详情与游戏匹配。无需给每种 DTO 另添代际或 TTL。已有名称覆盖只用于列表显示，不合并回运行规则，继续保持名称编辑不改变解析身份。

JSONL 保持 **v5**，Lua API 单独为 **v1**。P1 已同步 `SourceRule.script` 可选描述的生成 TS/Python/JSON Schema 与 Flutter 资源，并验证现有契约；导入/读取 RPC 方法、两端编辑器与新能力表仍归 P5。当前 `validate_capabilities` 精确比较方法表，P5 新旧客户端/核心组合应报能力不兼容，不偷偷使用旧入口。

## P0 证据范围

录制信息见 [recording.json](../../../crates/core/tests/fixtures/script-sources/recording.json)。三个真实 HTTP 200 页面保留相关 DOM 片段，不将导航、JS、图像资源全部放入仓库。列表与详情在同一夹具上与现有 Rust 解析器对照相等。真实详情没有 Steam app 链接，返回空 ID；Steam ID 提取及分页终止分别用明确标记的合成输入补充测试，不能冒充真实站点的完整抓取或身份确认。

四个内置来源及 `table_html` 的生产迁移均仍待 P6，见 [迁移矩阵](migration-matrix.md)。通过原型不代表导入、缓存失效、安装、双端界面、发行包或设备验收已完成。
