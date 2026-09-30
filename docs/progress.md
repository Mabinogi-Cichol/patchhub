# 开发进度

## 2026-10-01：P0 首批可运行基础

已实现：

- Cargo workspace：共享核心与 CLI；Rust 1.94，依赖锁定于 Cargo.lock。
- Windows Unicode 注册表 / 常见路径发现、Linux HOME 下候选路径；手动指定 Steam 库。
- 新旧 libraryfolders 与 appmanifest 解析、多库去重、相同 AppID 不同实例保留、损坏清单逐项诊断。
- 有界 KeyValues 解析、重复键拒绝、相对安装目录与越界检查、未完成安装保守处理。
- 严格开发目录模型、AppID 精确匹配、平台 / buildid 条件、补丁指纹状态与变更预览。
- JSON-lines 只读协议、大小限制与错误恢复；Python → Rust 桥接 PoC。
- Windows / Ubuntu CI 配置（尚未在远端触发）。

验证记录：

- Windows 本地 Rust 编译通过；Rust 17 项测试通过，包含更新标志、坏清单、旧库格式、AppID 冲突、文件指纹、库外 junction、配方路径、协议恢复与扫描到预览的闭环。
- cargo fmt 与 Clippy（warnings 视为错误）通过。
- Python 3 项桥接集成测试通过，使用真实编译核心。
- 独立审查发现的 common 指向库外问题已修复，并经 Windows junction 回归测试与定点复核确认。
- Windows 实际 Steam 只读扫描：发现 1 个根目录、28 条安装记录、0 条诊断。未将游戏清单 / 路径持久化到仓库；不代表已验证这些游戏的补丁。

没有完成：

- 真实补丁获取、哈希审核、AppID 核对、源许可确认；正式目录保持为空。
- 可信目录签名 / 撤销 / 到期验证与正式 JSON Schema 发布。
- ZIP 获取、安装事务 / 备份 / 回滚 / 崩溃恢复；此阶段没有修改游戏文件的接口。
- Windows Tauri UI、Decky 游戏模式插件、Linux 构建与 Steam Deck 实机验证。
- Windows registry 自动发现路径包含中文的专门实机验证；测试中的中文多库路径已通过。

已知限制：工具 / DLC / 运行时尚无类型元数据，kind 为 unknown；当前状态 installed 只说明目录存在且清单状态为完全安装，不等于校验了整份游戏内容。只接受已知干净状态；其他标志保守不允许预览为就绪。指纹读取不追踪链接，但还没有未来文件写入所需的句柄级抗竞态保障。

本轮不宣称 P0 完成：计划中的 3 个真实补丁与 1 款双端安装 / 回滚门槛尚未达到。

下一批按依赖推进：选择并审核真实官方补丁样本 → 目录签名与配方契约 → 本地 ZIP 预检 → 持久事务 / 备份 / 恢复测试 → 同配方双端实机验证 → Windows / Decky UI。

## 2026-10-01：安装核心与双端 Alpha 实现

- 首轮 Git 提交：6c8bf32；双端 Alpha 实现提交：2614041。
- 新增 Ed25519 签名、有效期、密钥 / release 撤销模型、SQLite revision 防回退；写事务内再次检查目录 revision。
- 受控 HTTPS ZIP 获取、大小 / 哈希 / HTML / 重定向检查；本地 ZIP 流式安全检查与暂存，备份、安装、卸载、持久恢复。
- 状态目录写入前扫描并拒绝与游戏目录重叠；卸载意图与状态一并持久化，兼容旧 committed + uninstall_requested 恢复窗口。
- Windows Tauri 原生宿主编译、Clippy 与前端构建通过；Decky TypeScript 与 Rollup 构建通过，插件目录打包成功。
- Windows 核心 / CLI 43 项、Linux 46 项测试通过，包含官方 ZIP 的受限 NTFS 时间戳兼容回归与签名 release 撤销执行检查；Python 3 项桥接测试通过。
- Windows 与 Steam Devkit 连接的 Steam Deck 均通过生产 CLI 的签名目录 → 计划 → 安装 → 托管检测 → 卸载 → 恢复隔离库测试；存档保留。
- 两端真实库匹配《夜勤病栋》AppID 3239270，build 16533492；Deck 上位于 SD 卡。
- 13 个 AppID 已入目录；12 个保持 discover_only，Windows 夜勤病栋配方在真实闭环完成后发布为 managed_zip，仅允许已验证 build。Deck 自动安装仍禁用。
- 官方 patch.moe 已经浏览器年龄确认和简体中文选择；沿用原有下载取得 NSN_R18PATCH_20241120.zip，未重新下载。包大小 4,081,525,083 字节，SHA-256 为 e646d4817f1b7bfebb01d37d9218a9ae18abca6c51c12379cf1fe3c770dfa0ef；审核 50 个新增文件及哈希，不包含可执行安装器。
- Windows 真实安装 committed、指纹检测 installed_managed；用户确认游戏能启动且补丁生效。卸载后 50 个补丁文件全部移除，12 个原版资源哈希一致；游戏运行修改的 systeminfo.json 被保留。
- Steam Deck 真实配方预检与 13 个原版文件基线读取完成。SD 卡读取受后台 Steam I/O 影响耗时约一小时，旧计划被 plan_expired 拒绝，未开始游戏文件写入；待确认 Steam 下载 / 更新已暂停后重新预检安装。真实安装 / 游戏运行 / 卸载验收未完成。
- 已生成最新 Windows NSIS 客户端安装包与便携 ZIP；NSIS 只安装管理工具，游戏补丁按可信配方直接解压文件。Decky ZIP 已实机安装，用户确认能够选择夜勤病栋；控制器导航与宿主重载仍待完整验收。
- Windows 与 Decky 前端具有取消下载按钮；只取消下载子进程，暂时不显示字节级进度。已新增本地数据与隐私说明。
- 已生成 726 个依赖组件的 SBOM 清单；npm 官方安全接口返回 0 条已披露漏洞。RustSec 全锁文件审计没有命中漏洞，但报告 glib 的 unsound 与 proc-macro-error 的 unmaintained 提示；依赖树确认两者均不进入当前 Windows 客户端或 Linux CLI 的构建依赖，完整锁文件的提示仍保留。

剩余验收：来源许可条件与发行依赖通知补齐、真实游戏 Deck 闭环，另外两个代表补丁样本、5 款双端运行矩阵；Windows 原生客户端完整流程、Decky 控制器与宿主重载。已生成发行测试包和签名方案文档，但未配置 Windows 代码签名证书。当前没有字节级 UI 下载进度；更新要求先安全卸载，不宣称版本迁移事务已实现。远端 CI / 商店审核未执行。本轮不能据此宣布开发计划全部完成。
