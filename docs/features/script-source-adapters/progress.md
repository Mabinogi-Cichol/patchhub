# 可导入脚本来源适配器施工记录

[设计文档](design.md) · [施工计划](implementation-plan.md)

## 当前状态

**施工中。P0–P6 已完成核心链路、双端脚本管理、四个内置来源及通用表格脚本迁移，旧 Rust 网站模块已退役；本地自动化与逐源只读网络抽样通过。P7 本地发行核验与 Deck 部署、RPC/界面抽样已完成；Windows 原生界面和实体手柄/软键盘验收仍待执行。**

- 调研基线：`main@cb06188`，2026-10-30。
- 文档开发分支：`feature/script-source-adapters`。
- 用户此前要求开始施工，并逐阶段要求提交和继续施工。P0 代码/文档为 `138c0c6` / `57dd65c`，P1 为 `901e592` / `5abf92a`，P2 为 `4c97d0e` / `ca581e9`，P3 为 `03cb2ae` / `45a2a85`；P4 代码/文档为 `20f921d` / `7550815`。随后用户要求“提交并开始 P6”：P5 文档/代码已提交为 `2009704` / `66ac372`，P6 代码已提交为 `c39fd62`，本文件随独立文档提交保存本阶段证据。截至 P6 提交时未推送、合并或操作设备；随后 P7 的授权部署见后文，未安装真实游戏补丁。
- 已有来源名称编辑/示例功能属于此前工作，不是脚本适配器功能的实现或验收证据。

## 阶段状态

| 阶段 | 工作 | 实现 | 自动化 | 实机/真实网络 |
| --- | --- | --- | --- | --- |
| P0 | 运行时原型与 API 合同 | 原型及合同已完成 | Windows / WSL Linux 各 18 项通过 | 3 个真实页面已录制；非全站/Deck 实机验收 |
| P1 | 导入、读取与原子设置 | 核心已完成 | Windows 10 / Linux 9 专项通过，完整回归通过 | 本地隔离状态验证；无 UI/设备验收 |
| P2 | 运行时、宿主能力与取消 | 核心执行器已完成 | Windows/Linux 运行时专项各 8 项，完整回归及 lint 通过 | 录制夹具及本地 TCP 验证；无新增网站/设备验收 |
| P3 | 发行商索引/详情与缓存、安装一致性 | 已实现 | 两平台 9 项链路专项与完整范围回归通过，Windows workspace lint 通过 | 本地 HTML/JSON 夹具与真实核心调用；无新网站/设备/实际安装验收 |
| P4 | 脚本补查与多源聚合 | 已完成 | 两平台 11 项专项、完整范围回归及最终边界专项通过；Windows workspace lint/契约通过 | 离线 HTTP 夹具与真实核心读取/投影，无新增真实网站或设备验收 |
| P5 | 协议与两端导入/编辑 | 已完成 | 两平台协议/CLI、跨语言、双端表单和真实核心往返通过 | Windows 桌面安装版、Decky 游戏模式输入/软键盘待 P7；无新增真实站点验收 |
| P6 | 全部内置来源/通用表格脚本迁移及旧 Rust 路径退役 | 已完成 | Windows 668 / Linux 679 passed，各 9 ignored；双端往返、lint/构建/契约通过 | 四源只读元数据抽样通过；非全量目录、下载或设备验收 |
| P7 | 全面验收、发行包与约定范围实机验证 | 施工中 | 本地 Release 构建、双端 ZIP/NSIS、包内核验、隔离 ZIP/EXE 安装已通过 | Kagura 完整目录与自定义样例通过；Deck 已迁移部署并通过 RPC、元数据更新/取消和界面抽样；Windows 原生/实体手柄待验收 |

## 2026-10-30：调研与计划

- 检查工作区为干净的 `main`，从当前基线建立设计分支。
- 核对适配器注册、来源白名单、索引更新、Kagura 专用详情解析、只读快照、缓存候选读取与 ZIP/EXE/手动候选重新解析路径。
- 确认当前补查结果为聚合记录，计划复用该形状扩展来源集合和查询键，不预先新建每源查询数据库。
- 编写独立设计与施工计划，分为 P0–P6；更新功能索引。运行时拟选 Lua 5.4/`mlua`，实际版本和平台可行性留待 P0 原型验证。
- 查阅 Lua 与 `mlua` 官方资料，确认宿主可控制库集合、注册 scoped 回调和 hook；该查阅不能代替本项目构建、取消或权限测试。

## 2026-10-30：按用户要求明确全部迁移目标

- 用户明确最终目标是全部内置来源迁移，修正初稿“不以全部迁移作为交付前提”的范围设定。
- 将四个内置来源与通用表格脚本化列为交付条件，增设 P6 逐源迁移和旧 Rust 网站路径退役，原最终验收顺延为 P7；设计、施工计划与索引同步更新。
- Rust 网站实现只允许在分阶段接入期间临时共存；最终删除对应模块/分派及脚本失败回退路径。HTTP、DOM、URL、缓存、身份、下载和安装等通用核心能力继续保留。
- 明确内置脚本随包提供、与自定义脚本共用运行时；保留来源 ID 与用户设置，内置已迁移脚本可显式保存为用户覆盖。迁移时解析身份的变化按真实缓存/选择边界处理。
- 本次仍仅修改计划文档；全部来源迁移、代码退役与 P0–P7 功能验证均待实施。

## 方案过度设计审阅

结论：**本轮方案可进入 P0 验证；没有把未实现能力写成已支持。** 对象为本目录设计与施工计划，依据为上述代码基线和实际调用路径，属于静态方案审阅。

- 按用户确认的目标将全部内置来源统一到脚本执行器，允许分批迁移但不长期保留双实现；迁移本身属于需求范围。无脚本商店、远端自动更新、通用插件框架或多语言运行时。
- 必要边界有实际依据：导入可执行代码需要限制主机能力；Lua 纯计算循环需要接入用户取消；脚本变化必须改变缓存和安装选择身份。方案未据此增加正常任务的任意页数、文件大小、内存或总时长上限。
- 同文件提交规则/代码、内容摘要及只读加载要求对应真实跨进程与快照一致性；明确 P1 需证明完整设置事务，不能仅沿用表面相似的多个独立写调用。
- 覆盖了内置分派之外的 Kagura 详情路径、缓存直接读取和安装重新解析；没有将“索引更新能调用 Lua”当作完整支持。
- 多补查源先复用现有聚合回执，要求明确混合成功/失败语义与完整无结果条件；不因为新增脚本就增加每源数据库或恢复状态机。
- 尚待原型核实的关键依据：库版本/构建、回调生命周期、非表格真实样例覆盖、长 C 调用取消边界及源码替换失效所需最少字段。均已设为 P0/P3 出口，未通过增加防御代码来假定解决。

## 本轮文档验证

- 18 个本地文档链接及功能索引的设计/施工记录/施工计划三个入口已检查，均指向存在的文件。
- `git diff --check` 通过；改动范围仅为本功能三份文档与功能索引。
- 功能自动化、真实网站、双端构建、部署、安装和实机验收：**均未执行**；不是文档阶段的通过项。

## 2026-10-30：开始施工，完成 P0 原型与接入合同

- 沿用 `feature/script-source-adapters` 与未提交设计材料，基线仍为 `cb06188`；保留用户已有文档内容。
- core 新增开发依赖 `mlua = 0.12.2`（`lua54/vendored/serialize`）和已有 DOM 树使用的 `ego-tree 0.11`，更新锁文件；实际编译 Lua 5.4.9。`cargo tree -p patchhub-core --edges normal -i mlua` 无生产依赖路径。
- 新增原型测试、测试宿主和 [Kagura EN Lua 样例/夹具](../../../crates/core/tests/fixtures/script-sources/README.md)。P0 当时宿主仅编入测试目标，不注册到产品；P2 后代码迁为 [核心执行器](../../../crates/core/src/sources/adapters/script.rs) 和 [单元测试](../../../crates/core/src/sources/adapters/script/tests.rs)，下述 P0 命令记录为当时执行证据。
- 验证 scoped HTTP 借用及回调失效、DOM 节点所有权、独立 VM、显式标准库、纯文本语法检查不执行顶层、GET/表单/Cookie/重定向/域名、类型错误与 JSON/UTF-8/数值转换。网络策略沿用现有 `RequestContext`，未新增脚本重试框架。
- 实验确认 hook yield 在 `string.gsub` 的 C 回调嵌套受保护循环中不能取消，手动中断该实验。最终采用 hook 报错 + 私有原始 `pcall/xpcall` 包装前后取消检查；覆盖 C 回调、错误处理器、模块顶层、普通循环及捕获取消后伪成功，普通保护调用语义保持。
- `mlua` 默认转换存在混合键丢失及空对象变数组的真实问题；在边界一次检查连续数组/对象/循环与非有限数，再使用类型转换，显式空数组保留 metatable。JSON 超出 Lua 有符号整数范围明确报错，不静默转浮点。
- 只读录制 Kagura EN `/patch/`、产品分类和一个补丁详情，均 HTTP 200；保存提取夹具、URL、日期与 SHA-256。列表/详情与原 Rust 解析器在相同录制输入上相等；真实详情没有 Steam ID，合成 Steam 链接测试单独标记。多根分页的终止页也是合成输入，未将其当成全站抓取通过。
- 新增 [P0 合同](p0-contract.md) 与 [全部来源迁移矩阵](migration-matrix.md)，明确导入/读取 DTO、宿主与入口、值/错误边界、JSONL v5 + Lua API v1、最少索引解析身份字段及后续实现责任。
- 沿真实代码核对：现有设置修改先改内存再单独 bump/save，P1 必须新建完整的锁内设置提交；P3 索引修订目前只比较条目/完整性/队列，必须补解析身份并覆盖缓存直接读取及安装；这些仍是未实现项，不能引用原型测试宣称通过。

