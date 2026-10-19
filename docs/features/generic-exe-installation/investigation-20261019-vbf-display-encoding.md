# VenusBlood FRONTIER EXE 补丁安装失败调查报告

日期：2026-10-19（Asia/Shanghai）。状态：**调查完成；根因已定位并复现；用户判定显示文本无业务用途，决定跳过解码。编码回退修改已回退，实施计划见[跳过显示文本解码计划](skip-display-decoding-plan.md)（已实施，Deck 样本分析/提取与插件部署通过；游戏安装待用户操作，Windows 实样本沙箱受 ACL 阻断）。**

关联：[设计](design.md) · [施工记录](progress.md) · [安装器默认值限制移除计划](fix-plan.md) · [旧 EXE 静态审计](../exe-patch-installation/p0-audit.md)。本报告只记录失败调查，不代表补丁已可安装，也不改动既有验收结论。

## 1. 结论

安装 `VenusBlood FRONTIER International`（AppID 1189440）的 Kagura CN v5 EXE 补丁时，返回 `exe_analysis_failed · [worker] exe_display_encoding_unsupported`。

根因不是下载损坏、网络、权限或磁盘空间，而是安装包元数据中**一条显示文本使用 Windows-1252 编码的 `©`（字节 `0xA9`）**，而完整图解析器对 ANSI 显示文本**只按 GB18030 严格解码**：GB18030 中 `0xA9` 是前导字节、后一字节必须是合法尾字节，而该字符串中紧随其后的是空格 `0x20`，因此严格解码失败，worker 以非零退出并返回错误码，核心把该 worker 错误归类为 `exe_analysis_failed`。

失败发生在 `analyzing_installer` 阶段、生成安装计划之前，**没有写入任何游戏文件**。

## 2. 用户可见错误与代码链路

错误文本由“错误码 + 详情”组成：

```text
exe_analysis_failed · [worker] exe_display_encoding_unsupported
```

| 环节 | 位置 | 行为 |
| --- | --- | --- |
| 安装编排调用分析 | [exe_orchestrator.rs:172-179](../../../crates/core/src/installer/exe_orchestrator.rs) | `analyze_behavior` 描述符交给隔离 worker |
| 元数据分析入口 | [audit.rs:1056-1060](../../../crates/installer-format/src/audit.rs) | `behavior=true` 时调用 `graph::decode` 完整解析 |
| 显示文本解码（修复前） | [graph.rs:50-106](../../../crates/installer-format/src/graph.rs) | ANSI 显示文本只尝试 GB18030，失败即 `exe_display_encoding_unsupported`；编码回退曾实施后回退，当前选定方向是跳过解码，见第 8 节 |
| worker 错误返回 | [production.rs:388-401](../../../crates/installer-worker/src/production.rs) | worker 退出码非 0 时读取 stdout 的 `error`，stage 记为 `worker` |
| 错误展示格式 | [model.rs:31-42](../../../crates/installer-worker/src/model.rs) | `IsolationError` 显示为 `[stage] message` |
| 归类为分析失败 | [exe_orchestrator.rs:16-35](../../../crates/core/src/installer/exe_orchestrator.rs) | 详情含 `[`/空格，非纯错误码，包装为 `exe_analysis_failed` |

因此 `[worker] exe_display_encoding_unsupported` 是 worker 阶段错误详情，`exe_analysis_failed` 是编排层归类；前端按“错误码 · 详情”显示。

## 3. 复现

本地样本与 2026-10-05 审核的 v5 完全一致（[p0-audit.md:101-113](../exe-patch-installation/p0-audit.md)）：

| 项 | 值 |
| --- | --- |
| 样本 | `.artifacts/exe-patch-research/vbf/VenusBlood-FRONTIER-CN-Patch-v5.exe`（仓库忽略目录） |
| EXE SHA-256 | `502063d21d2e741533e7479955b6bbdc68dd36d55ad3cd01e86fa509d9056410` |
| EXE 大小 | 1,126,694,464 |
| 元数据 `irsetup.dat` SHA-256 | `5a0ca3e10199a457ac8a52c83f4d61bc4662f2bbeb5a149825263b2ee558f5e3` |
| 元数据大小 | 112,533 |

将样本作为 `input/archive.exe` 交给当前 Rust worker，执行 `analyze_behavior` 描述符：

```text
{"action":"analyze_behavior","exe_sha256":"502063d2…","exe_size":1126694464}
```

实际输出（退出码 1）：

```text
{"error":"exe_display_encoding_unsupported"}
```

同一元数据交给仓库现有 Python 解析器（[sf9_graph.py](../../../scripts/sf9_graph.py)，第 60-66 行）也得到同一个 `exe_display_encoding_unsupported`，说明这不是 Rust 移植独有的行为，而是当前解析器对显示文本编码的共同假设。元数据在进入图解码前已按 overlay 内 CRC 校验通过（[audit.rs:1036-1045](../../../crates/installer-format/src/audit.rs) 传入 CRC，校验逻辑在 `decode` 内 `payload_crc_mismatch` 分支），因此可排除下载或解压损坏。

