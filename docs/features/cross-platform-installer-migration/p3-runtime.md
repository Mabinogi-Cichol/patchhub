# P3 生产运行时与 EXE 编排

设计：[design.md](design.md)。施工与证据：[progress.md](progress.md)。冻结边界：[p0-contracts.md](p0-contracts.md)。独立复核与处置：[p3-gate-review.md](p3-gate-review.md)。状态：**已实现；双平台自动化、隔离对照与独立复核修复完成**（P4 Windows 实机、P5 Decky 部署与旧 profile 真包待验）。

## 实施范围

1. 固定 sf-worker 由受信宿主定位，复制进入私有运行目录；每次调用仅提供固定输入和输出、严格描述符。Windows LPAC/Job、Linux bwrap/受信身份/单进程限额均实际探测，失败关闭。
2. 来源缓存重算 EXE selection_id；下载仅允许来源白名单 HTTPS，逐跳公共 IP 解析并绑定实际连接，禁环境代理。核心不解析安装器 EXE。
3. 核心严格验证分析/转换报告和 ZIP 全部载荷摘要，再基于 Steam 实例与只读文件事实评估静态条件；重新读取来源、事实和条件后才 apply。worker installable=false 只表示分析结果，不授予安装许可。
4. 空间预算包括下载、转换、stage、原文件恢复材料；同卷合并。content_id 只绑定来源、语言、变体、EXE/元数据/有序载荷，不绑定临时 URL 或适配器版本。
5. acquire 前创建任务 job；租约/EOF/cancel 先同步持久 revoked，再触发网络/worker 的有效取消。计划后失败回收并以 journal 终态为准；冲突保留恢复材料。
6. CLI 固定 worker 定位和真实沙箱能力探测；不向 RPC 开放任意 runtime 路径、UID/GID、限额或命令。
7. 动态 EXE 镜像选择（2026-10-09 P4 实机验收裁决补冻）：同一候选内合法 EXE 镜像按 mirrors 原顺序**确定性择一**（非法镜像跳过）；下载后按实际字节建立内容身份；`exe_download_ambiguous` 收窄为合法镜像 basename 异构守卫。与动态 ZIP 同一规则（[p1-zip-orchestration.md](p1-zip-orchestration.md) §2.1(6)）。

## 验收

- 来源/身份、报告拒绝项、条件事实变化、同内容重装、空间合并与溢出、撤销先于停止、恢复结果有针对性自动化证据。
- 双平台隔离执行使用测试 fixture/临时游戏目录；权限失败、取消、超时、超输出与父进程退出有明确结果。未执行的 Windows 标准系统矩阵和 Steam Deck 部署留后续阶段，旧 profile 真包回归沿用已有待验记录。

## 实施与验证（2026-10-09）

- 实现模块：`installer-worker/src/production.rs`（固定 worker 生产启动器：真实 probe、私有 staging/owner lock、严格 descriptor/IPC、超时/取消/输出限额/父退出回收）、双平台启动器生产化（LPAC/Job、bwrap+seccomp/run_as/PDEATHSIG/env_clear）、`core/src/installer/{exe_orchestrator,exe_report,exe_resolve,space,task}.rs`、CLI EXE 路由与能力探测、`acquire` EXE 获取与公共 IP 绑定。
- 自动化：双平台 core/cli 测试与 clippy 全绿；三份真实 SF 样本在 release worker + 实际沙箱下分析/转换通过，产物摘要与 P2 冻结基准跨平台 3/3 全等（证据 `.artifacts/p3-production/{windows,linux}-real-sf.json`）。
- 隔离对照：`.artifacts/p3-production/{windows,linux,linux-root}-controls.json` 全 pass（权限失败/禁网/子进程/超输出/取消/父退出/超时/残留回收）。
- 复核修复：M1 任务运行态迁 `tasks/`（终态回收 + 崩溃清扫）、报告落 `reports/<content_id>/`；M2 只读路径进程级缓存探测、不写 installer state；R1 EXE 投影 release_id 改用来源 release 身份。R2–R8 建议登记见 [p3-gate-review.md](p3-gate-review.md)。
- 验收裁定：自动化验收必须使用 **release** worker（debug 构建无法在 300s CPU 沙箱限额内完成大样本转换）；命令见 [progress.md](progress.md) 与 `.artifacts/p3-production/core-linux-check.sh`。