### 已执行验证

| 验证 | 命令 / 证据 | 结果 |
| --- | --- | --- |
| Windows 原型 | `./scripts/cargo.ps1 test --locked -p patchhub-core --test script_runtime_prototype '--' '--include-ignored' '--nocapture'` | **18 passed**，包含单独标为测量用途的 C 标准库用例 |
| Linux 原型 | WSL Ubuntu，设置仓库 `.tools/linux-cargo` / `.tools/linux-rustup` / `target/linux`，运行相同 `cargo test --locked ... -- --include-ignored --nocapture` | **18 passed**，非 Steam Deck 实机 |
| 静态分析 | `./scripts/cargo.ps1 clippy --locked -p patchhub-core --test script_runtime_prototype '--' '-D' 'warnings'` | 通过；现有 Windows 增量缓存硬链接失败后复制提示仍存在，不是新增代码 lint |
| 原注册入口回归 | `./scripts/cargo.ps1 test -p patchhub-core --lib sources::adapters::tests::unknown_adapter_is_rejected` | 1 passed |
| 依赖与产物 | Windows `dumpbin /DEPENDENTS`、Linux `ldd` | 无外部 Lua DLL / `.so`；Lua 静态库 debug 大小分别 1,610,388 / 1,798,916 字节 |
| 类型转换 | 两平台同一原型目标 | 中文/emoji、空对象/数组/null、`u32::MAX/i64::MAX` 及错误输入通过 |

规范夹具尾随空白后两平台再次完整通过。最终示例调用 Lua 分配约 44,745 字节；64 MiB `string.rep` 测量 Windows 184.5876 ms、WSL 482.747817 ms，保留 Lua 分配 67,127,664 字节。取消只能在 C 调用返回后被宿主拒绝发布，**不是内存隔离或取消延迟保证**。测试 EXE / ELF 含测试框架和其他 core 代码（约 10.2 MB / 106.7 MB debug），不能作为正式安装包的体积增量；正式 release/SBOM/许可证随包检查仍归 P7。`mlua`、`mlua-sys`、`lua-src` 与 Lua 均为 MIT，已核对锁定源码的许可元数据/文本。

本轮最终格式/文档检查：`cargo fmt --all -- --check`、`git diff --check` 与新增文件尾随空白检查通过；本功能所有新增/修改 Markdown 共 201 个本地链接均存在。夹具记录同时保留完整响应文件字节摘要、解码并统一 LF 的摘要及规范后的夹具摘要，未将它们混作原始 HTTP 字节。

### P0 实现过度设计审阅

结论：**未发现需增加或删除的无依据防御策略，可进入 P1。** 对象为本轮开发依赖、原型宿主和 Lua 样例；追踪代码加载 → 保护调用 → scoped 网络/DOM → 输出转换 → 取消检查。

- `pcall/xpcall` 包装来自已复现的取消失效，不引入 watchdog、重试、指令配额或进程框架。
- DOM 句柄所有权解决真实生命周期，复用 scraper；未增加独立 HTML 引擎、运行时池或站点专用 Rust 回调。
- 表结构与数值校验针对实际有损转换，位于 Lua/Rust 边界；无脚本字节数、条目数、页数、任务总时长或额外内存限额。
- 样例仍是 P0 证据，不承担生产完整性判定、部分结果发布、缓存失效或四源迁移验收；P1–P7 继续按原计划推进。

## 2026-10-30：提交 P0，完成 P1 核心导入与设置事务

- 按用户要求先提交已有工作，拆分为 `138c0c6 test(sources): 验证 Lua 脚本运行时与取消边界` 和 `57dd65c docs(sources): 记录脚本适配器方案与 P0 验证`。沿用已有验证，未在提交前重复执行 P0 测试；提交后工作区干净，再开始 P1。
- 核心 `Sources` / `SourceRegistry` 新增 `import_script_source` 与 `get_script_source`，保存原始 UTF-8 源码与核心生成的 API v1 / SHA-256 描述。语法检查只编译文本，不执行模块顶层；错误 API/UTF-8/语法明确失败；缺失入口按设计留到 P2 执行时报告。`mlua` 移为正常核心依赖，P0 语法检查复用产品实现，DOM/执行原型仍只在测试目标中。
- 首次导入拒绝同 ID；`replace=true` 只更新已有脚本来源，不能覆盖 Rust 内置来源。替换保留用户名称覆盖、有效启用、逐游戏订阅与间隔；角色改变后继续执行现有发行商/补查调度约束。原 `add_source` 白名单继续拒绝 Lua，不能绕过导入。
- 增加 `SourceRule.script`（无脚本时省略）及设置 `script_sources`（缺省空、为空时不输出）。未迁移来源的原指纹保持，脚本变化改变规则指纹，名称覆盖不改变它。普通列表只含描述，源码仅通过显式读取返回。
- 设置文件单独升级为 schema 3，命名空间 marker 与索引/详情/匹配格式保持旧版本。旧设置读写打开时一次升级并保留 revision、名称、启用、订阅等内容；只读接受 v1–v3，不改文件。已验证升级不会重跑命名空间 v1→v2 迁移、不会碰原缓存文件，第二次打开不重写设置。旧二进制不能读取 v3 设置，不承诺降级兼容。
- 完整设置事务在现有进程锁/文件锁内读取最新落盘状态，修改副本，一次原子保存后再更换内存；初始化/升级也持文件锁。启用、名称、订阅、普通来源添加和间隔改为同一事务，移除旧 `apply_overrides → bump_revision` 分步链。否则旧 writer 会覆盖新导入的脚本/名称/订阅，此调整是脚本一致性所需的共同写入路径修正。
- 读取与 `SourceSnapshot` 从同一捕获设置中取得规则/源码。显式读取检查 API 与摘要；损坏/缺失源码报告该来源错误，不回退旧代码，也不阻断其他来源元数据列表。只读加载、列表、读取不创建 Lua VM、不执行代码、不联网。
- 审阅导入校验时直接删除共用规则中的页面数 5、域名数 8 上限及专属拒绝断言，保留实际必需的非空、域名、URL 和角色校验；没有为 Lua 另造豁免分支或扩大阈值。合法的大源码、多入口和多域名导入已有测试。
- 因 `SourceRule` 同时是 wire DTO，同步生成 TS/Python/JSON Schema 与 Flutter bundled schema，保持现有客户端契约一致；没有提前添加 P5 的导入/读取 RPC、能力方法或编辑器。

### P1 已执行验证

| 范围 | 命令 / 证据 | 结果 |
| --- | --- | --- |
| Windows 核心/协议/CLI | `cargo test --locked -p patchhub-core -p patchhub-protocol -p patchhub-cli`（仓库工具链） | 常规测试 **637 passed / 8 ignored**；首次 doc 阶段并行构建检查造成 rlib 缺失，随后串行重建 `cargo test --locked -p patchhub-core -p patchhub-protocol --doc` 通过，0 个文档用例 |
| Linux 核心/协议/CLI | WSL 仓库工具链同一 `cargo test --locked ...` | 全部常规及文档目标通过；原本 ignored 的网络/原型资源测量未执行 |
| P1 最终专项 | `cargo test --locked -p patchhub-core --test script_source_import` | Windows **10 passed**；Linux **9 passed**。Windows 额外用共享文件句柄阻止 rename，验证真实保存失败、旧字节/内存保留及释放后重试成功 |
| 共用事务故障注入 | core 单元测试 `failed_full_settings_write_does_not_publish_partial_memory_or_disk` | 两平台完整回归中通过：注入 writer 确认完整规则/摘要/源码/revision 已组成一个文档，返回失败后磁盘与内存均为旧值 |
| 精简校验后的复验 | `cargo test --locked -p patchhub-core --lib sources::validate::tests` | 两平台各 **12 passed** |
| 静态分析与契约 | `cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo run --locked -p patchhub-contract-gen -- check` | 通过；Windows 原有增量缓存硬链接提示不属于代码 lint |
| Flutter 契约 | `flutter test --no-pub test/wire_contract_test.dart` | **117 passed**，包含 schema 资源一致性及旧夹具 |
| Decky / 公共契约 | Python `test_v5_bridge.py`、`npm run test:contracts`、`npm run typecheck` | Python **13 passed**；共享 TypeScript/Flutter 展示夹具与类型检查通过 |

