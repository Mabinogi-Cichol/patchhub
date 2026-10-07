# P1-0 宿主契约冻结稿初审报告（归档）

日期：2026-10-08（Asia/Shanghai）。评审者：独立 @oracle 会话（ora-2）。评审对象：[p1-host-contract.md](p1-host-contract.md) v1。结论：**暂不能定稿**——方向正确、覆盖大体完整；6 项必须修（MF1-MF6）+ 7 项注意事项（N1-N7），均为文档级。

状态：修订执行中（v1.1，复用 fix-2 会话）；修订后定向复审（attempt 2/3）。

---

# P1-0 门禁独立初审报告（宿主契约冻结稿 v1，attempt 1/3）

评审对象：`docs/features/cross-platform-installer-migration/p1-host-contract.md`（§0-§9）。对照：`design.md` §6/§5.2/§7.3、`p0-contracts.md` §0/§3/§5/§6/§10/§11、现行 `crates/cli/src/{main,runtime}.rs`、`apps/windows/src-tauri/src/main.rs`、`apps/windows/src/main.tsx`、`apps/decky/backend/{bridge,tasks,source_engine,source_service}.py`、`apps/decky/main.py`、`packages/ui/src/index.ts`、`packages/contracts/README.md`。只读评审。

## 结论

冻结稿**方向正确、覆盖面大体完整**：design §6 六组接口、协议 v3 基线、9 阶段事件、终态 `InstallOutcome`、能力/错误形状、幂等/租约/取消、两端宿主迁移要点均已给出；方法白名单 24→16 的自洽性核对通过，删除方法的现行消费者均可归属 P4/P5。但存在 **6 项必须修**（其中 2 项属正确性/安全，4 项属完整性与内部一致性）与若干 P1 注意事项。**暂不能定稿。**

---

## 1. 相符性（design §6 / §7.3）

**已覆盖（接受）**
- design §6 六组：`get_capabilities`、`scan|preview`、`install_patch`、`list_transactions|recover`、`discard_plan|cancel、EOF`、缓存清理 → 分别对应 §1.1 A、§1.1 C、§1.1 `cleanup_cache`。协议 v3/旧版本拒绝、≤64 KiB/≤8 MiB/worker ≤16 MiB+受控报告目录、9 阶段、字节重置/未知 total、递增 sequence、终态内容 ID/归属/恢复/清理诊断、来源事件分离、乱序/旧任务、15s 租约+续租、持久撤销优先、幂等绑定 → §0/§2/§3/§4/§5 均有。
- 方法集合核对：现行 24（`runtime.rs:49-73`）→ v3 = A(8)+B(8)=16；删除 9（`plan_install`/`start_install`/`fetch_artifact`/`migrate_state`/`get_source_state`/`get_source_settings`/`list_source_matches`/`get_source_policy`/`resolve_source_record`）= 24-9+1(`cleanup_cache`)=16，**自洽**。
- 删除归属可落地：Windows `start_install` 是 Tauri 命令且内部已调 `install_patch`（`src-tauri/src/main.rs:727`），`Operation` 枚举含 `MigrateState/GetSourcePolicy/ResolveSourceRecord` 待删；Decky `plan_install/start_install/fetch_artifact/migrate_state` 由 `main.py`、`publisher.py:664-667`、`exe_installation.py:376-379` 使用（P5）；`get_source_state`（`source_service.py:86`）、`get_source_settings`（`source_engine.py:85`）由 P5 消费者删除。均已在 §1.1 D/§6.1/§6.2/§1.3 标注。

**未充分覆盖**
- **design §6 要求同步 `packages/contracts/README.md`**（“P0 同步 契约、Rust 类型、宿主白名单与共享 UI”），冻结稿 §6.3 只提 `packages/ui/src/index.ts`，未提 contracts README；而该文件当前是 **v2 陈旧内容**（`README.md:3/8/12/25/27`：`protocol_version 2`、白名单含 `plan_install/start_install/fetch_artifact`）。→ 见必须修 MF2。
- **design §7.3 清理面**：冻结稿 §9 声称“覆盖 §7.3 清理”，但只列了 RPC 可见项；非 RPC 项未归属：`source_engine.py`/`plugin_settings.py` 的 `engine-switch.json`、`EngineSwitch*`/conflicts（§7.3 第 2 行），以及 SourceService 的 `matches/cached_candidates/_rules/builtins`（第 6 行）。→ 见 MF3。
- **design §6.1**“scan/list_transactions 一致只读快照/短时协调、真实 busy 仍报告”未复述或交叉引用（p0-contracts §4.1 已冻结）。→ P1 注意事项 N4。

