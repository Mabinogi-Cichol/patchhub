# P0 投影性能与固定输入基线

设计：[design.md](design.md)。施工记录：[progress.md](progress.md)。工具：[benchmark-projection.py](../../../scripts/benchmark-projection.py)。日期：2026-10-11（Asia/Shanghai）。

本次实测当前 v3 调用链；不代表 P1 核心 API 已实现或 v4 已验证。测量对象是 Windows debug CLI 的进程启动、受限 JSONL serve、完整 scan 与 JSON 序列化，总耗时不等于投影函数单独耗时。每个场景 6 次独立进程，第 1 次单列，后 5 次记录中位数/最大值。fixture 创建已预热文件系统，未测冷缓存；不同磁盘、release 构建和后台负载不可直接与此数值比较。

## 实际结果

| 场景 | 实例数 | 首轮秒 | 后 5 次中位秒 | 后 5 次最大秒 | 取证尝试/完成哈希/读取字节 | v3 响应字节（含 LF） |
| --- | --- | --- | --- | --- | --- | --- |
| manifest-only | 500 | 1.761667 | 0.324238 | 0.327899 | 0/0/0 | 397,332 |
| recipe-all-match | 24 | 1.268764 | 0.668940 | 0.682324 | 96/96/25165824 | 37,799 |
| recipe-mixed-evidence | 24 | 0.741165 | 0.373439 | 0.389414 | 96/48/12582912 | 35,524 |
| real-manifest-only | 36 | 0.054778 | 0.044995 | 0.047225 | 未计数；无 catalog/installer state | 17,192 |

完整原始结果：[p0-projection-baseline.json](../../../.artifacts/rust-core-consolidation/p0-projection-baseline.json)。归一化结果快照：[p0-projection-baseline-snapshots.json](../../../.artifacts/rust-core-consolidation/p0-projection-baseline-snapshots.json)。这些是本地忽略的实测证据；本表、工具和确定性 fixture 定义纳入仓库，其他 checkout 需按命令重跑。结果记录生产源码、Cargo 配置/lock 和内嵌资源 SHA-256、plain/counted binary SHA-256、每轮时间及响应体积，不以已有 target binary 的修改时间替代源码基线。

## 固定 fixture 与哈希口径

- 两个 Unicode 路径库；每个 AppID 在两库各有一个实例。manifest-only 为 500 实例/250 AppID；recipe 场景为 24 实例/12 AppID。
- recipe 每实例 4 个 256 KiB 文件，内容为 `bytes(range(256))*1024`；检测 hint 重复 recipe 的第一个 destination，验证不会多哈希一次。
- 全匹配：24×4=96 次取证与完成哈希，25,165,824 字节；检测 installed_external（unsigned 开发目录，无托管收据）。
- 混合：每实例依次 match/mismatch/missing/unreadable_or_unsafe（directory），96 次尝试、48 次完成、12,582,912 字节；无法取证不得冒充匹配，整体 detection=unknown。
- 计数仅覆盖当前 `catalog::hash_file` 和 CLI `hash_target` 的实际调用、完成次数、哈希读取字节。原子计数注入 `.local` 的一次性源码副本，计数版和未计数版响应逐字段完全相等。正式源码 SHA-256 在运行前后相等，正式 target/runtime binary 未改。
- 当前固定 fixture 不含来源候选或动态 committed plan，dynamic 计数实际为 0；不据此声称动态投影没有开销。catalog 匹配/缺失/未知、多库隔离与 recipe 去重已实测，来源/signed/收据/preview 行为基线由现有测试定位与 [真值表](p0-behavior.md) 冻结，需在 P1 增加相应 core fixture 的计数比较。

## 真实实例可得性

本次通过注册表/VDF 核对后显式传入 `D:\Program Files (x86)\Steam`；完整清单扫描读取 Steam manifests/appinfo，36 个实例，无诊断。没有传入 state_dir/catalog/signed_catalog，不打开 installer/sources/trust DB，不写游戏、宿主设置或任务历史，不联网。此场景只确认清单与实例可得，不验证补丁 payload/收据或安装行为。

| AppID | 库 | availability | buildid |
| --- | --- | --- | --- |
| 2303800 | F:\SteamLibrary | installed | 10821988 |
| 2743810 | F:\SteamLibrary | installed | 23866175 |
| 3137110 | F:\SteamLibrary | installed | 16717597 |

Deck 上的 ZIP/EXE/大包/写入取消仍仅引用 [历史基线](p0-data-debt.md)，本轮未连接设备，也没有重新确认远端状态或下载缓存。大包在旧安装后自动清理，其现存原始包不能视为已确认。