专项覆盖：编译不执行顶层、重启读取、显式替换、重复/不存在 ID、API/UTF-8/语法错误、普通 add_source 拒绝、摘要损坏/源码缺失、用户设置保留、同 ID 两 writer 仅一个成功、陈旧 writer 不覆盖其他设置、源码与规则快照一致、只读文件字节不变、大源码/多入口、旧格式一次升级与失败不发布。Windows 文件锁/原子替换测试、Flutter 工具缓存和 Decky 桥接在受限沙箱内遇权限阻碍，按现有工具权限机制在正常权限下验证通过；没有修改产品阈值来规避测试环境。

最终 `cargo fmt --all -- --check`、`git diff --check` 和新增文件尾随空白检查通过；本轮修改 Markdown 的 180 个本地链接均指向存在的文件。

### P1 实现过度设计审阅

结论：**未发现未关闭的过度设计问题，P1 出口已满足。** 基线为 `57dd65c` 后本轮 diff；覆盖导入编译、重复/替换校验、锁内设置读改写、失败返回、快照读取及元数据序列化。

- 完整设置事务直接替换旧分步写入，不叠加日志、租约、重试或恢复服务；private writer 参数仅为故障注入复用同一事务，不形成产品配置/多后端层。
- 设置 schema 与缓存命名空间分离，避免无关缓存迁移；不清空用户设置或索引，不为每种对象新增一套代际。
- 摘要只在明确脚本读取处核对，列表不编译/执行；不重复运行语法校验来“验证”只读状态。
- 页面/域名数量上限直接删除；未新增源码容量、任务时长限制或 Lua/Rust 失败回退。

P1 此时改动尚未提交，后续按用户要求提交，见下节。真实来源解析、脚本替换后的缓存/安装消费者失效、补查、多端编辑和全部内置来源迁移仍分别属于 P2–P7，不能以本阶段通过代替。

## 2026-10-30：提交 P1，完成 P2 核心运行时

- 按用户要求先提交已验证的 P1，代码为 `901e592 feat(sources): 支持脚本来源导入与完整设置事务`，文档为 `5abf92a docs(sources): 记录脚本来源 P1 实现与验证`；沿用已有验证，提交后工作区干净再开始 P2。
- 将 P0 宿主移入 [adapters/script.rs](../../../crates/core/src/sources/adapters/script.rs)，原型用例迁入该模块单元测试，删除旧测试目标/宿主文件。`ego-tree` 移为正常依赖，Lua/DOM/值转换和 scoped HTTP 只有一套实现。
- 新增 `collect_index`、`resolve_page`、`discover` 类型化入口。每次调用创建独立 VM，验证规则/源码摘要，加载文本模块并执行入口；仍显式选择标准库、使用拥有文档的 DOM 句柄。采集返回现有 `IndexOutcome`；详情返回 `PageFact`；补查只返回页面与身份事实，候选生成/多源聚合仍待 P4。
- 边界校验条目类型、标题、Steam ID、页面/下载域名、完整与 pending 的关系；304 复用需要已有完整索引。详情/补查拒绝伪造候选权限字段，镜像格式由核心重算。脚本修改传入 hosts 不能扩大捕获规则的实际网络权限。诊断/进度的来源 ID 和入口由宿主绑定。
- 返回结构化语法、运行、合同、摘要和 API 错误，保留来源、入口、可取得的 Lua 位置及完整原因；领域错误经 `io::Error` 和 CLI 映射仍保留详情。取消统一为 `task_cancelled`，顶层、入口、受保护循环和捕获取消后返回均不能发布成功。
- 实际检查确认旧阻塞 `send`/正文读取在等待网络时不能及时观察取消。将安装下载已有的异步取消等待抽为 `operation::cancellable`，来源 HTTP 以现有同步 `Fetcher` 接口驱动 async reqwest，等待响应头和逐块正文均复用该机制；网络策略和既有请求超时保持。删除被替代的同步正文读取及其专属测试，增加真实本地 TCP 等待/大正文用例；无任务总时长限制或正文大小上限。
- 普通元数据列表、只读快照仍不创建 VM、不执行代码。尚未修改 `builtin_adapter` 的生产分派、索引解析身份或安装消费者，避免在缓存绑定未完成时启用脚本来源。

### P2 已执行验证

| 范围 | 命令 / 证据 | 结果 |
| --- | --- | --- |
| Windows 核心/协议/CLI 完整回归 | `./scripts/cargo.ps1 test --locked -p patchhub-core -p patchhub-protocol -p patchhub-cli` | **644 passed / 8 ignored**，所有常规及文档目标通过；最终新增取消用例后按下列专项复验 |
| Linux 同范围完整回归 | WSL Ubuntu，仓库 Linux 工具链/`target/linux`，相同 `cargo test --locked ...` | **655 passed / 8 ignored**，所有常规及文档目标通过；非 Steam Deck 实机 |
| 最终运行时专项 | `cargo test --locked -p patchhub-core --test script_source_runtime` | 两平台各 **8 passed**：导入非表格样例列表/详情、部分结果/诊断、错误详情、域名/权限、补查事实、条件 GET/POST/GBK/304、捕获取消、顶层/受保护入口循环取消 |
| 原型语料迁移回归 | core 单元测试 `sources::adapters::script::tests` | 两平台各 **17 passed / 1 ignored**；资源测量仍单独 ignored，P0 测量证据保留，不重复运行 |
| 网络上下文 | core 单元测试 `sources::http::tests` | 两平台各 **28 passed**；最终 TCP 夹具 lint 修正后额外复验 `real_transport` 两项均通过，含等待响应头/正文取消及 9 MiB 正文完整读取 |
| CLI 错误传播 | `cargo test --locked -p patchhub-cli script_error_tests` | 两平台各 **1 passed**，保留长中文 Lua 详情及错误码 |
| 静态分析 / 契约 | `cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo run --locked -p patchhub-contract-gen -- check` | 通过，无生成契约漂移；现有 Windows 增量缓存硬链接失败改复制的提示不属于代码 lint |

完整回归已覆盖 P1 导入、设置事务和原来源行为；最后的小范围错误映射/测试夹具变更仅补相关专项。8 个 ignored 为已有网络/手工或资源测量用例，未记作通过。本轮未新增真实站点请求，也未执行双端 UI、正式发行包、设备部署/安装验证。

最终 `cargo fmt --all -- --check`、`git diff --check` 通过；本功能 Markdown、夹具说明及功能索引共 203 个本地链接均存在，已更新迁移后的源码/测试链接。旧原型测试命令保留为 P0 历史证据，不作为当前命令。

### P2 实现过度设计审阅

结论：**未发现未关闭的过度设计问题，P2 出口已满足，P3 尚未实施。** 基线为 `5abf92a` 后本轮 diff；沿类型化入口 → 摘要验证 → 私有文本执行 → scoped 网络/DOM → 事实校验 → 领域错误与取消传播检查。

- 迁移原型而非复制执行器；一次调用一个 VM，无运行时池、额外线程服务或脚本失败回退。
- 表结构、域名及字段校验位于可执行代码返回事实的真实边界，进度/诊断身份由宿主生成；不把结构通过等同于事实可信。
- 取消 hook 和受保护调用包装复用 P0 复现依据。网络等待机制抽自现有安装下载以修复实际阻塞路径，25 ms 是观察取消的唤醒间隔，不是任务期限；未加 watchdog、重试、内存/源码/条目/正文硬上限。
- 纯 C 长调用仍只能返回后检查取消，没有内存隔离或取消延迟保证；已在合同和历史测量中说明。
- P3 需要统一外层借用取消接口与运行时 `Arc` 所有权，并覆盖更新、匹配、缓存直接读取、只读投影及 ZIP/EXE/人工安装解析；当前执行器测试不能代替这些链路的验收。

P2 此时改动尚未提交，后续按用户要求提交，见下节。生产调度分派、源码替换后的缓存/安装身份、补查聚合、双端编辑、全部来源迁移及发行/实机分别留待 P3–P7。

## 2026-10-30：提交 P2，实施 P3 发行商生产链路

