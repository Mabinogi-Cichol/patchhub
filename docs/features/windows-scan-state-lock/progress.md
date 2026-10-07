# Windows 扫描刷新状态锁冲突修复施工记录

设计：[design.md](design.md)。状态：**已修复、重新部署并通过 Windows 实机验收**。

## 2026-10-07（Asia/Shanghai）

- Windows 本地化名验收首次启动及重新扫描均出现 os error 32，0 个安装实例；原部署及用户数据库已备份，未删除状态文件。
- 确认 refresh 并发 scan/list_transactions 均访问 Store；改为顺序 await，在全部成功后一次更新状态。
- npm bundle 中 typecheck/Vite 及生产 Tauri/NSIS 构建通过；更新原便携部署，全部部署文件 SHA-256 与清单一致。
- 既有用户数据库存在时首次加载 33 实例；连续 3 次重新扫描和正常关闭/重启全部通过，无 os error 32。事务记录保留，启动迁移仅刷新 updated_at；未删除数据库、未安装游戏补丁。
- 失败现场与原部署备份位于忽略目录 `.artifacts/localized-game-names-windows-20261007/`。
