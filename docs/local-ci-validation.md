# 本地 CI 验证

以当前 [workflow](../.github/workflows/ci.yml) 的实际 `run`、`if`、`env`、工作目录和工具版本为准。本指南提供执行入口与逐步骤核对方法，不维护另一套替代 CI 的命令清单。

## 触发范围

- 完整对齐只在发布版本前执行：打 tag、创建或核验发行附件之前，用来确认发行输入与当前 workflow 一致。日常功能开发、修复和文档改动不跑本指南的完整清单，避免拖慢开发。
- 日常开发按改动范围做定向验证：受影响包的单元/集成测试；改动 Rust 代码执行 `cargo fmt --all -- --check` 与受影响包的 Clippy（保留 `-D warnings`）；改动共享契约、锁文件或生成物时核对生成结果。
- 排查线上 CI 失败时只重跑失败 job 的失败步骤及其必要前置，不扩成三 job 全矩阵；失败原因确实需要跨平台确认时再补对应平台，并写明理由。
- 用户明确要求完整对齐时按其指示执行。

## 确定范围

1. 用 `git diff --name-only <base>` 和 `git ls-files --others --exclude-standard` 收集本次发布范围的完整改动；base 取上一版本 tag 或待合并范围的起点，不能只检查最后一个提交。
2. 依据 [select_jobs.py](../.github/ci/select_jobs.py) 的 `select_jobs(paths)` 确定 Core、Windows、Decky；三个 job 的选择相互独立。版本标签和 workflow_dispatch 按 CI 执行全部 job。
3. 完整对齐时先执行 changes 的版本检查、版本单测、CI 选择器单测。`select_jobs.py` 的事件入口依赖 GitHub 事件环境，本地使用实际路径集合调用 `select_jobs`，不伪造线上成功结果。
4. 为选中的每个 job 建立步骤记录，逐项复制当前 workflow 命令执行；不能只写“Rust 测试通过”。新增/修改 workflow 后重新核对本地清单。

## 平台与步骤对应

下表是完整对齐时的覆盖范围，只在发布前（或用户明确要求时）执行：

| CI job | 本地环境 | 必须逐步骤覆盖 |
| --- | --- | --- |
| changes | Python 3.12 | project_version 检查、版本单测、`.github/ci` 单测、实际变更路径选择结果 |
| core / Windows | 原生 Windows，Rust 1.94.0 + rustfmt/clippy，Python 3.12 | 协议测试、fmt、四包 all-targets Clippy、Core/CLI 测试与 build、sf95 audit/worker 单测 |
| core / Linux | WSL Linux，同版 Rust 与 Python | Linux 专属 contract-gen check，以及 Windows Core 相同的全部后续步骤 |
| windows | 原生 Windows，Node 22、Flutter 3.47.7、NSIS、CI 同版 Rust/Python | 六组 Python 单测、npm ci、契约、release core/worker、完整 Flutter 脚本、Cargo metadata、含 Flutter 信息的 SBOM、便携包、NSIS 安装包、锁文件/生成物差异检查 |
| decky | WSL Linux，Node 22、CI 同版 Rust/Python | 四组 Python 单测、npm ci、契约、typecheck、Decky 前端测试、bridge 测试、Linux release core/worker、使用真实 core 的全部后端测试、plugin 打包、metadata/SBOM、发行 ZIP、锁文件/生成物差异检查 |
| ci | 本地验收记录 | 每个选中 job 的每个平台、每个检查均有结果；失败/未执行不能写为通过 |
| artifact upload | 本地产物核验，上传由线上完成 | 核对 workflow 的全部 path、必需文件、包内容、平台及哈希；不能只确认命令退出成功 |
| release | 仅在用户授权发布时 | 按发布技能执行；普通本地验证不创建 Release、不替换附件 |

`scripts/run-windows-check.ps1` 是真实 EXE 样本专项验收，Clippy 仍是可选且包集合不同，不能代替本表中的 Core job。Flutter 脚本正常入口含 analyze/test/build，不传跳过检查参数替代完整 Windows job。

