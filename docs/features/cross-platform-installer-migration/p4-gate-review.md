# P4 Windows 接入独立复核与处置

日期：2026-10-09（Asia/Shanghai）。范围：P4 Windows 宿主/前端接线、core 边界收口（R3/R6/H2）、契约 README 与 Windows runner（[design.md](design.md) §8 P4）。施工记录：[progress.md](progress.md)。实施与验收记录：[p4-windows.md](p4-windows.md)。

独立复核：只读 oracle 会话，对照 [p1-host-contract.md](p1-host-contract.md)（protocol v3 冻结契约）、[design.md](design.md) §2.2/§8、[p3-gate-review.md](p3-gate-review.md) §3 登记项，逐项核对全部 P4 新增与修改代码；初审后经修复由同一会话定向复审。本文件归档结论、必修/建议项处置与验证证据。

## 1. 初审结论（2026-10-09）

**有条件放行**：宿主 v3 主链（协议 3、`selection_id`/`release_id` 互斥与前缀路由、`archive` 限制、`install_progress` 平铺解析与 `(request_id, sequence)` 去重、`renew`/`cancel`/EOF 生命周期、删除旧命令与降级、错误 `detail` 组合）、前端 N1 判据/动态路由、R6 变体预过滤、R3 嵌套键校验、README 16 方法集合与 `runtime.rs::METHODS` 精确一致——逐项与冻结契约一致；未发现可越权的 RPC/宿主注入面或状态机破坏。须处置 2 项必修（H1、H2）与 1 项待证据（M3）；R2 裁定保留并登记。

## 2. 必修项与处置

| 项 | 级别 | 问题 | 处置 | 落实位置 |
| --- | --- | --- | --- | --- |
| H1 | 高 | `scan_sources` 任务未转发 `renew`：核心对每个 serve-task 启动 15s 租约看门狗（`crates/cli/src/main.rs:198-205,311-322`），Windows 宿主只对安装任务启动续租线程；来源扫描超过 15s 会被自动取消，违反 §5.2「宿主转发续租」 | 来源任务与安装任务同模式：3s `renew` forwarder（纯函数 `should_forward_renew` + 泛型 `spawn_renew_forwarder_with_interval`），任务收尾（终态/取消）停止 + unpark + join；`cancel` 后迟到 `renew` 无效 | `apps/windows/src-tauri/src/main.rs`（`start_source_scan`、forwarder） |
| H2 | 高 | 动态 EXE「重装」判据失效：前端按 `[release_id, selection_id, content_id]` 匹配已提交事务，但 `list_transactions` 的 `Transaction` 缺 `selection_id`/`content_id`；EXE 事务 `release_id=exe-content-*` ≠ 投影 `release_id=source-<id>-<appid>`，安装成功后 `reinstall_available` 恒 false、按钮回落「安装补丁」 | `Transaction` 增 `selection_id`/`content_id`（`#[serde(default)]`，旧 journal 可读）；新增 `prepare_with_binding_at`，事务建立时随计划一次性持久化（动态 ZIP `Some(zip-selection-*)`+`None`；动态 EXE `Some(exe-selection-*)`+`Some(exe-content-*)`；签名 `None`）；`list_transactions(_at)` 直出 | `crates/core/src/install.rs`、`installer/orchestrator.rs`、`installer/exe_orchestrator.rs` |
| 低危 | 低 | `truncate_detail` 输出可达 2051 字节（超 §3.3 ≤2KiB）；契约 §2.4/§2.8 对「终态后迟到事件」表述互相矛盾 | 截断为省略号预留空间（输出含 `…` ≤2048）；契约统一为「终态后迟到 `install_progress` 忽略（§2.4）；终态后第二个终态响应行 → `invalid_core_response`（§2.8）」 | `apps/windows/src-tauri/src/main.rs`；[p1-host-contract.md](p1-host-contract.md) §2.4/§2.8 |

### 配套测试（新增/更新）

