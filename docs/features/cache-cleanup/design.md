# 下载缓存清理设计与流程

当前流程更新（2026-10-06）：[一站式安装设计](../simplified-install-flow/design.md) 已替代本文涉及的人工确认、卸载及长期备份规则。Decky 下载后直接安装，只提供安装/重装；临时恢复材料由内核在成功或完整回滚后清理，未完成恢复仍保留。以下原有调研与验收事实保留。

施工记录：[progress.md](progress.md)。

本文设计 Decky 插件在**补丁安装成功后删除已下载 ZIP 缓存**的流程：新增**自动清理开关（默认开启，可关闭）**与**手动“清理下载缓存”入口**。核心原则：只删除本工具自己下载、且位于工具状态目录内的缓存文件；绝不触碰用户 `~/Downloads` 中手动导入的 ZIP、事务备份、暂存文件或订阅源查询缓存。

## 背景与现状

当前两条下载路径都把 ZIP 持久保存在工具状态目录（`~/.local/share/steam-patch-manager` 下），成功后都不删除：

| 路径 | 触发场景 | 缓存位置 | 元数据 |
| --- | --- | --- | --- |
| publisher | patch.moe / `source-*` 的“自动下载并准备安装” | `<state>/publisher/<release_id>/<instance-hash>/cache/archive.zip` | 同目录 `download.json`（url/sha256/size/etag） |
| core | signed catalog 的 `fetch_artifact`（managed_zip 无本地包时） | `<state>/downloads/<artifact_sha256>.zip` | 无（文件名即摘要） |

关键事实（来自 `publisher.py` / `download.rs` / `main.py` / `runtime.rs`）：

- `publisher._download` 只有临时文件 `download-*.tmp` 在 `finally` 删除；最终 `archive.zip` 通过 `os.replace` 保留，并在下次准备时命中缓存复用（含 304 条件请求）。
- core `download.rs::fetch` 同样只删 `<uuid>.partial`，最终 `<sha256>.zip` 保留复用。
- **安装/卸载/恢复不再依赖原始 ZIP**：core `prepare` 会把 ZIP 中每个映射成员解出并写入 `jobs/<plan_id>/<index>.stage`，`apply`、`uninstall`、`recover` 都只读 journal 与 stage/backup，`start_install` 不重新打开 ZIP。
- `<state>/downloads` 同时是“手动导入白名单”（`main.py` 与 `find_archive` 允许用户把 ZIP 放这里被复用），因此**不能整目录删除**。
- `~/Downloads`（`Plugin.downloads`）是用户个人目录，永不清理。
- 取消下载必须**保留已有的完整缓存**（`docs/features/decky-ux/design.md`、`test_tasks.py::test_cancel_removes_partial_and_preserves_old_archive`）；本设计只删“安装提交成功”后的缓存，与该约定一致。

## 已确认的产品决策

1. **自动清理时机：安装事务提交成功后立即删除**（`transaction.state == "committed"`）。
   - 提交失败、回滚、`needs_recovery`、取消、仅生成计划未安装：**不删除**，保留以便重试/复用。
2. 同时覆盖 **core `downloads` 缓存**。
3. 提供**手动“清理下载缓存”入口**（全局设置）。
4. 提供**自动清理开关，默认开启**；关闭后安装成功不再自动删除，仅保留手动清理。设置持久化到后端状态目录。
5. 提供**备份保留开关 `keep_backups`，默认关闭**：关闭时安装提交后删除该事务的 `.backup`（以及 `.stage`），该补丁无法卸载、只能重新安装；开启时保留备份、可卸载。无备份的事务通过 `list_transactions` 的 `backups` 字段标示，界面隐藏卸载并只提供重新安装。
6. 本期不做大小/年龄上限（列入未来项）。

代价：开启自动清理时，安装成功后若用户先卸载再重装，需重新下载。这是本决策接受的取舍；需要复用时可关闭开关。

## 设置持久化

自动清理开关必须由后端持有，因为删除动作发生在后端安装流程中；前端 `preferences.ts` 目前仅为内存态，不可复用。

