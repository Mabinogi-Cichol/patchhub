# CI 优化施工记录

[设计](design.md)

状态：待实现（仅完成调研与方案）。

## 任务状态

| 任务 | 状态 |
| --- | --- |
| Job 级 `timeout-minutes` | 待实现 |
| 盘点等待写法与手写替身 | 待实现 |
| 测试内等待增加失败出口 | 待实现 |
| 替身与真实接口对齐 | 待实现 |
| 脆性断言分类与清理 | 待实现 |
| 回归演示与线上 CI 核验 | 待验证 |

## 2026-11-18：调研与前置修复

- 查明 push CI #124/#125 失败原因：Decky 后端 `test_failed_source_cancellation_blocks_replacement` 的假 `call` 不接受 `browser_request`，导致 `started` 永不置位，`decky` job 挂起 6 小时被取消。
- 已在 `main` 提交 `f3b4f64` 修复：假 `call` 增加 `**options`。本地 `test_app_update.py` 14 项通过；Decky 后端全量 127 项中 2 项真实 core 集成用例因本地未设置 `PATCHHUB_TEST_CORE`、使用旧二进制出现 `capability_mismatch`，与本问题无关。Linux/WSL 全量与线上 CI 待验证。
- 近 40 次 main 运行：6 次非成功（2 次挂起取消，4 次失败）。后续盘点需据此建立失败证据清单。