- 先沿用 P2 已有验证，拆分提交 `4c97d0e feat(sources): 接入核心 Lua 执行器与可取消宿主` 与 `ca581e9 docs(sources): 记录 P2 运行时实现与双平台验证`，确认工作区干净后开始 P3。
- 在 [adapters/mod.rs](../../../crates/core/src/sources/adapters/mod.rs) 增加捕获有效源码的 `SourceAdapter`，接入真实 `update_source` 和发行商详情解析。Lua 来源不进入 Rust Kagura 解析，也不在脚本失败后回退；尚未迁移的来源继续原 Rust 实现，全部内置迁移/退役归 P6。
- `match_page_entries` 与分类遍历统一调用详情入口，覆盖直接命中、分类内联正文、链接和下一页。索引更新仅采集列表；匹配仅读共享索引和命中的详情，不重新抓全站列表。同页面详情以既有修订跨游戏共享，失败保留此前正式详情并返回诊断，取消不能保存当前游戏的假成功。
- `SourceIndex.parser_fingerprint` 保存脚本解析身份，旧 Rust 来源缺省/省略。旧源码索引和私有进度不供新代码复用，保留原文件；发布新脚本时基于旧正式索引递增修订，即使条目字面相同，也使旧详情/匹配按 `index_revision` 失效。不完整/异常仍不覆盖正式索引。
- 在 [index.rs](../../../crates/core/src/sources/index.rs) 集中解析相容性和候选绑定。`SourceCandidate` 增加同一个可选解析指纹，核心绑定候选 ID 和候选指纹；ZIP/EXE 选择 ID 纳入该指纹，人工选择与手动 URL 继续核对当前真实候选。补充此字段的实际原因是：只改索引修订时，相同事实仍会产生相同旧选择 ID。未迁移来源无新增字段，其候选/选择 ID 与既有夹具保持。
- [registry.rs](../../../crates/core/src/sources/registry.rs) 的懒读与冻结快照均检查脚本索引相容性；匹配复用、来源/游戏列表、`matches`、`cached_candidates`、`cached_discovery`、直接条目投影及公开匹配摘要排除旧解析结果，重算投影状态。旧缓存不能在替换后或只更新未匹配时继续授权。匹配摘要读取设置失败明确返回失败诊断，不将无法核对的脚本候选当作当前结果。
- ZIP、EXE、人工候选与显式 URL 的安装重新解析都读取同一有效视图，保持原域名/格式/身份/订阅检查；读取和核心预览不执行 Lua、不联网、不写设置。用顶层无限循环的脚本替换后验证这些路径仍可返回。此阶段验证到预览和来源解析，没有执行实际文件安装。
- CLI、更新、匹配/重匹配及直接调用统一传递同一 `Arc<AtomicBool>` 并建立操作作用域，hook 和网络观察同一取消信号；无 `unsafe` 或与任务脱节的新标志。任务开始/复用和提交时核对跨进程规则替换：旧实例不能复用旧成功，途中替换不发布旧索引，旧匹配文件保留但不作为新定义的成功返回。
- 同步候选字段的 TS/Python/schema 和 Flutter bundled schema。设置仍为 schema 3，命名空间 2，索引/详情/匹配均为 1；新增字段可缺省，仅脚本结果写入，无新全库迁移或清空。导入/读取 RPC、编辑器和能力方法扩展仍归 P5；多源补查仍归 P4。

### P3 已执行验证

| 范围 | 命令 / 证据 | 结果 |
| --- | --- | --- |
| Windows 核心/协议/CLI | 先运行 `cargo test --locked -p patchhub-core -p patchhub-protocol -p patchhub-cli`，取消夹具修正后按目标复验并续跑未到达目标 | 29 个常规/文档目标合并去重为 **655 passed / 8 ignored**，最终全部通过 |
| Linux 核心/协议/CLI | WSL Ubuntu 仓库工具链，相同全范围及目标续跑 | 29 个常规/文档目标合并去重为 **666 passed / 8 ignored**，最终全部通过；非 Steam Deck 实机 |
| 最终生产链路专项 | `cargo test --locked -p patchhub-core --test script_source_pipeline` | 两平台各 **9 passed**。覆盖非表格导入→更新→匹配→核心预览/重新解析、共享详情、源码替换各时点/同条目、分类分页、取消、失败保留、途中替换和设置错误 |
| 显式 URL 与直链条目 | core 单元测试 `installer::manual_selection::script_binding_tests` | 两平台各 **1 passed**；无需匹配任务的直链投影也绑定脚本身份，替换前/后旧候选 ID 均不能解析显式 URL |
| 取消与设置变化回归 | `source_acceptance`、`source_matching`、`source_update` | 两平台各 **11 / 24 / 18 passed**；失效/失败项已修复并复验 |
| 静态分析 / 契约 | `cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo run --locked -p patchhub-contract-gen -- check` | Windows 最终通过；原有增量缓存硬链接改复制提示仍存在，不属于代码 lint |
| 跨语言 / Flutter | `npm run test:contracts`、`npm run typecheck`、Python `test_v5_bridge`、Flutter `test/wire_contract_test.dart` | 共享夹具和类型检查通过；Python **13 passed**，Flutter **117 passed** |

回归中两处旧取消夹具此前只给适配器传裸标志，未让 HTTP 观察同一操作作用域；统一后取消可在响应解码前返回。已将“完成游戏保留”测试改在第一游戏提交后的回调取消，将“完成列表页保留”改为第一页解析后、第二请求期间取消，保留原业务断言；不放宽产品取消检查。最初 lint 的测试多余借用已修正，最终通过。生成契约曾遇 Windows 文件映射占用，释放后重跑生成/检查通过，无产品重试分支。

最终 `cargo fmt --all -- --check`、`git diff --check` 通过；本功能 Markdown、夹具说明与索引共 206 个本地链接均存在。

上述自动化使用离线夹具、隔离状态与真实核心调用。P0 真实页面录制继续作为已有证据，本轮没有新增真实网站抓取；实际安装、双端 UI 导入/编辑、正式发行包、部署与设备验收均未执行。8 项 ignored 仍为既有 live/手工或资源测量，不计通过。

### P3 实现过度设计审阅

结论：**未发现未关闭的过度设计问题，P3 出口与双平台自动化已完成。** 基线为 `ca581e9` 后本轮 diff，沿导入后的规则/源码捕获 → 更新/详情分派 → 发布修订 → 详情/匹配复用 → 懒读/冻结投影 → ZIP/EXE/人工选择/显式 URL 重新解析核对。

- 两种适配器分派直接表达迁移期间的 Rust/Lua 入口；无脚本失败兜底，也未复制详情遍历、HTTP 或候选算法。旧网站模块仅按 P6 计划暂存。
- 索引解析指纹解决同条目代码变化，候选上的同一指纹解决现有选择 ID 不含解析身份这一已验证缺口；继续沿用已有详情/匹配修订，无每源数据库或每种 DTO 独立代际。
- 跨进程替换检查位于真实持久化读取/发布边界，启用/订阅沿原绑定；去掉为同一次提交重复加载设置的辅助函数。旧文件保留不等于旧结果授权。
- 取消共享同一拥有生命周期的信号；没有增加超时、重试、TTL、配额、运行时池、看门狗或全库清理。C 长调用与资源隔离边界仍按 P0/P2 合同说明。

P3 代码已提交为 `03cb2ae`，文档另行提交；P3 出口已完成，下一阶段为 P4，不能据此宣称补查聚合、双端导入或全部内置来源脚本化已完成。

## 2026-10-30：继续完成 P4 脚本补查与多来源聚合

- 按用户要求核对 Git：P3 代码/文档为 `03cb2ae` / `45a2a85`，工作区干净，无遗漏改动，未创建空提交；继续实施 P4。
- [SourceAdapter](../../../crates/core/src/sources/adapters/mod.rs) 接入 `discover`。Lua 输出页面/内联事实后由公共算法生成候选，补全当前 AppID 的确认别名、重算镜像格式并绑定解析身份；其他游戏身份不能授权当前候选。HGamer 沿同一分派运行，仍待 P6 完整迁为脚本。
- [matching.rs](../../../crates/core/src/sources/matching.rs) 从单个补查源改为按优先级/ID 遍历全部启用且订阅的来源。每源看到当前已知候选事实；本次单源失败保留其他源候选和来源归属/完整错误详情，不吞异常为无结果。取消在各源之间及入口返回后检查，不保存本轮部分聚合为成功。任务开始及提交时也核对跨进程新增来源，不能用旧规则列表漏掉新增来源再复用或发布旧聚合。
- 聚合保留原 `FallbackMatchRecord`：可信候选为 `Matched`（保留失败诊断），全部完整且无候选才为 `NoMatch`，部分完成/仅线索为 `Incomplete`，全部失败且无候选为 `Failed`。游戏总状态继续沿原发行商覆盖规则推导，不把聚合失败强行改成整个游戏全部失败。保留发行商可信命中优先及索引覆盖完整后补查的既有调度条件；补查来源不能全站更新或配置索引定时。
- 查询键包含 AppID、名称/提示、排序后的全部有效规则/脚本摘要和解析版本。聚合记录增加默认缺失的 `sources_fingerprint`：只读路径没有原前端提示，需用该来源集合摘要核对当前结果。来源增删、启用/订阅、源码变化后，普通缓存、匹配摘要、冻结快照及候选解析均不展示旧聚合；名称显示覆盖不触发失效。旧无绑定结果保留文件、重新匹配，无全库清理或额外数据库。
- [registry.rs](../../../crates/core/src/sources/registry.rs) 只对实际返回可信候选的补查源标记命中，不再因聚合成功而把所有补查源标记为命中；部分线索在任务返回与缓存视图保持可见。显式重新匹配查询所有参与来源并把 `force` 交给脚本。
- 审阅确认旧补查包装在候选校验失败后仍保留原正候选，可能将无效候选当作部分命中。删除该保留路径，改为明确失败，由聚合层保存其他来源成功；新增来源 ID 不一致的边界用例证明不会返回正结果。
- 无新增 wire 字段、RPC 或客户端界面。存储 schema 保持，`sources_fingerprint` 仅在聚合缓存中可选保存；迁移形成的无绑定旧记录仍可读取，但不作为当前结果。旧验收中需要“有效当前结果”的夹具已补真实集合绑定，迁移/状态推导夹具仍保留缺省字段验证兼容读取。

### P4 已执行验证

