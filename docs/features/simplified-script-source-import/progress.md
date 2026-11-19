# 单文件自描述 Lua 与面向 AI 的极简脚本导入施工记录

状态：**已实现（本地定向自动化与过度设计审阅通过，Windows / Steam Deck 实机待验收）**。设计方案见 [design.md](design.md)。当前分支：`feature/simplified-script-source-import`（基于 `main@9289242` 创建）。

## 2026-11-20

### 任务进展

- [x] **P0：分支与立项文档**
  - 基于 `main@9289242` 创建开发分支 `feature/simplified-script-source-import`（保留并整合 [docs/custom-subscription-sources.md](../../custom-subscription-sources.md) 原有修订）。
  - 创建设计文档 [design.md](design.md) 与本施工记录 [progress.md](progress.md)，并在 [功能索引](../README.md) 登记条目。
- [x] **P1：核心规则精简、Lua `meta` 解析、自动 ID 与本地名称缓存**
  - 5 个随包脚本（[patchmoe.lua](../../../crates/core/resources/source-scripts/patchmoe.lua)、[kagura-cn.lua](../../../crates/core/resources/source-scripts/kagura-cn.lua)、[kagura-en.lua](../../../crates/core/resources/source-scripts/kagura-en.lua)、[yozakura.lua](../../../crates/core/resources/source-scripts/yozakura.lua)、[hgamer.lua](../../../crates/core/resources/source-scripts/hgamer.lua)）内联静态 `meta` 表并清理多余 `rule.url` 分支；[sources.json](../../../crates/core/resources/sources.json) 瘦身为 `schema_version: 2` 注册清单；[build.rs](../../../crates/core/build.rs) 停止生成 `DISCOVERY_ADAPTERS`。
  - 精简 [model.rs](../../../crates/core/src/sources/model.rs)、[validate.rs](../../../crates/core/src/sources/validate.rs)、[builtin.rs](../../../crates/core/src/sources/builtin.rs)、[identity.rs](../../../crates/core/src/sources/identity.rs) 及安装解析模块：移除 `RootLayout`、`RawSourceRule`、`Columns`、`discovery_only`、`SourceMatch.root_layout`，引入 `ScriptMeta`、`official_title_match`、顶级显式 `SourceSummary.name`、`SourceRule` 精简结构、由 `pages` 自动推导 `page_hosts` 及按导出函数自动推断 `role`。
  - 在 [script.rs](../../../crates/core/src/sources/script.rs) 实现零网络/零 IO 沙箱 `parse_script`（配 1,000,000 条指令钩子防止顶层死循环卡死同步保存，超限返回 `script_source_init_limit_exceeded`）。
  - 在 [registry.rs](../../../crates/core/src/sources/registry.rs) 与 [store.rs](../../../crates/core/src/sources/store.rs) 实现自动 slug `id` 分配与冲突后缀、本地 `settings.names` 名称缓存与 `rule_fingerprint` 解耦、Schema `1..=3` 向 `4` 平滑迁移（保留既有自定义脚本与重命名，将原 `!discovery_only && publisher` 映射为 `official_title_match = true`，剥离已退役表格字段）。
- [x] **P2：协议契约、CLI 与 Decky 后端**
  - 更新 [crates/protocol/src/lib.rs](../../../crates/protocol/src/lib.rs) 与 [fixtures.rs](../../../crates/protocol/src/fixtures.rs)：新增只读方法 `get_script_source_prompt`（总方法数 `27`、来源方法数 `13`），精简 `import_script_source` 为 `{source_id?, api_version?, code}`，移除 `RawSourceRule`，`AddSourceResult` 增加 `name`，通过 `patchhub-contract-gen` 重生成 `packages/contracts/generated/` 并同步 [apps/windows-flutter/assets/protocol-v5.schema.json](../../../apps/windows-flutter/assets/protocol-v5.schema.json)。
  - 更新 [crates/cli/src/runtime.rs](../../../crates/cli/src/runtime.rs)、[crates/cli/src/main.rs](../../../crates/cli/src/main.rs) 及冒烟测试：接入 `get_script_source_prompt`（无 `state_dir` 亦可用）与新 `import_script_source`，移除 `--rule` 与 `replace`，补充 `invalid_source_meta` 错误前缀映射。
  - 更新 [apps/decky/main.py](../../../apps/decky/main.py)、[source_service.py](../../../apps/decky/backend/source_service.py) 及 Python 单元测试。
