# Decky 协议 v3 客户端

游戏模式提供游戏列表、补丁详情、来源管理与任务进度。Python 宿主只配置可信平台身份和固定运行时、桥接 RPC 与事件；Rust 核心负责来源、能力、预览、下载、配方、转换、安装、卸载和恢复。动态候选使用 selection_id，签名目录使用 release_id；前端不能指定可执行文件或执行 shell。

进度覆盖九个安装阶段及应用子阶段。核心每三秒续租，宿主租约依赖界面查询；失去会话或请求取消后，结果必须等待核心实际提交或回滚。需要恢复的事务显示独立恢复状态。

root Loader 启动时按可信用户目录降权运行整个 CLI。当前来源配置继续使用 sources/rust-v1，任务历史使用 host-v3；旧 Python 来源、任务和安装日志不会作为当前运行时输入。切换前必须通过旧插件完成事务收尾并保留完整备份。安装按钮使用覆盖安装语义，只写入本次配方目标；已有补丁或旧保护文件不会阻止安装，执行中的事务仍需先完成恢复。

生产包固定包含 bin/steam-patch、bin/sf-worker、协议与哈希清单、签名目录、许可及 SBOM，不包含旧 Python SF、publisher/exe 或引擎切换实现。可信 EXE 只由 Rust 核心在 worker 沙箱中转换；不能把任意 EXE 当作游戏安装程序运行。

仓库根目录执行 npm ci、npm run typecheck、npm run test -w apps/decky；在 Linux/WSL 执行 sh scripts/build-linux.sh，再执行 npm run package:plugin -w apps/decky。生成当前依赖元数据后运行 python scripts/build-sbom.py .artifacts/cargo-metadata.json 与 python scripts/package-artifacts.py --platform decky。完整包通过 Loader 正式 ZIP 安装，不能裸替换后手工结束后端进程。

当前实施与分项验收见 [迁移设计](../../docs/features/cross-platform-installer-migration/design.md)、[P5 记录](../../docs/features/cross-platform-installer-migration/p5-decky.md) 和 [施工进度](../../docs/features/cross-platform-installer-migration/progress.md)。历史游戏补丁验收不能作为本轮协议 v3 的安装或实体手柄验收结果。
