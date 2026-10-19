# 跳过显示文本解码计划

日期：2026-10-19（Asia/Shanghai）。状态：**已实施；自动化回归和 Deck 样本校验、插件部署通过；Windows 真实沙箱样本受 ACL 错误阻断，游戏安装由用户操作。** 此前方案 A 的编码回退已全部回退。

关联：[设计](design.md) · [施工记录](progress.md) · [VBF 分析失败调查](investigation-20261019-vbf-display-encoding.md) · [限制清单](policy-inventory.md) · [默认值限制移除计划](fix-plan.md)。

## 执行范围修订（2026-10-19）

- 用户在执行中明确选择“只完成构建、样本校验和插件部署；游戏安装由我操作”。下文原 P4 的游戏安装/恢复验收不在本轮执行范围；当前产品已移除备份/恢复，不为本计划重新引入。
- 实施基线为 `4d1059143fef2af3e7e78e21f0f825eae8bd9f58`。当前 worker 已改为 `extract_behavior` 流式输出，`convert_behavior` 已删除；因此 P3 按真实生产协议核对逐成员路径、字节、CRC、SHA 和清单摘要，不调用旧转换接口或声称产生规范化 ZIP。参考生成器中的 ZIP 仅是独立内容基线。
- P0 已完成，原 13 个 graph/20 个 behavior v2 预期对等；全报告另同步元数据分析不解压载荷的契约。P1/P2 已完成。P3 的 9 个合成包及 VBF/夏日本体/DLC 在 Deck bwrap 通过，Windows 受 `protect private staging DACL failed`（错误 5）阻断；2303800/3606360/3721970 本机缺少原样本，保留待验证。P4 插件部署通过；P5 已更新。具体证据与命令见[施工记录](progress.md)。


## 1. 目标

在通用 EXE 安装分析链路中**跳过显示文本解码**：只读取显示字段的长度并消费对应字节，不把字节解码为字符串。

同时删除只为显示文本服务的证据字段与代码：

- 删除 `variables`（项目/SessionVar 变量）与 `terms`（许可协议正文）输出；
- 删除 `legacy_display` / Python `gb18030` 显示解码、`exe_display_encoding_unsupported` 错误路径及其测试；
- 删除 `installer-format` 中仅为显示解码使用的 `encoding_rs` 依赖。

不新增编码探测、回退链、配置项或新的限制。安装条件、载荷选择、文件写入集合与事务行为保持不变；报告校验随删除字段和版本标识同步调整。

### 1.1 层次与范围

生产安装已由 Rust 核心体系承担：Decky 前端负责交互，Python host 负责桥接；Rust core 负责安装编排、报告校验和文件事务，worker 与 `installer-format` 负责隔离分析及转换。本计划中的 `scripts/sf9_graph.py`、`scripts/sf9_behavior.py` 是研究与测试参考实现，不是 Decky 前端或生产安装引擎。

本次不把安装逻辑迁回 Python，也不开展全仓旧代码清理。参考实现的维护范围仅覆盖本次报告对等、golden 生成与现有安装语义验证所需的链路。

## 2. 事实依据

| 事实 | 证据 |
| --- | --- |
| 显示文本不在任何界面显示 | Decky/Windows 前端不读取 `script_inventory`、`variables`、`terms`；`report.json` 只写不读 |
| 显示文本不参与安装授权 | 条件、载荷、路径、事件脚本来自文件表与非显示字符串；核心只对 `variables`/`terms` 做形状校验 |
| `variables` 无消费者 | 仅进入 `script_inventory` 并写入 `report.json` |
| `terms` 只影响摘要 | 复制到 `behavior.terms`，参与 `behavior_sha256`；不参与条件、载荷或写入 |
| 重装身份不依赖 `behavior_sha256` | 重装使用 EXE/metadata/payload 内容身份与历史事务，见 [install.rs:1194-1239](../../../crates/core/src/install.rs) |
| 显示解码是 VBF 失败唯一来源 | `graph::decode` → `legacy_display` → worker 退出 1，见[调查报告](investigation-20261019-vbf-display-encoding.md) |

当前解码实现位置：

