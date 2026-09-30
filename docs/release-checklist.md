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
2. 完成 fmt、Clippy、核心事务与恢复测试、Python 桥接测试、双端前端构建；记录实际执行结果。
3. 审查依赖变更、许可及已披露漏洞；保留工具版本和可复查报告。2026-10-01 npm 官方安全接口对当前锁定依赖返回 0 条已披露漏洞。cargo-audit 0.22.2 使用 RustSec 数据库 commit 9b3a3b73a7f42606494c943e95f8196e9994df46 检查 528 个 Rust 依赖，无漏洞命中，另有下方两个提示。scripts/build-sbom.py 生成 726 个组件的 CycloneDX 依赖清单，包括开发及其他平台依赖；清单与审计均不代表依赖没有未知漏洞，也不替代许可证法律审查。
4. 重新构建核心、刷新客户端 sidecar / 目录资源，再打包；检查 ZIP 不含 .local、私钥、个人游戏库路径、备份或游戏资产。
5. 在目标平台核对包哈希并测试首次运行、签名目录、安装与卸载、下载取消、异常恢复；Decky 另测控制器导航、宿主重载和 SD 卡离线。
6. 更新 docs/progress.md 中的已完成与未完成项目，发布说明明确缺失验收及已知限制。

当前重要限制：补丁更新需先安全卸载；尚无版本迁移事务。下载取消可能留下 .partial 缓存，不能取消已开始的安装写入。文件事务依靠日志恢复，不是整个游戏目录的原子替换；对同一用户恶意改变路径与日志的防御仍有限。UI 暂无字节级进度。

RustSec 提示：RUSTSEC-2024-0429（glib 0.18.5，unsound）与 RUSTSEC-2024-0370（proc-macro-error 1.0.4，unmaintained）。cargo tree 分别按 steam-patch-windows / x86_64-pc-windows-msvc 及 steam-patch-cli / x86_64-unknown-linux-gnu 核对，均没有这些依赖路径；它们来自全平台锁文件中的其他平台 UI 依赖。当前不提供 Linux Tauri 桌面客户端。未来增加该平台前必须重新处理这些提示，未添加全局忽略。

许可证材料仍需补齐：从本地依赖包收集了 LICENSE / COPYING / NOTICE 文本，但 53 个全平台 / 开发 / 工作区包未发现顶层许可文件（不等于它们没有许可）。正式分发前需按实际发行依赖复核这些材料与图标来源署名，不能以 SBOM 中有 SPDX 声明推导全部分发条件已满足。
