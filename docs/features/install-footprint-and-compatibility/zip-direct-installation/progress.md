# ZIP 无备份直接安装施工记录

日期：2026-10-18（Asia/Shanghai）。关联：[设计规划](design.md) · [父功能施工记录](../progress.md) · [EXE 直接安装记录](../install-transaction-performance/progress.md)。

当前状态：**Z1–Z4 已实现；Z5 自动化完成，Steam Deck 部署与一次 ZIP 直接安装通过，用户反馈自行验证无问题；界面截图、中断与性能对比未逐项归档。**

| 阶段 | 状态 |
| --- | --- |
| Z1 映射与入口契约 | 已实现（映射/入口契约自动化通过） |
| Z2 普通 ZIP 直接安装 | 已实现（自动化通过） |
| Z3 签名目录和本地 ZIP | 已实现（自动化通过） |
| Z4 状态及删除收尾 | 已实现（授权链删除、旧事务夹具保留） |
| Z5 自动化/实机/性能验收 | 自动化通过；Deck 部署与一次 ZIP 直接安装通过；用户反馈自行验证无问题；界面截图、中断及性能对比未逐项归档 |

## 2026-10-18：独立精简计划

- 用户要求另外制定 ZIP 精简计划。按已有子功能组织方式新建 zip-direct-installation/，与 EXE 计划独立维护；父功能与功能索引链接同步更新。
- 静态基线 68a2c45，读取 ZIP 编排、recipe 映射、签名入口和已存在的 direct 接口。确认普通/签名 ZIP 仍进入旧 prepare/apply，而当前 EXE 已有可复用的直接安装接口；没有覆盖或修改 EXE 实施记录。
- 决定新 ZIP 使用无备份直接覆盖，移除全量预解压、原文件摘要、旧授权/备份事务、逐文件持久化及用户本地 ZIP 托管复制。保留实际路径映射和签名入口一次身份核对，收据摘要随唯一写入流计算。
- 明确失败/取消保留部分结果、从头重试，旧事务按旧路径处理；原态条件消费者在实施前核对，避免旧规则阻断重试。
- 使用 overengineering-review 审阅方案：复用现有接口，不引入通用框架或新的防御流程。本轮仅文档，没有运行安装、测试、部署或提交 Git。
- 文档相对链接及 git diff --check 已通过；这些检查不代表实现或性能验收。

## 2026-10-18：Z1–Z4 实施

### 映射与入口（Z1）

- `zip_recipe` 重写为 `open` / `plan` / `plan_mapped`，返回成员索引、原始名称、目标相对路径与声明大小；删除成员内容预读、原文件摘要与 stage 估算。
- 保留并复用路径越界、保留前缀、编码、重复/大小写冲突、文件/目录冲突、链接/特殊文件、extra 字段及 XP3/YPF 包装映射检查；包装检测只读必要成员头。
- 签名 recipe 直接使用 `source → destination` 映射，成员声明摘要随解压流比较；未引用的成员只做安全校验、不安装。
- 核对事实条件：普通来源仍在入口执行一次 `game_facts_unreviewed`（语言、DLC、实例/清单）；签名入口执行一次 reviewed facts。删除 prepare 的 `baseline_conflict` 与 reinstall 原文件摘要要求，部分写入后的重试不再被旧基线阻断。

### 普通 ZIP（Z2）

- 新流程：获取 ZIP（下载时已算摘要）→ 同一 `ZipArchive` 句柄完成成员映射 → 一次事实检查 → `begin_direct_at` → `zip_direct::extract` → `commit_direct_at`。
- `zip_direct` 直写最终目标：已有文件覆盖、新文件创建、保留既有权限；按文件同步、成功后处理受影响目录；解压流中计算 SHA-256，成员 EOF 时由 zip 库校验 CRC，并核对声明大小与签名摘要。
- 进度使用既有 `downloading`/`extracting`/`cleanup` 阶段和字节/文件事件；取消在阶段边界与每个读取块检查。
- 空间模型改用 `ZipSpaceBudget::for_direct_overwrite`：只按目标卷峰值净增长加统一 margin 预留，已下载 ZIP 与旧文件备份均不重复计入。
- 失败/取消在写入前返回原错误或普通取消；写入后 `fail_direct_at` 记录 `install_incomplete: …` 或 `cancelled_incomplete`，保留已写入文件，重放由请求账本返回同一失败终态。
- 内容身份由归档摘要 + 目标映射生成，同时写入 `recipe_sha256` 与 `content_id`；安装态摘要来自唯一解压流，供收据/检测消费者使用。

