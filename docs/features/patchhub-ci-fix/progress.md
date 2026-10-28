# PatchHub 合并后 CI 修复施工记录

[设计](design.md)

## 2026-10-29：Windows CI 再次失败与构建精简

状态：已实现，本地验证通过；完整在线 CI、缓存命中与耗时改善待验证。用户明确要求直接在 `main` 修复；基线 `48be014`，随后授权按功能代码与文档拆分本地提交，未推送。

- [push #82](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40557800912)、[#83](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40729708845)、[#84](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40746465376)、[#85](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40751223704) 均在 `test_other_client_blocks_preparation_before_current_client_exits` 触发 15 秒超时；Flutter 与打包步骤未执行。
- 原生复现：普通 TEMP 的该测试通过，真实短 TEMP 触发相同超时；诊断确认目标目录为长名、进程目录含短名，直接比较为 false。临时脚本副本规范化进程路径后，短 TEMP 的 12 项更新测试通过；这仅为上一轮调查证据，不代表仓库已修改。
- 完整成功 [#81](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40532281883) 耗时：Windows 698 秒、Decky 221 秒。Rust 契约检查 111/54 秒，Release 构建 206/132 秒；Windows 另有 Flutter 环境 89 秒、分析/测试/构建 178 秒、NSIS 准备/生成 29 秒。Flutter SDK 约 1.84 GB 缓存已命中，不能将准备耗时归为缓存失效；两平台未使用 Rust 构建缓存。
- Decky 真实桥接测试在构建前查找 `target/debug`，日志显示 `core binary not built` 并跳过；CI 实际输出为 `target/linux/release`。

### 实际修改

- `Assert-Idle` 对进程路径也执行 `GetFullPath` 后比较；复用原有拒绝断言，新增真实短路径启动另一客户端的回归，不修改 15 秒测试超时或业务等待逻辑。
- Rust 生成物检查收敛到 Core 的 Linux matrix，删除 Windows/Decky 的重复检查；补充生成器与协议夹具输入的选择器回归。各平台消费者测试及最终汇总保持覆盖。
- 三个构建 job 使用 Rust 依赖缓存，Decky 显式指定 `target/linux`；删除重复 Chocolatey NSIS 安装，Flutter 后续命令使用 `--no-pub`。
- Decky 后端测试移到 Release 构建之后，以 `PATCHHUB_TEST_CORE` 指定本次程序；显式缺失程序不再跳过，真实扫描使用临时状态目录。

### 验证与复核

- 新增短路径启动回归在旧仓库脚本上失败，精确触发 15 秒超时；修复后普通 TEMP 的 13 项更新测试全部通过（50.146 秒），真实短 TEMP 的 13 项全部通过（60.466 秒），均无跳过。
- actionlint 通过；CI 选择器 9 项全部通过，包括生成器/协议输入必选 Core 的检查。Windows 沙箱 TEMP ACL 最初阻止临时 Git 仓库操作，原生环境重跑通过；未改代码绕过沙箱限制。
- 完整 `build-windows-flutter.ps1 -SkipRust` 通过：只执行一次依赖解析，analyze 无问题，265 项测试通过、1 项跳过，Windows Release 构建成功。记录：`.artifacts/ci-flutter-validation-20261029.log`。本地 Rust 二进制复用，未重建 Windows Rust 或生成新安装包。
- WSL 当前源码 Release Core/worker 构建通过（1 分 13 秒）。初次 WSL 默认 Python 3.10.12 在取消夹具报错后卡于模拟回滚清理；最小诊断确认该版本的 `IsolatedAsyncioTestCase` 不将 setUp 的 ContextVar 带入异步用例，随后仅终止本轮该测试进程。未为本地旧 Python 增加产品兼容逻辑。
- 使用发布方 SHA-256 校验的隔离 Linux Python 3.12.16 匹配 CI 的 3.12 系列，重新执行全部 Decky 后端测试：111 项全部通过（39.536 秒），无跳过，真实 Release Core 桥接用例实际执行。记录：`.artifacts/ci-decky-linux312-validation-20261029.log`。Windows Python 3.12 的补充运行也通过（111 项中 3 项因 POSIX/Unix 身份/符号链接权限跳过），不将其等同于 Linux 验证。
- 显式设置不存在的 `PATCHHUB_TEST_CORE` 时，真实桥接用例按预期以 `FileNotFoundError` 失败且不跳过。
- 按 overengineering-review 沿更新准备/拒绝/等待路径、生成物输入选择、缓存目录映射和桥接测试失败传播复核：只统一路径表示、删除重复工作并修复真实测试覆盖；未增加重试、硬限制或状态恢复分支。`git diff --check` 通过，锁文件与生成物未漂移。

### 本地提交

- `86e8234`：Windows 短路径进程识别修复及回归。
- `fed8e9a`：Decky 后端测试顺序、Release Core 路径及状态隔离。
- `d69d8e7`：CI 构建精简、Rust 缓存与消费者构建脚本。配套设计、施工记录、功能索引和 CI 指南另作文档提交。沿用上方已完成验证，没有重复运行；本轮未推送。

### 待在线验收

- [ ] 推送后核验新 head 的完整 CI，确认 Windows 已通过原失败步骤，并完成构建、打包和上传。
- [ ] 通过后续运行确认 Rust 缓存保存/命中，并对比各阶段实际耗时；本地测试不代表缓存效果已验收。
- 本轮未发布、部署或操作 Steam Deck，未进行新安装包或真实游戏验收。

## 2026-10-27：发行附件精简

- 已移除 Release 暂存阶段的两份校验 JSON、两份外置 SBOM 和 Python 安装脚本，删除仅供外置分发的脚本复制及 CI artifact 条目。
- 保留 CI 校验 JSON/SBOM、包内依赖清单与仓库安装脚本；同步发布技能、客户端指南和发行检查要求。
- 自动化验证：直接执行工作流中的 Bash 暂存片段，夹具输出恰好 4 个附件，NSIS 空格转点号后 3 条 SHA-256 全部通过；WSL 一键安装入口 8 项、WSL 安装器 30 项、原生 Decky 自更新 8 项全部通过。
- 发布核对：v0.4.0 原 9 个附件均已备份并与当前 API 大小及 SHA-256 一致，三个安装包同时匹配原清单；精简后 API 确认为 4 个附件，公开 TXT 与三包实测哈希一致，三个安装包的 ID/大小/哈希、Tag、正文及发行状态保持不变。实际 Release 数据经 Decky 自更新候选识别和安装器下载校验通过。证据：`.artifacts/release-v0.4.0/assets-simplification-20261027/`。
- 过度设计复核：仅删除冗余分发步骤，没有新增抽象、限制、重试或兼容路径；实际运行时安装及自更新继续使用 ZIP 和 TXT。未执行设备部署或真实游戏操作。

## 状态

2026-10-26 的修复与当时待办记录保留如下；后续完整成功运行 #81 已提供构建耗时基线。2026-10-29 本轮修复与当前待验收状态以上方记录为准。

## 2026-10-26

### 线上证据

- [push run #70](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40121256739)，head 为 `c0501b3bc5c1e6136a7df05a76f756ef1fba3a4e`，结论 failure。
- changes、core (ubuntu-latest)、core (windows-latest) 成功。
- [Decky job](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40121256739/job/120953101884)：test_install_decky.py 共 30 项、14 个错误，均引用不存在的 PLUGIN_NAMES。
- [Windows job](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40121256739/job/120953101927)：test_windows_update.py 共 7 项、5 项失败，报 Path escapes target；其后构建/打包未执行。
- 汇总 ci 因模块失败而失败，release 跳过；不属于新增独立故障。

### 实际修改与验证

- 从当前 main `f92e7dc` 建立 feature/patchhub-ci-fix，保留已有 README 提交。
- scripts/test_install_decky.py：FakeLoader 三处旧名称集合引用改为当前 NAME 的成员判断/过滤。
- scripts/windows-update.ps1：Safe-Path 对 Root 先执行 GetFullPath，保证前缀比较两端一致。
- scripts/test_windows_update.py：允许传入短事务路径，新增 GetShortPathNameW 真实短路径更新回归。
- 修复前：原生 Windows 普通路径 7 项通过；新增短路径用例失败并报 Path escapes target。WSL 安装 30 项中 14 个错误，与线上相符。
- 修复后：原生 Windows 更新 8 项全部通过（短路径用例实际执行，无跳过）；WSL Linux 安装 30 项全部通过；Windows Node 桥接 9 项全部通过。
- WSL 未安装 Node，桥接测试改在本机现有 Node 执行，通过；不将其记录为 WSL 验证。
- 按 overengineering-review 沿 Safe-Path 调用与 FakeLoader 设置变更路径复核：改动仅统一路径表示和清理夹具旧身份引用，未新增兼容层、重试或硬限制，未发现需修改项。
- git diff --check 通过。

## 2026-10-26：后续 push 失败调查与补充修复

- 首轮代码提交 `7c2c465`、`2e48a3f`，文档 `391ff80`，合并提交 `fb232c9`。用户随后推送，触发 [run 40346092622](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40346092622)。
- [changes](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40346092622/job/121653274276) 成功，实际选择 `core=false, windows=true, decky=true`；改动范围与输出一致。
- [ci](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40346092622/job/121653949474) 实际创建并失败，用于汇总 Windows/Decky 失败。这两项并未删除；未操作用户浏览器，无法确认用户所见列表未显示它们的具体原因。
- [Decky](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40346092622/job/121653349683) 安装测试已通过，后续 test_decky_bootstrap.py 为 9 项中 8 个错误：仍查找已删除的旧短链接，触发 StopIteration。WSL 修复前复现一致；改用现有完整 ENTRY_URL，删除短链接专属夹具、命令长度限制及重复 fallback 用例后，8 项全部通过。
- [Windows](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40346092622/job/121653349439) 为 8 项中 2 项失败：恢复报 Transaction target mismatch，短事务路径报 Path escapes target。本机将 TEMP/TMP 指向专用临时目录的 8.3 别名后，修复前精确复现相同两项失败，说明首轮普通 TEMP 验证覆盖不足。
- 脚本补充规范化 Work 和持久化事务路径比较；增加短路径恢复记录回归。短 TEMP 与普通 TEMP 各 9 项全部通过，无跳过。
- 使用 overengineering-review 复核路径入口、恢复、清理与 README 启动测试：只统一同一路径的表示并移除过时测试契约，无新增防御机制、兼容路径或限制。workflow 未修改，沿用已成功的 changes 在线证据。
- git diff --check 通过。构建、打包与上传在本次线上仍未执行，不将本地针对性测试等同于完整 CI 通过。

## 待办

- 首轮已按要求分别提交并合并 main；本轮 Windows 补充修复提交 `119d49c`，Decky 启动入口测试提交 `386b051`，文档独立提交后合并本地 main；合并提交 ID 以 Git 历史为准。沿用本轮有效验证，无新增功能代码，不重复测试，未推送。
- [ ] 补充修复提交并推送后检查新 head 的完整在线 CI。
- [ ] 完整在线构建、打包及附件上传待验证；本轮未部署或发布。
