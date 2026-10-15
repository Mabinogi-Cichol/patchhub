# 全项目限制与材料生命周期审计

日期：2026-10-15。关联：[设计](design.md) · [施工记录](progress.md)。状态：初稿静态审计历史记录；2026-10-16已按删除优先复审，已有未提交实现及局部回滚，未实测容量。代码基线 HEAD `be099fa`；行号对应本次读取快照，后续施工需重新定位。覆盖下列主链路，不将一次静态搜索当作全项目无缺陷证明。

## 2026-10-16复审：旧建议的取代关系

本文件原表保留初次审计证据，**原“判断与计划”不再全部有效**。当前决定以[设计第6节](design.md#6-删除优先的精简原则2026-10-16修订)及[删除清单](design.md#10-删除清单与范围核对)为准。代码施工已暂停；此前已局部回滚错误新增逻辑，尚未实施全部删除。本次审阅后仅收紧文档：容量计算限定为固定阶段函数；保留的字节上限须说明资源依据；查询全部来源结束后进入终态并保留错误。当前执行顺序见[设计第7节](design.md#7-实施顺序与当前状态)与[施工待办](progress.md#后续施工待办当前执行入口)，旧表不直接作为施工清单。

| 旧项 | 复审决定 | 新编号 |
| --- | --- | --- |
| A01/A02：保留短期授权、长任务后重签 | 删除TTL/续期和独立短期许可状态机，恢复映射并入既有记录 | D01 |
| A02/S07：延长或续期前台租约 | 删除心跳、watchdog、lease落盘；保留cancel/EOF和进程/事务恢复 | D02 |
| S03/S05/S06：预算耗尽后反复续轮 | 删除总deadline/请求额度及续轮双模式；真实分页游标仍保留 | D03 |
| 第6节及原设计9.1：固定轮数无进展暂停、长退避结束 | 删除轮数/300秒截止，按来源等待、按实际游标去重 | D04 |
| F03：统一按大小或设备计算执行秒数 | 删除正常工作累计CPU/墙钟截止及大小公式；保留实际隔离和解析资源边界 | D05 |
| I02/I03/A01：逐个补未知事实豁免、全快照再比对 | 删除全事实/manifest摘要相等门槛，只核对实际动作依赖 | D06 |
| A02/S04：全局revision转相关摘要 | 删除全局业务否决及重复绑定，不另叠摘要平台；存储并发保护保留 | D07 |
| S01/S02：数量许可线变公式、先建设分页平台 | 删除无独立资源依据的数量线/静默截断，保留真实字节边界 | D08 |
| I04：首期继续硬编码来源名单 | 删除支持格式手动安装的来源名称/builtin门槛，保留下载身份和隔离解析 | D09 |
| N01：MIME拒绝按样本加例外 | 删除仅凭MIME否决，实际内容/格式/摘要决定有效性 | D10 |
| D01：额外开发逐成员释放stage过渡路线 | 取消新过渡路线；新计划无stage，旧事务仅保留必要恢复兼容 | 设计第4节 |

新增证据：`ManifestFacts.sha256`绑定完整Steam manifest，授权使用`facts != current.facts`，会将无关字段变化带入安装失效；`commit_if_revision`在来源查询提交处使用全局revision。候选`size_of`公式未包含堆内存。EXE重装仍从authorization文件读取内容映射，删除许可机制时必须保留并迁移这项实际用途。CLI已有cancel和stdin EOF取消，双方宿主仍包含renew线程/循环，需一起清理并验证进程收束。

本轮已回滚新增的progressive/renew_slice/整轮进展hash/固定暂停/全局Retry-After等待包装、大小推导时限公式、候选size_of公式及专属测试/文案。回滚至基线的29秒/24请求、旧worker时限、128候选仍待直接删除；回滚不等于完成D01–D10。详见[施工记录](progress.md)。

## 分类和证据规则

- A：人为许可/重复门槛，可删除或收敛；仍需调用链回归。
- B：现有实现能力/资源边界，先补能力再放宽。
- C：结构、内容、身份、恢复一致性检查，有正面作用，保留。
- H：历史已修复，仅列回归基线。
- Q：静态风险或证据不足，实施时补复现，不将常量本身等同真实失败。

优先级 P0 表示优先处理的兼容或空间问题；P1 表示后续能力工作；P2 表示进一步精简。审计编号与设计阶段 P0–P6 不同。

## 1. 磁盘与生命周期

| 编号/优先级/分类 | 证据与当前行为 | 判断与计划 |
| --- | --- | --- |
| D01 / P0 / A+B | [install.rs:1169–1277](../../../crates/core/src/install.rs#L1169)：prepare为所有映射逐成员写stage；[1635–1715](../../../crates/core/src/install.rs#L1635)：全量backup后stage复制到游戏.tmp再rename | 确认重复磁盘副本。省整份stage需要载荷源/租约/journal/前缀恢复能力，不是删一行copy |
| D02 / P0 / A | [space.rs:ZipSpaceBudget](../../../crates/core/src/installer/space.rs#L221)：下载已在盘，stage前state=P+O+M，target=P+O+M，stage后才扣P | 按串行写/恢复及目标净增长计算后续峰值；不把P+O两种scratch相加 |
| D03 / P0 / A | [space.rs:41–63](../../../crates/core/src/installer/space.rs#L41)：EXE state=2D+2P+O+2M，target=P+O+M；[exe_orchestrator.rs:228、313](../../../crates/core/src/installer/exe_orchestrator.rs#L228)两次使用同一预算 | 实时空闲已经扣掉已有下载/输入/产物；改为当前→后续的增量预算，Z用真实/预测输出字节，不再默认等P |
| D04 / P0 / A | [exe_orchestrator.rs:134–156](../../../crates/core/src/installer/exe_orchestrator.rs#L134)：下载前固定2GiB+M；下载后又要求D+32MiB | 前者可拒绝小包，后者主要为下一次沙箱输入；统一阶段材料预算，禁止将所有预检一概删除 |
| D05 / P0 / B | [production.rs:29–34](../../../crates/installer-worker/src/production.rs#L29)、[133–180](../../../crates/installer-worker/src/production.rs#L133)、[222–224](../../../crates/installer-worker/src/production.rs#L222)、[495–513](../../../crates/installer-worker/src/production.rs#L495)：WorkerExecution持有整个InvocationDirectory，沙箱复制输入/worker，TempDir在Drop清理 | [exe_orchestrator.rs:244](../../../crates/core/src/installer/exe_orchestrator.rs#L244)已有drop分析对象，但在预算之后；转换artifact与输入拆所有权，关闭worker/句柄后才释放 |
| D06 / P0 / C | [install.rs:1751–1848](../../../crates/core/src/install.rs#L1751)：writing时用stage验证不完整.tmp前缀；恢复先核对全体live/backup/tmp | 是无stage改造的关键依赖。换为受控archive成员流/可信检查点，不能删stage后沿用原恢复代码 |
| D07 / P0 / C | [install.rs:2176–2230](../../../crates/core/src/install.rs#L2176)：仅transient_recovery且指定终态清stage/backup，清理异常作为cleanup_pending诊断 | 自动清理保留恢复材料；不能声称所有历史/持久收据的备份都在成功时删。逐成员早释需显式记录材料可选状态 |
| D08 / P1 / C+Q | [cache.rs](../../../crates/core/src/installer/cache.rs)、[cache_lock.rs](../../../crates/core/src/installer/cache_lock.rs)、[backup-cleanup设计](../backup-cleanup/design.md)：工作区正在加入全量backup清理和共享锁 | 新archive租约接入最终锁实现，不复制另一套清理；保留“空闲时用户直接全清、不按事务状态筛选”的既定约定；不借本方案修改另一聊天代码 |

## 2. 格式、资源与执行能力

| 编号/优先级/分类 | 证据与当前行为 | 判断与计划 |
| --- | --- | --- |
| F01 / P0 / B | [acquire.rs:33、152–155](../../../crates/core/src/installer/acquire.rs#L33)，[audit.rs:21–26](../../../crates/installer-format/src/audit.rs#L21)，[exe_report.rs:19–22、597、801–820](../../../crates/core/src/installer/exe_report.rs#L19)，[sf.rs:12–14、65–89](../../../crates/installer-worker/src/sf.rs#L12) | EXE大小、单文件/总解压量与artifact多层2GiB上限。拆分input/output/metadata/memory预算，端到端同步解除；不能只改下载常量 |
| F02 / P0 / B | [sf.rs:208–224](../../../crates/installer-worker/src/sf.rs#L208)：只读mmap整包；[audit.rs:750–855](../../../crates/installer-format/src/audit.rs#L750)：按CHUNK流式解码，只有retain元数据收集Vec | 纠正“生产链路整包读进Vec”的假设。整包映射仍消耗地址空间，窗口映射/范围读取需重构slice/offset接口及句柄不变性；先测平台实际内存语义 |
| F03 / P0 / B | [production.rs:193–243](../../../crates/installer-worker/src/production.rs#L193)：分析180/240s，转换480s，内存3GiB，CPU300s，artifact文件上限跟sf；[audit.rs:818](../../../crates/installer-format/src/audit.rs#L818)循环检查内部deadline | 增大文件门槛不保证大包能跑。统一可取消预算和阶段进度，区分CPU/墙钟/地址空间/提交内存；保留进程隔离与单进程限制 |
| F04 / P0 / B | [sf.rs:338–486](../../../crates/installer-worker/src/sf.rs#L338)：已有ZIP64局部头与尾记录，成员start<=u32::MAX，rows.len() as u16；[audit.rs:231–232](../../../crates/installer-format/src/audit.rs#L231)SF表计数读u16、策略限8192 | ZIP64是部分能力，不是完全没有。补central的64位offset/count，所有narrowing checked；SF格式计数和ZIP输出计数分开。兼容小包既有字节/摘要 |
| F05 / P0 / A | [facts.rs:28–29、249–297、763](../../../crates/core/src/installer/facts.rs#L249)，[exe_report.rs:252–254](../../../crates/core/src/installer/exe_report.rs#L252)：digest_file流式，但baseline/条件文件调用仍传512MiB | 无需缓冲内容的摘要不应受内容Vec上限约束；分开digest与read_file限额，保留取消、身份前后复查、摘要/CRC |
| F06 / P0 / A | [space.rs:12、original_bytes](../../../crates/core/src/installer/space.rs#L12)：被覆盖原文件合计>16GiB返回exe_original_size_limit；[exe_orchestrator.rs:283](../../../crates/core/src/installer/exe_orchestrator.rs#L283)摘要也传16GiB | 属人为总量许可线，改为checked算术和实际卷容量；新安装仍需要O大小的可恢复备份 |
| F07 / P1 / B+C | [audit.rs:24–28](../../../crates/installer-format/src/audit.rs#L24)，[exe_report.rs:19–22](../../../crates/core/src/installer/exe_report.rs#L19)，[graph.rs](../../../crates/installer-format/src/graph.rs)，[lua.rs](../../../crates/installer-format/src/lua.rs)：16MiB元数据、8192节点/成员、96深度、Lua100000 token等 | 资源边界不是样本白名单。区分二进制布局真实范围、图深度/CPU防护和可分页文件清单；针对合法大输入扩容，拒绝指数/深递归资源耗尽 |
| F08 / P1 / C+B | [audit.rs:967–1105](../../../crates/installer-format/src/audit.rs#L967)：PE overlay、schema、metadata名称布局、payload边界/CRC/流结束；[behavior.rs](../../../crates/installer-format/src/behavior.rs)及[lua.rs](../../../crates/installer-format/src/lua.rs) | 真实布局与必要语义检查保留。额外metadata记录、未知schema/密码对象、必要Lua动作需要格式/执行实现后才开放，不作为默认值名单一删了之 |
| F09 / P1 / A+B+C | [zip_recipe.rs:204–265](../../../crates/core/src/installer/zip_recipe.rs#L204)，[install.rs:1093–1152](../../../crates/core/src/install.rs#L1093)：extra均按TLV遍历，未知tag通常放行；NTFS要求固定32字节/固定子字段；0x000d/0x756e拒绝 | 固定无副作用时间戳布局可泛化，未知tag不应用属性。Unix extra中可能有链接语义，必须先解析区分/证明不会创建链接，再决定是否允许；保留TLV界限 |
| F10 / P1 / B | [zip_recipe.rs:263–265](../../../crates/core/src/installer/zip_recipe.rs#L263)与[install.rs:1114](../../../crates/core/src/install.rs#L1116)：两层以不同方式限制原始/解码名称，CP437等名称可能无法贯通 | 共用无损解码与目标规范化规则，补UTF-8、合法Unicode extra、非ASCII历史编码样本；禁止lossy路径落盘 |
| F11 / P1 / C | [zip_metadata.rs:5、42–187](../../../crates/core/src/zip_metadata.rs#L5)：64MiB目录/计划元数据预算，含ZIP64预检 | 不恢复旧ZIP固定payload许可，不删目录预算；若大成员数合法包超限，先流式索引/分块计划，并计入协议与授权内存 |
| F12 / P0 / H | [policy-inventory.md](../generic-exe-installation/policy-inventory.md)：普通配置默认值、尾字节、事件顺序、存档名称等已处理 | 本功能只补回归。未确认语义的进度字段、隐藏While副作用、package后置字符串不猜测为新阻断，也不声称已等价支持官方运行时 |

## 3. 候选、身份与来源发现/下载

| 编号/优先级/分类 | 证据与当前行为 | 判断与计划 |
| --- | --- | --- |
| I01 / P0 / H+C | [manual_selection.rs:10–31、34–105](../../../crates/core/src/installer/manual_selection.rs#L10)：手动ID绑定实例/AppID/候选/具体镜像；已知其他AppID不得生成手动选择；[projection.rs:475–503](../../../crates/core/src/projection.rs#L475)产出手动项 | 保留显式选择，不能把缺少版本/variant/DLC变成按钮禁用。已明确绑定其他游戏的候选和实例错误仍拒绝 |
| I02 / P0 / A | [exe_resolve.rs:34–44](../../../crates/core/src/installer/exe_resolve.rs#L34)、[resolve.rs:154–169](../../../crates/core/src/installer/resolve.rs#L154)：手动resolve返回后仍进入同一language/DLC expectation；[facts.rs:414–439](../../../crates/core/src/installer/facts.rs#L414)unreviewed仍要求build存在/一致，DLC完全无安装字段标unknown；[exe_orchestrator.rs:177–185](../../../crates/core/src/installer/exe_orchestrator.rs#L177)全体facts失败即拒绝 | 已有手动入口不代表端到端无元数据门槛。改required facts集合；未知非依赖只提示，实际安装器/可信声明必需事实不明仍报告其依赖，不能伪装条件为真 |
| I03 / P0 / A | [projection.rs:553–556](../../../crates/core/src/projection.rs#L553)：所有ZIP无game.buildid均阻断；[orchestrator.rs:661–668](../../../crates/core/src/installer/orchestrator.rs#L661)当前build被传到verified列表；[facts.rs](../../../crates/core/src/installer/facts.rs)后端再次检查 | 手动非签名复制路线build缺失不应一概拒绝；拆“观测构建”与“审核版本”，同步projection/resolve/facts/auth/install，不只改UI。已审核签名recipe的真实版本要求保留 |
| I04 / P1 / B+C | [exe_resolve.rs:55–108](../../../crates/core/src/installer/exe_resolve.rs#L55)：自动路径限kagura-cn/en、publisher/builtin、Base/Dlc；手动在此前返回；[manual_selection.rs:65–71](../../../crates/core/src/installer/manual_selection.rs#L65)仍限内置kagura来源；[exe_report.rs:284–301](../../../crates/core/src/installer/exe_report.rs#L284)内容ID现已接受unknown/demo | 不能误报手动unknown被content_id阻断，也不能说手动绕过来源许可。未来用能力/信任契约替代ID名单；自动匹配谨慎条件保留为自动路线准入，手动候选不套自动分类条件 |
| S01 / P1 / B+Q | [sources/mod.rs:29–40](../../../crates/core/src/sources/mod.rs#L29)：HTML4MiB、entries4096、candidate128、output512KiB、cache4MiB、mirror32/64、evidence64；[validate.rs:526–600](../../../crates/core/src/sources/validate.rs#L526)执行检查 | 需对大来源分页/分片并显式partial。尚未复现真实来源撞线，不能凭常量断言正在漏候选；超限原因应保留到UI |
| S02 / P0 / Q | [kagura.rs:239–250](../../../crates/core/src/sources/adapters/kagura.rs#L239)：merged.truncate(4096)无本地截断诊断；[273](../../../crates/core/src/sources/adapters/kagura.rs#L273)、[568–591](../../../crates/core/src/sources/adapters/kagura.rs#L568)其余未完成靠work/pages诊断；[adapters/mod.rs:150–161](../../../crates/core/src/sources/adapters/mod.rs#L150)无诊断可判complete | 存在静默截断并可能无incomplete信号的静态路径；未做端到端4097条复现，不断言当前真实DLC因此丢失。补截断证据、pending/cursor与partial，禁止裁剪后覆盖旧complete |
| S03 / P1 / B+C | [sources/mod.rs:56–71](../../../crates/core/src/sources/mod.rs#L56)：29s/24请求、6s官方子预算、6详情、100页seen；[kagura.rs:386](../../../crates/core/src/sources/adapters/kagura.rs#L386)检查预算；[registry.rs:763–842](../../../crates/core/src/sources/registry.rs#L763)partial保留上次success | 请求预算正面控制等待，不无限爬站；继续checkpoint恢复，不把未完成当无补丁。按候选分页扩大能力，保留既有本体+DLC回归 |
| S04 / P0 / H | [persistent-source-cache记录](../persistent-source-cache/progress.md)；[sources/mod.rs:50–54](../../../crates/core/src/sources/mod.rs#L50)旧TTL常量搜索当前仅定义 | 常量存在不等于来源仍按时间过期。保持长期缓存、真正force和镜像合并；settings/rule变化导致的stale与时间过期分开 |
| N01 / P1 / C+Q | [acquire.rs:95–101、248–287、597–639](../../../crates/core/src/installer/acquire.rs#L95)：HTTPS/规则主机、逐跳重定向验证、TLS、DNS地址固定；公共IP限制已有FakeIP修复 | 保留实际下载身份与TLS，不重加公共IP启发式。5跳、MIME/HTML错误页拒绝、cookie/HTML预算等按真实网络样本查误拒，不直接删除；与EXE/ZIP URL策略差异一并审查 |

## 4. 授权、事务与双端UI

| 编号/优先级/分类 | 证据与当前行为 | 判断与计划 |
| --- | --- | --- |
| A01 / P0 / A+C | [authorization.rs:288–321、358–407](../../../crates/core/src/installer/authorization.rs#L288)：TTL<=900、facts全失败集合、settings_revision/策略/内容/候选一致性；[exe_report.rs:319](../../../crates/core/src/installer/exe_report.rs#L319)评估前也拒全部facts失败 | facts采集错误不能都当必需条件；引入required集合并携带依赖。所有权/内容/撤销/真实条件保留，不能仅删auth而绕过其他失败检查 |
| A02 / P1 / C+B | [authorization.rs:372–390](../../../crates/core/src/installer/authorization.rs#L372)、[exe_orchestrator.rs:389–425](../../../crates/core/src/installer/exe_orchestrator.rs#L389)：写前重解候选/事实/报告/产物 | 保留写前检查。短期计划与来源持久缓存不同；长期阶段后自动重新授权，按当前来源相关摘要收窄不相关settings失效，需先实现有效撤销链 |
| T01 / P0 / C | [install.rs:1596–1715](../../../crates/core/src/install.rs#L1596)、[1779–1848](../../../crates/core/src/install.rs#L1779)：写前核对、所有备份完成、写入前状态持久化、目标.tmp校验/rename、恢复全体预检 | 数据一致性检查保留。可复用同一不可变句柄/同阶段结果减少重复读，但跨进程/外部可变文件不能按mtime跳过摘要 |
| U01 / P0 / H+C | [Windows main.tsx:448、522](../../../apps/windows/src/main.tsx#L448)，[Decky index.tsx:350、439–446](../../../apps/decky/src/index.tsx#L350)：已有manual confirmation与切镜像重置；[UI index.ts:20–28、46–47](../../../packages/ui/src/index.ts#L20)合并显示及手动候选文案 | 不再重复实现已有确认；把实际后端missing/mismatch/changed/unsupported/resource_limit映射到双方，未知不显示“已审核”。新增事实策略必须到UI→RPC/CLI→核心完整验收 |
| U02 / P1 / A+C | [UI index.ts:184–235](../../../packages/ui/src/index.ts#L184)：多处“安全限制/暂不能安装”统称；[protocol](../../../crates/protocol/src/lib.rs)、[CLI runtime](../../../crates/cli/src/runtime.rs)携带能力及错误 | 展示具体阶段/实际值/设备预算/操作建议；不把未来支持写成当前可用。协议若新增required facts/分页预算，同步contracts生成文件、后端桥、Windows与Decky测试；本轮不触碰其并行改动 |

## 5. 实施前必须补的证据

1. 同卷/分卷实际峰值和释放时点，含小文件分配开销、失败恢复、Windows句柄/ACL、Deck内置盘/SD卡。当前公式均非实测数据。
2. 4097索引、129候选、8193成员等合成输入，确认从生产者到存储/协议/UI的超限传播；截断测试需区分index与候选两个层次。
3. 手动EXE/ZIP分别缺版本/分类/语言/DLC/buildid的真实或最小夹具；同时验证声明的必要DLC/基线条件不满足仍报具体原因。当前unknown/demo内容ID已支持，避免错误归因。
4. >2GiB及ZIP64成员offset/size/count边界，与小包冻结输出对照；格式u16计数不得靠放大常量突破。
5. 并行backup-cleanup落地后重新核对锁、活动材料租约、显式全部清理和旧事务恢复；当前未提交实现不是本方案施工成果。
6. 已修复来源持久缓存、本体+DLC/镜像、SF默认值和FakeIP维持回归，不重复声称本轮已完成这些实现/验收。

## 6. 2026-10-16：查询时间预算专项补充

- **S05 / P0 / A+B**：[sources/mod.rs:56–71](../../../crates/core/src/sources/mod.rs#L56)、[http.rs:474–490](../../../crates/core/src/sources/http.rs#L474)、[session.rs:154–162](../../../crates/core/src/sources/session.rs#L154)：总上下文29秒/24请求，官方首轮6秒及当前请求数加3；时间与节流消耗同一上下文。保留单请求超时/节流，取消“单轮结束即用户查询结束”的耦合。
- **S06 / P0 / B**：[kagura.rs:287–298、574](../../../crates/core/src/sources/adapters/kagura.rs#L287)、[session.rs:392–399](../../../crates/core/src/sources/session.rs#L392)：已保存并恢复checkpoint；[Decky session.ts:83–135](../../../apps/decky/src/session.ts#L83)、[Windows main.tsx:276–290](../../../apps/windows/src/main.tsx#L276)单游戏调用一次，partial返回后结束busy。需补共享任务自动续查，不能误判底层完全不支持断点。
- **S07 / P1 / Q**：[Decky bridge.py:352–354](../../../apps/decky/backend/bridge.py#L352)区分controlled与普通调用的外层timeout；[CLI runtime.rs](../../../crates/cli/src/runtime.rs)区分discover_game与扫描流式方法。需对齐整个任务链生命周期，单调高core常量不足。尚未采用户现场超时日志，不断言其所有中断均由同一层导致。
- 方案与待验证场景见[设计第9节](design.md#9-2026-10-16补充查询不能因单轮时间预算半途结束)。本次确认静态调用链，未运行网络查询、性能测试或生产改动。
