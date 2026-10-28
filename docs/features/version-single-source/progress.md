# 应用版本号单一来源施工记录

- [设计与调研](design.md)
- 当前状态：**已实现；版本同步、隔离锁验证、本地双端构建与打包通过；在线 CI、实机和正式跨版本更新待验证**。

## 2026-10-29：提交与合并安排

- 用户明确授权“提交并合并 main，然后推送”。提交前 fetch 核对本地 `main` 与 `origin/main` 均为 `2c2663c`，无分歧。
- 实现及相关测试已独立提交为 `7ef8a9a`（`fix(version): 统一应用版本来源并自动同步双端构建清单`）；调研 / 设计、施工记录、构建与发行指引、发布技能作为同一项文档维护另行提交。
- 沿用本 session 的 43 项通过 / 3 项条件跳过及实际构建验收，不重复运行产品测试；提交时仅检查差异、暂存范围与空白。
- 按授权将开发分支快进合并至 `main`，仅推送 `origin/main`；不修改版本、不创建 Tag 或 Release。在线 CI 结果与实机边界仍需另行验收。

## 2026-10-29：实施与本地验证

用户在调研后授权“开始施工”，继续在 `fix/version-single-source` 分支实施，保留上一阶段未提交文档。主版本仍为 `0.4.0`，没有以本次施工为由升版。

### 实际修改

- 新增 `scripts/project_version.py`：明确读取 `workspace.package.version`，验证 SemVer / Tag；`--sync` 定点更新 Decky / UI / Flutter 清单和 Cargo/npm 项目锁字段，`--check` 只读列出差异。先计算全部修改再写文件；无变化时不重写。外部依赖解析与摘要不参与同步。
- Windows / Decky BAT 在 npm ci 和 Cargo --locked 前同步；Flutter PS1 覆盖预览和完整构建；Linux 构建、Decky npm build 与直接插件暂存接入同一脚本。Windows 入口保持 python / py 支持，Node 仅在 executable ENOENT 时尝试 py，执行失败直接传播。
- Windows 更新清单生成器、SBOM 与 NSIS 读取公共版本，删除重复提取和 SBOM / NSIS 固定应用版本；保留预览返回前刷新两个版本 JSON。NSIS 的外部 `-Version` 只能与主版本相等。
- 最终打包增加 SBOM 应用版本和 Decky 已暂存 package.json 与主版本核对，拒绝旧暂存包；保留既有依赖、运行时、签名来源和受管文件检查。
- CI changes 任务在编译前执行只读检查 / Tag 校验及版本测试，Windows 任务运行含原生入口夹具的版本测试；新脚本纳入三端输入选择，构建后 diff 检查覆盖派生清单。
- 更新构建 / 发行指引和发布技能，移除已退役 Tauri 的升版指令；应用运行时继续读取包内元数据，没有更改 Core RPC、自更新状态机或数据格式版本。

### 已通过验证