- Rust：[graph.rs:50-106](../../../crates/installer-format/src/graph.rs)（`legacy_display`）、[graph.rs:161-195](../../../crates/installer-format/src/graph.rs)（`display`）、[graph.rs:539-566](../../../crates/installer-format/src/graph.rs)（输出 `variables`/`terms`）；调用点包括屏幕、控件、消息、SessionVar、外观、项目、图像、密码等。
- Python：[sf9_graph.py:43-67](../../../scripts/sf9_graph.py)、`sf9_graph.decode` 输出；[sf9_behavior.py:336](../../../scripts/sf9_behavior.py) 的 `"terms"`。
- 报告校验：[exe_report.rs:518-565](../../../crates/core/src/installer/exe_report.rs) 要求 `behavior.terms`、`script_inventory.terms/variables`；[exe_report.rs:602-606](../../../crates/core/src/installer/exe_report.rs) 校验 `terms` 数组。
- 落盘：[exe_orchestrator.rs:233-239](../../../crates/core/src/installer/exe_orchestrator.rs) 写入 `reports/<content_id>/report.json`。

已确认的前置不一致不限于版本号：Python graph/behavior 仍标记 v1，Rust 与已提交 golden 已采用 v2；Python 事件缺少 `lifecycle`、behavior 缺少 `file_conditions`，还保留 Rust 已移除的部分屏幕、组件默认值限制。直接改版本号不能取得语义对等。必须先完成 §3.0 的参考基线核对，再生成新 golden。

## 3. 实施方案

### 3.0 前置：确定可复现的参考基线

1. 以实施开始时的 Rust 源码、现有 golden 和施工记录中已验证的 v2 行为为基线，记录提交与已有工作区改动；保留原始测试输入及现有预期，避免新输出覆盖后无法判断变化来源。
2. 查找此前生成 v2 golden 的独立参考实现及生成方式；若能够恢复，优先复用并记录来源。若不能恢复，则补齐仓库 Python 参考链路与当前 Rust 的语义差异，至少核对事件生命周期、include selectors、文件尾标志、组件状态/条件及逐文件条件输出，并检查相关 audit/worker 调用。仅同步当前已实现的语义，不借此新增安装功能。
3. 在跳过解码前，先以现有可成功解析的合成用例验证 v2 报告对等。参考脚本保留的旧默认值拒绝分支及专属测试，按当前契约删除；仍验证业务和结构边界的测试保留。VBF 已知编码失败应记录为待修复，不要求其在此阶段成功。
4. 参考基线对齐与跳过解码分别记录改动、验证结果及预期 diff；若后续提交，按独立修复及文档拆分。不得用 Rust 实际输出直接覆盖预期、过滤有意义的差异或修改生产 Rust 去迎合旧 Python。
5. 未完成对齐时，不把修改版本号或重新冻结文件记为“Rust/Python 对等通过”。先解决具体差异，再进入 §3.4 的正式重生成。

### 3.1 Rust 解析器（`crates/installer-format/src/graph.rs`）

1. 将 `display()` 改为 `skip_display()`：读同样的长度头（`0xFF`/`0xFFFE`/`0xFFFF` 分支），按 `wide` 计算字节数，`r.read(byte_len)` 消费后返回 `()`。保留现有长度上界检查，避免改变畸形输入的既有错误语义。
2. 删除 `legacy_display` 及其两个测试（`display_codec_keeps_python_legacy_mapping`、`display_codec_rejects_malformed_bytes_without_panics`）。
3. 删除 `variables`、`terms` 字段：
   - `CSessionVar`：仍读变量名、跳过值、按原宽度消费 flag；删除变量 map 与重名校验。当前 Rust 对 flag 只读取、不限制枚举值，不从旧 Python 恢复 `flag in (0, 1, 2)` 准入限制。
   - `CSUFMessageID`：仍读消息名、跳过文本；删除 license 屏幕里的 `terms` 收集循环。
   - 其余 `self.display()?` 全部改为 `self.skip_display()?`。
4. `decode()` 输出删除 `variables`/`terms` 键，decoder 版本改为 `sf9-graph-v3`。
5. 删除 [Cargo.toml:13](../../../crates/installer-format/Cargo.toml) 的 `encoding_rs` 依赖（该 crate 内无其他使用），同步更新 `Cargo.lock`；若其他 crate 仍使用该包则保留其锁定记录。

### 3.2 Rust 行为与报告

