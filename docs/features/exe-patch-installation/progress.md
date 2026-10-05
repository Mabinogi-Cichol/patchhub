# EXE 补丁安装施工记录

- 设计：[design.md](design.md)。调研：[research.md](research.md)。P0 审计：[p0-audit.md](p0-audit.md)。验收矩阵：[acceptance.md](acceptance.md)。
- 状态：P1–P4 已实现并自动化验证；2026-10-05 在真实 Deck 上用当前源码复跑 Linux Rust 55 项、后端 201 项与隔离合成事务闭环，并实机复核 P4 QAM 拆字修复；对 VenusBlood FRONTIER（AppID 1189440）完成首个不同引擎的审核 profile 与真实下载→安装→恢复闭环。夏日两份样本的完整事件语义与夏日适用实例/实体手柄待验收。生产 registry 仅含该经审核的 VBF 条目，目标继续进行，未宣称全部完成。

## 任务状态

| 工作 | 状态 | 证据与边界 |
| --- | --- | --- |
| 现有安装链路及工具调查 | 已调研 | Python 编排 + Rust ZIP 事务；候选安装拒绝、工具能力及源码局限见 research |
| 官方样本格式和静态提取 | 开发机实验通过 | 中文 v1.03、英文 v3：Setup Factory 9.5；没有执行安装器或游戏 |
| 开发计划与功能归类 | 已编写 | design、research、progress 与功能索引 |
| P0 语义、目标条件与设备资源补足 | 部分实现并自动化/实机验证；门槛未满足 | 专用只读 parser 与 worker 最新 17 项自制测试及两份官方样本载荷校验通过；Deck Python/bwrap、网络隔离、取消与资源通过；当前实例已安装 DLC，CRC 不符，完整语义尚未审核 |
| P1 能力与契约 | 已实现并自动化验证 | 受审 profile 独立 registry、EXE release、来源身份与有效期守卫；正式 registry 为空 |
| P2 受限提取与转换 | 已实现并自动化/Deck 合成验证 | 固定离线 bwrap worker、写前完整清单、流式校验和确定性 ZIP；官方语义授权未开放 |
| P3 事务接入 | 已实现并自动化/Deck 合成验证 | prepare/apply/进程重启/来源停用后 uninstall、冲突 recover、取消闭环通过；真实游戏未安装 |
| P4 UI、依赖与打包 | 已实现并自动化验证，待界面实机验收 | EXE 条款/确认/强制备份、进度、撤销能力、小屏滚动、离线包；实体手柄待验证 |
| P5 Steam Deck 验收 | 待验证 | 安装、运行、生效、存档、恢复、语言/DLC 组合和手柄均无本轮设备证据 |

## 2026-10-05

- 使用 multi-agent 组织两个只读调查：现有安装链路和第一方格式/工具资料；主 Agent 负责样本实验和文档。后续对 Setup Factory 调查追加同一个格式 Agent 的任务，没有创建新的实现线程。
- 保留现有 Kagura 发现功能等工作区未提交改动；建立 `exe-patch-installation` 独立目录，在功能索引追加本功能。
- 官方中文/英文样本前 1 MiB 均取得 HTTP 206，品牌字符串指向 Setup Factory。进一步下载完整样本，通过 manifest 和 overlay 签名核实 9.5，完整字节大小及 SHA-256 见 research。
- 审查 `sfextract` 源码与依赖，固定研究源码 commit。首次受限 probe 不允许实际 LZMA2 压缩而退出；新增受限 LZMA2 后改用新输出目录完成两份静态提取。研究脚本自读 PE 信息并复用 overlay parser，不声称原版 CLI/完整依赖或生产安全已通过。
- 中文得到 2,507 条记录，其中 2,502 个游戏文件，游戏载荷 152,368,950 字节；英文得到 2,897 条记录，其中 2,892 个游戏文件，游戏载荷 167,968,393 字节。逐成员声明大小、CRC（存在时）及 SHA-256 复核通过。五个运行时/元数据/界面资源未被当作游戏文件。
- 中文安装元数据局部调查发现 `Game.exe` 和 `Script.vdf` CRC/Steam 版条件；据此把“完整审核并转写前置条件”加入 P0，不能直接复制解包目录。
- 编写 P0–P5 开发阶段、release 信任边界、写前路径与资源校验、规范化 ZIP 桥接、备份要求、依赖打包和验收标准。所有方案项保持待实现状态。
- 第三个只读 Agent 独立审查设计、代码和实验摘要；补足提交后按实时全局设置删除备份的竞态，明确计划级持久保留策略、重启/手动清理边界和来源停用后的本地恢复。修订后未发现待修复的可操作问题；不代表实现安全或设备验收已经通过。
- 研究 EXE、源码、提取文件和报告仅放在忽略目录 `.artifacts/exe-patch-research/`。没有运行 Windows 安装器、Lua、提取出的游戏代码或修改游戏文件；没有修改安装实现、构建/部署插件或提交 Git。

## 本轮检查

- 开发机静态样本实验：上述两份样本通过，不等于功能自动化或实机安装通过。
- 文档相对链接和功能索引检查通过；两份 EXE 及提取器源码 ZIP 的 SHA-256 与调研文档一致。`git diff --check` 通过，新建三份文档另行检查无行尾空白或冲突标记。
- 独立只读审查通过；实际创建三个子 Agent，路线为 GPT-6 Luna / low（代码调查）、GPT-6 Sol / medium（格式调查）、GPT-6.1 Sol / high（风险审查），均只传递任务上下文。
- Python/Rust/TypeScript 项目测试、前端构建、生产 extractor 安全测试：未执行，本轮仅新增计划与调研文档。
- Steam Deck 安装、启动、生效、卸载恢复及实体手柄：未执行，待验证。

## 待办与遗留问题

- P1–P4 工程按合成样本推进；官方 profile 开放仍必须完成 P0 完整语义与适用真实实例的 P5，具体顺序见 design。
- SF7/8、其他 SF9/SF10、DLC、其他 Kagura 游戏和安装包版本没有本轮样本提取或安装证据。
- 官方镜像内容等价性未检查；本体补丁不可用于对应 DLC/GOG，语言/build 不匹配不可跳过条件。
- 本轮样本可提取的结论仅适用于 research 所列 SHA-256；未知更新需要重新审核。

