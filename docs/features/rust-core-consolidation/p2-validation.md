# P2 签名安装、信任与统一终态验证

日期：2026-10-11 至 2026-10-12（Asia/Shanghai）。阶段：已实现，Windows/Linux 自动化与独立复核通过。P3–P6、双端最终产物与实机待实施。

设计：[design.md](design.md)。施工：[progress.md](progress.md)。冻结：[P0 契约](p0-contract.md)、[终态真值表](p0-behavior.md)。前阶段：[P1 验证](p1-validation.md)。

## 实际实现

- [service.rs](../../../crates/core/src/installer/service.rs) 是公开的三类 install 入口。受检查的 RequestId / InstanceId / ReleaseId / ZIP 与 EXE SelectionId / LocalArchive 禁止超限、控制字符、错误选择形状和链接路径。ValidatedHostContext 只保存受信状态根、Steam 根与本机平台，写入重新扫描事实，检查状态与游戏不重叠、非 root 身份、信任库绑定到同一状态根；配方和游戏路径不再由调用者直接提交。
- [signed.rs](../../../crates/core/src/installer/signed.rs) 迁移 signed request claim、绑定/重放、下载或本地包、计划、facts/授权和 apply。应用前重新加载原始签名、公钥、revision floor，核验有效期、撤销、release fingerprint、Steam 当前事实及授权。注入 Transport 保留签名包 HTTPS、域名、重定向、大小与完整 SHA-256 限制，允许既有不含 .zip 后缀的签名下载 URL。
- [trust/revision.rs](../../../crates/core/src/trust/revision.rs) 统一使用 state/trust.sqlite，沿用既有表与 singleton；缺失库/行只读返回零而不初始化。remember 仅接受 VerifiedCatalog，FULL / 5 秒 busy timeout / Immediate 内二次读取及 MAX 更新防并发回滚，拒绝 SQLite i64 溢出、负 revision 和损坏状态。数据库及 sidecar 检查链接、硬链接和路径变化；Windows 使用 nofollow 文件句柄确认链接数。只读遇外来 WAL 库明确拒绝，避免创建 shared-memory 文件；生产原 DELETE 模式不变。
- [outcome.rs](../../../crates/core/src/installer/outcome.rs) 保留原事务字段与持久身份，增加三值 disposition、cause、recovery、cleanup。commit 优先；取消/租约导致的 rolled_back 为 cancelled，普通失败为 failed，needs_recovery 始终 failed 且 recovery.required=true。精确取消码支持事务核心固定的非空清理诊断后缀，cause.code 与诊断分开；任意含 cancel 的自由文本不能建立取消事实。原 error 完整保留，结构化 detail 按 UTF-8 边界限制 2 KiB。
- TaskControl、ZIP/EXE 路线保留实时租约监控，使用明确原因回调区分 UserCancelled / LeaseExpired。失败撤销不伪造用户取消，授权撤销/普通失败不受迟到取消覆盖。计划后的失败返回真实事务；终态账本更新失败以 journal 为准，不能把已提交结果变成 RPC 失败。
- [requests.rs](../../../crates/core/src/installer/requests.rs) 为新请求导出有域分隔的稳定 UUID；三路线共用，无新 schema 或日志。请求行已存在但 plan 与 terminal 回填均失败时，按绑定检查实际 journal，重放同一个终态事务，不新建计划或重新加载签名。旧 plan_id 优先保留，旧随机身份仍读取；非终态缺失回填明确报告中断。
- 底层 prepare/apply/recover/discard、裸 ZIP/EXE inputs/deps 及 writer Store 收为 crate 内部接口。公开 recover/discard 在 journal 写锁内校验实际计划与新 Steam 事实，恢复整批先校验后写，包含已提交但有 uninstall_requested 的任务。安装失败自动恢复只处理自身 plan，不能顺带恢复共享状态里的其他实例。
- [CLI](../../../crates/cli/src/runtime.rs) 删除私有 signed、ready_release、highest/remember_revision、signed download 与 outcome_json，只构造依赖并调用 core。签名 envelope 保持原 8 MiB 容量；EXE 选择错误优先于 worker 启动错误。既有 JSONL 进度由 CLI operation 转发；直接库 emit 当前只发 planning/finalizing，不将其描述为所有细粒度进度的新订阅 API。
- 原安装集成测试完整移入 [install_tests.rs](../../../crates/core/src/install_tests.rs)，允许私有底层 API 回归；相应历史验收文档的活跃测试链接已更新。Cargo 依赖与持久 schema 未增加。

## 自动化证据