## WSL 入口

从 PowerShell 查询并进入发行版：

```powershell
wsl --list --verbose
wsl -d Ubuntu-22.04 -- bash -lc 'cd /mnt/e/Git/steam-hentai-patch-searcher && python3 --version && cargo --version && node --version'
```

在 Linux 中确认 `command -v cargo rustc python3 node npm` 指向 Linux 工具，使用仓库 rust-toolchain.toml；不要调用挂载目录中的 Windows `.exe`。WSL 的非交互 PATH 可能未载入已有工具链，先定位并配置它，不因 `command not found` 判定没有环境。

Core 使用独立 `CARGO_TARGET_DIR`，例如 `target/wsl-core`；Decky 沿用 CI 的 `CARGO_TARGET_DIR=target/linux`。Decky 后端单测前设置 `PATCHHUB_TEST_CORE` 为本轮 Linux release core 的绝对路径，不能使用 Windows 二进制或跳过真实核心桥接测试。

Decky 的 npm 安装、构建在独立 Linux 工作副本中执行，避免复用/覆盖 Windows node_modules。复制或同步本轮实际源码（包括未提交文件），排除 `.git`、`.tools`、`.artifacts`、target、node_modules 与 Flutter 构建缓存；记录验证副本来源，确认验证的是当前改动而非旧 checkout。不要为了隔离复制用户游戏、存档或生产状态。

## 本机已知约束

以下是 2026-11-03 实测确认的环境差异，处理方式不改变 CI 命令本身：

- WSL 必须用非 root 用户执行。GitHub runner 为非 root；以 root 运行 core 测试会出现 `installer_host_requires_non_root` 及其相关断言失败，这是环境差异而非代码问题。非 root 用户读取 Windows 工作区前按需配置 `git config --global --add safe.directory`。
- 在 `/mnt/e`（drvfs）上以非 root 构建会失败：`mlua-sys` 构建脚本复制头文件被拒（Operation not permitted）。core 与 Decky 改在 ext4 工作副本执行，各自使用独立 `CARGO_TARGET_DIR`（如 `target/wsl-core`、`target/linux`），副本内记录同步来源、提交号与工作区状态。
- Windows 侧 Rust 与 Flutter 位于仓库 `.tools/`，执行前设置 `CARGO_HOME`、`RUSTUP_HOME` 并把 `.tools/cargo/bin`、`.tools/flutter/bin` 加入 `PATH`。
- 本机 Node 可能高于 CI 的 22，NSIS `makensis` 可能缺失。原样记录，不替换 CI 版本要求；缺失的步骤（如安装包生成）标为环境阻塞，留待线上 CI。

## 执行与记录

以下执行与记录要求针对触发完整对齐的场合（发布前，或用户明确要求）：

- 从仓库根目录按 workflow 顺序执行，首个失败步骤停止；日志保存到 `.artifacts/`，命令成功不反复读取完整日志。
- `--locked`、包集合、`--all-targets`、`-D warnings`、环境变量和平台条件全部保留；代码/测试的新改动同样必须经过 fmt 和 Clippy。
- setup/cache 不在本地模拟 GitHub action；检查工具版本与必要环境。WSL 通过只代表本地 Linux 检查通过，不代表 GitHub 镜像、在线上传或 Deck 实机通过。
- workflow 最终的 `git diff --exit-code -- ...` 原样执行，并确认生成物/锁文件在验证前没有未提交漂移。变更本身需要更新生成物时，先审查其预期 diff，再核对验证没有进一步改写，不把已有改动误判为构建污染。

施工记录采用以下粒度，每个 workflow run step 各记一行，长命令可链接证据文件：

| job / platform | step / 完整命令 | 结果 | 日志或产物 | 未验证原因 |
| --- | --- | --- | --- | --- |
| core / WSL Linux | `cargo fmt --all -- --check` | 待执行 | — | — |

已有有效证据可复用，明确其代码范围；仅文档变化不要求无关全量重跑。未完成的步骤必须保留为待验证，不能将“定向通过”写成“本地 CI 全通过”。
