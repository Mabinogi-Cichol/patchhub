# 覆盖安装 / ZIP 资源策略 / YPF 映射修复独立复核

设计：[design.md](design.md)。施工：[progress.md](progress.md)。方案：[fix-plan.md](fix-plan.md)。日期：2026-10-11。

复核方式：独立 @oracle 只读复核（新会话，未参与实现）。复核对象为工作区未提交的 F1–F3 改动（基线 `dcb63f2`），对照 [fix-plan.md](fix-plan.md) §3–§8、[p0-contracts.md](p0-contracts.md) 与 zip crate 6.0.0 源码（`spec.rs` EOCD/ZIP64 选择、`read.rs` 中央目录回退）核对。未运行写操作；本地全量回归由编排者另行执行（见 §3）。

## 1. 结论

**有条件放行**：未发现可导致数据损坏或安全绕过的阻断缺陷；`zip_metadata::preflight` 对 zip 6 的 EOCD 回退已 fail-closed（任一候选超预算立即 `zip_metadata_limit`，结构不一致立即拒绝）。文档一致性、隐式条目边界与未实测性能项收口后结项。

## 2. 发现与处置

| 编号 | 级别 | 发现 | 处置 |
| --- | --- | --- | --- |
| I1 | 重要 | `design.md` §10 仍标“（待实现）”，与 progress/fix-plan 的“实现完成”矛盾 | 已修订 §10 与首部状态为“代码主体已实现、整合/实机待验” |
| I2 | 重要 | `progress.md` “46 项…全部通过”与 `tests/install.rs` 实有 48 个测试、fix-plan §8 “待跑”冲突 | 已按本轮实测修正：Windows 46（另有 2 个 `#[cfg(unix)]` 在 Linux 运行）、Linux 48，均 0 失败 |
| I3 | 重要 | 6.78 GB 包中央目录预检开销未实测；动态路径 `zip_recipe` 与 `install::prepare` 各自调用一次 preflight，可能整包多读约两遍 | 保留为部署后实测项（实机安装 3137110 计时）；若延迟明显再评估复用预检结论 |
| A1 | 建议 | `MAX_METADATA_BYTES=64 MiB` + 每记录 1 KiB 估算形成约 6.3 万条目隐式边界 | 登记为有实测依据的宽裕预算（10k/50k 压测通过）；不新增低条目门 |
| A2 | 建议 | `validate_metadata_budget` 以每行 2 字节预留 overlay，低于读取口径 `length(state)+32` | 已修复：预检改为逐条 `state.len()+32` 与 load 同口径；收口复跑通过（fix-1） |
| A3 | 建议 | YPF 规则对含非 `pac/*.ypf` 成员（如说明文件）整包拒绝，比 XP3 规则严格 | 登记限制；本样本无说明文件，不推断删除说明；同类包需单独证据 |
| A4 | 建议 | 目标仅大小写不同的现有文件在大小写敏感文件系统上也被拒绝 | 登记为保守权衡（避免同配方歧义目标）；不放松 |
| A5 | 建议 | 残留旧错误码：`install.rs` u64 溢出报 `zip_expansion_limit`；`orchestrator.rs` 仍映射不再产生的 `archive_entry_limit/archive_too_large`；fix-plan §4.3 引用已删除的 `MAX_ARCHIVE_BYTES` | 已修复：溢出改报 `zip_size_overflow`、移除陈旧映射（fix-1）；fix-plan 表述已修订 |
| A6 | 建议 | `installer::preexisting` 成为生产死代码（仅模块声明与自身测试引用） | 已修复：删除模块与声明，CLI marker 不阻断回归保留（fix-1） |
| P1 | 提示 | Windows 完成文案 `operation === "overwrite" ? "覆盖安装完成"` 因 UI 恒发 overwrite，全新安装也显示“覆盖安装完成” | 已修复：按按钮语义（`installed`）区分文案（fix-1） |
| P2 | 提示 | 两端 UI 恒定发送 `operation="overwrite"`（含首次安装） | 与 fix-plan §3.2 一致，非缺陷 |
| P3 | 提示 | 切换工具移除 `--write-protection`、`safe_to_replace` 新逻辑 | 与 fix-plan §3.2(2) 一致，非缺陷 |

## 3. 全量回归（2026-10-11，编排者执行）

| 套件 | 结果 |
| --- | --- |
| Windows `cargo test -p steam-patch-core -p steam-patch-cli` | core lib 287、catalog 8、install 集成 46、localized 4、steam_scan 6、trust 3、cli 41：0 失败 |
| Windows `cargo test -p steam-patch-windows` | 23：0 失败 |
| Linux（WSL2）同套件 | core lib 288、catalog 9、install 集成 48（含 2 个 unix-only）、localized 4、steam_scan 6、trust 3、cli 41：0 失败 |
| Decky Python `unittest discover` | 159 通过（5 平台跳过） |
| Decky TS `node --test` | 63 通过（修复 `validating` 文案期望后） |
| typecheck / 两端前端构建 | 通过 |

修复项（fix-1）完成后需对受影响套件复跑；复跑结果见 [progress.md](progress.md) 2026-10-11 节。

## 4. 需求覆盖缺口（与 fix-plan §7 对照）

- 已覆盖：marker 存在/损坏不阻断、覆盖与旧 committed superseded、外部修改/旧内容缺失、覆盖幂等、取消（准备/备份/写入三时点）、提交失败旧收据保留、不扩大文件清单/不删存档、合成 wrapper/pac（YPF+ASF）、混合布局/冲突/XP3 回归、签名与动态内容重核、EXE overwrite 条件不伪造。
- 未自动化（保留为实机/压测项）：6 GB 级真实流式提取与恢复（由 3137110 实机安装覆盖）、10k/50k 压测（已登记数值，`#[ignore]` 基准保留）、overwrite 对 `needs_recovery` 的直接新用例（由既有恢复用例与未完成事务阻断用例间接覆盖）。
- Windows 实装与共享事务回归、Deck 正式部署与三款游戏实机属于 fix-plan §7 F4：已于 2026-10-11 全部完成（[progress.md](progress.md) 续/续二节），不再待执行。

## 5. 与施工记录的不一致说明

复核发现的进度计数与状态矛盾（I1/I2）已按上文修订；`progress.md` 中“46 项”历史表述保留原文并旁注准确计数与实测日期。
