# 功能开发文档

2026-10-07：Decky UX 前台安装与独立结果面板已实现并部署，自动化、真实 RPC / 页面与合成任务渲染验证通过；真实下载 / 恢复与实体手柄待验证。详见 [设计](decky-ux/design.md) 与 [施工记录](decky-ux/progress.md)。

每个功能在本目录下建立独立文件夹，设计文档与施工记录分别存放。文档状态为“方案”时，不代表功能已经实现或通过实机验证。开发规则见仓库根目录的 [AGENTS.md](../../AGENTS.md)。

| 功能 | 状态 | 设计文档 | 施工记录 | 附加文档 |
| --- | --- | --- | --- | --- |
| 来源发现与订阅迁入共享 Rust 后端 | 迁移方案（待实现）；先 Windows，后 Decky，保留安装授权边界 | [设计](shared-source-discovery/design.md) | [施工记录](shared-source-discovery/progress.md) | — |
| Windows 扫描刷新状态锁冲突修复 | 已修复、重新部署并通过 Windows 实机验收 | [设计](windows-scan-state-lock/design.md) | [施工记录](windows-scan-state-lock/progress.md) | — |
| 通用 EXE 自动安装（按安装器行为判定） | 实施与验收完成；四款本地矩阵及用户确认生效，Deck 部署/RPC/真实页面、合成任务 UI、只读 worker 和用户手柄验收通过；2026-10-07 列表口径修订：无阻断通用 EXE 显示“有可用补丁”，已部署并实机验证（QAM/全屏），实体手柄待验收 | [设计](generic-exe-installation/design.md) | [施工记录](generic-exe-installation/progress.md) | [调研](generic-exe-installation/research.md) |
| 补丁一站式安装与流程精简 | 后端、Decky 与 Windows 客户端均已迁移协议 v2；后端/Decky 已部署，VBF 重装/下载取消及界面通过；Windows 一站式界面、进度/取消与真实 CLI 协议 smoke 通过；完整实机矩阵与 Windows 实机待验收；2026-10-07 通用 EXE 无阻断时显示“有可用补丁” | [设计](simplified-install-flow/design.md) | [施工记录](simplified-install-flow/progress.md) | [调研](simplified-install-flow/research.md) |
| Decky UX | 2026-10-07 前台安装与结果面板已部署，自动化、真实 RPC / 页面与合成渲染验证通过，真实下载 / 恢复与手柄待验收；既有 UX 已实现并部署；2026-10-06 来源文案修正已部署，真实 RPC、QAM 与 VBF 全屏页面验证通过；实体手柄验收待完成 | [设计](decky-ux/design.md) | [施工记录](decky-ux/progress.md) | [修复计划](decky-ux/fix-plan.md) |
| 下载缓存清理 | 已实现并部署，后端实机验收通过，部分安装与手柄验收待完成 | [设计](cache-cleanup/design.md) | [施工记录](cache-cleanup/progress.md) | — |
| KaguraGames 国内 / 海外内置源与 HGamer 兜底索引 | 已实现并部署，自动化、真实网络、RPC 与界面通过；重新查找后来源消失修复已自动化、离线复现并在真实 Deck 部署复验通过；严格样本与实体手柄待验证 | [设计](kagura-hgamer-fallback/design.md) | [施工记录](kagura-hgamer-fallback/progress.md) | [可行性](kagura-hgamer-fallback/feasibility.md) · [验收](kagura-hgamer-fallback/acceptance.md) · [修复计划](kagura-hgamer-fallback/fix-plan.md) |
| EXE 补丁静态提取与安装 | P1–P4 已实现并自动化验证；2026-10-05 Deck 上 Linux Rust/后端/合成事务复跑与 QAM 修复实机复核通过；VenusBlood FRONTIER、夏日官方 DLC 补丁与夏日 CN 本体补丁完成真实下载→安装→全量恢复；2026-10-06 审阅发现两项待修；游戏内生效、英文本体与实体手柄待验收 | [设计](exe-patch-installation/design.md) | [施工记录](exe-patch-installation/progress.md) | [调研](exe-patch-installation/research.md) · [P0 审计](exe-patch-installation/p0-audit.md) · [验收](exe-patch-installation/acceptance.md) · [流程图](exe-patch-installation/flow.md) · [代码审阅](exe-patch-installation/review.md) |
| 本地化游戏名（核心产出 + Windows 展示对接） | 已实现并自动化验证；Windows 便携部署及当前简体中文界面验收通过，Decky 实机待验收 | [设计](localized-game-names/design.md) | [施工记录](localized-game-names/progress.md) | — |

功能目录至少包含 `design.md` 和 `progress.md`，额外的调研、修复计划及验收材料也放在所属功能目录内。新增功能时同步更新本索引，具体完成范围与验证证据以各功能的施工记录为准。
