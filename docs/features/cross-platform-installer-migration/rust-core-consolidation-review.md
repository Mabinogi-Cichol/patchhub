# 迁移完成后的 Rust 核心整合审阅

日期：2026-10-11（Asia/Shanghai）。状态：**静态审阅完成；下列后续整合建议均待实现，未立项、未修改生产代码**。

关联：[设计](design.md) · [施工记录](progress.md) · [P6 复核](p6-gate-review.md)。

2026-10-11 后续立项：用户要求按优先级清除本次识别的技术债，已建立 [Rust 核心服务收敛重构](../rust-core-consolidation/design.md)及其[施工记录](../rust-core-consolidation/progress.md)，当前待实现。本审阅保留立项前的原始证据与建议。

## 审阅结论与范围

安装器迁移已经完成。当前仍有值得整合的模块，最大收益来自将 CLI 中已有的 Rust 领域逻辑提取到共享核心，以及让核心提供统一的安装动作可用性。Decky 剩余 Python 主要负责宿主适配，进一步整合宜采用小范围契约提取。

这里区分两个现状：Windows 与 Decky 已经通过同一个 Rust CLI 共用生产引擎；其中部分领域服务仍位于 crates/cli/src/runtime.rs，尚不能通过 steam-patch-core 的类型化 API 独立复用。后续整合属于架构收敛，不改变本轮迁移验收结论。

检查范围包括 Rust core、CLI、Windows Tauri/React、Decky 六个 Python 生产模块/入口、Decky React 与共享 TS UI。只读参考现有测试和完成记录，未重新运行测试、构建、部署或实机验收。以下模块名与 DTO 字段均为建议，尚未成为既定设计。

## 建议顺序

| 顺序 | 优先级 | 候选 | 收益 | 建议边界 |
| --- | --- | --- | --- | --- |
| 1 | 高 | 统一扫描投影与安装后检测 | 核心直接输出目录/来源/事务结合后的完整结果，消除 CLI 与 resolver 的规则漂移 | core 的 scan/projection 服务；CLI 保留参数、JSONL 编解码 |
| 2 | 高 | 签名目录安装编排与信任 revision 存储 | 三条安装路径都经类型化核心服务；核心使用者也能沿用防回滚规则 | installer/signed_orchestrator 与 trust revision store；宿主提供路径、时钟与环境 |
| 3 | 中 | 缓存清理服务 | 清理边界、字节统计与安装后清理结果统一 | core 的 installer cache 服务；是否自动清理由显式偏好决定 |
| 4 | 中 | 安装/覆盖动作可用性 DTO | 双端消费同一能力与事务证据快照，减少各自组合状态 | core 输出动作与理由代码；UI 保留忙碌/选中/文案控制 |
| 5 | 较低 | 类型化安装请求与公共 RPC 契约 | 互斥条件、请求 ID、错误边界易于统一 | 领域请求放 core；wire 格式辅助放独立 protocol 模块/crate |

顺序 1–2 是主要建议，可分成独立功能实施。顺序 3 规模较小，也适合作为下一项独立整合。顺序 4 应在扫描 DTO 稳定后实施；顺序 5 避免将 JSONL 和 Tauri 依赖引入核心。

## 1. 统一扫描投影与检测（高）

直接证据：

- [runtime.rs](../../../crates/cli/src/runtime.rs) 的 execute_controlled（502）、扫描结果组合（630–680）、dynamic_patches（1515）、dynamic_detection（1637）、preview_route（1711）、committed_changes（1776）仍在 CLI。它们决定签名目录优先级、动态选择、来源启用/订阅/缓存 blocker、事务归属和文件完整性。
- ZIP 扫描投影再次检查 source_not_enabled、source_not_subscribed、selection_stale（1548–1564）；[resolve.rs](../../../crates/core/src/installer/resolve.rs) 的 resolve_zip（392–452）已维护同一组条件。release ID 拼法也分别在 CLI dynamic_release_id（1505）与核心 source_release_id（184）实现。
- CLI hash_target（1693）直接读取目标并哈希；[catalog.rs](../../../crates/core/src/catalog.rs) 的 hash_file（283）已有相对路径、库边界、链接路径、文件类型与读取尺寸检查。提取检测服务时应统一受限文件取证能力，保留严格边界；当前审阅未执行链接路径场景，未把该差异定性为运行缺陷。
- [Decky main.py](../../../apps/decky/main.py) 的 _project_row（70）与 _refresh_rows（80）已经回调 Rust scan 获取完整投影。_refresh_rows 特意批量扫描一次，避免逐事件重哈希所有已安装载荷。