| 范围 | 命令 / 证据 | 结果 |
| --- | --- | --- |
| Windows 核心/协议/CLI | `cargo test --locked -p patchhub-core -p patchhub-protocol -p patchhub-cli` | 30 个常规/文档目标，**666 passed / 8 ignored**，完整通过；最终包装校验修正后补下列相关专项 |
| Linux 核心/协议/CLI | WSL Ubuntu，仓库 Linux 工具链/独立 `target/linux`，同一全范围命令 | **677 passed / 8 ignored**，全部常规及文档目标通过，非 Steam Deck 实机 |
| 最终补查专项 | `cargo test --locked -p patchhub-core --test script_source_fallback` | 两平台各 **11 passed**：多脚本/仅 HGamer/混合、混合失败、完整空/部分/线索、重启复用、集合/订阅/代码失效、外游戏身份、取消、途中新增/替换、只读投影/旧选择、旧负缓存、输入顺序/名称提示/强制重查 |
| 最终包装校验与既有匹配 | core `sources::adapters::tests`、`source_matching` | 两平台各 **6 / 24 passed**；包装专项含最后新增的 1 项无效候选拒绝用例，不将 6 项重复计入完整回归总数 |
| 既有验收/只读与缓存 | `source_acceptance`、`source_snapshot`、core `sources::registry::tests` | Windows 首轮专项各 **11 / 5 / 8 passed**；两平台完整回归包含相同目标 |
| 静态分析与契约 | `cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo run --locked -p patchhub-contract-gen -- check` | Windows 最终通过，无生成契约漂移；原有增量硬链接改复制提示不属于代码 lint |

完整回归后对最后的无效候选错误传播及跨进程新增来源绑定补相关专项，两平台均通过；未重复全量测试。最终 `cargo fmt --all -- --check`、`git diff --check` 通过，功能 Markdown 与索引共 209 个本地链接均存在。8 项 ignored 为既有 live/手工/资源测量用例，不计通过。本轮未新增真实网络、双端界面、正式发行包、部署或实际安装/设备验证；客户端代码/生成 wire 契约未变化，沿用 P3 的跨语言验证，不重复执行 Flutter/Python/前端测试。

### P4 实现过度设计审阅

结论：**未发现未关闭的过度设计问题，P4 出口已完成。** 基线为 `45a2a85` 后本轮 diff；沿脚本事实 → 公共候选/身份算法 → 全来源循环与聚合状态 → 查询复用 → 普通/冻结读取 → 旧选择重新解析检查。

- 使用同一执行器和公共候选算法，不增加另一套站点引擎或脚本失败回退。校验失败明确返回，不保留无效候选假成功。
- 一个聚合结果、一份来源集合摘要；额外摘要针对只读路径缺少名称提示这一实际输入差异，不引入每源缓存数据库、代际/租约或恢复状态机。
- 单源失败只影响本次该源结果；已完成来源仍保留，取消则不发布本轮聚合。查询次序确定，集合摘要与输入次序无关。
- 无新增配额、总时长限制、重试框架、TTL、watchdog 或无关清理；已有覆盖调度和安装权限继续生效。

P4 代码已提交为 `20f921d`，文档按本轮用户要求独立提交。两端导入/编辑及全部内置来源迁移尚未完成，不能以核心补查通过代替 P5–P7。

## 2026-10-31：P5 协议、双端导入与编辑

- 按用户“开始 P5”从 `feature/script-source-adapters@7550815` 继续，开始时工作区干净；本轮未要求 Git 提交，不推送、不合并、不部署。
- [protocol](../../../crates/protocol/src/lib.rs) 新增 `import_script_source/get_script_source`，方法表由 22/10 扩展为 24/12。导入复用 `{rule, settings_revision}` 回执，读取返回作者规则、API、完整源码及 SHA-256；`replace` 默认 false。JSONL 仍为 v5，新增独立 `sources.script_api_version=1`，严格核对新旧能力组合。
- [CLI](../../../crates/cli/src/runtime.rs) 复用 P1 核心事务导入；读取使用只读冻结快照，不初始化、升级或写状态，不执行 Lua、不联网。没有接受脚本文件路径。同步 Rust、TS、Python、JSON Schema、固定语料和 Flutter bundled schema；有效大源码作为固定语料，不套用旧 Decky `add_source` 长度检查。
- 复核发现旧跨语言递归检查将任何 `code` 都视为短错误码，无法承载 Lua 文本。将错误码字符/长度约束归到相应 DTO 的 schema；源码 DTO 不设此限制。Rust 和 Flutter 的 schema 子集补上对应字段约束，错误回执也按 schema 检查；新增空码、大小写、过长码和含控制符的错误码的拒绝语料，保留未知合法错误码及完整详情。
- [Windows 编辑器](../../../apps/windows-flutter/lib/pages/script_source_editor.dart) 与 [Decky 编辑器](../../../apps/decky/src/script-source-editor.tsx) 使用三个有标签的文本输入：规则 JSON、API 版本、Lua 源码。新导入、卡片显式读取、原 ID 绑定的“保存并替换”、API/SHA 展示、语法错误完整显示均已接入；失败保留输入，查看和复制示例不覆盖草稿或自动创建记录。源码可视行数只是滚动区域，不是文本上限。
- 沿用现有名称、启用、订阅、间隔及更新/匹配入口。只在保存回执成功后清理 UI 的旧来源候选和动态补丁，再读取核心投影；签名目录结果保留，避免原“查询失败时保留结果”恢复已失效脚本选择。刷新来源/投影与保存分开，保存后刷新失败显示“已保存，但刷新失败”，不误报成未保存、不要求盲目重试导入。
- Decky Python 只传输文本与结果，不执行 Lua；新 RPC 保留 CoreError 的完整 Lua 诊断，避免插件异常字符串仅剩错误码。Windows 卡片脚本元数据增加独立 PageStorage key，修复新错误提示插入/卡片重建后，内部文本滚动恢复误读 ExpansionTile 布尔展开状态的实际异常。
- 保存只编译，不运行顶层和入口；保存成功不代表网站或执行通过。界面提示仅导入可信脚本、请求受规则域名约束，后续需更新来源或重新匹配。当前四个内置来源尚未脚本化，因此不显示脚本编辑按钮；P6 迁移后复用同一读取/显式用户覆盖路径，不在 P5 额外实现内置脚本版本层。
- 更新[来源指南](../../source-adapters.md)与[作者示例](examples/README.md)。示例采用占位域名/合成 HTML，核心实际执行验证直接条目，双端测试核对文档/界面内容一致；不冒充真实站点抓取或内置迁移验收。

### P5 已执行验证

| 范围 | 命令 / 证据 | 结果 |
| --- | --- | --- |
| Windows 协议/CLI | `cargo test --locked -p patchhub-protocol -p patchhub-cli` | 最终 **69 passed**，7 个常规/文档目标通过；含实际 JSONL 子进程导入、重开读取、重复 ID、显式替换、语法/API 失败不改盘、设置保留、缺失状态只读 |
| Linux 协议/CLI | WSL Ubuntu，独立 Linux 工具链与 `target/linux`，同一命令 | 最终 **69 passed**，全部通过；不是 Steam Deck 实机 |
| 核心运行时/作者样例 | 两平台 `cargo test --locked -p patchhub-core --test script_source_runtime` | 各 **9 passed**（原 8 项及新文档样例）；未重复未改动的全量 core 测试 |
| Rust lint / 生成契约 | `cargo clippy --locked --workspace --all-targets -- -D warnings`；`cargo run --locked -p patchhub-contract-gen -- check` | 最终通过；仅既有增量硬链接改复制提示，无代码 lint、无生成漂移 |
| Decky Python | 显式 `PATCHHUB_TEST_CORE=target/debug/patchhub-core.exe`，`python -m unittest discover -s apps/decky/backend -p 'test*.py'` | **117 tests / 114 passed / 3 skipped**；含真实 Windows Core 大源码往返、替换、失败保留与重开读取；跳过为既有平台/身份用例 |
| Decky → Linux Core | WSL `PATCHHUB_TEST_CORE=target/linux/debug/patchhub-core`，`test_bridge.BridgeIntegrationTests` | **2 passed**：实际扫描/错误与脚本服务往返；不等于设备插件部署 |
| Decky 前端 | `npm test -w apps/decky`；`npm run typecheck`；`npm run build:decky` | 最终 **88 passed**，类型检查/构建通过；实际 JSX + 控件回调验证导入、读取、替换、失败草稿、复制、刷新失败和旧结果清理，原生 Decky 控件在测试中由 HTML 替身表示 |
| Flutter 全量与最终相关回归 | `flutter test --no-pub`；最终再运行脚本编辑器/真实 CoreClient、source_model、wire_contract、core_protocol、global_sources_layout 共 6 个文件 | 全量 **283 passed / 1 skipped**；最终相关范围 **158 passed**。后者包含新增真实核心测试及新增错误语料，不与全量重复累计；深浅色、580×600 / 1.5 倍字体、长源码和卡片重建通过 |
| Flutter 静态分析 | `flutter analyze --no-pub`，相关 Dart 格式化 | 最终通过，无问题 |
| 共享契约表现 | `npm run test:contracts` | 生成 DTO / UTF-8 /动作/进度/URL 及 Flutter 表现夹具一致 |