- 新增后端设置文件：`<state>/settings.json`，结构 `{"schema_version": 1, "auto_cleanup": true, "keep_backups": false}`。
- 新建 `apps/decky/backend/plugin_settings.py`（原 `settings.py`，因 Decky 运行时存在 `decky_loader.settings` 会劫持同名导入而改名），参考 `sources._write_json` 的原子写入与 `no_links` 校验方式（`O_EXCL` 临时文件 + `fsync` + `os.replace`，0600），可将该写入辅助抽取为共用函数；失败回退默认值并记诊断。
- 默认值：`auto_cleanup = true`、`keep_backups = false`。文件缺失或字段缺失按各自默认；字段类型非法才回退并记诊断，不阻塞插件启动。
- 后端只暴露最小接口，前端不传文件路径：
  - `async def get_settings(self) -> dict`：返回 `{auto_cleanup, keep_backups, cache: {..., jobs}}`。
  - `async def set_auto_cleanup(self, enabled: bool) -> dict`、`async def set_keep_backups(self, enabled: bool) -> dict`：校验布尔值，持久化后返回最新设置。
- 与安装任务并发：设置读写为独立小文件原子替换；自动清理读取的是当次安装开始时/各分支读取时的最新值，不要求与前端显示实时一致。

## 术语

- **缓存 ZIP**：本工具下载得到的 ZIP 文件，位于工具状态目录内。
- **缓存条目**：一个缓存 ZIP 及其伴随元数据（publisher 的 `download.json`）。
- **受保护资产**：用户 `~/Downloads` 内文件、`<state>/downloads` 内非摘要命名的用户文件、`jobs/` 事务与备份、`sources/cache/` 订阅源查询缓存、`publisher/.../catalog.json|signed.json|public.json|private.key|journal.sqlite`。

## 自动清理流程

### 触发点

在 `apps/decky/main.py::Plugin.start_install` 中，`bridge.start_install(...)` 返回后：

```
result = await bridge.start_install(value, confirmed_game_closed)
if result.get("state") == "committed" and self.settings.auto_cleanup:
    await self._delete_download_cache(plan_id)   # 尽力而为，失败不影响安装结果
return self._wrap_publisher(result, *owner) if owner else result
```

- 开关关闭时该分支直接跳过；缓存留待手动清理或下次复用。
- 开关读取以已持久化的设置为准，见“设置持久化”。
- 该函数同时被 `start_install_task` 的任务体调用，因此自动清理发生在安装任务内，任务结束前完成。
- `_delete_download_cache` 捕获所有异常，仅记录诊断（如 `cache_cleanup_failed`），不得改变安装成功状态。
- 清理完成后从 `_plan_metadata` 移除该 `plan_id` 条目，避免元数据字典泄漏增长。

### 候选判定（位置 + 摘要，双重约束）

自动删除**不依赖 `acquisition` 标签**（publisher 命中缓存时它的值是 `"local"`，与用户本地包混淆）。改为按“路径是否等于本工具推导出的缓存路径”判定：

1. **publisher 缓存**
   - 由 `plan_id` 经 `_dispatch_id` 解出 `(instance_id, release_id)`，再取
     `state = publisher._state(instance_id, release_id)`，期望文件：
     - `state/cache/archive.zip`
     - `state/cache/download.json`
   - 判定：准备阶段确实走的是 publisher 分支（`support_level == "publisher_zip"`）。
2. **core 缓存**
   - 期望文件：`bridge.state_dir / "downloads" / f"{artifact_sha256}.zip"`，其中摘要来自准备阶段已计算的 `release_metadata(...)[0]`。
   - 判定：仅当实际使用的 archive 路径等于该期望路径时才删；`find_archive` 从 `~/Downloads` 或非摘要命名的用户包命中时不会误删。

### 删除前校验（每个文件）

- 路径必须位于 `publisher` 或 `state_dir/downloads` 根之内（`Path.resolve().is_relative_to(root)`）。
- `no_links(path)`：拒绝符号链接 / junction，且所有祖先无链接。
- 必须是普通文件；`unlink(missing_ok=True)`。
- publisher 的 `download.json` 若存在一并删除；残留的 `download-*.tmp` 一并清理。
- 可选加固（默认不做，因会二次读取数 GB）：删除前重算 SHA-256 与期望值比对。由于路径已在 0700 状态目录内且由内部推导，默认仅做路径 + 链接校验，文档记录该取舍。

