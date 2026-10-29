# 订阅源显示名称与规则示例施工记录

设计见 [design.md](design.md)。

## 状态

已实现，相关本地自动化通过并完成本地提交收口；Windows 原生窗口、Steam Deck 实机及用户体验验收待执行。开发分支：`feature/source-display-names`；未合并或部署。

## 2026-10-29

- 已确认现有两端仅支持来源启用/订阅/更新间隔设置，没有名称写入接口。
- 方案审阅：使用现有设置文件的名称覆盖及单次保存接口，不复制内置源、不增加迁移、重试或任意长度上限。空白名称拒绝与未知源检查对应真实业务边界。
- Rust 设置新增默认空的 `names` 字典，旧设置无需迁移；读模型应用自定义名称，内部规则保持不变。内置 `patchmoe` 默认名称改为 Shiravune，抓取/下载地址和源 ID 保留。Kagura 国内/海外与 HGamer 索引名称保留。
- 完成 `set_source_name` 协议、CLI `--name` 参数、生成契约与 Flutter schema 资源同步，以及 Decky Python RPC 转发。
- 两端展开卡片内提供“来源名称”与“保存名称”，空白不可保存，失败保留草稿。Decky 名称保存不清空匹配结果，游戏订阅页使用最新全局名称。
- 名称保存失败回归发现 Windows 错误 `SelectableText` 的内部滚动位置与源卡片展开布尔值共用 PageStorage 路径，触发 `bool is not a subtype of double?`。为保存错误文本增加独立键后，成功/失败与展开收起回归全部通过。

## 自动化验证

- `cargo test --locked -p patchhub-core -p patchhub-cli -p patchhub-protocol --lib --bins`：核心 373 passed / 5 ignored，CLI 58 passed，协议 2 passed。
- `cargo test --locked -p patchhub-protocol --test fixtures`：3 passed；新增名称方法已进入跨语言请求/结果夹具。
- `cargo test --locked -p patchhub-core --test source_data_v2`：11 passed；覆盖设置结构与迁移兼容。
- `cargo run --locked -p patchhub-contract-gen -- check`：通过。
- Rust 名称专项覆盖内置/添加/补查源、首尾空白、超过旧规则名称限制的合法显示名称、未知源/空白拒绝不写入、重开、订阅/启用/间隔保持，以及自定义名称下内置规则升级仍生效。
- `npm run typecheck`、`npm run test:contracts`：通过；Decky 原有 80 项前端测试通过，新增 2 项界面回归后 `game-manager.test.ts` 全部 9 项通过；`npm run build:decky` 通过。
- `python -m unittest discover -s apps/decky/backend -p 'test_source*.py'`：30 passed，包含名称 RPC 转发、快照刷新、失败保持和现有安装忙碌约束。
- `flutter analyze --no-pub`：通过。Flutter wire 契约夹具通过；最终来源页面/模型定向回归 8 passed，包含名称成功/失败与深浅主题、1.0/1.5 字体的展开和滚动。
- `git diff --check`：通过。
- 首次沙箱执行 Rust 临时文件锁失败、Python 异步测试停滞、Flutter 工具日志访问被拒绝；使用正常工具权限重跑后上述检查通过，未增加产品侧绕过逻辑。首次 Flutter 名称回归同时暴露测试标题定位冲突和真实错误文本键冲突，修正后重跑通过。
- 用户随后要求构建 Windows：`pwsh -NoProfile -File scripts/build-windows-flutter.ps1 -Preview` 通过，完成 Rust Release 与 Flutter Windows Release 构建，沿用本 session 已有验证记录。输出 `apps/windows-flutter/build/windows/x64/runner/Release/patchhub.exe`，版本元数据保持 `0.4.0`。
- Windows 构建产物检查通过：核心/worker 与 `target/release` SHA-256 一致，程序配套文件与打包 schema 齐全；用新构建核心及隔离临时状态验证名称方法、默认 Shiravune、改名后新进程读取保持。证据：`.artifacts/windows-source-display-names/build-smoke.json`。未启动原生界面或替换已安装客户端。

## 实现过度设计审阅

结论：未发现需继续修改项。范围为本分支的设置持久化、来源读模型、协议/桥接与两端名称保存路径。只复用现有设置版本及写入机制，没有复制内置规则或新增迁移/重试框架；检查仅保留源存在及名称非空的业务边界。用户改名不进入规则指纹，不触发来源抓取，也不丢弃既有匹配结果。保存失败明确展示并保留输入，Windows 错误显示键冲突已由回归验证关闭。

## 添加规则示例补充（2026-10-29）

- 用户要求在添加来源规则旁增加示例；Windows 标题旁增加示例对话框，Decky 添加按钮旁增加展开示例，均包含通用表格 JSON、占位字段说明及复制按钮，保留现有输入。
- 方案与实现精简审阅：仅使用静态示例与现有界面/剪贴板能力，无新增协议、适配器或自动添加流程。
- 本轮增量验证：Flutter 静态分析通过，来源界面/模型 10 项通过（含深浅主题、580×560 窗口与 1.5 字体下查看/复制示例、保留输入）；Decky 类型检查、相关界面 10 项及构建通过。示例 JSON 已由生产核心在隔离临时状态中执行 `add_source` 校验通过，未访问占位网站。
- 实现审阅未发现需继续修改项：只有静态示例、显式复制及必要的复制失败反馈；示例查看不写入规则、不改动草稿、不触发网络请求。
- `scripts/build-windows-flutter.ps1 -Preview -SkipRust` 重新构建 Windows Release 通过。Rust 未改动，复用本 session 产物；AOT 界面文件晚于源码修改，核心/worker 哈希与 `target/release` 一致，schema 资源一致。证据：`.artifacts/windows-source-display-names/rule-example-build.json`。仍输出原 Release 目录，未替换已安装客户端。

## 本地提交收口（2026-10-30）

- 按用户“提交 GIT”要求，功能代码分为 `b7e65c3`（订阅源名称编辑与 Shiravune 默认名称）和 `5c97dd9`（来源规则示例）两个提交；同文件的改动已按功能分别暂存，工作区最终代码保持本 session 已验证的内容。
- 设计、施工记录、功能索引与来源使用说明作为独立文档提交。沿用上述自动化和 Windows 构建记录，本轮没有代码修改，不重复运行测试。
- 提交前工作区及各暂存区 `git diff --check` 通过；只创建本地提交，未合并 main、未推送或发布。

## 实机待验收

- Windows 原生窗口的实际键盘输入、保存及重启展示。
- Steam Deck 部署、真实 RPC、游戏模式与实体手柄输入。
- 用户视觉与交互验收。本轮未打包替换已安装客户端，未发布。
