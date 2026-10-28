# PatchHub 合并后 CI 修复

[施工记录](progress.md)

## 2026-10-29：Windows 短路径进程识别与 CI 精简

本轮按用户明确要求直接在 `main` 修复，不创建开发分支；用户随后授权本地 Git 提交，按功能代码与文档拆分，不推送。

- push #82–#85 均在同目录多客户端更新测试中超时。原生 Windows 已复现：`GetFullPath(Target)` 展开短名，而 `Get-Process.Path` 保留启动时的短名，导致 `Assert-Idle` 漏检并进入等待调用者退出的路径。比较前对进程路径使用相同的 `GetFullPath`；新增用真实短路径启动第二客户端的回归，保留现有拒绝行为及超时值。
- Rust 生成物一致性检查只在 Core 的 Linux matrix 中执行。生成器、协议、Core 和生成物输入均由现有规则触发 Core；Windows/Decky 保留消费者契约测试，汇总 `ci` 继续覆盖 Core 失败。不为客户端检查重复编译 debug Core。
- Core、Windows、Decky 使用 `Swatinem/rust-cache@v2` 缓存 Cargo 依赖及依赖构建产物，Decky 显式映射 `. -> target/linux`；保留默认按 job、平台和 Rust 环境区分的缓存，不增加自定义失效或重试机制。
- Flutter 保留一次 `pub get --enforce-lockfile`，后续 analyze/test/build 使用 `--no-pub`；保留完整分析、测试、构建和打包校验。删除在当前 `windows-2022` 镜像中重复安装已存在 NSIS 的 Chocolatey 步骤，沿用实际 makensis 路径。
- Decky 后端测试移到 Release 二进制构建之后。通过测试专用 `PATCHHUB_TEST_CORE` 明确指定本次构建的程序；显式指定的文件缺失必须失败，本地未指定时沿用可选 debug 二进制测试。真实扫描以独立临时状态目录运行。

验收：普通 TEMP、短 TEMP 的全部 Windows 更新测试；新增回归在旧脚本失败；CI 选择器、工作流 actionlint；Flutter 完整脚本构建；Decky 在 `target/linux/release` 二进制上执行全部后端测试，真实桥接用例不得跳过。缓存命中、完整在线 CI 及耗时改善必须在推送后另行核验，不以本地验证代替。

## 2026-10-27：发行附件精简

在 `fix/release-assets` 分支调整发布工作流：公开附件仅保留 Windows 安装版、Windows 便携版、Decky ZIP 及覆盖三个安装包的 `SHA256SUMS.txt`。分平台校验 JSON 和 SBOM 继续作为 CI 产物，包内依赖清单与仓库安装脚本保持原样；删除仅为单独分发服务的安装脚本复制与 artifact 上传。

用户授权同步精简 v0.4.0：保留 Tag、三个二进制包、发布说明与发行状态，备份后移除五个冗余附件并重建校验清单。验收包括工作流暂存夹具、三包实测哈希、在线安装/自更新候选选择，以及公开附件清单读回；不重复执行与本次修改无关的构建或实机操作。

## 目标与范围

修复提交 `c0501b3bc5c1e6136a7df05a76f756ef1fba3a4e` 的 [push CI #70](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40121256739) 暴露的两处失败。分支为 `feature/patchhub-ci-fix`，从当前 main 创建。

## 问题与方案

- Decky：品牌改名删除了生产模块的 `PLUGIN_NAMES`，测试 FakeLoader 的探测与设置清理仍引用它，导致 Linux 安装测试 14 个错误。夹具改用当前唯一身份 `NAME`，不恢复旧品牌兼容层。
- Windows：更新脚本 Safe-Path 对文件调用 GetFullPath，却与未规范化的 Root 比较。Windows PowerShell 展开 8.3 短路径后，两者字符串不再匹配，合法更新被误判为越界。本地短事务路径复现与线上相同的 `Path escapes target`；普通长路径测试原本通过。线上日志没有输出具体失败路径，不能据此确认 runner 使用的短路径字符串。此逻辑在该合并前已存在，并非本次改名新增。
- 在 Safe-Path 入口先用相同 API 规范化 Root，保留托管路径、越界和重解析点检查。新增真实短路径事务回归，验证更新、用户文件保留与事务清理；卷不提供短名时明确跳过。

### 后续 push 暴露的补充问题

- `fb232c9` 的 [push run](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40346092622) 中，Decky 安装测试通过，后续 bootstrap 测试仍依赖 README 中已经删除的旧短链接，导致 8 个错误。测试统一使用当前完整 ENTRY_URL，删除旧短链接夹具、50 字符限制及已与主入口重复的 fallback 用例。
- Windows 仍有两项失败：短事务路径更新误报越界、恢复事务误报目标不一致。将整个 TEMP 改为短路径后，本地精确复现同样两项失败。初次修复只处理 Safe-Path；当短名祖先下的 stage/backup 尚不存在时，GetFullPath 对不同存在状态的路径展开结果可能不一致。应在追加这些路径前先规范化已经存在的 Work，并对恢复标记、journal 与清理标记中的路径使用相同规范化方式比较。
- 覆盖普通 TEMP、短 TEMP 全套 Windows 更新测试，并增加短路径持久化记录的恢复回归。保留路径边界检查和非托管文件保护，不新增兼容层或重试。
- changes 与 ci 均保留且在线实际执行。此次 changes 选择 Windows/Decky、跳过 Core；ci 正常汇总模块失败。无须修改 job 定义。

## 实施与验收

1. 读取具体 job 日志，分别复现 Linux 夹具错误与 Windows 短路径误报。
2. 修复夹具中的三处引用和 Safe-Path 的目录规范化。
3. Windows 更新全套、WSL 安装全套及桥接测试通过，检查差异并完成过度设计审阅。
4. 提交和推送后另行核验完整 GitHub CI；本轮本地测试不代表被失败步骤阻断的打包、构建和附件上传已在线通过。

不调整 workflow、模块选择、重试、超时或既有业务限制，不发布版本、不操作设备。
