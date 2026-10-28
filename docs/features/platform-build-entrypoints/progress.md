# 分平台 BAT 构建入口施工记录

设计：[design.md](design.md)。

## 状态

已实现，入口与模拟流水线验证、Windows 快速预览实际构建通过；完整 Windows / Decky 发行构建待验证。

## 2026-10-28：预览残留旧版本元数据修复

- 用户发现新候选界面仍显示 0.3.0。核对 EXE 文件版本为 0.4.0+1，但同目录 `app-version.json` 和 `update-manifest.json` 仍为 0.3.0；`main.dart` 使用前者显示版本并判断客户端更新。
- 根因：Flutter 编译不更新这两个旁路文件；`-Preview` 在调用现有元数据生成器前直接返回。本轮直接 Flutter 编译候选也未补该步骤。
- 将原有 `update_manifest.py` 调用移到预览返回之前，预览与完整构建均使用现有版本统一检查和元数据生成，无新增版本常量、兜底或配置。
- 当前候选目录已刷新：界面元数据与本地更新清单均为 0.4.0，EXE 产品版本仍为 0.4.0+1；无需重新编译 EXE。未关闭用户正在运行的实例，旧进程需要退出后重新打开才会重读版本。
- 验证：PowerShell 语法解析与工作区 diff 检查通过；带空格 SDK 路径的隔离夹具实际执行修改后的 `-SkipRust -Preview` 脚本（Flutter 为桩命令），确认残留 0.3.0 的两个元数据文件均更新为 0.4.0。证据 `.artifacts/preview-version-metadata/result.json`。该夹具证明预览控制流与元数据写入，不冒充一次新的 Flutter 编译。
- 过度设计复核：只移动现有生成器调用，未引入第二份版本来源、兼容分支或额外配置。未重跑无关的产品测试、完整发行或设备操作。
- 按用户要求创建本地代码提交 `118ba12`（`fix(build): 修复预览构建遗留旧版本元数据`）；本节及设计修订另作文档提交。未合并或推送。

## 2026-10-27

- 按用户指示直接在当前工作区修改，未创建分支。
- 新增 Windows 转发入口、Decky WSL 构建与发行打包入口；保留原 `build.bat`。
- 新增 README 使用说明与功能索引。
- 过度设计静态复核：Windows 复用现有入口，Decky 复用已有 Linux、前端与打包脚本；未引入新抽象、硬限制、超时或重试。未发现需修改项。
- 入口验证：两个 BAT 的帮助命令返回 0、未知参数返回 2；从仓库外目录调用同样通过，Windows 参数转发正常。
- Decky 模拟流水线：在带空格的夹具目录替换 npm、Python 与 WSL 为桩命令（WSL 的 exe 调用替换为 call cmd），确认 8 次依赖检查/构建/打包调用顺序、目录定位和安装脚本复制；npm 失败返回 23 时立即停止并保留退出码。初次夹具检查暴露参数 shift 后 `%~dp0` 改变，已改为解析前保存根目录；Windows 转发入口也先切换至根目录。
- 模拟结果仅证明 BAT 控制流，不证明真实工具链、WSL 参数执行及产物构建通过。夹具和日志位于忽略目录 `.artifacts/build-entrypoint-smoke path/`。
- `git diff --check` 通过。未执行完整构建、设备部署或实机验收。

### Windows 快速预览调整

- 用户明确只需快速查看修改后的版本，将 `build-windows.bat` 从完整发行转发改为调用 `build-windows-flutter.ps1 -Preview`，成功后打开本地 `patchhub.exe`。
- 预览仍增量构建 Rust 核心/worker 和 Flutter Release，复制运行所需资源；跳过 analyze/test、依赖清单采集、更新清单与发行打包。原 `build.bat` 默认完整发行路径保留。
- 入口帮助和未知参数退出码通过；PowerShell 语法解析通过；带空格夹具中的模拟构建返回 23 时立即停止，不启动应用。
- 过度设计复核：仅增加一个对应实际预览需求的模式开关，复用已有构建和资源复制路径，没有默认跳过核心编译而产生旧核心的风险；没有增加缓存判定、重试或额外限制。未发现需修改项。
- 真实预览构建通过（退出 0）：Rust release 增量编译约 40 秒，Flutter Windows 构建约 44.7 秒（不含工具检查与依赖解析）；产物为 `apps/windows-flutter/build/windows/x64/runner/Release/patchhub.exe`。首次沙盒运行在 Flutter 用户目录遥测文件写入处失败，使用获准的沙盒外构建完成验证。
- 运行目录的应用、Flutter DLL、Flutter 资产及 catalog/trust-keys 文件存在；复制的核心/worker 与本次 `target/release` 二进制 SHA-256 一致。尚未执行界面交互验收，本轮不运行测试和完整发行构建。
- 按用户要求合并至本地 `main`，未推送；合并无冲突且不新增实现改动，沿用上述验证结果。
- 2026-10-27 随 v0.4.0 推送至远端 `main` 并发布（入口为开发工具，不随安装包分发；见[发行检查](../../release-checklist.md#v040-发布记录2026-10-27)）。