## 重跑与 P1 对照门槛

```powershell
python scripts/benchmark-projection.py --output .artifacts/rust-core-consolidation/p0-projection-baseline.json --real-root 'D:\Program Files (x86)\Steam'
```

不传 `--real-root` 时仅构造与删除仓库 `.local` 下的临时 fixture；cargo 使用 `--offline`，本项目隔离 toolchain，构建目标为 `.local/p0-projection-target`。依赖缺失时报错，不自动联网。临时目录删除前已验证其绝对 parent；不会删除真实 Steam 根或正式 state。

P1 在同 fixture、同 profile/平台运行新核心：按实例/补丁身份对照归一化快照，保持检测/来源/优先级语义，新增 actions 的差异单列；整库扫描、单实例与批量投影分别记录时间及哈希次数。不得把单实例时间与旧完整 serve 时间直接对比。新增 snapshot 内只可共享同次取证，不使用跨文件变更未失效的哈希缓存。

当前 `--binary PATH --protocol-version 4` 为新 CLI 构造六字段必需 `params.host`（无 state/signed/trust），按相同 fixture 测时间/体积；这是面向冻结 v4 的适配，v4 实际运行仍待 P5 验证。`--protocol-version 3` 保持现行参数形状；未注入计数时报告 null，不能作为哈希门槛通过证据。P1 迁移 hash 入口后须明确更新计数 adapter（冻结锚点漂移即失败），对 source/动态 committed/signed 优先/preview 补齐核心固定输入，再记录与旧基线的差异。wire 体积还需按 [冻结契约](p0-contract.md) 构造 v4 样本；本表的 v3 体积不是 v4 实测。

## 已执行与待执行

已执行：plain/counted 隔离 debug 编译；18 次合成 serve 扫描、3 次独立计数扫描及 6 次真实清单扫描；fixture 身份/检测断言、响应边界（64 KiB 请求/8 MiB 响应）与语义一致性断言；生产源码前后 SHA-256 核对。

首两次隔离构建发现缺少内嵌 core resources 和 catalog/trust-keys.json，工具已补齐，并在最终完整运行通过。此为测量副本准备问题，正式源码未调整。

未执行：release/Linux 性能、真实补丁大文件哈希、来源候选/收据动态投影计数、v4 真实协议、双端产物、安装/回滚与 Steam Deck 实机。后续各阶段按新增边界补验证；本次不重复既有迁移完整验收。

## v4 DTO 静态体积核对

[P0 wire 样本](p0-wire-samples.json) 只表达本轮冻结形状，不是已生成 Rust/TS/Python 契约，也不是 v4 真实响应。核对以 UTF-8 compact JSON + 一个 LF 为口径；64 KiB/8 MiB 均包含 delimiter，不扩张上限。原始核对结果：[p0-wire-size.json](../../../.artifacts/rust-core-consolidation/p0-wire-size.json)。

| 静态样本 | 完整行字节 | 结论 |
| --- | --- | --- |
| signed 本地 install 请求（host/overwrite/Disabled） | 384 | 低于 65,536 |
| committed / cancelled rolled_back / needs_recovery outcome | 509 / 572 / 633 | 低于 8,388,608 |
| 500 实例无 patch 的拟 v4 report | 279,296 | 低于上限 |
| 24 实例各一 patch 的 all-match / mixed evidence | 45,155 / 42,880 | 低于上限 |
| 500 实例各一 patch，四动作各一个 2048-byte detail | 4,970,799 | 低于上限 |
| 500 实例各四 patch，同等 detail | 19,208,799 | 超限，P5 应返回 response_too_large，不裁 actions/截 JSON/增大限额 |

体积使用归一化 `$FIXTURE` 路径与拟 actions/诊断字段，因此不能与上面的实际 v3 绝对路径行大小直接作增减比较。合成 stress 以 all-match row 为模板，四个 availability 各带 unsigned_catalog blocker；场景扩为 500 行并逐行分配 fixture identity，detail 填满 2048 ASCII bytes，再逐 patch 复制一或四份。这里只验证大小，不把 stress 当合法 signed 准入或生产投影响应。

P5 必须用真实 Rust 生成契约与宿主校验重新测含最大 detail、路径与实例规模的响应，以及超限后下一合法请求的处理；超限导致输出错误不能掩盖已提交事务。安装路径应在写入前保证结构化终态 envelope 可输出；日志/大量扫描结果不得无限复制进 outcome。该目标尚未执行 v4 runtime 验证。
