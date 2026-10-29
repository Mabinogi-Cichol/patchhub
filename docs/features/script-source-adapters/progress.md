# 可导入脚本来源适配器施工记录

[设计文档](design.md) · [施工计划](implementation-plan.md)

## 当前状态

**施工中。P0–P3 已实现核心导入、运行时及发行商分派、缓存/安装绑定；P3 双平台自动化已通过。补查聚合、双端入口、全部内置来源迁移和发行/实机待 P4–P7。**

- 调研基线：`main@cb06188`，2026-10-30。
- 文档开发分支：`feature/script-source-adapters`。
- 用户先要求开始施工，随后逐阶段要求提交和继续施工，本轮明确“提交并完成 P3”。P0 代码/文档为 `138c0c6` / `57dd65c`，P1 为 `901e592` / `5abf92a`，P2 为 `4c97d0e` / `ca581e9`；P3 代码已提交为 `03cb2ae`，文档独立提交。未推送、合并、部署、安装或操作设备。
- 已有来源名称编辑/示例功能属于此前工作，不是脚本适配器功能的实现或验收证据。

## 阶段状态

| 阶段 | 工作 | 实现 | 自动化 | 实机/真实网络 |
| --- | --- | --- | --- | --- |
| P0 | 运行时原型与 API 合同 | 原型及合同已完成 | Windows / WSL Linux 各 18 项通过 | 3 个真实页面已录制；非全站/Deck 实机验收 |
| P1 | 导入、读取与原子设置 | 核心已完成 | Windows 10 / Linux 9 专项通过，完整回归通过 | 本地隔离状态验证；无 UI/设备验收 |
| P2 | 运行时、宿主能力与取消 | 核心执行器已完成 | Windows/Linux 运行时专项各 8 项，完整回归及 lint 通过 | 录制夹具及本地 TCP 验证；无新增网站/设备验收 |
| P3 | 发行商索引/详情与缓存、安装一致性 | 已实现 | 两平台 9 项链路专项与完整范围回归通过，Windows workspace lint 通过 | 本地 HTML/JSON 夹具与真实核心调用；无新网站/设备/实际安装验收 |
| P4 | 脚本补查与多源聚合 | 待实施 | 待验证 | 待验证 |
| P5 | 协议与两端导入/编辑 | 待实施 | 待验证 | 待验证 |
| P6 | 全部内置来源/通用表格脚本迁移及旧 Rust 路径退役 | 待实施 | 待验证 | 待验证 |
| P7 | 全面验收、发行包与约定范围实机验证 | 待实施 | 待验证 | 待验证 |

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

## 下一步

进入 P4，接入脚本补查事实的核心候选生成与多来源聚合，覆盖混合成功/失败、完整无结果、来源集合与脚本变更的查询键失效。P5–P7 的双端入口、全部内置来源迁移与发行/实机验收仍按原计划实施。
