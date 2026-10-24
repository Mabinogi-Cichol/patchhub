# PatchHub 品牌与全面改名施工记录

[设计与改名计划](design.md)

## 当前状态

全面改名已实现；Windows/Linux 构建、自动化、打包和 Windows 隔离原生更新链通过，GitHub 仓库已改名。本轮代码与文档按用户要求分别提交并合并至本地 main；未推送源码、未部署 Steam Deck、未发布新版本；公开在线安装、Windows 系统级安装与 Decky 实机仍待验收。

## 2026-10-25

- 用户选定原拼图造型的深炭黑主体、蓝紫渐变对勾与左侧缺口边带版本，以及对应浅灰首页横幅。
- 从 main 创建 feature/patchhub-branding；原自更新相关未提交改动完整保留，未作为品牌任务提交。
- 替换 docs/images/branding/logo.png、banner.png，以及 Windows assets/branding/logo.png；从相同 PNG 生成 Runner 与 NSIS 共用的 resources/app_icon.ico。
- 建立独立设计、施工记录并更新功能索引和品牌素材说明；README 图片说明同步所选 PatchHub 横幅，正文产品改名留待实施。
- 源素材：Logo 为内置 imagegen 输出 exec-18b1b857-3c76-40da-9a9e-ccccf04f794d.png；横幅为 exec-60e0f8d7-368d-452f-891c-329fb9161558.png。正式素材已复制入仓库，不依赖生成目录运行。
- 自动化文件检查：PNG 可读取，Logo 1254 × 1254 / RGBA，alpha 范围 0–255；横幅 2172 × 724 / RGB；ICO 包含 16/24/32/48/64/128/256 七种尺寸。进一步一致性、链接及差异检查见下方验证记录。
- 已目视检查生成原图和横幅：对勾与缺口边带均为蓝紫渐变，横幅品牌名及三行说明正确。小尺寸 ICO 与真实系统显示待验证。
- 本轮仅素材和文档，不运行完整业务测试；程序运行、系统图标缓存、客户端构建及 Decky 实机均未验证。

### 验证结果

- 正式 Logo 与 Windows PNG 字节一致；PNG 角点和七种 ICO 帧角点均透明，所有帧可解码。
- 本功能两份文档及品牌说明中的本地 Markdown 链接目标均存在。
- `git diff --check` 通过；功能索引原有自更新和 Windows 记录保留，本轮仅新增 PatchHub 行。
- Logo SHA-256：`8f5fad09c1dbb95873e085fdbf1b57dd63fb0208ecbe837017f02f2052089fb6`。

## 2026-10-25：全面改名实施

### 修改范围

- 开始时工作区干净，当前分支已整合自更新及 Windows 修复，基线 `75cc867`。没有将原独立功能重新作为本轮改名实现。
- npm 根包改为 patchhub，Decky/UI 使用 @patchhub/decky、@patchhub/ui；Cargo 六个 crate 及 Rust 导入统一 patchhub-* / patchhub_*，同步锁文件和协议生成器引用；Flutter 包名 patchhub_windows。
- Windows 主程序 patchhub.exe，核心 patchhub-core.exe，worker patchhub-worker.exe；Linux 使用对应无扩展名程序，探针程序亦使用 patchhub 前缀。两端宿主启动路径、方法通道、内部环境变量、日志命名、更新标记与维护锁同步更新。
- Windows 状态目录 %APPDATA%/PatchHub，SteamOS 为 ~/.local/share/patchhub；Decky 安装目录 patchhub、显示名 PatchHub；NSIS 注册项、快捷方式、卸载及更新助手使用单一新身份。
- 删除 Decky 已发布旧名称识别、跨身份卸载/设置名称迁移及专属测试，保留同一 PatchHub 身份下的安装恢复、更新互斥和其他插件设置保护。删除 Windows 旧产品注册表发现和旧名称文件/快捷方式清理，不主动迁移或删除旧用户数据。
- Decky 通过现有 Rollup 图片支持接入正式 PNG，打包为 dist/assets 下的哈希命名图片，不新增依赖。CI 将品牌 Logo 列为两端构建输入，横幅或普通文档变更不因此触发重构建。
- 更新 README、客户端指南、安装说明、构建脚本、CI、SBOM 与打包/更新清单。项目技能当前部署示例更新，技能调用 ID 保留为工具接口名称；旧功能历史、发布证据与旧 Release 内容未改写。
- 原生更新链测试补齐“查看更新 → 下载更新”的实际页面入口；测试此前仍按旧布局直接查找下载按钮，未改变产品更新流程。

### 自动化与构建

| 检查 | 结果 |
| --- | --- |
| npm install --package-lock-only / clean npm ci | 离线安装通过，workspace 新名称链接生效 |
| Decky typecheck / tests / build | 通过；68 项前端测试通过，正式 PNG 已输出 |
| 安装器 JS 桥接 | 9/9 通过，覆盖精确请求确认、写门禁、设置邻居顺序及恢复 |
| scripts Python tests | 96 项运行，31 项因 Windows 缺少 POSIX 条件跳过，其余通过；包括真实 PowerShell 更新助手夹具 |
| Decky backend Python tests | 109 项运行，3 项平台条件跳过，其余通过 |
| CI selector tests / actionlint | 9/9 通过；workflow 静态检查通过 |
| Cargo workspace tests | Windows 原生 --workspace --locked --offline：669 项通过、7 项忽略；不作为实机结果 |
| Cargo clippy / fmt / v5 contract-gen check | 通过；跨语言共享 UI 契约及 Flutter presentation 一致性检查通过 |
| Flutter analyze / test | 无问题；219 项通过、1 项原生 opt-in 测试跳过，随后单独执行该原生测试通过 |
| Windows / Linux Release | patchhub.exe、patchhub-core(.exe)、patchhub-worker(.exe) 构建通过；Linux 核心/CLI 609 项测试通过、7 项忽略 |

