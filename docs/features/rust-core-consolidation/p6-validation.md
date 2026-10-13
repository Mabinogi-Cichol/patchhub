# P6 阶段验证记录

日期：2026-10-13。状态：**P6 双端最终自动化与产物核验、Deck 正式部署及 ZIP/EXE 覆盖与写入取消回滚通过；Windows 夜勤游戏生效、Deck 两款游戏生效和实体手柄操作均由用户确认。Windows 真实界面与直链 EXE 按用户决定跳过；远端验收暂存已清理；最终独立 gate findings=[]，D1–D10 已关闭，P6 按调整后范围完成。**

关联：[设计](design.md)、[施工记录](progress.md)、[独立复核](p6-review.md)。本轮从 `d55e99d` 开始，修改尚未提交；证据保存在忽略目录 `.artifacts/rust-core-consolidation/p6/`，不随 Git 发布。

## 实际修改

- 投影与扫描基准统一使用正式 v4/完整 host 请求；移除历史 legacy 适配，加入 release/plain/counted 隔离运行及动态收据场景。体积压力测试改用 v4，证据归入 P6。
- 共享 URL helper 拒绝尾点主机名，双端相关回归通过；修正 core 与 installer 模块中已经失效的职责说明。
- Windows 便携包和 Tauri 资源补齐项目及 sfextract 许可证；双平台打包均要求 SBOM，增加缺失 SBOM 与许可证字节校验。
- 客户端指南同步 CLI/worker 构建与 v4 基准入口。没有放宽下载私网地址保护，没有调整 TUN、代理或 DNS 设置。

## 初轮已完成验证（最终修复后结果见末节）

| 检查 | 结果 | 本地证据（P6 目录） |
| --- | --- | --- |
| Windows `cargo test --locked --workspace` | 全套通过，6 个既有 ignored；包含 CLI/core/protocol/worker/Tauri | `windows-tests.log` |
| 双端 typecheck、生成契约 check、共享 UI 契约 | 通过 | `typecheck-final.log`、`generate-check.log`、`contracts-final.log` |
| Windows UI 状态/动作 | 10 状态用例、13 动作断言通过 | `windows-ui.log` |
| Decky TypeScript | 65 通过，含尾点 URL 回归 | `decky-ts-final.log` |
| Decky Python | 78 项运行：76 通过、2 平台 skip | `decky-python-normal.log` |
| 打包与 SBOM 工具测试 | 打包 7 项、SBOM 1 项通过；打包最终回归包含许可证与必需 SBOM | `package-tests-final.log`、`sbom-tests.log` |
| Windows release CLI/worker 与 NSIS | 构建完成 | `windows-release-final.log`、`windows-bundle-final.log` |
| Windows 便携包核验 | 11 文件 allowlist、三个 AMD64 PE、源码/资源字节、许可证、SBOM、SHA-256 通过 | `windows-package-verification.json` |
| Windows release 投影 | 四场景、每场景六次；隔离 plain/counted 事实一致且生产源无修改 | `windows-release-counted-projection.json` |
| Windows manifest 扫描与真实 DTO 体积 | v4、500 实例基准通过；体积记录保留 | `windows-manifest-benchmark.json`、`wire-size.json` |

投影计数：manifest 0 次，混合已安装 48 次/12 MiB，全量已安装 96 次/24 MiB，动态收据 32 次/8 MiB。计数范围是共享检测读摘要，不是所有磁盘 I/O；本机 release 结果不能直接充当另一机器历史 debug 的性能对照。

取消修复前的历史 Windows 产物（版本 0.1.0、protocol v4，已保存在 `before-cancel-fix/`；当前最终产物见末节）：

| 产物 | 字节数 | SHA-256 |
| --- | --- | --- |
| `.artifacts/steam-patch-manager-windows-portable.zip` | 10632835 | `9787fee53e60ceb8ea7cecb64cea27e44c2f38380184388cdea3f99cc311863d` |
| `target/release/bundle/nsis/Steam Patch Manager_0.1.0_x64-setup.exe` | 7647156 | `80e82fb0f26eec727ec8449a96529a02c9e512b605a4084700331bafb6eaead6` |

## 真实实例与备份

本节下方首次暂停时的事实作为历史保留；恢复后的结果见末节。