1. [behavior.rs:1357-1360](../../../crates/installer-format/src/behavior.rs)：删除 `"terms": graph["terms"]`；`VERSION` 升为 `sf9-behavior-v3`。
2. [exe_report.rs:518-529](../../../crates/core/src/installer/exe_report.rs)：behavior 精确键集合删除 `terms`。
3. [exe_report.rs:551-565](../../../crates/core/src/installer/exe_report.rs)：`script_inventory` 精确键集合删除 `terms`、`variables`。
4. 校验常量更新为 `sf9-graph-v3` / `sf9-behavior-v3`；删除 `behavior["terms"]` 数组校验。
5. 更新受影响的 Rust 测试与手工构造的 graph JSON（如 [semantics_bounds.rs:31](../../../crates/installer-format/tests/semantics_bounds.rs)）。

### 3.3 Python 参考实现

本节以前置参考基线对齐完成为条件；以下字段删除不能代替 §3.0 的语义同步。

1. [sf9_graph.py](../../../scripts/sf9_graph.py)：删除 `display=True` 解码路径，改为跳过字节；删除 `variables`/`terms`；`VERSION = "sf9-graph-v3"`。
2. [sf9_behavior.py](../../../scripts/sf9_behavior.py)：删除 `"terms"`；版本升为 `sf9-behavior-v3`。
3. 调整依赖这些输出的测试；不保留为旧字段服务的断言。

### 3.4 golden 与真实样本参考

1. 参考基线对齐、显示字段删除及 §3.6 合成回归完成后重新生成 golden：`python scripts/freeze_sf_rust_fixtures.py`。graph、behavior、worker golden 会因字段删除与 decoder/policy 版本变化而更新；inventory/audit golden 是否变化取决于其实际输出是否包含这些字段，以重生成后的真实 diff 为准，不手工只改个别字段。
2. 重新生成 `.artifacts/generic-exe-local/*-behavior-analysis.json` 等本地参考报告，供 `verify_sf_rust_samples.py` 对比。
3. 本计划不提交真实游戏样本或报告。逐项核对新旧 golden 差异：本次允许显示字段删除、版本标识及相关行为摘要变化；安装条件、载荷和写入集合的其他变化必须解释并解决，不能仅以“重新生成结果”为正确性依据。
4. `verify_sf_rust_samples.py` 默认仅覆盖三个 AppID。实施时显式纳入 VBF 1189440 与夏日样本，记录实际 AppID、样本路径、EXE/metadata SHA-256、参考生成命令与结果路径；缺少样本标为待验证，不能以默认脚本成功代表完整矩阵通过。

### 3.5 文档

实施完成后更新 `progress.md`（记录实际修改与验证）、`design.md`（显示文本为跳过字段）、`policy-inventory.md`（删除显示编码行/说明），并保留本调查报告的历史事实。

### 3.6 本次故障的合成回归

使用可提交的合成元数据，不复制真实游戏内容；Rust 与 Python 都覆盖以下契约，并将适用用例纳入 golden 生成：

- 在显示字段放入本次触发字节 `0xA9 0x20`，完整分析成功；与有效显示文本的对照样本相比，安装条件、选中载荷及写入集合一致，输出不含 `variables`/`terms`。
- 覆盖 ANSI 与宽字符的短长度、扩展长度头（`0xFF`/`0xFFFE`/`0xFFFF`），在显示字段后放置可识别的结构字段，验证偏移准确；宽字符字节不要求能够解码成合法 UTF-16。
- 长度头或声明内容截断时仍返回结构读取错误，不吞异常、不把后续字段误当文本。保留非显示路径/脚本字符串原有校验的回归，确保跳过范围没有扩大。
- 删除仅验证旧显示编码拒绝/映射的测试，不删除仍验证上述业务或结构契约的用例。

## 4. 兼容性与影响

- **报告形状**：`script_inventory` 少 `terms`/`variables`，`behavior` 少 `terms`；decoder/policy 版本升级用于区分语义。报告数字 `schema_version` 不单独升级；版本标识由 decoder/policy 承担，避免增加一份无消费者的版本字段。
- **`behavior_sha256`**：所有新分析的摘要会变化（`terms` 移除）。未发现跨版本比较该摘要的安装/重装路径；重装身份来自 EXE/metadata/payload。实施时必须用现有重装/授权测试确认。
- **已有事务与 receipt**：不需要迁移；旧 receipt 保留原摘要，不被重新计算。
- **审计证据**：`report.json` 不再包含许可协议正文和项目变量。用户已确认这些内容没有消费方；若未来需要，应以独立只读工具从原始包提取，而不是重新引入分析准入依赖。
- **错误码**：`exe_display_encoding_unsupported` 从显示路径消失；VBF 及任何非 GB18030 显示文本不再阻断分析。路径/脚本等非显示字符串仍按原规则校验。
- **UI/协议**：前端交互及外部调用协议不变；内部 worker 报告契约变更，核心与 worker 必须配套更新。旧核心严格要求 v2 及旧字段集合，不能只部署新 worker，也不增加 v2/v3 双版本兼容分支。

