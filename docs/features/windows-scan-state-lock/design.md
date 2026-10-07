# Windows 扫描刷新状态锁冲突修复

施工记录：[progress.md](progress.md)。状态：**已修复、重新部署并通过 Windows 实机验收**。日期：2026-10-07（Asia/Shanghai）。

## 问题与目标

在本地化名称 Windows 部署验收中，真实便携客户端启动及重新扫描均显示 `另一个程序正在使用此文件，进程无法访问。 (os error 32)`，列表为空。已有用户状态包含 journal.sqlite，现场只有一个客户端，无遗留核心进程。

[Windows refresh](../../../apps/windows/src/main.tsx) 同时通过 Promise.all 启动 scan 和 list_transactions 两个独立 CLI。scan 自身也调用 list_transactions；两者都经 [Store::open](../../../crates/core/src/install.rs) 打开和验证同一状态目录、journal 文件及数据库，会争用 Windows 文件共享/排他锁。

## 方案与边界

刷新先 await scan，再 await list_transactions，两者成功后统一投影扫描结果并更新界面。保留 busy、错误处理及事务归属口径；不改变核心状态锁，不通过忽略错误掩盖缺失事务，也不改写已有数据库或游戏文件。

本修复独立于[本地化游戏名](../localized-game-names/design.md)，后续代码和文档应各自拆分提交。

## 验收

生产 release 构建与打包通过；原有用户数据库存在时启动能加载游戏列表，连续重新扫描和重启均无 os error 32。沿用既有验证，仅补本次前端修改的 typecheck/build 与真实界面验证。
