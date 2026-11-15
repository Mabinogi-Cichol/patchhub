# 客户端构建与打包

[设计](design.md) · [施工记录](progress.md)

仓库固定 Rust 1.94，使用 Cargo.lock 和 package-lock.json。共享 TS/Decky 使用 `npm ci`、`npm run typecheck` 与 `npm run build:decky`；Windows 使用固定 Flutter 3.47.7 和 `npm run build:windows`。Flutter 依赖由 `apps/windows-flutter/pubspec.lock` 固定。

## 应用版本

应用版本只人工修改根 `Cargo.toml` 的 `[workspace.package].version`。Windows / Decky 构建入口自动同步 Flutter、Decky、共享 UI 和两个锁文件中的项目版本；SBOM、NSIS 与更新元数据读取同一来源。派生清单仍需随版本变更一同提交，不再逐个手改。构建工具需要 Python 3.11+（Windows 可使用 `python` 或 `py`，Linux 使用 `python3`）。

不构建而仅准备版本变更时，执行 `python scripts/project_version.py --sync`；`python scripts/project_version.py --check` 只读检查同步状态及当前 Tag。直接使用底层 `cargo build --locked`、`npm ci` 或 `flutter build windows` 前也先执行同步；标准 BAT、Flutter PS1、Linux 构建脚本和 Decky npm build / 暂存入口已自动接入。规则和验证边界见[版本方案](../version-single-source/design.md)与[施工记录](../version-single-source/progress.md)。

普通版本 `0.4.0` 派生 Flutter 版本 `0.4.0+1`；预发行版同样保留 `+1`。主版本已有 `+metadata` 时原样传递，不重复追加；Flutter 保留完整字符串，Windows 数字资源中的第四段由 Flutter SDK 映射，复杂非数字 metadata 对应 `0`。协议、依赖、数据格式版本及历史测试中的版本不随应用升版修改。

## Windows 一键构建

准备 Flutter 3.47.7、PowerShell 7、NSIS 3、Node.js 22（共享检查）、Rust 1.94 MSVC 工具链、Python 3.11 或更新版本，以及 Visual Studio 2022 Build Tools 的“使用 C++ 的桌面开发”和 Windows SDK。将 node、npm、cargo、rustc、python（或 py）加入 PATH；已有仓库隔离工具链 `.tools/cargo` 和 `.tools/rustup` 时，脚本优先使用它。首次构建需要联网下载依赖和 NSIS 工具。

双击仓库根目录 `build.bat`；脚本自动切换到仓库目录，执行依赖安装、类型检查、release CLI/worker、Flutter 分析/测试与 release 构建，再生成包含 Rust、Dart/Flutter 的依赖 SBOM、Windows 便携 ZIP 和 NSIS 安装包。失败时显示步骤和非零退出码，窗口保持打开。终端或自动化调用可关闭末尾暂停：

```bat
build.bat --no-pause
```

便携包为 `.artifacts/patchhub-windows-portable.zip`，SHA-256 清单为 `.artifacts/checksums-windows.json`，依赖清单为 `.artifacts/dependencies.cdx.json`。NSIS 安装包位于 `.artifacts/patchhub-windows-x64-setup.exe`。安装包使用 PatchHub 安装目录和卸载条目，卸载保留用户数据。旧品牌需先卸载，再安装 PatchHub，不迁移旧设置。此脚本构建 Windows 客户端；Decky 的 Linux CLI 在 Linux 环境构建。

## GitHub Actions

`.github/workflows/ci.yml` 在 push、Pull Request 和手动触发时运行 Rust 格式、Clippy、测试、桥接测试及打包测试，并在独立 Windows/Linux runner 上构建客户端。Node.js 使用 22，Rust 使用 1.94.0，依赖按仓库锁文件安装。

在 GitHub 的 Actions 页面打开成功运行，下载 `windows-portable`、`windows-nsis` 或 `decky-plugin` Artifact。便携 ZIP 与插件 ZIP 附带对应 SHA-256 校验清单和依赖清单。实机验证结果见各功能施工记录。

## Tag 发布

推送以 `v` 开头的 Tag（例如 `v0.1.0`、`v0.1.0-alpha.1`）会触发构建；只有 core、windows、decky 三个任务全部成功后，才自动创建对应 Tag 的 GitHub Release。分支推送、PR 和手动运行仅构建，不发布。非 `v` 开头的 Tag 不触发此工作流。

先确保要发布的提交包含最新工作流，只修改根 Cargo workspace 的主版本并执行同步，检查生成差异后一并提交。CI 在编译前检查派生字段和 `v<主版本>` Tag，一致性检查不会替提交自动修复版本。Tag 不会自动改写程序内版本。发布步骤及附件核验见[发布技能](../../../.agents/skills/steam-patch-release/SKILL.md)和[发行检查](../../releases/release-checklist.md)。

Tag 名含 `-` 时标记为预发行版，其余标记为正式发行版。Release 公开附件仅包含 Windows 便携 ZIP、NSIS 安装包、Decky ZIP，以及覆盖这三个安装包的 `SHA256SUMS.txt`，供自动更新和在线安装校验。依赖清单保留在安装包内，分平台校验 JSON 保留为 CI 产物；不再单独上传这些 JSON 或 `install-decky.py`。终端一键安装通过仓库 `install.sh` 下载仓库中的 Python 安装脚本。发布说明按“新增与优化”和“问题修复”分类，无变更的分类省略。

发布使用 GitHub 自动提供的 `GITHUB_TOKEN`，仅发布任务授予 `contents: write`，无需配置个人访问令牌。同一 Tag 的发布任务串行执行；失败后可重新运行该次 Actions，若 Release 已存在则覆盖同名附件。已发布 Tag 应保持指向不变；新版本使用新 Tag。

## 手动构建

Windows：执行 `pwsh -NoProfile -File scripts/build-windows-flutter.ps1`，生成完整 `apps/windows-flutter/build/windows/x64/runner/Release/`；再用 `cargo metadata --locked --format-version 1` 生成元数据，执行 `python scripts/build-sbom.py .artifacts/cargo-metadata.json --flutter-deps .artifacts/windows-flutter/pub-deps.json --flutter-version .artifacts/windows-flutter/flutter-version.json`、`python scripts/package-artifacts.py --platform windows` 和 `pwsh -NoProfile -File scripts/build-windows-installer.ps1`。NSIS 可用 `-Makensis` 指定路径。完整解压便携包后运行 `patchhub.exe`，不可单独复制主程序。

Linux：`sh scripts/build-linux.sh` 使用仓库内隔离工具链，测试核心与 CLI，并输出 `target/linux/release/patchhub-core`。然后 `npm run package:plugin -w apps/decky` 生成 plugin-output 目录。安装到 Decky 前应检查二进制可执行权限、文件所属用户、宿主版本和日志；root 宿主启动核心时按游戏用户降权并同步 HOME / XDG，状态存于用户 ~/.local/share/patchhub；商店审核另设门槛。
