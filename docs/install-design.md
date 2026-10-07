# 本地 ZIP 安装与恢复

`crates/core/src/install.rs` 提供 `prepare`、`get_plan`、`apply`、`recover`、`migrate_state`、`list_transactions`。测试只操作 `TempDir` 中的合成 Steam 库。

## 授权与输入

生产调用方在 prepare 前验证目录签名、时间、撤销、AppID、平台和 build；apply 前再次验证，并将计划文件映射与当前可信配方逐项比较。核心会独立验证 AppID/build、扫描对应 Steam 清单与实例、拒绝下载中实例、检查路径及文件基线。计划有效期 15 分钟，ID 是 UUID；apply 不接受前端目的路径。

安装模式只支持声明式 ZIP 文件新增与替换，不运行脚本或 EXE。未引用成员不会写入目标目录，但所有成员名、类型、额外属性和展开尺寸均检查。拒绝绝对路径、父目录、反斜杠、ADS、设备名、末尾空格/点、重复和大小写冲突（含隐式父目录）、文件与目录冲突、符号链接与特殊 Unix 类型。拒绝带 NTFS / Unix 链接语义的扩展属性；仅接受严格固定结构的 32 字节 NTFS 时间戳记录，忽略这些时间戳、不应用其属性。限制压缩包 8 GiB、单文件 4 GiB、全部展开 16 GiB、4096 个条目；大于 1 MiB 的成员最大压缩比 200。映射成员实际读取时限制字节数并校验完整 SHA-256，压缩包本身也校验 SHA-256；有 Artifact.size 时必须精确匹配。

## 事务边界

SQLite 使用 `synchronous=FULL` 与 DELETE journal；每次状态更新独立持久提交。state 中仅使用程序派生的 `jobs/<uuid>/<index>.stage` 和 `.backup`，不反序列化任意备份绝对路径。state 与游戏根互不包含。读取 state、游戏根及其所有现存祖先时拒绝符号链接/Windows reparse point。普通文件拒绝硬链接。

prepare 在隔离 state 目录暂存配方文件并返回空间需求、变更列表和冲突。游戏目录不改变。apply 取得 state 独占锁和游戏根独占锁 `.steam-patch-manager.lock`，重新确认基线和剩余空间，先备份并核验所有覆盖文件，再执行逐文件提交。

文件状态为 `pending → backed_up → writing → installed → restoring → restored`。事务记录先于创建目标子目录、写目标临时文件及替换落盘。备份与暂存数据 `sync_all`，Linux 上同时同步目录项。替换和还原保留原文件权限（包括 SteamOS 可执行位）；新增文件使用普通文件默认权限。目标临时文件与目标位于同一父目录，复制校验完成后 rename；跨磁盘仍是逐文件替换，不是全目录原子提交。崩溃后通过 journal 和文件前后哈希识别实际进度。

apply 返回实际事务状态，调用方必须区分 `committed`、`rolled_back`、`needs_recovery`，不能将所有成功返回值解释为安装成功。重复 apply 对已提交事务重新验证所有结果文件。

## 失败恢复与重装

恢复前先检查整个文件集合与所有需要的备份。当前文件必须等于记录的安装结果或原基线；任何新修改都会保留并报告 `needs_recovery`。Steam build 改变、库离线、清单下载状态、备份损坏都会阻止旧备份恢复。符合条件时恢复原文件、删除新增文件，仅移除事务创建且仍为空的目录；用户存档与无关文件不处理。

恢复支持在临时备份与替换中断后重试；旧卸载意图仅作为历史未完成事务的恢复输入，不提供新的卸载入口。未知内容的遗留目标临时文件也保留为冲突，不盲删。复制中断留下的部分临时文件，仅在 journal 为 writing / restoring 且其内容是完整哈希验证过的对应暂存 / 备份文件的严格前缀时允许清理（包括空文件）。恢复先检查所有目标、备份和临时文件，再执行任何还原；删除临时文件前再次验证。错误前缀、来源变更、链接或不匹配的进度均保留为冲突。准备阶段在 journal 建立前崩溃可能留孤立 UUID 暂存目录，不自动递归清除。

同实例待恢复事务阻止新安装；已提交事务允许显式 `reinstall`，仅限同内容、同实例、同 build 和目标映射。新任务成功后旧收据标记 `superseded`，失败则保留原收据。成功提交或完整回滚后清理临时 stage/backup，保留最小收据；清理失败可重试，不将已提交安装改为失败。内容升级、语言切换等不作为重装处理。

`migrate_state` 用持久迁移标记撤销升级前的可执行计划，处理历史材料与未完成恢复；新版取消先写 durable revoked，再丢弃计划。Decky 与 Windows 安装均通过 request_id 去重、一次完成 prepare→apply；安装保持前台，离开界面（Windows 关闭窗口）请求协作取消，恢复结束前不强制杀死宿主。详见 [精简安装流程](features/simplified-install-flow/design.md) 与 [施工记录](features/simplified-install-flow/progress.md)。Windows 客户端已适配协议 v2，安装走 `install_patch` 一步任务，`serve-task` 提供阶段/字节进度与 stdin 协作取消。

## 保证范围与待实机验证

文件读使用 Windows reparse-point 句柄和禁止共享写入、Linux `O_NOFOLLOW`；写入前逐步重新检查祖先、实例和哈希。父目录仍通过路径操作，尚未实现 Linux `openat2` / 目录句柄相对写入或 Windows 目录句柄绑定。具有同账户写权限的并发攻击者仍可能利用父目录检查与替换间的竞争窗口；不能声称完全消除 TOCTOU。锁协调本工具实例，Steam / 游戏不会遵守此锁，使用前应关闭游戏并暂停更新。

SQLite、state 目录和 Steam 清单属于当前本地账户的信任边界。路径约束阻止篡改记录指向库外文件，但不提供抵御同账户完整数据库伪造的认证备份；签名目录检查不能把本地 journal 变成可信签名数据。恢复从本地记录执行，调用方应保护 state 权限。Windows 数据文件 sync 已执行，但标准库没有本实现可用的目录 fsync，不宣称覆盖所有真实断电/存储驱动行为。Linux/Deck 挂载与 Windows 权限、满盘、断电仍需实机验收。

集成测试覆盖：新增/替换/幂等/重装、用户存档保持、变更基线与过期计划、Steam 更新/下载、损坏备份和暂存文件、不兼容重装拒绝、四个崩溃状态、卸载中断、恢复冲突、路径与 NTFS 名称攻击、大小写和目录冲突、ZIP 符号链接与压缩炸弹、完整哈希、journal 路径篡改、库外实例、state 重叠、硬链接与并发锁；Unix 另测符号链接目标和祖先。
