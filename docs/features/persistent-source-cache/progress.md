# 来源缓存持久化与重新查找修复施工记录

- 设计与修复计划：[design.md](design.md)。
- 状态：P1–P4 已实现并完成自动化/构建/打包；P5 已正式部署并完成真实 RPC / 页面验证。真实游戏补丁安装及实体手柄待验证。

| 阶段 | 状态 | 验证 |
| --- | --- | --- |
| P1 缓存持久化与准入解耦 | 已实现 | Windows / Linux 注册表与投影回归通过 |
| P2 主动查询与身份保留 | 已实现 / 实机查询通过 | 主动 force、旧请求隔离、失败保留及索引继续推进回归通过；真实页面主动查询取得国内候选，普通扫描保留正结果 |
| P3 中文提示与候选选择 | 已实现 | 双端类型/构建通过；人工 ZIP/EXE resolver、实例/内容/来源绑定及临时目录 ZIP 事务通过 |
| P4 自动化与打包 | 已完成 | Linux core/CLI 回归、最终 release 构建与生产 ZIP 逐文件校验通过 |
| P5 Steam Deck 验证 | 已部署 / RPC与界面验证通过 | 用户解锁后完成；首次完整备份、正式 Loader ZIP 安装、24 文件校验、真实主动查询/普通扫描/人工确认/滚动通过；真实游戏安装与手柄待验证 |
| 独立 Summer DLC 安装器兼容 | 待调研/修复 | 待验证 |

## 2026-10-14

- 用户要求取消来源缓存时间过期机制，并制定修复计划。
- 沿用本会话已完成的 Steam Deck 排查证据：旧缓存触发 selection_stale；按钮仅 scan(false)；真实查询后国内同候选身份降级；local_archive_signed_only 未翻译；当前 DLC 候选有历史 exe_project_options_unsupported。
- 阅读当前共享 Rust 注册表、resolver、discovery session 和 Decky/Windows 查询入口，建立持久化缓存、真实主动查询、身份保留及验收拆分方案。
- 创建本功能目录及索引。本次未修改插件代码、构建、重新连接/部署设备或安装游戏补丁；既有排查不是新方案实机验收。
- 两份新增文档的本地相对链接检查通过；`git diff --check` 通过（仅有 Git 的 LF/CRLF 提示）。功能测试、构建和实机验收待实施后执行。

## 待办

- 实体手柄焦点/返回与真实游戏补丁安装另行验收；本轮未点击安装按钮。
- 安装器兼容单独处理；不能把取消时间限制视为解决 DLC 分析失败。
- 保留既有来源缓存、设置、任务、事务与游戏数据；本计划不要求清空数据。

### 补充需求与施工开始

- 用户授权开始施工；此前仅完成代码阅读，尚无插件代码改动。
- 用户要求自动匹配失败不能封死手动安装：已在设计补充人工候选/下载项选择、可信游戏实例绑定、未知条件确认、共享后端准入与验收矩阵。
- 手动选择不伪造候选身份，且需覆盖后端/任务/计划绑定，不能只放开前端按钮。


### 实施与验证结果

