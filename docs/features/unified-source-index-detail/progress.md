# 订阅源缓存与抓取模型统一重构施工记录

状态：**待实现**。设计方案见 [design.md](design.md)。当前分支：`feature/unified-source-index-detail`（基于 `feature/script-source-adapters@c0e94d1` 创建）。

## 2026-11-17

### 当前状态

- **阶段**：方案设计与审阅完成，**待实现**（尚未修改产品代码与测试代码）。
- **已完成工作**：
  - 核对 `crates/core/src/sources/`（`model.rs`、`matching.rs`、`update.rs`、`store.rs`、`registry.rs`、`index.rs`、`identity.rs`、`adapters/mod.rs`、`adapters/script.rs`）、安装解析路径（`installer/resolve.rs`、`exe_resolve.rs`、`manual_selection.rs`、`projection.rs`）及 5 份内置脚本（`patchmoe.lua`、`kagura-cn.lua`、`kagura-en.lua`、`yozakura.lua`、`hgamer.lua`）的现有实现。
  - 完成 [design.md](design.md) 方案编写，明确 `SourceIndexEntry` + `SourceDetail` 统一模型、`resolve_detail` Step 推进契约、单一候选生成流水线、Schema 2 缓存淘汰策略及 5 份内置 Lua 脚本的迁移映射。
  - 按 `overengineering-review` 完成设计方案过度设计审阅，确认不引入多余状态机、多入口反射或无依据硬限制。

### 待办与待验证项

- [ ] P1：核心数据模型与存储缓存层重构（**待实现 / 待验证**）
- [ ] P2：Lua 执行器与候选生成统一（**待实现 / 待验证**）
- [ ] P3：五个内置 Lua 脚本迁移与测试夹具更新（**待实现 / 待验证**）
- [ ] P4：`cargo fmt --all -- --check`、受影响包 Clippy（`-D warnings`）、定向测试、空内置导入恢复验证及实现后代码过度设计复核（**待实现 / 待验证**）
- [ ] Windows 与 Steam Deck 实机验证（**待验证**）