最终固定语料含 69 个请求、102 个响应、25 个事件、3 组序列、8 个早期错误、2 个原始请求；Rust/Python/Flutter 均验证适用部分。源码传输包含超过旧入口 32768 字符限制的合法中文脚本，真实保存的顶层无限循环/抛错未被执行。完整回归后对错误码约束、保存/刷新边界和 UI 旧结果清理的最终修改补跑上述相关检查。最后将一项错误码语料收紧为末尾换行，补验两平台 Rust 语料各 3 项、Python 117 项及 Flutter 协议 143 项通过；未重复未改动的 UI、全量 core 或安装器测试。`cargo fmt --all -- --check`、`git diff --check` 与功能/指南/索引本地链接检查通过。

未执行：Windows 安装版交互、Decky Loader 中游戏模式手柄焦点/软键盘、正式发行包、远程设备部署、真实网站完整执行及实际安装。前端构建和离线/本机自动化不代替上述验收。

### P5 实现过度设计审阅

结论：**未发现未关闭的过度设计问题，P5 实现与本地自动化出口已完成。** 基线为 `7550815` 后的本轮改动；沿输入 → 能力/schema → JSONL/Python 转发 → 核心事务/只读快照 → 成功或错误回执 → 来源与游戏投影核对，并复查失败草稿、显式替换、只读与完整错误路径。

- 两端仅编辑/传输文本，复用一个 Lua 执行器及设置事务，没有脚本包管理器、前端解释器、额外文件路径执行或并行版本系统。
- 原 ID 绑定对应显式替换的实际覆盖风险；能力校验对应严格方法表；错误码字段分型对应源码被误拒绝的实际问题。没有对合法源码增加字符/文件/页数/报告上限或新的超时、重试与看门狗。
- 旧 UI 结果保留及保存/刷新误报均沿真实执行路径发现并关闭；只处理本次脚本管理需要的边界，没有扩展成通用缓存代际、自动恢复或重试框架。
- 输入草稿、错误详情、摘要和保存/执行状态均可直接查看；当前没有用核心测试、HTML 控件替身或 WSL 冒充安装版/Steam Deck 实机验证。

## 下一步

P6：将 Shiravune、Kagura 国内/海外、HGamer 及通用表格全部迁为随包 Lua，验证行为并删除旧 Rust 网站实现。随后进入 P7 发行资源与约定范围实机验收。P5 已于随后用户要求“提交并开始 P6”时拆分提交为：`2009704`（文档）、`66ac372`（代码）。未推送或合并。

## 2026-10-31：P6 开始全部内置来源迁移

- 按用户要求提交 P5 后开始 P6。`2009704` / `66ac372` 为独立文档/代码提交，提交后工作区干净；沿用 P5 已完成验证，没有重复运行 P5 全量检查。
- 迁移前旧适配器离线基线：`cargo test --locked -p patchhub-core sources::adapters --lib`，40 passed / 1 ignored；Kagura 录制列表/详情固定期望保留为 JSON。另用同一批输入比较旧 Rust 与新 Lua 的四源/通用表格索引、页面事实、候选事实及 HTTP 请求，9 组对照通过，固定输入/期望保存在 `tests/fixtures/builtin-migration/`。候选解析身份和观察时间属于明确变化，不作为站点事实等价字段。
- 新增内嵌 Lua 资源、共享 Lua 文本/DOM helper 与单一脚本分派。保留旧 adapter 名称仅用于定位随包源码；用户 adapter=lua 覆盖仍按原事务保存。正在迁移回归、验证只读/替换和安装消费路径；本阶段尚未完成，真实网络和最终双平台证据待记录。

### P6 实现与行为收口

- [随包资源](../../../crates/core/resources/source-scripts/) 包含 `common.lua`、Shiravune、Kagura 国内/海外共用代码、HGamer、通用表格。由 [builtin.rs](../../../crates/core/src/sources/builtin.rs) 静态内嵌完整源码并生成 API 1/SHA-256 描述；`.gitattributes` 固定 Lua LF。完整摘要见[迁移矩阵](migration-matrix.md)。不依赖外部解释器或额外源码文件部署。
- 旧 adapter 名只定位内嵌文本，[生产分派](../../../crates/core/src/sources/adapters/mod.rs) 统一执行 Lua。删除 `patchmoe.rs`、`kagura.rs`、`hgamer.rs`、`table.rs` 和旧站点 DOM 模块；网站选择器、表单、分页、正文提取全部迁入 Lua，公共 HTTP/DOM/URL、身份、候选、缓存与安装留在核心。
- 宿主只补真实迁移所需的直接子元素、URL 分解/规范化、公共 Steam 身份/别名/日期及单词匹配；没有站点专用解析回调。条件请求处理 `ctx.null` 的可选验证器，解决真实索引序列化后的 null 与 Lua 字符串转换冲突。表格 ID 改为字面比较，同时删除无依据的 20 列及 64 字节 ASCII ID 限制；第 22 列和 Unicode/冒号 ID 有生产路径回归。
- 读取内置脚本不初始化状态、不编译/执行、不联网；返回 `adapter=lua` 规则供显式同 ID 覆盖。导入只编译，用户覆盖在重启和后续默认规则变更后仍保留，名称覆盖、启用、逐游戏订阅和间隔保持。Windows CoreClient、Decky SourceService 均通过真实核心往返。
- 迁移后普通读取、快照、更新、匹配和安装继续检查当前脚本解析身份。旧无绑定索引/匹配文件保留但不授权新解析器。安装、投影、CLI 等旧合成测试补齐当前规则的索引/候选证据，未放宽生产身份门槛；迁移专用回归另验证旧无绑定缓存仍失效。
- 完整回归发现普通发行商索引修订变化会连带清除未变的 HGamer 补查结果。`current_record` 现在区分索引修订与解析身份：修订变化仅移除对应发行商记录，解析变化继续失效依赖证据，补查源集合/源码变更仍失效补查结果。既有“后续发行商命中隐藏旧补查”和“按源重匹配复用补查”回归恢复通过，P3/P4 源码替换失效回归继续通过。
- 取消遵循 P2 已冻结合同：脚本未返回事实时，不生成本轮 VM 局部进度；既有正式索引和已有进度保留。明确返回部分结果时才存 `pending`。迁移测试相应更新，不新增 VM 检查点、重试或总时长限制。运行时错误现在含来源/入口/行号，测试保留原因断言而不要求旧裸错误字符串。
- 历史设计/审计中指向已删除网站模块的链接改为对应 Git 基线，保留当时事实；当前指南、设计、合同、迁移矩阵与功能索引同步更新。

### P6 验证结果（2026-10-31）

| 范围 | 实际执行 | 结果 |
| --- | --- | --- |
| Windows Rust | `cargo test --locked -p patchhub-core -p patchhub-protocol -p patchhub-cli --no-fail-fast` | 668 passed / 9 ignored；35 个测试/文档测试目标，无失败 |
| WSL Ubuntu Rust | 独立 Linux target 执行相同命令 | 679 passed / 9 ignored；平台差异来自已有 OS 专用测试，无失败 |
| 迁移专项 | 九组固定结果/请求对照；迁移回归 18；内置读取/覆盖与旧缓存 2；发行商脚本 9；补查脚本 11；运行时 9 | 两平台全部通过；已包含在完整回归中，非额外累计 |
| 来源及消费者 | 来源更新 18、匹配 24、验收 11、安装投影 24，另有核心安装单元/集成回归 | 两平台通过；合成隔离状态和游戏目录，不是实际游戏生效验收 |
| Flutter | `flutter analyze --no-pub`；原生脚本往返、编辑器、wire contract、core protocol 四文件 | 无分析问题；148 passed；新增内置读取不初始化状态和显式覆盖 |
| Decky Python | 显式 `PATCHHUB_TEST_CORE=target/debug/patchhub-core.exe`，backend unittest discover | 117 tests，OK / 3 skipped；含真实 SourceService 内置读取与覆盖；未在 Deck 设备运行 |
| lint / 格式 / 契约 | workspace/all-targets clippy `-D warnings`、cargo fmt check、contract-gen check、`npm run test:contracts` | 通过，无生成漂移；仅既有增量硬链接降为复制的环境提示；修正一个等价布尔断言 lint 后补跑门禁，未重复无改动测试 |
| Decky 前端 | `npm run typecheck`、`npm run build:decky` | 通过；前端实现未修改，沿用 P5 表单自动化，不把构建当设备验收 |
| 文档/差异 | 功能目录、索引、指南与夹具说明的本地 Markdown 链接、`git diff --check` | 通过；旧模块无生产分派/引用 |

本地日志保留于忽略的 `.artifacts/p6-rust-{windows,linux}-final.log`、`p6-live.log`、`p6-python.log`、`p6-flutter.log`、`p6-clippy.log` 等，未将运行产物加入 Git。

### P6 真实网络证据

通过生产分派执行 `builtin_scripts_live` 与 `hgamer_live` 的显式 ignored 网络测试，只读取公开 HTML/元数据，没有下载补丁：