建议新增类型化 ScanInputs/ScanReport 与单实例投影 API。目录、来源快照、事务和宿主提供的 SandboxAvailability 作为输入/依赖；动态 blocker 复用 resolver 的公共判定。全量扫描继续提供完整报告；来源发现更新可使用单实例或批量重投影，避免把全量文件哈希放入每个 UI 事件回调。

边界：扫描 generation、旧事件屏蔽、QAM 挂载与 async 会话继续由宿主/UI 管理。提取时保留签名优先、动态 EXE selection/content 身份、最新 committed 收据、superseded 排除与部分/冲突检测的既有语义。

## 2. 签名目录安装编排与信任存储（高）

直接证据：

- CLI [runtime.rs](../../../crates/cli/src/runtime.rs) 的 install_patch（1043）决定三种路由；install_signed_route（1250）与 install_signed_route_inner（1283–1470）实际管理 request claim、TaskControl、下载、计划、facts、授权、应用前重核、事务提交、错误撤销和失败恢复。
- 动态 ZIP 与 EXE 已分别位于核心 [orchestrator.rs](../../../crates/core/src/installer/orchestrator.rs) 与 [exe_orchestrator.rs](../../../crates/core/src/installer/exe_orchestrator.rs)。签名路线复用核心组件，但其完整业务流程仍属于 CLI 私有函数。
- CLI highest_revision（336）、remember_revision（357）、signed（389）、ready_release（411）维护 trust.sqlite、防目录回滚、撤销与有效期、平台/build/支持等级准入；[trust.rs](../../../crates/core/src/trust.rs) 已提供签名验证能力。

建议先把签名路线整体提为核心服务，沿用现有事务、授权、请求账本和取消回滚组件；再统一上层 InstallTarget 路由与 InstallOutcome。信任 revision 存储封装为核心组件，并保留写事务内再次核对 revision 的既有防并发回滚保障。默认内嵌信任公钥与宿主配置文件查找可由 CLI/宿主注入。

实施验收应沿用/迁移 install_flow_tests 对 signed 重试、重装与幂等的覆盖，并验证非法状态目录、撤销/过期、防回滚、失败回收与取消路径。迁移不应顺带重写已有动态安装器。

## 3. 缓存清理服务（中）

直接证据：

- CLI [runtime.rs](../../../crates/cli/src/runtime.rs) 的 cleanup_cache（688）和 directory_size（708）负责遍历、统计和删除。
- 核心 [store.rs](../../../crates/core/src/installer/store.rs) 的 cleanable_dirs（168）、list_cache_entries（173）、is_protected（189）已定义允许清理与保护区域；[task.rs](../../../crates/core/src/installer/task.rs) 的 cleanup_stale_tasks（57）管理遗留任务清理。
- [Decky main.py](../../../apps/decky/main.py)（261–268）在 committed 且 auto_cleanup 开启后调用 cleanup_cache，并合成 cleanup 结果；[plugin_settings.py](../../../apps/decky/backend/plugin_settings.py) 保存偏好。当前 Windows 路径未检出 auto_cleanup/cleanup_cache 调用，是否补齐该体验应单独确认产品范围。

建议核心提供显式 cleanup_cache(paths, control) 与类型化 CleanupResult，集中维护清理边界和统计。Decky 可以继续保存偏好并调用该服务；如果要统一自动清理，则由宿主传入明确策略，在核心提交后处理清理结果。缓存删除失败应保持已提交安装的成功状态。

实施时需核对活动任务、任务锁、收据、授权、日志、stage/backup、撤销标记、链接路径与中断恢复材料的保护，不允许把所有 state 子目录都当成缓存。Windows 自动清理不是本次审阅已批准的新增范围。

## 4. 安装动作可用性 DTO（中）