## 4. 根因细节

### 4.1 失败字节与文本

元数据文件内偏移 `71364` 处的显示字符串：

```text
raw   = 1e 43 6f 70 79 72 69 67 68 74 20 a9 20 32 30 32 30 20 25 43 6f 6d 70 61 6e 79 4e 61 6d 65 25
长度头 = 0x1e（30 字节内容）
cp1252 = "Copyright © 2020 %CompanyName%"
```

它对应项目变量 `%Copyright%`（同文件的 `%CompanyName%` 等变量均正常）。`0xA9` 在 Windows-1252/Latin-1 中是 `©`；在 GB18030 中它是双字节前导字节，后续字节必须是合法尾字节（`0x40–0xFE`，不含 `0x7F`），而这里是空格 `0x20`，严格 GB18030 解码必然失败。

### 4.2 不是整包编码选错

对同一元数据做逐字符串容错解码统计：

- 显示字符串共 772 条；其中 560 条包含非 ASCII 字符，项目界面为中文（`%ProductName%`、欢迎页/许可协议等均为中文 GB18030 文本）。
- 771 条可由 GB18030 正常解码；**只有 1 条失败**，即上述版权行。
- 用 Windows-1252 补上该条后，整个元数据可完整解析，文件表、事件、变量、语言表等均无其他编码错误。

因此这是一个**中文项目里混入一条西文编码默认版权文本**的混合编码个案；不是选择了错误语言，也不是整包编码损坏。

### 4.3 为什么显示文本不是安装授权依据

显示文本只用于界面文案、项目变量、语言表消息等展示信息；安装授权所依赖的文件表、目标路径、条件、事件脚本分别由结构字段和严格 UTF-8 脚本字符串解析（`legacy_display` 仅用于 `display=True` 分支，见 [graph.rs:161-195](../../../crates/installer-format/src/graph.rs)）。因此对显示文本做容错解码不会放宽安装授权，也不会改变实际写入的文件集合。

## 5. 为什么既有审核与验收未发现

| 时间 | 事实 |
| --- | --- |
| 2026-10-05 | VBF v5 审核用 `sf95_audit.audit(data)`（默认 `metadata_inspector=None`，只解析文件表/PE overlay），事件由 `vbf_events.py` 按帧扫描；审核脚本 `vbf_audit.py` 从未调用完整图解码，因此从未解码该版权字符串，也未发现 `0xA9`。 |
| 2026-10-07 | 提交 `50befc3` 引入按安装器行为的通用路线与 `sf9_graph.py` 完整图解码，加入严格 `exe_display_encoding_unsupported`；Python 与后来的 Rust 都只支持 GB18030 ANSI 显示文本。 |
| 2026-10-09～19 | Rust 移植 `d88d223` 及后续提交复刻同一假设；冻结 golden 由 Python 合成用例生成（ASCII/中文文本），真实样本对比脚本 [verify_sf_rust_samples.py](../../../scripts/verify_sf_rust_samples.py) 只覆盖 AppID 2303800/3606360/3721970 三个样本，没有 VBF/夏日真实字节。 |
| 2026-10-19（v0.2.0） | 通用 EXE 安装路线每次安装都先执行 `analyze_behavior` 完整图解析；VBF v5 因此必然在分析阶段失败。 |

结论：既往“VBF 真实安装/恢复通过”记录属于旧的 profile 管线，并不覆盖当前“完整图解码 + 通用行为判定”路线；该补丁从未在这条路线下通过验收，这是一处被验收矩阵遗漏的真实兼容缺口，不是本次下载或设备环境问题。

## 6. 影响范围

- `VenusBlood FRONTIER International`（AppID 1189440）的 Kagura CN v5 EXE 补丁在 v0.2.0 的通用 EXE 安装路线下无法安装；Windows 与 Steam Deck 共用同一 Rust 解析/worker，行为一致。
- 任何 Setup Factory 9.5–9.7 安装包，只要 ANSI 显示文本中出现非 GB18030 字节（西文 `©`、日文 CP932、其他本地化模板等），都会以相同方式失败；影响不限于 VBF。
- 失败在下载完成后、安装计划与游戏写入之前发生，属安全失败：不会留下半安装状态；已下载的补丁缓存仍保留，修复后重试不必重新下载。
- 次要问题：`exe_analysis_failed` 的详情直接透出 worker 的 `[worker]` 阶段前缀，用户看到的是内部错误分层，不是可读原因。

## 7. 显示文本解码的必要性

按层拆分，结论是：**安装结果不依赖显示文本的“解码成功”，但字段字节仍必须读取。** 修复前的严格阻断属于超出安装需求的防御。

