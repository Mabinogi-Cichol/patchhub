# 移除公开静态补丁目录

施工状态见 [progress.md](progress.md)。

## 目标和范围

程序仅内置来源规则及解析脚本。用户扫描本机 Steam 游戏，更新来源并动态匹配候选，选择后安装。删除随程序发布的逐游戏目录、公开目录验签/有效期准入、宿主参数、静态投影和安装入口，避免公开目录到期阻断正常客户端。Windows、Decky、CLI、协议生成物和打包检查同步修改。

## 已核实的执行路径

- Windows CorePaths 和 Decky CoreBridge 固定注入公开目录文件；CLI scan 在动态投影前验证 signed_catalog，错误覆盖 scan.catalog_error，Flutter 展示目录不可用。
- 动态 ZIP/EXE 和手动选择路径从本地 SourceSnapshot/SourceRegistry 解析选择，并在安装服务重新读取源、实例及游戏事实，无需公开目录。
- installer/service.rs 的 catalog/revision_store 是静态路线依赖；动态路线仍被无业务用途的 trust_state_mismatch 前置检查牵连，应一并删除。
- list_transactions、recover、discard_plan 与 uninstall 使用本地 journal、plan、文件哈希和备份，不加载公开目录。保留历史 plan/transaction 字段及反序列化契约，不迁移或重写用户状态。
- 当前直接安装路线已采用本地计划和 request ledger；不因本任务删除其授权边界、哈希、文件路径约束、游戏事实检查、幂等控制、事务/收据、卸载或恢复，也不另造签名机制。

## 最小方案

1. 删除 catalog/ 公开资源及生产构建复制/必需文件检查；保留内嵌来源规则与 Lua 脚本。
2. HostInputs 只保留 roots/state_dir/platform。能力版本更新为 projection=3、actions=2、request_validation=3，outcome=1 不变。公开安装与预览只支持 selection_id 或既有 manual_download。删除旧 release_id/archive 的公开入口，不把 release_id 从历史收据中删除。
3. projection 仅投影动态来源，去除 catalog 信任/错误/版本字段及静态选择优先级。删除静态验签安装模块、维护命令及其专属测试。保留共享 catalog 模块中仍被本地安装计划使用的数据结构与验证。
4. 同步客户端、生成契约、夹具、基准/验收脚本和当前指南。历史施工记录保留实际发生事实，并注明后续替代，避免篡改历史证据。
5. 回归覆盖无目录干净扫描→来源脚本查询→匹配→动态选择，动态安装安全检查及事务回放/恢复，以及双端实际打包入口在无 catalog 目录下的资源检查。

## 验证和验收

执行相关 Rust、协议生成一致性、Decky Python/TypeScript、Flutter 静态检查/测试和资源打包回归。能够本地构建的平台执行构建；环境不可用明确记录，不能以窗口启动替代打包验证。采用真实当前日期，不冻结时钟、不延长有效期。不提交、推送、发布、操作 Steam Deck。

## 过度设计审阅

计划审阅：移除公开目录及仅为其服务的入口；不引入空目录兜底、时间宽限、兼容双读、迁移或新状态机。历史事务按已存储契约原样读取。实现后沿实际动态安装和恢复路径复核。

原签名目录投影基准和签名 ZIP 原生夹具生成器随旧路线退役；历史结果保留。原生验收入口接受已完成动态来源匹配的隔离 spec（manual_download），本轮不执行实机 UI 验收。