直接证据：

- [共享 UI](../../../packages/ui/src/index.ts) 的 patchInstallable（130–142）组合 blockers、download_available、support_level、selection_id、catalog_trust 与 installer.links；该函数已由双端共享。
- [Windows main.tsx](../../../apps/windows/src/main.tsx) 的 projectScan（68–96）另根据 committed 事务的 release/selection/content 身份与 detection 推导 auto_installable/reinstall_available。
- [Decky index.tsx](../../../apps/decky/src/index.tsx) 的 projectPatch（22–27）派生 auto_installable；详情页（283–288）再组合能力与 detection 决定安装/覆盖按钮。

这里存在下沉价值，但它是“将展示前的动作判定归核心”，不是两端复制了完整安装授权算法。建议核心投影例如 actions.install/actions.overwrite、manual_archive_allowed 与理由代码；两端对相同快照获得相同动作事实，保留平台能力差异。主机 busy、扫描中、按钮焦点/确认与用户语言仍由 UI 控制。

动作事实仅描述该快照下是否允许发起任务；真正安装时仍需核心重核当前来源、实例 facts、授权、租约与事务条件。不要将扫描产物作为不可撤销的授权。

## 5. 请求与协议契约（较低）

直接证据：

- [Windows Tauri main.rs](../../../apps/windows/src-tauri/src/main.rs) 的 build_install_request（974–1007）和 CLI install_patch（1043–1075）都检查 release/selection 互斥、必需字段和 selection 禁止本地 archive；[Decky main.py](../../../apps/decky/main.py)（244–250）也实施操作枚举与 archive 约束。
- CLI [main.rs](../../../crates/cli/src/main.rs)（43–57）与 Windows Tauri（62–64、228–239）分别维护 request ID 与 detail 截断，截断末尾语义略有不同。

建议用类型化 InstallTarget/InstallRequest 封装领域约束，在核心执行入口做权威校验；宿主可以保留早期拒绝。公共请求 ID/错误 DTO/大小边界可放独立协议模块，以固定测试样本校验 Python 消费者。

JSONL 编解码、stdin/stdout 有界读取、Tauri invoke/event、SteamOS 降权和固定二进制查找继续属于适配层。将这些全部下沉到 core 的收益有限。

## 保留在宿主/UI 的模块

- Decky state_host.py：Decky 提供的 state 路径、root 下用户 UID/权限核对与宿主目录准备。
- Decky bridge.py 与 Windows Tauri：固定二进制、子进程、renew/cancel/EOF、窗口或插件卸载、事件派发与流传输。
- Decky tasks.py：15 秒前台租约的宿主触发、任务历史/保留上限、异步任务生命周期；核心 TaskControl 负责持久撤销和写入门控，两者职责不同。可选的小范围提取是统一安装结果到任务终态分类，但不建议整体移动任务管理器。
- Decky scan_session.py/SourceService：报告快照、代次和事件薄投影，当前业务事实已来自 Rust；不另建第二套来源/安装引擎。
- UI 文案、颜色、搜索筛选、偏好排序、进度百分比与 ETA、QAM 内存偏好：继续使用共享 TS UI。来源链接 guard 在 Decky discovery.ts（40–48）与 Windows sources.ts（282–294）重复，可先提到共享 TS helper；浏览器链接打开与下载网络防护契约应分别处理，不能直接套用下载许可策略。
- scripts/sf9_*、sf95_* 等冻结分析/验收工具：开发证据工具不属于重复生产引擎，本次未发现需要为架构收敛而迁入运行核心的依据。

## 验证和遗留

- 已完成：三路只读代码探索与主 Agent 调用链/行号交叉核验；审阅前工作区干净。
- 本次修改：仅本审阅、施工记录关联与功能索引；没有生产代码变化，没有 Git 提交。
- 未执行：测试、构建、部署、实机、性能基准。沿用已完成迁移验证，不宣称建议中的新 API 已实现或验证。
- 若进入实施，应为独立整合功能建立 design.md/progress.md，并分功能开发/验收。建议先做扫描投影和签名编排；方案冻结后按职责移动现有测试，仅针对新增契约与尚未覆盖的边界补充验证。
