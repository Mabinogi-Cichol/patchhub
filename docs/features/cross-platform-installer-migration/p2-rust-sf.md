# P2 Rust SF 解析与固定 worker

设计：[design.md](design.md)。施工记录：[progress.md](progress.md)。基线：[P0 worker 契约](p0-contracts.md#6-worker-schema--等价内存限额)。

## 范围

本阶段实现 `installer-format` 中的 SF9.5–9.7 PE/resource/overlay/证书 framing、受限 LZMA2、完整 CArchive 对象图、Lua AST 与静态行为 IR；`installer-worker` 新增固定 `sf-worker` 二进制，支持 `analyze`、`analyze_behavior` 与 `convert_behavior`。

worker 始终不提供安装授权。分析报告保持 `installable:false`，核心授权、平台生产启动器、facts/空间与任务接线属于 P3；Windows/Decky 入口分别属于 P4/P5。Python 工具仅用于开发黄金对照，当前生产宿主切换和删除 Python SF 属于 P5/P6。

## 实现边界与契约

- 格式库无宿主进程、网络、游戏目录或脚本执行依赖。类引用按顺序解析，未知类/字段/选项与不完整覆盖阻断；不按包 SHA、AppID 或脚本摘要准入。
- 严格描述符拒绝额外键、路径、命令、旧 `semantics_approved` 与未知 action。SHA-256、大小、元数据、行为和完整载荷清单均在首次产物写入前核验。
- Linux 固定输入 `/input/archive.exe`、输出 `/output/normalized.zip`；Windows 使用宿主固定的隔离工作目录下 `input/archive.exe` 与 `output/normalized.zip`。不接受路径参数。宿主须保证输入在整个 worker 调用期间不可变。
- 只读 mmap 避免将 2 GiB 包完整复制入堆。Windows 打开输入禁止写入/删除共享；两端均重核描述符、文件身份与内容。生产沙箱的文件不可变保证在 P3 复核。
- P2采用受限ZIP_STORED序列化器，以精确控制冻结的CPython 3.12 ZIP64 local/central header；无新增ZIP写库。流式转换为 ZIP_STORED，固定时间、Unix 权限、UTF-8 标志与 ZIP64 local header。第二次载荷解码再次核对 CRC/长度/摘要，匹配CPython 3.12 `force_zip64=True`冻结输出。文件大小、条目偏移或central offset超过2^31−1时，同步生成central ZIP64 extra及ZIP64 EOCD/locator，不能误用ZIP字段的2^32−1作为Python阈值。
- `normalized.zip.partial` 以 create_new 独占创建；同卷 hard-link 原子无覆盖发布，再删除自身 partial，避免 Unix rename 覆盖迟到输出。既有 final/partial 不删除。文件系统不支持 hard-link 则失败，不降级为覆盖写入。
- 完整 worker 响应含换行 ≤16 MiB；转换前检查响应大小后再发布。EOF/取消/进程树资源限制由 P3 平台启动器保障，不将本阶段单测等同于生产隔离验收。

## 限额与保守收紧

沿用包/单文件/展开 2 GiB、8192 文件、元数据/报告16 MiB；规范化产物限额2 GiB+16 MiB（含ZIP64结束记录）。另有LZMA2字典128 MiB、Lua 1 MiB/100000 token/96层、分析200000步/函数调用32层、1024分支与条件8192节点。

相对旧 Python 新增的保守限制：XML 深度96、诊断库存未解析引用1024条、AST最终树深度96（含循环生成的链）、展开后的条件IR8192节点/96层、聚合全局脚本16 MiB、累计语义分配64 MiB、表DAG8192节点/96层及仅允许标量UI属性更新。超限均阻断，不能用沙箱 OOM 代替可预测的解析拒绝。不可变字符串与条件 DAG 共享，表采用受控复制，避免短输入经别名/分支造成无界内存放大。

GB18030 展示文本保留旧 Python 编码表语义：18个新版映射差异、A3A0/A8BC 与四字节映射由固定测试覆盖；非法字节不替换接受。来源 ID 编码不涉及这张展示文本编码表，也不恢复 Python 来源哈希兼容。

## 依赖与许可

| 依赖 | 用途 | 许可/处理 |
| --- | --- | --- |
| lzma-rust2 0.21.0 | raw LZMA2流式解码 | P0冻结，Apache-2.0，关闭encoder/xz/optimization等默认功能，只启用std |
| quick-xml 0.38.4 | PE manifest资源的有界XML解析 | MIT；相对P0自研PE方案的补充，PE/framing仍自研，避免自写XML语法。拒绝非法实体/重复扩展属性/命名空间错误 |
| memmap2 0.9.11 | worker只读包映射 | MIT OR Apache-2.0；直接调用平台映射API，Unix依赖libc，无Python/C解压运行时 |

锁文件固定版本，Cargo metadata/SBOM自动收录；保留 [sfextract MIT布局参考许可](../../../scripts/licenses/sfextract-MIT.txt) 与原commit `21f5c5595a70e9284fff83476c0e307d5b92f573`。便携包包含worker/许可的生产接线仍属P3/P4/P6。

## 黄金与复现

[freeze_sf_rust_fixtures.py](../../../scripts/freeze_sf_rust_fixtures.py) 在CPython 3.12下从现有35项合成Python测试采集并重新计算确定性输入，生成 [fixtures/sf](../../../crates/installer-format/tests/fixtures/sf/)。不嵌入真实发布方二进制或脚本；测试PE的runtime只含惰性文本。Rust测试独立读取JSON，无Python运行时依赖。 [freeze_sf_zip64_fixture.py](../../../scripts/freeze_sf_zip64_fixture.py)另冻结9组central/EOCD边界，仅捕获逻辑偏移和几十至几百字节header，不分配或写入2GiB载荷。

[verify_sf_rust_samples.py](../../../scripts/verify_sf_rust_samples.py) 使用 [隔离对照example](../../../crates/installer-worker/examples/sf_sandbox_check.rs) 调用固定Rust worker。runner仅授权新建的 `.artifacts/p2/<case>`，Windows LPAC零capability+Job/干净环境，Linux bwrap离线+只读输入+独占输出。真实字节只有worker在沙箱中解析；宿主只复制与流式哈希。既有Python报告先核对原EXE摘要/大小，再比对完整报告、行为摘要及规范化ZIP摘要。历史规范化ZIP存在旧header编码差异，先按冻结载荷清单/摘要检查其文件，再用冻结的CPython 3.12 ZipInfo/force_zip64规则重建ZIP基准。开发oracle显式固定local header版本45与0xffffffff大小占位、central版本45，消除Python 3.10保留版本20/实际大小的差异；六个合成黄金在Python 3.10与3.12下均逐字节一致。不把历史重打包的整个ZIP摘要当作当前converter黄金。`--reuse-run`只复用同worker摘要/同描述符且退出成功的本次结果。

```powershell
./scripts/cargo.ps1 test --locked -p steam-patch-installer-format -p steam-patch-installer-worker
& ./scripts/cargo.ps1 @('clippy', '--locked', '-p', 'steam-patch-installer-format', '-p', 'steam-patch-installer-worker', '--all-targets', '--', '-D', 'warnings')
./scripts/cargo.ps1 build --locked --release -p steam-patch-installer-worker --bin sf-worker --example sf_sandbox_check
python scripts/verify_sf_rust_samples.py --synthetic-only
python scripts/verify_sf_rust_samples.py
```

Linux开发验证用仓库 `.tools/linux-cargo`/`.tools/linux-rustup` 与 `target/linux`，随后 `python3 scripts/verify_sf_rust_samples.py --platform linux`。WSL仅为开发对照环境，不是Windows产品运行时依赖。

## 验收状态

实际结果、独立审查修复与证据摘要见 [施工记录](progress.md#2026-10-08-p2-rust-sf与固定worker)。三条旧profile样本在当前仓库无二进制，仅以合成fixture覆盖通用语义；真实旧样本回归保持待验证。独立审查见 [p2-gate-review.md](p2-gate-review.md)。不得将本阶段转换成功写成真实游戏安装、生效、P3生产隔离或Deck实机验证。