- 版本同步与入口 12 项：仅改主版本、派生值 / 外部依赖保持、幂等、只读差异检查、无效 SemVer、错误 Tag、缺失锁条目不产生部分同步、保留 Cargo 注释、实际 CLI、Windows 旧元数据刷新、带空格 SDK 路径的 PS1 预览夹具、Decky 直接暂存旧包刷新均通过。PS1 夹具使用 Flutter 桩；Decky 暂存夹具使用合成 ELF，不冒充实际编译。
- SBOM 1 项、打包 10 项、Flutter SBOM 1 项、CI 选择 9 项通过，合计 **33 项**；旧 Decky package / 旧 SBOM 应用版本在 ZIP 生成前被拒绝。新增边界用例与原有测试均使用隔离项目根目录。
- 原有 Windows 更新回归再通过 10 项，当前 E 盘临时目录不提供 8.3 别名，3 项短路径检查按已有条件跳过；总计 **43 项通过、3 项跳过**。首次沙箱运行被 PowerShell 初始化输出与本机 GBK 解码干扰；改用 UTF-8 并在沙箱外运行隔离夹具后通过，未修改既有更新测试或产品脚本掩盖环境问题。
- 隔离副本只将 Cargo 主版本从 0.4.0 改为 0.4.1，再运行新同步器：实际 `cargo metadata --locked --offline --format-version 1` 完整解析通过，六个工作区组件均为 0.4.1；实际 npm `ci --dry-run --ignore-scripts --offline --no-audit --no-fund` 通过。自动化逐项核对外部依赖条目未变。证据 `.artifacts/version-single-source/locked-validation.json`。
- 实际 `build-windows-flutter.ps1 -SkipRust -Preview` 构建通过，Flutter 3.47.7 生成原生 Release；EXE 文件 / 产品字符串版本 `0.4.0+1`，`app-version.json` 为 `0.4.0`。没有运行 BAT 自动打开候选，也没有停止用户应用。Rust 源码与主版本未变，本次复用已有 Rust Release 二进制。
- 实际 `npm run package:plugin -w apps/decky` 通过：Rollup 前执行同步，直接暂存再次使用同一入口，包内 package.json 为 0.4.0。本轮复用已有 Linux Rust 二进制，没有把该结果说成重新编译了 Linux 核心。
- 实际 Cargo 完整元数据、Decky SBOM（437 组件）、Windows SBOM（505 组件）、两端 ZIP 打包及 Windows NSIS 编译通过。包内应用版本为 0.4.0；Windows Flutter SBOM 原有的 SDK 包许可元数据提示仍存在，未在本次版本任务中扩大处理。
- py-only 兼容性实测：移除 PATH 中的 python.exe 目录、保留实际 py.exe / Node / PowerShell 后，Node 同步入口成功；Windows 预览隔离夹具通过；安装器能读取版本并按预期拒绝外部 `-Version 0.4.1`。
- Flutter 3.47.7 隔离 config-only 实测四种输入：`9.8.7+1` / `9.8.7-rc.1+1` 的数字 build 为 1，`9.8.7+42` 为 42，`9.8.7-rc.1+build.sha` 为 0 且 SDK 给出预期警告，完整字符串均保留。探针副本移除插件列表与 CMake 插件引用以避免本机符号链接权限阻断；实际项目未改，也没有把 config-only 当作四种版本的完整编译 / 安装验收。证据 `.artifacts/version-single-source/flutter-version-mapping.json`。
- 最终两端 ZIP 的 CRC、包内应用元数据 / SBOM 版本通过，Windows 更新清单版本和文件集合与 ZIP 一致；NSIS 编译输出已记录哈希（版本由脚本传入 0.4.0，没有运行安装程序）。证据 `.artifacts/version-single-source/artifact-versions.json`。
- 当前工作区 `project_version.py --check`、Node / PowerShell 语法、相关文档相对文件链接和 `git diff --check` 通过。当前版本与两个源锁文件没有升版差异。

### 实现审阅与修正

- 使用 multi-agent 技能分派 1 个 worker（gpt-6-luna / medium）处理构建入口与 CI；1 个 reviewer（gpt-6.1-sol / medium）只读审阅实现。主 Agent 负责公共版本 / 打包逻辑、测试、集成与文档。
- 独立审阅确认 Node 入口需要保留已有 py-only Windows 契约，随后发现 PowerShell `if` 输出单元素数组会被解包，参数拼接必须显式声明 `string[]`。两项均已修正，并完成上述 py-only 实测；最终复核无待修问题。
- 按 overengineering-review 沿正常 / 错误路径复核：只有工具必需的派生字段和真实版本 / 打包边界检查；没有依赖重解析、版本数据库、全局钩子、重试或自动发布。解释器 ENOENT 分支对应已确认的既有 py-only 支持，不在版本脚本失败时兜底。
- 沙箱内 Flutter 用户遥测文件写入与 NSIS DLL 加载受限，经工具授权在沙箱外完成构建；未改产品以绕过环境限制。测试临时目录统一放在仓库忽略目录，保留实际检查结果。

### 实施验证结束时的遗留与边界

- 在线 CI / Tag 任务未运行；尚未提交、合并、推送、发布或创建新 Tag。
- Steam Deck 部署、真实 Loader / 界面读取、Windows 安装器安装、正式跨版本自更新未执行。本地 ZIP / NSIS 和合成暂存验证不替代这些验收。
- 新增版本机制不拦截 SDK 的所有底层命令；直接 `cargo build --locked`、`npm ci`、`flutter build windows` 前应先同步，标准构建入口已自动接入。

---

## 2026-10-29：版本来源与构建路径调研

以下保留施工前的调研事实与当时待办；当前实现和验证状态以上一节为准。

### 范围与实际变更

- 基线：`main` / `2c2663c`，开始时工作区干净；建立 `fix/version-single-source` 调研分支。
- 读取 Cargo / npm / Flutter 清单、锁文件、Windows 资源链路、双端版本读取、自更新与安装器、SBOM / 打包脚本、CI 和分平台构建入口。
- 新增本目录的设计与施工记录，并更新功能索引；**未修改产品代码、构建行为、版本号或项目锁文件**。
- 未提交、合并、推送、创建 Tag、发布或操作 Steam Deck。

