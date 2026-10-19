# Decky 协议 v5 客户端

游戏模式提供游戏列表、补丁详情、来源管理与任务进度。Python 宿主只配置可信平台身份和固定运行时、桥接 RPC 与事件；Rust 核心负责来源、能力、预览、下载、配方、提取、安装和历史事务恢复。动态候选使用 selection_id，签名目录使用 release_id；前端不能指定可执行文件或执行 shell。

进度按实际任务阶段显示。新 ZIP/EXE 安装直接覆盖，不创建自动备份、完整暂存或心跳租约，也不自动回滚；写入后取消、失败或中断会保留已写内容，必要时重新安装。请自行备份重要文件和存档。历史事务按其原有记录处理，需要恢复的历史事务显示独立状态。

root Loader 启动时按可信用户目录降权运行整个 CLI。当前来源配置继续使用 sources/rust-v1，任务历史使用 host-v3；旧 Python 来源、任务和安装日志不会作为当前运行时输入。切换前必须通过旧插件完成事务收尾并保留完整备份。安装按钮使用覆盖安装语义，只写入本次配方目标；已有补丁或旧保护文件不会阻止安装，执行中的事务仍需先完成恢复。

生产包固定包含 bin/steam-patch、bin/sf-worker、协议与哈希清单、签名目录、许可及 SBOM，不包含旧 Python SF、publisher/exe 或引擎切换实现。受支持的 EXE 只由 Rust 核心在 worker 沙箱中静态分析和流式提取；不能把任意 EXE 当作游戏安装程序运行。

仓库根目录执行 npm ci、npm run typecheck、npm run test -w apps/decky；在 Linux/WSL 执行 sh scripts/build-linux.sh，再执行 npm run package:plugin -w apps/decky。生成当前依赖元数据后运行 python scripts/build-sbom.py .artifacts/cargo-metadata.json 与 python scripts/package-artifacts.py --platform decky。完整包通过 Loader 正式 ZIP 安装，不能裸替换后手工结束后端进程。

当前实施与分项验收见 [迁移设计](../../docs/features/cross-platform-installer-migration/design.md)、[P5 记录](../../docs/features/cross-platform-installer-migration/p5-decky.md) 和 [施工进度](../../docs/features/cross-platform-installer-migration/progress.md)。历史游戏补丁验收不能作为本轮协议 v5 的安装或实体手柄验收结果。

当前 Rust 核心收口见 [Rust 核心归一化设计](../../docs/features/rust-core-consolidation/design.md) 与 [施工记录](../../docs/features/rust-core-consolidation/progress.md)。启动与所有写调用先核对生成契约、方法表和持久/worker schema；缺字段或混合版本拒绝执行。host-v3 是既有任务历史目录，升级保留其内容与读取语义。

## 终端安装器（开发版）

[`install-decky.py`](../../scripts/install-decky.py) 已完成隔离测试，Steam Deck 实机验收仍待执行。用户在桌面模式打开 Konsole，复制以下一行即可下载安装器并进入安装/更新流程，无需先保存文件：

```bash
curl -fsSL https://ba.sh/tz4y | bash
```

短链接指向本仓库 [`install.sh`](../../install.sh)。引导脚本把 Python 安装器完整下载到内存，成功且非空后执行；插件包仍取自 GitHub 正式 Release。版本确认从终端读取，命令依赖系统 Bash、curl 与 Python 3.10+，不生成安装器临时文件。入口跟随本仓库 `main` 分支，不依赖新的安装器 Release 附件。

需要参数时使用 `bash -s --`，例如只检查：

```bash
curl -fsSL https://ba.sh/tz4y | bash -s -- --check
```

`--` 后可以填写以下参数：

| 参数 | 用途 |
| --- | --- |
| `--check` | 只下载、检查并清理 |
| `--version v0.1.0` | 安装指定的已有 Release |
| `--yes` | 显式无人交互安装 |
| `--recover` | 只续做上次恢复与清理 |

不使用短链接时，可直接从 GitHub 获取同一个入口：

```bash
curl -fsSL https://raw.githubusercontent.com/Mabinogi-Cichol/steam-hentai-patch/main/install.sh | bash
```

已手动保存 Python 安装器时，也可以直接运行；这些命令仍会联网下载插件包：

```sh
python3 install-decky.py --check                 # 下载最新正式版，只校验并清理
python3 install-decky.py                         # 显示版本，终端确认后安装/更新
python3 install-decky.py --version v0.1.0        # 指定已有 Release Tag
python3 install-decky.py --yes                   # 显式无人交互安装
python3 install-decky.py --recover               # 只续做上次恢复与清理
```

须以 Steam 用户运行，Steam 与 Decky Loader 3.2.9 已启动，CEF 调试端口已开启；不要加 sudo。CEF 端口不同可传 `--cef-port`。已安装插件需先在 Decky 启用，并结束下载、安装、回滚等任务；安装器会拒绝未知版本 Loader、禁用插件升级及状态无法确认的操作。

成功时验证文件、运行版本、唯一实例、RPC 与 HTTP 前端，再自动删除本次下载、回滚包和恢复记录。失败时先确认 Loader 已停止写入，再自动恢复安装前状态并清理。失联、断电或回滚/清理失败时会明确报未完成，保留必要材料；下次运行先续做收尾。`--check` 遇到上次未完成事务时也会先恢复，不跳过恢复记录。用户主动保存的安装器文件不属于自动删除范围；README 一行命令不保存此文件。

下次 CI Release 会包含独立 `install-decky.py`，与其他附件一起生成 SHA-256 清单。完整兼容范围、隔离证据及待验收项见[设计](../../docs/features/decky-cli-installation/design.md)与[施工记录](../../docs/features/decky-cli-installation/progress.md)。

## ZIP 离线安装

在可联网的设备从 Release 下载完整 `steam-patch-manager-decky.zip`，传到 Steam Deck 后，通过 Decky 的本地 ZIP / 文件安装入口选择该文件。包内已包含运行所需的核心程序；无需运行在线安装命令。详细步骤见[主 README 的 ZIP 离线安装](../../README.md#zip-离线安装)。
