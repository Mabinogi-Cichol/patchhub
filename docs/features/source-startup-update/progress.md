# 订阅源打开更新施工记录

设计见 [design.md](design.md)。

## 状态

已实现并随 v0.5.2 发布，本地相关自动化、静态检查、标签 CI 与发行附件核验通过；Windows / Steam Deck 功能实机验收待执行。本轮未部署设备。

## 2026-11-01

- 确认双端定时器当前只处理 `next_due_at` 到期的发布者源，首次未更新源没有到期时间，因此不会自动初始化。
- 确认内置发布者源未配置周期时为 0；HGamer 不支持整站更新，继续按游戏补查。
- 在 `feature/source-startup-update` 分支实施，工作区起始干净。
- 方案审阅：复用现有任务和调度路径，无需新协议、迁移、重试或持久化状态。

### 实际修改

- Rust 来源摘要：未显式保存周期的内置发布者源默认 168 小时；显式关闭 / 自选周期保持优先，自定义源默认关闭，HGamer 继续按游戏查询。
- Windows：启动时立即检查未更新源，传入 `force: true`，即使其周期为 0 也更新；正常到期任务仍传 `force: false`。运行中的同源任务不重复创建，每分钟检查不触发额外初始化重试。
- Decky：QAM 和完整管理界面打开时读取当前源与后台任务，启动启用且 `not_refreshed` 的发布者源；重新打开再次检查。完整界面的订阅轮询提升至 Manager，覆盖直接进入缓存页，并避免页签切换触发多次打开检查。
- Decky 来源读取沿用已有 `waitForAppUpdate()`，等待客户端更新检查结束后再读取；后台更新任务继续走原 RPC / 进度 / 结果投影。
- 更新设置字段的注释与设计 / 功能索引。

### 验证与结果

- Rust：`cargo test --locked -p patchhub-core --lib sources::`，114 passed / 2 ignored（原有资源测量与夹具生成项）。覆盖每周默认值、已有 0 / 24 小时设置跨重开保留、自定义源默认值、fallback 排除及首次失败后的每周到期时间。
- Flutter：`flutter test --no-pub test/source_model_test.dart test/core_task_test.dart test/global_sources_layout_test.dart test/app_lifecycle_test.dart`，35 passed。新增打开强制标志、周期关闭仍初始化、运行任务去重与后续定时检查不重复初始化的回归。
- Decky：`npm test -w apps/decky`，97 passed。新增打开 / 重开、重复订阅、禁用 / fallback / 已更新跳过、周期为 0、运行任务去重和来源读取失败的回归。
- 静态检查：`flutter analyze --no-pub`、`npm run typecheck -w apps/decky`、`cargo fmt --all -- --check`、`git diff --check` 通过。
- 验证期间修正新增 Rust 夹具缺少下载域名及一处 Dart 格式提示。Flutter / Rust 的真实本机 HTTP 测试在沙箱中受回环网络限制，已在沙箱外重跑通过；未修改网络实现以规避测试环境。
- 按 `overengineering-review` 技能完成实现审阅：覆盖打开入口 → 来源摘要 / 默认周期 → 任务选择 → 现有更新任务 / 结果读取；未发现需修改项。没有新协议、重试、硬限制、状态恢复或设置迁移。

### 待验收

- Windows 与 Steam Deck 真实首次打开、未更新源失败 / 取消后重新打开、已有用户周期展示、真实网站后台更新及更新后的游戏匹配。
- 本轮未构建发行包或部署设备；自动化结果不代表实机验收已完成。

## 2026-11-02：CLI 默认周期断言修复

- CI run [41335313400](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/41335313400/job/124933837896) 在 `list_sources_returns_defaults` 失败：实际周期为 168，旧断言仍要求 0；协议、格式和 Clippy 检查已通过。
- 在 `fix/source-startup-update` 修正 CLI 回归断言：发布者源默认 168 小时，fallback 源为 0；保留未初始化索引与无到期时间断言。未修改产品逻辑或设计。
- 过度设计复核：检查 CLI `list_sources` 默认值断言与 registry 默认周期契约，无新增抽象、限制、重试或防御分支，未发现需修改项。
- 本地 Windows 验证：`./scripts/cargo.ps1 test -p patchhub-cli --locked runtime::sources_tests::`，24 passed；`cargo fmt --all -- --check` 与 `git diff --check` 通过。Linux 在线 CI 待推送后验证。
- 按用户要求将测试修复提交为 `3bbcf70`，配套文档单独提交；未推送。

### v0.5.2 发布验证

- 已合并推送 main，修复随 v0.5.2 发布；标签 CI 双平台 core 与两端打包全部通过，四个正式附件的哈希、来源和版本核验通过，Windows 便携包隔离启动 / 正常退出通过。
- 发行提交 `3ac65f4`，详见[发行记录](../../release-checklist.md#v052-发布记录2026-11-02)。Windows / Steam Deck 功能实机验收仍待执行，本轮未部署设备。
