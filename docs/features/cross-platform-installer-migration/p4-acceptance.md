# P4 Windows 收尾验收

设计：[design.md](design.md)。施工记录：[progress.md](progress.md)。前序实施：[p4-windows.md](p4-windows.md)。日期：2026-10-09（Asia/Shanghai）。

状态：**P4 按调整后的验收范围完成：Windows 功能、EXE/动态 ZIP 真安装与两款游戏内确认通过；最终代码自动化、恢复/历史真实 RPC、列表与恢复提示浏览器验证及便携包校验通过。2026-10-10 用户明确不要求断网测试，取消原生 GUI 断网重开人工验收项；该项未执行，不计为通过。** 本文补充当前验收，不改写前序会话的历史证据。

## 本轮修复

- Windows 游戏列表主标签改为补丁状态：优先“已安装补丁”，其次“补丁需检查”或“有可用补丁”，完成查询无候选时“未找到补丁”。未查询、查询中、失败、不支持安装的来源线索分别显示待查询、查找中、查询未完成、待核对，不能将线索视为可安装补丁。来源查询状态保留在次级说明/详情，不占用列表主标签。
- 动态 EXE 原先固定 `detection=unknown`。现在按同实例 committed 事务的 release/selection 身份读取计划并重哈希实际载荷：匹配才报 `installed_managed`，缺失/变更报 `partial_or_conflict`，其他实例/选择不借用收据。ZIP 继续复用同一检测路径。
- 恢复页面原先忽略 RPC 返回事务并统一显示完成。现在 `needs_recovery` 明确显示冲突与剩余项；无待恢复事务/已恢复事务分别给出结果，不用成功提示覆盖冲突。

## 自动化证据

| 检查 | 结果 | 本地证据 |
| --- | --- | --- |
| 后续修复基线完整回归 | core lib 272 passed / 5 ignored；CLI 38；Windows 宿主 23；集成 catalog/install/localized/steam_scan/trust 为 8/40/4/6/3，全绿 | `.artifacts/p4-windows/final-regression-20261009.log` |
| 本轮 EXE 检测新增回归后 CLI | 39 passed，含选择身份/实例隔离、载荷变更与缺失检测 | `.artifacts/p4-windows/final-cli-regression-20261009.log` |
| Windows 主状态语义 | 10/10（已安装优先于查源、外部安装、可用、无结果、仅来源、被阻断、不完整、未查询、查询中、查询失败） | `npm --prefix apps/windows run test:status`；[检查脚本](../../../apps/windows/scripts/test-game-status.mjs) |
| 静态检查 | core/CLI/Windows clippy `--all-targets -- -D warnings`、CLI rustfmt、TypeScript 与 Vite build 通过 | `.artifacts/p4-windows/final-clippy-20261009.log`、`final-tauri-build-20261009.log` |
| 浏览器展示 | 生产前端通过主要补丁状态、页面重载后真实事务快照历史和恢复成功/冲突展示检查，无页面错误 | `.artifacts/p4-acceptance/run-20261009/browser-evidence.json` 与 `browser-*.png` |

完整回归先于本轮 CLI/前端小修复；core/宿主未新增修改，沿用本轮基线，受影响 CLI 与前端补充定向回归。SF 解析/worker 未改变，沿用前序 release worker 三真实样本证明，不重复 15 分钟转换。

浏览器展示使用真实扫描/事务/RPC 响应快照和两条合成列表项，Tauri 桥接模拟；**不计原生 GUI 实机操作**。Windows Computer Use 初始化两次超时，浏览器控制工具也初始化超时；本轮没有通过旧 PowerShell UI 自动化绕过该限制。

## 实际 RPC、恢复与文件保护

隔离根目录 `.artifacts/p4-acceptance/run-20261009/`，通过核心生成当前 schema 计划，用 release CLI `serve` 发 protocol v3 请求；每次 RPC 启动新进程。所有故障写入仅限合成 Steam 库，真实游戏只读扫描。

| 场景 | 结果 |
| --- | --- |
| committed 历史 | 两次重新启动 RPC 返回相同事务与绑定；可无签名目录/信任文件读取 |
| 中断应用 | 当前 schema journal 的 applying/installed 阶段恢复为 rolled_back，原文件 SHA-256 恢复；重复 recover 返回空；备份清理 |
| 中断后外部改动 | recover 与重复 recover 均返回 needs_recovery；用户改动保持、原始备份保持，不覆盖冲突 |
| 存档保护 | 三场景 save.dat sentinel 摘要全部保持 |
| 真实安装缓存扫描 | 2303800 EXE 与 2743810 ZIP 均按实际载荷返回 installed_managed；2303800 同时仍保留 selection_stale blocker，不因已安装放宽新安装授权 |

证据：`fixtures.json`、`*-before.json`、`*-recover*.json`、`*-after-reopen.json`、`real-cached-scan.json`、`real-history.json`、`rpc-evidence.json`。故障由验收 helper 构造，不冒充真实进程崩溃。恢复材料为冲突场景保留。

网络条件：合成历史/恢复传入不存在的目录/信任文件并设置不可达代理；恢复/历史路由本身不访问来源，读取独立于这些依赖。真实扫描读取现有缓存。**本轮未关闭整机网络，不能代替原生 GUI 物理断网重开验收。**

用户在本轮明确确认 2743810 已在游戏内生效；与前序 2303800 确认分别登记，两款均已满足游戏内确认。

## 最终便携包

- 路径：`.artifacts/steam-patch-manager-windows-portable.zip`；解压验收目录 `.artifacts/p4-acceptance/portable-final/`。
- 大小：8,513,897 字节；SHA-256 `e7204378b006ae8723dcd6e69949d7d36ad8e7878a5b19ca4c0468080d68f0f4`。
- 宿主 SHA-256 `cc2c1702e749c069a342edcb14a272c69c3f87bb7139e09fdb068e6f96bbcf63`。
- CLI SHA-256 `a2087541c507243f05c1798262637c87f230db351b5d1f563d82ac319b58d825`。
- worker SHA-256 `f4b27ad28ee0d35970362da7fdf6ac7c6204227d48a2276d9abf327d97e4e916`。
- 三件套与 target/release 一致；从最终解压包运行 get_capabilities：protocol 3、EXE/ZIP links.available=true，worker 为同目录固定程序。证据 `portable-final-hashes.json`、`portable-final-capabilities.json`。
- 先前包已备份到 `.artifacts/p4-acceptance/pre-status-portable.zip`，前序文档的 `c4a159ca…`/`69354d7d…` 属历史产物，不作为当前包摘要。

## 验收范围调整与登记项

1. 2026-10-10 用户明确“不需要测断网”。取消此前原生 GUI 断网重开及其列表/历史/恢复按钮人工核对请求，不再作为 P4 阻塞项，无需用户回复此前请求。P4 按调整后范围完成；既有 RPC 与浏览器证据仍按各自层次记录，未将模拟结果改写为原生 GUI 或物理断网通过。
2. 更早 Win11 版本未覆盖；本机版本证据不扩大为全版本矩阵。
3. TUN/fake-IP 支持决策、kagura 来源质量诊断、P5/P6 和三旧 profile 样本沿用前序登记，不因本轮验收自动结项。

本轮未提交 Git、未操作 Deck、未再次安装真实游戏补丁。
