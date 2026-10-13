# Windows FakeIP 下载阻断调研

日期：2026-10-13。见 [设计](design.md) 与 [施工记录](progress.md)。以下保留最初调研时的代码行为与方案评估；随后用户决定取消 IP 地址分类检查，当前方案以 design.md 为准。调研阶段没有修改程序或代理配置，也没有执行补丁安装。

## 当前证据

### 代码

- [共享下载传输](../../../crates/core/src/installer/acquire.rs)：`ReqwestTransport::get` 在连接前通过 `ToSocketAddrs` 获取地址，任一地址未通过 `public_ip` 即返回 `download_private_address`；随后才创建带 `no_proxy()` 与 `resolve_to_addrs()` 的客户端。
- 同文件的 `public_ip` 拒绝 IPv4 `198.18/198.19`；IPv6 `2001` 的第二段小于 `0x200` 也拒绝，所以本机 `2001:2::…` 同样被拒绝。现有 `public_dns_policy_rejects_reserved_and_transition_ranges` 测试包含 `198.18.1.1`。
- [CLI 安装入口](../../../crates/cli/src/runtime.rs) 创建 `installer::https_transport()` 并传入安装服务；[ZIP 编排](../../../crates/core/src/installer/orchestrator.rs)、[EXE 编排](../../../crates/core/src/installer/exe_orchestrator.rs)、[签名下载](../../../crates/core/src/installer/signed.rs) 均使用该传输。
- ZIP 编排的 `map_download_error` 会把这个原因包装为 `download_failed` 的 detail；[共享 UI](../../../packages/ui/src/index.ts) 尚无 `download_private_address` / `download_dns_failed` 的专用中文解释。
- [来源发现客户端](../../../crates/core/src/sources/http.rs) 没有同一套公共 IP 预检和 `no_proxy()`，网络行为与下载不同。默认代理是否选中仍取决于运行环境，不能把发现成功视为下载已验证。
- 仓库还保留 [旧下载模块](../../../crates/core/src/download.rs)，其客户端行为不同；本次搜索没有找到 `download::fetch` 调用。不能仅查看该文件就判断当前安装流程支持系统代理。

### Windows 本机只读核验

读取 `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings` 的静态字段：`ProxyEnable=1`，`ProxyServer=127.0.0.1:7897`。未读取完整代理配置或认证信息；未探测端口可用性。

经批准在沙箱外调用 `[System.Net.Dns]::GetHostAddresses`，结果如下：

| 域名 | IPv4 | IPv6 |
| --- | --- | --- |
| kaguraserver.com | 198.18.0.50 | 2001:2::28 |
| patch.moe | 198.18.0.51 | 2001:2::29 |
| www.kaguragames.com | 198.18.0.52 | 2001:2::2a |

`kaguraserver.com` 是当前内置 Kagura 下载白名单域名，可直接支持下载链路冲突结论。`patch.moe` 与官网解析作为环境旁证，不代表某个具体下载 URL 已实测。沙箱内的首次 DNS 查询失败不作为真实网络结论。

证据链：系统 DNS 返回保留地址 → 共享下载公共地址检查拒绝 → HTTP 请求尚未发送。现有 `.no_proxy()` 还意味着系统代理/环境代理不会用于这条下载链路；它不代表 Windows TUN 一定无法接管流量，但前置拒绝让流量根本没有机会发出。

### 既有仓库记录

- [Windows P4 记录 S6](../cross-platform-installer-migration/p4-windows.md)：此前已记录 Mihomo TUN/FakeIP 导致 `download_private_address`，临时关闭 TUN 后验收通过，代理兼容支持待决策。
- [P4 复核](../cross-platform-installer-migration/p4-gate-review.md) 与 [P6 验证](../rust-core-consolidation/p6-validation.md) 有相同阻断记录。

这些是历史验收证据，本次没有重新关闭 TUN 或执行安装。

## 立即可用的配置办法

推荐优先只让下载域名返回真实地址，保留现有流量规则。Mihomo 支持 `fake-ip-filter` 排除域名，以及 `redir-host` 模式；具体语义依当前 `fake-ip-filter-mode` 决定。[官方 DNS 文档](https://wiki.metacubex.one/config/dns/)

下面适用于 **blacklist 模式**，应合并进现有列表，不能覆盖原有条目。下载域名取自 [内置来源配置](../../../crates/core/resources/sources.json)：

```yaml
dns:
  fake-ip-filter-mode: blacklist
  fake-ip-filter:
    - '+.kaguraserver.com'
    - '+.kaguragamespatch.com'
    - '+.download.patch.moe'
```

若原配置是 rule 模式，应在宽泛 FakeIP 规则前添加对应 `DOMAIN-SUFFIX,…,real-ip`；若是 whitelist 模式，应按其语义调整，不能直接粘贴 blacklist 片段。不要为了本次兼容覆盖其他 DNS 策略。

配置生效后，按代理客户端方式清理其 DNS/FakeIP 缓存，再在 Windows 运行 `ipconfig /flushdns` 清理系统缓存。重新解析实际下载及重定向域名，确认 A/AAAA 都是公共地址。当前实现只要混入一个非公共地址就会拒绝，单独关闭 IPv6 无法解决 IPv4 FakeIP。

真实解析只解决地址预检；仍需确认 TUN 或其他路由能够到达补丁服务器。若只启用 Windows 系统代理而无可用 TUN/直连，当前下载器仍不会使用该代理。

更宽泛的备选是将 DNS `enhanced-mode` 改为 `redir-host`。这影响所有域名，优先采用定向排除。临时关闭 TUN 是历史上验证过的绕过方式，但会改变其他应用的路由，且部分海外站点可能仍需代理。

浏览器下载后导入本地 ZIP 也可绕过程序网络获取；仅使用客户端已有的本地导入类型，不假定支持手动导入 EXE。

## 程序解决办法评估

| 方案 | 优点 | 代价与结论 |
| --- | --- | --- |
| 下载域名 FakeIP 排除 | 无程序改动，保留当前直连公共地址检查 | 依赖用户代理配置及真实 IP 的路由；适合作为短期办法 |
| 显式本地 HTTP/mixed 代理 | HTTPS CONNECT 使用域名，避免本地 FakeIP 预解析；符合系统代理使用习惯 | 需用户明确选择信任代理；代理目标公共 IP 无法由客户端证明；推荐作为正式支持 |
| 自建真实 DNS/DoH 路径 | 可保留公共 IP 筛选与连接绑定 | 引导解析、额外服务依赖和路由兼容成本高；留作备选 |
| 直接允许 FakeIP 网段 | 修改量小 | 无法证明 TUN 存在，放宽保留地址边界；不推荐 |
| 只删除 `no_proxy()` | 修改量小 | 更早的 DNS 校验仍然拒绝；不构成完整修复 |

Reqwest 文档明确说明 `no_proxy()` 会禁用自动系统代理；代理应显式配置，CONNECT 的实际域名传递需实现测试确认。[官方 ClientBuilder 文档](https://docs.rs/reqwest/latest/reqwest/blocking/struct.ClientBuilder.html)

## 验证局限

本次确认当前 DNS 与拒绝逻辑一致；没有运行完整下载或修改网络设置。尝试执行现有公共地址单测，但环境没有可用 `cargo`，仓库 `scripts/cargo.ps1` 也未找到隔离工具链，测试未执行成功。方案中的代理、DNS 排除、ZIP/EXE 实装和 Deck 回归均待验证。
