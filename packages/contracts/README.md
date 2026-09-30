# Alpha 数据契约

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

`scan.result` 包含 roots、platform、catalog_revision、catalog_trust、installs、diagnostics；每行有 game、match_status、patches。匹配状态为 matched / no_catalog_match；补丁检测为 unknown / not_detected / installed_external / partial_or_conflict，匹配有效 committed 事务后可显示 installed_managed。

每个 patch 的 download_available 仅表示可信目录声明了可获取 Artifact；为 false 时使用官方浏览器下载与本地 ZIP 导入。managed_zip 表示有审核安装配方，不保证其站点允许后台下载。

`preview.result` 是只读提案，executable 为 false；可执行计划使用独立 plan_install 接口，返回短期 plan_id、preconditions、文件变更、空间和 blockers。start_install 重新核对签名目录与保存的配方，不接受前端任意目的路径。

生产方法：scan、preview、plan_install、start_install、fetch_artifact、list_transactions、uninstall、recover。Params 包含 signed_catalog、trust_keys、state_dir、archive、plan_id、transaction_id；宿主固定这些路径，UI 只传 ID 和关闭游戏确认。start_install / uninstall / recover 的 confirmed_game_closed 默认 false，未明确确认则拒绝。取消下载由各宿主绑定固定下载子进程，不能用于终止安装。

目录模型与 schema 位于 catalog-payload.schema.json；权威 Rust 类型在 core 的 catalog / trust / install 与 cli/runtime.rs。签名封装含 schema_version、key_id、payload（base64 原始 JSON）、signature（base64 Ed25519）。payload 含 catalog、generated_at、expires_at、revoked_release_ids。

目录记账使用 SQLite 防回退，同一写事务重新检查 revision。扫描失败时不能将 unknown 解释成未安装。语言按目录声明与审核配方选择，不能只根据游戏名称推断。
