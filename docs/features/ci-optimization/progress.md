# CI 优化施工记录

[设计](design.md)

状态：已实现，本地定向验证通过，线上 CI 待推送后核验。

## 任务状态

| 任务 | 状态 |
| --- | --- |
| Job 级 `timeout-minutes` | 已实现（6c2cff9） |
| CI 测试项必要性审查 | 已完成，结论见设计文档 |
| 移出 `sf95` 步骤、去除重复 `test_project_version.py` | 已实现（6c2cff9） |
| 测试内等待增加失败出口 | 已实现（ff77fc6） |
| 替身与真实接口对齐 | 已盘点，无需额外改造 |
| 脆性断言分类与清理 | 已取证，无需删除 |
| 回归演示 | 本地已完成 |
| 线上 CI 核验 | 待验证 |

## 2026-11-18：调研与前置修复

- 查明 push CI #124/#125 失败原因：Decky 后端 `test_failed_source_cancellation_blocks_replacement` 的假 `call` 不接受 `browser_request`，导致 `started` 永不置位，`decky` job 挂起 6 小时被取消。
- 已在 `main` 提交 `f3b4f64` 修复：假 `call` 增加 `**options`。本地 `test_app_update.py` 14 项通过；Decky 后端全量 127 项中 2 项真实 core 集成用例因本地未设置 `PATCHHUB_TEST_CORE`、使用旧二进制出现 `capability_mismatch`，与本问题无关。Linux/WSL 全量与线上 CI 待验证。
- 近 40 次 main 运行：6 次非成功（2 次挂起取消，4 次失败）。后续盘点需据此建立失败证据清单。

## 2026-11-18：实施记录

- **超时**：取 9 次成功运行的各 job 耗时，峰值 windows 686s、core(windows) 323s、decky 159s、release 135s、changes 13s。设置 changes 5、core 20、windows 30、decky 20、release 15 分钟（≥2.5 倍余量），仅作挂起兜底。`ci.yml` YAML 可解析。
- **移出 `sf95`**：确认 `scripts/freeze_sf_rust_fixtures.py` 仅在开发期生成 `crates/installer-format/tests/fixtures/sf/*.json`，Rust 测试读取冻结 JSON，生产代码无引用。删除 core 的两个 `test_sf95_*` 步骤及选择器中仅服务它们的 core 规则，更新 `test_select_jobs.py`（9 项通过）与 `.github/ci/README.md`。`test_sf9_*`、`test_decky_debug_update.py` 保留文件但不入 CI（原因见设计表）。
- **去重**：windows job 删除 `test_project_version.py`（`changes` 已跑）。
- **等待失败出口**：`test_app_update`、`test_bridge`、`test_source_browser`、`test_source_tasks` 中 5 处等待改为 `asyncio.wait_for(…, 5)`（`test_source_tasks` 新增 `until` 辅助）。`test_tasks`、`test_v5_bridge` 等已有 `wait_for` 的用例不动；其余 `await finish.wait()` 等在测试自身控制下释放，不依赖被测任务触发，不改。
- **回归演示**：临时还原旧假 `call` 签名，`test_app_update.py` 在 5.4 秒内以 `TimeoutError` 失败（此前无限挂起）；文件已恢复。
- **替身盘点**：Decky 后端对生产函数的手写替换只有 `test_app_update`（已修）与 `test_source_tasks.MatchBridge._call`（仅用于不传 `browser_request` 的 `match_games`）两处，其余为 `AsyncMock`。无新增失败证据，不为其引入 autospec 层。
- **脆性断言取证**：近 40 次 main 运行的 4 次失败（默认周期断言、FakeLoader 引用已删常量、docs 触发的两次）均已在各自修复中处理，当前没有未修复的失败证据，不依据文件大小批量改动断言。
- **验证**：Decky 后端全量 127 项，2 项真实 core 用例因本地未设 `PATCHHUB_TEST_CORE`、使用旧二进制出现 `capability_mismatch`（与改动无关）；Windows 沙箱外运行。WSL 在沙箱内不可用，Linux 全量与线上 CI 待验证。
