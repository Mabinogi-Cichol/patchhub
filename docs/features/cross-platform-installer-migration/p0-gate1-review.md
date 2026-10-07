# P0-Gate 1 独立初审报告（归档）

日期：2026-10-08（Asia/Shanghai）。评审者：独立 @oracle 会话（ora-2）。评审对象：[p0-contracts.md](p0-contracts.md) v0.3、[p0-sandbox-probe-windows.md](p0-sandbox-probe-windows.md)、`crates/installer-worker/**`。

结论：**条件放行（conditional pass）**——无阻塞级返工；C1（旧 discovery/index 失效语义）为 P1 前置修订，另有 7 组文档级修订。C1 若保持原文则升级为阻塞。

状态：修订执行中（fix-2，15 条裁决）；修订后定向复审（re-review 1/2）。

---

# P0 门禁独立初审报告（只读）

评审对象：`p0-contracts.md` v0.3（冻结草稿）、`p0-sandbox-probe-windows.md`、`crates/installer-worker/**`，并抽查 `install.rs`、`sources/{canonical,session,registry,store,validate,model}.rs`、`exe_worker.py`、`sf95_worker.py`、`exe_generic/profiles.py`、`runtime.rs` 等。

总体判断先行：**架构方向与原型证据成立，无需要重做原型或重跑探针的阻塞；但 v0.3 作为“待冻结契约”存在 1 项较严重的失效语义错误与若干必须先修的缺口。修毕即放行 P1（条件见文末）。**

---

## A. 关键决策正确性与可落地性

| 决策 | 结论 | 依据 / 最小修改 |
| --- | --- | --- |
| `protocol_version = 3` + 硬拒绝 | 接受 | 现行 `crates/cli/src/main.rs:16` 为 2，`apps/windows/src-tauri/src/main.rs` 多处硬编码 2（:221/:357/:725/:833）；P4/P5 必须同步。RPC/事件字段级契约未在本文件冻结（见 C5）。 |
| canonical 确定性编码 | 需修改（小） | 方向正确（UTF-8 字节序=码点序，Rust `String` Ord 成立）。但 §1.1 入口声明 `canonical_bytes(value: &serde_json::Value)` 与同句“字段集合以结构体 `Serialize` 的显式字段为准，不依赖 `serde_json::Value` 运行时键集”自相矛盾：Value 无法约束字段集。最小修改：改为 `canonical_bytes<T: Serialize>(&T)`，或明确“调用前一律 `serde_json::to_value(结构体)`”。另“字符串按原样 UTF-8”应补“仅转义 JSON 必需字符”。 |
| URL 规范化（原样 path） | 接受 | `validate.rs:126-176` 的 `split_url` 确实不归一化 path，冻结为“原样”与现行一致；“path 百分号编码待调研”属增量决策，但须声明其变化同样要 bump 编码版本。 |
| `query_key` 纳入 `encoding_version` 的失效语义 | **需修改（重要，P1 前置）** | 见 C1：仅靠 query_key 变化**不能**失效 confirmed discovery 与 discovery-index。 |
| selection/content ID 分离 + 终态 release_id 回填 | 接受（附澄清） | `candidate_binding` 字段集与 `exe_profiles.py:125-129` 一致；`identity` 与 `exe_generic.py:195-196` 一致。但 §0 声明的 `INSTALLER_SELECTION_VERSION` / `INSTALLER_CONTENT_VERSION` 在 §2.1/§2.2/§2.3 中**没有任何引用**（只用 §1 编码原语），属悬空常量：需写明落点（写入记录/授权绑定）或删除。 |
| `installer/rust-v1/` 布局 + journal 真相源 | 接受（附补漏） | 与 `install.rs:397-440` 的 `journal.lock/journal.sqlite/jobs/<uuid>` 结构可复用。补漏见 C6：新请求幂等账本位置、`receipts/` schema 与 `instance_hash` 未冻结（design §5.2 的同内容重装判据依赖它）。 |
| `open_rw`/`open_ro` 锁拆分与锁顺序 | 需修改 | 方向接受（现行 `Store::open` 对 `list_transactions` 也取独占，确是问题）。但 §4.1 内部矛盾 + 重核路径未定义，见 C2。 |
| 取消/租约（15s/900s、持久撤销、needs_recovery） | 接受 | 15s 对应 `tasks.py:14`；`revoke()` 的 `sync_all+sync_dir`（`install.rs:1485-1497`）与“先持久撤销”一致；900s 由 `validate_job`（:488）强约束。注意 Windows 端续租路径需在 P1 一并定义。 |
| worker 协议与限额 | 接受（限额表述需改） | 协议字段与 `sf95_worker.py:47-64/67-125`、`exe_worker.py:42-93` 一致；限额见 B。 |
| 规则 schema 仅 2 | 接受（§7 描述需改） | 见 C3。 |