## 2026-10-05：执行 P0 开发计划

- 新增 `scripts/sf95_audit.py` 与固定格式参考许可，选择标准库专用 parser 维护路线。按 PE 资源目录读取 manifest，独立验证 overlay、证书边界/对齐、完整文件表及路径，再流式校验游戏载荷长度/CRC/SHA-256；只输出受限报告，不写出载荷、不执行 EXE/Lua/DLL。
- 不再沿用上游忽略文件条件的 parser 行为：报告保留条件、安装类型、包、快捷方式及未解析字段的原始 bytes/base64/摘要。文件表外的完整事件序列尚未解析；所有成功报告仍标记 `discover_only`、`installable=false` 与未审核阻止原因。
- 新增 12 项合成测试，Windows 开发机及 Deck 系统 Python `-I` 通过；测试加入 Windows/Linux CI，远端 Actions 待验证。没有将官方 EXE/游戏载荷加入 Git 或插件。
- 两份原 EXE 摘要与 research 一致；中文/英文全部 2,502 / 2,892 个游戏文件逐成员大小、CRC、SHA-256 通过，与旧研究清单完全一致。全部文件表条件为空、安装类型为 `All`，但不据此推断全部安装语义安全。
- 用户确认设备可连接且已安装游戏后，通过既有 Devkit 只读检查：AppID 1227890，build 6270017，语言 schinese，安装了 DLC 1390150 的两个 depot。`Script.vdf` CRC 为 2002373744，中文本体样本局部条件要求 3876755782；不匹配，且 DLC 违反原设计范围。不卸载/禁用 DLC，不修改游戏，不生成 profile/安装计划。
- Deck Python 3.13.5、lzma/resource 与 bwrap 可用。独立无网络 sandbox 内无游戏/state 挂载；外网连接拒绝，取消后四个受控进程全部停止。两份样本 worker 峰值约 165 / 170 MiB，审计约 3.3 / 2.5 秒，保留原设计预算。该探针不代表生产 TaskManager/取消接入完成。
- 首次 Linux 文件报告因输出文件限额为零失败，调整为 16 MiB、补充 CLI 重定向回归后通过。取消探针第一次按进程组跟踪漏掉新会话后代，修正为父子关系后通过，保留失败记录而不当作成功证据。
- 游戏 `Game.exe` 与 `Script.vdf` 在检查后再次读取，摘要与检查前相同。未做全游戏目录审计；worker 无游戏挂载，不把这项关键文件检查扩称全量审核。
- 更新 design 的 parser/依赖维护决策，新增 p0-audit 文档，更新功能索引。保留既有 Kagura 功能未提交修改；本轮没有提交 Git、修改安装 RPC/UI、构建/部署插件或实施真实安装。

### 本次验证与停止条件

- 只读审计自动化：12 项通过；官方两份样本开发机及真实 Deck 通过。许可与新文档相对链接、`git diff --check` 通过。
- 系统 Python 隔离/资源/取消探针：该设备通过，详细证据与剩余边界见 [p0-audit.md](p0-audit.md)。
- Rust/TypeScript 和插件构建：本轮未修改这些实现，未运行；生产提取、规范化 ZIP、事务、UI 和打包未完成。
- 真实安装、启动、生效、存档、恢复与手柄：未执行。当前语言/DLC 实例不得用本体样本进入 P5。
- **P0 仍未达门槛**：完整事件与未知文件字段审核、适用本体实例的真实基线/验收仍缺失。依 design 的 P0→P1/P2 顺序与停止条件，P1–P5 保持待实现/待验证，不将本轮工具标成安装功能完成。覆盖当前 DLC 实例需要新增相应官方样本和独立范围审核。

## 2026-10-05：持续目标与 P1–P4 实现

- 用户要求设立目标直到全部计划完成，并明确要求 multi-agent。建立持续目标；调整 design 的工程依赖：P0/P5 是具体官方 profile 开放门槛，独立工程可使用自制样本推进。上节停止记录保留为历史，不再沿用为工程停止理由。
- 成功创建服务（GPT-6.1 Sol/high）、界面（GPT-6 Sol/medium）、审查（GPT-6 Sol/high）共 3 个 agent；最初 GPT-6.1 Sol/high 审查创建尝试因容量失败。额度/容量错误后按用户要求恢复原任务继续执行，持续分配 Linux 验证、会话测试、官方语义调查与设备交付，没有因此把任务交回用户。后续服务 agent 遇工具 thread limit 时，把待做设备交付分配给仍可继续的界面 agent（tester 范围）。
- P1：增加版本受控独立 profile registry 与摘要绑定 EXE release。精确绑定官方来源记录、AppID、语言/variant/版本、页面、候选有效期/订阅、EXE/元数据/载荷摘要、已验证 build、DLC/基线与条款。未审核/过期/漂移或伪造候选维持引导；生产 registry 为 schema 1、零条目。
- P2：共享下载器增加独立 EXE 缓存、大小及预期 SHA 锁定，每跳校验域名、取消和原子发布。固定 `/usr/bin/python3 -I` worker 在非特权 bwrap 中无网络、无游戏/state 挂载；完整清单验证后流式解压，以确定性 ZIP_STORED 写入规范化包，双遍摘要对应且限额独立。未知格式/路径/CRC/长度/尾部均拒绝，清理只处理本次拥有的产物。
- P3：服务从可信 Steam manifest 读取 build/语言/DLC 与 Game.exe、Script.vdf 基线；在 state 独立工作目录生成本地签名 ZIP 配方，再调用 Rust plan，TTL 从此开始。计划策略记录原 EXE/ZIP 两份摘要、来源/最终 URL、profile、条款确认、条件事实、租约与强制备份；apply 前重新扫描/复核。来源/profile 停用不阻止既有本地事务恢复。
- 接入 main 的 EXE prepare/preview/apply/ID 分发与任务阶段；任意 EXE 路径/URL不能进入导入或直接 fetch/plan。重启、全局备份开关、自动/手动清理都不能删除 EXE 备份；post-commit 清理失败不改写已提交结果。过期 planned 可安全 discard 后重入；缺/坏 receipt 的缓存默认保守保留，仅在有界只读 Rust journal 的匹配终止/过期证据下回收。
- P4：EXE 显示语言/版本/条款及三项明确确认，准备前启用备份；任务支持提取/转换/签名计划阶段与中文错误。发现投影按 release 去重，来源撤销、stale/失败刷新会撤销旧 EXE 能力，旧请求不会清除新扫描结果。窄屏内容可滚动、任务返回按钮独立底栏；这属于代码检查，尚无真实 Deck UI/实体手柄证据。
- 独立审查发现并修复候选 ISO 时间格式、Windows fstat/path ctime 差异、失败 worker 删除预存输出、过期计划永久阻塞、post-commit 异常误报失败、缺/不完整 receipt 缓存租约问题。Windows 的 fd 前后完整身份仍检查，path 对照采用稳定身份字段，Linux 保持 ctime 检查。