## 5. 实施与验收步骤

| 阶段 | 工作 | 验证 |
| --- | --- | --- |
| P0 | 按 §3.0 恢复或同步独立 Python 参考基线 | 现有可解析用例与 v2 Rust/golden 对等，记录差异及解决结果 |
| P1 | Rust 解析器、behavior、报告校验、依赖与单元测试 | `cargo fmt --all -- --check`；`cargo clippy -p steam-patch-installer-format -p steam-patch-core --all-targets --locked -- -D warnings`；`cargo test -p steam-patch-installer-format -p steam-patch-core -p steam-patch-installer-worker --locked` |
| P2 | Python 跳过解码、合成回归、golden 重生成 | `python -m unittest discover -s scripts -p 'test_sf*.py' -v`；`cargo test -p steam-patch-installer-format --locked`（golden） |
| P3 | 真实样本分析 | 构建的 `sf-worker` 对 VBF v5 返回成功；Rust 报告与 Python 参考一致且不含 `variables`/`terms`；现有四款样本/夏日分析/转换摘要核对 |
| P4 | 实机验收 | Windows 与 Steam Deck 部署同次源码构建的核心宿主/CLI 与 worker，并同步打包清单及哈希；确认运行版本后，从应用安装入口执行 VBF v5 安装与恢复、同内容重装及任一既有样本回归安装 |
| P5 | 文档 | 施工记录、设计、限制清单更新；调查报告状态改为已实施/已验证的范围 |

执行顺序：P1 的代码修改可先完成，但完整 Rust 测试依赖 P2 的 v3 golden；P1/P2 合并后统一执行表中格式、clippy 和 Rust 测试，不把旧 golden 导致的阶段性失败记为最终结果。

前置条件：实施时核实仓库要求的 Rust 工具链（原计划记录为 1.94）及真实样本路径，不沿用“当前开发机没有”的旧判断。缺少环境或样本时明确记录待验证项。原计划编制阶段未构建、部署或操作设备；本次执行按上方范围修订推进。

## 6. 验收标准

- VBF v5 分析不再返回 `exe_display_encoding_unsupported`，报告无显示文本字段。
- 安装条件、选中载荷、文件写入与转换摘要与修改前一致（显示字段除外）。
- Rust/Python 对等样本报告一致；golden、单元测试、格式与 clippy 全通过。
- Windows/Deck 实际安装与恢复通过，同内容重装与既有样本回归通过。
- 可执行代码及活跃测试不再消费旧 `variables`/`terms` 报告字段、调用 `legacy_display` 或要求显示编码错误；历史调查记录和计划中的说明保留，不按全仓关键词匹配删除历史证据。
- 合成回归覆盖非 GB18030 显示字节、长度分支、后续字段定位及截断错误；参考基线与 v3 golden 均可按记录复现。

## 7. 回退

改动集中在解析/报告/参考实现与 golden，不涉及数据库迁移。出现问题时整体回退本次解析/报告变更，并配套恢复核心宿主/CLI、worker 及打包清单，不能单独回退一个组件。已有 receipt 与事务不迁移；实机验证产生的游戏文件变更按正常恢复流程处理，回退程序本身不等于恢复游戏文件。

## 8. 过度设计审阅

生产修复以删除无用解码及字段为主；另补齐已有验证链路，不新增生产策略：

- 不新增代码页探测、回退链、编码配置或“以防万一”的异常处理；显示字段只按长度读取。
- 不保留 `variables`/`terms` 空占位字段；没有消费者的字段直接删除，避免后续继续维护。
- 不新增报告版本字段；沿用已有 decoder/policy 版本标识。
- 保留长度上界与结构校验，属于既有格式边界，不是新增防御。
- 真实样本与实机验收按需执行；在完成前不把计划描述当成已实施证据。
