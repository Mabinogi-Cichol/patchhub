# CI 优化设计

[施工记录](progress.md) · [本地 CI 验证指南](../patchhub-ci-fix/local-ci-validation.md)

状态：已实现，线上 CI 待验证；实施结果见施工记录。

## 背景与证据

- push CI #124、#125 的 `decky` job 均运行满 6 小时后被取消，汇总 `ci` 因 `DECKY: cancelled` 失败。根因是 `test_app_update.py::test_failed_source_cancellation_blocks_replacement` 的手写假 `call` 签名未跟上生产代码（提交 65dc3b3 向 `bridge._call` 新增 `browser_request`），抛出的 `TypeError` 被任务吞入失败状态，测试随后 `await started.wait()` 永久等待。该用例已在 `f3b4f64` 修复。
- 近 40 次 main 运行中，除上述 2 次外有 4 次 `failure`，其中包含默认周期断言过期、品牌改名后 FakeLoader 引用已删除常量等“测试夹具过期”类问题（见 [patchhub-ci-fix](../patchhub-ci-fix/progress.md)）。
- 用户反馈：功能更新或重构后，构建出的包实测通常没有问题，CI 红灯多由测试夹具或断言过期造成，CI 成为阻断点而非问题探测器。
- 现有工作流没有任何 `timeout-minutes`，挂起测试会占用 GitHub 默认的 6 小时，期间无任何反馈。

## 目标

1. 挂起类故障在数分钟内以明确的失败位置暴露，而不是占满 6 小时。
2. 消除“测试替身与真实接口脱节”这一类过期来源：接口变化时测试立即以清晰错误失败，不静默挂起或假通过。
3. 保留能发现真实问题的检查，删除只锁定实现细节、文案或默认值而没有契约意义的断言。

## 非目标

- 不重写 `select_jobs.py` 及按变更选择 job 的机制；不改变发布流程；不增加重试、自动重跑或第二套配置。
- 不降低 Rust fmt、Clippy、协议契约、打包与 SBOM 一致性、锁文件差异等检查。
- 不一次性重写全部测试；只处理有证据的问题。

## 方案

### 1. Job 级超时（有实际依据的兜底）

依据是 #124/#125 的 6 小时挂起。按近期成功运行的实际耗时（Decky、Windows 约 10 分钟）为 `changes`、`core`、`windows`、`decky`、`release` 设置 `timeout-minutes`，取值留出冷缓存与慢 runner 余量，并在 `progress.md` 记录所依据的耗时数据。该超时只作为挂起兜底，不用于判断正常任务是否过慢；`ci` 汇总 job 无需设置。

### 2. 消除同类挂起：测试内等待必须有失败出口

对 Decky 后端测试中 `await event.wait()`、`while not flag: await asyncio.sleep(0)` 等等待“被测代码触发”的写法，增加带明确消息的 `asyncio.wait_for`（或等价的对任务异常的传播），使被测任务提前失败时测试以真实异常失败，而不是等待永不到来的事件。范围限于已存在这类等待的用例，先盘点再改，不引入通用测试框架。

### 3. 测试替身与真实接口对齐

盘点 `apps/decky/backend/test_*.py`、`scripts/test_*.py` 中手写替换生产函数/方法的假对象（如 `bridge._call = call`、`Fake*` 类）。对模拟生产接口的替身，改为 `create_autospec`/`AsyncMock(spec=...)` 或直接复用真实签名，使参数变化即时报出 `TypeError` 并指向测试。纯数据夹具、测试自身的受控进程（如 `ControlledProcess`）不动。

### 4. 脆性断言分类与清理

以过去 CI 失败记录为证据来源，将涉及的断言分为：

- 契约：协议 schema、生成物一致性、打包产物与校验和、跨进程消息格式、安全边界。保留。
- 偶然细节：具体默认值、提示文案、内部调用次数/顺序。仅在有失败记录或明显与行为无关时，改为断言行为结果或删除，同时删除仅为其服务的夹具。

不按“看着死板”批量改动，每处清理在 `progress.md` 写明失败证据或无契约意义的理由。

## CI 测试项必要性审查（2026-11-18 初审）

耗时取自成功运行 #43720562222；规模为仓库当前测试数量。判定：保留＝能发现真实问题或守护发布产物；复核＝需先取得证据再决定；建议移出＝与发布产物和运行时行为无关。

