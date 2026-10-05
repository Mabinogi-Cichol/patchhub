# KaguraGames 双站内置源与 HGamer 兜底索引施工记录

- 设计依据：[design.md](design.md)
- 状态：发现与来源引导已实现并部署；自动化、真实网络、Decky RPC 与游戏模式界面验收通过。严格未知发行商已安装样本及实体手柄操作仍待验证，详见[验收记录](acceptance.md)。
- 更新日期：2026-10-05

## 任务状态

| 工作项 | 状态 | 证据 / 下一步 |
| --- | --- | --- |
| 来源调研、匹配模型与实施方案 | 调研与设计完成 | 站点观察、边界和验证矩阵见设计文档 |
| 功能文档归类 | 已完成 | 建立独立功能目录，设计与施工记录分别存放 |
| 方案可行性复核 | 已完成，修订后可实施 | [可行性记录](feasibility.md)；只读 HTTP / HTML 实验及当前接口核对，未实施或部署 |
| P0 数据与兼容层 | 已实现、自动化及实机 RPC 通过 | v2 迁移、分片缓存、TTL、工作进程校验、代次隔离与订阅边界 |
| P1 Kagura 国内适配器 | 已实现、真实网络通过 | 夏日本体 1.03 / 扩展 2.02.1、镜像、DLC 条件；归档不完整如实为 partial |
| P2 Kagura 海外适配器 | 已实现、真实网络通过 | 本体 v3 / 扩展 2.03.0、真实 Mirror 与 EXE；未确认身份保留线索 |
| P3 HGamer 兜底索引 | 已实现、真实网络及已安装游戏兜底通过 | AppID 核实、简繁检索、推荐卡排除、格式冲突与预算隔离 |
| P4 Decky 整合 | 已实现并部署，自动化与实机界面通过 | 35 项测试、类型检查、构建；界面角色、详情、来源按钮与滚动检查 |
| P5 自动化、真实网络与设备验收 | 核心范围通过，保留样本和手柄边界 | 145 项 Linux 后端测试；真实 RPC、系统 worker 与 CEF 界面；[证据及未执行项](acceptance.md) |

## 日志

### 2026-10-04

- 既有方案记录了来源调研、发现模型、安装边界、实施拆分和验收标准；具体观察及证据保留在设计文档中。
- 将原单文件方案归入 `docs/features/kagura-hgamer-fallback/design.md`，修正相对引用，并新增本施工记录。此次仅整理文档，未实施该功能或执行新的网络、安装及实机验收。
- 随后进行独立可行性复核：通过现有抓取函数读取10个页面，均为 HTTP 200；另保存繁体搜索 HTML。验证海外227个产品链接、3个镜像、夏日搜索3条真实结果，以及空搜索推荐卡不会造成匹配。
- 未内置发行商样本的简体完整标题查询断言失败；繁体变体成功提取游戏及补丁。将简繁体查询、查询词级负缓存和游戏级无结果区分写入设计。
- 修正2 MiB小于真实海外页面（2,385,014字节）、缺少单游戏发现RPC、旧订阅迁移、TTL未实现、候选进入安装接口等问题；补充范围和验收项。后台自动批量更新列为后续优化，首期采用选中游戏查询并合并结果。
- 新增可行性记录；验证离线样本断言、文档本地链接及diff空白。未修改功能代码，未运行功能测试、前端构建、补丁安装或Steam Deck验收。