- [x] **P3：精简 AI 提示词与权威规范文档重构**
  - 新增 [ai-source-prompt.md](../../../crates/core/resources/ai-source-prompt.md) 并编译导出 `AI_SOURCE_PROMPT`，聚焦提问确认类型/网址、拉取 GitHub 规范文档与输出单份完整 Lua。
  - 重写 [docs/custom-subscription-sources.md](../../custom-subscription-sources.md)（整合工作区既有的 `priority`、`resolve_detail` 推进与缓存说明、`ctx.url_parts` 及调试清缓存说明；修正 `discover` 的 `query` 精确字段为 `{appid, name, name_hints, terms, force}`；清除全部已废弃规则字段说明；提供 3 个直接可单文件导入的完整 Lua 示例，并纳入 `script_source_import` 自动化解析校验）。
- [x] **P4：Windows 与 Decky 双端极简界面及卡片域名展示**
  - 更新 [script_source_editor.dart](../../../apps/windows-flutter/lib/pages/script_source_editor.dart)、[source_pages.dart](../../../apps/windows-flutter/lib/pages/source_pages.dart)、[protocol_models.dart](../../../apps/windows-flutter/lib/host/protocol_models.dart) 与 Windows 测试：新建态提供 **复制 AI 提示词** 按钮与单一 Lua 源码框，编辑/只读态展示只读摘要（来源 ID、名称、类型、`page_hosts`、`download_hosts`），来源卡片详情补充 `允许访问域名` 与 `允许下载域名`。
  - 更新 [script-source-editor.tsx](../../../apps/decky/src/script-source-editor.tsx)、[index.tsx](../../../apps/decky/src/index.tsx) 及 Decky 前端测试：与 Windows 保持一致的三态极简界面与卡片域名展示。
- [x] **P5：空内置恢复、静态检查与过度设计复核**
  - 完成核心空内置构建按单文件粘贴恢复 5 源与 schema 3 升级测试、双端原生 `patchhub-core.exe` 传输链路测试、`cargo fmt` / Clippy / 契约字节一致性核对及过度设计专项审阅，并同步更新设计文档中的最终契约细节（`AddSourceResult.name` 及编辑态只读摘要定位）。

### 验证记录

#### 已通过的本地定向自动化与静态检查

- **格式、Clippy 与契约一致性**：
  - `cargo fmt --all` 后执行 `cargo fmt --all -- --check`：通过（0 差异）。
  - `cargo clippy -p patchhub-core -p patchhub-protocol -p patchhub-cli -p patchhub-contract-gen --all-targets -- -D warnings`：通过（0 警告）。
  - `cargo run -p patchhub-contract-gen -- check`：通过，且核对 `packages/contracts/generated/v5.schema.json` 与 `apps/windows-flutter/assets/protocol-v5.schema.json` 字节完全一致（`schema byte check: ok`）。
- **核心库与集成套件（`patchhub-core`）**：
  - `cargo test -p patchhub-core --lib sources:: -- --skip real_transport`：114 passed, 0 failed, 2 ignored（跳过项为 `real_transport` 相关本地回环联网测试；223 filtered out）。
  - `cargo test -p patchhub-core --test builtin_scripts --test script_source_import`：`builtin_scripts` 6 passed（含 `empty_builtin_registry_recovers_all_five_sources_via_ordinary_import`），`script_source_import` 13 passed（含 `ai_prompt_and_documentation_examples_stay_aligned_with_core_parser`、`old_schema3_settings_upgrade_smoothly_and_legacy_scripts_keep_working`、并发自动 ID 分配、init 预算上限拦截）。
  - P1 阶段扩展验证（已随本次 Rust 核心改造完成，`cargo fmt` 前执行）：`installer::resolve` / `installer::exe_resolve` / `installer::manual_selection` 单元测试，以及 `source_data_v2`、`source_update`、`source_matching`、`source_acceptance`、`script_source_runtime`、`script_source_pipeline`、`script_source_fallback`、`browser_source_update`、`builtin_script_migration`、`builtin_script_regression`、`projection`、`installer_service`、`source_snapshot -- --skip linked_legacy` 共 15 个集成测试目标全部通过（未再对 `crates/core` 逻辑做后续修改）。
