# P3 运行时独立复核与处置

日期：2026-10-09（Asia/Shanghai）。范围：[P3 生产运行时](p3-runtime.md)（[design.md](design.md) §4.3/§5/§6/§8）。施工记录：[progress.md](progress.md)。

独立复核：只读 oracle 会话，对照 P3 范围、p0-contracts、p1-host-contract/p1-zip-orchestration 逐项核对全部新增与修改代码。本文件归档复核结论、必修/建议项处置与验证证据；复核报告原文要点见下。

## 1. 复核结论

**有条件放行**（处置完成后签 P3）：隔离启动器、报告校验、授权重核、撤销/租约顺序与 fail-closed 主链基本正确，未发现可越权的 RPC/宿主注入面；须处置 2 项必修（M1、M2）与 1 项契约偏离（R1），其余建议登记。

## 2. 必修项与处置

| 项 | 级别 | 问题 | 处置 | 落实位置 |
| --- | --- | --- | --- | --- |
| M1 | 高 | `TaskControl` 在受保护 `jobs/<uuid>/` 建任务目录且终态后无任何回收路径；`report.json`（≤16 MiB）/`space.json` 写入该目录，长期运行无界增长，且混淆了 p0-contracts §3.2 的事务恢复材料边界 | ① 任务运行态迁移到可清 `tasks/<uuid>/`（`owner.lock` 排它锁 + 终态删除；`cleanup_stale_tasks` 回收崩溃残留：有锁且可锁→删、无锁且 mtime>1h→删；`cleanup_cache` 接入）；② 分析报告与空间证据改落 `reports/<content_id>/`（cleanable/归档）；③ `jobs/` 仅保留事务恢复材料 | `installer/store.rs`（`tasks_dir`/`task`）、`installer/task.rs`（锁/回收/sweep）、`installer/exe_orchestrator.rs`（reports 路径）、`cli/runtime.rs`（cleanup_cache） |
| M2 | 中 | `scan`/`preview`/`get_capabilities` 等只读路径会创建 `<state>/installer/rust-v1/runtime/` 并真实启动沙箱进程；`scan` 对每个 EXE 候选各起一次，违反 p0-contracts §4.1 只读快照约定 | `exe_sandbox_availability`：探测真实执行（进程缓存，key 含 worker 身份），staging 改用 OS 临时根并事后删除，**不写 installer state**；安装路由（`install_exe_route`）保持状态根 staging + 运行时重核探测 | `cli/runtime.rs`（`bundled_worker`/`exe_sandbox_availability`/`probe_in_temp_staging`；capabilities/scan/preview 共用） |
| R1 | 低 | EXE 动态投影把 `selection_id` 赋给 `release_id`，选择身份冒充 apply 句柄（p1-host-contract §4.1 禁止） | EXE 投影 `release_id` 改用 `dynamic_release_id(source_id, appid)`（与 ZIP 一致），push 前与既有 patch 去重；`selection_id` 仍为 `exe-selection-*` 路由句柄 | `cli/runtime.rs`（`dynamic_patches`） |

### 配套测试

- `task.rs`：`terminal_run_reclaims_its_task_dir`、`cleanup_stale_tasks_reclaims_unlocked_and_spares_locked`、`cleanup_stale_tasks_spares_recent_unowned_and_tolerates_missing_dir`、`cleanup_stale_tasks_reclaims_old_unowned_dir`（unix）。
- `exe_orchestrator.rs`：`precancel_persists_revocation_without_worker_or_game_writes` 增加无任务残留断言；`sandbox_to_authorization_install_retry_reinstall_and_external_edit` 结尾无任务残留断言（ignored，release worker）。
- `cli/runtime.rs`：`read_only_capabilities_never_create_installer_state`（含重复调用缓存路径）、`scan_projects_exe_release_id_without_masquerading_selection`。

## 3. 建议项登记（不阻塞 P3）

| 项 | 内容 | 处置 |
| --- | --- | --- |
| R2 | `resolve_exe` 硬编码仅接受 `kagura-cn`/`kagura-en` 内置来源；其它来源（如 hgamer）EXE 候选会被静默排除 | 登记为 P3 产品决策：EXE 仅放行已验来源白名单；后续如扩展来源需按来源能力派生并补测试 |
| R3 | `exe_report` 的精确键校验未覆盖 `script_inventory`/`certificate`/`diagnostics` 等嵌套区（固定受信 worker，风险低） | 登记，P4/P6 收口时评估是否补齐精确校验 |
| R4 | 报告落盘位置（原 `jobs/` → 现 `reports/<content_id>/`） | 已随 M1 修复 |
| R5 | EXE 重装内容变化报 `reinstall_unowned_or_incomplete`，与 ZIP 路径 `reinstall_content_identity_changed` 语义不完全一致 | 登记，按需统一错误码 |
| R6 | `demo`/`unknown` variant 在 `resolve_exe` 未提前过滤，会在计划中途以 `exe_content_identity_invalid` 失败而非干净 blocker | 登记，P4 展示面接入时评估 |
| R7 | Linux root 宿主下 `recover`/`discard_plan` 也被 `host_identity_blocker` 拒绝（fail-closed） | 登记为预期行为：生产要求宿主先降权整个 CLI/core；P5 Decky 集成必须验证 Loader root 场景的降权路径 |
| R8 | `cleanup.performed` 由 state 推导而非真实清理结果 | 登记，随协议诊断增强处理 |

## 4. 处置后验证证据

- 修复后自动化（最终代码）：Windows core lib 249 passed / 4 ignored、cli 36 passed、集成全绿；WSL2 core lib 250 passed / 4 ignored、cli 36 passed；两平台 clippy `-D warnings` 通过；改动文件 rustfmt 通过（`apps/windows` 既有未格式化差异属 HEAD 遗留，非本次改动）。
- 双平台 ignored 生产测试（release worker）：`precancel_persists_revocation_without_worker_or_game_writes`、`sandbox_to_authorization_install_retry_reinstall_and_external_edit`、`production_available_real_samples_admitted_and_converted` 全通过；真实样本产物摘要与 P2 冻结基准逐项一致：
  - Windows `.artifacts/p3-production/windows-real-sf.json`，WSL `.artifacts/p3-production/linux-real-sf.json`；
  - 2303800 `e7e232c1…` / 3606360 `6d8c5d92…` / 3721970 `24c2fbca…`（两平台与 P2 基准 3/3 全等）。
- 隔离对照（复核前既有）：`.artifacts/p3-production/{windows,linux,linux-root}-controls.json` 全 pass。
- 失败定位记录：debug 构建的 worker 无法在 300s CPU 沙箱限额内完成 1.2 GiB 样本转换（Windows 311s 后 `0xC0000142`、WSL `exit=137`）；release 构建 59s 成功且摘要一致。自动化验收固定使用 release worker（`SF_WORKER_TEST_BIN`、`core-linux-check.sh` 已更新）。

## 5. 复核未覆盖 / 沿用待验

- Windows 标准 Win11 零售机 LPAC/Job 版本矩阵（P4 发布条件）；SteamOS Deck 实机部署（P5）。
- 三条旧 profile 真实包回归（样本不在仓库，沿用 P2 待验记录）。
- R2 来源白名单是否为长期产品决策（P4/P6 复核）。
