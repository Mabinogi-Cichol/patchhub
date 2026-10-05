# Kagura / HGamer 方案可行性校验

- 日期：2026-10-04
- 设计：[design.md](design.md)
- 实施状态：[progress.md](progress.md)
- 结论：**双站补丁发现、HGamer 逐游戏兜底和来源页面引导可实施；原方案有需要修正的接口及采集假设，已回写设计。功能尚未实现，自动安装 EXE 不在本次结论内。**

## 校验方法与范围

对照当前仓库的源注册、单游戏订阅、扫描 RPC、前端状态、安装任务和打包代码；使用插件现有 `SourceRegistry._fetch` 进行只读 HTML 请求，再以 Python 标准库 `html.parser` 编写最小解析实验。没有下载补丁载荷、执行安装器、修改游戏文件或部署 Steam Deck。

本轮保存了 11 份页面样本：其中 10 次通过现有抓取函数取得 HTTP 200，另一次繁体查询使用 curl 保存 HTML 并验证真实结果。实验脚本、响应和摘要在本机忽略目录 `.artifacts/kagura-feasibility/`，不作为插件资源提交。它们证明当前页面可读取和关键结构可解析，不等同于完整适配器、全站覆盖或设备验收。

## 真实页面实验

| 页面 / 用例 | 观察结果 | 对方案的影响 |
| --- | --- | --- |
| [海外补丁列表](https://www.kaguragames.com/patch/) | HTTP 200；HTML 2,385,014 字节；产品卡区域提取 227 个不同产品链接，含本体与扩展 | 原提议 2 MiB 会拒绝真实页面；改为现有 4 MiB 上限并准备受限分页入口，不能假设该入口每页只有21条 |
| [海外本体详情](https://www.kaguragames.com/product/summer-memories-patch/) | HTTP 200；3 个 Mirror：两个 EN v3 EXE URL，一个 Google Drive 页面 | 可发现镜像及语言版本；不能按 ZIP 安装，也不能据 URL 后缀推断安装器内部结构 |
| [国内首页](https://kagurafan.com/) | HTTP 200；当前主列表只有10篇文章；分类列表仍含本体及“+” | 首页文章不能代表完整历史目录；必须利用分类 / 归档继续定位旧游戏 |
| [国内本体详情](https://kagurafan.com/2020/06/14/%e5%a4%8f%e6%97%a5%e7%8b%82%e6%83%b3%e6%9b%b2%ef%bc%9a%e4%b9%a1%e9%97%b4%e7%9a%84%e9%9a%be%e5%bf%98%e5%9b%9e%e5%bf%86/) | HTTP 200；两个 CN v1.03 EXE 链接；正文含本体 / DLC 适用说明 | 采用用户更正后的地址；保留条件，不能与“+”合为同一个通用补丁 |
| [HGamer 夏日狂想曲搜索](https://hgamer.tw/zh-CN/search?q=%E5%A4%8F%E6%97%A5%E7%8B%82%E6%83%B3%E6%9B%B2) | HTTP 200；重定向到同站 zh-TW；真实结果组为1游戏、2补丁 | 支持同站语言重定向；仅解析结果组 |
| HGamer 随机不存在标题搜索 | HTTP 200；真实结果0条，但页面仍有8条推荐链接 | 扫描页面所有 `/games/`、`/patches/` 链接会产生假阳性；只按“游戏 / 遊戲”和“补丁 / 補丁”标题所在的结果容器取链接 |
| [HGamer 本体游戏页](https://hgamer.tw/zh-CN/games/summermemories) | HTTP 200；明确 Steam AppID 1227890 | 可补全跨语言身份；补丁适用本体 / DLC 仍须分别核对 |
| [未内置发行商游戏页](https://hgamer.tw/zh-CN/games/steam-4483210) | HTTP 200；发行商 azucat；Steam AppID 4483210；含英文别名及补丁页 | 兜底不必局限于 Kagura；样本仅用于验证，不加入生产游戏清单 |
| 以上游戏用简体完整标题搜索 | HTTP 200；“新搬来的大姐姐真是太可爱了”真实结果0条 | 简体零结果不能直接判定游戏没有补丁 |
| [同标题繁体搜索](https://hgamer.tw/zh-CN/search?q=%E6%96%B0%E6%90%AC%E4%BE%86%E7%9A%84%E5%A4%A7%E5%A7%90%E5%A7%90%E7%9C%9F%E6%98%AF%E5%A4%AA%E5%8F%AF%E6%84%9B%E4%BA%86) | 保存 HTML 中提取到目标游戏及补丁两条真实结果 | 首期查询策略需要简繁体变体或已核实别名；简繁体转换只能扩展查询，不单独作为身份匹配证据 |
| [该游戏补丁详情](https://hgamer.tw/zh-TW/patches/the-too-cute-girl-who-moved-in-next-door-patch) | HTTP 200；提供 azucat 官方补丁入口及 Steam 链接 | 能给未知发行商提供来源引导；官方入口仍不等于可直接安装的文件 |

第一次“简体完整标题应命中未知发行商”断言失败；改用繁体查询后目标游戏与补丁均命中。这项失败保留为方案修订依据，不能描述为原查询策略已全部通过。

## 当前代码约束与已修订内容

| 问题 | 代码依据 | 设计修订 |
| --- | --- | --- |
| 当前源条目只接受可安装 ZIP | [sources.py](../../../apps/decky/backend/sources.py) 的规则和条目校验、[publisher.py](../../../apps/decky/backend/publisher.py) | 独立 SourceCandidate 与展示联合类型；发现候选不伪装成 release_id；所有安装 RPC 都隔离此类型 |
| 扫描先刷新来源，前端一次性接收完整结果 | [main.py](../../../apps/decky/main.py)、[session.ts](../../../apps/decky/src/session.ts) | 首期扫描读缓存，选中游戏后调用单游戏发现 RPC；明确超时、取消、返回协议及单行合并 |
| 安装 TaskManager 有互斥和 UI 租约 | [tasks.py](../../../apps/decky/backend/tasks.py) | 发现使用独立调度，不能直接复用安装任务而阻塞安装或在 QAM 关闭后失去租约 |
| 当前已支持单游戏退订 | [sources.py](../../../apps/decky/backend/sources.py) 的 `game_subscribed`、`set_game_source` | schema 迁移保留 overrides.games；全局与单游戏开关共同约束查询、fallback、委派和缓存展示 |
| `force=False` 没有自动提供 TTL | [sources.py](../../../apps/decky/backend/sources.py) 的 refresh / collector | 新增显式 freshness 检查，TTL 内实际 HTTP 请求数必须为0；失败不覆盖成功缓存 |
| 本地化名称目前仅用于 UI；语言与 DLC 信息不完整 | [steam.ts](../../../apps/decky/src/steam.ts)、[共享 UI 类型](../../../packages/ui/src/index.ts) | RPC 接收有限名称提示；AppID 从后端实例取；偏好语言不等于已安装语言，DLC 未知须保留待确认 |
| 默认“有补丁”过滤隐藏冷缓存游戏；unknown 可能全落入需处理 | [共享 UI 类型](../../../packages/ui/src/index.ts)、Decky 列表状态 | 提供未查询游戏入口；已核实来源引导计入有补丁，正常 discover_only 不因 unknown 全部判为需处理 |
| 旧采集 worker 的输出和生命周期不适合单游戏发现 | [sources.py](../../../apps/decky/backend/sources.py) | 定义固定 `--discover`、受限 JSON 输入输出、父进程独占缓存写入及 generation / settings_revision 校验 |
| 新增 Python 依赖不一定存在于 Deck 系统解释器 | [package-plugin.mjs](../../../apps/decky/scripts/package-plugin.mjs) | HTML 可用标准库解析；简繁体转换器与字典必须随包提供，实施时确定依赖和许可证并做隔离 Python smoke test |

此外，候选去重保留语言、补丁类型、版本和 DLC 条件；30秒游戏查询期限包含冷索引采集，不能同步遍历整个站点再返回。结构改变、查询词未全部完成或分页未完成时返回 partial，不能写入游戏级“无结果”缓存。

## 后台增量更新的定位

这里的后台增量更新指先展示本机游戏列表，再随网络查询完成逐项补充补丁结果。它解决的是首次无缓存、大游戏库或站点较慢时的等待；文件断点续传解决的是补丁文件下载中断后从已有字节继续下载，两者无关。

本功能首期采用“扫描读缓存 → 选中游戏 → 查询该游戏 → 合并该行结果”，不要求自动遍历全库。自动批量发现和持续进度推送保留为后续体验优化。索引分页检查点也只记录采集进度，不涉及补丁文件下载。

## 验证结果与剩余门槛

- 已核验：真实 HTML 可读取；海外产品卡 / 镜像、国内本体链接、HGamer 真结果容器及 AppID 可解析；空搜索推荐卡可排除；简体零结果 / 繁体命中的差异可复现。
- 已检查：修订文档的本地相对链接与 diff 空白检查。未运行功能单元测试、前端构建或 Rust 测试，因为本轮没有修改实现。
- 待实施验证：完整解析器与 fixtures、候选 DTO、RPC、TTL / 失败缓存、查询去重和取消、退订竞态、简繁体依赖打包、安装 RPC 拒绝发现候选、旧 patch.moe 回归。
- 待实机验证：Steam Deck 网站访问、冻结 Python / 系统 Python 运行、实际库名称和 DLC 条件、QAM 交互与性能。开发机 HTTP 200 不等于 Deck 网络必然可访问。
- 范围外：补丁载荷真实性、EXE 安装器结构、实际安装 / 启动 / 生效 / 恢复；本次没有执行这些操作，也不据此授予自动安装能力。

按修订后的 P0–P5 推进即可。上线前应以实现测试与设备证据更新施工记录，不能把本报告的“可实施”写成“已实现”或“已实机通过”。
