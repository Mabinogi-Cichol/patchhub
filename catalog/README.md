# 补丁目录

`catalog.json` revision 4 包含 13 个 AppID：12 个官方来源引导，以及 Windows / SteamOS《夜勤病栋》的审核 ZIP 配方。该配方只允许 build 16533492，已配置官方 HTTPS ZIP 直链、完整大小与 SHA-256；Decky 自动查找已有匹配 ZIP，未找到时下载并准备安装计划。Windows 与 Deck 内置 SSD 的真实安装、检测、卸载已验证，用户确认 Deck 桌面模式及 GE-Proton8-8 游戏模式运行且补丁生效。该运行结果只覆盖此设备、build 和兼容工具组合。`signed-catalog.json` 是维护端 Ed25519 签名封装，`trust-keys.json` 是受信公钥。

Decky 的补丁发现由源订阅负责，配置位于 `apps/decky/backend/sources.json`，运行时读取官网并按源规则匹配。已删除先前的静态游戏 / ZIP 索引。此目录的审核配方仍供 CLI 和历史事务使用；源查询结果在安装准备时生成本机独立签名文件计划，不写入公开目录，也不宣称逐游戏维护审核。

通用方式保持 ZIP 内相对路径、不剥离顶层目录、不执行安装程序。从官方获取后生成文件哈希与本机原文件基线，由设备用户的独立密钥签署本地配方，再通过同一 Rust 核心预检、备份、提交和卸载。每个库实例使用独立状态，私钥不进入公开信任目录或发布包；本地签名表示这次文件计划的设备授权，不等于维护端审核、发行商签名或逐游戏运行验证。非 ZIP、危险归档和需要另行运行 EXE 的补丁不保证适用。

开发条目结构示例（完全合成，AppID 42 不代表已验证游戏）：

```json
{
  "schema_version": 1,
  "revision": 1,
  "releases": [{
    "patch_id": "synthetic",
    "release_id": "synthetic-v1",
    "appid": 42,
    "name": "Synthetic metadata example",
    "version": "1",
    "language": "zh-CN",
    "category": "content_restore",
    "source_page": "https://example.invalid/patch",
    "support_level": "discover_only",
    "platforms": ["windows"],
    "verified_buildids": [],
    "detection": [],
    "recipe": null
  }]
}
```

`managed_zip` 元数据要求 recipe、verified_buildids 和 detection 均非空，；生产安装另要求可信签名、有效期、未撤销、兼容 build 和文件基线全部通过。哈希必须为小写 64 位 SHA-256。检测与配方使用相对路径、正斜杠；禁止链接与 Windows 特殊文件名。检测文件上限 4 GiB，超过返回 unknown，不能把未读文件视为未安装。

真实来源候选与证据位于 docs/research-sources.md，P0 还需核对 AppID、取得适用样本、计算哈希、检查许可和实机测试。
