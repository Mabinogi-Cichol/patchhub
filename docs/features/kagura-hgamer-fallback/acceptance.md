# Kagura / HGamer 实施验收

- 日期：2026-10-05（Asia/Shanghai）。
- 设计：[design.md](design.md)；实际施工历史：[progress.md](progress.md)。
- 结论：本期发现、身份核实、来源引导和旧安装边界通过自动化及实际设备验证；严格未知发行商已安装样本、实体手柄操作仍待验证。EXE 安装能力不属于本次验收。

## 已执行检查

| 检查 | 环境与结果 |
| --- | --- |
| 完整后端套件 | Steam Deck `/usr/bin/python3`，145 项全部通过、无跳过；包含真实 Linux CoreBridge 测试 |
| 前端回归 | `apps/decky` 执行 `npm test`，35 项全部通过；涵盖显示联合类型、筛选、未查询入口、订阅隔离、重挂载与陈旧结果拒绝 |
| 类型与构建 | `npm run typecheck`、`npm run build` 成功 |
| 文档与差异 | 54 个相关本地链接有效；git diff --check 通过；设计、施工记录、索引及用户指南同步 |
| 插件打包 | `npm run package:plugin -w apps/decky` 成功；包含 discovery.py、v2 规则、简繁字典与许可证，排除测试和 fixtures |
| 独立审查 | 三名实现 Agent 加一名只读审查 Agent；均为 gpt-6.1-sol / high；审查问题修复并补回归，主 Agent 完成后续边界与实机集成 |
| 安装拒绝 | 实际 Decky API 对 candidate ID 的 prepare_install 返回 source_requires_guidance；RPC 测试覆盖 preview / plan / fetch / prepare / task 边界 |
| 全局 / 逐游戏开关 | 实际 RPC 停用 HGamer、夏日退订 kagura-cn 后不再投影对应缓存；测试结束恢复原设置 |
| 旧 ZIP 与事务 | patchmoe-3239270 仍为 publisher_zip、blockers 为空；现有事务可读取；未重新安装或卸载游戏补丁 |
| 缓存与失败 | 实机热缓存命中；离线测试覆盖错误、结构变化、TTL、版本失效、负缓存、partial、强制刷新和退订 / 代次竞态 |
| 游戏模式界面 | 真正 Steam CEF 渲染，1280×800 截图 / 854×534 CSS 视口；发行商源 / 索引分组、语言、DLC、版本与格式可见；页面宽度与 scrollWidth 同为 566，无横向溢出；滚动到底可见来源按钮 |

后端测试位于设备独立目录，通过 staged `source/apps/decky/backend` 发现全部 `test_*.py`；测试使用设备原有核心二进制、临时状态与合成库，不写真实游戏目录。未以 Windows 上缺少 Linux 二进制的测试结果代替这一步。

## 真实游戏与网络

### 夏日狂想曲

- 实际安装实例 AppID 1227890，核心名称 Summer Memories，Steam 本地化名称“夏日狂想曲：乡间的难忘回忆”，build 6270017；库扫描共 38 行。
- 解析版本升级后的实际查询 11.5275 秒、6 条候选；热查询 0.0048 秒。最后重载前端后仍从成功缓存取得三站结果。
- 国内本体 1.03 / 扩展 2.02.1、海外本体 3 / 扩展 2.03.0、HGamer 本体 / 扩展分开呈现。国内扩展保留正文 2.021 与直链 2.02.1 的原版本差异；提取码 v103 / v202 不作为版本。
- 国内本体排除 DLC 1390150，国内扩展要求该 DLC；设备 DLC 拥有 / 安装状态未知，页面明确要求核对。海外扩展仅标题线索，未伪造 AppID 确认。
- HGamer 声明 zip、上游官方链接为 exe，实际详情显示“页面标示 zip，上游为 exe”，只提供详情及上游网页按钮，没有自动下载、准备或确认安装按钮。

### 兜底样本

| 样本 | 实际验证与边界 |
| --- | --- |
| 催眠学习，1788100 | 设备真实已安装；临时关闭该游戏 patch.moe 后，由 HGamer 页面 Steam 链接核实 AppID，版本 1.1.0、variant=dlc、语言及 DLC 条件未知；finally 恢复订阅。覆盖官方选择变化后不能复用跳过兜底的缓存 |
| 新搬来的大姐姐真是太可爱了，4483210 | 设备未安装；Deck 固定 `/usr/bin/python3 -I --discover` 真网络工作进程在 6.611 秒得到 1 条核实 AppID 的 HGamer 候选，上游 https://azucat.net/patch/；国内归档 partial、海外 complete 零结果。覆盖未内置发行商发现，不冒充已安装样本 |

`DLC補丁` 误分类通过实机发现并修复，新增中文相邻 DLC / Demo 回归，解析版本升至 2。上述样本仅为验收输入，没有添加生产固定游戏数组。

## 部署与证据

- 插件位置：`/home/deck/homebrew/plugins/steam-patch-manager`，使用 deck 身份部署，无 sudo。
- 10 个文件逐一按 SHA-256 校验，使用 Decky 已认证 reload_plugin 正常重载，最后 `plugin_loader` 为 active。
- 初始原文件备份：`/home/deck/steam-patch-validation/kagura-hgamer-20261004/backup`，重复部署保留第一份备份。
- 最终前端 SHA-256：`6583f9fba850300425b46381b7acc21ad7d594cea469a7df25a6b4315fd54691`。
- 最终 sources.py SHA-256：`077780444c80775b5484044511d213d4ee45a65779cdd166d564278db48a12c8`；discovery.py：`bbe0b6b41a91c3483f913d2f1223a337b52ca004bafcb22c41c513c28fc9b502`。
- 原始验收 JSON、截图与受限部署脚本位于忽略目录 `.artifacts/`，设备对应目录为 `/home/deck/steam-patch-validation/kagura-hgamer-20261004/`。本机关键证据：`cef-rpc-result.json`、`cef-rpc-final.json`、`installed-fallback-result.json`、`unknown-publisher-worker.json`、`ui-sources-scroll.json`、`ui-game-scroll.json`、`ui-hgamer-scroll.json` 及同名 PNG；最终清单 `kagura-manifest.json`。这些含真实库路径的材料不提交或打包。

## 未执行与后续

- 严格“未知发行商且已安装”的单一样本在现有设备没有可用实例；保留为待验证。已安装游戏与未知发行商网络分别覆盖，不能相互替代。
- 实体手柄连续操作、真实断网后菜单操作未执行。实际 CEF 导航、按钮选择、滚动及截图已执行；菜单重挂载和失败缓存由自动化覆盖，不记作人手验证。
- 官方分页受 30 秒单游戏预算约束，可能 partial 并继续检查点；本期不保证一次遍历完整站点。
- 未下载或运行 Kagura EXE，未验证此类补丁安装、生效或恢复，未获取可靠游戏语言 / DLC 状态。格式扩展另建功能计划。

因此本记录确认可交付的首期发现与引导行为，不宣称完整设计矩阵中的每项实体操作均已完成。