用户指定两端均使用《夜勤病栋》（3239270）与《妃神会秘史》（2303800），Deck 当前地址 `192.168.1.32`。

- Windows 两款游戏原始文件全量备份 `windows-games-before.tar`（874291200 字节），SHA-256 `d854bceda3a1f6ec539103e62528f15010b38901305131a3764c72e02c7b95be`；原始路径/逐文件摘要见 `windows-baseline.json`。
- Deck 正式插件和状态备份 `pre-cutover.tar`（44318720 字节），SHA-256 `1853c644e33b8355da350dfcbe7fe8f551f9c4e4715927cd1d1e43a7760683e2`；游戏备份 `deck-game-baseline.tar`（6210754560 字节），SHA-256 `01a02a58ec7f34bf8d95e43cdd4d2fd38d99e7b5b68eb46c7ee8dfb322a855f0`。均已拉回本机并核验，见 `deck-backup-verification.json`。
- 官方《夜勤病栋》ZIP 本机与 Deck 均已取得，4081525083 字节，SHA-256 `e646d4817f1b7bfebb01d37d9218a9ae18abca6c51c12379cf1fe3c770dfa0ef`，与签名目录一致；见 `windows-zip-verification.json`。
- Windows 核心直链下载遇到 fake-IP，写入前以 `download_private_address` 拒绝。用户允许跳过该环节；通过本地已核验官方 ZIP 执行真实实例安装，签名路线、overwrite 操作、自动清理 Disabled。任务正常结束：`disposition=completed`、`state=committed`、无 error，事务 `b7566033-0a6f-81d4-a9bf-72cdf46a4337`，见 `windows-night-install.json`。此结果不包含第二次覆盖、写入取消回滚或游戏内确认。
- Windows《妃神会秘史》EXE 未执行。真实 Windows UI、两款游戏内生效确认尚未执行。
- Deck 仅完成连接、正式 Loader/CEF/RPC/scan 读取与备份。正式插件仍为 protocol v3，未部署 P6、未安装或修改 Deck 游戏。现有任务均已终态；原扫描的旧 SD 卡路径诊断保留。

## Deck 构建残留清理

早先在 Deck 尝试 Rust 构建因缺少 C 编译器失败；rootless Podman 拉取 Ubuntu 22.04 因网络失败，没有生成镜像或容器。这些尝试不算 Linux 构建验证通过。

用户纠正为本机构建后部署，并要求删除 Deck 构建环境。已在确认绝对路径和边界后，仅清理本轮目录 `/home/deck/steam-patch-validation/rust-core-p6-20261013` 内的 `source`、`target`、`cargo-home`、`rustup-home`、`rustup-init`、`source.tar` 和两个构建脚本；相关尝试进程已停止。清理后核对没有本轮 rustup/rustc/cargo/podman/curl 进程，Podman 镜像/容器列表为空。见 `deck-build-cleanup.json` 和 `deck-pause-check.log`。

保留正式插件、正式状态、游戏、备份、官方 ZIP 与验证日志；不修改 SteamOS 系统分区。临时 CEF SSH 转发和构建代理反向转发均已关闭。Windows 安装任务已经到达 committed 终态，无正在执行的验收写入。

## 恢复入口

以下为首次暂停时的计划，现已恢复执行。

1. 用户安装好 WSL 后，从本机建立 Linux 构建环境，生成兼容 SteamOS 的 CLI/worker/Decky 包；不再在 Deck 安装构建工具。
2. 核对 `pause-state.json` 的源码摘要和现有证据；没有新增改动或未解决问题的检查沿用记录。补齐 P6 fmt/Clippy、Linux 全量自动化、release 基准与最终包/SBOM/allowlist/同包冒烟。
3. 核对目标仍为 `192.168.1.32`、备份及正式状态；按部署技能完成备份保全、部署、哈希、Loader 重载和真实 v4 RPC/UI。
4. 完成双端真实实例覆盖、写入取消回滚、终态/自动清理、UI/实体手柄与游戏生效验收；Windows 下载/EXE 当前记为环境限制待验证，沿用用户允许跳过直链的决定。
5. 最终产物与实机证据齐全后，再完成独立 gate 和 D1–D10 逐项关闭。当前未提交、推送或发布 P6。

## WSL恢复后的验证（2026-10-13）

