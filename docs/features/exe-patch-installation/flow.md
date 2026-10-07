# EXE 补丁完整流程图（当前实现）

2026-10-07 更新：[通用 EXE 自动安装](../generic-exe-installation/design.md) 已增加按完整对象图、受限 AST 与行为条件判断的新内容入口，旧 profile 不再是新安装的唯一入口。下图保留 2026-10-06 审阅时的历史流程，最新实现与验收见[新功能施工记录](../generic-exe-installation/progress.md)。

审阅日期：2026-10-06。基于提交 `d2f551f` 的代码；设计见 [design.md](design.md)，验证记录见 [progress.md](progress.md)，本次缺陷见 [review.md](review.md)。下图区分已实现流程和审阅发现的异常分支，不将设计意图当作已通过验收的行为。

## 1. 来源发现与准备

```mermaid
flowchart TD
    A[扫描已安装 Steam 游戏实例] --> B[查询或读取官方来源候选缓存]
    B --> C[EXE 候选仍为 discover_only]
    C --> D{唯一匹配受审核 profile?}
    D -->|否| GUIDE[查看官方说明 / 来源引导]
    D -->|是| E[核验来源启用与逐游戏订阅<br/>候选身份、6 小时新鲜度、审核标记<br/>已安装状态、审核 build、直接下载镜像]
    E -->|不满足| GUIDE
    E -->|满足| F[投影独立 exe release<br/>显示准备安装与适用条件]
    F --> G[用户接受条款、适用条件<br/>启用并确认保留备份]
    G --> H[启动可取消的 prepare 任务]
    H --> I[重取可信实例与 profile / candidate<br/>核验语言、DLC、基线文件<br/>磁盘预算与既有事务]
    I --> J[受限 HTTPS 下载<br/>校验跳转主机、EXE 大小和 SHA-256]
    J --> K[非特权 bwrap 离线 worker<br/>只读 EXE / 固定工具<br/>仅工作目录可写，不挂载游戏目录]
    K --> L[固定 SF 9.5 解析器<br/>检查元数据与完整文件表<br/>路径、重名、数量、展开大小、资源限制]
    L --> M[流式提取审核的游戏载荷<br/>逐成员校验大小、CRC、SHA-256<br/>生成确定性 ZIP_STORED]
    M --> N[后端复核 EXE、元数据、ZIP 摘要<br/>完整载荷清单必须与 profile 相同]
    N --> O[核对目标原文件并生成 add / replace 配方<br/>再次核验来源、实例与事务]
    O --> P[本地密钥签署受限 catalog<br/>发布 normalized.zip 与实例状态]
    P --> Q[Rust 校验签名、ZIP、路径、原文件与空间<br/>持久化载荷 stage 和 planned journal<br/>此时开始 900 秒计划有效期]
    Q --> R[保存 exe-policy.json<br/>绑定确认、profile、候选、实例事实<br/>文件摘要、计划期限与有效租约]
    R --> S[最后复核来源与实例<br/>返回计划预览：新增 / 替换、空间、阻止项]
    I & J & K & L & M & N & O & P & Q & R -->|失败或取消| X[准备异常清理：终止 worker<br/>尝试 discard plan 与释放租约<br/>删除本任务拥有的缓存 / 工作目录]
    X --> BUG[已知 P2：discard RPC 未执行即失败时<br/>释放租约会被跳过，stage 可能仍可应用]
    classDef issue fill:#fff0f0,stroke:#b42318,color:#7a271a;
    class BUG issue;
```

能力投影阶段不等于实例适用性已经通过：语言、DLC、游戏基线在 prepare 和 apply 中重新核验。`profile.review` 是维护者审核事实的声明；运行时摘要校验不自动证明安装器全部语义安全。当前 registry 有 VBF 中文 v5、夏日中文 DLC v2.02.1、夏日中文本体 v1.03 三条审核条目，其他 EXE 不因格式相同获得安装权。

对应代码：[exe_profiles.py](../../../apps/decky/backend/exe_profiles.py)、[exe_installation.py](../../../apps/decky/backend/exe_installation.py)、[exe_worker.py](../../../apps/decky/backend/exe_worker.py)、[sf95_audit.py](../../../scripts/sf95_audit.py)、[sf95_worker.py](../../../scripts/sf95_worker.py)、[install.rs](../../../crates/core/src/install.rs)。

## 2. 确认安装与事务结束

