# Alpha 发行检查

本清单区分已构建产物与已验收发行版。当前版本仍为开发 Alpha，完整开发计划的样本数量及双端 UI 门槛尚未达到。

## 构建与来源

- Rust 依赖锁定于 Cargo.lock，前端依赖锁定于 package-lock.json；安装依赖使用 npm ci。
- Windows 构建使用仓库隔离的 Rust 工具链；Linux 核心在 Ubuntu 构建，不修改 SteamOS 系统分区。
- Windows NSIS 仅安装客户端及核心。便携 ZIP 包含同一客户端、核心和目录资源；两者均不包含游戏补丁资产。
- Decky ZIP 的 bin/steam-patch 使用 Unix 0755 权限。插件不携带私钥、不镜像发行商补丁。
- scripts/package-artifacts.py 为 ZIP 生成 SHA-256 清单。此清单检测传输变化，不能替代可信发布渠道或代码签名。
- 依赖 SBOM 已通过 CycloneDX 官方仓库的 1.6 JSON Schema 验证；其锁文件哈希必须匹配当前 Cargo.lock 与 package-lock.json，否则打包拒绝使用过期清单。

## 签名方案与维护

目录使用 Ed25519 签名，校验到期、密钥撤销、release 撤销及已观察 revision 的防回退。开发私钥仅保存在忽略目录，正式发布应使用离线或受保护的独立签名密钥，通过受控客户端更新分发可信公钥。

Windows 当前未配置 Authenticode 证书，安装包没有发行者身份认证。正式发行前需配置签名与时间戳服务，并验证安装包、客户端及核心的签名；不得把目录签名称作 Windows 可执行文件签名。Decky 商店提交与成人内容相关规则仍需外部确认。

## 每次发布前

1. 核对公开目录的 AppID、来源链接、包哈希、游戏 build、语言、许可条件和平台验证证据。未知平台或版本保持禁用自动安装。
2. 完成 fmt、Clippy、核心事务与恢复测试、Python 薄桥接模块测试（生产 backend allowlist）、双端前端构建；记录实际执行结果。
3. 审查依赖变更、许可及已披露漏洞；保留工具版本和可复查报告。2026-10-01 npm 官方安全接口对当前锁定依赖返回 0 条已披露漏洞。cargo-audit 0.22.2 使用 RustSec 数据库 commit 9b3a3b73a7f42606494c943e95f8196e9994df46 检查当时的锁文件 528 个 Rust 依赖，无漏洞命中，另有下方两个提示；该记录早于后续依赖变更，当前 Cargo.lock 为 545 个包，正式发布前需重跑 cargo-audit 并更新记录。scripts/build-sbom.py 生成 744 个组件的 CycloneDX 依赖清单（2026-10-11 P6 重建实测），包括开发及其他平台依赖；清单与审计均不代表依赖没有未知漏洞，也不替代许可证法律审查。
4. 重新构建核心、刷新客户端 sidecar / 目录资源，再打包；检查 ZIP 不含 .local、私钥、个人游戏库路径、备份或游戏资产。
5. 在目标平台核对包哈希并测试首次运行、签名目录、安装与覆盖、下载取消、异常恢复；Decky 另测控制器导航、宿主重载和 SD 卡离线。
6. 更新 docs/progress.md 中的已完成与未完成项目，发布说明明确缺失验收及已知限制。

当前重要限制：不提供补丁卸载入口或同目标增量升级，覆盖安装不会删除上一版本多出的文件。从 v0.2.0 起，新 ZIP/EXE 直接安装不创建备份、暂存或授权文件，也不回滚；写入后取消、失败或中断保留已写内容，需要时重新安装。历史事务仍按其原有记录恢复，冲突报告 `needs_recovery`。下载取消可能留下 `.partial` 缓存。文件写入不构成整个游戏目录的原子替换；对同一用户恶意改变路径与日志的防御仍有限。UI 在已知总量时显示字节与文件进度，未知总量不伪造百分比。

