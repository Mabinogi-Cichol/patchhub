# P4 Windows：动态安装接入

设计：[design.md](design.md)（§2.2、§8 P4）。施工与证据：[progress.md](progress.md)。宿主契约：[p1-host-contract.md](p1-host-contract.md)。独立复核与处置：[p4-gate-review.md](p4-gate-review.md)。状态：**宿主/前端与 core 收口已实现并通过自动化与独立复核；2303800 实机验收完成（真实下载 → 隔离分析 → 转换 → 真安装 → 同内容重装 → 取消 → 游戏内确认）；版本矩阵由本机 Win11 26H2 正式版证据覆盖；残余与待决策项见下**。

当前收尾状态与最新便携包摘要见 [p4-acceptance.md](p4-acceptance.md)：两款游戏内确认、动态 ZIP、恢复/历史 RPC 已通过；2026-10-10 用户取消断网实测要求，P4 按调整后验收范围完成。以下前序验收段保留当时的历史状态。

## 实施范围

1. Windows Tauri 宿主升级 `protocol_version=3`：`selection_id`/`release_id` 互斥路由、`archive` 仅签名路由、`install_progress` 平铺事件解析与 `(request_id, sequence)` 去重、`renew`/`cancel`/EOF 控制行（15s 租约转发）、错误 `detail` 组合（≤2KiB）。
2. 宿主命令收敛：删除 `MigrateState`/`GetSourcePolicy`/`ResolveSourceRecord` 等旧命令与启动迁移；能力读取命令更名 `get_capabilities`。
3. 前端与共享 DTO：`Patch.selection_id`、`Transaction.selection_id/content_id/has_replacements`；动态补丁可安装判据以 `installer.links.*.available` 为准（N1，不得用 `formats`）；动态路由使用 `selection_id`（不得以投影 `release_id` 作安装句柄）；删除 `ensureSourcesSupported` 旧核心降级；9 阶段进度与真实错误展示。
4. core 边界收口：R6 `demo`/`unknown` 变体在解析期干净阻断（`exe_candidate_variant_unsupported`，投影仍生成选择并以 blocker 呈现）；R3 报告 `script_inventory`/`certificate`/`diagnostics` 嵌套键严格校验；H2 `Transaction` 归属 `selection_id`/`content_id` 持久化（动态 ZIP/EXE），修复动态 EXE 重装判据。
5. 契约与工具：`packages/contracts/README.md` 升 v3（16 方法、平铺事件、`InstallOutcome`、`error.detail`）；新增 Windows 验证 runner `scripts/run-windows-check.ps1`（release worker + 真实样本 ignored 测试）。
6. 多镜像确定性择一（实机验收发现，2026-10-09 裁决）：同候选同名多域镜像按 mirrors 原顺序择一（下载后按实际字节建立内容身份），`*_download_ambiguous` 收窄为合法镜像 basename 异构守卫；实现 `resolve.rs::select_download_url`（ZIP/EXE 共用），契约修订见 [p1-zip-orchestration.md](p1-zip-orchestration.md) §2.1(6)。

## 验收

### 自动化与独立复核（2026-10-09）

- 回归（Windows 本机最终树）：core lib **255 passed / 4 ignored**、cli **37 passed**、集成 catalog 8 / install 40 / localized 4 / steam_scan 6 / trust 3、windows 宿主 **20 passed**；core/cli/windows clippy `-D warnings`、改动文件 rustfmt、前端 `tsc` + `vite build` 通过。
- Windows runner（release worker + 真实样本）：ignored 生产测试 3/3 通过（含 2303800/3606360/3721970 分析/转换，905.76s）；产物摘要与 P2 冻结基准逐项一致；证据 `.artifacts/p4-windows/windows-check.log`、`windows-real-sf.json`。
- 独立复核：初审**有条件放行**（H1 来源任务续租缺失、H2 事务归属字段缺失、M3 待证据）→ 修复 + 回归 → 定向复审**通过**（无剩余必修项）。处置与证据见 [p4-gate-review.md](p4-gate-review.md)。
- R2 裁定：`resolve_exe` 仅接受 kagura-cn/kagura-en 白名单为产品决策；hgamer 等来源 EXE 候选以 blocker 呈现（显示但不可安装）。

### 实机验收（2026-10-09 完成）

