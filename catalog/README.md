# 补丁目录

`catalog.json` 包含 13 个官方来源引导 AppID。`signed-catalog.json` 是维护端 Ed25519 签名封装，`trust-keys.json` 是受信公钥。未验证包 / build / 平台的来源保持 discover_only。官方夜勤病栋 ZIP 的真实双端验收正在进行，验收配方暂存于忽略的 .local 目录，尚未发布为默认自动安装条目。

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
