# P1 门禁初审报告（归档）

日期：2026-10-08（Asia/Shanghai）。评审者：独立 @oracle 会话（ora-6）。评审对象：P1 全交付（p1-host-contract v1.2+增量、p1-zip-orchestration、crates/core installer/sources、crates/cli、测试）。结论：**有条件放行进入 P2**；必修 F1-F6 + 低危 F7/F8。

状态：修订执行中（F1-F8；代码 fix-14 / 文档 fix-15）；修订后定向复审（attempt 2/3）。

**补录（独立跑测）**：后台独立跑测完成：`cargo test -p steam-patch-core -p steam-patch-cli`（rustc/cargo 1.94.0，`-j1`）**全绿，exit=0**，与仓库记录一致；第 0 节的“未能跑测”说明由本补录取代。

---

# P1 门禁独立初审报告（review attempt 1/3）

## 0. 评审方法、范围与证据边界

- 逐文件审阅了：`p1-host-contract.md`(v1.2+增量)、`p1-zip-orchestration.md`(P1-3b-3 + §9)、`p0-contracts.md`(§1/§2/§2.5/§3/§4.1/§5/§7)、`design.md`(§7.3/§8/§9)、`progress.md` P1 各节；`crates/core/src/installer/**` 全部 8 模块；`sources/{canonical,mod,store,registry,validate,model,session}.rs`；`crates/cli/src/{main,runtime}.rs`；`crates/core/tests/install.rs` 测试清单；git log/status。
- 工作区干净，P1 改动按功能分批提交（`6873723`→`df84865`→`6fab735`），无未提交残留。

## 1. §8 P1 门槛逐项证据与结论

| 门槛项 | 证据（测试/符号/行为） | 结论 |
|---|---|---|
| **攻击** | ZIP 穿越/NTFS 保留名/ADS/设备名/symlink/高压缩比：`tests/install.rs:624/650/668/714/737/800`；`zip_recipe.rs:657/673/689/728/745` + `no_links`（`zip_recipe.rs:497`）；下载 URL 主机/端口/凭据/`.zip` 收紧：`acquire.rs:99`+`validate.rs:221`；archive 仅签名路由、动态 selection 携带 archive 拒绝：`runtime.rs:952-955`+测试 `install_patch_route_matrix`；state/game 越界：`install.rs:758`+测试 `fixture()`；伪造 path/ID 越界：`tests/install.rs:714/726`。 | **通过（自动化）**；真实写入/隔离阻断待 P4/P5。 |
| **冲突** | 路由矩阵（签名/`zip-`/`exe-`/未知前缀/互斥/皆缺）：`runtime.rs:930-984`+测试；`request_id_binding_conflict`：`orchestrator.rs:163-170`+测试；plan blockers：`install.rs:680/1038/800`+测试。 | **通过（自动化）**。 |
| **取消** | 计划前/下载中/apply 中/迟到取消：`orchestrator.rs` 测试；租约过期落 `revoked`：`lease_expiry_after_plan_persists_revocation`；事务层 `tests/install.rs:16/50/94/1105`。 | **通过（自动化）**；见 F4/F6 语义偏差。 |
| **重装** | 同内容 `committed`、新字节 `reinstall_content_identity_changed`、无主 `reinstall_unowned_or_incomplete`，签名 ZIP 重装。 | **通过**。 |
| **恢复** | `recover_at` → `rolled_back`/`needs_recovery`：`orchestrator.rs` 两个测试；`tests/install.rs:426/465/517/562/603/983` 完整恢复矩阵。 | **通过**。 |
| **幂等** | 重放返回同 `transaction_id`、异绑定冲突、计划前失败可重试且不写 `error`、同内容重装换新 `request_id`；账本单测 `requests.rs`。 | **通过**；见 F6 形态不一致。 |
| **无 Python 来源导入** | `crates` 内导入链符号零匹配；`SourceStore::open(state_dir)` 直接初始化（`store.rs:144-176`）；测试 `legacy_python_state_is_not_read_on_open`。 | **通过**。 |
| **无哈希复刻** | `python_json`/`gen_identity_golden`/`identity-golden.json` 生产路径零匹配；`canonical_bytes` 唯一入口；golden 重写为 `encoding-golden.json`。 | **通过**。 |
| **无旧规则接受** | `config.schema_version != 2 → invalid_sources_config`（`mod.rs:164`）；测试 `rules_schema_version_must_be_two`。 | **通过**。 |
| **原签名 ZIP 无回归** | 签名路由迁新根/新账本/授权；测试 `signed_install_uses_the_installer_root_and_new_ledger` 等；保留 `ready_release`/`download::fetch`/`prepare+apply` 语义。 | **通过（自动化）**；P4 实机待验。 |
| **来源订阅无回归** | `set_game_source`/`game_subscribed`/`rules`/`matches` 语义保留；订阅测试通过。 | **通过（自动化）**。 |

> 记录误差（低）：`progress.md` 的集成测试计数有 60/61→58 的漂移，建议收尾以单次 `cargo test` 汇总统一。

## 2. `p1-zip-orchestration.md` §9 设计缺口裁决（6 项 + fix-13 残留）

