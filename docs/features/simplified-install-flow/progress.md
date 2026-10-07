# 一站式安装与流程精简施工记录

设计：[design.md](design.md)。调研：[research.md](research.md)。

状态：**后端与 Decky 已部署，自动化验证、Steam Deck VBF 重装/下载取消和真实界面验证通过；完整异常矩阵、实体手柄与游戏生效待验收。Windows 适配 TODO。**

## 任务状态

| 工作 | 状态 | 证据/待办 |
| --- | --- | --- |
| Luna 只读调研现有代码 | 已完成 | 调查 UI、后端任务、EXE 条件/receipt、Rust 事务、Windows/CLI、重装与清理；见 research |
| 读取用户提供 EXE 工作流聊天 | 已完成 | read_thread 读取“执行 EXE 补丁安装计划”，以当前源码复核；不复制旧验收结论为新 feature 验收 |
| feature 设计、计划与功能索引 | 已同步实施与验收结果 | design/progress/research，详细验证范围见下方记录 |
| P0 契约与迁移样本 | 已实现，自动化验证 | 自动适用证据、任务身份、稳定内容身份、旧未完成事务与运行状态检测边界 |
| P1 内核重装、最小收据与恢复清理 | 已实现，自动化验证 | 不删 journal 绕过 committed；成功后清临时恢复材料 |
| P2 一站式后端任务与 EXE 契约 | 已实现，自动化验证 | 同任务 prepare→apply；去人工确认；取消先撤授权 |
| P3 Decky 精简界面 | 已实现，自动化验证 | 安装/重装主动作，无确认/备份/卸载产品步骤 |
| P4 CLI/共享契约 | 已实现 | 协议 v2；删除旧确认参数和卸载方法，不保留兼容性 |
| Windows 客户端适配 | TODO，延后 | 用户明确本轮不修改，旧 Windows 调用可能无法工作；后续迁移一步安装协议并验证 |
| P5 自动化与实机验收 | 部分实机通过 | VBF 重装、下载取消、退出页面与界面通过；其余矩阵、手柄与生效待验收 |

## 2026-10-06

- 用户要求先调研和编写精简 feature 计划，明确使用 Luna 子 Agent；本轮没有获得或执行插件实施、Git 提交、设备部署任务。
- 调研基线 `d2f551f`。开始时已存在 EXE feature 及索引的未提交文档改动，并有 flow.md/review.md；保留这些内容，本轮只新增本 feature 文档和追加索引条目。
- 创建两次调研 Agent：首次在终端/node 环境启动故障后中止；定向重新创建的 GPT-6 Luna/high 只读 explorer 完成代码调查，主 Agent 复核并编写文档。未委派实现或设备操作。
- 明确删除三项人工确认、下载后二次确认、卸载入口/协议与成功后的长期备份。必要内部计划、写前检查与未提交操作临时恢复材料继续作为自动处理机制，不成为用户步骤。
- 将同内容重装、保留最小收据、移除 UI 续租依赖、升级迁移及 EXE 取消授权缺陷列为实施依赖，避免仅删除页面控件。
- 本轮未修改或运行插件功能代码。自动化测试、构建、实际下载/安装、Steam Deck/Windows 实机和手柄验收均未执行。

## 文档检查

### 同日需求补充：扫描阶段分类

- 用户明确扫描阶段自动查找可自动安装补丁，不等进入详情或准备步骤；design 增加扫描会话编排、只读能力探测与增量更新方案。
- 主状态补充“找到补丁来源”：仅 HGamer fallback 匹配、未审核或不能自动获取并安装的来源采用此状态；可自动安装显示“有可用补丁”。同一游戏汇总优先已安装，其次自动安装，其次仅来源。
- 同步更新状态表、目标流程图、P0/P2/P3 任务及扫描分类/一致性验收；仍为待实现，本次只修订文档。

