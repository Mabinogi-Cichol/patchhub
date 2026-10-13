# Windows FakeIP 下载兼容施工记录

[设计文档](design.md) · [调研证据与临时办法](research.md)

## 当前状态

公共 IP 限制已从共享下载传输移除。Windows release 构建、便携部署、原生窗口检查及当前 FakeIP 环境真实 ZIP/EXE 下载通过。未修改用户代理/DNS 配置，未执行游戏补丁安装；Deck 回归待验证。

## 2026-10-13

- 查明共享传输在连接前拒绝所有非公共解析地址，并显式禁用系统代理；覆盖当前动态 ZIP、EXE 与签名目录下载。
- 本机只读核验：静态系统代理启用，端点 `127.0.0.1:7897`；经批准核验系统 DNS，Kagura 下载白名单域名返回 IPv4 `198.18.0.50` 与 IPv6 `2001:2::28`，均命中拒绝规则。
- 对照既有 P4 S6 和 P6 记录，确认该问题此前登记但尚未实施兼容支持；历史关闭 TUN 后成功不等同于本次实测。
- 建立设计、调研与施工记录，并更新功能索引。提出短期定向 FakeIP 排除和正式显式本地代理方案，明确代理目标 IP 校验的信任边界变化。
- 自动化：尝试现有 `public_dns_policy_rejects_reserved_and_transition_ranges` 单测；系统 `cargo` 和仓库工具链均不可用，未跑通。文档相对链接及差异检查见本次交付核验。

## 2026-10-13：按用户决定直接使用解析结果

- 用户明确要求不检查 FakeIP、直接放行，不预设用户网络场景；取消此前 DNS 排除/代理设置方案作为实施前提。
- `crates/core/src/installer/acquire.rs` 删除公共 IP 分类函数和地址拒绝分支，所有来源统一使用系统 DNS 结果；空结果改为 `download_dns_failed`，删除旧公共地址拒绝策略单测。
- 保留 HTTPS、证书与域名验证、URL/重定向白名单、每跳解析地址绑定，以及既有文件和安装检查。未改动 `no_proxy()`。
- 同步改写 design.md、更新索引，research.md 保留旧实现与初始建议并明确标为历史调研。
- 验证：`git diff --check` 通过；共享获取模块已无 `public_ip` / `download_private_address` 引用。尝试 `scripts/cargo.ps1 test -p steam-patch-core installer::acquire::tests --locked --offline`，因系统和仓库均无可用 Cargo 未能执行测试；未构建或宣称真实下载成功。

## 当前待办与验收

- [x] 移除公共 IP 校验及旧策略测试，不新增 FakeIP 例外规则。
- [x] 可用 Rust 工具链下运行下载模块现有测试并构建 Windows 产物。
- [x] 保持现有 FakeIP 配置的 Windows 真实下载验收。
- [ ] Steam Deck 共享下载传输必要回归。

## 2026-10-13：Windows 部署与真实下载验收

- 找到既有隔离工具链 `.artifacts/cargo-isolated/{cargo,rustup}`，Rust 1.94.0 MSVC 可用；经批准联网补齐 Cargo.lock 依赖。前序“不可用”是当时 PATH 与 `.tools` 未找到工具链的结果，不代表本机没有隔离工具链。
- `installer::acquire::tests`：9 passed，覆盖 ZIP/EXE 格式、URL、重定向、缓存、哈希/大小、空间与取消。
- 签名安装集成回归：24 passed。最初在沙箱中全部报 `installer_state_busy`；在本机环境重跑全部通过，确认是沙箱文件锁限制。
- Windows 前端 TypeScript/Vite 生产构建、Rust release CLI/worker/宿主构建通过；最终宿主启用 `tauri/custom-protocol`，使用内嵌生产前端，不依赖 Vite 开发服务。
- 新增 `crates/core/tests/live_download.rs`，默认 ignored，显式传入官方 URL 和隔离证据目录；通过生产 `https_transport` 与 ZIP/EXE acquire 获取完整文件，再独立重算摘要，ZIP 额外读取目录。测试不调用安装、worker 或执行 EXE；每轮采用唯一缓存目录以避免缓存命中替代网络。
- 真实 ZIP：`Cicada_Communication_patch.zip`，304,436,240 字节、9 个 ZIP 条目；系统 DNS 为 `198.18.0.72` / `2001:2::38`，生产下载成功，独立摘要复核通过。SHA-256：`f6675d8c629e366939d381133f0d3948d2e90d04cb722fb10d3bab4f141fedbd`。
- 真实 EXE：`Summer-Memories-CN-Patch-v1.03.exe`，126,137,104 字节；系统 DNS 为 `198.18.0.50` / `2001:2::28`，生产下载成功，独立摘要复核通过。SHA-256：`21400ce2540b91a75cc56c3f90bc2f3f276b9ae3bd1e3fbb522675b0c6332130`。下载获取通过不等同于静态转换/游戏内生效验收。
- 部署目录：`.artifacts/windows-fakeip-download/portable/`；测试便携 ZIP：同根 `windows-fakeip-test.zip`（本次测试分发包，未生成新发布 SBOM/NSIS）。CLI、worker、宿主三文件部署前后 SHA-256 一致；部署目录 CLI 能力查询成功，协议 v4。
- 通过 computer-use 技能打开并检查真实 Tauri 窗口：扫描 6 个真实安装实例，显示已验证目录签名与夏日回忆来源详情。夏日回忆来源候选仍因 DLC 条件待确认而未开放安装入口，来源发现还显示时间预算耗尽/稍后重试；这些不是 `download_private_address`，本次未修改发现逻辑。没有执行 GUI 自动安装或写入游戏文件。
- `cargo fmt --all -- --check`、新增 live 测试最终编译、`git diff --check` 通过。
- 证据根目录 `.artifacts/windows-fakeip-download/`：`zip/download-evidence.json`、`exe/download-evidence.json`、`test-summary.json`、`deployed-hashes.json`、`portable-capabilities.json`。证据与下载文件不纳入 Git。
