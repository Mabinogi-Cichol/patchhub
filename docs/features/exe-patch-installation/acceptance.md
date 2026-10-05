# EXE 安装验收矩阵

设计：[design.md](design.md)。实际施工与原始证据摘要：[progress.md](progress.md)。本表按设计第 4–9 节核对完成门槛，日期为 2026-10-05。合成库通过、代码检查和真实游戏验收分别记录；任何待验证项均不能计为计划全部完成。2026-10-05 新增的 VenusBlood FRONTIER（AppID 1189440）真实组合单独标注，不替代夏日样本的待验证项。

## 工程与官方审核

| 门槛 | 当前证据 | 结论及剩余工作 |
| --- | --- | --- |
| P0 官方样本身份、全部载荷与资源预算 | 夏日中文/英文 EXE 元数据与逐成员摘要；VenusBlood FRONTIER CN v5 的 Setup Factory 9.5 审计（3 个根文件、展开 1.15 GiB）；Deck 只读审计、bwrap 隔离及资源探针 | 载荷核验与资源探针通过。夏日当前实例 DLC/CRC 不适用；VBF 已用真实实例审核 |
| P0 完整事件、条件、变量、覆盖及高级动作 | 夏日静态 runtime/元数据双证据（见 p0-audit）；VBF framed 事件与动作覆盖报告（见 progress 与 `.artifacts/exe-patch-research/vbf/`） | VBF：仅 `VBFI.exe` 定位、无删除/运行/下载/注册表写，按纯新增建模。夏日：对象及项目级未知项尚未全解，保持引导 |
| P1 后端关联与严格 registry | [profile 测试](../../../apps/decky/backend/test_exe_profiles.py)、[RPC 测试](../../../apps/decky/backend/test_exe_rpc.py) | 自动化通过；候选、前端任意路径/URL、伪造或过期关联不能升级。生产 registry 现含 1 条经审核 VBF 条目 |
| P1 来源、订阅、镜像、时间、profile 及实例漂移 | profile/RPC/服务测试，apply 前重新验证 | 自动化通过；当前 build 不会自动成为已验证 build。VBF 由 HGamer upstream 页 + 官方源合并得到 `official_title` 绑定，未按标题/后缀单独授权 |
| P2 HTTPS/每跳域名/大小/SHA/取消/原子缓存 | [下载测试](../../../apps/decky/backend/test_exe_download.py)；2026-10-05 VBF 从白名单 `kaguraserver.com` 真实下载 1,126,694,464 字节并通过大小/SHA 复核 | 自动化与真实官方下载链通过；镜像等价性仍只按摘要，不拼接不同镜像字节 |
| P2 PE/版本/UTF-8/安全路径/Unicode及大小写/文件目录冲突 | [审计测试](../../../scripts/test_sf95_audit.py)（14 项，含根目录 `%AppFolder%` 回归） | 自动化通过；不支持的格式无执行 fallback |
| P2 偏移、计数、CRC、长度、压缩结束及尾部 | 审计测试、三份官方样本只读载荷报告 | 自动化及官方载荷核验通过；未知元数据尾区保留为未审核证据 |
| P2 EXE 2 GiB/单成员 1 GiB/总量 2 GiB/元数据 16 MiB/条目 8192/字典 128 MiB/地址空间 3 GiB/180 秒限额 | parser/worker 固定限额、审计测试、Deck 资源/取消探针、VBF 1.05 GiB 与夏日 DLC 5,874 条目实测 | 预算按真实补丁在 Deck 调整并验证（条目上限 4,096→8,192），仍低于核心其它上限；不在失败后自动提高限额 |
| P2 运行时隔离、无网络/游戏/state 挂载、完整取消 | Deck bwrap 探针及最新实际 worker 取消报告 | 通过；未执行官方 EXE/Lua/DLL |
| P2 审核清单锁定、确定性 ZIP、预存产物归属 | [worker 测试](../../../scripts/test_sf95_worker.py)、最新 Deck 脚本测试 | 通过；两遍摘要一致，失败不删除预存输出 |
| P3 EXE→清单→ZIP→签名配方→plan 摘要链 | [服务测试](../../../apps/decky/backend/test_exe_installation.py)、Deck integration-result；VBF 真实 prepare（结果 SHA `8a3074a8…`） | 合成库与 VBF 真实链通过；本地签名不表述为发行商签名 |
| P3 build/语言/DLC/SHA/CRC 与确认 | [可信事实测试](../../../apps/decky/backend/test_exe_facts.py)、服务与 RPC 测试；VBF profile 基线 `VBFI.exe` | 自动化通过；安装 depot 不表述为已购买权益；基线由 profile 声明而非硬编码 |
| P3 state/目标卷分别估算空间、TTL/取消/互斥 | 服务测试及真实 worker 取消报告 | 自动化/合成通过；VBF 在内置盘真实通过，外置库和满盘待验证 |
| P3 唯一 Rust 写入、prepare→apply→uninstall/recover | Deck 实际 bwrap + Rust 合成库；2026-10-05 设备当前源码合成闭环（debug CLI `e69a1cd8…`）与 VBF 真实闭环（install SHA `e8304129…`、uninstall SHA `867b32ed…`） | 通过；prepare 不改载荷/存档，首次空协作锁单独记录；VBF 纯新增事务在无备份时安全卸载 |
| P3 权限保留、崩溃恢复、备份损坏的完整检查 | [Rust 安装测试](../../../crates/core/tests/install.rs)；2026-10-05 设备 Linux `cargo test -p steam-patch-core -p steam-patch-cli` 通过 | 权限保留、崩溃恢复、备份损坏/用户修改阻断恢复均已在 Linux 实际运行通过 |
| P3 重启/设置变化/清理后保留备份、来源停用后恢复 | [保留测试](../../../apps/decky/backend/test_exe_retention.py)、RPC 测试与 Deck restart | 通过；用户修改冲突保持，损坏/未知策略 fail closed |
| P4 条款、条件、备份确认、阶段、错误、取消、重入 | [安装 UI 测试](../../../apps/decky/tests/install.test.ts)、[任务测试](../../../apps/decky/tests/task-controller.test.ts)；真实未审核候选页面截图 | 前端测试、typecheck/build 通过；VBF 真实流程经 RPC 传入三项确认执行。真实屏幕上的三确认界面与实体手柄仍待验证 |
| P4 来源失效/旧请求隔离/旧 ZIP 回归 | [会话测试](../../../apps/decky/tests/session.test.ts)、后端 Linux 回归 | 自动化通过；实际屏幕/实体手柄不能由此推定 |
| P4 离线包、固定脚本/许可、源与包摘要 | 32 文件插件包，Deck 包内 worker 实际运行；[打包测试](../../../scripts/test_package_artifacts.py) | 通过；无官方 EXE/游戏资产/tests/fixtures 入包 |
| P4 完整实际 SBOM、Windows/Linux 托管 CI | [SBOM 测试](../../../scripts/test_build_sbom.py)、CI 已接入；本机用隔离 Rust 工具链生成 `.artifacts/cargo-metadata.json` 并运行 `build-sbom.py`，727 组件、无缺失许可元数据 | 真实完整 SBOM 已在本机生成；远端 GitHub Actions 仍未触发验证，Windows/Linux 托管 CI 结论保持待验证 |
| P4 设备部署、reload、唯一实例、持久设置 | Decky 官方接口更新；32 包文件、1 个实例、state 检查；设备 `dist/index.js` SHA `c7243259…` 与本地修复版一致，`RestartJSContext` 后新 bundle 生效 | 部署、reload 与新 bundle 生效通过；实体手柄仍待验收 |

