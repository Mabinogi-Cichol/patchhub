# 过度设计审阅

对象：`fix/remove-static-catalog` 相对任务开始时干净 `main` 的本地改动；设计见 [design.md](design.md)，验证状态见 [progress.md](progress.md)。

结论：未发现需要增加兼容层、宽限期、重签或兜底的依据；当前清理沿真实执行路径完成，未扩大安装权限。

- [宿主与请求](../../../crates/protocol/src/lib.rs)：只接受 roots/state_dir/platform，删除目录路径与 release/archive 安装输入。旧字段由现有严格 schema 拒绝；跨语言共享夹具覆盖这些拒绝。能力版本区分新旧核心，未增加双读。
- [投影](../../../crates/core/src/projection.rs) 与 [CLI](../../../crates/cli/src/runtime.rs)：完全从 Steam 快照、本地来源快照与事务投影，不构造空公开目录，不捕获目录过期后伪装正常结果；删除了产生该错误的依赖。
- [安装服务](../../../crates/core/src/installer/service.rs)：删除仅静态路线使用的 catalog/revision_store/clock 注入和 trust_state_mismatch，动态来源在真实写边界重新解析选择、来源设置与游戏事实。下载摘要、文件路径检查、worker 与 request ledger 执行代码保持原有语义。
- [历史事务](../../../crates/core/src/install.rs)：本次不改变存储格式和恢复逻辑，catalog_revision 等原有字段保留，未新增迁移、恢复状态或假想兼容层。历史无 selection 的 committed plan 通过实际 journal 夹具继续投影。
- 打包只移除不再需要的资源与原生 ZIP 选择器；Windows 对旧输出的三个目录资源及旧 DLL 做明确文件清理，因为增量构建确实复用该输出目录。没有增加任意数量、大小或耗时限制。

本结论覆盖静态调用路径与施工记录中实际执行的测试，不代表 Steam Deck 实机、真实发行商下载或游戏内效果验收。