### 自动化与打包证据

- Windows 后端回归：190 项通过，3 项链接检查因本机创建权限跳过；其后新增 EXE 下载锁定/大小/取消/重定向测试 4 项通过。最新缓存迭代补跑相关 32 项通过（1 项 Windows 链接 skip）。SF95 审计与转换更新为 17 项通过。
- Decky `npm run typecheck`、`npm test` 43 项、`npm run build`、`npm run package:plugin` 通过。最终包内 32 个文件，包含固定审计/转换代码、项目 LICENSE、参考 MIT 许可与空正式 registry；无 tests/fixtures、EXE/游戏载荷，关键源码与包内 SHA 一致。SBOM 新增维护 worker 的固定版本、脚本/两份许可摘要和参考 commit；合成锁文件测试通过，加入 Windows/Linux CI。真实完整 SBOM 生成尚缺本机 Cargo metadata，不声称已完成。
- 最终前端 `dist/index.js` SHA-256：`4f1e4ded7d522ddcbff76962c2b701c92b85306bc8aafb6b5a3bac809cab7d4c`。包内审计 v2 SHA：`393566a63810638523d5109a46aa85aae88e94dd71d189dfcccf11252ba4125e`；worker SHA：`29c212654bf50908ac1da14a61fc6142f301694ec83396c2479e6c2ffdd10d7e`。远端 Actions 尚未执行，不把本地构建算作托管 CI 通过。
- Deck 独立合成库第一轮：Linux EXE 后端 48 项、SF95 15 项全部通过且无 skip；包内真实 bwrap worker → Rust prepare/apply committed、跨进程重启、来源停用后 uninstall、外部修改冲突 recover、实际提取取消均通过。随后全部 Linux backend 197 项通过（含实际 CoreBridge 3 项与新增 EXE 下载 4 项，无 skip）。首次上传漏 vendor 的 6 项失败、wrapper 将测试名称误判为 skip 的错误分别修正，保留失败日志；不把失败 run 标为通过。下载传输用合成复制代替网络；这不构成官方补丁/P5 通过。
- 最终缓存/审计 v2 的 Deck 补核在连接恢复后完成：17 项审计/worker、32 项缓存/RPC 无 skip/ResourceWarning；新 HOME 完整真实提取/安装/重启恢复/冲突/取消再次通过。准备不改变任何游戏载荷或存档；Rust 首次 plan 创建空 `.steam-patch-manager.lock`，首次快照单独排除此已知协作锁，后续取消完整快照含锁也不变。同步 design 的审计边界，未改写核心行为。
- 最新独立目录 `/home/deck/steam-patch-validation/exe-p3-synthetic-20261005-c4dd8ef9`，本地证据 `p3-device-c4dd8ef9/`。integration JSON SHA：`fcdc7d0d9a78243d031a6330cb29a9426b23d7052f30d803731fc827084f7016`；restart JSON SHA：`ebaa91240d5b2e11f16977eb8875687d8c1543c229842416980b99adbed6e477`；17 项日志 SHA：`7a947fb104fa692d144968bd8c3a181ba12891733694c1d44071c260317a0606`；32 项日志 SHA：`d188a3353d064ae5f639233b0d753ac0b20ff1833024778a0ccda9af9ae11e7a`；原全 197 项日志 SHA：`f30232baa278b20c4c7e91856940a51013b591da3aaf1eb611145f28e20cba7a`。
- 独立设备目录：`/home/deck/steam-patch-validation/exe-p3-synthetic-20261005-10fffb7c`；本地证据 `.artifacts/exe-patch-research/p3-device-10fffb7c/`。CLI SHA：`0c125fd0196ed1c169d5734194eb5511dfed9c6b66cc29ea05fefd8dfc105188`；合成 EXE SHA：`5ef4fdcb843884ad2810e3c4203611b79370d31d0a78ccd45be60c598d3512bc`；合成 ZIP SHA：`633d739842cd8bb9253ef6e1c32f1b1d0f8ec62b7338b9aa31e5ab29c1a22bc9`。`0.879s` 是整段合成测试用时，不是官方准备或安装性能指标。

### P0 语义调查补充与 v2 边界修正