- Shiravune：完整索引 165 条，`complete=true`，无诊断；表单语言请求沿现有 Cookie/最终 URL 网络上下文执行。
- Kagura 国内：从当日首页选择 `https://kagurafan.com/2026/10/22/sacrifice_sanctuary_the_ritual_maidens_life_with_monsters/`，详情返回标题和 4 个镜像；该页没有 Steam ID。
- Kagura 海外：`https://www.kaguragames.com/product/sacrifice-sanctuary-the-ritual-maidens-life-with-monsters-patch/`，详情返回标题和 4 个镜像；该页没有 Steam ID。发行商网络用例耗时约 5.94 秒，4 个逻辑 URL（同首页的语言 POST 不作为新逻辑 URL）。
- HGamer：App1250180 查询 3 个逻辑请求，完整、0 候选、1 身份；App1227890 查询 5 个逻辑请求，完整、本体/DLC 两候选、1 身份、上游 Kagura；均无诊断，合计约 13.05 秒。

Kagura 真实验证仅为详情抽样，列表分页、多入口、分类、DLC 与候选变体由离线生产链路覆盖；未将其写成全站实时采集通过。后续只调整公共镜像标签边界/空白类名提取，最终离线回归已通过，未重复无必要的网络请求。

### P6 过度设计审阅

按 [overengineering-review](../../../.agents/skills/overengineering-review/SKILL.md) 审阅 `66ac372` 至本轮差异，并沿注册/覆盖/读取、更新/详情/补查、缓存身份与安装消费者追踪。结论：**未发现本阶段仍需修改的过度设计项。** 静态资源定位复用一个执行器和现有设置事务；公共宿主能力均有迁移输入依据；未增加第二引擎、VM 池、网站 Rust 回调、源码大小/条目/页数/总时长配额、TTL、重试或新检查点协议。旧表格无依据限制已连同校验分支及专属拒绝断言删除。测试用解析绑定仅在 cfg(test)/集成夹具中，生产旧缓存仍不能取得当前授权。此结论不代表发行包或设备验收完成。

### 下一阶段

P6 已完成，代码提交为 `c39fd62`；文档按仓库规范独立提交。P7：正式构建/发行资源与许可证核验、干净环境启动、自定义导入样例真实网络、全量目录与约定范围设备/手柄验收。实际游戏安装、设备部署和发布按后续用户授权执行；未执行项保持待验证。

## 2026-10-31：P7 开始发行与全面验收

- 用户要求“提交并开始 P7”。P6 拆分提交为 `c39fd62`（代码）与 `270ca13`（文档），开始本阶段前工作区干净；未推送/合并。
- 沿用 P6 已通过的 Windows 668 / Linux 679 项回归及双端/契约/lint 证据；未修改对应实现，不重复完整门禁。P7 新增验证针对 Release 优化构建、发行包资源/依赖、干净状态、自定义样例真实网络与全目录行为。
- 本轮先完成可在本地隔离状态执行的验收。设备部署、真实游戏安装和实体手柄操作尚未执行，不能将本地构建或自动化写成设备通过。

### P7 本地构建、打包与依赖结果

- Windows/Linux Release 核心和 worker 均以 `--release --locked` 构建通过。Windows 使用 `scripts/build-windows-flutter.ps1 -SkipRust -Preview` 构建实际 Release UI；这里 `Preview` 只跳过已通过的分析/测试门禁，生成的仍是 Release 包，未将其称为安装版界面验收。Decky 前端构建与 `package:plugin` 通过，Linux ELF 与 runtime manifest 相符。
- 发现原打包流程只记录 Lua/mlua 的 MIT 类型，未携带正文。补入 Lua 5.4.9 原始许可声明、mlua/rlua 及 lua-src wrapper 的 MIT 正文；Windows build/ZIP/NSIS 和 Decky 插件均携带相同正文，更新清单包含相应文件。SBOM 增加许可证路径与 SHA-256，并区分实际内嵌 Lua 5.4.9 与 `lua-src 551.0.2` crate 版本；没有增加运行时依赖。
- `cargo metadata --locked` 与当前 Flutter pub deps/SDK 元数据生成 CycloneDX SBOM，共 518 个组件（含开发/目标平台依赖，不是实际可达性分析）。Lua/mlua 三个相关条目许可与正文摘要齐全；沿用的 Flutter 收集器仍将 6 个 SDK 包的独立许可元数据标为待核对，并指向包内 `NOTICES.Z`，不能写成全量许可证审核已完成。
- 生成 Windows portable ZIP、NSIS 和 Decky ZIP，本地未发布、未打 Tag、未推送。该候选版本仍为项目当前 `0.4.0`，不因本地验收自行修改版本。

| 本地候选 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `patchhub-windows-portable.zip` | 19896578 | `0efc0bfdfb17ab0be0fe0c78cd8a1f0ec3aba17a44a02a764fc0708ac22cbf6b` |
| `patchhub-decky.zip` | 8272487 | `f634c5f58cdd55aed87ac3cade201aaaaf024ecfbec6e93c34ba60d1074ce822` |
| `patchhub-windows-x64-setup.exe` | 15002357 | `66db71ff85ac2c2c4314b24e0d1b2d6be87d22a09cbf52c78e11146d3fe6e7b7` |

- 从上述两个实际 ZIP 解包，在临时工作目录、空 `PATH` 和不存在的状态目录启动各自核心，读取四源完整源码与 SHA-256，均与 Git Lua 资源逐字一致；读取没有创建状态。随后显式覆盖内置脚本为顶层无限循环，保存/读取成功且没有执行该循环。两包许可证正文与源文件相同；没有外部 Lua 可执行程序/DLL/SO。Linux `ldd` 只列系统 libc/libm/libgcc/loader；Windows `dumpbin /DEPENDENTS` 无 Lua/Python DLL。此为本地包内核心验证，不是 Deck SteamOS 实机兼容性结论。
- 包装回归 11 项、SBOM 回归 1 项、Flutter SBOM 回归 1 项通过；新增拒绝 Decky 过期许可证正文与 Lua 运行时标注断言。版本同步检查和差异格式检查通过。

### P7 真实网络与隔离安装

- 实际 Windows Release `serve-task update_source`：Kagura 国内完整索引 **472** 条，海外完整索引 **229** 条，均 `complete=true/source_refreshed`。这里完整指来源脚本完成其配置入口/分页并通过核心验证，不声称穷尽网站未公开或未链接的页面。
- 新增独立[产品详情页作者示例](examples/product-pages/source.json)与[源码](examples/product-pages/adapter.lua)，通过相同 Release RPC 导入、更新发布 1 条完整配置范围索引、跨进程读取。该脚本只采集显式配置的详情页，不把单页示例当成全目录。占位表格示例继续是合成离线输入；复杂内置四源的其他真实详情/补查证据沿用 P6，无重复无意义请求。
- 新增 `installer_service::script_source_update_to_isolated_zip_and_exe_install`：真实 Lua 生产分派提取合成列表/详情 → 发布索引 → 匹配 → 解析当前选择 → 公共 `install` 写入口；分别用合成 ZIP 和仓库 SF golden EXE，在临时 Steam/游戏/状态目录安装。EXE 使用本轮实际 Release worker，预先解析不写游戏文件，安装最终 `committed`，原有 inert `engine.exe` 保持，ZIP payload 字节核对、EXE payload 长度核对。
- Windows 与 Linux 上该专项各 1 项通过（每项包含 ZIP/EXE 两种格式）；新用例依赖显式 `SF_WORKER_TEST_BIN`，默认忽略，不自动操作任何真实游戏。对应 clippy `-D warnings` 通过；没有修改生产安装逻辑，因此未重复 P6 完整回归。
- 日志与候选保存在忽略目录 `.artifacts/p7-*`，例如 `p7-network-results.json`、`p7-package-probe-{windows,linux}.log`、`p7-script-install-{windows,linux}.log`、`p7-packaging.log`、`p7-nsis.log`。

### P7 Deck 设备准备与当前边界

- 用户已明确授权“Decky 部署与验收”，不含真实游戏补丁安装。按 [decky-deploy-verify](../../../.agents/skills/decky-deploy-verify/SKILL.md) 核对当前 Devkit 配对与独立可信 known_hosts，SSH 和 Loader/CEF 可用；未绕过主机密钥校验。
- 实际目标仍安装旧品牌“Steam 黄油补丁助手 0.3.0”，目录 `steam-patch-manager`，状态也为旧名称；不存在当前 `patchhub` 插件目录。活动任务 43 项均为终态，0 活动。没有把旧终态失败/取消记录清掉，也没有操作游戏。
- 技能明确“旧品牌不自动迁移”。因此停止正式插件替换，另向用户确认备份后迁移状态、安装 PatchHub 并移除旧插件的范围；普通部署授权未被自行扩大为品牌迁移。
- 已完成首份旧插件/状态备份及回滚 ZIP，清单 492 个文件（插件 25、状态 467），并取回本地核对哈希。证据与配对路径放在忽略目录 `.artifacts/script-source-adapters-p7-deck-20261031/`，不提交设备地址、凭证、真实库路径或原始任务数据。插件备份 SHA-256 `ab277fd0d605e9b055dc35fa7c8bf15e2393bc1ef9cf763ffddcac5bac742fe9`，状态备份 `cbebf205eca525403b360fc4e2355136da7b541aad5a18fb8e81c710674c747d`。
- 正式替换、后端重载、运行时 RPC/HTTP bundle 对齐、游戏模式画面、实体手柄及 Windows 原生界面尚未验收，P7 保持施工中。当前 SSH 隧道只用于本轮 CEF，结束后关闭；远端唯一暂存目录在证据取回并核验后按技能清理，不能删除唯一备份。

