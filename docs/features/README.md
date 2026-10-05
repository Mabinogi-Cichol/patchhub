# 功能开发文档

每个功能在本目录下建立独立文件夹，设计文档与施工记录分别存放。文档状态为“方案”时，不代表功能已经实现或通过实机验证。开发规则见仓库根目录的 [AGENTS.md](../../AGENTS.md)。

| 功能 | 状态 | 设计文档 | 施工记录 | 附加文档 |
| --- | --- | --- | --- | --- |
| Decky UX | 已实现并部署，部分真实操作与手柄验收待完成 | [设计](decky-ux/design.md) | [施工记录](decky-ux/progress.md) | [修复计划](decky-ux/fix-plan.md) |
| 下载缓存清理 | 已实现并部署，后端实机验收通过，部分安装与手柄验收待完成 | [设计](cache-cleanup/design.md) | [施工记录](cache-cleanup/progress.md) | — |
| KaguraGames 国内 / 海外内置源与 HGamer 兜底索引 | 已实现并部署，自动化、真实网络、RPC 与界面通过；严格样本与实体手柄待验证 | [设计](kagura-hgamer-fallback/design.md) | [施工记录](kagura-hgamer-fallback/progress.md) | [可行性](kagura-hgamer-fallback/feasibility.md) · [验收](kagura-hgamer-fallback/acceptance.md) |
| EXE 补丁静态提取与安装 | P1–P4 已实现并自动化验证；2026-10-05 Deck 上 Linux Rust/后端/合成事务复跑与 QAM 修复实机复核通过；VenusBlood FRONTIER 首个审核 profile 完成真实下载→安装→恢复；夏日语义与实体手柄待验收 | [设计](exe-patch-installation/design.md) | [施工记录](exe-patch-installation/progress.md) | [调研](exe-patch-installation/research.md) · [P0 审计](exe-patch-installation/p0-audit.md) · [验收](exe-patch-installation/acceptance.md) |

功能目录至少包含 `design.md` 和 `progress.md`，额外的调研、修复计划及验收材料也放在所属功能目录内。新增功能时同步更新本索引，具体完成范围与验证证据以各功能的施工记录为准。