RustSec 提示：RUSTSEC-2024-0429（glib 0.18.5，unsound）与 RUSTSEC-2024-0370（proc-macro-error 1.0.4，unmaintained）。cargo tree 分别按 steam-patch-windows / x86_64-pc-windows-msvc 及 steam-patch-cli / x86_64-unknown-linux-gnu 核对，均没有这些依赖路径；它们来自全平台锁文件中的其他平台 UI 依赖。当前不提供 Linux Tauri 桌面客户端。未来增加该平台前必须重新处理这些提示，未添加全局忽略。

许可证材料仍需补齐：从本地依赖包收集了 LICENSE / COPYING / NOTICE 文本，但 53 个全平台 / 开发 / 工作区包未发现顶层许可文件（不等于它们没有许可）。正式分发前需按实际发行依赖复核这些材料与图标来源署名，不能以 SBOM 中有 SPDX 声明推导全部分发条件已满足。

## v0.1.0 首发记录（2026-10-15）

- 发行 Tag `v0.1.0` 固定于 `a4e1056ce9ffb770672637ff893488ef8b18bfcc`，包含最新插件代码与用户版 README。GitHub Actions `38689216660` 的 Windows/Linux core、Windows、Decky 和 release 任务全部成功。
- 首个正式 Release 已公开，包含 Windows NSIS、Windows 便携 ZIP、Decky ZIP、两端校验与 SBOM，以及 `SHA256SUMS.txt`；中文说明列明功能、安装方法及已知限制。
- 从公开 Release 下载全部 8 个附件，核对 7 个内容附件的 SHA-256、两端 ZIP 校验、SBOM 锁文件摘要、11/24 文件 allowlist、ZIP CRC、PE/ELF x86_64、Unix 执行权限、runtime 哈希和 Tag 内资源/桥接源码。Windows 文本按 runner CRLF 检出规则核对；没有以 Windows SBOM 的锁文件哈希直接比较 Git 的 LF blob。
- GitHub 上传时会把 NSIS 文件名的空格转换成点号，原工作流在转换前生成 SHA256SUMS，导致名称不一致。所有附件内容哈希均正确；本轮按公开文件名重写该清单并上传，重新下载核对通过。工作流改为暂存 NSIS 时先把空格改成点号，再生成摘要，供后续版本使用；已发布 Tag 保持不变。
- 验证 JSON、下载附件、原始/修正清单及构建日志保存在忽略目录 `.artifacts/release-v0.1.0/`。本轮发布检查不替代逐游戏兼容性、全新安装或独立安全/许可证审计。
- 工作流修正的实际 Bash 暂存片段已在隔离夹具目录运行，带空格的 NSIS 名称转换为点号后，`sha256sum -c SHA256SUMS.txt` 通过。
- 用户要求补充 Decky 实机截图，随后明确由用户操作、助手仅按指令截图，最后停止追加截图。已选取游戏列表、夏日 DLC 补丁/镜像选择、人工确认及实际下载 30% 共 4 张原始 PNG，放入 `docs/images/decky/` 并链接至 README；未合成或伪造安装完成画面。
- 只读 RPC 确认用户实际执行的《夏日狂想曲+》覆盖任务 `38a5cc1d-35e9-4087-8156-16de0725582b` completed、事务 `96b611cf-f42c-880b-ae07-d23892f88aeb` committed，自动清理 completed/failures=[]，无活动任务、插件实例 1。助手没有触发本次安装，没有核验游戏内效果。设备现有插件用于截图，未重新部署 Release ZIP；这组画面不作为 Release 包的设备部署证明。
- 原计划演示《夜勤病栋》时创建的 71 文件游戏及工具状态备份（4,864,491,520 字节）已取回本地并校验 SHA-256，保留于 `.artifacts/release-v0.1.0/deck-backup/`；未修改夜勤游戏。远端仅清理本轮独立备份暂存，用户当前页面不回退。


## v0.2.0 发布记录（2026-10-19）

