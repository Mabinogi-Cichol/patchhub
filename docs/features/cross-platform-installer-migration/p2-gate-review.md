# P2 独立安全与语义审查

日期：2026-10-08（Asia/Shanghai）。设计：[design.md](design.md)。实施边界：[p2-rust-sf.md](p2-rust-sf.md)。施工记录：[progress.md](progress.md)。

## 范围与方式

4个子Agent实际均为 `gpt-6.1-sol / high`，按用户要求不使用Astra；3个实现Agent分别负责容器、对象图、Lua/IR，1个只读reviewer审查全部P2新代码与冻结Python/P0契约。主Agent负责固定worker、黄金fixture、跨平台沙箱对照、集成与最终判断。没有Git提交、生产部署或游戏写入。

Reviewer未修改仓库，也未重复完整Rust测试。其独立执行了 `python -B` 的惰性静态基线复现，仅解析/静态lower合成语法，不执行Lua/EXE。初版与修订代码均按实际文件复核。

## 发现与处置

| 项 | 触发与影响 | 修复/回归 |
| --- | --- | --- |
| 大字符串别名放大 | 约603182字节/40004token/20002step输入，512KiB字符串赋给10000别名，Rust深复制可保留约5GiB | `Rc<str>`共享；表内容COW；累计64MiB语义分配预算；事件/全局别名压力回归 |
| XML namespace复制放大 | 512KiB URI配10000短属性，XML不足1MiB，每属性复制URI会分配约5GiB | URI一次规范化驻留ID，绑定指针缓存，属性名借用；大URI压力测试、不同prefix同扩展名/实体规范化冲突回归 |
| 数值比较精度 | 大于2^53的整数转换f64可改变分支；受保护global int与float相等判断可错误放行赋值 | 精确int/int及int/float比较，排序与受保护状态相等共用；不安全分支和runtime mutation回归 |
| 全局作用域差异 | Python `if`/`do` scope_copy后仍以原始global环境作为基线；初版Rust使用最终scope会新增准入 | 记录原始全局作用域；`do`/`if`及分支后赋值回归，保持冻结Python边界 |
| 原始环境快照性能 | 初修每全局语句复制整个env，有平方级工作量 | 仅离开origin的if/do边界冻结，否则结束时move原env；10000全局别名回归 |
| Rc表循环/大DAG | UI表可引用自身，或成指数结构 | UI更新仅允许有限标量；表DAG/条件展开8192节点及96层；循环更新与DAG回归 |

Reviewer最终结论：在当时已复核的修订代码中，阻断问题均修复，**未发现剩余需修复的安全或正确性问题**。随后实现者完成原始环境快照的性能优化，主Agent以新增压力回归和完整黄金再次验证；未将此后优化伪称reviewer重复实测。

## 覆盖与限制

审查覆盖PE/resource/XML、CArchive顺序和全覆盖、LZMA字典/长度/EOS/trailing、Lua AST/IR限额、未知调用与受保护状态、严格描述符、只读输入身份、流式ZIP、原子无覆盖与清理所有权。未发现具体路径穿越、任意action、PE区间或LZMA framing绕过。

主Agent的真实SF9.5对照另发现5条受限 `string.find` 模式的Rust raw string双重转义，已修复并增加全部模式回归。此前worker正确阻断并只产生错误证据，未写游戏。实际测试/真实报告/转换摘要以 [施工记录](progress.md#2026-10-08-p2-rust-sf与固定worker) 最终记录为准。

主Agent在真实矩阵完成后的边界检查发现：CPython的ZIP64 central/EOCD阈值为2^31−1，初版按32位ZIP字段范围省略了接近2GiB时的扩展。已补central大小/偏移extra、ZIP64 EOCD/locator与产物大小限额计入；9组不含大载荷的CPython独立header黄金覆盖阈值等于/超过及组合extra顺序，双平台worker回归通过。真实样本对照重新绑定最终构建；这项属于主Agent后续修复，不宣称reviewer曾复核该改动。

P3生产隔离/核心授权/取消树级限额、两端宿主安装、三条旧profile真实样本与Deck实机不在本次独立审查实测范围，继续保持待验证。