- 按用户要求开始 P0–P5 实施，保留前述未提交设计修订。三名实现 Agent 分别负责站点解析、来源注册表、Decky 前端；主 Agent 负责 `main.py` 整合与验收；第四名只读 Agent 负责独立审查。
- 前端增加发现 DTO / installable-discovery 展示联合类型、单行合并、代次与订阅版本隔离、未查询入口、语言偏好、DLC / 格式与来源说明；33 项测试、类型检查和 Rollup 构建通过。
- 后端增加可信本机实例的 `discover_game` RPC、扫描缓存投影，以及候选在 preview / plan / fetch / prepare / task 入口的显式拒绝。新增 4 项 RPC 边界测试通过；已有 67 项安装 / 缓存等测试通过（1 项 Windows 权限条件跳过），旧扫描测试替身同步补全发现接口。
- 开发机真实适配器已取得三站公开 HTML；注册表端到端夏日样本查询 10.547 秒返回 6 条候选，热缓存 0.000 秒。未知发行商 4483210 初次整合查询因官方归档耗尽时间而未取得兜底结果，明确记为未通过并继续修复，不能当作无补丁。
- 独立审查复现旧查询合并、旧候选抑制 fallback 和 force 未刷新索引三个边界问题；正修复并补回归测试。设备初始只读检查确认 Decky 服务 active，夏日实际本机名称 Summer Memories、AppID 1227890、build 6270017；Steam 本地化显示名为“夏日狂想曲：乡间的难忘回忆”。尚未部署本功能，未执行 EXE 或修改游戏文件。

### 2026-10-05

- 完成四名 Agent 的三路实现与独立审查，主 Agent 集成并负责最终验收。四名 Agent 均使用 `gpt-6.1-sol / high`；审查只报告问题，修复由实现者完成。
- 修复旧代次请求复用、旧官方空结果抑制兜底、强制查询未刷新索引、HTML 真结果结构失效被当作空结果、解析版本缓存、名称 / 官方订阅上下文变更后负缓存误复用等边界；增加回归测试。父进程同时核对候选匹配依据与 AppID，旧来源候选在全局或逐游戏停用后不再投影。
- 实机发现 `DLC補丁` 的 Unicode 单词边界造成误分类，改为 ASCII 标识边界并加入中文相邻 DLC / Demo 回归；解析版本升级至 2。重新查询实际安装的催眠学习 AppID 1788100，HGamer 候选已正确为 `dlc`，未知 DLC 条件不推断为可安装。
- SteamOS 独立验收目录运行完整 Python 套件：145 项全部通过，包含真实 Linux CoreBridge 测试。前端最终 35 项全部通过，TypeScript 与 Rollup 成功；打包检查包含发现模块及简繁字典，排除 fixtures / test 文件。
- 部署 10 个文件并逐一核对 SHA-256，保留首次原文件备份，使用 Decky 已认证重载入口。`plugin_loader` 为 active。未替换核心二进制、未执行 Kagura EXE 或写入游戏文件。
- 真实设备 38 行游戏库；夏日 AppID 1227890、build 6270017 从缓存扫描进入已匹配，三站共 6 条候选。新解析版本查询 11.5275 秒，热查询 0.0048 秒。国内与海外、本体与 DLC、原版本和 EXE 分开保留；伪造候选安装被 `source_requires_guidance` 拒绝。
- 全局停用 HGamer、夏日逐游戏退订国内源后扫描不再展示对应缓存；测试后恢复设置。旧 `patchmoe-3239270` 仍为 `publisher_zip`、无 blocker，既有事务仍可读取；本轮未重新安装旧补丁。
- 未内置发行商 AppID 4483210 在 Deck 固定 `/usr/bin/python3 -I --discover` 真实工作进程中核实身份并得到 HGamer / 上游发布页，耗时 6.611 秒；设备未安装该样本。另以设备已安装催眠学习临时关闭 patch.moe 后验证兜底，并在 finally 中恢复，不把两项证据合称严格“未知发行商已安装样本”。
- Steam 游戏模式真实渲染检查来源分组、夏日详情、HGamer ZIP / EXE 冲突、来源跳转按钮及滚动；页面无横向溢出，discover_only 详情未显示准备或确认安装。修正 `source_partial` 中文文案及重复格式警告后重建部署并复验。截图与 RPC 原始证据放在忽略的本机和设备验收目录，概要见验收记录。
- 最后核对最终部署前端哈希与本机构建一致，打包内容校验通过，54 个相关本地文档链接及 `git diff --check` 通过。更新源适配器、客户端指南、隐私、Decky README 和功能索引；保留原有未提交修改，本轮未创建 Git 提交。

### 2026-10-05：修复重新查找后 Kagura 官方来源消失

