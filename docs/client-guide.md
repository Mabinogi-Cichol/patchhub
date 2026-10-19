# Alpha 客户端与目录维护

Windows 使用 Tauri 2，Decky 使用 @decky/api 和 @decky/ui；两端调用同一个 Rust CLI。核心路径、目录、公钥和状态路径由宿主配置。前端只传入实例、release、受限 operation 与 request ID，不能指定目标目录、核心程序或执行脚本。

## 使用流程

扫描游戏库 → 选择游戏与补丁 → 打开官方来源 / 导入 ZIP → 点击“安装补丁”或“重新安装补丁”一次完成获取、校验与安装。安装后重新扫描文件指纹和持久事务。不提供产品级补丁卸载；新 ZIP/EXE 安装直接覆盖，不创建自动备份或回滚。写入后失败、取消或中断会保留已写内容，必要时重新安装；请自行备份重要文件和存档。历史事务按其原有记录处理。

来源条目仅供引导时，“状态未知”表示没有审核指纹，不能当作未安装。目录签名表示本工具维护端认可这些数据，不代表发行商授权，也不能代替游戏内运行验证。

Decky 手动 ZIP 导入限于用户 Downloads 或本工具下载缓存；Windows 通过原生文件选择器导入。安装无需确认步骤，安装前应关闭游戏并暂停 Steam 更新；有平台、版本或来源阻止条件时禁用安装入口。安装结果按实际事务状态显示，失败或中断不当作成功。Windows 安装时显示实际阶段与字节、文件进度，并提供“取消安装”；Decky 安装保持前台，离开进度界面即请求取消。不要在提交文件时强制杀死宿主。

Decky 点击“安装补丁 / 重新安装补丁”即开始自动获取、校验与安装；维护审核的条目先查 Downloads 和工具缓存，按完整 SHA-256 复用文件，patch.moe 的其他精确匹配游戏保留归档相对路径并解压至游戏根目录，内部按当前 build 和原文件哈希生成本地计划。安装为一次任务，成功后按缓存设置处理下载缓存；新链路不创建 `.stage` 暂存或安装备份。缓存与备份设置可清理下载缓存及历史安装备份，安装失败后的文件按上面的直接覆盖语义处理。下载缓存的前台进度、取消与结果见 [精简安装流程](features/simplified-install-flow/design.md) 与 [Decky UX](features/decky-ux/design.md)。

订阅源的通用安装状态位于 `~/.local/share/steam-patch-manager/installer/rust-v1/`，使用统一 schema、内容存储、任务与收据，不再复用 `publisher/<release>/<instance-hash>/` 旧布局；官方获取缓存默认在安装成功后删除。每个 Steam 库实例隔离，重启插件后安装记录仍可路由。设备上的签名只固定本机文件计划，不宣称逐游戏维护审核或运行兼容性；公开目录的签名、信任数据库与密钥不被修改。插件保持默认非 root 运行；非 ZIP 和需要执行安装程序的来源需另行处理。

## 构建

仓库固定 Rust 1.94，使用 Cargo.lock 和 package-lock.json。先 `npm ci`，再 `npm run typecheck`、`npm run build:windows`、`npm run build:decky`。

### Windows 一键构建

准备 Node.js 22（含 npm）、Rust 1.94 MSVC 工具链、Python 3.11 或更新版本，以及 Visual Studio 2022 Build Tools 的“使用 C++ 的桌面开发”和 Windows SDK。将 node、npm、cargo、rustc、python（或 py）加入 PATH；已有仓库隔离工具链 `.tools/cargo` 和 `.tools/rustup` 时，脚本优先使用它。首次构建需要联网下载依赖和 NSIS 工具。

双击仓库根目录 `build.bat`；脚本自动切换到仓库目录，执行依赖安装、类型检查、release CLI、Tauri 应用与 NSIS 构建，再生成依赖 SBOM 和 Windows 便携 ZIP。失败时显示步骤和非零退出码，窗口保持打开。终端或自动化调用可关闭末尾暂停：

```bat
build.bat --no-pause
```

便携包为 `.artifacts/steam-patch-manager-windows-portable.zip`，SHA-256 清单为 `.artifacts/checksums-windows.json`，依赖清单为 `.artifacts/dependencies.cdx.json`。NSIS 安装包位于 `target/release/bundle/nsis/`。此脚本构建 Windows 客户端；Decky 的 Linux CLI 在 Linux 环境构建。

### GitHub Actions

`.github/workflows/ci.yml` 在 push、Pull Request 和手动触发时运行 Rust 格式、Clippy、测试、桥接测试及打包测试，并在独立 Windows/Linux runner 上构建客户端。Node.js 使用 22，Rust 使用 1.94.0，依赖按仓库锁文件安装。

在 GitHub 的 Actions 页面打开成功运行，下载 `windows-portable`、`windows-nsis` 或 `decky-plugin` Artifact。便携 ZIP 与插件 ZIP 附带对应 SHA-256 校验清单和依赖清单。这些 Alpha 构建仍需客户端与游戏实机验收。

### Tag 发布

推送以 `v` 开头的 Tag（例如 `v0.1.0`、`v0.1.0-alpha.1`）会触发构建；只有 core、windows、decky 三个任务全部成功后，才自动创建对应 Tag 的 GitHub Release。分支推送、PR 和手动运行仅构建，不发布。非 `v` 开头的 Tag 不触发此工作流。

先确保要发布的提交包含最新工作流，并手动将 Cargo workspace、Windows package.json 和 tauri.conf.json 等版本字段更新为计划发布的版本。Tag 不会自动改写程序内版本。将提交推送后，再创建并推送 Tag，例如：

```powershell
git tag -a v0.1.0-alpha.1 -m "Release v0.1.0-alpha.1"
git push origin v0.1.0-alpha.1
```