| 检查 | 最终结果 | 证据 |
| --- | --- | --- |
| Windows core / CLI | 465 项通过、6 项既有 ignored、0 失败 | [日志](../../../.artifacts/rust-core-consolidation/p2/windows-tests.log) |
| WSL Ubuntu 非 root、Rust 1.94 core / CLI | 472 项通过、6 项既有 ignored、0 失败 | [日志](../../../.artifacts/rust-core-consolidation/p2/linux-tests.log) |
| fmt / Clippy | 全 workspace fmt --check、Windows core/CLI all-targets -D warnings 通过 | [Clippy](../../../.artifacts/rust-core-consolidation/p2/windows-clippy.log) |
| 公共 signed / trust / outcome / service | signed15、trust Windows12/Linux13、outcome8、service Windows5/Linux6 通过 | [signed](../../../crates/core/tests/signed_install.rs)、[trust](../../../crates/core/tests/trust_revision.rs)、[outcome](../../../crates/core/tests/outcome.rs)、[service](../../../crates/core/tests/installer_service.rs) |

另用最终已编译 Linux 测试程序以实际 root 身份执行两项负向验证：公共宿主入口与 trust remember 均拒绝 installer_host_requires_non_root，状态根未创建。两项额外验证不重复计入非 root 全量；[service 日志](../../../.artifacts/rust-core-consolidation/p2/linux-root-service.log)、[trust 日志](../../../.artifacts/rust-core-consolidation/p2/linux-root-trust.log)。

验证汇总：[validation-summary.json](../../../.artifacts/rust-core-consolidation/p2/validation-summary.json)；源码、链接与空白检查：[final-checks.json](../../../.artifacts/rust-core-consolidation/p2/final-checks.json)。

文档检查覆盖本 feature 和本次变更：306 个本地链接引用，新建/更新链接有效；另19个旧迁移/EXE验收文档的 Python 文件引用已在 HEAD 中失效（P6 已删除对应实现），已单独列入 final-checks 的历史项，未改写既有验收事实。git diff --check、44个变更文本的空白/EOF、原1377行安装回归保留和最终测试源码指纹核对通过。

Windows：scripts/cargo.ps1 test --offline -j 2 -p steam-patch-core -p steam-patch-cli。Linux 使用既有 .tools/linux-cargo / .tools/linux-rustup / .local/p1-linux-target 执行相同离线命令。Clippy：core/CLI --all-targets -- -D warnings；fmt：全 workspace --check。新增修改后补检查，不重跑已冻结的 P0/P1 基准。

Linux 新增清理 fixture 初轮发现目录枚举包含请求锁文件、快速提交早于异步取消监控确认；已改为仅选实际 job 目录并等待可观察的 revoked 记录后验证提交前取消，再重跑完整套件。迟到取消仍按 commit 优先。

重点负向：应用前撤销/过期/revision/key/配方和 manifest 变化；本地包缺失/篡改、域外重定向；计划前取消无伪造事务；计划后取消/租约/普通错误及真实写后清理失败；账本 UPDATE 失败与双回填失败重放；两线程 trust 高水位不下降；链接/硬链接/sidecar 不影响外部文件；恢复校验持有 writerlock、拒绝整批时零游戏变更、discard 拒绝不创建游戏锁、失败恢复不触及其他计划。

## 独立复核与修复

本阶段共四个 Sol/high 子 Agent（不继承无关上下文）：三路 specialist 分别实现 signed、trust、outcome；第四路 reviewer 只读审查公开写边界与持久终态，主 Agent 集成、修复、验证。reviewer 因公共 API/数据安全风险从 balanced 的 medium 提升为 high，路线在委派前公布。

复核发现并修正：裸写入口绕过准入、实时 EXE 租约、终态账本失败、签名 envelope 容量、取消原因竞态、授权撤销误判、Windows 硬链接、双回填失败幂等、计划前取消码、恢复/丢弃读写校验窗口、内部恢复波及其他实例，以及清理后缀改变取消终态、planned 丢弃的清理失败掩盖真实回滚事务。上述边界都有实现或定向回归；最终只读复核 findings=[]，无剩余可操作问题。审查是静态证据，测试结果独立记录。

## 阶段边界

- CleanupPolicy::Disabled 生效；AfterCommit 在 P3 实现之前明确返回 cleanup_policy_unavailable，不冒充已自动清理。已有事务 stage/backup 收尾保持成熟逻辑，cleanup DTO 默认 not_requested；失败诊断保留原 error/cause.detail。缓存服务、cache.lock 和 Decky 追加删除迁移留待 P3。
- 正式 wire 仍为 v3 开发期适配；core outcome 已权威化，双端 task 消费、actions、params.host、v4 与生成契约在 P4/P5；D3 已阶段完成，D7 客户端删除及其他债项仍待后续与 P6 最终关闭。
- 测试只写临时合成 Steam 库与隔离状态，未下载外网包、连接设备、部署或修改用户实际游戏/状态。实现验证阶段未提交；2026-10-12 用户授权后，P1/P2 代码已本地提交 `e77d0a8`，文档独立提交；未推送。
- 六项既有 ignored 测试不计为通过；真实 worker/网络/手动 benchmark、release 产物、双端构建/安装与实机验收仍待 P6，不以本阶段核心/CLI 通过替代。