- 两份安装元数据探索性识别出 59 个事件框，事件顺序与脚本文本摘要相同。该扫描尚不是完整反序列化，不能只凭字符串/框数量证明不存在其他动作。
- 独立审查确认原参考 parser 在 CRC 后固定跳过 8 字节，跨越 MFC 对象边界；原 v1 `serialized_record` 和最后表结束位置含后续对象的字节。因此 v1 的载荷校验仍有效，但不能把其文件记录边界当作已审核序列化语义。
- v2 将记录证据截于最后已知 CRC；非末条间隔单列为未解释 framing；末条后全部 bytes 保留在未知尾区。`end_basis` 明确这是最后已知 CRC 结束而非对象结束。新增边界及预存输出归属回归，17 项脚本测试通过；两份官方样本只读重审，所有目标/大小/CRC/载荷偏移/摘要以及 EXE/元数据证据与旧报告完全一致。
- 后续相关性证据表明 CRC 后首字节可能是当前文件尾字段，其余 7 字节含已注册类引用和下一对象前缀；因此“间隔 framing”仅是未知边界 bytes 的保守保存标签，并不声称 8 字节全属于下一对象。没有把反相关的首字节猜成已审业务开关，也没有据此授权覆盖行为。
- 中文/英文末 CRC 结束偏移分别 `1383485` / `1590969`，未知尾区 `68441` / `80370` 字节。新报告保存在忽略目录 `p0-semantic-20261005/{cn,en}-audit-v2.json`。正式 profile 继续为空，完整 framing、文件属性/替换策略及表外语义仍在调查。
- 新建互斥字节分类账本：解析字段、未知字段/边界、全局 Lua、事件正文、类 tag、会话变量和剩余区域完整覆盖原元数据，避免漏区/重复计数；这是覆盖账本，不等于语义完成。两样本独立对齐得到相同 54 个 `CSessionVar` 键/值；尾随 0/1/2 开关含义未知，Package01 对象只解析部分名称/语言字段。
- [Microsoft TN002](https://learn.microsoft.com/en-us/cpp/mfc/tn002-persistent-object-data-format?view=msvc-170) 说明 MFC 类/对象 PID 与引用形式，支撑文件及事件类 tag 的结构判断。[文件属性指南](https://www.indigorose.com/docs/suf/users_guide_chapter_2_working_with_files.htm) 与 [包指南](https://www.indigorose.com/webhelp/suf9/Users_Guide/Chapter_6_Packages.htm) 表明覆盖、条件和高级行为需单独处理；未分配包的成员不代表可以省略其他属性。原 SF9 文件指南入口现跳转 SF10，不能把新版选项说明当作 9.5 byte schema。公开文档未给本样本类内 byte schema，常量 bytes 不能被猜成已审核策略。完整报告、一手链接及原型位于研究目录，静态 runtime 解包/字段定位继续进行。

### 真实实例只读基线

- 用户后续实例选择尚未返回，未擅自改变 DLC/语言/游戏安装。仅补充对现有目录的有界只读全量基线：7,110 个普通文件、873,511,015 字节，无链接/特殊文件；前后目录路径/类型/大小/dev/ino/mtime/ctime 和 manifest 一致，每文件哈希读取期间无变化。读取使用 O_RDONLY/O_NOFOLLOW/O_NOATIME，报告仅写独立验证目录；本轮采集 4.856 秒。
- AppID 1227890、build 6270017，UserConfig/MountedConfig 均 schinese；已安装 DLC 1390150，不据此推断购买权。Game.exe、Script.vdf 摘要与原 P0 核验一致；manifest SHA：`f27db86ea35853d57c839205a98b7832e688a6f60f92fc03c84c2f2374f8d867`。
- 树 SHA：`2dc6b4a8d523510bcbdb9e2444919c662202d05bfa070c1d4e0fdc8c8b00d27d`；原始报告 `.artifacts/p5-readonly-20261005-1aa7d26d/summer-memories-readonly.json` SHA：`60a286e7fecac2873b11097a74ddd5723d97600abc569f01de714c64b3364d78`。它是采集时刻的基线，不能替代官方适用性/安装生效验收。
- 共享 UI 类型变更后的 Windows 客户端 `npm run typecheck -w apps/windows` 通过。SBOM/打包再增加源码、两份许可和实际 Decky 包的摘要一致性门槛，过期清单/旧 worker 包在写 ZIP 前拒绝；打包与 SBOM 合成测试共 8 项通过，原无 SBOM 流程保留。

### 尚未完成

- 官方两份样本的完整安装事件、未知字段和替换策略语义仍在独立调查；正式审核条目未开放。
- 当前真实 Deck DLC 实例不适用于本体样本。已向用户询问使用符合条件的本体实例，或明确扩展到 DLC 官方样本；相关验收保持待验证。
- 真实游戏安装后补丁生效、退出/存档、全文件恢复、外置库/满盘及实体手柄尚无本轮验收证据。隔离合成库不能代替这些项目。
- 保留原工作区 Kagura 功能等未提交内容；未部署真实插件、运行官方 EXE/Lua、修改真实游戏或提交/推送 Git。

### 子 Agent 持续推进中的证据

- 按用户要求，原审查与界面 Agent 继续负责各自剩余工作；主 Agent 仅协调、复核和记录，未接手静态语义实现或实机部署。
- 审查 Agent 在研究副本上静态解包 runtime 并定位 `CSetupFileData::Serialize` 与属性导出。两样本全表的 Overwrite=2、InstallOrder=1000、NeverRemove=0 已与相应对象字段交叉核对；覆盖决策及其他高级字段、系统条件、项目级动作仍在复核。该阶段性结果不授予正式安装能力，parser 与生产 profile 未因这些发现开放。
- 界面 Agent 恢复设备连接后，通过 Decky RPC 确认没有活跃任务；旧插件 24 个文件、8,594,840 字节已完整备份至 `/home/deck/steam-patch-validation/exe-device-p4-20261005/backup`，逐文件 SHA 与源一致。新包 32 个文件已写入同一验证目录的 `stage` 并逐项核对 SHA，正式 profiles 仍为零。
- 实际插件目录为 root:root 755，非交互 `sudo -n true` 需要密码；Agent 正在核实 Decky 支持的安装入口。以上备份/stage 均在独立验证目录，不代表已覆盖插件或完成 reload。未改游戏、目录权限或 SteamOS 根文件系统。
- 后续界面 Agent 使用 Decky Loader v3.2.9 官方本地 ZIP + SHA-256 安装接口完成更新，无需 sudo；包内 32 文件逐项 SHA 一致，loader 返回本插件实例为 1。仅合并恢复本插件的排序项，其余 hidden/frozen/disabled 无变动；独立 state 目录 52 文件摘要全部未变，备份与回滚 ZIP 保留。新后端 scan/RPC、前端重导入、真实屏幕和完整实际 SBOM 继续由该 Agent 验证，不把部署成功视作 UI/P5 完成。
- 主 Agent 依据 design 第 4–9 节建立 [验收矩阵](acceptance.md)，复核最新 integration/restart 和 Linux 197 项报告。逐项区分自动化、实际合成事务、官方语义、真实游戏和屏幕/手柄证据；目标仍未达到全部完成门槛。
- 新验收矩阵与功能文档的 59 处相对链接检查通过；`git diff --check` 通过。仅补充协调文档，未重复已通过且实现未变的测试。
- 后续实机运行核验通过：直接 Python WebSocket 被现有 Steam 前端单连接重连关闭，Loader 仍 active 且无后端错误；改用 Steam SharedJSContext 的已有 Decky 连接执行前端 `importPlugin`、scan 与新 `exe_installation_reviews` RPC，前端插件实例为 1、扫描 38 个游戏、审核结果均空、活跃任务为 0。这是实际运行中的前端/后端 RPC 证据，仍不是屏幕/手柄验收。
- 部署结果保存于 `.artifacts/exe-device-p4-deployment-result.json`。部署 ZIP SHA：`974656c8c81b3caf0d77fbe9fd1cb484952b8e0b3799799145a3319f5a0626a7`；回滚 ZIP SHA：`94ce3af6c1cc218a4593379c9d1691c7265f3e3b7957e28501b09faf34fae9ee`。生产审核条目仍为零，未运行官方安装器或修改真实游戏。
- 两个子 Agent 的轮次发生额度错误后，仍按用户要求恢复原 Agent 自行继续，而非由主 Agent 接管。部署 Agent 后续设备检查命令被自动审批以 usage limit 拒绝，命令未执行，提示 17:25 后重试；已完成的部署/RPC 证据不受影响，HTTP bundle/真实屏幕仍未验证，当前继续整理本地证据与评估 SBOM，不绕过拒绝。
- 复核审查 Agent 保存的 report 与两样本字段探针，将 runtime 属性/全表交叉证据、末条边界和剩余未知区域归档到 [p0-audit.md](p0-audit.md)。这只是阶段性静态证据，未改 parser 或授予正式安装能力。
- 后续确认静态审查 Agent 的本地工具可用，项目级调查仍在继续；不将设备命令的额度拒绝扩称所有工作停止。最新功能文档 61 处相对链接与该目录 `git diff --check` 通过。
- 17:25 后审批恢复，界面 Agent 继续原任务取得真实 Steam 全页补丁详情与滚动底部截图；物理图 1281×801、CSS viewport 854×534。主 Agent 已实际查看两图：长名称截断和中文详情可读，底部官方链接/B 提示可见，无横向遮挡；未审核 EXE 只有来源引导，无准备/确认/自动下载按钮。它们是全页证据，不是 QAM 侧栏或实体手柄证据；零正式 profile 下无法触发正式三确认界面，保持待验证。
- 两图位于 `.artifacts/exe-device-p4-patch-page.png` 与 `exe-device-p4-patch-page-bottom.png`，SHA 分别为 `b7f7dd7bbefa9875046d8a7828ecc42c2a43c770845fc1cbdbfd967e17eddaaa` / `6b2219ba6e9d13196b9a6523a044380256297130e7a9c766b5563c278c807568`。HTTP 实际 bundle 摘要与已部署 dist 一致由界面 Agent 报告，具体结果待其归档；尚未按此声称完整 UI 验收通过。
- 复核审查 Agent 新保存的中英 Package prefix JSON，Install/Enabled/AdditionalDiskSpace 三个已定位 DWORD 均为 0；归档局部包证据，其他项目配置继续由原 Agent 审核。
- 后续 CUninstallData prefix 双样本探针与 runtime 分支核对通过，CreateUninstall DWORD（CN1413535 / EN1624000）均为 0；原安装器不会因存在默认卸载路径字符串就创建卸载数据。主 Agent 已读两份 JSON 并归档到 p0-audit；本项目备份恢复要求不变，其他项目动作仍未宣称完整审核。
- 真正 QAM 侧栏后续由界面 Agent 通过 `OpenQuickAccessMenu()` 捕获，主 Agent 已查看 `.artifacts/exe-device-p4-qam-window.png` / `exe-device-p4-qam-bottom.png`：搜索/筛选、默认 4 游戏计数、全局设置与重新扫描可见；CSS viewport 854×454，滚动容器可达底部，无横向溢出。顶部长按钮将“游戏”拆开，交回原 UI Agent 做最小文案/排版修复，再自行构建/打包/部署/实屏复核；不改变扫描/过滤策略，不把 DevTools 导航/滚动称为实体手柄验收。
- completion audit 发现 P3 合成报告尚不能证明权限保留/崩溃/备份损坏全部场景，虽然 `crates/core/tests/install.rs` 已有对应测试，但本轮运行证据未定位。验收矩阵新增独立待验证项，并交原服务 Agent 定位已有记录或补真实 CLI 合成验证；不重复已经通过且没有新改动的 197/17/32 回归。
- 数小时后的真实实例关键事实已由界面 Agent 重新只读核对，主 Agent 读取 `.artifacts/exe-device-p4-game-facts.json`：build 6270017、两处 schinese、DLC1390150 的两个 depot、Game.exe/Script.vdf 大小/SHA/CRC 均与旧基线相同，原本体阻止条件仍有效。未改安装或重扫全树。该报告 SHA：`0c78a4e1630bc3aae16ccd71bdb69dab7f6d67963e04695c15ae47c204f0404d`。
- 修复前两张真实 QAM 图片 SHA：`922ccf8923d8256960fce046fbd9d16b9d4279792580656d63d20a7fb5021986` / `b5ad5363ff3a45fee9e4aab9dbed35ab41f8ee0fcf54edac0afb1ac3258cb609`；最小文案修改已 typecheck/build/package，正在由原 UI Agent 自行部署并复核。已构建新 dist 不代表已部署，本段不替换前述正式部署 SHA。

## 2026-10-05：本机 CI 等价复核与 SBOM 补齐

- 环境：Windows 开发机，Python 3.11.9、Node 24.21.0、项目隔离 Rust 工具链 `.artifacts/cargo-isolated`（rustup 1.94.0，CARGO_HOME/RUSTUP_HOME 指向该目录）。Windows 本机无可用 Linux（`wsl -l -v` 无发行版、无 Docker/Podman），Linux/设备复验改在真实 Steam Deck 上执行，见下一节；本节仅为 Windows 侧证据。
- Rust 核心：`cargo test -p steam-patch-core -p steam-patch-cli --locked` 全部通过（5 + 4 + 6 + 28 + 6 + 3 = 52 项）。其中安装事务测试已定位并覆盖权限保留（`replacing_and_restoring_preserves_executable_permissions`，`#[cfg(unix)]`，本机 Windows 未运行）、崩溃恢复（`crash_after_backup_before_and_after_replacement_recovers`、`interrupted_install_copy_accepts_only_verified_stage_prefixes`）和备份损坏（`corrupt_backup_blocks_recovery_before_other_file_is_removed`、`user_modification_and_corrupt_backup_block_the_entire_restore`）。
- `cargo fmt --all -- --check` 与 `cargo clippy -p steam-patch-core -p steam-patch-cli --all-targets --locked -- -D warnings` 通过。使用构建出的 `target/debug/steam-patch.exe` 后，后端 197 项全部通过（3 项 Windows 链接检查按设计 skip），此前缺 CLI 的 3 项 bridge 集成错误消除。
-  `scripts/test_sf95_audit.py` 13 项、`scripts/test_sf95_worker.py` 4 项、`scripts/test_package_artifacts.py`、`scripts/test_build_sbom.py` 通过。
- 前端 `npm run typecheck`（windows + decky）、`npm run build:decky`、`npm test -w apps/decky` 43 项通过。`apps/decky/scripts/package-plugin.mjs` 需要 `target/linux/release/steam-patch`，本机不能构建 Linux 产物，未运行打包；沿用此前 Deck 上 32 文件包证据。
- 真实完整 SBOM：用已构建工具链生成 `.artifacts/cargo-metadata.json` 并运行 `python scripts/build-sbom.py`，得到 727 个组件、`license_metadata_missing` 为空，输出 `.artifacts/dependencies.cdx.json`。此前“缺本机 Cargo metadata”的缺口已补齐；该 SBOM 覆盖全部已解析 Rust 依赖、锁定 npm 依赖与内置 SF95 worker，不做可达性分析。
- 本节未运行官方 EXE/Lua、未修改真实游戏、未提交 Git；P0 完整语义与 P5 真实验收仍待外部条件。按仓库规则，文档改动与后续代码/部署拆分提交。设备上的 Linux 全量复验见下一节。

## 2026-10-05：Steam Deck Linux 全量复验与 P4 界面实机复核

- 连接：通过既有 Steam Devkit 密钥经 SSH 连接 `deck@192.168.31.186`（SteamOS 6.18，系统 Python 3.13.5，x86_64，`/usr/bin/bwrap` 可用）。设备上无预存 C 编译器，使用隔离目录 `tools/`（不写入 SteamOS 根文件系统）：Zig 0.14.1（`zig cc`/`zig ar` 作为 C 工具链）与 rustup 1.94.0（`CARGO_HOME`/`RUSTUP_HOME` 均指向验证目录）。工作区源码 tar 上传至 `/home/deck/steam-patch-validation/exe-ci-20261005-1a2b3c4d/`。
- 为让 `ring`/`rusqlite` 等通过 Zig 编译，`cc` 包装器过滤掉 Rust 传给 C 编译器的 `--target=x86_64-unknown-linux-gnu`（原生构建）。此为设备验证脚手架，不进入仓库或插件。
- Linux Rust 核心：`cargo test -p steam-patch-core -p steam-patch-cli --locked` 全部通过（5 + 4 + 7 + 30 + 6 + 3 = 55 项）。其中 `replacing_and_restoring_preserves_executable_permissions`、`crash_after_backup_before_and_after_replacement_recovers`、`corrupt_backup_blocks_recovery_before_other_file_is_removed`、`user_modification_and_corrupt_backup_block_the_entire_restore` 均在 Linux 实际运行通过。日志 `evidence/core-tests.log` SHA：`9bbef321b004e9b00591f303d5d0b7a51c7932d156ec6b8e31bf3ad0c486b9d9`。
- 构建 Linux CLI（`cargo build -p steam-patch-cli --locked`）：SHA-256 `ca67023c93a21e0716351e3ed1fe2a30f3a1d165f0e816525661e01c2702525e`。`scripts/test_sf95_audit.py` 13 项、`test_sf95_worker.py` 4 项通过（日志 SHA `95c42c4f0cdca8d7d878a77cbc35d7b0819568a7c0cc356abaf57a3ad6f8a13b` / `66d6e314674f6e6ea71cf48674237688347597b068c9457453dcd6ce6dfe124b`）。
- 在合成 HOME 下运行完整后端 197 项，Linux 无 skip 全部通过；日志 `evidence/backend-all.log` SHA `cdff160f851098187a9e3549a8dfd86cd5c9b06d537229b03b2cf2840190dc7c`。
- 真实 bwrap worker + Rust 合成事务闭环（`exe-p3-synthetic-20261005-ci1a2b3c4d`）：prepare→`committed`、进程重启 uninstall、来源停用拒绝 apply 且可本地恢复、外部修改冲突保留、recover→`uninstalled`、存档保留、worker 运行中取消且游戏快照不变，全部通过；`integration-result.json` SHA `94ee9299a9162826bc8341ff2bcf415cc787dc0422a656564aa2092a117fe887`，`restart-result.json` SHA `2c6cc567fb747252bb696d37f6caad102fcb9f674d3b83e89d9881351728b2b4`。合成 `--all-tests` 再跑 197 项、`--related-tests` 32 项（无 ResourceWarning）通过。正式注册表在本轮仍为零，`official_profile_approved=false`；这是合成样本，不是官方补丁验收。
- P4 界面实机复核：设备上部署的 `dist/index.js` SHA 为 `c7243259086c5a52f7ba8c4eeebbf43a0f3ba85da6283610c37b8dddf16e7e77`（与本地修复版一致）。运行中的前端此前仍为旧 bundle，通过 `SteamClient.Browser.RestartJSContext()` 重载后，用 Steam CEF（`127.0.0.1:8080`）打开 QAM 并进入插件页，实测 `oldButton=false`、`newButton=true`，界面文本为独立的“还有 31 款未查询。”状态行与“查看全部游戏”按钮，不再出现“游戏”被拆行。截图 `.artifacts/exe-deck-ci/qam-final.png` SHA `1b99872d73d27201f97df4b854d641501754a18ee2684f2b512b3c711a1f92fe`。QAM 已用 `DFL.Navigation.CloseSideMenus()` 关闭。
- 设备操作仅限独立验证目录与插件前端重载；未运行官方 EXE/Lua、未修改真实游戏或目录权限、未改动 SteamOS 根文件系统、未提交 Git。P0 完整语义与 P5 适用真实实例仍未完成。

## 2026-10-05：Venus Blood Frontier 真实 EXE 安装 P0/P5 闭环

- 用户提供 Steam 已安装实例 VenusBlood FRONTIER International（AppID 1189440，schinese，build 5430305，无 DLC，内置盘；主程序 `VBFI.exe`，KiriKiri 引擎）。Kagura FAN 官方中文补丁 v5 为 Setup Factory 9.5 EXE，直链来自白名单 `kaguraserver.com` / `kaguragamespatch.com`。
- 真实发现与身份绑定：Kagura CN 适配器对该页面得到 `game_appid=null`、`match_basis=title`；HGamer 兜底索引给出 AppID 1189440 且 upstream 页与官方页一致，`SourceRegistry` 合并后官方候选提升为 `game_appid=1189440`、`match_basis=official_title`、`version_identity=5:VenusBlood-FRONTIER-CN-Patch-v5.exe`、`languages=[zh-CN]`、`variant=base`。未使用标题或 URL 后缀单独授权。
- P0 静态审计（只读，不执行 EXE/Lua）：`exe_sha256=502063d21d2e7415…`、大小 1,126,694,464；`metadata_sha256=5a0ca3e10199a457…`；仅 3 个游戏根目录成员 `extra.xp3`（584,428,965）、`extra2.xp3`（648,789,888）、`R-18 Patch readme (CN).txt`（1,580），条件全空、安装类型 `All`、无包；展开总量 1,235,314,094。事件只按 `VBFI.exe` 定位、推进界面、取消时 `Application.Exit`，以及通用关机重启助手；framed 文本中无 `File.Copy/Delete/Run/Move`、无 `Web.*` 下载、无注册表写（`HKEY_` 命中为常量定义与区域设置读取）。报告在忽略目录 `.artifacts/exe-patch-research/vbf/`。P0 剩余的项目级序列化字节仍按保守边界记录，未据此扩大任何其它样本的授权。
- 工程改动：
  - 解析器缺陷修复：接受根目录 `%AppFolder%`（无结尾反斜杠）目标，`extra.xp3` 等根文件不再误报 `unsupported_target_variable`；新增回归测试。
  - 预算按 Deck 实测调整：`MAX_EXE` 1→2 GiB、`MAX_FILE` 512 MiB→1 GiB、审计/worker `RLIMIT_AS` 1→3 GiB；`MAX_TOTAL` 维持 2 GiB；`exe_profiles`、下载与重哈希上限同步。仍低于核心上限（归档 8 GiB、单文件 4 GiB、总 16 GiB）。
  - profile 基线泛化：校验不再硬编码 `Game.exe`/`Script.vdf`，改由受审 profile 声明必需原文件（VBF 为 `VBFI.exe`）。
  - `reviews` 过滤修正：接受页面未标注但镜像观测为 `exe` 的候选（`format_reported=unknown`/`format_observed=exe`），仍拒绝 zip/mixed 冲突。
  - Rust 事务新增 `has_replacements`（`list_transactions`/`uninstall`/`recover` 从计划计算，旧 journal `serde(default)` 兼容）；Python `uninstall` 允许纯新增事务在没有备份时卸载（无可恢复原文件），替换型无备份仍以 `uninstall_backups_missing` 拒绝。
- 新审核 profile：`profile_id=venusblood-frontier-cn-v5`，release `exe-15da7b6a44639568116b0b42-1189440`，`exe_size=1126694464`、`verified_buildids=["5430305"]`、`requires_dlc=[]`、`excludes_dlc=[]`、baseline `VBFI.exe`（SHA `8ab4e801…`）；`apps/decky/backend/exe_profiles.json` SHA `7c2212a4e9ca1f5e92bea59a8ca363850504d3ae641c0352c57388ddbb2d5586`。生产 registry 首次含 1 条经审核条目。
- P5 设备真实闭环（插件 RPC + 真实网络与 Rust，未运行官方 EXE）：
  - 下载官方 1,126,694,464 字节 → 提取/校验（`normalized_size=1,233,220,833`）→ 计划 3 个 `add`、无 blocker。prepare 证据 SHA `8a3074a8423f8379ad3672e06b4fcdaa0b95eca3ab3e6894b3e6b3adcb7adc71`。
  - 安装 `committed`；安装后 3 文件 SHA-256 与 profile 完全一致，`VBFI.exe`/`data.xp3`/`interface.xp3` 摘要不变。install 证据 SHA `e8304129bb43a19035959b90157ee3662df05e5bd3414140cca1e7362792de83`。
  - 卸载 `uninstalled`、`recover` 为空；3 个新增文件移除、无关文件摘要不变；游戏目录回到 2.1G，仅保留既有空协作锁。uninstall 证据 SHA `867b32ed0f94bbf43cae3b681f3a647a4a9b932720739817bd511668fc9b029c`。
- 当前代码 Linux 复验：Rust 核心测试通过（含权限保留/崩溃/备份损坏）；backend 201、sf95 14+4、真实 bwrap + Rust 合成事务闭环、`--all-tests` 201、`--related-tests` 34（无 ResourceWarning）；debug CLI SHA `e69a1cd85b81e490762d779ef29d24f2663e4ef99a2589bbeb821f7247b3a431`。
- 设备上仅执行受控安装/卸载与前端重载；未运行官方 EXE/Lua/DLL、未执行 Windows 安装器，真实游戏改动已由核心恢复；未提交 Git。夏日两份样本的完整事件语义与夏日适用实例/实体手柄仍待完成。

## 2026-10-05：夏日狂想曲官方 DLC 补丁真实 EXE 安装闭环

- 用户目标是让“夏日狂想曲这类 Setup Factory EXE”可自动安装。来源分为本体系列（CN v1.03 / EN v3，要求 `Script.vdf` CRC 3876755782、排除 DLC）与 DLC/扩展系列（CN v2.02.1 / EN v2.03.0，要求 DLC 1390150）。用户当前实例为 schinese、build 6270017、已装 DLC 1390150、`Script.vdf` CRC 2002373744，因此适用 DLC 补丁而非本体补丁。
- P0 语义证据：夏日安装器类清单（CN/EN 各 18 个）只有文件表、`CIREvent`、界面/控件、语言/消息、`CSessionVar`、`CSUFPackage`、`CPasswordData`、`CImageInfo`、`CGlobalIncludeLuaFile`，**没有任何文件动作/运行类**；非事件字符串只有项目元数据、被禁用的卸载路径和通用错误文案；framed Lua 无 `File.Copy/Delete/Run/Move`、无 `Web.*`。据此确认夏日行为＝文件表复制 + 已解码事件（定位 `Game.exe`、`Script.vdf` CRC 门槛、取消 `Application.Exit`、通用关机重启助手）。
- DLC 补丁 CN v2.02.1 审计：`exe_sha256=de370e86645e273bae205517b49077a9fd682a9d73309c518f5524d96216ecad`、大小 279,515,928；`metadata_sha256=6cc974e6b7a1c997097554acff0d0cb7cb8d1a8f677a38b5f86a4166d6699baf`；**5,874 个 `www/` 成员**（9 新增、5,865 替换）、条件空、类型 `All`、全部未压缩、展开 277,366,632 字节；DLC 屏幕的 `SetPackageProperties` 仍为注释；门槛条件为 `Script.vdf` CRC 2002373744。
- 工程改动：为容纳 5,874 个成员，核心 `MAX_ENTRIES`、解析器、`exe_profiles`、`publisher` 的 ZIP 条目上限从 4,096 上调到 **8,192**（仍远低于核心其它上限，并在 Deck 上真实跑通）。新增审核 profile `summer-memories-dlc-cn-v2-02-1`（release `exe-4fd81a78c401784a42e6ad40-1227890`，`requires_dlc=[1390150]`，基线 `Game.exe` + `Script.vdf` CRC 2002373744）；registry SHA `df541421b17dc25331044c39ecf681ffb13b1b08daa95d7c56e679806d8eb37b`。
- 设备真实闭环（插件 RPC + 真实网络与 Rust，未运行官方 EXE）：
  - 下载 279,515,928 字节 → 提取/校验 → 计划 5,874 个变更（9 add / 5,865 replace）、无 blocker；prepare 证据 SHA `64b4882e5fe88b39e86ce219768ea45d2f69edf79ed07d3f2ee9a06c804f8956`。
  - 安装 `committed`；全部 5,874 个文件安装后 SHA-256 与计划一致，`Game.exe`/`Script.vdf` 未变；install 证据 SHA `e5f59202b453f5d45831487925db2a57376aed21fe4ac51b776241d6bf1409a9`。
  - 卸载：事务 `uninstalled`；**5,865 个替换文件全部恢复原 SHA-256、9 个新增文件全部移除**，`Game.exe`/`Script.vdf` 未变；restore 证据 SHA `46ac3fa4f6dd27b33c4e84bec6acaee09cb72420b20180d328a65489ef92623f`。
- 说明：本事务的卸载为同步 RPC，恢复 5,865 个备份耗时约 2 分钟，期间 loader 客户端连接超时但后端正常完成（核心 `journal.lock` 正确串行化）；大规模事务的卸载进度/超时体验列入待办。
- 本体 profile（CN v1.03 / EN v3）仍需要**未装 DLC**的适用实例来记录基线 `Script.vdf` SHA-256 并做 P5；当前 DLC 实例不得用本体补丁。相关实现与 P0 证据已就绪，待实例。

## 2026-10-05：夏日本体系列 CN v1.03 真实 EXE 安装闭环

- 用户临时在 Steam 中取消勾选 DLC 1390150，Steam 回写本体文件后：`Script.vdf` 大小 504、SHA-256 `5cdc0e696aea4ea526840c1327bbb258846cbc7c703982c6e0aa833420554d6c`、CRC 3876755782（与本体补丁门槛一致）；`Game.exe` SHA 256 `16aef53d…` 不变；build 6270017；已安装 depot 为 1227891/1227892（DLC depot 已移除）。
- 新审核 profile `summer-memories-base-cn-v1-03`（release `exe-f24bb4f4233fac85593eb982-1227890`，variant=base，`excludes_dlc=[1390150]`，基线 `Game.exe` 与 `Script.vdf` CRC 3876755782）；payload 2,502 个 `www/` 文件来自既有 P0 审计（`cn-audit-v2.json`）。registry SHA `73ffdbc22e43f6684c6d7f5a742ebb52eeecfc0427efd09c898a65182585cb6a`，共 3 条 profile（VBF、夏日 DLC、夏日本体）。
- 设备真实闭环（插件 RPC + 真实网络与 Rust，未运行官方 EXE）：
  - 下载/缓存 126,137,104 字节 → 提取/校验 → 计划 2,502 个变更（8 add / 2,494 replace）、无 blocker；prepare 证据 SHA `743a44f5acc23492bdc780127601066fa9f8442bbd34ba6ea4c328a321551ccd`。
  - 安装 `committed`；2,502 个文件全部与计划一致，`Game.exe`/`Script.vdf` 未变；install 证据 SHA `a4cc9e18befc49e2576dc68c0ab6590b62a0b91f1e58a89ea88ffe350646da35`。
  - 卸载：事务 `uninstalled`；2,494 个替换文件全部恢复原 SHA-256、8 个新增移除、无关文件不变；restore 证据 SHA `d28bc21251c3f5a2a29c52d49ad8a823009211e875c498140dbae027c8194d6b`。
- 本轮曾因上一次中断留下一个 planned 计划触发 `publisher_existing_transaction`，用 `discard_plan` 丢弃后重试成功；保留该失败记录，不当作成功证据。
- 至此夏日本体（CN）与 DLC（CN）均可自动安装。用户可重新勾选装回 DLC：届时本体 profile 因 `excludes_dlc` 不适用，DLC profile 生效。英文本体 v3 仍需英文本体实例。
