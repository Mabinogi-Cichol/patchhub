# 应用版本号单一来源调研与方案

- 状态：**已实现，本地版本同步 / 锁验证、双端构建与打包验证通过；在线 CI、实机及正式跨版本更新待验证**。
- 调研基线：`main` 的 `2c2663c`；调研分支：`fix/version-single-source`。
- [施工记录](progress.md)记录实际检查、实验结果与待验证项。

## 目标与范围

升级应用版本时，只人工修改一处配置；Windows、Decky、安装包、自更新与 SBOM 使用同一应用版本。保留各工具必须存在的清单和锁文件，由程序同步，不能继续要求维护者凭清单逐项修改。

调研阶段未修改产品，用户随后授权开始施工；现已实现以下版本来源与派生流程，应用主版本仍为 `0.4.0`。协议版本、数据格式版本、依赖版本、历史发行文档和新旧版本测试夹具不随应用版本同步。

## 调研基线事实

以下描述基线 `2c2663c` 的原有行为，实施后的变化见后文。

### 手动维护点

| 位置 | 当前内容 | 用途与现状 |
| --- | --- | --- |
| [Cargo.toml](../../../Cargo.toml) 的 `workspace.package.version` | `0.4.0` | 六个 Rust crate 均通过 `version.workspace = true` 继承；这一部分已经收敛 |
| [Decky package.json](../../../apps/decky/package.json) | 包版本 `0.4.0`、内部 UI 依赖 `0.4.0` | 后端自更新读取包版本；打包直接复制这个文件 |
| [UI package.json](../../../packages/ui/package.json) | `0.4.0` | 私有工作区包，与 Decky 的精确依赖需要一起同步 |
| [Flutter pubspec](../../../apps/windows-flutter/pubspec.yaml) | `0.4.0+1` | Flutter 将应用版本和构建号传给 Windows CMake / RC，生成 EXE 版本资源 |
| [SBOM 脚本](../../../scripts/build-sbom.py) | 硬编码应用版本 `0.4.0` | `metadata.component.version` 不从 Cargo 清单读取；依赖组件版本则从实际元数据和锁文件读取 |
| [NSIS 脚本](../../../scripts/windows-installer.nsi) | 默认 `VERSION = 0.4.0` | 正常 PowerShell 入口会覆盖该值，但仍存在一个容易过期的字面量 |
| [Cargo.lock](../../../Cargo.lock) | 六个工作区包版本 | 派生数据，随工作区版本变化更新；同名数字的外部依赖不能一起替换 |
| [package-lock.json](../../../package-lock.json) | Decky / UI 包版本及内部依赖 | 派生数据，与两个 npm 清单一致 |

Git 提交 `16a1143`（统一为 0.4.0）实际修改了上述 **8 个文件、16 处版本值**。根 `package.json` 当前没有应用版本；[plugin.json](../../../apps/decky/plugin.json) 的 `api_version = 1` 是 Decky API 版本，不是应用版本。

### 实际消费路径

1. **Windows 文件属性**：pubspec → Flutter 生成配置 → [runner CMake](../../../apps/windows-flutter/windows/runner/CMakeLists.txt) 的 `FLUTTER_VERSION*` → [Runner.rc](../../../apps/windows-flutter/windows/runner/Runner.rc)。RC 中的 `1.0.0` 是缺少宏时的模板默认值，正常构建使用 Flutter 宏；无需每次修改 RC。
2. **Windows 界面与自更新**：[main.dart](../../../apps/windows-flutter/lib/main.dart) 读取 EXE 同目录的 `app-version.json`，不是直接读取 EXE 文件属性。更新启动握手也读取该文件。
3. **Windows 元数据与受管文件清单**：[update_manifest.py](../../../scripts/update_manifest.py) 的 `project_version()` 分别读取 Cargo、Flutter、Decky，要求相等；设置了 Tag 环境时还要求 `GITHUB_REF == refs/tags/v<version>`。随后生成 `app-version.json` / `update-manifest.json`。[Flutter 构建脚本](../../../scripts/build-windows-flutter.ps1) 在预览返回前调用它；Windows ZIP、NSIS 打包也调用它。
4. **Windows 安装程序**：[build-windows-installer.ps1](../../../scripts/build-windows-installer.ps1) 默认从 Cargo 正则取版本，检查其与 `app-version.json` 相等，传入 NSIS `/DVERSION`，写入卸载注册信息的 `DisplayVersion`。
5. **Decky**：[package-plugin.mjs](../../../apps/decky/scripts/package-plugin.mjs) 把源码 `package.json` 原样复制到插件输出目录；[AppUpdate](../../../apps/decky/backend/app_update.py) 从部署目录的该文件读取 `current`。Loader 的插件包版本也来自该清单；不能只改前端显示文本。[安装器](../../../scripts/install-decky.py) 检查下载包版本与所选 Release Tag 一致。
6. **SBOM / 最终 ZIP**：SBOM 应用版本来自脚本字面量；[package-artifacts.py](../../../scripts/package-artifacts.py) 检查依赖锁摘要、安装器和运行时文件，但未检查 SBOM 的应用版本是否等于主版本，Decky 打包也没有调用 Windows 的 `project_version()`。
7. **CI / Release**：[ci.yml](../../../.github/workflows/ci.yml) 从 Tag 创建 Release；Windows 构建间接执行现有 Tag 一致性检查，所以不能说当前完全没有保护。检查发生在 Windows 编译后的元数据生成阶段，未覆盖共享 UI 或 SBOM 应用版本，且依赖 Windows 路径成功执行。CI 尾部的 diff 检查只覆盖部分生成文件，不是完整版本一致性检查。