| 项 | 裁决 | 理由与动作 |
|---|---|---|
| §9.1 `!discovery_only` 校正为仅 `enabled` | **接受** | 与设计一致；`resolve.rs:288` 的 `!rule.enabled` 因上游过滤冗余但无害。 |
| §9.2 `source_not_enabled/subscribed/selection_stale` 不可达 | **接受（需更正文档）** | 前半正确；但 **`selection_stale` 实际可达且有测试**（`stale_discovery_yields_selection_stale`）。注记第 2 条自相矛盾，应改写。 |
| §9.3 buildid 经 `expectation/conditions_sha256` 交接 | **接受** | 自洽，`verified_buildids=[现场 buildid]` 与 Python publisher 语义对齐。 |
| **§9.4 签名路由授权绑定为实施确定值** | **接受，但需收窄设计（P1 内折入）** | 实施值可用但语义较弱：`settings_revision` 与来源设置无关、重核仅覆盖 facts + revoked + lease。建议冻结绑定值并收窄 §4.1 step 2。**不阻塞 P2**。 |
| §9.5 `error.detail` 经 `\u{1}` 分隔 | **接受（低）** | 协议层已拆分；建议 P4 前 typed error。 |
| §9.6 空 journal 容忍 / `cleanup_cache` 形状 | **接受** | 测试覆盖；形状保持。 |
| fix-13 残留（Decky `import_report`/`engine-switch`/`MAX_OVERRIDES_BYTES`/doc 引用） | **接受（P5/P6）** | 属 P5 消费者切换；注意 v3 核心已拒绝 `get_source_state`，继续遵守 **N5** 发布时序。 |

## 3. 正确性与风险

**F1（高 / 需修改·阻塞门禁结论）— `source_not_released` 门实际不可达。**
`resolve.rs:320-329` 的 released 门通过 `registry.matches(game, true)` 查日期证据，但 `matches` 对 `rule.discovery_only` 直接跳过；而动态 ZIP 候选只来自 discovery-only 规则。因此该码在动态路径**永不返回**，`scan` 投影（`runtime.rs:1279-1300`）也未加入该 blocker。设计 §2.1 step 7、§4.4、p1-host-contract §1.1 均承诺此码，属契约承诺 vs 实现缺口。动作：实现（扩候选日期模型）或从契约显式删除并记录。**不可静默保留。**

**F2（中 / 需修改）— `install_patch` 可经 `serve`（非 serve-task）执行。**
`main.rs::serve` → `respond` → `runtime::execute_controlled`，`METHODS` 含 `install_patch`，无执行面门禁。经 `serve` 执行使用永真租约与永不置位的 cancel，**用户无法取消**。动作：`serve` 分支拒绝任务面方法（`task_required`）。

**F3（中 / 需裁决）— 租约未落盘。**
全仓库无 `lease.json` 写入。p0-contracts §5.1、p1-host-contract §5.5、设计 §3.5 将“计划建立时写 `lease.json`”列为 P1 交付；实现仅内存 `LeaseState` + 看门狗。动作：P1 内补做或正式降级登记。

**F4（中 / 需裁决）— 动态路由租约检查用固定 `now`。**
`install_zip_route` 取一次 `trust::now()`（`runtime.rs:1043`），`lease_active(fixed_now)` 恒真：`installer_lease_expired` 生产路径形同虚设；真实过期仅靠看门狗（≤500ms 竞态）。签名路由用实时时间，两路由不对称。动作：动态路由改实时或不实。

**F5（中）— 只读 `scan`/`preview` 通过独占 `journal.lock` 读取 plan。**
`dynamic_detection`→`get_plan_at`（`try_lock_exclusive`）与 `committed_changes` 同。p0-contracts §4.1 要求 open_ro。后果：并发时 detection 退化 `unknown`、preview 报 `installer_busy`；读路径有写副作用。动作：新增只读 `get_plan_ro`。

**F6（中低）— 计划后失败返回形态不一致 / 幂等重放形态漂移。**
`fail_after_plan` 先 discard+record_terminal 再返回 `Err(code)`：首次 `ok:false+error`，重放返回 `state="rolled_back"`；租约过期同。契约 §2.3 要求计划后取消→`rolled_back` 的 `InstallOutcome`。动作：统一形态。

**F7（低）— `SourceStore::open` 在 `schema.json` 缺失但 `settings.json` 存在时会覆盖用户设置。** 动作：优先复用现有 settings。

**F8（低）— 语义/文档债。** `installer/mod.rs` 头注释过时；`authorization.rs` `exe-policy.json` 注释；`support_level=ManagedZip` 借用；`source_not_enabled/subscribed` resolve 路径无 fixture。

## 4. 放行判断与 P2 条件

**结论：P1 有条件放行进入 P2。** P2 主体（SF 对象图/AST/IR、worker 协议、LZMA2、确定性 ZIP）与动态 ZIP 路径低耦合。

**必须修项（进入 P2 前或并行完成，不得静默延期）：**
1. **F1**：裁决 `source_not_released`（实现或从契约删除并登记）。
2. **F2**：`serve` 面拒绝 `install_patch`。
3. **F3/F4**：租约落盘与动态实时检查——补做或正式降级记录。
4. **F5**：scan/preview 只读 plan 路径。
5. **F6**：统一计划后失败形态。

**P2 注意事项：**
- `installer-format`/`installer-worker` 消费 p0-contracts §6 冻结的 worker schema；不改 `installer/rust-v1` 布局与 `install::Store` 事务算法。
- EXE 动态路由授权绑定需独立设计；`exe_route_unimplemented` 占位在 P3 移除时同步回写 `get_capabilities.links.exe`。
- `conditions_sha256`/`candidate_sha256` 为调用方占位；引入真实条件求值器时保证重核一致，必要时 bump `selection_version`。
- 继续遵守 **N5**：v3 核心不得随旧 Decky 宿主发布。
- 收尾补一次权威 `cargo test` 汇总，消除计数漂移。