### 签名目录与本地 ZIP（Z3）

- 目录签名、版本/撤销与 release 选择只校验一次；删除 apply 前的目录/事实/计划复验与 `authorization.json` 写入校验。
- 远程包沿用下载流一次摘要比对；缓存复用沿用既有重哈希；本地用户 ZIP 在入口做一次声明摘要校验后直接打开，不再复制/adopt 到 job 目录，也不再被缓存清理引用。
- 签名与普通 ZIP 共用同一条直写路径；本地包失败/取消/缓存清理后仍留在原路径。本地包在同一打开句柄上完成一次声明摘要校验后交给解压库，不再按路径重开；签名成员计划按 recipe 顺序写入日志，保持 projection 的重装比较一致。

### 状态与删除收尾（Z4）

- 删除 `installer/authorization.rs` 及模块导出；移除 orchestrator 的授权/复验/recheck 链和 `state_dir` 依赖，移除签名 `clock` 依赖。
- `begin_direct_at` 增加可选 `recipe_sha256`，保留 ZIP 重装身份检查所需计划身份；EXE 行为不变。
- 旧 `prepare_for_request_at`、`apply_at`、`adopt_external_plan_archive_at` 及旧 stage/backup 前向写路径改为 `#[cfg(test)]` 历史事务夹具；`recover_checked_at`/`restore` 仍按旧语义恢复旧备份事务。
- 直接安装核心层不强制旧成功收据；`projection` 的 `reinstall_*` blocker 继续约束 UI 的重装入口。核心层显式 `Reinstall` 与 EXE 一致按覆盖执行。
- `recover_selected_at` 在把 direct 事务标记为失败前探测游戏锁：另一个进程仍在写入时跳过该事务，不重写其日志；并修复 `open_direct_target` 先截断后拒绝硬链接的问题，改为校验通过后才清空目标。

### 验证

- `cargo check --offline --workspace --all-targets`：通过。
- `cargo test --offline --workspace`：全部目标通过；core lib 409 通过 / 5 ignored，signed_install 21 通过，CLI 47 通过。
- `cargo clippy --offline -p steam-patch-core -p steam-patch-cli -p steam-patch-protocol -p steam-patch-contract-gen --all-targets -- -D warnings`：通过。
- `cargo fmt --all --check`：通过。
- `cargo run --offline --locked -p steam-patch-contract-gen -- check`：`v4 contracts check: ok`。
- 新增自动化覆盖：成员映射与目标路径（含 XP3/YPF、ZIP64、大小写/目录冲突、签名映射与顺序）、单流解压与 SHA-256、损坏成员内容不阻断映射且解压校验保留已写内容（证明映射不预解码）、签名摘要不匹配保留已写内容、取消、失败/取消终态与重放、重装覆盖、既有文件权限保留、硬链接拒绝且不截断、无 backup/stage/authorization 产物、旧计划恢复不回滚 direct 事务、直接覆盖的空间预算不重复计入旧文件。
- 未执行：Windows/Steam Deck 实机安装、取消/断电中断、真实大包性能与空间峰值测量；不得据此宣称实机或性能验收通过。
- 遗留边界：`recover` 的活锁探测在 Unix 有自动化覆盖；Windows 的 `fs2` 锁为进程级，同进程并发 `recover` 与 `install` 未新增进程内登记表，也未实测，跨进程场景由游戏锁覆盖。direct 计划使用随机 plan id，缓存清理的废弃任务目录关联仍按 `request_plan_id`，硬崩溃后遗留的 direct 任务目录不会被回收（仅任务元数据，不含用户 ZIP）。
- 本轮未创建 Git 提交（用户未要求）。

