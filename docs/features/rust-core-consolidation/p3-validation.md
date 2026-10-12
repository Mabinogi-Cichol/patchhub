# P3 核心缓存与清理策略验证

日期：2026-10-12（Asia/Shanghai）。状态：P3 已实现，Windows/Linux 自动化与独立复核通过；实机待验证。

关联：[设计](design.md) · [施工记录](progress.md) · [P0 契约](p0-contract.md) · [P3 复核](p3-review.md)。

## 实际实现

- [CacheService](../../../crates/core/src/installer/cache.rs) 为唯一公共缓存删除入口，接受已验证宿主；inspect 只盘点 content/downloads/reports，不建状态、数据库或锁。cleanup 前置检查缓存与 tasks 根，逐级固定目录句柄，受限遍历100,000项/128层，累计仅确认删除字节；诊断至多512条，末项明确省略，独立失败计数保留真实状态。
- [CacheLock](../../../crates/core/src/installer/cache_lock.rs) 保持固定 cache.lock、schema不变；安装、恢复与丢弃持shared，清理持exclusive，冲突installer_busy。Linux额外flock原installer根目录inode，防仅替换cache.lock分裂锁；Windows禁止DELETE share。空恢复不建状态。句柄/路径身份复查与独立[安全gate](../../../crates/core/src/operation.rs)覆盖安装拷贝、哈希和写入检查，不能由任务lease gate覆盖；安全失效后恢复也停止写入。
- Linux删除使用openat/nofollow、私有隔离目录捕获、身份核验及unlinkat；Windowsdelete-by-handle，拒绝reparse与多链接文件。未知、stage/backup/auth/revoked以及jobs/receipts/runtime/数据库/sidecar/锁均保护。
- 遗留任务只回收精确journal终态/身份绑定、可信过期租约、已知schema和文件、空闲owner的UUID目录；持有owner直到最后删除。旧随机plan或缺失终态、活跃owner/lease、needs_recovery、额外材料全部保留；不凭目录mtime推断可删除。任务自身收尾也采用启动时捕获的安全目录句柄（Windows固定parent并在DELETE重开前后核对原volume/file index），移除原remove_dir_all(self.job)，仅针对自己拥有的瞬态目录，保持非致命收尾；全量遗留回收仍只在cache独占边界。
- [service](../../../crates/core/src/installer/service.rs) 的AfterCommit仅本次新committed触发：三路线内部返回瞬态replayed元数据，未增加持久schema；先释放shared，再尝试exclusive。Busy/Partial/Failed单独记录cleanup，不能改真实commit/cause/disposition；重放与非commit不再次清理。
- [CLI](../../../crates/cli/src/runtime.rs) 删除私有遍历/递归删除，传实际policy到核心；[Decky](../../../apps/decky/main.py) 在后台任务开始前快照保存的auto_cleanup，传disabled/after_commit，移除追加cleanup RPC与终态改写。[显示](../../../apps/decky/src/cache.ts)区分五种清理状态；[Windows](../../../apps/windows/src-tauri/src/main.rs)显式disabled，不新增用户开关。

## 自动化证据

| 验证 | 实际结果 | 证据 |
| --- | --- | --- |
| Windows core/CLI | 487 passed、6既有ignored、0失败 | [日志](../../../.artifacts/rust-core-consolidation/p3/windows-tests.log) |
| 非root WSL Ubuntu core/CLI | 494 passed、6既有ignored、0失败 | [日志](../../../.artifacts/rust-core-consolidation/p3/linux-tests.log) |
| Windows Tauri宿主 | 23 passed | [日志](../../../.artifacts/rust-core-consolidation/p3/windows-host-tests.log) |
| Decky Python backend | 71项：69通过、2既有skipped | [日志](../../../.artifacts/rust-core-consolidation/p3/decky-python-tests.log) |
| Decky TS/类型 | 63 passed、typecheck通过 | [测试](../../../.artifacts/rust-core-consolidation/p3/decky-ui-tests.log) · [类型](../../../.artifacts/rust-core-consolidation/p3/decky-typecheck.log) |
| Rust格式/lint | 全workspace fmt --check、Windows core/CLI all-targets Clippy -D warnings通过 | [Clippy](../../../.artifacts/rust-core-consolidation/p3/windows-clippy.log) |
| 实际Linux root | 可只读inspect、cleanup拒绝写入且不建state，1项通过 | [日志](../../../.artifacts/rust-core-consolidation/p3/linux-root-negative.log) |
| 独立安全复核 | 发现修正后定向复核findings[] | [复核](p3-review.md) |

[机器汇总](../../../.artifacts/rust-core-consolidation/p3/validation-summary.json)、[最终文档/差异检查](../../../.artifacts/rust-core-consolidation/p3/final-checks.json)、[最终源码SHA-256](../../../.artifacts/rust-core-consolidation/p3/tested-source-sha256.json)。最后新增安全收尾/Windows共享修正后重新执行完整Windows/Linux，未沿用失败的中间结果。P0/P1性能基准不因缓存提取重复运行。

Windows命令：scripts/cargo.ps1 test --offline -j 2 -p steam-patch-core -p steam-patch-cli；Windows宿主另test -p steam-patch-windows。Linux使用既有.tools/linux-cargo、.tools/linux-rustup、.local/p1-linux-target执行相同离线测试。Decky python -m unittest discover -s apps/decky/backend -p test_*.py，npm test / npm run typecheck -w apps/decky。

- 核心临时fixture：只读缺失状态、完整保护allowlist、硬链接与部分删除真实bytes、symlink/junction、恢复材料、可信终态task/活跃owner/lease/needs_recovery、512诊断截断、大小溢出、任务父目录替换。
- 跨进程shared/shared与所有writer冲突、RAII释放、锁身份/硬链接/root替换、部分store初始化保留锁。
- [公共signed API](../../../crates/core/tests/signed_install.rs)：一次提交清理、重放保留新增缓存、其他writer Busy、Partial/前置拒绝不改commit、回滚不清理、独占阻断安装/恢复/丢弃。
- [真实CLI子进程](../../../crates/cli/tests/cache_smoke.rs)：v3 serve-task after_commit→原幂等ID重放→serve手动清理，保留控制stdin直到终态；覆盖冻结前宿主策略传入，不将单位测试当真实进程证据。

## 边界与待验

生产协议仍为开发期v3，P5切换严格v4与生成契约；本轮不部署混合产物。空inspect不做锁/初始化，返回的是受限缓存树的读取快照，tasks不计入缓存库存。保守保留未知task可留下旧材料，需要后续有确切证明才回收。

Unix advisory flock不能完全阻止同UID非合作进程任意改写整个状态祖先；本轮用句柄锚定删除、目录+文件双锁、身份重核和失效后停止写入覆盖已检查边界，不宣称为同UID任意恶意修改提供完整隔离。没有接触实际用户库/游戏或设备；release最终产物、双端实机与最终验收留待P6。

后续提交记录（2026-10-12）：P3/P4 代码已本地提交 `19ffc8a`，文档独立提交。提交时沿用 [P4 最终矩阵](p4-validation.md)，199 个源码指纹无漂移；未部署。