本机WSL Ubuntu22.04、非root隔离快照，Rust1.94.0构建CLI/worker；Deck不安装构建工具。Linux内存探针仅精确允许slow_vector_initialization，保留fallible allocation；cargo基准保留代理文件名以正确分派rustup。Ubuntu自带Python3.10的async测试fixture无法正常结束，停止该测试进程后以独立Python3.12.15重跑，无游戏writer被终止。

| 检查 | 结果 | P6证据 |
| --- | --- | --- |
| Windows fmt/Clippy | 通过 | `windows-fmt.log`、`windows-clippy.log` |
| Linux workspace（不包含Windows Tauri） | 575 passed、6既有ignored | `linux-tests.log`、`linux-validation-summary.json` |
| Linux fmt/Clippy、生成check、release | 通过 | `linux-fmt.log`、`linux-clippy.log`、`linux-generate-check.log`、`linux-release.log` |
| Linux Decky Python3.12 | 78 passed、0 skip | `linux-decky-python.log` |
| Linux打包/SBOM工具 | 7/1通过 | Linux对应工具日志 |
| Linux release/count投影与500实例scan | 四场景各六次，计数/响应事实一致；生产源指纹无漂移 | `linux-release-counted-projection.json`、`linux-manifest-benchmark.json` |
| NSIS内部应用内容 | 8文件核验；主exe差异仅NSS/UNK三字节标记，其余逐字节一致；未运行安装器 | `windows-nsis-content-verification.json` |
| Deck最终包 | 24文件allowlist、ELF64 x86_64、生成契约、mode、许可证、SBOM与同源字节通过 | `deck-package-verification.json`、`deck-deploy-manifest.json` |
| Linux最终同包冒烟 | WSL非root临时解包：CoreBridge preflight/空库scan、v3拒绝/同进程后续v4合法、worker未知描述符拒绝 | `linux-package-smoke.json` |
| Deck正式部署 | Loader本地ZIP+SHA正常安装/后端重载；24文件匹配，Loader active，插件实例1，真实scan27实例、目录rev4/signed，Loader设置与基线一致 | `deck-install-result.json`、`deck-formal-verification.log`、`deck-post-install.json` |

修复前历史Deck ZIP（保存于 `before-cancel-fix/`，非当前最终包）：7119144 bytes，SHA-256 `d33c1e1c07c6e48a87a7c4fa26ff0b81c71737648d8a1766ed4ab10d5dcf3f51`。CLI/worker分别为`d6d6932075b49f71e05ae394114583b1dc900270064fd40f1e48c0a791e84b36`、`ecb769c3ab7190bf52d82c9afd2a5422315b765c9f4f720ea1e47ba88a6d87df`，已在正式SteamOS运行；本机构建要求最高GLIBC2.34，设备libc2.41。新增部署前备份`deck-pre-deploy.tar`已回传核验，SHA `f5eaf1d1fbd7cdfa6ac919545b4adaf6223cbbc9ea319c76eee008d21af7c129`；v3回滚ZIP也已核验。

Windows《夜勤病栋》第二次覆盖`completed/committed`（事务`8a6a650e-b2a1-84af-9db3-f45185f5a68a`），旧收据superseded；写入取消`cancelled/rolled_back`（事务`f97db561-498c-87cd-b71a-580b9912438d`），取消前后71个游戏文件SHA/size完全相同。证据`windows-night-overwrite.json`、`windows-night-write-cancel.json`、`windows-night-transactions-after.json`。用户已确认游戏内补丁生效，保存`windows-night-user-confirmation.json`。Windows直链/EXE沿用用户允许跳过；最新用户明确“windows真实界面验收先跳过”，标记跳过，不将UI自动用例或已打开程序当作真实界面验收通过。

新一轮GPT-6.1 Sol/high只读reviewer核验Linux/Deck最终包、源指纹、NSIS内容、Windows覆盖/取消和Deck正式RPC，findings=[]。剩余Deck游戏及控制器证据完成后再作最终收口。

## Deck夜勤ZIP实机与真实页面

设备中途地址由192.168.1.32改为用户确认的192.168.1.112，新地址严格主机校验成功，沿用原插件/游戏/备份；没有重复部署。旧IP ping正常而22端口拒绝连接，原隧道断开，记录为连接变化，不误报核心安装失败。