Tag 名含 `-` 时标记为预发行版，其余标记为正式发行版。Release 自动生成说明，包含 Windows 便携 ZIP、NSIS 安装包、Decky ZIP、各平台校验与依赖清单，以及覆盖所有附件的 `SHA256SUMS.txt`。Windows、Decky 的外部依赖清单分别命名为 `dependencies-windows.cdx.json` 和 `dependencies-decky.cdx.json`，避免同名覆盖。

发布使用 GitHub 自动提供的 `GITHUB_TOKEN`，仅发布任务授予 `contents: write`，无需配置个人访问令牌。同一 Tag 的发布任务串行执行；失败后可重新运行该次 Actions，若 Release 已存在则覆盖同名附件。已发布 Tag 应保持指向不变；新版本使用新 Tag。

### 手动构建

Windows：先构建 `cargo build --release --locked -p steam-patch-cli -p steam-patch-installer-worker --bin steam-patch --bin sf-worker`，将工具链 bin 加入 PATH，再运行 `npm run bundle -w apps/windows -- -- --locked`。prepare-sidecar 会复制同版 CLI、固定 worker 以及三个目录资源，Tauri 生成 NSIS 安装包。开发构建也需准备 sidecar 与资源。不要把私钥放入资源或提交到 Git。

Linux：`sh scripts/build-linux.sh` 使用仓库内隔离工具链，测试核心与 CLI，并输出 `target/linux/release/steam-patch`。然后 `npm run package:plugin -w apps/decky` 生成 plugin-output 目录。安装到 Decky 前应检查二进制可执行权限、文件所属用户、宿主版本和日志；root 宿主启动核心时按游戏用户降权并同步 HOME / XDG，状态存于用户 ~/.local/share/steam-patch-manager；商店审核另设门槛。

## 维护签名目录

`steam-patch keygen PRIVATE PUBLIC KEY_ID` 创建 Ed25519 私钥与公钥目录；文件存在时拒绝覆盖，Unix 私钥文件权限为 0600。Windows 私钥继承目录 ACL，应保存在受限维护目录。

`steam-patch sign-catalog CATALOG PRIVATE KEY_ID OUTPUT LIFETIME_SECONDS [REVOCATIONS_JSON]` 对严格校验的目录生成签名，最长有效期 90 天。签名覆盖原始 payload 字节。可选的撤销文件是 release_id 字符串数组，例如 `["withdrawn-release"]`，写入签名 payload 的 revoked_release_ids；拒绝重复或空 ID。撤销密钥通过升级受信公钥文件实施。生产维护需离线密钥、密钥轮换和审核流程。revision 4 使用维护密钥 alpha-local-2026-10，公钥目录同时保留 alpha-local-2026；私钥不提交或打包。

`steam-patch schema OUTPUT` 导出 payload JSON Schema。现役 IPC 仅支持 v5，必需 host 六字段和显式 operation/cleanup_policy；旧版本混装在写入前拒绝。跨语言类型、校验和夹具详见 [共享契约](../packages/contracts/README.md)。修改配方必须递增 revision；同一 state 拒绝目录版本回退。客户端签名到期后禁用安装，仍可显示开发目录中的官方来源引导。

## 验证边界

旧 `scripts/smoke-transaction.py` 已在跨平台安装器迁移收口（2026-10-11）删除；其协议 v1 安装/卸载与强杀恢复结果仅为历史基线。现役 v5 子进程检查位于 [projection_smoke.rs](../crates/cli/tests/projection_smoke.rs)、[cache_smoke.rs](../crates/cli/tests/cache_smoke.rs) 与 [CLI 协议测试](../crates/cli/src/main.rs)及宿主测试，覆盖真实 JSONL、版本写前拒绝、签名 ZIP、长实例身份和首终态消费。

`python scripts/benchmark-projection.py --binary target/release/steam-patch.exe --output .artifacts/projection.json` 使用 v5 release CLI 测量四组隔离夹具，包含真实签名 ZIP 安装后动态收据投影；省略 `--binary` 则在临时源码副本构建 plain/counted，并核对哈希次数与响应等价，`--profile release` 可测发行构建。Linux 应显式提供本机 `--cargo` 或 `--binary`。`benchmark-scan.py CORE_BINARY FIXTURE_PARENT` 使用 v5 测量两库 500 清单，空目录 catalog 避免读取内嵌补丁。基准均不写用户游戏，不代表冷缓存或实机验收；真实样本检查继续使用 `scripts/run-windows-check.ps1` 和自动化套件。两端游戏内生效与 Deck 实体手柄须单独确认。

默认不上传游戏库，不设置遥测。诊断与状态保存在本机；分享日志前应删除用户名、库路径与下载 URL 参数。安全安装拒绝链接、路径越界、危险 ZIP 成员、哈希错误和未知 build；同账户篡改完整 journal 与父目录 TOCTOU 的剩余风险见 install-design.md。


补丁源订阅通过“管理订阅源”维护。默认扫描读取缓存；显式刷新更新 ZIP 来源，选择游戏后按需查找 Kagura 国内 / 海外及 HGamer。未查询的游戏可通过“全部游戏”进入；“有补丁”包含已核实身份的来源候选。详情展示语言、本体 / DLC、格式和发布页；EXE、网盘或未知布局候选只有网页引导，不能准备安装。游戏语言和 DLC 拥有 / 安装状态需按实际情况确认。查询失败保留成功缓存，部分完成可重新查找。新游戏只需网站发布符合规则的记录，无需更新固定游戏清单。添加网站及专用解析器的方法见 [源适配器与订阅](source-adapters.md)。停用源停止查询和匹配，既有安装收据仍保留。
