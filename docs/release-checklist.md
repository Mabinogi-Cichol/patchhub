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

当前重要限制：不提供卸载入口或同目标增量升级，安装/覆盖以本次开始前现场为回滚基线，且不删除上一版本多出的文件。下载取消可能留下 .partial 缓存；写入阶段取消按事务回滚，恢复冲突报告 `needs_recovery`。文件事务依靠日志恢复，不是整个游戏目录的原子替换；对同一用户恶意改变路径与日志的防御仍有限。UI 在已知总量时显示字节与文件进度，未知总量不伪造百分比。

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


## v0.2.0 发布准备（2026-10-19）

- 用户明确要求分项提交、合并功能分支至 main，并发布 v0.2.0；本地合并提交为 70a1632。应用、Cargo/npm 锁文件、Tauri 与 SBOM 应用版本均为 0.2.0。
- 沿用各功能施工记录中本 session 已完成的验证；版本调整后 SBOM 单测及 npm 工作区一致性检查通过。发布必须使用最终 Tag 对应 CI 产物，在线状态与附件核验在实际执行后补记。
- 重新执行 npm 审计：完整依赖报告 7 条 high，均属于同一 braces 栈耗尽公告沿 @decky/rollup → rollup-plugin-delete → del/globby/fast-glob/micromatch 的构建依赖传播；npm audit --omit=dev 为 0。当前 Decky 构建清理模式固定为 ./dist/*，未把游戏/来源内容作为该模式输入。上游公告 [GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm) 尚无修复版本，本轮不声称全依赖无漏洞。证据位于 .artifacts/release-v0.2.0/npm-audit*.json。
- 本轮尚未重跑 RustSec 审计；原记录不能当作当前锁文件审计结果。完整 Windows 实机、实体手柄、来源自动调度/休眠以及新版直接安装完整中断矩阵仍未全部验证。发布说明保留这些边界；未因本次发布操作设备或游戏。
- GitHub SSH 凭据不可用、HTTPS 没有保存的凭据，已启动 Git Credential Manager 设备授权，待用户完成登录后推送与发布；不将本地合并称为远端合并或已发布。