```mermaid
flowchart TD
    A[查看已准备计划] --> B{取消 / 过期?}
    B -->|取消| C[discard_plan<br/>撤销 planned，释放租约]
    B -->|过期| D[拒绝 apply，重新准备]
    B -->|继续| E[用户确认关闭游戏并暂停 Steam 更新]
    E --> F[start_install_task<br/>从持久状态解析实例 / release / plan]
    F --> G[重新扫描可信实例<br/>核验 receipt 身份、租约与有效期<br/>重取 profile / candidate / 订阅状态]
    G --> H[确认与条款摘要一致<br/>语言、DLC、基线事实一致<br/>catalog / 公钥摘要及载荷绑定一致]
    H -->|失败| DENY[拒绝写入，报告具体原因]
    H -->|通过| I[Rust 获取 journal / 游戏协作锁<br/>核验 planned、期限、阻止项<br/>stage 摘要、目标原文件、空间]
    I -->|失败| DENY
    I -->|通过| J[持久化 applying 与逐文件进度<br/>先保存原文件 backup<br/>通过临时文件写入并原子替换 / 新增]
    J -->|全量校验成功| K[committed]
    J -->|写入错误或受控取消| L[按 journal 尝试恢复]
    L -->|成功| M[rolled_back]
    L -->|冲突或恢复失败| N[needs_recovery<br/>保留日志与备份]
    J -->|进程中断| O[重启发现持久事务<br/>由 recover 恢复]
    O --> L
    K --> P[后端释放计划租约<br/>按设置清理下载 / 转换缓存和 stage<br/>EXE 原文件备份继续保留]
    K & M & N --> UI[UI 根据实际事务状态反馈<br/>不以 RPC 成功或取消请求等同成功 / 回滚]
```

apply 使用 Rust 在准备阶段保存的 `.stage`，不重新读取 `normalized.zip`；因此删除转换缓存不能使一个计划失效。准备取消必须可靠撤销计划或本地授权租约。安装取消与提交存在竞态，应读取真实终态；崩溃恢复不依赖进程内状态。

对应代码：[main.py](../../../apps/decky/main.py)、[bridge.py](../../../apps/decky/backend/bridge.py)、[tasks.py](../../../apps/decky/backend/tasks.py)、[index.tsx](../../../apps/decky/src/index.tsx)、[install.rs](../../../crates/core/src/install.rs)。

## 3. 卸载与恢复（含当前界面缺陷）

```mermaid
flowchart TD
    A[已提交事务 committed] --> B{界面判断 backups 是否为 false?}
    B -->|否| C[显示卸载按钮]
    B -->|是| BUG[当前 UI 隐藏卸载<br/>P1：纯新增事务也被误判为不可卸载]
    C --> D[用户确认关闭游戏并暂停更新]
    D --> E[uninstall RPC<br/>解析持久事务与对应 Core]
    DIRECT[直接调用合法 uninstall RPC] --> E
    E --> F{包含原文件替换?}
    F -->|是且没有备份| DENY[拒绝：uninstall_backups_missing]
    F -->|是且有备份| G[Rust 标记 uninstall_requested<br/>持久化 uninstalling]
    F -->|否：纯新增| G
    G --> H[对全部变更先预检<br/>目标只能是原内容或本次安装内容<br/>必要备份 / 临时文件必须有效]
    H -->|冲突 / 损坏| N[needs_recovery<br/>保留用户变化、日志与备份]
    H -->|通过| I[逆序恢复<br/>替换项恢复原文件<br/>新增项仅删除仍与安装摘要一致的文件<br/>移除本次创建且仍为空的目录]
    I -->|校验成功| U[uninstalled]
    I -->|恢复错误| N
    R[重启或用户点击恢复未完成操作] --> S[发现各持久 state 的 journal<br/>跳过 planned / rolled_back / uninstalled<br/>跳过正常 committed]
    S --> H2[对未完成事务加锁并按 journal 恢复<br/>执行同样的完整预检与逆序恢复]
    H2 -->|冲突 / 失败| N
    H2 -->|成功且请求过卸载| U
    H2 -->|成功且安装未提交| RB[rolled_back]
    N --> FIX[人工处理冲突后重新 recover<br/>不强制覆盖用户变化]
    FIX --> S
    classDef issue fill:#fff0f0,stroke:#b42318,color:#7a271a;
    class BUG issue;
```

后端与 Rust 已支持纯新增事务卸载，VBF 的三个新增文件不需要原文件备份。当前 UI 仍只检查 `backups`，两个卸载入口都被阻止，详见审阅 P1。卸载 / 恢复使用持久目录与 journal，不要求官方来源仍启用；再次准备 / 安装仍要求现行审核与来源条件成立。恢复不是无条件覆盖：预检通过后，每次写入前仍会检查目标内容。

卸载目前是同步 RPC。施工记录已记载夏日 DLC 的 5,865 个备份恢复约耗时两分钟、客户端超时但后端完成；这是待改善的进度与超时体验，超时不能作为真实事务终态。

对应代码：[index.tsx](../../../apps/decky/src/index.tsx)、[共享 Transaction 类型](../../../packages/ui/src/index.ts)、[main.py](../../../apps/decky/main.py)、[install.rs](../../../crates/core/src/install.rs)。

全过程不执行官方 EXE、Lua、DLL 或 Wine / Proton 安装器。Rust 是游戏文件安装与恢复的写入者；“静态提取成功”“文件事务成功”和“进入游戏后补丁生效 / 存档兼容”是不同验收层次，后一层仍需真实游戏证据。