### 实现后过度设计审阅

- 沿普通 ZIP、签名 ZIP、本地 ZIP 的真实入口核对：映射只读中央目录/包装头，唯一解压流产出 SHA-256 并由解压库校验 CRC，失败/取消不引入回滚或恢复状态，旧恢复路径只服务历史事务。
- 随审阅简化：映射不再重复 stat 目标（由 `zip_target_sizes` 与游戏锁内的 `begin_direct_at` 各负责一次），删除 `validate_entries` 已无消费者的累计字节检查，删除已无调用的 `space::original_bytes`，`for_stage`/`after_staging` 仅保留给历史事务测试；移除自算 CRC 与 `crc32fast` 依赖，改由 zip 库在成员 EOF 校验。
- 独立实现审阅（oracle，静态逐路径，未实机）发现并修复：空间预算错误计入旧文件（新增 `for_direct_overwrite`）；签名成员计划按归档顺序写入导致 projection 重装比较失配（改为 recipe 顺序）；`open_direct_target` 先截断后拒绝硬链接（改为校验后清空）；本地 ZIP 校验后按路径重开（改为同一句柄）；并发 `recover` 可能改写运行中 direct 事务（增加游戏锁探测）。
- 结论：未发现需要新增防御分支或新框架的问题；未将自动化通过视为实机或性能验收。

## 2026-10-18：Steam Deck 部署与 ZIP 直接安装

- 设备：`deck@192.168.31.186`（Devkit 当前配对，独立 `known_hosts`）。通过 Loader `utilities/install_plugin` 安装本轮 ZIP：`7d2a0bfc…`（23 个文件），安装 prompt 的 name/version/hash/type 与实际 ZIP 一致。
- 安装后逐文件 SHA-256 核对 23/23 一致；`bin/steam-patch` 为新 Linux 构建 `fdfb0823e5fbd4f86cfcdb26513b424f9b6cd5ced90f2ad37c7b3af500b0db92`，`sf-worker` 未变；插件实例 1、前端 bundle SHA-256 `ae1cac…` 与本地构建一致，后端进程已由 Loader 重启。
- 前两次安装任务分别在下载 68%（`4238766b…`）和 208 MB（`60dbc151…`）时取消，均在预计划阶段，未写入游戏文件；日志保留。
- 随后从本机下载同一 HTTPS ZIP（813,370,375 B）并按内容寻址放入设备缓存，补 `Last-Modified` 使条件请求返回 304。一次 ZIP 直接安装在约 15 秒内完成：plan `da4fd30e-4dc4-4617-ba94-f94c4194b6bd`，request `90812614…`，transaction `committed`、`direct=true`，2 个文件（`pac/update1.ypf` 2,116,101,941 B、`readme.txt` 2,248 B）。
- 该 plan `installed_hashes=2`、`original_hashes=0`、`sources=0`、`archive_source=null`，job 目录为空，无 `.backup`/`.stage`/`authorization.json`/`payload.zip`；`content_id` 与 `recipe_sha256` 相同。两个安装态 SHA-256 与同补丁的旧事务收据完全一致，游戏内容未改变，仅 mtime 更新，无需回滚。
- 证据：本地 `.artifacts/zip-direct-deck-20261018/`；设备 `/home/deck/steam-patch-validation/zip-direct-20261018/`（旧插件备份、manifest、脚本与日志）。
- 用户反馈已自行验证无问题；本轮未逐项采集其界面/中断操作证据，未归档项仍不标记为自动化或本轮实测通过。
- 仍待归档：QAM/全屏真实界面截图、提取中取消/断电中断、Windows 实机、同包同初始状态的旧路径与新路径性能对比（当前 15 秒仅为单次 journal 时间戳，不作为正式基准）。