## 2. 正确性 / 可落地性

**事件契约**：`install_progress` 命名、9 阶段、`backing_up/writing/committing` 折叠为 `applying`+`substage`、`rolling_back` 忽略取消、`sequence` 从 0 严格递增、终态一次、宿主按 `(request_id, sequence)` 去重 —— 自洽，与 design §6 一致。当前 `apps/decky/backend/bridge.py:180-186` 仅对 `source_*` 校验 sequence、对 `progress` 仍读嵌套 `row["progress"]`；契约 §2.2 以**平铺**字段列出 → 形状变化未显式说明。→ N2。

**终态语义不完整**：§2.3 `InstallOutcome.state` 仅列 `committed|superseded|rolled_back|needs_recovery|uninstalled`。但 `install_patch` 在**计划建立前**的失败（下载/分析/转换/计划失败）与**下载期间用户取消**的终态未定义（应返回 `ok:false` 错误还是某 state？）；`uninstalled` 也与 design §2.2“不新增长期备份/卸载开关”矛盾（`install_patch` 不会产生该 state）。design §6 明确“不伪装 cancelled/completed”。→ 必须修 MF4。

**能力协商**：§3.1 `installer.links.*` 按平台探测、`blockers[].code` 复用 UI reason、只阻断 EXE —— 可实施。但 `formats.exe:"supported"` 与示例 `links.exe.available:false` 并存，未说明“可安装性以 `links` 为准”，UI 可能误判。→ N1。

**幂等/租约/取消**：绑定元组、账本 `installer/rust-v1/requests.sqlite`、同绑定返回原终态/异绑定冲突、崩溃不二次写、15s 租约+`renew`、持久撤销顺序、提交优先 —— 覆盖 design §6/§5.2/§5.1、p0-contracts §5。现行账本绑定为 `(instance, release_id, operation)`（`runtime.rs:885`），契约扩展含 `selection_id`/`content_id`，方向一致；但 `content_id` 需分析后才解析，重放/冲突判定时序未说明（→ N6）。**§6.1 的 request_id 复用策略有正确性问题（MF1）。**

**安全边界**：UI 不传配方/路径/roots/keys 已冻结（§0.1/§6.3）。但 `install_patch` 的 `archive` 未限定仅签名 ZIP 路由，动态选择路由也可携带本地路径，与 design §2.1“动态安装只接受后端解析 URL、不新增任意 EXE 导入”存在被绕过的余地。→ 必须修 MF5。

## 3. 矛盾 / 风险

- **内部不一致（必须修）**：§0.1 把 `discover_game` 列入 `serve-task` 流式方法，而 §1.1 B 以普通响应列出、§7 测试未覆盖流式，且现行 `discover_game` 是 `serve` 一次性方法（`runtime.rs:94` `SOURCE_STREAM_METHODS` 只含 `scan_sources/refresh_sources`）。→ MF6。
- **与 design 冲突（必须修）**：§6.1 “`request_id` 按 `(instance, selection/release, operation)` 复用”会与 design §2.2/F5 的**同内容重装**冲突（同键→同 id→幂等短路，返回旧终态而不再写入），也与现行 Windows 每次生成唯一 id（`src-tauri/src/main.rs:72-77`）不符。→ MF1。
- **p0 冲突**：无实质冲突。`p0-contracts §10/§11` 的“宿主契约延后 P1”由本稿承接（§9），但未列入“P1 定稿后回写 p0-contracts §11 状态”的迁移动作（建议 N7）。
- **宿主迁移影响可能被低估**：
  1. **发布时序**：P1 即把 `main.rs PROTOCOL_VERSION=3` 并删方法，而 Decky 宿主到 P5 才切换；由于旧版本被拒，任何在 P1–P4 期间把 v3 核心随 Decky 包发布的组合都会**整体不可用**。契约未规定“v3 核心与各宿主 P4/P5 同包发布、P1–P3 不面向 Decky 用户发布”。→ N5。
  2. **UI 细节**：`main.tsx` 现依赖 `progress.stage === "writing"`（line 502）与 `CoreOperation` 含 `migrate_state`（line 29/59），§6.3 未点名这些具体改动（仅列 DTO 表）。→ N3 附带。
  3. **reason 键**：`packages/ui/src/index.ts` 现有 `publisher_*`/`exe_profile_*` 文案（:262-355）依赖旧生产者；契约 §6.3 已注明随 P5/P6 删除，但未列“旧键在这些阶段仍被生产”的过渡约束。

