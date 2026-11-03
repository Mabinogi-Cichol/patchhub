# 施工记录

设计见 [design.md](design.md)，实现审阅见 [review.md](review.md)。

## 状态

已实现。2026-11-03 本地 Rust、Flutter、Decky 前后端及双端打包资源检查通过；最终 Windows 发行包重建、资源/二进制/schema 核对，以及从真实便携包提取核心后的干净应用全链路实测均通过。Linux ELF 发行构建、Steam Deck 实机、真实来源网站/游戏内效果及原生 UI 交互待验证。本轮按代码与文档分开创建本地提交；未推送、合并或发布。

## 2026-11-03：调查与实现

- 任务开始时实际为干净 `main`；用户明确授权后切至 `fix/remove-static-catalog`。未改动 `crates/core/tests/script_source_import.rs` 或 `docs/features/patchhub-ci-fix`，未暂存任何文件。
- 核实 `3ac65f44ebfa2176716a645608c803689605cd78` 的 signed-catalog payload：generated_at=1790974154（北京时间 2026-10-03 04:49:14），expires_at=1793566154（2026-11-02 04:49:14）。本任务不重签、不续期，也不把它与独立的并发导入测试故障合并处理。
- 删除 `catalog/` 公开目录、公钥及说明、公开目录 schema、CLI 签名维护命令、静态验签安装与 trust revision 模块，以及只验证旧路线的测试。保留本地安装计划和历史事务所需数据结构、原有反序列化字段及恢复执行代码。
- Windows、Decky、CLI 的 host 统一为 roots/state_dir/platform。预览只接收动态 selection，安装接收 selection 或既有 manual_download；release/archive 及静态 local_archive action 删除。结果和历史事务的 release_id 保留为身份。
- 核心投影只读取 Steam、来源快照和本地事务；不再返回 catalog_error/catalog_trust/catalog_revision。删除 Flutter 目录不可用提示对应的失效输入路径。能力契约同步为 projection=3、actions=2、outcome=1、request_validation=3；生成 Rust/TS/Python/Flutter 共享 schema 与夹具，明确拒绝旧目录字段。
- 删除两端打包静态资源要求。Windows 增量构建会清理三个旧目录资源与旧文件选择器 DLL；Decky 原有重新暂存流程不会复制它们。移除只服务旧静态 ZIP 路线的文件选择器 UI、依赖、锁定的 12 个相关包及原生插件注册。
- 原静态签名投影基准和签名 ZIP 原生夹具生成器退役；当前指南、共享协议及受影响的历史链接同步处理，历史实测事实保留。
- 依据 overengineering-review 技能完成计划与实际执行路径审阅：未增加宽限期、空目录兜底、双读、状态迁移、重试或硬限制。详见 [review.md](review.md)。

## 验证

日志和摘要位于忽略目录 `.artifacts/remove-static-catalog/`。测试均使用隔离临时状态和真实当前系统日期，未修改用户游戏。

| 检查 | 结果 |
| --- | --- |
| Rust `cargo test --workspace` | 681 通过、10 个原有忽略；包含干净应用→扫描 Steam 清单→执行来源 Lua→更新索引→动态匹配/选择→本机 HTTP 下载→安装→幂等回放，以及历史无 selection 的 committed plan 投影 |
| Decky Python 后端 | 最终 124 项，2 项跳过（开发者模式开启后符号链接测试可执行）；实际核心桥接、参数注入、取消、回执和恢复相关回归通过 |
| Decky TypeScript | 类型检查、107 项测试、Rollup 构建通过 |
| 共享契约 | 生成一致性、跨语言请求/响应夹具、Flutter presentation 夹具一致性通过；旧 host/release/archive 输入拒绝得到回归覆盖 |
| Flutter | analyze 无问题；292 通过、1 项原有跳过；Windows Release 构建通过。最终契约清理后的同入口重建亦通过 |
| 双端真实打包入口夹具 | 12 项版本/入口测试通过；Windows 无 catalog 源目录且预放旧资源，构建脚本清除旧文件；Decky 无 catalog 源目录仍正常暂存，bin 仅 core/worker/runtime.json |
| 发行包与安装脚本检查 | package-artifacts 11 项通过；Decky 安装脚本 32 项中 25 项平台相关跳过；CI 路由 9 项通过 |
| Windows 实际产物 | 核心/worker release 编译、便携包构建通过；43 个文件，无 catalog.json/signed-catalog.json/trust-keys.json/文件选择器 DLL，ZIP 中核心/worker 与本轮 release 二进制逐字节一致，内嵌 schema 与生成物一致。最终包已重建并再次完成同样核对 |
| Windows 真实便携包全链路 | 从最终 ZIP 取 core/worker 到空应用目录；以当前日期在临时 Steam 库完成扫描→来源 Lua 更新/查询→动态候选匹配/选择→本机 HTTP 下载→真实文件安装→同请求幂等回放；HTTP 请求数为 1，未使用公开目录。证据 `.artifacts/remove-static-catalog/package-smoke-result.json` |

### 环境信号与处理

- 沙箱内 Windows 文件锁导致临时来源测试统一 `source_state_busy`；同一测试在正常权限下通过。后续真实锁/子进程测试使用正常权限运行，没有修改生产锁策略。
- 批量源码改写与提权测试合并执行曾被自动审批拒绝；改为逐文件可审阅补丁、源码修改与验证分离后完成，没有覆盖主会话工作。
- 最终包附加实测一度因自动审批模型容量不足/审阅超时未执行；受限沙箱中已通过扫描、脚本查询与动态匹配，但本机 HTTP 被网络边界阻止。用户要求继续等待重试，审批恢复后正常权限执行同一夹具成功，未改变生产逻辑或绕过网络边界。
- 用户追加要求安装环境后，从 Flutter 官方 Git `3.47.7` 标签 `abaf9c523780a608bd46686fd5e53740a07077f8` 安装到 `.tools/flutter`。官方归档清单/下载地址返回 404，因此使用已核实的官方标签初始化。最终工具识别 stable / Flutter 3.47.7 / Dart 3.13.5。
- 用户明确同意后通过 UAC 开启 Windows 开发者模式；用户 PATH 加入 `.tools/flutter/bin`。Windows C++、CMake、Windows SDK 探测及实际编译通过。Android 环境不属于本任务范围。

## 未验证与交付边界

- 没有 Linux 发行 ELF 工具链/产物，本轮只验证 Decky 前后端、前端构建和真实打包入口/资源夹具，不声称完整 Linux 发行 ZIP 已生成。
- 未操作 Steam Deck，未执行真实发行商下载、真实游戏安装或游戏内生效测试，未进行原生 UI 交互/视觉验收。
- 已按用户要求分开本地提交代码与文档；代码提交为 `e75ca87`（`refactor(catalog): 移除公开静态补丁目录及客户端依赖`）。未推送、发布或合并，提交保留在 `fix/remove-static-catalog` 分支。