- 用户明确要求分项提交、合并功能分支至 main，并发布 v0.2.0；本地合并提交为 `70a1632`。应用、Cargo/npm 锁文件、Tauri 与 SBOM 应用版本均为 0.2.0。发行 Tag `v0.2.0` 为注解 Tag，对象 `8111abebc1b2de8d1934ccb620f0d0847e3a37f2`，指向提交 `4d1059143fef2af3e7e78e21f0f825eae8bd9f58`（与远端 main 一致）。
- GitHub Actions [39245207148](https://github.com/Mabinogi-Cichol/steam-hentai-patch-searcher/actions/runs/39245207148)（Tag push，head `4d10591`）的 core（Windows/Linux）、windows、decky、release 任务全部成功。Release 由该运行创建：<https://github.com/Mabinogi-Cichol/steam-hentai-patch-searcher/releases/tag/v0.2.0>，标题与 Tag 均为 `v0.2.0`，非草稿、非预发行，发布时间 2026-10-19T11:41:14Z，附件 9 个。
- 工作流以 `--generate-notes` 创建 Release，正文初始只有自动生成的 Full Changelog 链接（106 字符）。中断后本轮用[发行说明](releases/v0.2.0.md)（4,060 字符）经 GitHub API PATCH 补全中文正文；读回确认 `name == tagName == v0.2.0`、正文与仓库文件逐字符一致、`draft=false`、`prerelease=false`、附件仍为 9 个、发布时间与 Tag 未变，正文 `updated_at` 为 2026-10-19T12:05:49Z。标题只使用版本 Tag，未重建 Tag 或附件。
- 附件在线核验（2026-10-19）：从公开 Release 下载全部 9 个附件（合计 26,407,945 字节）并逐项核对，全部通过：名称、大小与 GitHub API `sha256:` digest 一致；`SHA256SUMS.txt` 8 条与下载哈希及公开文件名一致；两端 checksums JSON 的 file/size/sha256 与对应 ZIP 一致；两端 ZIP `testzip` 无 CRC 错误、无重复条目，条目 allowlist（Decky 23 / Windows 11）与 Tag 内资源、契约、文档和源码一致（Windows 按 runner CRLF 检出规则）；两份 SBOM 均为 CycloneDX 1.6、版本 0.2.0、各 749 组件、Cargo/npm 锁摘要与 Tag 匹配、附件与 ZIP 内 SBOM 字节一致；Decky `bin/steam-patch` 与 `bin/sf-worker` 为 ELF64 x86_64，runtime 哈希、大小与执行位正确；Windows 三个 exe 为 PE x86_64，NSIS 引导桩 machine 为 0x14c；`install-decky.py` 与 Tag 内 `scripts/install-decky.py` 逐字节一致。下载与证据 JSON 保存在本次会话临时目录（当时 DSH 沙箱对 `.artifacts/` 不可写），关键结果记录如下。
- 附件 SHA-256（下载实测，与 GitHub API digest 及 `SHA256SUMS.txt` 一致）：

| 附件 | 字节 | SHA-256 |
| --- | --- | --- |
| Steam.Patch.Manager_0.2.0_x64-setup.exe | 7,742,035 | `60bb1628a9cc48b32f0a3e8240ae6dfc609a1500ddf4747f7aa627bc6b0c3066` |
| steam-patch-manager-windows-portable.zip | 10,748,348 | `99d933db051098f1836ac195eef7d92da7102fbf2b1325999509cfb0b6cc1c49` |
| steam-patch-manager-decky.zip | 7,163,458 | `e62d4bff9c88aeca52a645ecdeea6e29fe79d2de68bc708405b82d962ac4edd3` |
| checksums-windows.json | 178 | `eee93c4c0822ea5d60d745c6a40c6fb3c01111f8d14ca05697f0ce58d2ba6815` |
| checksums-decky.json | 159 | `ac2290f528ef41743e25f75bff7d6509eab3719a27cb437a4bcd05d007c69766` |
| dependencies-windows.cdx.json | 361,564 | `6a00b2fcd0db3efba83bbf1d08d9f0c0b2fb8c24fe1ec141ad28b78a0513b0c7` |
| dependencies-decky.cdx.json | 347,896 | `e4148f1836a74e55529b9861b32792ec8cab5cbde1b09acf067de5ae1a39a165` |
| install-decky.py | 43,549 | `65e34c058da384431eb68be6053a783996634d7959fd32f871f0cd917b9d0b31` |
| SHA256SUMS.txt | 758 | `bd6b491133c443bfd611a62cfbb563adea0a1e92ed86f40d5d534f88e3ed3f70` |

- 重新执行 npm 审计：完整依赖报告 7 条 high，均属于同一 braces 栈耗尽公告沿 @decky/rollup → rollup-plugin-delete → del/globby/fast-glob/micromatch 的构建依赖传播；`npm audit --omit=dev` 为 0。当前 Decky 构建清理模式固定为 `./dist/*`，未把游戏/来源内容作为该模式输入。上游公告 [GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm) 尚无修复版本，本轮不声称全依赖无漏洞。证据位于 `.artifacts/release-v0.2.0/npm-audit*.json`。
- 本轮未重跑 RustSec 审计，原记录不能当作当前锁文件审计结果。完整 Windows 实机、实体手柄、来源自动调度/休眠、终端安装器实机入口以及新版直接安装的完整取消/中断矩阵仍未全部验证；发行说明保留这些边界。
- 2026-10-19 另行按用户要求，把公开 Release 的 `steam-patch-manager-decky.zip`（SHA-256 `e62d4bff9c88aeca52a645ecdeea6e29fe79d2de68bc708405b82d962ac4edd3`，7,163,458 字节，与 Tag 对应 CI 产物一致）通过 Loader v3.3.0 本地 ZIP 安装到 Devkit 当前配对设备 `steamdeck`（`deck@192.168.31.186`）：安装前旧插件完整 tar 备份、Loader 设置、外部状态基线与回滚 ZIP 已取回本地并核对哈希；安装提示 name/version/SHA/type 与包一致后才确认。部署后正式目录 23/23 文件哈希与包清单一致，`bin/steam-patch` 与 `bin/sf-worker` 为 ELF64 x86_64 且保留执行位；设备实际 HTTP bundle SHA-256 与包内 `dist/index.js` 一致（`52ea420f4e00692ac5e28cfe60218f8dcb56da6060847b7011fde9776e29eb1f`）；插件实例 1、运行版本 0.2.0、`plugin_loader` active；RPC `scan` 返回 28 个实例（18.0 秒，另有两条既存 SD 卡 `library_unavailable` 诊断）、4 个来源、无活动安装/来源任务、23 条历史事务，`cache_status` 与 `backup_status` 可读。真实游戏模式 QAM 显示 7 款有补丁游戏；来源页 4 张卡片默认收起、展开时 `aria-expanded` 与正文正确；缓存与备份页、VenusBlood FRONTIER International 游戏页（HGamer 命中与重新匹配入口）正常渲染，无横向溢出。安装导致的 Loader `pluginOrder` 变化已恢复为安装前成员关系，其他设置未改；外部状态文件清单、`journal.sqlite` 与 `requests.sqlite` 哈希均未变。实体手柄焦点/返回、来源更新与重新匹配、Windows 实机仍未验证。证据与截图：`.artifacts/deploy-release-v0.2.0-20261019-2011/`（忽略目录）。
- 随本版本附件分发的 `client-guide.md`（Windows 便携包）与 Decky 插件 README 仍保留旧版“事务回滚/每三秒续租”的表述，与本版直接安装语义不符；发行说明已明确新行为，本轮未重建已发布附件，相关文档修订随下一版本处理。


## v0.2.1 发布准备（2026-10-19）

- 用户确认显示文本解码问题已修复，并明确要求提交 Git 和发布 v0.2.1。参考基线、修复代码、既有发行文档与本轮验收分别提交；程序、Cargo/npm 锁、Tauri 与 SBOM 应用版本为 0.2.1。
- 沿用本 session 已通过的 Rust/Python/格式/clippy/Deck 样本及部署验证。版本修改仅补锁文件、npm 工作区版本一致性和 SBOM 测试。修复源码部署不代替最终 CI 发行包的部署验收。
- 正式版以 Tag 对应 CI 产物为准；标题严格为 v0.2.1，中文正文来自 [v0.2.1 发行说明](releases/v0.2.1.md)。CI 运行与附件核验将在实际完成后记录；不移动或覆盖已有 v0.2.0 Tag。


## v0.2.1 发布记录（2026-10-20）

- 参考同步 `0f27187`、显示文本解码修复 `98b9b3a`、历史发行/指南文档 `c25cc3e`、功能验收 `c549ee9`、版本更新 `424430b`、发行说明 `d2e9c59` 分别提交，并以 `cc7dbc4bfddc5a943550622dc0a1c67a1a8004ef` 合并回 main。注解 Tag v0.2.1 保持指向该提交，未改写旧 Tag。
- [Tag 对应运行 39280868529](https://github.com/Mabinogi-Cichol/steam-hentai-patch-searcher/actions/runs/39280868529) 的实际完整日志已取回：两端 core、Windows、Decky 和 release 共 5 个任务，日志均无 error annotation；release 日志确认发布 URL 与 9 个附件来自本次 Tag。正式 [Release v0.2.1](https://github.com/Mabinogi-Cichol/steam-hentai-patch-searcher/releases/tag/v0.2.1) 由 github-actions[bot] 于 2026-10-19T16:02:46Z（北京时间 10-20 00:02:46）发布。
- 状态接口异常：REST 与连接器持续返回早期 queued/null，和完整任务日志、已公开附件不一致；最初据此判断仍在排队，随后以 Release 及真实日志纠正。尝试取消时 API 返回 409（not in progress），空运行移除也被拒绝，未成功取消/删除任何运行。未伪造接口结论为 success。此前 GitHub 状态页未报告 Actions 故障，不能据此确定状态异常的根因。
- 用户在排队表象下选择本地构建。Windows 核心、worker、Tauri 与 NSIS 已成功本机构建，程序版本 0.2.1，另有完整便携包/SBOM/校验文件，保存在 `.artifacts/release-v0.2.1/release-assets/` 作为备用。设备旧编译缓存已不存在；本轮独立 Linux 源码目录创建后 SSH 中断，Linux 本地重建尚未完成。发现正式 CI 包已发布后未覆盖附件，当前公开附件保持原始 CI 构建。后续是否替换以用户选择为准；设备本轮源码目录 `/home/deck/steam-patch-validation/release-v021-local-20261020` 的清理需待连接恢复。
- 中文发行正文已由 [docs/releases/v0.2.1.md](releases/v0.2.1.md) PATCH 并逐字符读回；`name == tagName == v0.2.1`、draft=false、prerelease=false，9 个附件 ID/digest 和发布时间未变。
- 下载全部 9 个公开附件（26,022,010 字节）：公开文件名、大小、GitHub API digest、SHA256SUMS.txt 的 8 条内容附件及两端 checksums JSON 全部一致。ZIP CRC 与条目白名单通过（Decky 23 / Windows 11）；资源、文档、生成契约和许可与 Tag 匹配，Windows 按实际 LF/CRLF 检出规则核验。两份 CycloneDX SBOM 为 0.2.1、749 组件，内外 SBOM 字节一致、锁与安装器清单摘要匹配 Tag。Decky CLI/worker 为 ELF64 x86_64，runtime 配对摘要/大小/执行位正确；Windows 三个 exe 为 PE x86_64，Tauri FileVersion/ProductVersion 为 0.2.1，NSIS 引导桩为 x86。
- 校验与完整日志保存在忽略目录 `.artifacts/release-v0.2.1/`。沿用前一轮已验证源码的 Deck 部署，不宣称本次公开发行包已经重新部署或游戏矩阵全通过。Windows 实样本 ACL、三个缺失样本及既有遗留验收范围继续保留。

| 附件 | 字节 | SHA-256 |
| --- | --- | --- |
| checksums-decky.json | 159 | `c58c3f21532b8a90359cf19425c24d18d0de684dcda8e52b4482688b2241223f` |
| checksums-windows.json | 178 | `42d3c0b06abec2e949970dad7c0402c9d55d965f55c265a71f5f193a12b9c338` |
| dependencies-decky.cdx.json | 347,896 | `1289cb200c6b183b41fd2dd6c8bca38e2e088e9fde71d755add7d01c10976436` |
| dependencies-windows.cdx.json | 361,564 | `4b2e77843e59813af72dac8fb33b85db6a55dff01b38e0e2beff2bfb1d3a22da` |
| install-decky.py | 43,549 | `65e34c058da384431eb68be6053a783996634d7959fd32f871f0cd917b9d0b31` |
| SHA256SUMS.txt | 758 | `72d394523d7896fbe212491912db44e788b9a540c69014bc46b47feb380688e0` |
| steam-patch-manager-decky.zip | 7,025,959 | `2a38645332cf9f264982ef0051d7d020f09423db7c7106c6ce709a42d0685cb8` |
| steam-patch-manager-windows-portable.zip | 10,601,983 | `e86c1cbfe105b3d281beaaf43dc9b38c5a7ee30e40635986421dd594a8219951` |
| Steam.Patch.Manager_0.2.1_x64-setup.exe | 7,639,964 | `7ce0ba6b80f8f7c93e9128a12c969f5d3425fd7d86c88c4683e4541df9391d10` |

## v0.3.0 发布准备（2026-10-24）

- 用户授权将 Windows Modern UI 分支合并至 main 并发布 v0.3.0。沿用已完成的功能回归、原生截图及候选启动/退出记录；版本变更补充 Cargo 锁文件、npm 工作区和 SBOM 检查，均通过。
- Cargo workspace、Flutter、Decky、共享 UI、安装器与 SBOM 应用版本统一为 0.3.0。正式发行以 Tag 对应 CI 新构建为准，不上传本地旧版本候选。
- 发行标题严格为 v0.3.0，中文正文来自 [发行说明](releases/v0.3.0.md)。CI 与公开附件核验尚待执行；本次不执行设备部署或真实游戏补丁安装。

### 首轮 CI 阻断与修复

- v0.3.0 初始 Tag 指向 7030553；运行 39996679880 的 Decky 与 Windows 契约步骤因未声明 esbuild 失败，未生成公开 Release。干净安装还发现 React DOM 测试依赖缺失，已补齐并通过契约、类型及 Decky 68 项测试。
- 已推送 Tag 保持不变，待用户明确决定是否例外更新尚未发行的 v0.3.0 Tag，或改用新版本。当前不宣称发行完成。

- 用户于 2026-10-24 明确授权：将已验证依赖修复合并并推送 main，例外更新尚未生成 Release 的 v0.3.0 Tag，再重跑 CI 完成发布。旧 Tag 对象 b85d721d2cc5b34464f02afa3ea110dab5790505、目标提交 7030553 保留在历史记录；更新时按旧对象精确租约检查，不覆盖其他并发变更。

### 冷启动检查与并发复验

- 39998306298 的 Linux 核心和 Decky 成功；Windows SDK 首次启动日志污染机器 JSON，已用显式初始化顺序修复并验证。Windows 核心有一次并发信任库测试失败；本机精确复验通过，保留日志，不改断言或降低 CI 门槛。继续依用户授权更新未发行 Tag，正式发行仍以新 Tag 全部检查通过为前提。

### 安装器附件上传修复

- 运行 39999762120 的 Windows/Linux 核心和 Decky 全部通过，Windows 也完成 Flutter 检查、Release 构建、SBOM、便携包及 NSIS 生成；最后上传 NSIS 时通配符没有匹配到隐藏 .artifacts 目录内文件，release 被跳过。
- 根据 actions/upload-artifact 官方说明，仅对 windows-nsis 的 .artifacts/*_x64-setup.exe 路径开启 include-hidden-files，不扩大文件范围。修复只改一个工作流输入，静态核对与 diff 检查通过，待新 Tag CI 实证。此前并发信任库测试本轮已通过，保留失败历史，不宣称其根因已定位。

## v0.3.0 发布记录（2026-10-24）

- 用户授权合并至主线并发布 v0.3.0，后又明确允许更新尚未生成 Release 的同名 Tag。依赖修复、Flutter 冷初始化修复和 NSIS 附件上传修复分别提交、合并；未降低测试门槛。此前失败运行及未发行 Tag 更新历史保留在上文，所有日志已存入忽略目录。
- 最终发行提交：`48c9de68745ad59a6019ac3a165a8e7d0c03dab8`；注解 Tag 对象：`121bd2a3ea1ecf6dab0776ba0ed063edb14ce059`。公开 Release 生成后不再移动 Tag。
- [Tag 运行 40001266403](https://github.com/Mabinogi-Cichol/steam-hentai-patch/actions/runs/40001266403) 的 Windows/Linux core、Windows、Decky、release 共 5 个任务全部成功。Flutter 210 项测试、静态分析和原生 release 构建通过；此前信任库并发测试在本轮完整 CI 再次通过，未修改该测试或生产逻辑，早期单次失败不宣称已定位根因。
- [正式 Release v0.3.0](https://github.com/Mabinogi-Cichol/steam-hentai-patch/releases/tag/v0.3.0) 于 UTC 2026-10-23 18:23:52（北京时间 2026-10-24 02:23:52）发布。标题与 Tag 均为 `v0.3.0`，非草稿、非预发行，共 9 个附件；中文正文与 [发行说明](releases/v0.3.0.md)逐字符读回一致。
- 从公开 Release 下载全部 9 个附件，名称、大小、GitHub digest、SHA256SUMS 的 8 个内容文件及两端 checksums JSON 全部一致。ZIP CRC、无重复/越界条目及内容白名单通过（Windows 39 文件 / Decky 23 文件）；两端源码、目录、契约、品牌及许可与 Tag 一致，Windows 按 runner 实际 CRLF/LF 规则核对。
- 两份 CycloneDX 1.6 SBOM 的应用及全部 6 个 Rust 工作区组件版本为 0.3.0，内外 SBOM 字节一致，Cargo/npm/Dart 锁及安装器清单来源摘要匹配；Windows 505 组件、Decky 437 组件。Windows 程序与 DLL 为 PE x86_64，NSIS 引导桩为 x86；Decky CLI/worker 为 ELF64 x86_64，runtime 哈希、大小和执行位正确。独立 install-decky.py 与 Tag 源码字节一致。
- 正式 Windows 便携包在独立 APPDATA 中启动并正常关闭，FileVersion/ProductVersion 为 `0.3.0+1`，退出码 0、关闭 1001ms；未操作用户原配置或真实游戏安装，没有强制结束正常程序。证据：`.artifacts/release-v0.3.0/` 中的 release.json、tag-run.json、tag-jobs.json、tag-ci-logs.zip、verification.json、ci-validation-summary.json、portable-smoke-result.json 及 downloads/。
- 本次没有重新部署最终 Decky 包或执行真实游戏安装。系统 DPI 切换、真实自定义艺术作品及其他既有未验证边界仍按发行说明保留；不将本次构建/附件核验改写成这些实机项目通过。

| 附件 | 字节 | SHA-256 |
| --- | --- | --- |
| checksums-decky.json | 159 | `a3156bf5f7db94e95986cb9efc7972fdd622af3636cb0eccc9a1beb3dede5dfd` |
| checksums-windows.json | 178 | `237314e7040bf46f02bdb11c1268b98682234b90c767ea1077d4b8536b1e8f12` |
| dependencies-decky.cdx.json | 206617 | `f1196a39d9e876076b8b38e52a45d04a16ca85159299f4b41a9776c42c31224f` |
| dependencies-windows.cdx.json | 1698821 | `e867cef26d89120556fd9df16f5d1546751042ee7053bdd96f185e193668588d` |
| install-decky.py | 44671 | `7d89a0a5a125c4ac666642e34f97aace5df87d8565f2e8298c6b2cf0f1afc918` |
| SHA256SUMS.txt | 757 | `eb4e1a4aacee36c64942ad162a67c8cd50482b4c73753ba50bf0c17054870095` |
| steam-patch-manager-decky.zip | 7003317 | `59f3c713d3b735c54fbb8c1be936b416ddd05b3f6ac44b9bd7711c7377a76d62` |
| steam-patch-manager-windows-portable.zip | 19199825 | `6cadfe5cbd9f8bc0126ff85ce483973dd2f3e9e5e95c0519d1881787fc001604` |
| Steam.Hentai.Patch_0.3.0_x64-setup.exe | 14551300 | `57aee4da5bf891cb40af6210b60292246d5c50214a4d6f12c8ab3faee3eb2fd3` |