### 已发生的故障与风险边界

- **确认的历史故障**：2026-10-28 预览候选的 EXE 为 `0.4.0+1`，但两个旁路 JSON 仍为 `0.3.0`，界面因而显示旧版本。原因是 `-Preview` 在元数据生成前返回；`118ba12` 已修复。证据见[原施工记录](../platform-build-entrypoints/progress.md#2026-10-28预览残留旧版本元数据修复)。这证明“只统一源码常量”仍不足以保证产物一致。
- **静态可确认的遗漏面**：若只忘改 SBOM 字面量，现有打包校验不会因 SBOM 应用版本不符而拒绝；若只忘改 UI 包版本，它也不在 `project_version()` 的比较集合内。这里描述的是当前检查覆盖，不声称这些情况已在线上发生。
- 用户提到的漏改问题不能全部归因于同一个历史故障；本轮没有证据证明某个已公开 Release 的程序版本错误。

## 实施方案：复用 Cargo.toml 作为唯一来源

唯一人工维护入口为根配置的现有字段：

```toml
[workspace.package]
version = "0.4.0"
```

不新增 `version.json` / `VERSION`。Cargo 原生继承要求清单里存在版本值；另建主配置会使 Cargo 本身也变成必须预生成的副本，增加一个同步层，却不减少其他工具的格式约束。

“只改一处”指**只人工编辑一个版本字段**，不是 Git 最终只变化一个文件。npm / Flutter 的原生清单与锁文件继续纳入 Git；标准构建自动更新派生字段，版本提交包含这些生成差异。只提交主配置、未运行任何同步或构建的提交应由 CI 明确指出尚未同步。

### 一个小型版本脚本

已新增 [project_version.py](../../../scripts/project_version.py)，提供读取主版本、`--sync` 同步派生字段、`--check` 只读检查三种用途；使用已有 Python 3.11+ 环境和标准库 `tomllib` / `json`，不引入通用配置框架或第三方版本管理器。

- 读取必须明确访问 `workspace.package.version`，替换各处“文件第一条 version”的正则提取。
- 同步 Decky 包版本、Decky 对 UI 的依赖版本、UI 包版本、Flutter 应用版本，以及两个锁文件中项目自身的对应元数据。
- 同步时只修改这些已知字段，不对全仓库或整个锁文件做字符串替换。Cargo 工作区包集合取自成员清单；外部依赖版本、解析结果与摘要保持原值。TOML 写入应保留其他内容与注释，不引入全文件重排。
- 锁文件已采用定点同步项目自身元数据，不在每次预览时运行依赖解析器。这样 Decky 的 Windows 入口也不必为了修改 Cargo.lock 而先安装 Windows Rust；其 Linux 构建仍使用现有 WSL 工具链。
- 只读检查报告实际文件、期望值与当前值；有差异时失败。缺少或无效主版本直接报错，不猜测版本、不回退旧值。
- 保留普通版本 / 预发行版的 Flutter 构建号语义 `+1`；主版本已有 `+metadata` 时原样传递，不重复追加。Flutter 保留完整版本字符串，纯数字 build 对应 Windows 第四段，复杂 metadata 按 Flutter SDK 规则对应数字 `0`。不引入按日期、Git 提交数或 CI run 编号自动升版，不添加额外版本硬限制。完整预发行安装 / 自更新仍须正式验收。

锁文件定点同步已实现；隔离副本只修改 Cargo 主版本为 `0.4.1` 后，同步输出通过实际 Cargo 完整 `metadata --locked --offline` 和 npm `ci --dry-run --ignore-scripts --offline` 检查。自动化比较确认外部依赖未变化；这不是任意未来依赖组合的通用保证。

### 构建与运行时接入

| 入口或产物 | 已实现调整 |
| --- | --- |
| `build.bat` / `build-decky.bat` | 在 `npm ci`、Cargo `--locked` 之前自动同步，避免还没进入客户端脚本就因旧锁失败 |
| `build-windows.bat` → Flutter PS1 | PS1 在 Rust / Flutter 构建之前同步；预览仍在返回前刷新两个版本 JSON，不新增预览专属版本逻辑 |
| `scripts/build-linux.sh` | 在 Rust 命令前接入同步，使直接使用该受支持入口也不会遗漏 |
| Decky npm build / package | 在读取清单、编译或复制包清单前调用同一同步入口；覆盖独立 `npm run build` 与直接暂存路径，不只覆盖外层 BAT |
| `update_manifest.py` | 从公共模块读取版本，移除自己维护的三份版本源提取；继续生成现有两份 JSON 和文件清单 |
| SBOM | 应用版本从公共读取函数获取，删除字面量；依赖组件和格式版本仍按原语义获取 |
| NSIS | PowerShell 从公共入口取版本并传参；删除 NSIS 的固定应用版本默认值，缺少调用契约参数时编译明确报错。保留对外 `-Version`，仅允许与主版本相等，不形成另一个覆盖来源 |
| 最终打包 | 在现有打包边界核对 SBOM 应用版本、Decky 已暂存 package 版本与主版本，防止复用旧暂存目录；保留 Windows 安装器的版本一致性检查 |
| 客户端运行时 | 继续读取各自包内元数据；不要求安装目录附带源码 Cargo.toml，不改 Core RPC 或自更新状态机 |

直接运行底层 `cargo build --locked`、`flutter build windows` 无法替所有其他生态执行版本同步；文档应说明此类低层命令前先同步。正式支持的构建入口自动接入，不通过 SDK 包装器、Git 全局钩子或 Cargo build.rs 修改源码树来拦截所有命令。

Windows PS1 复用所选 `python` / `py -3` 执行同步与元数据生成。Node 入口优先使用 Windows `python`，仅在解释器不存在（ENOENT）时使用 `py -3`；脚本失败直接传播，不以重试隐藏版本错误。这保留既有 BAT 支持的 py-only 环境。Linux 使用 `python3`。

### CI 与发布约束

- 在依赖安装和编译前执行只读版本检查，Tag 运行同时核对 `v<主版本>`。复用现有任务，不新增一组专用构建任务；把 Windows 的 Tag 检查语义前移，避免复制两套实现。
- 新公共脚本纳入[CI 输入选择](../../../.github/ci/select_jobs.py)的三端规则。`Cargo.toml` 本来已触发所有平台，不需要新版本文件的额外规则。
- CI 检查应覆盖所有派生清单和项目锁字段；正常构建后再次使用已有 diff 检查确认没有遗漏生成差异，不让 CI 静默修好未提交版本。
- 已更新发行检查清单、构建说明及发布技能，将原有 Windows package.json / Tauri 的过期操作指引替换为唯一来源、同步和产物核验步骤。
- Tag 只验证版本，不反向改写版本文件；不自动提交、创建 Tag、推送或发布。

## 实施拆分与验收

1. 实现公共脚本，完成定点同步和检查；保留现有清单格式，验证重复同步无差异。
2. 接入 Windows / Decky / Linux 构建入口、SBOM、NSIS 与最终打包；清理旧字面量和重复读取逻辑。
3. 接入 CI 和文档；代码与文档按仓库规则分开提交。

必要验证：

- 在隔离副本只改主版本，标准构建入口自动同步全部派生值，第二次同步不产生变化；外部依赖未变化。
- 实际 Cargo `--locked` 元数据 / 构建与 npm 锁一致性检查通过；不能用 `cargo metadata --no-deps` 当作锁文件刷新或完整一致性证据。
- 仅改主版本或破坏一个派生字段时，只读检查列出差异；错误 Tag 在编译前失败。
- 模拟已有旧暂存目录：Windows 元数据正常刷新，旧 Decky package / SBOM 在最终打包边界被识别。
- Windows 预览和完整构建的 EXE 资源、界面元数据、更新清单、NSIS、SBOM 应用版本按既定格式一致；Decky 包清单、Loader / 后端读取版本与 SBOM 一致。
- 覆盖普通正式版及现有预发行规则，明确验证 Flutter 构建号映射；不改写历史版本和比较测试中的新旧版本样本。

实际执行结果见[施工记录](progress.md)：自动化与本地构建 / 打包已验证，在线 CI、真实 Loader / 界面部署及正式跨版本更新仍待验证，不用本地包或夹具替代实机验收。

## 过度设计审阅

结论：**实现已按实际执行路径复核，独立审阅提出的 Python 启动器兼容和 PowerShell 参数类型问题已修正，最终未发现需修改项**。

- 单一来源解决实际多点维护；入口同步解决已经发生过的旁路元数据遗漏。保留格式必需的派生文件是工具约束，无需为了“只有一个文件”删除所有版本字段。
- Tag、锁字段及最终包版本检查有明确边界：防止来源不一致、复用旧包和 SBOM 描述错误。复用现有检查并集中读取，不在每层重复检查全仓库。
- 不新增发布状态机、版本数据库、定时同步、重试、全局 Git 钩子、动态版本服务、自动推送或兼容旧配置路径。
- 当前决定保留私有 UI 包版本并自动同步，避免同时开展内部包版本策略迁移；无需更换 npm 工作区协议。