### 删除对象 / 保留对象

| 对象 | 安装成功后 |
| --- | --- |
| publisher `cache/archive.zip` + `download.json` | 删除 |
| core `state/downloads/<sha256>.zip`（本次使用） | 删除 |
| publisher `cache/` 空目录 | 可保留（幂等，避免竞态） |
| `catalog.json` / `signed.json` / `public.json` / `private.key` | 保留（卸载、检测、续签需要） |
| `journal.sqlite` 与 `jobs/<plan_id>/*.stage/*.backup` | 保留（卸载与审计） |
| `sources/cache/*.json` | 保留（订阅源查询缓存） |
| 用户 `~/Downloads/*.zip` | 保留 |
| `<state>/downloads` 非摘要命名文件 | 保留 |

### 与其它状态交互

- **计划仅准备、未安装**：`archive.zip` 保留（下次可复用）。
- **安装回滚 / 取消 / `needs_recovery`**：保留缓存，支持重试。
- **提交与取消竞态但最终 `committed`**：按 `committed` 删除。
- **卸载成功**：本期不额外删除（缓存已在安装时删除；若当时删除失败，可由手动清理兜底）。
- **并发**：清理在安装任务内串行执行；手动清理入口在 `installer_busy` 时拒绝。

## 事务暂存与备份（`.stage` / `.backup`）

补丁安装会在 `<state>/jobs/<plan_id>/`（publisher 为 `<state>/publisher/<release>/<instance-hash>/jobs/<plan_id>/`）下产生事务文件（核心 `crates/core/src/install.rs`）：

- `<index>.stage`：生成计划时从 ZIP 解出并校验的文件副本，用于写入与复核。提交后不再需要，**安装成功后自动删除**。
- `<index>.backup`：覆盖前保存的原文件内容，卸载 / 回滚据此还原。删除后该事务无法卸载，只能重新安装。

原文件备份是“全量”的：要原样还原被替换文件就必须保存原件。理论上可用二进制增量（bsdiff / zstd `--patch-from`）在原件与补丁件高度相似时缩小体积，但游戏数据包多为压缩 / 重打包，增量常接近原件大小，且属于核心层改造（备份格式、崩溃恢复、哈希校验、跨平台测试），暂不实现。

当前策略：

- 安装提交后始终删除该事务的 `.stage`。
- `keep_backups` 默认关闭：同时删除 `.backup`，该补丁无法卸载、只能重新安装；开启时保留 `.backup` 以支持卸载。核心在安装过程中仍会临时创建备份用于崩溃回滚，提交后才按设置删除。
- 无备份的事务通过 `list_transactions` 的 `backups=false` 标示，界面隐藏“卸载”并只显示“重新安装”；后端 `uninstall` 也会以 `uninstall_backups_missing` 拒绝。
- 提供手动“清理事务备份与暂存”，删除 `committed` / `uninstalled` / `rolled_back` 事务的 `.stage` 与 `.backup`（`planned` / `applying` / `needs_recovery` 等保留）。需先关闭游戏且无进行中任务。清理 `committed` 事务的备份会使其无法卸载。
- 统计并显示这些文件占用。

### 无备份时允许重新安装

无备份的已安装补丁无法卸载，但核心 `install::prepare` 的 `ensure_no_active` 会因旧的 `committed` 事务拒绝新准备（`managed_install_exists_or_needs_recovery`）。在无法改动核心（本机与 Deck 当前都没有 Rust 工具链）的情况下，decky 侧这样处理：

- `scan` 不再把“无 `.backup` 的 `committed` 事务”计为阻止条件（`publisher_existing_transaction`）；**有备份的事务仍然阻止**，以保护卸载能力。
- `prepare_install` 在 publisher / 基础路径准备前调用 `_forget_without_backups(state_dir)`：仅当该状态目录下**没有任何 `.backup`** 时，删除核心 `journal.sqlite*` 与 `jobs/`，保留 `catalog.json`、`signed.json`、密钥与 `instance.json`；核心下次运行会重建空 DB。
- 效果：默认（不保留备份）模式下，用户可重新下载并覆盖安装；旧记录清空后检测状态变为 `installed_external`，界面提供“重新安装”。
- 这是临时桥接；长期应由核心提供一个“丢弃已完成且无备份的事务”方法（需要重建核心）。