## 真实游戏与交互验收

| 门槛 | 当前证据 | 结论及剩余工作 |
| --- | --- | --- |
| 具体组合的实例、build、语言、DLC、权益确认、profile 基线 | VBF：AppID 1189440、build 5430305、schinese、无 DLC、`VBFI.exe` SHA 匹配；夏日 DLC：AppID 1227890、build 6270017、schinese、DLC 1390150、`Game.exe`+`Script.vdf`（CRC 2002373744）匹配 | VBF 与夏日 DLC 组合通过并已开放 profile；夏日本体需要未装 DLC 的实例，不与 DLC 混用 |
| 官方完整下载→准备→预览→确认→安装 | VBF：1.126 GB、3 个 add、无 blocker、`committed`；夏日 DLC：279 MB、5,874 变更（9 add / 5,865 replace）、`committed` | 两个官方组合均通过 |
| 准备前后真实游戏全目录/存档/路径审计 | VBF：无关文件 `VBFI.exe`/`data.xp3`/`interface.xp3` 不变；夏日 DLC：`Game.exe`/`Script.vdf` 不变，逐变更核对 | 通过（变更文件核对，非全目录逐文件）；全目录审计仍待更严格证据 |
| 安装后全部声明文件与实际内容 | VBF：3/3 SHA-256 一致；夏日 DLC：5,874/5,874 与计划一致 | 文件层通过；实际进入游戏确认补丁生效仍待实机（本轮未运行游戏） |
| 原摘要恢复、新增文件移除、存档/无关文件、后续修改冲突 | VBF：3 个新增移除；夏日 DLC：5,865 个替换全部恢复原 SHA-256、9 个新增移除、无关文件不变；合成库冲突通过 | 两个组合恢复通过；用户后续修改冲突保持由合成与既有测试覆盖 |
| 内置盘/外置库、离线/满盘/取消 | VBF 与夏日 DLC 内置盘真实通过；合成库取消与空间守卫 | 外置库与满盘的真实检查待验证 |
| QAM 中文、长版本/路径、滚动、确认/恢复/取消、实体手柄 | 代码与自动化检查；真实 Steam 全页及 QAM 截图；设备 QAM 实机截图 `.artifacts/exe-deck-ci/qam-final.png` | QAM 拆字修复实机复核通过；正式安装任务交互、大事务卸载进度与实体手柄仍待验证 |
| 仅对完成全部门槛的具体组合开放 profile | VBF `exe-15da7b6a…-1189440` 与夏日 DLC `exe-4fd81a78…-1227890` 已开放；夏日本体仍为引导 | 两个组合开放；其他样本维持引导，不得外推 |

原始日志与研究材料位于本地忽略的 `.artifacts/exe-patch-research/`、`.artifacts/exe-deck-ci/` 及设备独立验证目录；progress 保存其 SHA-256 与范围。未执行的检查保持待验证，不能用文档、测试名称或进程仍运行本身证明结果。