- 用户报告：在《Venus Blood Frontier》补丁管理页点击“重新查找 / 查看更多来源”后，依赖 HGamer 回填身份的 Kagura 官方 EXE 来源消失。该游戏官方候选本身无 AppID，身份完全依赖兜底索引。
- 根因与修复见 [fix-plan.md](fix-plan.md)：`cached_candidates` 不再让 `partial` 的 `attempt` 候选遮蔽同 `candidate_id` 的 `success` 已验证候选；`_discovery_sync` 在本轮兜底失败或仅部分刷新时按同一 `candidate_id` 沿用上次已验证身份；官方源本轮为空且兜底索引仍指向同一官方页时保留上次已验证候选，不把空结果当权威删除。
- 回归测试：`test_partial_refresh_does_not_shadow_verified_candidate_and_keeps_new_leads`、`test_refresh_without_fallback_identity_keeps_previously_verified_candidate`、`test_empty_official_refresh_keeps_verified_candidate_when_index_confirms_page`。
- 开发机验证：Windows `apps/decky/backend` 全量 204 项通过（3 项按设计跳过）；离线构造 VBF 记录确认修复前来源不匹配、修复后命中 `venusblood-frontier-cn-v5`。
- Steam Deck 实机复验（`deck@192.168.31.186`，SteamOS 6.18.50，系统 Python 3.13.5）：部署 `sources.py`（安装 SHA-256 `42f0b899b23d9a47470b8feca25be64c2275b1c2210c96a1f0aefe553a3c32b9`，旧文件备份 SHA-256 `077780444c80775b5484044511d213d4ee45a65779cdd166d564278db48a12c8`），经 `loader/reload_plugin` 重载，后端重启。实机发现并确认第三个接缝：`kagura-cn success complete []` 而 hgamer 仍持有身份与官方上游页。
  - 确定性 A/B：旧代码投影 `kagura-cn title/null`、`reviews` guidance（来源消失）；修复后投影 `official_title/1189440`、`reviews installable`（`venusblood-frontier-cn-v5`）。
  - 空官方结果与同候选兜底失败两种 fake-module 场景在设备上均按预期保留；真实网络 `discover_game(force=True)` 连续两次均 `installable`。
  - 实时状态由部署代码恢复后读取为 `installable`；Steam CEF 真实界面显示 `VenusBlood FRONTIER International — 已找到补丁 · 1 个来源候选`（截图 SHA-256 `90040f3076ee51342222f787340ac04958c5322ee52b73451ab7d4f19429faf9`，实时分片 SHA-256 `4c10261fc09fef5c1a604f42a1dc38d13f07f49a56264736c7e54ad7181a6cd8`）。
  - 边界：直接 Python 连 Decky loader WebSocket 被 Steam 前端单连接重载关闭，实时状态恢复改用部署后代码对状态目录执行一次强制重查再重载；未做实体手柄操作，未运行官方 EXE、未修改游戏文件。取证在 `/home/deck/steam-patch-validation/kagura-requery-20261005-fix1/` 与本地 `.artifacts/exe-deck-ci/kagura-requery-20261005-fix1/`。

## 待办与遗留

- 严格设计 13.2 第 3 项的“未知发行商且已安装游戏”在现有设备缺少样本，待验证；已分别验证真实未知发行商网络与实际已安装游戏兜底，不擅自安装游戏补足证据。
- 实体手柄连续操作、真实断网后菜单操作未执行；重挂载、失败保留缓存、超时、竞态已由自动化覆盖，不将其记为实体操作。
- 官方归档超过单游戏预算时会保留分页检查点并显示部分完成；需要更多覆盖时主动重新查找，首期不承诺一次抓完整站。
- 游戏语言和 DLC 拥有 / 安装状态仍未知；EXE、网盘和聚合候选仍仅作来源引导。安装、运行、补丁生效与恢复不属于本次完成范围。
- [重新查找后 Kagura 来源消失修复](fix-plan.md) 已部署到设备并通过实机复验；仍未做实体手柄操作，且实时插件状态恢复未走真实前端点击（loader WebSocket 单连接限制）。
