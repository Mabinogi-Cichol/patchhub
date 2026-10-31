# 未决边界核查与最终决策

日期：2026-10-17。基线：`80829ea`。关联：[设计](design.md) · [施工记录](progress.md)。

本轮通过当前完整函数、调用方、现有测试源码和历史实测记录核查原计划未决项。以下是本轮确定的实施决定，不再把删留判断留给实施者。没有运行生产安装、访问设备或重跑历史测试；引用的历史复现不冒充本轮实测。保留策略的数值是当前实现支持范围，不宣称是格式理论上限或经过最优容量测定。

## 1. 来源本地数据：删除任意字节门槛

定位：[mod.rs](../../../crates/core/src/sources/mod.rs)、[store.rs](../../../crates/core/src/sources/store.rs) `read_bounded/write_json/write_atomic`、[registry.rs](../../../crates/core/src/sources/registry.rs) `commit`、[validate.rs](../../../crates/core/src/sources/validate.rs) `validate_candidates`、[kagura.rs](https://github.com/Mabinogi-Cichol/patchhub/blob/cb06188/crates/core/src/sources/adapters/kagura.rs)。

- 512 KiB候选/发现结果：canonical编码和serde序列化已完成才测长；不是防止这些分配发生的机制。commit超限先删除attempt.index再重编码，仍超限则丢弃此次持久化。这是完整结果被拒绝/降级，而非格式不合法。
- 4 MiB缓存/索引/设置：读端在完整JSON解析前测长，写端在完整序列化后拒绝。JSON结构及原子写入没有该固定容量要求；没有查到这些具体阈值对应真实瓶颈的证据。
- 4096字节status/schema：固定小结构已有类型/schema验证；没有独立必要性，删除同类字节拒绝，不为小文件单独保留一套测长API。
- 32来源：不是格式宽度或执行并发约束，删除。

**决定：全部列入S08删除。**保留文件类型、链接/身份变化、JSON结构/schema、原子写入与真实IO错误。移除仅为限额服务的先序列化测长、缩减索引副本和错误分支；共享索引本身仍用于复用，不删除它。来源store辅助函数可精简，但不要连带修改detection中其他调用者的独立语义。大数据读取仍会消耗实际内存，本结论不是“无限资源”承诺，也不预建分页系统。

## 2. HTML 4 MiB：删除固定字节拒绝

定位：[http.rs](../../../crates/core/src/sources/http.rs) `ReqwestFetcher::fetch`、`fetch_html`、`fetch_with`。

原始响应通过take(MAX_HTML+1)截取，fetch_html再检查原始和解码后长度；fetch_with缓存响应副本。4 MiB并非远端HTML协议上限，解码后的UTF-8还可能比原始编码更大。现有代码和记录没有给出这一数值的实测瓶颈；设置限额也未消除正文和缓存副本。

**决定：S19删除take截断及两次大小拒绝，按完整正文处理。**正文保持与基线一致的单次完整读取，取消沿用请求/操作级检查（读取进行中不另设分块取消）；不新建通用流式DOM或调度框架。保留HTTP错误、编码处理、挑战页识别、主机许可、逐跳校验及已有网络故障语义。测试验证原始正文超过4 MiB、解码后超过4 MiB仍完整，不再把长度超标当网络或格式失败。

## 3. 清理深度128：删除并采用局部显式栈

定位：[cache.rs](../../../crates/core/src/installer/cache.rs) `walk/inspect_entry/remove_entry/Node::delete`（Unix和Windows）、[backup.rs](../../../crates/core/src/installer/backup.rs) `material`。

深度计数独立于路径是否合法；超过128层的短目录名路径仍可能是合法路径。当前确有递归调用，Unix删除还持有捕获后的目录与身份，不能只删判断并声称栈风险已解决。

**决定：S16删除128层拒绝，同时将这几条遍历改为局部显式栈，按后序完成删除。**栈帧保留当前打开的节点/父身份、尚未处理的子项和必要的Unix隔离目录清理状态；沿用原有捕获、复核、失败恢复规则，不创建新的持久任务状态机。句柄打开失败等按真实OS错误报告，不换成另一任意深度阈值。验证深于128层的合法目录盘点/清理，且链接和并发替换不能越界。

## 4. 诊断512条和2048字节：随响应上限取消而删除

定位：[cache.rs](../../../crates/core/src/installer/cache.rs) `Walk::fail`、[backup.rs](../../../crates/core/src/installer/backup.rs) `Scan::fail`、[outcome.rs](../../../crates/core/src/installer/outcome.rs) `bounded_detail`、[projection.rs](../../../crates/core/src/projection.rs) `DomainError`、[protocol/lib.rs](../../../crates/protocol/src/lib.rs) `truncate_detail`及其宿主/生成契约调用方。

512条只保留前面诊断并用省略提示覆盖末条；不影响遍历继续，但丢失具体失败位置。2048字节截断的真实历史理由见[P5复核](../rust-core-consolidation/p5-review.md)：9 MiB历史诊断触发旧响应大小限制，可能掩盖已提交终态。此问题真实存在，但在S09删除响应硬上限后，不再需要靠截断信息规避。

**决定：S17在S09配套完成时取消诊断条数上限、detail字节截断及接收端长度拒绝。**保留结构化code、disposition与提交状态独立推导；不改写历史数据库原串。共享MAX_DETAIL常量、Python/TS生成定义、宿主展示转换及专属断言同步处理。界面可折叠长文本，但不能在数据层静默裁剪。验证超过512条和超过2048字节完整往返，9 MiB旧诊断重放仍保留正确终态。

## 5. 请求64 KiB：删除整行大小拒绝

定位：[protocol/lib.rs](../../../crates/protocol/src/lib.rs) `parse_request`、[cli/main.rs](../../../crates/cli/src/main.rs) `read_request`、[bridge.py](../../../apps/decky/backend/bridge.py)、[Windows宿主](../../../apps/windows/src-tauri/src/main.rs)。

该上限仅为本项目自定协议策略，JSON Lines本身无此要求；add_source传递整份RawSourceRule、扫描传递host/名称集合等变长字段，并不存在“合法请求必定小于64 KiB”的类型契约。多处发送前检查和CLI读取/协议解析重复限制完整请求，没有外部API的64 KiB要求。

**决定：S18删除普通业务请求整行上限，保留逐方法schema、方法白名单、身份/授权边界、JSON分帧及输入错误。**不把worker描述符和renew/cancel控制消息等不同通道的限额一并删除。验证合法大请求可跨两端解析；错误结构仍失败且后续请求仍能处理。

## 6. 解析器资源保护：保留当前受限解释与解码模型

定位：[audit.rs](../../../crates/installer-format/src/audit.rs) `Decoder::decode/Reader::string`、[lua.rs](../../../crates/installer-format/src/lua.rs)、[behavior.rs](../../../crates/installer-format/src/behavior.rs)、[production.rs](../../../crates/installer-worker/src/production.rs)。

| 保留机制 | 已核实的直接作用和依据 |
| --- | --- |
| LZMA字典128 MiB和解码器mem_limit | 压缩流prop直接决定分配；prop=40要求约4 GiB字典，生产worker额外内存基线3 GiB。流式输出不能消除字典分配，因此不能删字典检查后依赖OOM。现有container测试覆盖dictionary_limit。128 MiB是当前解码支持预算，不是LZMA格式最大值。 |
| 保留元数据16 MiB和字符串读取边界 | Decoder的retain路径把解压结果留在Vec中，再交对象图、脚本解析及证据编码；压缩流可由小输入生成大驻留数据。当前限制只约束保留元数据，载荷按流处理，不重新限制整个补丁大小。保留当前有限内存解析模型及其16 MiB预算，不将其称为磁盘容量限制。 |
| Lua源/令牌与解析深度、调用深度、求值步骤和分支预算 | AST保留及递归求值不是一般文件枚举；递归函数可不终止，符号分支会指数扩张。semantics_bounds含递归函数、11个二分条件、长链AST、token规模的具体输入。保留当前受限解释器预算，不将步骤预算误作任意墙钟超时。 |
| 累计语义分配64 MiB、表DAG/条件8192节点及深度边界 | P2历史记录有约603 KiB输入通过别名深复制保留约5 GiB的复现；当前Rc/COW解决别名复制，但160次大字符串拼接仍产生不同对象，累计分配检查有独立作用。现有测试同时覆盖共享别名成功、真实分配超限失败和20轮表DAG指数展开。 |
| XML namespace深度96和命名空间驻留/结构校验 | 当前解析维护namespace栈和属性处理，不执行脚本；历史有小XML导致URI逐属性复制约5 GiB的证据，现有container测试覆盖不同深度、重复扩展名与URI共享。保留当前解析结构保护，不把目录深度S16的决定套到解释/解析栈。 |

证据：[P2历史复核](../cross-platform-installer-migration/p2-gate-review.md)、[语义边界测试](../../../crates/installer-format/tests/semantics_bounds.rs)、[容器测试](../../../crates/installer-format/tests/container.rs)。这些证据证明资源保护有现实对象，不证明每个数值是数学最优；本轮明确决定保留现有解析器支持预算，不安排实施阶段再决定是否删除。S13–S15是额外的包/资源计数和恒真分支，仍删除，与此不冲突。

## 7. 超时与终态排空：明确分离

全仓apps/decky Python检索：生产代码无额外传入timeout=具体值的调用，只有桥接参数转发；删除S10默认300秒及仅为其存在的参数，不需要保留一个空配置口。

保留终态后的1秒/8 MiB前台排空转后台观察机制。此时真实终态已经收到，阈值只限定UI等待尾部诊断，不取消正常工作、不改写首终态、不释放仍被writer占用的锁。已有[P5验证](../rust-core-consolidation/p5-validation.md)及test_v4_bridge.py覆盖never_eof/continuous_tail/exit_wait。S09删除MAX_RESPONSE_BYTES时将该独立尾部预算明确定名保留，避免误删。

网络连接/单次读取故障超时、worker启动探针和锁竞争等待不等于普通调用总时限；本轮保留其当前故障语义，不把它们扩展成新的待调查清单。没有用这些机制为正常任务300秒终止辩护。

## 结论

原计划未决项全部给出决定：S08确定删除来源本地数量/字节拒绝；新增S16清理深度、S17诊断截断、S18业务请求上限、S19 HTML大小拒绝；解析器受限执行/内存模型和终态排空明确保留。后续只剩实施及行为验收，不再保留“执行时决定删留”的任务。