| 测试项 | 所在 job | 规模/耗时 | 判定 | 理由 |
| --- | --- | --- | --- | --- |
| `project_version.py --check` | changes | 1s | 保留 | 版本一致性，发布产物直接依赖。 |
| `test_project_version.py` | changes **与** windows | 229 行/各 1–3s | 已去掉 windows 中的重复 | 纯 Python、与平台无关，changes 已跑一次，windows 重复执行无新增信息。 |
| `.github/ci/test_select_jobs.py` | changes | 151 行/1s | 复核 | 守护选择规则，但规则一变测试就要同步改；只保留“漏选会导致构建被跳过”的核心用例，不为每条白名单写断言。 |
| `cargo test -p patchhub-protocol`、`contract-gen check`、`npm run test:contracts` | core / windows / decky | 2s 级 | 保留 | 协议契约与生成物一致性，跨 Rust/Flutter/TS 的真实边界；`test:contracts` 在 windows、decky 各跑一次属于各自消费者校验，可保留。 |
| `cargo test -p patchhub-core -p patchhub-cli` | core ×2 平台 | core 545、cli 57 项 | 保留，抽查 | 业务核心，是最主要的真实问题探测器；脆性断言按设计第 4 节抽查，不整体缩减。 |
| `test_sf95_audit.py`、`test_sf95_worker.py` | core ×2 平台 | 438 行 | **已移出 CI** | 迁移期 Python 参考实现，仅用于开发期生成 Rust 冻结夹具（`freeze_sf_rust_fixtures.py`）；Rust 测试独立读取冻结 JSON，无 Python 依赖，不进入发布包。文件保留，因夹具生成脚本引用 `test_sf95_audit`/`test_sf9_behavior` 中的辅助代码。 |
| `test_sf9_behavior.py`、`test_sf9_metadata.py` | **不在 CI** | 269 行 | 保留文件，不入 CI | 不在 CI 运行，但被夹具生成脚本引用其辅助代码，不能删除。 |
| `test_decky_debug_update.py` | **不在 CI** | 52 行 | 保留文件，不入 CI | 本地运行通过（1 项）；测试的是真机部署开发工具，不属于发布产物，不加入 CI。 |
| `test_package_artifacts.py`、`test_build_sbom.py` | windows **与** decky | 各 0–3s | 保留 | 决定发行附件内容与 SBOM，两平台的打包路径不同，各自执行合理。 |
| `test_flutter_sbom.py` | windows | 1s | 保留 | Flutter 依赖清单进入发行包。 |
| `test_windows_update.py`、`test_windows_single_instance.py` | windows | 15s + 12s | 保留 | 对应真实 PowerShell 更新脚本，曾暴露短路径缺陷（#82–#85），是少数真正探测运行时问题的脚本测试。 |
| Flutter 单测（约 100 项，位于构建脚本内，约 260s 含分析与构建） | windows | 占 job 最大头 | 保留，抽查 | 客户端行为；耗时主要来自构建而非测试，不是优化重点。 |
| `test_install_decky.py`、`test_decky_bootstrap.py`、`test-install-decky-bridge.mjs` | decky | 703/142/227 行 | 保留，抽查 | 覆盖安装脚本，曾因 FakeLoader 引用已删除常量失败，属于“夹具过期”重点抽查对象。 |
| `npm run typecheck` | decky | 秒级 | 保留 | 编译期检查，成本低、结果确定。 |
| `npm run test -w apps/decky`（约 15 个 TS 文件） | decky | — | 保留，抽查 | 前端逻辑；`session.test.ts` 502 行、`game-manager.test.ts` 439 行体量大，列入脆性断言抽查。 |
| Decky 后端 `unittest`（127 项，17 个文件） | decky | 本地 3.6s | 保留，优先改造 | 真实 core 桥接用例有价值；这里是此次挂起的来源，等待写法与手写替身是第 2、3 节的主要对象。 |

初步结论：

1. **没有发现需要整体砍掉的测试层**；core、契约、打包校验、Windows 更新脚本、Decky 桥接都对应真实发布风险。
2. **确定无必要或重复**：`sf95` 两个 Python 测试（迁移期参考实现，无运行路径）、windows 重复的 `test_project_version.py`，以及不在 CI 中的 `sf9_*`、`decky_debug_update` 孤儿测试。
3. **过期风险集中在夹具层**：Decky 后端、`test_install_decky.py`、Decky TS 大文件测试，以及 `test_select_jobs.py`。需要按设计第 4 节用失败记录逐条取证后处理，而不是按文件大小推断。
4. 这些判断来自静态审阅和耗时数据，尚未逐个用例核对 Rust 侧是否覆盖了 `sf95` 夹具，也未统计各测试的历史失败次数；这两项在施工中完成，并以结果修订本表。

## 实施拆分

1. 超时：读取近期成功运行耗时，设定并写入 `ci.yml`，更新 `.github/ci/README.md`。
2. 必要性处置：确认 Rust 已覆盖 `sf95` 夹具后移出其 CI 步骤并处理孤儿测试；删除 windows 中重复的 `test_project_version.py`；按失败记录取证修订上表。
3. 盘点：列出含等待写法与手写替身的测试清单，标注是否有失败历史。
4. 改造：按清单修改，每批以定向测试验证。
5. 回归：故意让一个被模拟接口变化，确认测试以清晰错误快速失败而非挂起（在本地临时验证，不提交）。

## 验收标准

- `ci.yml` 所有构建类 job 带有依据明确的 `timeout-minutes`，`actionlint` 通过。
- 盘点清单与处理结果写入施工记录；被改造测试在 Windows 与 Linux（WSL）本地通过。
- 回归演示：替身对应接口新增参数时，相关测试在数秒内以明确错误失败。
- 推送后对应 SHA 的线上 CI 全部成功；未执行项在施工记录中标为待验证。
- 发布前的完整本地 CI 对齐仍按 [AGENTS.md](../../../AGENTS.md) 规定执行，不在本功能中扩展。
