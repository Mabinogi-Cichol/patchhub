# 订阅源缓存与抓取模型统一重构施工记录

状态：**已实现（本地定向自动化与过度设计审阅通过；受沙箱隔离限制的回环/软硬链接测试、跨端构建与实机待验收）**。设计方案见 [design.md](design.md)。当前分支：`feature/unified-source-index-detail`（基于 `feature/script-source-adapters@c0e94d1` 创建）。

## 2026-11-17

### 任务进展

- [x] **P1：核心数据模型与存储缓存层重构**
  - [mod.rs](../../../crates/core/src/sources/mod.rs)：将 `SOURCE_INDEX_SCHEMA_VERSION` 与 `SOURCE_DETAIL_SCHEMA_VERSION` 升级为 `2`。
  - [model.rs](../../../crates/core/src/sources/model.rs)：移除 `FactLink`、`PageFact`、`DiscoveryPage` 以及 `SourceIndexEntry` 上的 `kind`/`url`/`language`/`date` 扁平字段；新增统一的纯业务详情结构 `SourceDetail`，并将 `SourceIndexEntry` 与 `DetailRecord` 统一改为 `current_step`/`total_steps`/`step_data`/`details`（及 `is_complete()`）。
  - [index.rs](../../../crates/core/src/sources/index.rs)：适配新版 `SourceIndexEntry` 与 `SourceDetail` 的归一化、同页条目去重边界及 v1→v2 迁移投影。
  - [store.rs](../../../crates/core/src/sources/store.rs)：新增 `read_versioned_json`；对 `schema_version < current` 的旧版索引/进度/详情缓存返回 `Ok(None)`（标记为需重刷），对缺失或高于当前版本的 schema 显式报错（`invalid_source_index` / `invalid_source_detail`）。
  - [test_fixtures.rs](../../../crates/core/src/sources/test_fixtures.rs)：同步 `SOURCE_INDEX_SCHEMA_VERSION`。
- [x] **P2：Lua 执行器与候选生成统一**
  - [script.rs](../../../crates/core/src/sources/adapters/script.rs)、[adapters/mod.rs](../../../crates/core/src/sources/adapters/mod.rs)：移除 `resolve_page` 与 `DiscoveryPage`，新增 `DetailStepState`、`DetailStepOutcome`、`validate_detail` 与 `resolve_detail(ctx, rule, state)` 契约；`discover` 改为消费 `details: Vec<SourceDetail>`。
  - [identity.rs](../../../crates/core/src/sources/identity.rs)：新增 `build_candidates_from_detail` / `build_candidates_from_detail_with_options` 并导出 `iso8601_utc`，统一承载 `published_at`、`reported_format`、`languages`、`upstream_page`、`single_download_group` 及基于 `!rule.discovery_only || !query.related_dlc.is_empty()` 的 `MatchBasis::OfficialTitle` 提升策略。
  - [matching.rs](../../../crates/core/src/sources/matching.rs)：移除 `select_entries`、`match_entry_rows`、`match_page_entries`、`process_category`、`resolve_fact`、`save_inline_fact`、`candidates_from_fact` 等双轨/拓扑逻辑；新增离线 `match_completed_entries`（含单镜像同 URL 按 `zh-CN` 优先去重）及按需步进循环 `match_incomplete_entries` / `resolve_entry_details`（支持从 `DetailRecord` 续跑、每步落盘、步停滞诊断退出、以及跨条目复用子文章 `DetailRecord` 缓存）。
  - [registry.rs](../../../crates/core/src/sources/registry.rs)：删除 `source_match_from_entry`、`candidate_from_index_match` 及重复 `iso8601_utc`/`civil_from_days`，将 `index_matches`、`index_candidates` 与内部测试统一收敛至 `matching::match_completed_entries`。
- [x] **P3：内置 Lua 脚本迁移、示例与测试同步**
  - 5 份内置单文件 Lua：[patchmoe.lua](../../../crates/core/resources/source-scripts/patchmoe.lua)（`1/1` 同步产满 `details` 并按同 URL 合并多语标题与别名）、[kagura-en.lua](../../../crates/core/resources/source-scripts/kagura-en.lua)（`1/2 -> 2/2` 移除无用表格与拓扑字段）、[kagura-cn.lua](../../../crates/core/resources/source-scripts/kagura-cn.lua)（文章 `1/2 -> 2/2`；分类动态 `1/2 -> 2/2` 或 `1/2 -> 2/3 -> 3/3` 并支持增量容错续传）、[yozakura.lua](../../../crates/core/resources/source-scripts/yozakura.lua)（`1/2 -> 2/2` 且保持 `single_download_group = true`）、[hgamer.lua](../../../crates/core/resources/source-scripts/hgamer.lua)（`discover` 直接产出 `details`）。
  - 配套夹具、示例与测试同步：库内单测（[script/tests.rs](../../../crates/core/src/sources/adapters/script/tests.rs)、[orchestrator.rs](../../../crates/core/src/installer/orchestrator.rs)、[resolve.rs](../../../crates/core/src/installer/resolve.rs)、[manual_selection.rs](../../../crates/core/src/installer/manual_selection.rs)）、15 个 `patchhub-core` 集成测试文件及夹具（含 [builtin_scripts.rs](../../../crates/core/tests/builtin_scripts.rs) 新增的 `empty_builtin_registry_recovers_all_five_sources_via_ordinary_import`、 [source_data_v2.rs](../../../crates/core/tests/source_data_v2.rs) 新增的 schema 1 过期缓存测试）、示例（[docs/features/script-source-adapters/examples/README.md](../script-source-adapters/examples/README.md) 及两份示例 Lua）以及 CLI 测试（[runtime.rs](../../../crates/cli/src/runtime.rs)、[cache_smoke.rs](../../../crates/cli/tests/cache_smoke.rs)）。
