# Flutter Windows 客户端

计划与当前阶段见 [Windows 重写施工记录](../../docs/features/windows-ux-redesign/progress.md)。
此工程为 Windows 正式入口，包含任务宿主、来源、安装/恢复与系统/浅色/深色主题。旧 React/Tauri 客户端已退役。

工具链固定在 `.flutter-version`，可从 [Flutter 官方归档](https://docs.flutter.dev/install/archive) 获取。
Windows 需要 Visual Studio 的“使用 C++ 的桌面开发”与 CMake。
未启用开发者模式时，请在管理员 PowerShell 中构建，以满足 Flutter 插件符号链接的权限要求。
仓库本地 SDK 可解压至 `.tools/flutter`，也可使用 PATH 中同版本 SDK。

从仓库根目录执行：

```powershell
# 完整检查并构建携带核心和资源的候选目录；不发布、不安装、不改旧数据。
pwsh -NoProfile -File scripts/build-windows-flutter.ps1

# 只在已有匹配版本的 release CLI/worker 时跳过 Rust 构建。
pwsh -NoProfile -File scripts/build-windows-flutter.ps1 -SkipRust

# 开发模式，资源位置通过显式绝对路径注入，不依赖启动工作目录。
cd apps/windows-flutter
../../.tools/flutter/bin/flutter.bat run -d windows --dart-define=STEAM_PATCH_ROOT=E:/ProjectFiles/Git/steam-hentai-patch-searcher
```

默认继续使用 `%APPDATA%/io.github.steampatch.manager`。迁移验证应先复制旧数据，
通过 `--dart-define=STEAM_PATCH_STATE_DIR=<副本绝对路径>` 使用副本。
不要同时运行两个客户端的来源调度。

```powershell
cd apps/windows-flutter
../../.tools/flutter/bin/flutter.bat analyze
../../.tools/flutter/bin/flutter.bat test
../../.tools/flutter/bin/dart.bat run tool/core_probe.dart <core.exe绝对路径> <状态副本绝对路径> <catalog绝对路径>
```

release 候选目录必须整体保留（包括 Flutter DLL、data、插件、核心、worker、resources），
不能仅复制主 EXE。NSIS 和便携 ZIP 均由根目录 scripts 下的构建与打包脚本生成。
