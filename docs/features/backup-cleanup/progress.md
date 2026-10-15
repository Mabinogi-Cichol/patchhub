# 全部安装备份清理施工记录

设计：[design.md](design.md)。状态：**核心 / 协议 / 设置页已实现，本地自动化验证通过；候选包已构建校验，实机待验收。**

## 2026-10-15：计划制定

- 用户要求追加备份清理，明确“不需要区分，直接全清”，随后要求“先定制计划”。
- 已只读核对全局设置中的下载缓存按钮、共享 Rust CacheService、安装事务 backup 写入/恢复/成功清理、InstallerPaths 与并发 CacheLock。当前清理下载缓存保留备份，尚无本计划的新全清入口。
- 建立独立功能目录与设计/施工记录，在功能索引登记待实现。设计明确全状态备份删除、孤立/旧版布局、占用与实际删除统计、并发互斥、直接操作、部分失败与恢复缺失结果。
- 本轮仅改文档；未修改代码或测试，未运行构建，未连接设备、部署或删除实际文件。此前安装器修复的未提交改动保持原样。
- 待办：P1–P6 均待实现/验证。布局识别细节与协议字段在施工时确定，不把计划描述为已有能力。

## 文档检查

- 4 条功能文档相对链接存在，功能索引新增链接存在，git diff --check 通过；这些检查只验证文档，不代表备份清理功能已实现。

## 2026-10-15：实施与本地验证

### 已实现

- P1/P2：新增 Rust `BackupService`，全状态扫描当前 `installer/rust-v1/jobs/`、旧 `jobs/` 与 `publisher/<release>/<instance>/jobs/`。仅清理数字 `.backup`、任务内 `backup/` 和本清理器专属隔离残留；不读取或筛选 journal 状态，包括损坏日志与孤立任务。
- 删除统计按实际成功删除文件数 / 字节累计；空结果幂等，失败列表与 partial / failed / busy 返回准确。链接、硬链接及无法访问路径报告失败，不跟随到外部。
- P3：新增 `backup_status` / `cleanup_backups` 合约与 CLI 路由，重新生成 Python / TS / schema / fixture；Decky main 与 bridge 仅转发固定方法，删除 RPC 不接受用户路径。
- P4：全局设置追加安装备份占用、说明、直接全清按钮和部分失败反馈；清理后刷新占用；标题“缓存与备份”，原路由保留。下载缓存开关和持久化设置不变。
- 当前 CacheLock 独占锁覆盖生产安装 / 恢复 / 丢弃。旧存储额外安全创建并持有固定 journal.lock，覆盖原本无锁文件的孤立目录，避免稍后旧写入者绕过互斥。
- 独立 reviewer（gpt-6.1-sol / high，只读）发现缺锁旧目录、Linux 隔离残留漏扫、partial + busy 文案问题，均已修复；定向复核未发现剩余可操作问题。Linux 专属 `.backup-cleanup-<规范 UUID>` 可在清理进程中止后重新统计 / 清理，通用 `.cleanup-*` 保留。

### 验证结果

- Windows 与 WSL Ubuntu：`cargo test -p steam-patch-core -p steam-patch-cli -p steam-patch-protocol` 通过（仅运行默认测试；原有显式网络 / 真实 EXE 样本 ignored 项未执行）。覆盖全状态 / 旧布局 / 缺损 journal / 保留 stage 与授权 / 幂等 / 硬链接部分失败和重试 / 当前锁与旧锁跨进程互斥 / 恢复缺少备份仍 needs_recovery。
- WSL Linux 另有真实终止子进程回归：在备份 rename 隔离后 SIGKILL，重新统计仍报告备份，重试删除成功。合成目录中测试，不涉及设备。
- CLI 真 JSONL 请求覆盖占用、删除结果、拒绝用户删除路径；协议新增 schema 请求分支穷尽所有方法的测试通过。
- 完整 Python 后端回归 81 项：通过，2 项平台条件跳过。新增备份薄桥接 3 项均通过。日志：[python-tests.log](../../../.artifacts/backup-cleanup-20261015/python-tests.log)（忽略的本地验证材料）。
- Decky 回归 69 项通过；Windows 与 Decky TypeScript 类型检查通过；Decky Rollup 构建通过；共享 UI 合约检查与 Rust 生成器 check 通过。
- 初次完整桥接回归暴露手工 Request schema 参数表随新增方法位移，已替换为穷尽 Method 匹配并补回归；修复后完整 Python 81 项通过。最初 CLI 方法数旧断言已同步为 18，最终回归通过。

