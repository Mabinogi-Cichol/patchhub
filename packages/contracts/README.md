# P0 数据契约

当前权威类型位于 `crates/core/src/steam.rs`、`catalog.rs` 以及 CLI 的请求类型中。字段采用 snake_case，schema_version / protocol_version 均为 1；未知请求和目录字段拒绝，新增不兼容字段应升级版本。

核心公开 JSON-lines 调用：

```json
{"protocol_version":1,"request_id":"scan-1","method":"scan","params":{"roots":["D:/Steam"],"catalog":"catalog/catalog.json","platform":"windows"}}
```

```json
{"protocol_version":1,"request_id":"preview-1","method":"preview","params":{"roots":["D:/Steam"],"catalog":"catalog/catalog.json","platform":"windows","instance_id":"从扫描结果复制","release_id":"从匹配结果复制"}}
```

`roots` 默认为自动发现，`catalog` 默认为空开发目录，`platform` 默认按宿主选择 Windows / SteamOS。Decky 桥接固定 SteamOS 平台且不向 UI 暴露 roots / catalog 配置。

响应成功：`{protocol_version, request_id, ok:true, result}`；失败：`{protocol_version, request_id, ok:false, error:{code}}`。不能解析的输入 request_id 为 null。请求最多 64 KiB，一行一个对象；单行错误不终止后续请求。

`scan.result` 包含 roots、platform、catalog_revision、catalog_trust、installs、diagnostics；每行有 game、match_status、patches。匹配状态为 matched / no_catalog_match；补丁检测为 unknown / not_detected / installed_external / partial_or_conflict，尚无托管记录与托管状态。

`preview.result` 包含 instance_id、release_id、catalog_revision、executable、blockers、changes。此阶段 executable 永远为 false；不提供 start_install。预览是变更提案，不是可提交的短期 plan_id。

这是开发期未签名目录结构，尚不是计划中的正式可信目录（签名、过期、撤销与事务类型下一阶段加入）。仅按 AppID 关联，platform / buildid 未验证时有 blocker；语言与 category 原样保留供未来 UI 选择，当前不自动选取一种语言。