## 手动清理流程

### 后端接口

新增到 `apps/decky/main.py`（设置读写见“设置持久化”）：

- `async def cache_status(self) -> dict`
  - 只读，统计：
    - `publisher`: 遍历 `publisher.base/*/*/cache/`，累加 `archive.zip`/`download.json`/`download-*.tmp` 字节与文件数。
    - `core`: 遍历 `<state>/downloads/`，仅统计匹配 `^[0-9a-f]{64}\.zip$` 或 `*.partial` 的文件。
    - 返回 `{total_bytes, file_count, publisher_bytes, core_bytes, scanned_at}`。
  - 全程 `no_links`；遇链接或异常跳过并计入诊断，不中断统计。
- `async def clear_cache(self) -> dict`
  - 前置：若 `self.tasks.busy()` 或 `self._prepare_cancel is not None` → `installer_busy`。
  - 删除范围同 `cache_status` 统计范围：
    - publisher：每个实例 `cache/` 下的 `archive.zip`、`download.json`、`download-*.tmp`。
    - core：`<state>/downloads/` 下摘要命名 `*.zip` 与 `*.partial`。
  - **不删除**：`jobs/`、`journal*`、`instance.json`、`catalog/signed/public/private`、`sources/`、`~/Downloads`。
  - 返回清理后的 `cache_status`。

### 前端

- 在 `index.tsx` 增加：
  - `const clearCache = callable<[], CacheStatus>("clear_cache")`
  - `const clearTransactionBackups = callable<[], CacheStatus>("clear_transaction_backups")`
  - `const getSettings = callable<[], Settings>("get_settings")`（返回值含 `cache` 占用，面板无需单独调用 `cache_status`）
  - `const setAutoCleanup = callable<[boolean], Settings>("set_auto_cleanup")`
- 入口为**全局设置的独立侧栏页**「下载缓存」（路由 `/steam-patch-manager/sources/cache`，与「管理订阅源」并列），不再嵌在订阅源页底部。页面包含两个分区：
  - **下载缓存**：`ToggleField`「安装成功后自动删除下载缓存」（默认开启）、当前占用、`Action`「清理下载缓存」。
  - **事务备份与暂存**：显示 stage/backup 占用，`Action`「清理事务备份与暂存」并弹确认，明确告知删除备份后无法卸载、只能重新安装。
- 文案与 `docs/features/decky-ux/design.md` 的措辞统一（“缓存”专指订阅源查询结果时需与“下载缓存”区分）。

## 安全与边界

- **路径约束**：删除前统一 `resolve().is_relative_to(允许根)`，并使用 `no_links` 拒绝链接与 junction；与仓库既有状态路径校验风格一致。
- **不可整目录删除**：`state_dir/downloads` 含用户文件；仅按摘要命名/临时扩展名精确匹配。
- **坚持本地信任边界**：`state` 目录权限 0700，属于当前账户信任边界；本设计不承诺抵御同账户篡改（沿用 `install-design.md` 的威胁模型）。
- **失败可观测**：清理失败写诊断而非使安装失败；手动清理返回实际剩余量与失败项计数。
- **幂等**：文件不存在时 `missing_ok=True` 静默成功。

## 测试计划

后端（`apps/decky/backend/test_*.py`）：

1. publisher 提交成功后：`install` 任务完成后 `archive.zip` 与 `download.json` 被删，`catalog.json`/`signed.json`/`journal.sqlite` 保留。
2. 命中 publisher 缓存（`acquisition == "local"` 的复用）安装成功后仍会被删除。
3. core `fetch_artifact` 摘要文件在 `committed` 后被删。
4. 回滚 / 取消 / `needs_recovery` / 仅 `planned`：缓存保留。
5. 用户 `~/Downloads` 与 `<state>/downloads` 非摘要命名文件在自动与手动清理后均存在。
6. 删除目标为符号链接时跳过且不删链接目标。
7. `clear_cache` 在 `installer_busy` 时抛 `installer_busy`。
8. `cache_status` 大小/计数正确，且不修改文件。
9. 默认 `auto_cleanup=True`：未写设置文件时提交成功即删除。
10. `set_auto_cleanup(False)` 后提交成功不删除；设置持久化，重启/重读后仍为关闭。
11. `settings.json` 缺失或损坏时回退默认开启，不抛错。