### 当前边界 / 待验收

- P1–P5 已完成本地实施和自动化；P6 Linux 发布二进制与候选插件包已完成本地构建和校验；设备部署与实机验收待执行。
- 未连接设备、未部署、未删除设备真实备份；真实 RPC、Steam 游戏模式设置页和设备清理结果仍待实机验收。设备验收按设计仅清理本轮创建的测试备份，既有真实备份全清需明确执行范围。
- 未创建 Git 提交。当前仓库起始状态干净，本轮变更仅属于本功能及所需协议 / 测试 / 文档。

### 候选包与发布二进制校验

- WSL Linux `cargo build --release -p steam-patch-cli -p steam-patch-installer-worker --bin steam-patch --bin sf-worker` 通过，Decky 候选目录已重新生成；Python/JSON schema 与源码生成产物逐字节一致，运行时清单内二进制 SHA-256 与长度全部匹配，`validate_decky` 通过。
- 使用候选包内 Linux 发布二进制，通过真实 JSONL `serve` 在 WSL 原生 Linux 临时目录验收：当前与旧 publisher 备份合计 2 文件 / 11 bytes → completed 删除 2 文件 / 11 bytes → 剩余 0 文件 / 0 bytes；再次清理为 0，stage / authorization 字节保持不变。
- 本地候选包：[backup-cleanup-candidate.tar.gz](../../../.artifacts/backup-cleanup-20261015/backup-cleanup-candidate.tar.gz)。SHA-256：`b0765a88a47835508d099c0b87cb4cd12540e06e684593f66e692ab8184511f5`。
- 证据：[candidate-verification.json](../../../.artifacts/backup-cleanup-20261015/candidate-verification.json)、[本地校验脚本](../../../.artifacts/backup-cleanup-20261015/verify_candidate.py)。这些均为忽略的本地材料，不纳入 Git。
- 环境限制：首次把合成状态放在 WSL `/mnt/e` Windows 挂载盘（DrvFS）上时，renameat 成功后 fstatat 返回 ENOENT，renameat2 恢复返回 EINVAL，清理正确报告失败且未声称释放空间。原生 Linux 临时目录通过；不以该结果宣称支持 DrvFS 状态存储。现场和 [strace](../../../.artifacts/backup-cleanup-20261015/drvfs-trace.log) 保留用于后续定位，未使用设备数据。
- Clippy 完成；仅报告来源适配器 `kagura.rs` 的 5 处已有 needless_borrow，本功能生产代码未新增告警。测试夹具 slice 克隆提示已按建议修正。
- 最终状态：P1–P5 已实现并通过本地验证，P6 候选包完成；Steam Deck 部署、真实设置页和设备测试备份清理仍待验收。

## 2026-10-16：本地 Git 提交

- 按用户要求提交本功能，功能代码、必要测试与生成合约已提交为 `5842180`（`feat(backups): 支持全量安装备份统计与清理`）；配套设计、施工记录与功能索引单独文档提交。
- 沿用本次 session 已完成的验证，不重复运行；提交前工作区与暂存区 `git diff --check` 通过。
- 本次只包含 backup-cleanup；另一个会话的 `install-footprint-and-compatibility` 文档及索引行保持未提交。本地候选包与设备/真实库材料未纳入 Git。未推送，Steam Deck 实机验收仍待执行。