沙盒 npm/Python/Rust 检查曾因目录权限或文件锁失败，改在原生环境验证后通过。CMake 旧 target 缓存通过 flutter clean 清理；本机缺少创建符号链接权限，使用项目生成目录中的 junction 指向现有 Pub 插件缓存，没有修改 Windows 开发者模式。Rust fmt 仅调整改名后的导入排序。

### 候选包核验

本地生成 patchhub-windows-portable.zip、patchhub-windows-x64-setup.exe 和 patchhub-decky.zip。Windows ZIP 42 个文件与受管理清单完全一致；Decky ZIP 26 个文件，新身份及 ELF runtime 校验通过，包内 PNG 与选定 Logo 字节一致。SBOM 根据当前 Cargo/npm/Dart 锁文件重建，相关校验通过。

完整文件大小与 SHA-256 见 `.artifacts/patchhub-branding/package-verification.json`。本地候选包未上传，不修改旧版本附件。

### Windows 原生验证

- 隔离 APPDATA 和程序副本，本地 HTTPS Release 夹具触发 0.2.9 → 0.3.0：UI 下载/校验、用户确认、正常退出请求、真实 PowerShell 文件替换、新 EXE Core v5 能力握手及提交确认通过。
- `last-update-result.json` 为 completed，程序版本为 0.3.0，非受管理 user-note.txt 保留，事务工作目录清理完成。
- 更新后窗口标题实测 PatchHub。首次正常关闭约 915 ms、无本夹具残留进程；枚举得到的进程对象未保留退出码句柄，因此该次不宣称已读出 exit code。
- 补充由测试直接创建的子进程正常关闭测量：标题 PatchHub、exit 0、1219 ms；未使用强杀。Windows 系统级安装/卸载注册项及真实桌面图标缓存仍待验收。
- 最终进程复查无本夹具 GUI/Core/worker 残留；Windows EXE 的 ProductName、FileDescription 均为 PatchHub，OriginalFilename 为 patchhub.exe。

### 远端与入口

- GitHub 仓库实际改为 [Mabinogi-Cichol/patchhub](https://github.com/Mabinogi-Cichol/patchhub)，仓库 ID 1398362578 不变，简介更新；origin 同步为新 HTTPS URL，ls-remote 成功。
- 改名前后核对所有当前历史 Release 的 ID、Tag、标题、附件 ID/名称，全部一致。没有移动 Tag 或修改旧附件。
- 当前 README 和 Decky 安装文档使用完整 raw 新地址，移除旧短链接推荐。新源码尚未推送，正式 Release 仍为旧命名附件，故公开在线安装/更新尚未验收；需要新命名代码进入 main 并发布新版本后验证，不使用重定向或旧包冒充通过。

### 复核与证据

- [实现与过度设计复核](review.md)：沿宿主启动、维护锁、安装/恢复、清单生成/消费及 Decky 打包路径核查；未增加旧品牌兼容层、通用配置、重试或限制。
- 本地证据目录 `.artifacts/patchhub-branding/`：cargo-test-native.log、linux-build.log、clippy.log、flutter-test.log、flutter-build.log、scripts-tests-native.log、decky-backend.log、ci-selection.log、package-verification.json、native-chain-test.log、native-chain-result.json、windows-smoke.json、remote-renamed.json。

## 待办

- [x] 内部包、程序名称与数据路径改名。
- [x] 安装/打包/CI/自更新清单同步及旧品牌兼容清理。
- [x] Decky 品牌图标接入、两端构建及候选包核验。
- [x] GitHub 仓库实际改名及 origin 同步。
- [x] Windows 隔离原生更新链、标题及正常退出验证。
- [x] 按用户要求分别提交本轮代码与文档，并合并至本地 main。
- [ ] 推送源码，发布首个新命名版本后验证公开入口。
- [ ] Windows 系统级安装/卸载、图标缓存验收。
- [ ] Steam Deck 部署、深色背景图标/真实 RPC/游戏模式与实体手柄验收。


## 2026-10-25：按用户要求合并主线

- 代码独立提交 `1ccb4ae`（feat(branding): 全面统一PatchHub名称与安装更新链路）；文档、技能当前示例与施工记录另作 docs 提交。
- 合并前复核暂存范围与差异，修复 AGENTS.md 中误指向不存在 patchhub-* 技能目录的链接，保留现有技能调用 ID；本地技能链接目标检查通过。
- 沿用本 session 的自动化、构建、包核验与原生更新链结果；没有新增功能代码，不重复运行完整检查。`git diff --check` 通过。
- 从 feature/patchhub-branding 合并到本地 main，保留功能分支与合并历史；一并带入该分支上先前已提交的品牌素材、设计和 Windows 界面修复。
- 本次仅本地提交与合并，不推送、不发布、不部署；合并提交 ID 以 main 的 Git 历史为准。