- 取消共享来源成功记录、共享索引和普通来源缓存的年龄失效；保留结构/解析器/编码兼容性与设置修订校验。旧日期兼容记录不再产生时间类 selection_stale；30 天旧缓存仍可投影与解析。
- Decky 重新查找进入当前游戏 discover_game(force=true)，携带当前名称/语言；失败保留旧结果。按 force/语言与请求 owner 隔离同游戏并发，旧请求不覆盖新结果。
- 已知官方详情优先查询；同候选且版本/语言/variant/DLC 等条件不变时沿用已确认身份。force 保留共享索引与 pending，先推进旧分页再补查首页，修复每次从头爬导致永远查不到深层游戏的问题。
- 新增 manual_selection：由共享核心从当前缓存推导人工候选与具体下载项的选择标识，哈希绑定可信目标实例、AppID、候选内容和下载 URL/格式。候选自身 AppID/身份/variant 不被篡改；执行前重新解析，沿用订阅/下载/路径/worker/内容授权/事务链。
- 仅标题匹配、缺 AppID、未知 variant 的 ZIP/EXE 可以人工选择；没有版本号不成为安装准入条件。不同下载文件/格式可单独选择，两端明确确认人工选择，并显示所选文件名/标签/URL以及版本、语言和 DLC 条件。已明确矛盾的 AppID 不按未知放行；执行器不支持的格式仍准确拒绝。
- 自动匹配与人工选择分开显示，人工候选不显示“可自动安装”；优先本体候选且保留已有用户选择。补齐 local_archive_signed_only 中文，并将结构不兼容提示与时间过期区分。
- PatchMatch 增加可选 download_url/download_label 输出字段；Rust 生成 protocol.ts/protocol.py/v4.schema.json，生成契约 check、共享 UI fixtures 与桥接投影 3 项测试通过，无新增安装请求参数。
- Windows 核心完整回归通过；之后的新增 sources 回归 138 passed / 1 ignored、投影 23 项、人工临时目录 ZIP 写入与 selection 事务记录测试通过。最后分页回归 5 项全部通过。
- Linux core/CLI 全套回归通过（日志中 core 单元 361 passed / 6 ignored，CLI 单元 46 passed；各集成套件通过；忽略项包含真实下载与未启用的真实 worker 测试，不算验收）。最后代码变更后补跑 Linux Kagura 5 项与 manual_unknown 投影 1 项，最终 release 编译通过。
- Decky 66 项测试、双端 TypeScript、Windows 状态与 action/outcome fixtures、共享契约 fixtures、Decky Rollup 和 Windows Vite 构建均通过。
- multi-agent 技能要求的只读独立复核：1 名 reviewer，gpt-6.1-sol/high。已修复其报告的兼容旧 success 遮蔽新 attempt、直查清空索引、未知 ZIP 被误判自动准入、混合下载选择与详情展示问题；最终 findings=[]，未代替实机验收。
- 新插件 ZIP 已生成并冻结在忽略目录 `.artifacts/summer-memories-diagnosis-20261014/steam-patch-manager-decky.zip`；SHA-256 `cf7d9b571ea429430b29805f4309afe6e53bea87548f88e69dea4a952f5f53c4`，逐文件字节与生产目录一致，Linux ELF 与可执行权限校验通过，清单见同目录 manifest.json。
- 部署前重新核对当前 Devkit 配对地址与密钥路径，SSH 连接超时；一次复核仍超时，且 Devkit selected_devkit 已返回未选中设备。按部署技能停止远端操作。本轮没有上传、插件替换/重载、游戏补丁安装/卸载或订阅修改，也未创建 Git 提交。
- Summer DLC 的 exe_project_options_unsupported 仍为独立待修项，本轮未声称其安装成功。真实人工 EXE 安装、游戏内生效、实体手柄和新来源查询网络行为待设备恢复后验证。


### 用户解锁后的正式部署与实机验收