---

## B. P0-B 证据与契约一致性

**§6.2 回填总体忠实**：内存（`memory_limit`/`memory_limit_control`）、`JOB_TIME`（`job_time_limit`，退出码 `0xC0000044`）、CPU rate 嵌套不生效（`cpu_hard_cap` not_covered + `host_in_job`）、LPAC 不能建子进程（`child_process_controlled`/`active_process_limit`）、环境块（发现 3）、注册表（发现 8）、worker 目录 DACL（发现 10）均能逐条对应原始 JSON。以下为偏差与需补强：

1. **`JOB_TIME` 的“主手段”表述过度**（需修改）。证据 `job_time_limit`：`PerJobUserTimeLimit=1s` 的 10s busy 实际 `wall_ms=5380` 才终止——user-time 限额粒度粗（约 5x 超调），且 launcher 把它记为 `reason=Completed`（`probe_runner.rs:1117-1142` 仅依据 exit code）。契约应改述为：**wall-clock 超时（180/240/480s）是硬时限主手段，`JOB_TIME` 是 CPU 兜底**，不得宣称精确 CPU 计费。
2. **`PROCESS_MEMORY` 与 `RLIMIT_AS` 不等价**（需修改）。`JOB_OBJECT_LIMIT_PROCESS_MEMORY` 限制 *commit*，而 Linux `RLIMIT_AS` 限制*地址空间*；worker 以 mmap 读入包（`sf95_worker.py:56`），文件映射在 Windows 不计 commit。§6.2 应写明等价性差异，并建议同时设 `JOB_MEMORY`（纵深）。当前只提 `PROCESS_MEMORY`。
3. **CPU rate cap 嵌套处理充分**（接受）：要求运行时探测 `in_job` 并如实上报降级，正确。
4. **LPAC 不能建子进程对架构影响**（接受）：结论正确（worker 单进程、核心多次启动）。已核实 convert 链本身不依赖 fork：`sf95_worker.py` 全程单进程、`exe_worker.py` 只被核心拉起，因此该限制不破坏现有链路。
5. **未覆盖项是否阻塞 P0（我的判断：不阻塞隔离结论，但必须登记）**：
   - **版本矩阵**：探针环境为 `CurrentBuildNumber=26300 / DisplayVersion 26H2 / ProductName "Windows 10 Pro"` 且 `runner_in_job=true`，并非标准 Win11 零售机。§8 建议冻结“仅 Win11 x64”**缺乏标准机验证**，必须作为 **P4 发布条件**补测，不能算 P0 已验证。
   - **环境块精确集合 / 注册表可读面 / worker 目录 `WRITE_DAC` / reparse·跨卷·杀软·profile 崩溃清理 / ARM64**：不构成隔离结论的阻塞，但 **§10“待实测/待调研汇总（不静默假设）”未收录这些项**，与其自身目的相悖。必须逐项进入 §10 并绑定归属阶段（这是本次证据链最明显的文档缺口）。
   - **生产 worker 目录**：探针已证明 `target\debug` 无 `WRITE_DAC`；契约只留一句提示。应冻结“worker 二进制放在用户可控/可授 DACL 的 installer 自有目录（app_data 类），而非 Program Files”，否则 P4 安装会失败。
   - **环境块**：应冻结显式 allowlist（Rust worker 实际所需）与 `ERROR_ENVVAR_NOT_FOUND` 分类，现仅“至少含…等变量”。
6. §6.2 对“LPAC 注册表可读面比预期更严”的引用忠实，但未给出“worker 需要 0 个注册表键”的冻结结论（应写明，否则 P1 无从判断）。

---

## C. 遗漏与矛盾