- 新增三份文档与功能索引共 4 份文件的 61 条本地相对链接全部存在。
- 文档冲突标记、行尾空白检查与 `git diff --check` 通过；Git 仅提示既有 LF/CRLF 转换规则，不是检查失败。
- 工作区核对确认本轮新增 simplified-install-flow/，索引仅追加本 feature 行；原 EXE 文档的未提交改动继续保留。没有插件代码变更。
- 这些是文档结构检查，不是自动化功能测试、构建或实机验收。

## 遗留与实施前核对

- 自动判定运行中游戏的可用接口与证据可靠性待 P0 核对；现有锁不提供这一保证。
- profile 本地安装条件与历史官网未购买条件存在语义差异，不能自动声明已核对；P0 明确每档审核证据，不恢复人工确认流程。
- profile 内容摘要变化会改变 release_id，旧 ID 映射必须绑定稳定载荷身份。
- 旧 committed 备份、失效 planned、未完成 installing/uninstalling、损坏收据迁移需可重复样本验证；不能一次性盲删所有 EXE jobs。
- 此 feature 完成后更新关联 feature 当前设计；已有施工、流程图和审阅文件保留历史事实。本轮不把旧卸载缺陷或取消授权缺陷标为已解决。

## 2026-10-06 实施

- 用户授权开始实施，随后限定后端与 Decky；明确无需兼容旧 API，Windows 暂不工作也可，列为后续 TODO。
- Rust 安装新增显式 install/reinstall、稳定内容摘要和旧收据校验，重装成功替代原收据，失败保留原收据。保留最小日志，终态清理临时 stage/backup，恢复未成功的材料仍保留。迁移具有持久标记，旧可执行计划撤销。
- 后端采用单个带 request_id 的任务完成查找、下载、校验、准备和安装；任务历史持久化，重复 RPC 返回原任务。移除前端续租取消，关页面不会终止安装；显式取消先写 durable revoked，再调用丢弃计划 RPC。
- EXE receipt 改为 schema 2，自动验证当前游戏事实，移除人工条款/适用条件/备份确认，不改变已审核 profile ID、载荷校验或静态提取边界。
- 扫描建立后台会话并以最多 3 个游戏并发查找；增量返回扫描进度和可安装分类。失败保留上次成功来源，旧 generation 不覆盖新会话。只有审核支持且当前条件满足的补丁可标 auto_installable；HGamer fallback 仅来源。
- Decky 仅保留安装、重新安装、查看来源及进度中的取消；移除卸载和长期备份设置。共享 UI 使用后端明确能力字段，损坏的自有收据允许同内容修复安装。
- 协议提升到 v2，后端/CLI 移除旧确认位和卸载入口。未适配 Windows，未运行 Windows 构建，旧版 Windows 可能无法工作。
- 自动化：Decky typecheck、42 项测试和 build 通过；Python 全套基线 189 项通过（3 项跳过），新增扫描/任务历史行为测试 5 项通过。最终全量结果和独立审阅见后续记录。
- 未执行 Steam Deck 部署、真实补丁下载/安装、手柄界面验证及游戏生效验收。自动化成功不代表这些项目通过。没有 Git 提交。

## 后续工作

- TODO：Windows 客户端迁移协议 v2、一步安装与安装/重装状态；删除其旧确认、卸载和备份 UI，再完成 Windows 回归。
- TODO：补齐 Deck 实机 ZIP、其他审核 EXE profile、写入阶段取消、空间不足与迁移矩阵；VBF 重装、离开页面和下载取消已通过，见下方记录。

## 2026-10-06 最终复核与回归

