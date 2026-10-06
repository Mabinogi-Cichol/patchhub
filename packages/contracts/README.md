# Alpha 数据契约

当前权威类型位于 `crates/core/src/steam.rs`、`catalog.rs` 以及 CLI 的请求类型中。字段采用 snake_case，catalog schema_version 为 1，CLI protocol_version 为 2；未知请求和目录字段拒绝，新增不兼容字段应升级版本。

核心公开 JSON-lines 调用：

```json
{"protocol_version":2,"request_id":"scan-1","method":"scan","params":{"roots":["D:/Steam"],"catalog":"catalog/catalog.json","platform":"windows"}}
```

```json
{"protocol_version":2,"request_id":"preview-1","method":"preview","params":{"roots":["D:/Steam"],"catalog":"catalog/catalog.json","platform":"windows","instance_id":"从扫描结果复制","release_id":"从匹配结果复制"}}
```

`roots` 默认为自动发现，`catalog` 默认为空开发目录，`platform` 默认按宿主选择 Windows / SteamOS。Decky 桥接固定 SteamOS 平台且不向 UI 暴露 roots / catalog 配置。

响应成功：`{protocol_version, request_id, ok:true, result}`；失败：`{protocol_version, request_id, ok:false, error:{code}}`。不能解析的输入 request_id 为 null。请求最多 64 KiB，一行一个对象；单行错误不终止后续请求。

`scan.result` 包含 roots、platform、catalog_revision、catalog_trust、installs、diagnostics；每行有 game、match_status、patches。匹配状态为 matched / no_catalog_match；补丁检测为 unknown / not_detected / installed_external / partial_or_conflict，匹配有效 committed 事务后可显示 installed_managed。

每个 patch 的 download_available 仅表示可信目录声明了可获取 Artifact；为 false 时使用官方浏览器下载与本地 ZIP 导入。managed_zip 表示有审核安装配方，不保证其站点允许后台下载。

`preview.result` 是只读提案，executable 为 false。Decky 的任务控制器内部调用 plan_install → start_install；用户界面只启动一次完整任务。计划返回短期 plan_id、operation、recipe_sha256、文件变更、空间和 blockers；preconditions 仅保留旧 journal 读取字段，新计划为空。start_install 重新核对签名目录与保存的内容身份，不接受前端任意目的路径。

方法白名单：scan、preview、install_patch、plan_install、start_install、discard_plan、fetch_artifact、list_transactions、recover、migrate_state。uninstall 与 confirmed_game_closed 已删除，协议 1 请求拒绝。Windows 调用方尚未迁移，记录为待办。

Params 包含 signed_catalog、trust_keys、state_dir、archive、plan_id、operation（install/reinstall，默认 install）和 request_id；宿主固定路径，UI 仅传实例、补丁和受限操作意图。install_patch 可在 archive 缺省时获取受信 ZIP，随后自动 plan/apply；持久 request_id 绑定实例、补丁、operation，重复请求不重复写入。Decky 获取与 EXE 静态提取由 Python 完整任务编排。

重装仅允许同一实例及审核内容身份；目标集合、载荷摘要、AppID、语言和 build 受检。已拥有目标可缺失/损坏；外部安装仅在全部审核目标完整匹配时重装。original_sha256 是本次操作前内容快照，不是卸载原版；成功提交后清理 stage/backup，旧成功收据原子标记 superseded，新失败不丢失旧 committed 收据。

取消先持久写入 jobs/<UUID>/revoked；Core 在 apply 入口、每次写入和提交前拒绝撤销授权，即使 discard RPC 失败也不能恢复执行。已提交成功优先于迟到取消。migrate_state 使用持久一次性升级标记撤销旧 planned；重复调用仅重试可信终态清理，旧未完成卸载沿用内部恢复，不暴露新卸载入口。

目录模型与 schema 位于 catalog-payload.schema.json；权威 Rust 类型在 core 的 catalog / trust / install 与 cli/runtime.rs。签名封装含 schema_version、key_id、payload（base64 原始 JSON）、signature（base64 Ed25519）。payload 含 catalog、generated_at、expires_at、revoked_release_ids。

目录记账使用 SQLite 防回退，同一写事务重新检查 revision。扫描失败时不能将 unknown 解释成未安装。语言按目录声明与审核配方选择，不能只根据游戏名称推断。