### C1（重要）旧 discovery/index 失效机制不成立
- `needs_refresh`（`registry.rs:598-607`）：`force || !discovery_fresh(old) || (!confirmed && old.query_key != key)`。**confirmed**（存在 `matched_basis` 命中候选）记录在 TTL 内**不因 query_key 变化刷新**。
- `discovery_fresh`（`registry.rs:512-516`）只看 `parser_version == PARSER_VERSION && query_version == QUERY_VERSION`，**不看 encoding_version**。
- `shared_index`（`registry.rs:623`）只看 `record.index.parser_version == PARSER_VERSION`；`DiscoveryIndexRecord`（`model.rs:448-454`）与 `DiscoveryIndex`（:438-446）**没有 query_key/encoding_version 字段**。
- 结论：§1.3 宣称“`query_key` 纳入该值，仅靠 key 不匹配自然过期”——**对 confirmed discovery 与 discovery-index 不成立**；这些恰是最关键的可复用缓存，会带出旧编码生成的 `candidate_id`/`source_record_id`。
- 最小修改：编码版本变化时**同时 bump `PARSER_VERSION`**（它已进入 `query_key`，且是 `discovery_fresh`/`shared_index` 的实际闸门），或把 `encoding_version` 显式纳入 `discovery_fresh` 与 `shared_index` 判定；并在 §1.3 写明该机制。

### C2 §4.1 锁顺序内部矛盾 + 重核路径未定义
- 编号把 `settings.lock` 列为顺序第 3 层，同段又注“**与 1/2 不嵌套**”。二者语义冲突。
- 同时 design §5.2 要求 apply 前重核 settings revision，而 `SourceStore::with_short_lock`（`store.rs:276-284`）明确“Never uses journal.lock”。若严格执行“不嵌套”，`validate_apply` 只能依赖进程内 `revision()`（`store.rs:240-244`）或只读原子读，跨进程改动的可见性必须写清。
- 最小修改：把规则改为“settings.lock 为独立短锁，绝不在持有 1/2 时获取；validate_apply 用启动快照/原子只读比对 revision”，并说明跨进程来源变更的可见性边界。

### C3 §7 `root_layout` 描述与代码不符
- `validate.rs:358`：非 discovery 规则**强制** `raw.root_layout == Some("game_root")`；`:354-355` 强制 discovery 规则 `root_layout.is_none()`。并非“接受但忽略/仅校验一致性/由 discovery_only 派生”。§7 的“待实测现有有效用户规则是否携带 root_layout”对 publisher 已有确定答案：**必须携带，否则当前即拒绝**。
- 风险：P1 若按“接受并忽略”实现，会**放宽**现有拒绝语义（允许缺字段的 publisher 规则）。最小修改：明确“publisher 必带 `game_root`、discovery 必须缺失”；若 P1 删字段，须给出等价的默认/派生校验。

### C4 §2.2/§2.3 授权类型交叉引用错误 + 字段集未枚举
- §2.2 称授权收据“迁为 Rust 授权类型（§5）”，但 §5 是取消/租约，非授权字段。design §5.2 列出的完整绑定（实例/路径、候选/来源、settings revision、规则/条件、产物、plan/task/request、manifest/build/语言/DLC/文件快照、期限/撤销）在契约中**从未枚举**，而 F4 又要求“授权绑定 + `validate_apply` 重核矩阵”。最小修改：修正交叉引用并指向 design §5.2；若 P0 刻意不含授权，须显式写“授权字段由 P1 冻结（以设计 §5.2 为准）”，避免 F4 无源。

### C5 契约范围 vs design §8/§10 的“锁与事件”
- design §8 P0 交付含“**锁与事件**”，§6 明确“P0 同步契约/Rust 类型/宿主白名单/共享 UI”。但本契约只冻结了 worker 协议，宿主 RPC 方法/事件命名空间/终态字段仍是“拟定”。§10 也未把“事件契约”列入待冻结。需明确：要么 P0 收录，要么显式声明延后到 P1 且以 design §6 表为准。