- **协议、CLI 与冒烟测试**：
  - `cargo test -p patchhub-protocol`：单测 2 passed，集成测试 `fixtures` 3 passed（0 failed）。
  - `cargo test -p patchhub-cli --bins --test projection_smoke --test script_sources_smoke`：CLI 单测 51 passed、`projection_smoke` 2 passed、`script_sources_smoke` 3 passed（0 failed）。
  - `cargo test -p patchhub-cli --test cache_smoke`（`BypassSandbox: true`，因绑定 `127.0.0.1:0`）：1 passed, 0 failed。
- **Decky 后端 Python 与前端 TypeScript**：
  - `python -m unittest discover -s apps/decky/backend -p "test_*.py" -v`（`BypassSandbox: true`，因 Windows `asyncio`/`IsolatedAsyncioTestCase` 使用隐式 `127.0.0.1` `socketpair`）：Ran 127 tests, OK (skipped=3；跳过项为 Windows 下 POSIX 进程会话/symlink 权限用例；`BridgeIntegrationTests` 真实调用已构建的 `patchhub-core.exe` 通过)。
  - `npm run typecheck`（`tsc --noEmit`）与 `npm test`（`node --experimental-strip-types --test tests/*.test.ts`）：113 passed, 0 failed。
- **Windows Flutter**：
  - `flutter analyze --no-pub --suppress-analytics`（`BypassSandbox: true`，读取本地全局 Pub 缓存）：0 issues。
  - `flutter test --no-pub --suppress-analytics`（`BypassSandbox: true`）：326 passed, 0 failed, 4 skipped（含真实连接 `patchhub-core.exe` 的 `script_source_native_test.dart` 通过；跳过项中 3 项为未传入专用 `PATCHHUB_EMPTY_CORE` / `PATCHHUB_EMPTY_WINDOWS_CORE` 空内置二进制路径的测试，另 1 项为既有生命周期跳过项）。

#### 过度设计专项审阅结果

- 按 [overengineering-review](../../../.agents/skills/overengineering-review/SKILL.md) 对本分支全部改动完成审阅，结论：**未发现需修改项**。
- 关键核查确认：保存阶段顶层沙箱的 `INIT_INSTRUCTION_LIMIT = 1_000_000` 针对同步 `import_script_source` 路径中模块顶层死循环卡死进程有明确依据，不限制脚本大小、域名数量及运行期入口执行；彻底下线旧多字段表单、`RawSourceRule`、`RootLayout`、`Columns`、`discovery_only`、`replace` 开关及双轨名称同步，无冗余兼容垫片或猜测性防御分支。

#### 未执行 / 待验收项

- **未在本地独立构建的专用测试 / 完整 CI**：
  - `PATCHHUB_EMPTY_CORE` / `PATCHHUB_EMPTY_WINDOWS_CORE` 对应的 Flutter 空内置构建专项测试（核心侧 `empty_builtin_registry_recovers_all_five_sources_via_ordinary_import` 已在 Rust 集成测试通过；Flutter 端空内置夹具构建依赖专用编译环境变量）。
  - 真实外网站点抓取与实机浏览器验证、`--lib` 全局含 `real_transport` 与 `linked_legacy` 的完整检查，以及发布前跨平台 CI 矩阵（`main`/WSL Linux/GitHub Actions）。
- **Windows 与 Steam Deck 实机待验收**：
  - Windows 桌面端打开「导入脚本来源」点击 **复制 AI 提示词** 写入系统剪贴板、粘贴 AI 生成的单文件 Lua 并保存、检查来源卡片「调度与地址详情」中的访问/下载域名、编辑替换与删除流程；真实集成测试 [script_import_acceptance_test.dart](../../../apps/windows-flutter/integration_test/script_import_acceptance_test.dart)。
  - Steam Deck 游戏模式下 Decky 插件「导入脚本来源」的复制 AI 提示词、Steam+X 软键盘/剪贴板粘贴 Lua 导入、编辑摘要及手柄焦点导航体验。
