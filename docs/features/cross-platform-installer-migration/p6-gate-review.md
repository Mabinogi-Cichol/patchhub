# P6 门禁独立复核报告（归档）

日期：2026-10-11（Asia/Shanghai）。评审者：独立 @oracle 会话（只读静态核对）。评审对象：P6 收口工作树（43 文件，未提交）——Rust/Decky/Windows 遗留清理、双端产物、文档同步。结论：**有条件通过**；必修 F1（feature 自有文档未同步）、F2（client-guide smoke 过度声称）；建议 F3–F6。

状态：**闭环**——F1/F2/F3/F4/F6 已在本轮处置；F5 按历史保留先例登记为已知限制。处置与证据见 [progress.md](progress.md) 2026-10-11 P6 节。

## 0. 评审方法、范围与证据边界

- 只读审阅：本 feature 文档、`crates/core/src/sources/{store,mod,session,registry,canonical}.rs`、`crates/core/src/install.rs`、`crates/cli/src/{main,runtime}.rs`、`apps/decky/**`、`apps/windows/**`、`packages/**`、`scripts/{package-artifacts,test_*}.py`、`scripts/smoke-transaction.py`、相关指南与索引。
- 未重跑测试矩阵/SBOM/产物重建；提交方证据（双端测试计数、zip/ELF sha256、SBOM 744）未由评审者独立复跑，属静态核对范围。
- 评审者未修改任何文件。

## 1. 结论

P6 代码收口主体成立：单一生产引擎、打包 allowlist、§7.3 六行清理均基本落实，删除项无运行时消费者。必修项为自有功能文档同步与 smoke 指南准确性问题；处置后 P6 判为通过。

## 2. Findings 与处置

| 项 | 级别 | 问题 | 处置 |
| --- | --- | --- | --- |
| F1 | 必修 | feature 自身 progress/design/索引仍写“P6 待执行/待实现” | 已同步：`progress.md` 2026-10-11 P6 节（含证据与待验证）、`design.md` 状态/§8、`docs/features/README.md` |
| F2 | 必修 | `docs/client-guide.md` smoke 描述与 `scripts/smoke-transaction.py` 不符（脚本为 protocol v1：`plan_install`/`start_install`/`uninstall`；v3 CLI 以 `unsupported_protocol`/`unsupported_method` 拒绝） | 已改为历史工具说明；脚本顶部加历史注记（仅 docstring）；登记“无等价 v3 冒烟替代” |
| F3 | 建议 | `Params.record_id` / `--record-id` 无读取者与调用方 | 已删除字段、解析分支与帮助文本 |
| F4 | 建议 | host-identity 禁用清单残留 `migrate_state`（已不在 v3 白名单） | 已删除该条目；保留 `unsupported_method` 断言测试 |
| F5 | 建议 | 其他历史功能文档（exe-patch-installation、generic-exe-installation、kagura-hgamer-fallback、simplified-install-flow）链接指向 P6 删除的 Python 文件 | 按历史保留先例登记为已知限制（见 progress 未执行节） |
| F6 | 建议 | 本 feature `design.md` §3/§9.1 与 `fix-plan.md` 对已删文件断链/引用 | `design.md` §3/§9.1 与 `fix-plan.md` 已加 P6 注记，保留原文 |

## 3. §7.3 六行逐项对照（评审者核实）

| 行 | 清理项 | 结论 |
| --- | --- | --- |
| 1 | Python 状态导入/导入报告 | ✅ 无 `import_*`/`ImportReport`；负向测试保留 |
| 2 | 引擎选择/engine-switch/EngineSwitch*/conflicts | ✅ 全删；`plugin_settings` 仅 `auto_cleanup`；无环境变量/回退 |
| 3 | `canonical.rs` python_json 复刻 | ✅ 已由 `canonical_bytes` 替换（边界见 A2） |
| 4 | 规则 schema 1/2 双接受 | ✅ 仅 `schema_version == 2` |
| 5 | Windows 旧核心静态降级 | ✅ 无 `ensureSourcesSupported`/`supported` 分支 |
| 6 | SourceService 安装适配/旧 release 解析 | ✅ 入口与 Rust 侧旧 API 全删（F3/F4 已处置） |

## 4. 已核实项 A2–A6

- **A2 边界**：`crates/installer-format/src/audit.rs` 的 `python_json` 属 P2 冻结 SF 行为绑定（经 installer-worker 生产调用，与冻结报告逐字节一致），不在 §7.3 第 3 行范围；已在 progress 登记，避免与“Python 哈希复刻退出生产”的宽泛表述冲突。
- **A3**：无第二生产策略；`plugin-output/backend` 仅 6 模块；无生产模块导入被删模块；无引擎选择/回退/检查点。
- **A4**：Decky allowlist 完整且被测试覆盖（`package-artifacts.py` 精确集合校验 + `test_package_artifacts.py` 负向用例）。
- **A5**：删除项（`migrate_state`/`Store::open`、`EngineSwitch*`、`source_for_release`/`validate_download`、`SourcePolicy`/`ResolvedSource`、`list_game_sources`/`game_subscribed`、`ENGINE`/`engine`、`SourcePanel.supported`）均无运行时消费者；未见过度删除；旧用户升级能力不依赖该新引擎死码。
- **A6**：`release-checklist` 545 与 `Cargo.lock` 实测一致；744 组件/覆盖语义/写入取消回滚表述与提供证据相符。

## 5. 必须登记的待验证项

1. Deck 正式重部署未重做（P5 运行时已验收；P6 变更不改变部署包行为）——保持待验证/已登记。
2. `smoke-transaction.py` 无 v3 等价替代——已在指南与 progress 登记。
3. 跨功能历史文档断链保持现状——已登记为已知限制。
4. 提交方证据未由评审者独立复跑——维持施工记录口径（静态核对范围）。
5. P2 三条旧 profile 真实包回归——样本不在仓库，仍待验证。
6. 既有登记项不变：更早 Win11 版本矩阵、S6（TUN/fake-IP）、kagura 来源质量专项。

## 6. 放行说明

必修 F1/F2 与建议 F3/F4/F6 处置后，P6 判为**通过**；F5 与第 5 节各项以登记方式随施工记录保留。
