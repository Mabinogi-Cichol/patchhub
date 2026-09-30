# 补丁目录

`catalog.json` 为尚无已审核补丁的空开发目录。调研中发现的条目不能直接成为自动安装配方；本阶段没有签名验证，也没有下载或安装命令。

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

`managed_zip` 元数据要求 recipe、verified_buildids 和 detection 均非空，但仍只用于只读检测与预览；不代表已授权安装。哈希必须为小写 64 位 SHA-256。检测与配方使用相对路径、正斜杠；禁止链接与 Windows 特殊文件名。检测文件上限 256 MiB，超过返回 unknown，不能把未读文件视为未安装。

真实来源候选与证据位于 docs/research-sources.md，P0 还需核对 AppID、取得适用样本、计算哈希、检查许可和实机测试。