- [x] **P4：实现后过度设计审阅与代码收敛**
  - 对 `crates/core/src/sources/` 差异对照 `.agents/skills/overengineering-review/SKILL.md` 复核：
    - 抽离 [identity.rs](../../../crates/core/src/sources/identity.rs) 中 `collect_match_targets` 与 `candidate_matches_targets` 消除重复标题/DLC 归一化循环，并收口空分组语言回退逻辑；
    - 删除 [registry.rs](../../../crates/core/src/sources/registry.rs) 重复日期转换与 [matching.rs](../../../crates/core/src/sources/matching.rs) 未使用辅助函数 `diagnostic`，并在 `resolve_entry_details` 内合并重复 `store.save_detail` 分支；
    - 确认 `resolve_entry_details` 仅校验 `outcome.current_step > state.current_step`（无诊断停滞报错由 `script.rs` 保证，有诊断则落盘诊断并退出循环），未增加无依据的硬性步数上限，也未吞掉失败。

### 验证记录（Windows 工作区 `.tools/rustup/.../1.94.0-x86_64-pc-windows-msvc`）

以下命令均在最后一次 Rust/Lua 代码修改完成后于沙箱内执行：

1. **格式与 Clippy**：
   - `cargo fmt --all -- --check`：通过（0 差异）。
   - `cargo clippy -p patchhub-core -p patchhub-cli --all-targets -- -D warnings`：通过（0 错误、0 代码警告）。
2. **`patchhub-core` 库内定向单元测试**：
   - `cargo test -p patchhub-core --lib sources:: -- --skip real_transport`：`113 passed; 0 failed; 2 ignored`（含 `sources::adapters::script`、`canonical`、`http` 非回环、`identity`、`index`、`match`、`registry`、`schedule`、`store`、`validate`）。
   - `cargo test -p patchhub-core --lib installer::manual_selection`：`1 passed; 0 failed`。
   - `cargo test -p patchhub-core --lib installer::resolve`：`13 passed; 0 failed`。
3. **`patchhub-core` 集成测试套件**：
   - `cargo test -p patchhub-core --test source_data_v2 --test source_update --test source_matching --test source_acceptance --test script_source_runtime --test script_source_pipeline --test script_source_fallback --test script_source_import --test browser_source_update --test builtin_script_migration --test builtin_script_regression --test builtin_scripts --test projection --test installer_service`：全部 14 个测试套件通过（`browser_source_update` 3、`builtin_script_migration` 1、`builtin_script_regression` 12、`builtin_scripts` 6、`installer_service` 5 passed / 1 ignored、`projection` 18、`script_source_fallback` 11、`script_source_import` 11、`script_source_pipeline` 10、`script_source_runtime` 9、`source_acceptance` 11、`source_data_v2` 12、`source_matching` 26、`source_update` 18）。
   - `cargo test -p patchhub-core --test source_snapshot -- --skip linked_legacy`：`4 passed; 0 failed; 1 filtered out`。
4. **`patchhub-cli` 单元测试与冒烟测试**：
   - `cargo test -p patchhub-cli --bins --test script_sources_smoke --test projection_smoke`：`50 passed`（bin 单测）+ `2 passed`（`projection_smoke`）+ `3 passed`（`script_sources_smoke`），全部通过。

### 受沙箱限制及待验收项

- **沙箱策略阻塞项（非本次重构回归）**：
  - `sources::http::tests::real_transport_cancels_while_waiting_for_headers_or_body`（全量测试时因 Windows 沙箱禁网拦截 `127.0.0.1` 回环连接而在 `listener.accept()` 长期阻塞，定向检查通过 `--skip real_transport` 跳过）。
  - `installer::acquire::tests::*real_transport*`、`manual_download_task_cancels_stalled_headers_and_stalled_body`、`crates/cli/tests/cache_smoke.rs`（`cache_smoke` 第 234 行在回环下载 `http://127.0.0.1:8416/fixture.zip` 时报 `download_network_error`，前置 `import`/`update_source`/`rematch_game` 已完成）、以及 `source_snapshot::linked_legacy_directories_are_ignored_and_new_state_directories_rejected` / `hard_link*` 测试（沙箱拦截 `cmd /C mklink /J` 与硬链接权限）。上述项目需在非沙箱原生 Windows 或 CI Runner 环境补测确认。
- [ ] **待验收**：Linux/WSL 侧构建与测试、真实站点网络（`--ignored` live 用例）及 Windows / Steam Deck 实机运行验收；发布前按 `AGENTS.md` 执行完整 CI matrix 对齐。