Deck两款实例已有历史托管补丁，本轮首轮安装操作均选择overwrite，不删除原游戏/收据来制造干净安装样本。夜勤首轮completed/committed（`f5172245-1579-8473-a76d-f660a3f3ab7a`）、再次覆盖completed/committed（`7e2b3ece-e6ae-8315-a74e-f6bb14337434`），AfterCommit清理completed、0失败。第一次取消观察错误使用writing主stage，实际宿主为applying/writing子stage，因此未取消而正常committed，证据保留`deck-night-cancel-missed.json`，不计为取消通过。修正仅验收脚本后重跑，观察到writing 65536字节后cancel_task，安全终态cancelled/rolled_back（`577b52d9-2b5a-89c0-94c5-72a52786c36c`），cleanup not_requested。没有kill writer。

终态后读取取消前/后全文件：夜勤71文件与妃神12文件SHA/size完全一致，见`deck-before-night-cancel.json`、`deck-after-night-cancel.json`、`deck-night-cancel-file-verification.json`。覆盖期间的早期哈希采样不作为最终基线；终态后已重采样。

正式HTTP bundle200、107516 bytes，SHA `1b34b4e0731aceb9f5d5cd35f0c00c6759975bb2324813c467e5182848c430f0`，与部署清单一致。实际Steam游戏模式夜勤补丁管理页顶部/底部截图已查看：已安装/托管、审核ZIP、覆盖按钮及本地ZIP入口符合投影，854×534 CSS视口，scrollHeight1072/clientHeight534，可滚到底，无横向溢出。证据`deck-http-verification.json`、`deck-night-page-final.png`、`deck-night-bottom.png`、`deck-ui-bottom.json`。这些是实机DevTools交互；实体手柄/游戏内确认仍待用户，不以截图代替。

QAM真实侧栏已打开并选择插件，顶部/底部截图均查看；有补丁筛选显示6款，两个指定游戏均显示已安装补丁，设置/重新扫描入口可见。CSS854×454，滚动容器440/1219、无横向溢出；保留原筛选与搜索值。证据`deck-qam.png`、`deck-qam-bottom.png`、`deck-qam-dom-final.json`、`deck-qam-bottom.json`。通过QAM妃神行进入实际全屏概览，来源/状态信息可见；该导航触发前端resume安全取消当时非前台RPC覆盖任务，任务download_cancelled、下载0字节、没有进入安装。此结果作为前台丢失保护实测保留，不能计为覆盖成功；随后保持页面稳定重跑。验收脚本不绕过前台失联安全策略。

妃神首轮官方中文EXE真实下载/沙箱转换/事务安装completed/committed（`13ee4687-814b-859b-a02a-b48d5bada382`，content `exe-content-6dcc185a0fd1bf8c5a96312b-2303800`），AfterCommit清理completed，回收1228413609bytes、0失败，证据`deck-cult-install.json`。再次覆盖和写入取消继续执行。

## 最终取消修复、产物与完整矩阵（2026-10-13）

独立复核发现 P2：publisher 下载阶段取消在 core 漏掉 download_cancelled 标准化，宿主显示 failed。修复 service.rs 单行错误码列表，新增公共API回归：task_cancelled/download_cancelled→cancelled、network_unavailable→download_failed，无事务或游戏文件修改。Linux旧代码运行新增测试确实失败（RED）；新代码双端通过（GREEN），没有在宿主追加特殊映射。见 linux-cancel-regression-before-fix.log 与最终取消修复测试日志。

| 最终检查 | 结果 / 证据 |
| --- | --- |
| Windows / Linux Rust | 604 / 576 passed，各6既有ignored；最终fmt、Clippy、release均通过 |
| Linux Decky Python | 78 passed、0skip |
| release投影计数 | 双端四场景各六次，计数/响应事实与当前生产源SHA一致，不跨机器宣称性能变化 |
| 最终包与同包冒烟 | Windows便携11文件、NSIS内部8文件、Deck24文件，许可证/SBOM/格式/mode/同源内容通过；Linux非root同包preflight/scan/v3拒绝→v4继续/worker非法输入拒绝通过 |
| 最终结果汇总 | final-validation-summary.json；windows/linux-release-counted-projection.json；linux-package-smoke.json |