- 宿主：`renew_forwarding_stops_on_stop_or_cancel`、`renew_forwarder_writes_until_stopped`、`renew_forwarder_ignores_renew_after_cancel`、`error_detail_is_truncated_to_the_frozen_bound`（含多字节 UTF-8 边界）。
- core：`transaction_binding_tests`（旧 journal 无字段反序列化为 `None`、两字段序列化输出）、`prepare_with_binding_records_selection_and_content`、`prepare_with_operation_leaves_dynamic_binding_unset`、动态 ZIP 端到端断言扩展（`install_zip_commits_with_the_frozen_stage_sequence`）、动态 EXE 断言（`#[ignore]` 端到端测试内，静态编译通过）。

## 3. M3（R3 严格校验对真实报告兼容性）关闭证据

- Windows runner（release worker + 真实样本）：ignored 生产测试 3/3 通过，`production_available_real_samples_admitted_and_converted` 905.76s 完成 2303800/3606360/3721970 分析/转换；日志 `.artifacts/p4-windows/windows-check.log`，证据 `.artifacts/p4-windows/windows-real-sf.json`（artifact 摘要 `e7e232c1…`/`6d8c5d92…`/`24c2fbca…` 与 P2 冻结基准逐项一致）。
- 结论：新增的 `script_inventory`/`certificate`/`diagnostics` 嵌套键严格校验不误拒真实 worker 报告。**M3 关闭**。

## 4. R2 裁定（保留并登记）

- `resolve_exe` 仅接受内置 `kagura-cn`/`kagura-en`（启用、订阅、builtin）为**产品白名单决策**，与 design §2.2/§4.1 一致；hgamer/其他来源 EXE 候选以 `exe_source_disabled` blocker 呈现（显示但不可安装，非静默排除）。
- **P4 放行附加条件**：实机验收须确认 2303800 的 EXE 补丁经 kagura-cn/kagura-en 可解析并安装。若该 EXE 仅由 hgamer 提供，须升级为产品决策（按来源能力派生白名单并补测试），**不得静默通过**。

## 5. 建议项登记（不阻塞）

| 项 | 内容 | 处置 |
| --- | --- | --- |
| S1 | `patchInstallable` 对 `managed_zip` 未按 N1 校验 `links.zip.available`（Windows 恒 true；Linux-root/Decky 为观察面） | 登记 P5 |
| S2 | UI `source_id === "hgamer"` 硬编码与核心白名单/blocker 重复 | 登记 P6 统一核对 |
| S3 | Windows runner 证据变量名沿用 `P3_EVIDENCE_PATH` | 登记 P6 更名评估 |
| S4 | `loadCapabilities` 首次契约失败后本进程内不再重试（fail-closed） | 登记，可接受 |

## 6. 处置后验证证据

- 修复后回归（Windows 本机）：core lib **255 passed / 4 ignored**、cli **37 passed**、集成 catalog 8 / install 40 / localized 4 / steam_scan 6 / trust 3、windows 宿主 **20 passed**；core/cli/windows clippy `-D warnings` 与改动文件 rustfmt 通过；前端 `tsc --noEmit` + `vite build` 通过。
- Windows runner（修复前基线，M3 证据）：core/cli 全绿 + ignored 3/3（见 §3）。
### 定向复审（2026-10-09，同一 oracle 会话）

**结论：通过** —— H1/H2 已正确落实且无新回归；M3 可关闭；**P4 可进入实机验收阶段**。复审独立复跑 core `--lib`、cli、windows（全绿）与三 crate clippy `-D warnings`（干净）；确认 forwarder 无泄漏/死锁（共享 `cancelled`、stop→unpark→join 后才清槽；窗口关闭走 cancel 路径）、迟到 `renew` 无法复活已取消任务、`truncate_detail` 边界正确、`prepare_with_binding_at` 随计划一次性写入无半写、旧 journal 兼容、前端 `committedKeys` 闭环、`Transaction` 字面量 4 处同步、无 schema/golden 影响。

