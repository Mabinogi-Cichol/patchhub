# P4 actions 与双端消费验证

设计：[design.md](design.md)。施工：[progress.md](progress.md)。审阅：[p4-review.md](p4-review.md)。

2026-10-12：P4 实现完成，最终自动化矩阵通过。实施验证期间保留 P3 未提交修改；该阶段没有提交或部署。后续按用户授权，P3/P4 代码已本地提交 `19ffc8a`，文档独立提交；199 个源码指纹无漂移，沿用最终矩阵，未部署。正式 protocol v4、生成契约与共同展示 helper 的剩余整合留待 P5，最终包/性能/实机与全范围独立 gate 留待 P6。

## 实际实现

- 核心必需 PatchActions：automatic/local_archive/reinstall/overwrite 各带 available 和封闭 ActionBlockerCode/detail，primary 带恒 overwrite operation、install/overwrite 标签、automatic/local_archive/unavailable 输入。删除 PatchMatch.download_available 与两个前端派生权限字段，无旧字段回退。
- 动作使用同一 projection 的信任、来源/订阅、selection resolver、实例与取证、收据、ZIP/EXE 本机 link。未知拒绝原因不会被吞掉，保留原 code/detail 并保守映射 source_requires_guidance。理由按规则稳定去重，available 等价 blockers 为空。
- signed 本地 ZIP 可绕过缺少远程 artifact；信任/时效/撤销、recipe/hash、平台/build、真实链路与身份要求仍保留。动态候选的原始镜像/下载事实未移除。EXE 发起检查仍不等于包授权。
- 严格 Reinstall 必须当前实例、相同内容、完整 owned committed receipt 和全部当前文件哈希匹配，prepare/apply 均重核；外装或损坏内容由明确 Overwrite 修复。needs_recovery 使当前实例四种写动作都 unavailable，包括另一个 patch；实际执行仍持 journal 互斥拒绝。
- 当前 committed 收据 plan 不可读仍是 overwrite 标签证据，但不授予 Reinstall。unknown 本身不禁用成熟安全 overwrite；可靠文件证据与路径风险分别处理。实例路径验证复用 detection 的 no-follow/句柄检查，不增加假文件哈希或复制边界规则。
- Windows/Decky 只读取 actions 决定主按钮标签/输入/操作/可用性；自动优先，本地导入有明确执行入口与路径要求。当前实例/选中/busy 保留宿主交互约束，执行入口重新核对现有动作。只读预览不随写入禁用消失，理由只展示当前提供动作的阻断，不把严格重装不可用显示为自动初装警告。

## 最终检查

| 检查 | 命令/结果 | 日志 |
| --- | --- | --- |
| Windows core/CLI | scripts/cargo.ps1 test --offline -j 2 -p steam-patch-core -p steam-patch-cli；502 passed，6 ignored，0 failed | [日志](../../../.artifacts/rust-core-consolidation/p4/windows-core-cli-tests.log) |
| Ubuntu WSL 非 root core/CLI | 同参数，独立 Linux toolchain/target；509 passed，6 ignored，0 failed | [日志](../../../.artifacts/rust-core-consolidation/p4/linux-core-cli-tests.log) |
| Windows Tauri | scripts/cargo.ps1 test --offline -j 2 -p steam-patch-windows；24 passed | [日志](../../../.artifacts/rust-core-consolidation/p4/windows-tauri-tests.log) |
| Decky Python | python -m unittest discover -s apps/decky/backend -p 'test_*.py'；73 run，71 passed，2 skipped | [日志](../../../.artifacts/rust-core-consolidation/p4/decky-python-tests.log) |
| Decky TS | npm test --prefix apps/decky；65 passed | [日志](../../../.artifacts/rust-core-consolidation/p4/decky-ts-tests.log) |
| Windows UI | npm run test:status --prefix apps/windows；10 状态用例和 13 动作断言 | [日志](../../../.artifacts/rust-core-consolidation/p4/windows-ui-tests.log) |
| 两端类型 | npm run typecheck --prefix apps/windows、apps/decky；均通过 | [Windows](../../../.artifacts/rust-core-consolidation/p4/windows-typecheck.log)、[Decky](../../../.artifacts/rust-core-consolidation/p4/decky-typecheck.log) |
| Rust 格式/Clippy | fmt --all -- --check；clippy --offline -j 2 -p steam-patch-core -p steam-patch-cli --all-targets -- -D warnings；均通过 | [fmt](../../../.artifacts/rust-core-consolidation/p4/fmt-check.json)、[Clippy](../../../.artifacts/rust-core-consolidation/p4/windows-clippy.log) |