- 用户说明设备刚才锁屏并已解锁。重新核对 Devkit 所选 steamdeck 与实际配对密钥，已有可信 known_hosts SSH 成功；plugin_loader active。正式目录 root:root，使用 Loader 正式 ZIP 安装，不修改目录所有权。
- 部署前 list_tasks 全部为终态；首次全插件备份 24 个文件已取回，backup SHA-256 `1b40459f2ef7ba1282d2a4503f2e5208cdb6f4decbd4f953c4b913884425bbb4`；回滚 ZIP SHA-256 `f8adc39aec52c93ef1f3238a16c9ff9c6cd3ffb812b2b4688463a5b4a7d71e81`。两轮更新均保留该首次备份，不将中间版本当原始回滚材料。
- 第一轮部署后正式文件与实际 HTTP bundle 均正确，但旧 Steam React 页面未释放；正常 importPlugin 超时且实例暂为 0，已记录失败现场。在无活动任务并保存基线后重启 Steam JS 上下文，未重启整机；新资源与唯一插件实例恢复，真实页面显示新版文案与人工候选。
- 实机追加发现：已找到但仅 title 匹配的国内正结果被当成“未确认/负缓存”，普通扫描的英文名称提示改变 query_key 后会自动重查并把候选清空。修复 needs_refresh：非空且兼容的成功结果都持续保留，不因提示变化重查；只有空结果上下文变化、结构不兼容、首次无记录或主动 force 才重查。
- 新增 title_only_positive_cache_survives_name_hint_changes_without_network_refresh；Windows 注册表 32 项通过，Linux 新回归通过，更新 release 核心与正式插件包；独立 reviewer 对此最后修改复核 findings=[]。前端哈希未变，第二轮正式 Loader 更新后正常使用已加载的新前端。
- 最终 ZIP SHA-256 `84e4c4338532851762c195eea7525e85428408bc351c8e864a342758fad5965f`（7,115,031 字节）；初次部署包另保留为 deployed-v1.zip。最终正式 24 文件逐项匹配冻结清单；CLI/worker 为 755，plugin_loader active。实际 HTTP index.js 200 / 110,535 字节 / SHA-256 `0f8170e530f8d8d43d708894cf46e000ac883ab382ee78c2da88b6b07f623100`，与构建和正式目录一致。
- 通过真实 QAM 游戏按钮打开夏日页面，点击页面“重新查找补丁”，确实出现单游戏查询进度并取得国内本体 v1.03。该候选 AppID 仍为 null / title，没有伪造自动匹配；页面显示“可手动选择补丁”、具体文件/下载标签/URL/语言/本体和排除 DLC 条件。
- 选择真实国内候选后勾选人工确认，安装按钮 Disabled 状态解除；随后恢复为未勾选/Disabled。未点击安装，截图和 DOM 证据分别保存。原“补丁选择信息已过期”与裸 local_archive_signed_only 不再出现在该候选详情。
- 点击真实 QAM“重新扫描”，国内成功记录 checked_at 仍为 `1791914881`，候选保留且 QAM 继续显示“可手动选择补丁”；证明名称提示改变的普通扫描未重新获取或清空已有 title 正结果。
- 真全屏 CSS viewport 854×534，实际截图 1280×800；滚动实际 DialogContent 到 scrollTop 1016 / scrollHeight 1550，安装按钮 bounds 347.73–387.73 位于视口内，内容无横向溢出。已通过 view_image 检查真实底部截图的文本换行与控件可见性；实体手柄不据此标为通过。
- 最终实例 1、活动任务 0、历史任务 22、事务 14，均无新增游戏安装记录；插件排序/隐藏/冻结/禁用与首次基线一致，受保护设置、任务与事务文件哈希未变化。仅真实来源查询更新其来源缓存。
- 海外来源仍有 retry_deferred / budget 超时；旧海外 success 的 encoding_version=0，与当前结构规则不兼容，自动安装仍要求新解析记录。这是结构兼容及网络查询限制，未按时间失效；未伪造当前编码版本或刷新时间。
- 证据与首次备份存于忽略目录 `.artifacts/summer-memories-diagnosis-20261014/`（final-state.json、manual-ui-final.json、verify-manual-toggle.json、scroll-actual.json、qam-final.png、manual-cn-bottom.png、deployed-verification.json、manifest.json 等）。取回并校验材料后，已检查本轮远端暂存绝对路径属于指定 staging 根，清理仅本轮目录；正式插件和游戏/状态目录未清理。
- Git 改动未提交。Windows 客户端代码/构建验证完成，本轮仅向 Steam Deck 部署。Summer DLC 安装器兼容、真实补丁安装/游戏内生效及实体手柄仍为未执行项。

### Git 提交

- 用户明确要求提交 Git。代码、必要测试和生成契约已提交为 `ed0b404`（`fix(sources): persist cached candidates and allow manual patch selection`），设计、施工记录和功能索引单独提交。
- 沿用本轮已完成的自动化、独立复核和实机验证；提交前差异格式检查通过。备份、部署包及实机证据继续留在忽略的 `.artifacts/`，不纳入 Git。仅创建本地提交，未推送。