- 独立审阅发现并修复：扫描遗漏非 discovery ZIP 源刷新；prepare 建立计划后任务历史写入前的崩溃授权窗口；拒绝不同内容的重装前已覆盖旧目录；查询失败误显示未找到；空库来源刷新未标记扫描进行中；ZIP 名称乱码。
- 启动时依据 journal 撤销全部遗留 planned，不依赖 tasks.json 是否已写 plan_id；丢弃 RPC 失败时持久撤销标记仍阻止 apply。
- ZIP 重装在发布新目录前比对原内容身份，更新载荷会明确拒绝，旧 signed catalog 与安装检测保持可用。不同内容更新不属于同内容重新安装。
- Core/CLI：65 项测试通过，CLI build 与 Rust 格式检查通过；公开 Rust uninstall、旧确认参数与 CLI 手动 apply 已删除。
- Decky：typecheck、42 项测试、build 通过（包括新增失败/部分查询分类）。Python：最终全套 203 项测试，200 项通过、3 项条件测试跳过（包含 ZIP 查询不完整用例）。
- 同步 README、总体安装设计与关联功能的现行规则；保留 EXE 原有未提交文档和历史实机记录。本 feature 未部署、未实机验收、未提交 Git。
- 后续 profile 元数据修订若改变 release ID，需要新增可靠内容别名迁移；本轮未修改 profile，保持旧 ID。

## 2026-10-06 Steam Deck 部署与验收

- 使用仓库 decky-deploy-verify 技能。用户明确授权 VenusBlood FRONTIER 重装及取消；其他游戏只查询，未发起安装。没有 Git 提交。
- Linux Core/CLI 68 项测试及 release build 通过；最终 Python 本地 206 项（203 通过、3 条件跳过）、Deck 隔离 Python 206 项全部通过；Decky typecheck、43 项测试、build/package 通过。
- 实机修复并补回归测试：公共 RPC 等待启动迁移完成，installer_busy 有限重试；核验官方来源刷新优先已知页面和已验证别名，解决 International 标题无法刷新候选；新扫描启动即撤销旧轮询，避免 scan_stale 留下 Python Exception。
- Loader 本地 ZIP 更新 33 文件，逐文件 SHA-256、Linux ELF 执行权限通过；Loader 3.2.10、plugin_loader active、本插件实例 1。最终包 SHA-256：33ba6879b455ef6689a41b70217a4c142f334e572591e667b1efd31c68fc49e8；实际浏览器 bundle/HTTP/本地 SHA-256：865ed9d3ab4c49d13cd1dcdd87aee80d0be6aa9c98035b7b68097b639f063ff0。
- 热重载残留旧路由会话；确认任务全终态后重启 Steam JS 上下文，再从 QAM 挂载新会话。恢复本插件原 Loader 顺序，未改其他插件设置。
- 真实扫描 37/37 已安装实例完成；自动能力包含 AppID 1189440、1788100、3239270，已安装时主状态优先“已安装补丁”；HGamer 匹配 1227890、2303800 显示“找到补丁来源”。扫描未下载完整包。
- VBF profile venusblood-frontier-cn-v5 一步重装成功：任务 3885f959-ef85-4d55-8702-ccc20b08f8f5；commit a07fc21a-aeb4-4910-a7e1-74c21127e2d1。1,126,694,464 字节 EXE 自动下载/校验/提取/安装，约 177 秒；一次 start RPC，没有二次 apply 或确认。立即离开页面后持续运行，超过旧续租时限。同 request_id 返回原任务，不重复执行。
- 真实 UI 点击“重新安装补丁”，任务 68d89995-94a3-49a9-ad66-b82db671b23b 下载 3,440,409 字节后点击“取消安装”，终态 cancelled/download_cancelled。原 committed 收据保留，没有新提交。
- 重装与取消后 readme、extra.xp3、extra2.xp3、VBFI.exe 与基线哈希相同；临时 stage/backup 文件 0，下载缓存空。旧成功收据 superseded、新收据 committed。本轮未卸载游戏或补丁。
- 已检查真实 QAM 与全屏截图；主按钮重新安装，没有条款勾选、确认安装或卸载入口；取消后刷新无 Python Exception。CEF 点击不代表实体手柄/触屏验证。
- 证据目录 .artifacts/simplified-deck-20261006/：final-approved.png、final-qam.png、cancel-result.json、idempotence.json、remote-evidence/game-after.json。48 个远端证据/备份文件取回并逐项校验，归档 SHA-256：736b584720f7f7dc00c200dc80a52589436bb7bb8e0a89f990601dd53e90b9a0。
- 首次旧插件备份 37 文件和 rollback.zip 保留，回滚包 SHA-256：2bfd23bedc27f61317bb6948fd7ae6929ca8f5527d8f568e7a801e19738df77b。仅回滚插件文件，不回退本轮安装后的事务状态。
- 待验收：实体手柄/触屏、游戏内补丁生效；ZIP/其他 EXE profile 实机安装重装；写入阶段取消、满盘和后端中断等完整矩阵。Windows 继续 TODO。本轮通过不等于 P5 全部完成。
- 归档校验后仅清理本轮远端验证目录；插件、state、游戏和旧工具链保留。结束时没有活动任务，插件实例 1，已恢复原顺序并返回 Steam 游戏库。