| 层 | 是否必须 | 依据 |
| --- | --- | --- |
| 结构解析与偏移推进 | 必须 | `display` 字段有独立的长度前缀；必须读出长度并跳过对应字节，否则无法定位后续对象。删除整段读取不可行。 |
| 安装条件、载荷选择与写入 | 不需要 | 安装决策只读取文件表、目标路径、`condition`、`packages` 和事件/include 的脚本文本，这些都不是显示字段；显示文本不进入条件求值、载荷选择或写入集合。 |
| `variables` / `terms` 证据 | 当前使用，但非授权 | 核心只对两者做报告形状校验，不读取其内容做判断；`terms` 仅参与 `behavior_sha256` 的完整性绑定。 |
| 报告与审计证据 | 当前使用 | 报告 schema 要求 `variables`、`terms` 字段；Python/Rust 对等与 golden 依赖其内容。 |

因此：

- 把“编码有效性”当作安装准入条件没有依据：元数据完整性已由 overlay CRC 校验，结构完整性由长度/对象图校验覆盖；显示文本编码错误并不能说明安装包不可信或不可安装。
- 真正必要的只是“有界读取并推进偏移”；解码成合法字符串可以失败，但不应该阻断安装。修复前的严格失败就是真实误杀；编码回退方案曾实施，但按用户决定已回退，改为跳过解码（见第 8 节与计划）。
- 更彻底的做法（安装路径完全不产出显示文本）已经是当前选定方向：跳过解码并删除证据字段，见第 8 节与[跳过显示文本解码计划](skip-display-decoding-plan.md)。

### 7.1 解码后的显示文本实际流向

| 去向 | 用户可见 | 说明 |
| --- | --- | --- |
| 屏幕/控件/消息/外观显示字符串 | 否 | 解析后丢弃，只用于推进偏移与验证结构 |
| `%ProductName%` 等项目变量（`variables`） | 否 | 进入 `script_inventory`，仅写入分析报告 |
| 许可协议正文（`terms`） | 否 | 进入 `script_inventory` 与 `behavior.terms`，参与 `behavior_sha256` 完整性绑定 |
| 分析报告 `reports/<content_id>/report.json` | 否 | 落盘为可清理证据；当前代码只写不读 |
| Decky / Windows 前端 | 否 | 前端不读取 `script_inventory`、`variables` 或 `terms` |

所以“显示文本”指**安装器自身的界面文案与项目元数据**，不是本工具的界面；工具的补丁名/来源/状态来自来源目录与事务，不来自这里。当前解码的产品价值为零，这些证据字段也没有消费者。因此下一步直接删除解码与证据字段，而不是继续维护编码回退；影响面见[跳过显示文本解码计划](skip-display-decoding-plan.md)。

## 8. 后续方向（待实施）

1. **放弃编码回退**：用户确认显示文本从不显示、不参与安装，回退链没有业务价值；此前方案 A 的 Rust/Python 修改已全部回退，当前源码保持严格解码。
2. **改为跳过解码**：安装路径只读取并跳过显示字段；删除 `variables`/`terms` 证据字段、显示编码函数、Python 对应实现及不再使用的 `encoding_rs` 依赖；报告/行为版本升级，并同步 Python oracle、golden、报告校验器。
3. **验收**：VBF v5 真实样本分析成功且报告不含显示文本；现有四款样本与夏日回归、Rust/Python 对等；Windows/Deck 实际安装与恢复通过。详见[跳过显示文本解码计划](skip-display-decoding-plan.md)。

## 9. 证据与边界

- 复现材料：上述样本、`vbf-irsetup.dat`、Rust worker 实际退出码与 stdout；可重跑脚本与结果保存于忽略目录 `.artifacts/vbf-display-encoding-20261019/`（`repro.py`、`result.json`），本地临时复现目录不进入仓库。
- 调查手段：只读解析与 worker 分析，没有执行安装器/EXE、没有运行 Lua/安装脚本、没有写入游戏目录、没有修改代码。
- 边界：本次调查证明 v5 样本在当前源码下的确定性失败与根因；此前编码回退代码已全部回退，当前源码不含本次功能改动；跳过解码计划尚未实施。若 Kagura 后续发布新版本补丁，需重新核对样本摘要与元数据。
- 历史事实保留：2026-10-05 的审核/安装记录与本次结论不矛盾，但不能再作为通用 EXE 路线可安装 VBF 的证据。

## 10. 方向调整记录（2026-10-19）

- 方案 A（GB18030 → Windows-1252 → Latin-1 回退）曾按用户要求实施，并通过 Python 测试与真实 VBF 样本的 Python 端到端分析。
- 随后用户确认显示文本从不显示、也没有安装用途，决定直接跳过解码。已回退全部相关代码改动：`crates/installer-format/src/graph.rs`、`scripts/sf9_graph.py`、`scripts/test_sf95_audit.py` 恢复原状，工作区不再保留编码回退实现。
- 新方向见[跳过显示文本解码计划](skip-display-decoding-plan.md)；计划未实施，当前源码仍会在显示文本编码失败时阻断分析。方案 A 的实现细节不再作为交付内容。