### 确认的结论

- 0.4.0 的版本提交 `16a1143` 修改 8 个文件、16 处值。六个 Rust crate 已共享 Cargo workspace 版本，其他生态和两处脚本字面量仍需同步。
- Windows 已有 Cargo / Flutter / Decky / Tag 检查，位置在 `update_manifest.py`，不是完全没有版本门禁；共享 UI 与 SBOM 应用版本未在该检查内。
- 2026-10-28 旁路 JSON 遗留旧版本的故障有仓库记录与修复提交 `118ba12`，当前预览入口已在返回前刷新元数据。新方案必须保留这一接线，不能以集中配置代替刷新构建产物。
- 建议以根 Cargo.toml 的现有版本字段为唯一人工来源，通过小型脚本同步工具必需的派生字段；不新增第二份主版本配置。

### 已执行验证与边界

1. 静态调用路径和 Git 历史核对完成，包括 Windows UI 与 EXE 资源使用不同读取路径，以及 Decky 复制 / 读取 package.json 的路径。
2. 在 `.artifacts/version-research/` 隔离副本中，将应用清单中的项目版本改为 `0.4.1`，保留原依赖要求和锁；源工作区实际版本仍为 `0.4.0`。
3. `cargo metadata --offline --no-deps --format-version 1` 退出 0，但副本 Cargo.lock 的六个项目版本**仍是 0.4.0**。因此它不能承担锁文件刷新，也不能作为本方案完整锁一致性的证明。
4. 副本运行 `cargo update --workspace --offline` 退出 0，六个项目锁版本变为 0.4.1，逐项比较外部依赖包对象与原锁一致。此结果证明本次工具刷新行为，不是通用依赖不变保证。
5. 副本运行 `npm install --package-lock-only --ignore-scripts --offline --no-audit --no-fund` 退出 0，Decky / UI 的包版本和内部依赖变为 0.4.1；排除这两个工作区条目后，其余锁条目与原文件一致。
6. 实验摘要保存在 `.artifacts/version-research/lock-probe-result.json`（忽略目录）。副本仅保留清单与占位 Rust 源码，**没有编译真实客户端，没有验证定点同步脚本**。
7. 本轮使用本地 Cargo 帮助核对 `update --workspace`，读取本地 Flutter SDK 的 Windows / manifest 版本传递代码；未查外部资料，未进行在线发布核验。
8. 实际调用现有 `project_version()` 返回 `0.4.0`；在同一个隔离 Python 进程中把 `GITHUB_REF` 设为 `refs/tags/v0.4.1` 后，按预期报 `Release Tag differs from application version`。这确认现有 Tag 门禁生效，没有修改进程外环境或构建产物。
9. 当前 Cargo / Decky / UI / 内部 UI 依赖版本均为 `0.4.0`，Flutter 为 `0.4.0+1`；三份本轮相关文档的相对文件链接检查通过，新文档无行尾空白，`git diff --check` 通过。上述版本与锁刷新问题没有被表述为当前主分支已不一致。

首次临时目录写入受本机沙箱 ACL 阻断，后续实验改在仓库已忽略的 `.artifacts/version-research/` 内完成。创建分支的 `.git` 写入经工具授权后成功；未因此扩大产品修改范围。

### 方案审阅

按仓库 overengineering-review 技能完成方案复杂度自审：保留已发生故障对应的产物刷新和真实打包边界检查；统一读取 / 同步，不增加版本服务、重试或全局钩子。当前仅完成方案审阅，实施后的代码路径仍待复核。

### 待办与未验证项

- 待实现：版本读取 / 定点同步 / 只读检查、标准构建入口、SBOM / NSIS 去常量、打包版本检查、CI 及发布指引更新。
- 待验证：定点锁同步输出可被 Cargo / npm 接受且无依赖漂移，幂等性、差异报错、错误 Tag 提前失败、旧暂存产物检测。
- 待验证：正式版 / 预发行与 Flutter 构建号映射；SemVer build metadata 没有新增兼容性承诺。
- 待验证：Windows 预览 / 完整构建与 Decky 最终包的完整版本一致性、真实界面 / Loader 读取；实机部署与正式跨版本更新没有执行。

仅调研和文档检查不需要重跑与本次无代码变更无关的全量产品测试。