## 2026-10-06 本地 Git 提交

- 用户明确要求提交 Git，沿用已有自动化与设备验证，没有重复运行功能测试。功能代码提交 adb34df；功能文档独立提交。保留本轮开始前已有的 EXE 审阅与流程图改动，没有推送。


## 2026-10-07 前台安装约束修订

用户撤销关闭界面后后台继续安装的交互要求，改为前台安装、原生关闭即取消、独立终态结果面板。已同步本功能设计中的任务存活、界面操作与验收要求；历史施工事实保留。该项代码修改与自动化验证记录集中在 [Decky UX 施工记录](../decky-ux/progress.md)，新交互尚未部署或实机验证。

## 2026-10-07 状态文案澄清

- 用户反馈：通用 EXE 游戏（妃神会秘史，AppID 2303800）游戏列表显示“找到补丁来源”，但进入详情可以自动安装，与 README 旧文案“找到补丁来源表示工具暂不支持自动安装”矛盾。
- 原因核对：该游戏没有逐游戏审核配方，走通用 EXE 动态路径；`packages/ui/src/index.ts` 的 `installSummary` 要求 `compatibility !== "check_required"` 才显示“有可用补丁”，而通用 EXE 为 `check_required`，因此汇总为“找到补丁来源”；详情页 `canAutoInstall` 仍为真，安装入口可用。行为符合通用 EXE 设计，问题在于旧文案把“找到补丁来源”等同于“不能自动安装”。
- 按方案 A 只修订文案，不改判定行为：README“使用方法”第 4 条改为说明“找到补丁来源”不排除自动安装，并区分官方通用 EXE（详情安装时自动检查条件）与需按官方说明手动处理的来源；README 首段同步放宽自动安装适用范围描述，纳入通用 EXE 包。
- 同步修订本功能 `design.md` 展示状态章节，补充通用 EXE `check_required` 归入“找到补丁来源”但仍提供安装入口，并链接通用 EXE 设计。
- 本轮仅改用户文案与文档，未修改插件代码、测试或后端；未运行测试、构建、部署或实机验证。

## 2026-10-07 状态口径修订：通用 EXE 归入“有可用补丁”

- 用户确认按方案 A 实施：详情可自动安装的通用 EXE 应在列表显示“有可用补丁”，不再归“找到补丁来源”；上一条“状态文案澄清”作为该问题的首次缓解保留为历史，README 相关措辞随本次口径修订调整。
- 代码改动：`packages/ui/src/index.ts` 的 `installSummary` 去掉 `compatibility !== "check_required"` 门槛，改为凡 `canAutoInstall`（无 blockers、具备下载能力、非 HGamer）即判“有可用补丁”；`compatibility` 仍用于详情显示“安装时自动检查条件”。HGamer fallback、存在 blockers 或安装链路不可用（reviews 为 guidance，不进入 patches）仍归“找到补丁来源”。
- 测试：`apps/decky/tests/install.test.ts` 将 `check_required` 用例期望改为“有可用补丁”，并新增 HGamer 与 blocker 仍为“找到补丁来源”的断言。
- 文档：README 第 4 条改回“工具无法自动安装”语义（通用 EXE 不再属于来源指引），第 32 条补充通用 EXE 安装时检查；本功能 design 展示状态说明同步；通用 EXE 功能的 design/progress 记录本次修订。
- 未改后端判定、扫描或安装流水线；未运行构建、部署或实机验证。