[汇总](../../../.artifacts/rust-core-consolidation/p4/validation-summary.json)，[最终源码指纹](../../../.artifacts/rust-core-consolidation/p4/tested-source-sha256.json)，[后续 Python 变更重验](../../../.artifacts/rust-core-consolidation/p4/source-revalidation.json)。各 6 ignored 是既有需真实网络/特定资源的测试，不冒充通过。Python 两项需要本地 Linux release binary，当前 Windows 未提供，保持 skipped。

## 行为证据与修复历史

- actions 内部 32 组合真值表、必需字段、封闭理由/旧别名拒绝、UTF-8 detail<=2048 bytes；投影覆盖 signed 本地/自动优先、签名/撤销/过期、来源、平台/build/游戏、不同实例的 recovery、ZIP/EXE 链路、URL/镜像准入、unknown 与完整/损坏/改内容/不可读/历史收据。
- 四项 signed 公共 API 回归先读取 projection actions 再改变信任/build/live files/事务，确认实际 service 拒绝旧动作；完整外装也不能凭匹配文件走 Reinstall。动态 CLI 扫描后停用来源/退订使旧 selection 从当前候选集消失，执行返回 selection_not_found，不写游戏或建立事务；重新扫描无可用动作。
- 实际 JSONL 子进程验证必需 actions、旧字段不存在、同 AppID 跨库实例身份、unsigned 禁止写入和只读 preview。双端 helper 测试执行 UI 实际调用函数，而非仅重写一份测试判定。
- 初次旧 CLI 测试仍期待损坏文件可 Reinstall，已改为完整同内容 Reinstall 成功、损坏 Reinstall 拒绝、明确 Overwrite 修复。新增动态测试最初混入安装专用 scan 参数、且期待来源码而非 resolver 当前 selection_not_found，按正式路由修正后 Windows/Linux 全量通过。未修改产品逻辑来迎合错误断言。
- 独立审阅两项 P2 已修复，范围和未收到的最终 reviewer 报告见 [审阅记录](p4-review.md)。主 Agent 最终检查已收到问题的修复与测试闭环；不声明 P6 全范围独立验收完成。

## 实际 DTO 体积与限额

实际 Rust projection（非 P0 手工 wire 样本）以真实 PatchActions、2048-byte link detail、500 个合成实例输出 compact v3 JSONL，含 LF：每实例 1 patch 为 4,787,585 bytes，每实例 4 patch 为 18,533,585 bytes。第二种超出固定 8,388,608-byte 上限；不裁动作、不截断 JSON、不增大限额。详见 [实测](../../../.artifacts/rust-core-consolidation/p4/wire-size.json)。体积 fixture 共用临时安装路径，仅验证真实 DTO 扩展规模/序列化，不冒充真实 500 游戏安装库。

现有 Tauri bounded reader 验证等于上限可读、多一个字节拒绝；Decky 同时验证 fake line 超限与真实 asyncio StreamReader 限额错误统一返回 response_too_large，后续合法调用仍成功。CLI 生产者的 v4 有界序列化/同进程下一请求恢复和安装终态可输出保证按 P0 明确留待 P5，本轮没有把宿主拒绝写成生产者已闭环。

## 未执行范围

正式 v4 生成/混装拒绝、最终客户端 bundle/sidecar/SBOM/哈希、性能基准复跑、Windows/Steam Deck 真实游戏操作与部署、控制器和最终全范围独立 gate 均待 P5/P6。本轮只操作临时 fixture，没有改持久 schema 或补写历史 outcome。

实际创建四名子 Agent：核心 specialist 与独立 reviewer 使用 GPT-6.1 Sol/high，Windows/Decky worker 使用 GPT-6 Luna/medium，均 fork_turns=none；主 Agent 负责共享类型、CLI、执行边界修复、集成验证与文档。