- **2303800 真安装/生效（通过）**：GUI 动态列表/详情 → kagura-cn 中文候选 → 真实下载（1,228,278,216 字节，下载包 SHA-256 `7336c5c9…` 与官方样本一致）→ 隔离分析（LPAC worker）→ 转换 → 写入 `data.xp3`（1,373,855,065）→ 用户游戏内确认生效。
- **同内容重装（通过）**：事务 `superseded` + 新 `committed`，`selection_id`/`content_id` 正确落账（`exe-selection-144a324e…` / `exe-content-6dcc185a…`）；重装按钮与操作路径经 GUI 验证。
- **取消（通过）**：计划前取消返回 `cancelled`、无新事务、游戏文件未改动（`data.xp3` 保持重装写入版本）。
- **进度/历史（通过）**：下载/分析阶段进度与字节展示、事务历史列表、成功清理（`tasks/` 回收为 0、报告落 `reports/<content_id>/`、无 stage/backup 残留）。
- **验收发现与处置**：① kagura-cn 候选多镜像被旧「歧义阻断」误伤 → oracle 裁决「确定性择一」（[p4-gate-review.md](p4-gate-review.md) §8，契约修订见 [p1-zip-orchestration.md](p1-zip-orchestration.md) §2.1(6)）；② 本机 TUN/fake-IP 代理环境与公共地址校验冲突（`download_private_address`）→ 临时关闭 TUN 后通过，代理环境支持登记待决策；③ 计划前取消文案缺口（`reason("cancelled")`）→ 已修复。
- **残余（登记）**：动态 ZIP 实机路径（无匹配目标时待样本）；GUI 断网重开的历史/离线项；更早 Win11 版本矩阵（23H2/24H2 等）。R2 附加条件已随本轮 kagura-cn 中文安装满足。
- **便携包（通过）**：最终 zip（SHA-256 `c4a159ca…`，含 `sf-worker.exe`）三件套哈希与 `target/release` 一致；便携 GUI 启动正常、`get_capabilities` smoke `installer.links.exe.available=true`（运行时按 CLI 同目录定位 worker）。

## 后续修复与动态 ZIP 实机复验（2026-10-09）

用户报告「你妈真漂亮」（AppID 2743810）无法自动安装（patch.moe ZIP）与 GUI 每次查询弹出 cmd 窗口。诊断（现场抓取归档 `.artifacts/p4-windows/patchmoe-*.html`）与独立裁决（[p4-gate-review.md](p4-gate-review.md) §9）后修复：

- patchmoe 全表解析（索引 3 → 160 条；`tbl new` + `tbl regular`）；
- 索引来源候选接线（B2：`SourceMatch → SourceCandidate` 并入 `dynamic_zip_candidates`）；
- 动态 ZIP 语言门按来源收窄（`require_language`：发布商来源保留、索引来源不设门）；
- 宿主 `refresh_sources` 改 serve-task 通道；宿主 spawn `CREATE_NO_WINDOW`（cmd 弹窗根除）。

实机复验（GUI 便携版）：2743810 显示「有可用补丁」→ 详情 `ZIP · 默认解压至游戏根目录`、`zip-selection-560d6cae…` → 真实下载 83,964,941 字节 → 写入游戏根 `patch.xp3`（840,228,417；原 `data.xp3`/`thekuppa.exe` 未动）→ 事务 `committed | release_id=patchmoe-2743810 | selection_id=zip-selection-560d6cae… | content_id=None`；重装 `superseded`+`committed`；serve-task `MainWindowHandle=0`（无 cmd 窗口）。契约修订见 [p0-contracts.md](p0-contracts.md) §2.5、[p1-zip-orchestration.md](p1-zip-orchestration.md) §2.1(4)/(8)。

## 登记（不阻塞，随 P5/P6 处理）

- S1 `patchInstallable` 对 `managed_zip` 未校验 `links.zip.available`（Windows 恒 true）→ P5。
- S2 UI `hgamer` 硬编码与核心白名单重复 → P6 核对。
- S3 runner 证据变量名沿用 `P3_EVIDENCE_PATH` → P6 更名评估。
- S4 `loadCapabilities` 失败后进程内不重试（fail-closed）→ 可接受。
- S5 Windows 更早版本矩阵（23H2/24H2 等）未覆盖 → 登记残余（本机 26H2 正式版已覆盖）。
- **S6（待决策）TUN/fake-IP 代理环境**：Mihomo/Clash TUN 模式下域名解析为保留段 fake-IP（198.18.0.0/15），公共地址校验按设计拒绝动态下载（`download_private_address`）；浏览器类应用不校验 IP 段故无感。需评估受控支持（如显式信任系统代理/受控 DoH 解析路径）或增强诊断提示；不阻塞 P4（直连公共地址为设计内环境前提）。
- S7 计划前取消文案 `cancelled` 已补（2026-10-09）。
- 复审建议：runner 日志与最终绑定测试的时序已在 [p4-gate-review.md](p4-gate-review.md) §6 注明；runner `-Clippy` 由复审独立覆盖；宿主展示串注释点明（可选）。
