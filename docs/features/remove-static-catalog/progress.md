# 施工记录

设计见 [design.md](design.md)，实现审阅见 [review.md](review.md)。

## 状态

### 2026-11-03：CI #98 后续修复

- [CI #98](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/41620527586) 在 Linux Core 的 Clippy 阶段失败：`cache_smoke.rs:213` 未处理 `Read::read` 返回长度，触发 `clippy::unused_io_amount`。Windows Core 被 matrix fail-fast 取消；Windows 打包与 Decky 成功。
- 从已包含目录移除改动的 `main` 创建独立修复分支 `fix/cache-smoke-request-read`。本机 HTTP 测试服务改用已有 BufReader 按行读取至请求头空行；头部未完成即 EOF 时断言失败，不再假设一次 TCP read 能读完请求。
- 过度设计审阅：沿接受连接、读取头部、返回 ZIP 路径复核，仅修正测试 HTTP 边界；无新增依赖、重试、超时、硬限制或生产逻辑修改。
- 本地验证通过：CI 同参数四包 `cargo clippy --all-targets --locked -- -D warnings`、`cargo fmt --all -- --check`、`cargo test -p patchhub-cli --test cache_smoke --locked` 以及 `git diff --check`。日志在 `.artifacts/ci-98/`。
- 按 [本地 CI 验证](../../local-ci-validation.md) 补齐全 job 覆盖：`crates/cli/tests/cache_smoke.rs` 命中 `crates/**`，选择器返回 core/windows/decky 三个 job 全为 true。Windows core、WSL Linux core、Windows 打包入口与 Decky Linux 全部按 workflow 逐步骤执行，结果见下表。本轮属于排查 CI #98 失败并建立流程；此后完整对齐只在发布版本前执行，日常开发按改动范围做定向验证。
- WSL 首轮以 root 运行 core 测试时，CLI 7 项失败（`installer_host_requires_non_root` 抢先于 `state_overlaps_game`、能力与备份断言返回 false、清理用例 unwrap 失败）。GitHub runner 为非 root，改以本机非 root 用户 `codex-p6` 重跑后全部通过；这是验证环境差异，不构成代码修复，未放宽生产 root 拒绝逻辑。
- `/mnt/e`（drvfs）上以非 root 构建时 `mlua-sys` 构建脚本复制头文件失败（Operation not permitted），core 与 Decky 均改在 ext4 工作副本执行并各自使用独立 `CARGO_TARGET_DIR`，与 Windows 编译产物不共享。副本来源与提交/工作区状态记入各副本 `.ci-sync-stamp`。

| job / platform | step / 完整命令 | 结果 | 日志或产物 | 未验证原因 |
| --- | --- | --- | --- | --- |
| changes | `python scripts/project_version.py --check`、`test_project_version.py`（12 项）、`.github/ci` 单测（9 项）、`select_jobs(paths)` | 通过 | `.artifacts/ci-98/windows-core.log` | — |
| core / Windows | `cargo test --locked -p patchhub-protocol` | 通过 | `.artifacts/ci-98/windows-core.log` | — |
| core / Windows | `cargo fmt --all -- --check` | 通过 | 同上 | — |
| core / Windows | `cargo clippy -p patchhub-core -p patchhub-cli -p patchhub-protocol -p patchhub-contract-gen --all-targets --locked -- -D warnings` | 通过 | 同上 | — |
| core / Windows | `cargo test -p patchhub-core -p patchhub-cli --locked`、`cargo build -p patchhub-core -p patchhub-cli --locked` | 通过 | 同上 | — |
| core / Windows | `test_sf95_audit.py`（18 项）、`test_sf95_worker.py`（5 项） | 通过 | 同上 | — |
| core / WSL Linux | `cargo run --locked -p patchhub-contract-gen -- check` | 通过（Linux 专属步骤） | `~/wsl-ci-logs/core-contract-gen-check.log` | — |
| core / WSL Linux | `cargo test --locked -p patchhub-protocol`、`cargo fmt --all -- --check` | 通过 | `core-protocol-test.log`、`core-fmt-check.log` | — |
| core / WSL Linux | 四包 `--all-targets` `-D warnings` Clippy | 通过，0 warning | `core-clippy-4pkgs.log` | — |
| core / WSL Linux | `cargo test -p patchhub-core -p patchhub-cli --locked` | 通过（381 通过/6 忽略 与 CLI 50 通过） | `core-core-cli-test.log` | — |
| core / WSL Linux | `cargo build -p patchhub-core -p patchhub-cli --locked`、`test_sf95_audit.py`、`test_sf95_worker.py` | 通过 | `core-core-cli-build.log`、`core-sf95-*.log` | — |
| windows | 六组 Python 单测、`npm ci`、`npm run test:contracts` | 通过 | `.artifacts/ci-98/windows-job.log` | — |
| windows | `cargo build --release --locked -p patchhub-cli -p patchhub-installer-worker --bin patchhub-core --bin patchhub-worker` | 通过 | 同上 | — |
| windows | `./scripts/build-windows-flutter.ps1 -SkipRust`、`cargo metadata`、含 Flutter 依赖的 SBOM、`package-artifacts.py --platform windows` | 通过 | `.artifacts/patchhub-windows-portable.zip`（44 项，0 个 catalog/trust-keys 资源，含 core/worker） | — |
| windows | `./scripts/build-windows-installer.ps1 -Makensis …` | 未执行 | — | 本机无 NSIS `makensis`，留待线上 CI |
| decky / WSL Linux | 四组 Python 单测（含 `test_install_decky.py`、`test_decky_bootstrap.py`）、`npm ci`、`npm run test:contracts`、`npm run typecheck` | 通过 | `~/wsl-ci-logs/decky-*.log` | — |
| decky / WSL Linux | `npm run test -w apps/decky`（107/107）、`node --test scripts/test-install-decky-bridge.mjs`（11/11） | 通过 | `decky-decky-frontend-tests.log`、`decky-bridge-tests.log` | — |
| decky / WSL Linux | Linux `cargo build --release … --bin patchhub-core --bin patchhub-worker`、`PATCHHUB_TEST_CORE=<本轮 release core>` 后端测试 124 项 | 通过 | `decky-linux-release-build.log`、`decky-decky-backend-tests.log` | — |
| decky / WSL Linux | `npm run package:plugin -w apps/decky`、`cargo metadata`、`build-sbom.py`、`package-artifacts.py --platform decky` | 通过 | `patchhub-decky.zip`（27 项，bin 仅 core/worker/runtime.json，无 catalog 资源） | — |
| 生成物一致性 | `git diff --exit-code -- Cargo.lock package-lock.json … packages/contracts/generated packages/contracts/fixtures …` | 通过（exit 0） | `.artifacts/ci-98/windows-job.log` | Decky 副本无 `.git`，该步在 Windows 仓库对同一同步源码执行 |
| core / windows 环境 | Node 版本 | 记录偏差 | — | 本机 Node 24.14.0，CI 用 22；Rust 1.94.0 与 workflow 一致 |

- 待验证：本修复提交后的在线 CI（含 Linux/Windows matrix、NSIS 安装包与线上 artifact 上传）、Steam Deck 实机。未提交、推送、部署或发布。

已实现。2026-11-03 本地 Rust、Flutter、Decky 前后端及双端打包资源检查通过；最终 Windows 发行包重建、资源/二进制/schema 核对，以及从真实便携包提取核心后的干净应用全链路实测均通过。Linux ELF 发行构建、Steam Deck 实机、真实来源网站/游戏内效果及原生 UI 交互待验证。本轮按代码与文档分开创建本地提交；未推送、合并或发布。

## 2026-11-03：调查与实现

- 任务开始时实际为干净 `main`；用户明确授权后切至 `fix/remove-static-catalog`。未改动 `crates/core/tests/script_source_import.rs` 或 `docs/features/patchhub-ci-fix`，未暂存任何文件。
- 核实 `3ac65f44ebfa2176716a645608c803689605cd78` 的 signed-catalog payload：generated_at=1790974154（北京时间 2026-10-03 04:49:14），expires_at=1793566154（2026-11-02 04:49:14）。本任务不重签、不续期，也不把它与独立的并发导入测试故障合并处理。
- 删除 `catalog/` 公开目录、公钥及说明、公开目录 schema、CLI 签名维护命令、静态验签安装与 trust revision 模块，以及只验证旧路线的测试。保留本地安装计划和历史事务所需数据结构、原有反序列化字段及恢复执行代码。
- Windows、Decky、CLI 的 host 统一为 roots/state_dir/platform。预览只接收动态 selection，安装接收 selection 或既有 manual_download；release/archive 及静态 local_archive action 删除。结果和历史事务的 release_id 保留为身份。
- 核心投影只读取 Steam、来源快照和本地事务；不再返回 catalog_error/catalog_trust/catalog_revision。删除 Flutter 目录不可用提示对应的失效输入路径。能力契约同步为 projection=3、actions=2、outcome=1、request_validation=3；生成 Rust/TS/Python/Flutter 共享 schema 与夹具，明确拒绝旧目录字段。
- 删除两端打包静态资源要求。Windows 增量构建会清理三个旧目录资源与旧文件选择器 DLL；Decky 原有重新暂存流程不会复制它们。移除只服务旧静态 ZIP 路线的文件选择器 UI、依赖、锁定的 12 个相关包及原生插件注册。
- 原静态签名投影基准和签名 ZIP 原生夹具生成器退役；当前指南、共享协议及受影响的历史链接同步处理，历史实测事实保留。
- 依据 overengineering-review 技能完成计划与实际执行路径审阅：未增加宽限期、空目录兜底、双读、状态迁移、重试或硬限制。详见 [review.md](review.md)。

## 验证

日志和摘要位于忽略目录 `.artifacts/remove-static-catalog/`。测试均使用隔离临时状态和真实当前系统日期，未修改用户游戏。

| 检查 | 结果 |
| --- | --- |
| Rust `cargo test --workspace` | 681 通过、10 个原有忽略；包含干净应用→扫描 Steam 清单→执行来源 Lua→更新索引→动态匹配/选择→本机 HTTP 下载→安装→幂等回放，以及历史无 selection 的 committed plan 投影 |
| Decky Python 后端 | 最终 124 项，2 项跳过（开发者模式开启后符号链接测试可执行）；实际核心桥接、参数注入、取消、回执和恢复相关回归通过 |
| Decky TypeScript | 类型检查、107 项测试、Rollup 构建通过 |
| 共享契约 | 生成一致性、跨语言请求/响应夹具、Flutter presentation 夹具一致性通过；旧 host/release/archive 输入拒绝得到回归覆盖 |
| Flutter | analyze 无问题；292 通过、1 项原有跳过；Windows Release 构建通过。最终契约清理后的同入口重建亦通过 |
| 双端真实打包入口夹具 | 12 项版本/入口测试通过；Windows 无 catalog 源目录且预放旧资源，构建脚本清除旧文件；Decky 无 catalog 源目录仍正常暂存，bin 仅 core/worker/runtime.json |
| 发行包与安装脚本检查 | package-artifacts 11 项通过；Decky 安装脚本 32 项中 25 项平台相关跳过；CI 路由 9 项通过 |
| Windows 实际产物 | 核心/worker release 编译、便携包构建通过；43 个文件，无 catalog.json/signed-catalog.json/trust-keys.json/文件选择器 DLL，ZIP 中核心/worker 与本轮 release 二进制逐字节一致，内嵌 schema 与生成物一致。最终包已重建并再次完成同样核对 |
| Windows 真实便携包全链路 | 从最终 ZIP 取 core/worker 到空应用目录；以当前日期在临时 Steam 库完成扫描→来源 Lua 更新/查询→动态候选匹配/选择→本机 HTTP 下载→真实文件安装→同请求幂等回放；HTTP 请求数为 1，未使用公开目录。证据 `.artifacts/remove-static-catalog/package-smoke-result.json` |

### 环境信号与处理

- 沙箱内 Windows 文件锁导致临时来源测试统一 `source_state_busy`；同一测试在正常权限下通过。后续真实锁/子进程测试使用正常权限运行，没有修改生产锁策略。
- 批量源码改写与提权测试合并执行曾被自动审批拒绝；改为逐文件可审阅补丁、源码修改与验证分离后完成，没有覆盖主会话工作。
- 最终包附加实测一度因自动审批模型容量不足/审阅超时未执行；受限沙箱中已通过扫描、脚本查询与动态匹配，但本机 HTTP 被网络边界阻止。用户要求继续等待重试，审批恢复后正常权限执行同一夹具成功，未改变生产逻辑或绕过网络边界。
- 用户追加要求安装环境后，从 Flutter 官方 Git `3.47.7` 标签 `abaf9c523780a608bd46686fd5e53740a07077f8` 安装到 `.tools/flutter`。官方归档清单/下载地址返回 404，因此使用已核实的官方标签初始化。最终工具识别 stable / Flutter 3.47.7 / Dart 3.13.5。
- 用户明确同意后通过 UAC 开启 Windows 开发者模式；用户 PATH 加入 `.tools/flutter/bin`。Windows C++、CMake、Windows SDK 探测及实际编译通过。Android 环境不属于本任务范围。

## 未验证与交付边界

- 没有 Linux 发行 ELF 工具链/产物，本轮只验证 Decky 前后端、前端构建和真实打包入口/资源夹具，不声称完整 Linux 发行 ZIP 已生成。
- 未操作 Steam Deck，未执行真实发行商下载、真实游戏安装或游戏内生效测试，未进行原生 UI 交互/视觉验收。
- 已按用户要求分开本地提交代码与文档；代码提交为 `e75ca87`（`refactor(catalog): 移除公开静态补丁目录及客户端依赖`）。未推送、发布或合并，提交保留在 `fix/remove-static-catalog` 分支。