## 4. 必须修项（最小修改）

- **MF1（正确性）**：将 §6.1 的 request_id 策略改为“宿主为**每次用户发起的 install/reinstall 动作**生成新的唯一 `request_id`；仅对该动作的传输/UI 重试复用同一 id；终态后新动作必须换新 id”。删除“按 (instance, selection/release, operation) 复用”的确定性派生表述，并同步 §5.1/§7.1 幂等测试（含“同内容重装需新 request_id”用例）。
- **MF2（完整性）**：在 §1.3/§6.3/§7 增加 `packages/contracts/README.md` 的更新与校验（协议 v3、16 方法白名单、事件/终态形状、`error.detail`），并注明删除 `plan_install/start_install/fetch_artifact` 描述。
- **MF3（完整性）**：§9 的 §7.3 覆盖声明改为准确表述，或补一行“非 RPC 清理项（`source_engine.py`/`plugin_settings.py` 的 engine-switch/`EngineSwitch*`；SourceService `matches/cached_candidates/_rules/builtins`）随 P5/P6 按 design §7.3 逐项删除”。
- **MF4（正确性）**：在 §2.3 增补“非终态失败/取消”段：计划建立前的失败返回 `ok:false`+`error.code`（含 `archive`/下载/分析/转换/计划失败码）而非 `InstallOutcome`；下载期用户取消的 terminal 语义；并删除或说明 `InstallOutcome.state` 中的 `uninstalled`（与 design §2.2 对齐）。
- **MF5（安全）**：在 §1.1 明确 `archive` **仅对签名 `release_id` 路由有效**，动态 `exe-selection-*` 路由传 `archive` 一律拒绝（如 `archive_not_allowed_for_selection`）；并补核心侧校验（普通文件、`.zip`、`no_links`、大小上限），与 design §2.1/§4.1 一致。
- **MF6（一致性）**：统一 `discover_game` 的执行面——要么保留 `serve`（并修正 §0.1），要么明确其为 `serve-task` 并同步 §1.1 B 模式列、§7 流式事件/取消测试与迁移动作。

## 5. P1 注意事项（不阻塞定稿，但需登记）

- **N1**：说明“可安装性以 `installer.links.*.available` 为准，`formats` 仅表示核心版本支持该类格式”。
- **N2**：明确 `install_progress` 是平铺字段（现行 `row["progress"]` 嵌套将被替换），并列入 P4/P5 宿主改动。
- **N3**：删除 `list_source_matches`/`get_source_policy`/`resolve_source_record` 前，复核 `source_service.py:18-19` 所述“optional export or Windows-host reads”，确认无隐藏消费者；否则把删除推迟到 P5 消费者切换后。
- **N4**：交叉引用 p0-contracts §4.1 的 `open_ro`/只读快照与 `installer_busy` 语义，覆盖 design §6.1。
- **N5**：写明发布时序——v3 核心与两端宿主按 P4/P5 同包发布，P1–P3 不面向 Decky 生产用户发布；避免旧版本拒绝导致中途不可用。
- **N6**：细化 `install_patch` 重放算法：`content_id` 在计划后可写账本，重放时比较 `selection_id|release_id`+`operation`+（已有计划的）`content_id`；说明“已解析内容”在未解析前如何判定冲突。
- **N7**：定稿后在 `p0-contracts.md §10/§11` 回写“宿主契约已冻结（指向本稿）”，并同步 `progress.md`/索引。

## 6. 放行判断

**P1-0 暂不能定稿。** 结构、命名空间、阶段、能力/错误形状与迁移归属基本正确；但 MF1（幂等 id 语义与重装冲突）、MF4（终态语义缺口）、MF5（本地 archive 路由限制）属**正确性/安全必修**，MF2/MF3/MF6 属**完整性/一致性必修**。上述均为文档级修订（不涉及设备或重跑），完成 MF1–MF6 后即可作为 P1 实现基线。N1–N7 建议一并登记为 P1 注意事项。

未重开已接受项（protocol v3、确定性编码、ID 分离、installer/rust-v1、worker 协议与限额、取消/租约）；本次未发现对已冻结 P0 契约的回归。