| 当前最终产物 | 字节 | SHA-256 |
| --- | ---: | --- |
| Windows portable ZIP | 10632667 | 33e1cc52f984511a25a45c5d135f79d5c3ad0afc97553194351393ec9c81d59f |
| Windows NSIS（未运行安装器） | 7645500 | 1ea0ae90edcae3e4d30fc12d230b361f04bc969e010dadf83b0c23acf3529fde |
| Deck ZIP | 7118977 | 452fc6acc92772fb9bc997e0af93a56502e27d112eacf8c4370a4fbd431b3bdc |
| Linux CLI | 14090272 | 46ad7fb6a4b12e032c8586e50763e20b26db3be7a76f040aa7de463f9dac101f |

此修复后通过Loader本地ZIP+SHA正常部署并重载后端；正式24文件匹配，新增部署前备份deck-pre-fix.tar回传验证，SHA772becf61f6f72b7f5caf79fd82e1af8f8533a4970d4942d48c0c1f5ebc94ea0。见deck-install-fix-result.json、deck-formal-fix-verification.log。之前的成功commit/rollback使用同源事务写入逻辑，取消修复只影响生成计划前的错误码；新版又进行了真实下载取消与写入取消验证。

妃神再次覆盖 completed/committed（5d5b3bfd-570b-80c0-902d-86f7902c5b17），AfterCommit回收1228413609bytes、0失败，见deck-cult-overwrite-retry.json。新版官方EXE下载7521bytes时取消，宿主cancelled/error cancelled/result null，见deck-cult-download-cancel-fixed.json。新版写入data.xp3达到65536bytes后cancel_task，终态cancelled/rolled_back，事务7b99cea9-48f5-80d0-92eb-68f49086c8df，cleanup not_requested，见deck-cult-write-cancel.json。取消前后夜勤71与妃神12文件的SHA/size完全一致，见deck-cult-cancel-file-verification.json。

全部任务与事务终态后实际recover返回[]，手动clear_cache completed、回收1228413609bytes、failures=[]，auto_cleanup保持true，事务列表完全不变；两款83个游戏文件仍与取消前完全相同。证据deck-final-maintenance.json、deck-maintenance-file-verification.json。没有杀死writer、恢复旧state或改动其他游戏。

最终实际RPC插件实例1、scan27实例、无活动任务，Loader设置与基线相同；HTTP200/107516bytes/SHA1b34b4e0731aceb9f5d5cd35f0c00c6759975bb2324813c467e5182848c430f0匹配manifest，见deck-final-runtime.json、deck-final-http.json。真实妃神页面已查看、无横向溢出；EXE只读预览显示exe_check_required，未触发下载/写入，符合仅安装阶段执行EXE分析的边界；夜勤ZIP只读预览列出文件变化，截图已查看。见deck-cult-patches-final.png、deck-cult-preview-final.png、deck-night-preview-final.png及对应DOM记录。用户确认“两款游戏补丁生效，手柄操作正常”，见deck-user-confirmation.json。

所有远端备份与回滚包已有本地副本，最终重新核对首次游戏/插件、后续两次插件/state备份、v3回滚ZIP以及远端小文件证据归档共6项SHA成功。见deck-final-backup-verification.json。边界检查确保绝对路径、非符号链接且严格属于本轮验证目录，清理三个rust-core-p6验证目录及Downloads/rust-core-p6-20261013，远端最终证据归档也在回传核验后删除。见deck-final-cleanup.json、deck-after-cleanup-health.log。正式插件/state/两款游戏保留，Loader active，Deck未留下本轮构建环境；本地完整备份与证据保留。

验收范围例外仍明确：Windows真实界面与直链/EXE按用户指示跳过；Deck原实例已经托管，使用真实覆盖安装验证，没有删除收据制造首次干净安装。游戏内生效/实体手柄为本轮用户确认，自动交互截图单列。没有新增Git提交、推送或发布。

结束验收后恢复原QAM活动插件（null）并关闭侧菜单，见deck-restored-menu.json；临时CEF SSH转发已关闭。此前安装writer全部终态，关闭隧道没有终止游戏写入任务。

最终只读gate通过，findings=[]；D1–D10已关闭，P6按上述用户调整范围完成，详见[独立复核](p6-review.md)。文档链接及差异检查通过，未新增生产源码变动。