前端（`apps/decky/tests/`）：

12. 确认弹窗取消时不调用 `clear_cache`；确认后调用并刷新状态。
13. 开关默认显示开启；切换关闭调用 `setAutoCleanup(false)` 并持久化显示。
14. `busy` 时按钮与开关禁用。

集成/实机（`docs/features/decky-ux/progress.md` 记录）：

15. Deck 实机：默认开启下安装一个补丁，确认 `publisher/.../cache` 与 `downloads/` 中对应 ZIP 消失、游戏内补丁仍生效、操作记录/卸载仍可用。
16. 关闭自动清理后安装，确认缓存保留；手动清理后再次“自动下载并准备安装”，确认能重新下载并复用流程正常。

## 文档同步（实现时一并更新）

- `docs/privacy.md`：改为“安装成功后会删除已下载 ZIP 缓存（可通过开关关闭）；事务历史与备份仍保留”，修正“卸载成功不等于自动删除全部下载和暂存”的表述。
- `docs/client-guide.md`：补充安装成功后清理下载缓存、自动清理开关（默认开启）以及全局设置中的手动清理入口。
- `docs/install-design.md`：在“卸载与恢复”末尾区分“下载缓存已清理”与“事务/备份暂无自动清理”。
- `apps/decky/README.md`：一句话说明缓存生命周期与开关。
- `docs/features/decky-ux/design.md`：在进度/取消相关条目附近注明“安装成功后清理本次下载缓存”。
- `docs/progress.md` / `docs/features/decky-ux/progress.md`：记录本轮施工与实机验收结果。

## 实施拆分

1. `plugin_settings.py`（新增，原名 `settings.py`）：`Settings` 持久化（默认 `auto_cleanup=true`）与原子读写。
2. `publisher.py`：新增 `clear_cache(instance_id, release_id)` 与缓存枚举/大小辅助（复用 `no_links`）。
3. `main.py`：`_plan_metadata` 记录缓存推导所需信息（`cache_kind`、`artifact_sha256`）；`start_install` 提交后按开关调用 `_delete_download_cache`；新增 `get_settings`/`set_auto_cleanup`/`cache_status`/`clear_cache`。
4. `index.tsx`：新增 callable、全局设置“下载缓存”分区（开关 + 占用 + 清理确认弹窗）。
5. 测试与文档同步。

## 未来项（本期不做）

- 按大小的缓存上限与自动裁剪最旧缓存。
- “安装后保留/删除”之外的更细策略（例如按发行版白名单）。
- 事务与 `jobs/` 备份的清理策略（与审计需求权衡）。
- 跨安装复用同一 ZIP 时撤销清理（需引用计数或摘要驻留策略）。


## 2026-10-19：恢复当前核心的缓存占用统计

- 全局设置“缓存与备份”的下载缓存“当前占用”显示真实容量，移除固定的“当前版本不提供占用统计”。
- 沿用 Rust `CacheService::inspect()`，经协议 v5 的只读 `cache_status` 方法、CLI 和 Decky 桥接返回 `CacheInventory { total_bytes, entries, diagnostics }`；不在 Python 重写文件遍历。
- 统计范围与现有 `cleanable_dirs()` 一致：`content/`、`downloads/`、`reports/`，不含安装备份、事务恢复材料、来源索引及用户 Downloads。`entries` 为遍历目录项数，界面只显示字节容量。
- 进入页面独立读取统计；手动清理返回后重新统计。读取失败显示错误，部分路径失败显示已统计容量及诊断；不将上次清理释放量当作当前占用，不增加轮询、重试或统计缓存。
- 验收：真实 RPC 能统计非空缓存、清理后归零并保留事务材料；前端能呈现零/非零/部分统计；桥接保留诊断与异常。实机部署及页面验收单独记录于 [施工记录](progress.md)。