- 证据溯源（登记）：§3 runner 日志早于最终 H1/H2 绑定测试（日志 core 255 = 251 pass；当前 259 = 255 pass，差值为 4 个绑定测试）。因 H2 未触碰 `exe_report` 校验路径且 ignored 生产测试不建事务，M3 结论不受影响；复审已独立验证当前树全绿。
- 复审建议（登记）：runner `-Clippy` 步未执行（由复审独立 clippy 覆盖）；宿主折叠的 `"{code}: {detail}"` 展示串可略超 2048（`detail` 字段本身 ≤2KiB，符合契约，仅注释可点明）。


## 7. 复核未覆盖 / 沿用待验

- 2303800 真安装/生效、动态 ZIP/EXE 实机路径、恢复/取消/离线历史（实机验收阶段，见 [p4-windows.md](p4-windows.md)）。
- Windows 标准 Win11 零售机 LPAC/Job 版本矩阵（P4 发布条件）。
- R2 附加条件实机确认（见 §4）。

## 8. 实机验收发现与补充处置（2026-10-09）

- **多镜像误伤（裁决 ora-3）**：kagura-cn 候选对同一 CN 补丁提供两域同名 EXE 镜像，被旧冻结语义「>1 镜像 → `exe_download_ambiguous`」误判为来源歧义（真实数据普遍多镜像，导致动态安装不可用）。裁决：**同候选同名多域镜像按 mirrors 原顺序确定性择一**（下载后按实际字节建立内容身份），`*_download_ambiguous` 收窄为合法镜像 basename 异构守卫；非法镜像跳过。ZIP 侧为契约修订（[p1-zip-orchestration.md](p1-zip-orchestration.md) §2.1(6)/§2.3），EXE 侧实现修复并补冻（[p3-runtime.md](p3-runtime.md) 实施范围 7）；实现 `resolve.rs::select_download_url`（ZIP/EXE 共用），测试更新后 core 259 / cli 37 全绿。
- **TUN/fake-IP 代理环境冲突（登记 S6）**：本机 Mihomo TUN 模式下域名解析为保留段（198.18.0.0/15），公共地址校验按设计拒绝动态下载（`download_private_address`）；临时关闭 TUN 后验收通过。产品需评估受控支持或增强诊断；不阻塞 P4。
- **取消文案缺口（已修）**：计划前取消 `error.code="cancelled"` 无 UI 映射 → `reason()` 已补「安装已取消，没有修改游戏文件。」。
- **实机验收结论**：2303800 真实下载 → 隔离分析 → 转换 → 真安装 → 同内容重装 → 计划前取消 → 游戏内确认全部通过；账本 `selection_id`/`content_id` 落账、清理与报告路径符合契约。残余：动态 ZIP 实机（待样本）、断网重开历史项、更早 Win11 版本矩阵。

## 9. 后续修复的独立裁决与实机复验（2026-10-09）

- **patchmoe 缺口（ora-4 裁决）**：索引解析退化（`parse_rows` 仅首个 table，3/116 条）与安装接线缺失（`SourceMatch` 无安装消费者）。裁决 **B2**：确定性 `SourceMatch → SourceCandidate` 投影并入 `dynamic_zip_candidates`（复用 `zip-selection-*` 全链路，不改 `discovery_only` 语义）。实现与契约修订见 [progress.md](progress.md)「P4 后续」节、[p1-zip-orchestration.md](p1-zip-orchestration.md) §2.1(4)。
- **动态 ZIP 语言门（ora-5 裁决）**：方案 d——`FactsExpectation.require_language`（默认 true）：发布商来源（`discovery_only`）保留语言门，索引来源不设门；语言仍进 `recipe_identity`。契约修订 [p1-zip-orchestration.md](p1-zip-orchestration.md) §2.1(8)/§2.2/§2.3/§2.5/§4.4/§5.1。
- **实机复验**：2743810 安装（`patch.xp3` 写入游戏根、事务 `committed | patchmoe-2743810`）与重装通过；宿主 refresh 通道修复（serve-task）与 cmd 弹窗修复（`MainWindowHandle=0`）实机确认。证据 `.artifacts/p4-windows/gui-2743810-*.png`。
- **登记**：kagura `discovery_pagination_cycle` / `discovery_index_incomplete` 两条诊断与 patchmoe 无关，单独排查（来源质量专项，随 P5/P6 评估）。
