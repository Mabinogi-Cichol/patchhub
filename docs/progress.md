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