### C6 其它遗漏（P1 会直接踩到）
- **新请求幂等账本**：§3.1 只把旧 `install_requests.sqlite` 列为“不读取的旧目录”，未给出新幂等存储位置；design §6“崩溃不触发第二次写入”需要它。
- **成功收据 schema / `instance_hash` 算法**：`receipts/<instance_hash>/` 只出现在布局，未冻结内容与身份，而 design §5.2“同内容重装只有成功收据能证明基线”依赖它（F5 断言“同内容重装”）。
- **F3 三条历史包可得性**：deepwork 明确“现无提交的二进制 fixture”；design §9.2/P2 gate 依赖三条旧 profile 包。契约未写其来源/归属/不可得时的降级判定，P2 可能无法执行。
- **publisher ZIP 限额**：design §8（8G/16G/4G/8192/压缩比 200）在契约中缺席；若视为沿用需写明，否则 P1 无冻结依据。
- **worker 协议信封 fixture**：F3 覆盖 IR/行为语义，但 `{schema_version, action, installable, capability}` 协议信封无独立 fixture。

### C7 “无旧格式兼容”一致性
整体一致（§1.3/§3.1/§4.2/§7 均不读旧格式，保留 settings/status/游戏文件）。唯一张力是 C1（§1.3 声称的失效机制名不副实）与 C6（§3.1 丢失新幂等账本）。

---

## D. 逐条结论（接受 / 需修改 / 阻塞）

| 项 | 结论 |
| --- | --- |
| protocol_version=3、硬拒绝 | 接受（宿主同步属 P4/P5） |
| canonical 确定性编码原语 | 需修改（§1.1 入口签名与“结构体字段集”矛盾；补转义说明） |
| 旧来源缓存/索引失效语义 | **需修改（P1 前置）**：C1，须明确 bump `PARSER_VERSION` 或扩展判定 |
| selection/content ID 分离、终态回填 | 接受；补 `INSTALLER_*_VERSION` 落点 |
| `installer/rust-v1/` 布局与 journal 真相源 | 接受；补幂等账本、收据 schema |
| `open_rw/open_ro` 与锁顺序 | 需修改：C2 内部矛盾与重核路径 |
| 取消/租约/needs_recovery | 接受 |
| worker 协议 | 接受 |
| worker 限额（内存/CPU/超时） | 需修改：B1（JOB_TIME 表述）、B2（commit≠address space） |
| 规则 schema 仅 2 | 接受；§7 `root_layout` 描述需修改（C3） |
| Windows 隔离配置/支持版本 | 接受为“非标准机已证”，作为 P4 条件 |
| 未覆盖项登记（环境块/注册表/ACL/reparse/版本矩阵等） | 需修改：全部并入 §10 并绑定阶段 |
| 授权字段集 / 宿主事件契约 | 需修改：C4/C5 明确归属或显式延后 |

**是否存在阻塞**：无“必须重做原型/重跑探针”的阻塞。C1 是唯一会直接导致实现结果错误（旧缓存不失效）的冻结缺陷，因此列为 **P1 前置修改**，而非 P0 整体否决。

---

## P0 是否可放行进入 P1

**结论：条件放行（conditional pass）。**

- 允许进入 P1 的条件（均为文档级修订，无需新设备/新样本）：
  1. 修正 C1 的失效机制（bump `PARSER_VERSION` 或扩展 `discovery_fresh`/`shared_index`），并同步 §1.3 措辞；
  2. 修正 §4.1 锁顺序矛盾并写明 `validate_apply` 的 revision 重核路径（C2）；
  3. 修正 §7 `root_layout` 描述（C3）；
  4. 明确 `INSTALLER_SELECTION_VERSION/INSTALLER_CONTENT_VERSION` 落点（A）；
  5. 把探针 §7 未覆盖项 + 环境块 allowlist + 注册表键集 + worker 目录约束 + 版本矩阵并入 §10 并绑定阶段（B5）；
  6. 明确授权字段与宿主事件契约的归属（P0 收录或显式延后并指向 design §5.2/§6）（C4/C5）；
  7. 补记新幂等账本、收据 schema/F3 历史包可得性等 P1/P2 依赖（C6）。

- Windows EXE 相关放行（P4）另设门槛：标准 Win11 零售机补测版本矩阵；完成 reparse/跨卷/杀软/profile 崩溃清理用例；确认生产 worker 目录可授权。在这些完成前，“仅 Win11 x64”不得作为已验证发布声明。

- 若用户/编排希望保持 v0.3 原文冻结，则 C1 必须至少升级为**阻塞级**（因为它会使“旧缓存不产生安装许可”的既有设计目标落空）；除此之外无其它阻塞。