### P7 本地过度设计复核

沿 `270ca13` 后差异核对打包、许可证/SBOM 和新增合成安装路径：正文缺失是实际发行问题，直接复用现有拷贝、清单和回归，没有新许可证平台或运行时配置。作者示例明确自己的页面范围，无隐藏数量/时间限制；安装验收仅增加显式依赖 Release worker 的隔离用例，没有放宽生产权限或修改身份门槛。未发现需修改的过度设计项；设备收尾仍待实际证据。

- 本轮等待品牌迁移确认期间，已把 Loader/本插件设置、回滚 ZIP 和清单一并取回本地。再次核对两份完整备份 SHA 后，仅清理本轮远端暂存目录，原插件/原状态未修改；本轮 SSH 隧道关闭。继续部署前需重新检查实时活动任务和原状态，不将此前基线永久当成当前事实。


### 2026-10-31：授权直接迁移并完成 Deck 部署与抽样验收

- 用户明确回复“**不备份直接迁移**”。按本次指示未新增备份；上一轮已完成的本地备份保持原样，不重做也不覆盖。复核 Devkit 当前配对、Loader v3.3.0、0 活动任务和目标目录不存在后执行迁移。
- 通过 Loader 正常卸载旧品牌插件以停止后端，再将旧状态及本插件设置目录直接重命名为 `patchhub`，未复制出额外备份。移动前后 **467 个状态文件逐文件哈希相等**，owner 仍为设备用户。安装后来源设置自动由 schema 2 升为 3；实际存在的 `enabled/added/games/intervals` 字段与迁移前一致，分别覆盖启用、已有规则、逐游戏订阅和间隔。旧版没有名称覆盖字段，名称保留仍以 P6 对应回归为依据。
- 使用 Loader v3.3.0 当前的[安装/卸载入口](https://github.com/SteamDeckHomebrew/decky-loader/blob/v3.3.0/backend/decky_loader/utilities.py)与[安装请求语义](https://github.com/SteamDeckHomebrew/decky-loader/blob/v3.3.0/backend/decky_loader/browser.py)，逐次核对事件返回的名称、版本、请求 ID、SHA 和类型后确认。未裸拷正式文件、未手工 kill 后端、未修改 SteamOS 系统分区或其他插件。
- 初次迁移候选为前文 `f634c5…`，随后实机发现并修复两个界面问题，重新经 Loader 安装最终候选。最终 Decky ZIP **8273287 字节**，SHA-256 **`d2cfd637040f9f6c0df78d3f750141a2006326ad348a010f1b99910af58ef131`**；最终正式目录的 **28 个文件全部匹配**，core/worker 均可执行。前文候选哈希保留为历史，当前 `.artifacts/patchhub-decky.zip` 对应此最终包；Windows ZIP/NSIS 未改变。
- 当前只存在 **1 个 PatchHub 0.4.0** 实例，旧品牌实例与旧插件目录已移除。Loader 重装曾删除本插件排序成员，已仅按原相邻位置将 PatchHub 放回 Storage Cleaner 后，其他插件及 frozen/hidden/disabled 成员关系保持。未全量覆盖旧 Loader 设置。

### P7 实机发现及修复

1. 来源页向下滚动后点击“编辑脚本”，新编辑器插入页顶但仍停留原滚动位置；实测规则输入框 top 为约 **-1065 CSS px**。在 `GlobalSources` 的编辑器容器挂载后用一次 layout effect 定位，没有定时器/轮询。重新挂载最终路由后 top 为约 **399 CSS px**，编辑器标题和首个输入在 854×534 CSS 视口内。重新部署后仍打开的旧路由会保留旧函数；通过退出再进入路由验证了实际新组件，未只凭 HTTP 新字节宣称修复生效。
2. 实际语法错误从核心带回来源和 Lua 行号，但 Loader 的异常对象 `String(error)` 只有“Python Exception”，正文放在 `pythonTraceback`。新增脚本管理边界的短格式化函数，取当前 Loader 包装的异常正文，保留完整多行 Lua 诊断，不向用户堆栈式展示 Loader 内部帧。读取、保存失败和保存后刷新失败复用同一处理；没有改变核心错误码或 RPC 形状。新增回归使用真实错误形态，并覆盖正文内再次出现 `Exception:` 时不截断。

修复后 **Decky 89 项测试、typecheck、构建/打包通过**。新增改动只涉及 Decky 展示层，沿用未变核心/Windows/契约的 P7/P6 验证，不重复完整 Rust 回归。按 UI/UX 技能核对可见性及错误提示，并按过度设计技能复核实际执行路径：仅新增有实机依据的容器定位和错误正文提取，无额外兼容引擎、重试、配额或状态平台。

### P7 最终设备证据

- 正式文件、本地构建与设备 HTTP `dist/index.js` 字节一致：**149567 字节**、SHA-256 **`2a1aa9d3c74a90f5d1a7031dc8a81eeabda940ea3e6b6a23d52434b83d3e6524`**；当前已挂载新组件，脚本编辑行为与新实现相符。
- 真实 `scan()`（当前插件为无参数签名）返回 **27 个安装实例**，`list_transactions` 为 **25** 条，历史 installer task **43** 项均终态。两个扫描诊断均为 `library_unavailable`，未将不可用库隐藏为成功；没有调用游戏安装/卸载/回滚接口。
- 四个内置脚本经实际 RPC 读取，在设备浏览器内重新计算源码 SHA，与核心描述及本地源码相同；API 1，作者规则为 `adapter=lua`。来源、间隔及开关保持。
- 真实 QAM 的 PatchHub 主界面、页底设置/扫描入口、全屏来源卡片和编辑器均截图检查；完整 SHA 可换行。无横向溢出。截图是设备 CDP/DOM 操作证据，不代表实体手柄或 Steam+X 键盘输入已验收。
- 在实际编辑器输入 `return {` 并提交，显示 `script_source_syntax_error`、`source=patchmoe` 和 `source/patchmoe.lua:1`；草稿保留，来源设置文件 SHA 仍为 `8cc2a9efa05b6a13f83d60bfe99387c5513d09cc519859357398476ce35418b5`，没有留下有效用户覆盖。随后关闭测试草稿。
- 实际设备运行 Shiravune Lua 元数据更新：经历 `running/index`，最终 **completed / complete=true / 165 条 / revision 2**。随后实际启动 Kagura 国内元数据更新并取消，最终 **cancelled / task_cancelled / running=false**。仅请求网页元数据，未下载补丁，未改变自动更新间隔。源码缓存因新解析身份需更新的提示符合 P6 设计。
- 重装通过 Loader 正常停止/启动后端，最终 RPC 和用户策略对照均通过。期间一次 Loader 自身出现“5 秒停止后 SIGKILL”的告警，未由本任务手工终止；后续日志可见卸载完成、0.1 秒停止与重新加载，未形成遗留活动任务。保留日志，不将这次过程描述为全程无告警。
- 最终证据、截图、迁移摘要与相关 Loader 日志位于忽略目录 `.artifacts/script-source-adapters-p7-deck-20261031/`。未把配对凭据、设备地址、真实库路径或原始任务数据加入 Git。

### 当前剩余验收

P7 保持施工中：Windows 原生桌面界面的实际导入/取消/重启，Deck 实体手柄焦点/返回及 Steam+X 输入、设备界面的有效自定义导入保存，仍待相应实际验证。真实游戏补丁安装和游戏内生效不在本轮授权范围。未发布或合并；P7 代码提交与本轮推送授权见末节。

- 本轮结束前已取回最终部署/迁移摘要、策略对照与相关日志；核验正式 28 文件后仅清理本轮远端唯一迁移暂存目录。恢复原 Steam 导航、关闭菜单/草稿、移除本轮临时监听与调试变量，并关闭 SSH 隧道。新插件和迁移状态保持运行，Loader service 为 active。


### 2026-10-31：按用户要求拆分提交并推送功能分支

用户明确要求“提交 GIT 并推送”。沿用本 session 已完成且实现未再变化的验证，不重复运行测试；提交前核对工作区与暂存区并执行差异格式检查。同一 `index.tsx` 中的两个修复按改动块分别暂存。

- `4386c12 build(sources): 补齐 Lua 发行许可证与依赖清单`
- `2c05444 test(sources): 验证脚本候选的隔离 ZIP 与 EXE 安装`
- `d35f7d2 fix(decky): 打开脚本编辑器时定位到可见区域`
- `dfd8d37 fix(decky): 保留脚本 RPC 的完整 Lua 错误正文`

本文、设计/索引及作者示例另作独立文档提交。推送目标为 `origin/feature/script-source-adapters`，包含此前尚未推送的阶段提交；未要求合并 main 或发布。提交/推送不改变上文仍待实际验收的事项，P7 不标为全部完成。
