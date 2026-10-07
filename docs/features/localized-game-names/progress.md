# 本地化游戏名施工记录

设计：[design.md](design.md)。状态：**已实现并自动化验证；Windows 便携部署及当前简体中文界面验收通过，Decky 实机待验收**。日期：2026-10-07（Asia/Shanghai）。

本文件保留前期调研与审阅历史，并记录本轮实际施工。当前实现及验证状态见任务表和施工记录；前期“未执行”记录不代表本轮状态。

## 任务状态

| 工作 | 状态 | 证据/待办 |
| --- | --- | --- |
| 现状与名称来源调研 | 已完成 | Decky 用 `appStore` 本地化名；Windows 用 manifest 名；核心匹配只按 AppID；见下方证据 |
| appinfo.vdf 格式调研 | 本轮契约已固化并验证 | 真实 V29 名称子树 + 合成 V28 fixture；tag 5 布局仍未确认，整条回退 |
| feature 设计与计划 | 已完成；审阅修订已写入 | design.md 与功能索引已建立；本次统一回退契约并补齐实施/验收边界 |
| P0 样本与契约 | 已完成 | 最小 V29 真实名称子树、V28 合成 fixture 与来源/版本/SHA256；完整解码和 config 缺键分支自动化通过 |
| P1 核心解析 | 已实现并自动化验证 | `appinfo.rs` 16 项专项测试；语言/资源/重试及 Windows/Unix 安全路径测试通过 |
| P2 扫描集成 | 已实现并自动化验证 | 多根/外置库/缺元数据/旧 JSON 集成测试及目录匹配回归通过；Windows 真实 CLI 取得中文名 |
| P3 前端对接 | 已实现并自动化验证 | 双端 typecheck/build、Decky 54 项测试通过；appStore 优先、搜索与未完成操作名称回退已覆盖 |
| P4 验收 | 自动化与 Windows CLI/当前简体中文客户端通过；Decky 实机待验收 | Windows Rust 91 项、Linux 94 项通过；双平台 Clippy 与本功能 fmt 通过；全量 fmt 既有差异见下文 |

## 2026-10-07 调研证据

### 名称来源与匹配

- Decky 展示名：`apps/decky/src/steam.ts:1-12` `gamePresentation` 用 `window.appStore.GetAppOverviewByAppID(appid).display_name`（Steam 按界面语言解析），取不到时回退 manifest 名。
- 核心扫描名：`crates/core/src/steam.rs:234` 取 appmanifest 的 `name`（通常英文），`steam.rs:269-278` 产出 `GameInstall`。
- 核心匹配：`crates/core/src/catalog.rs:407-412`、`catalog.rs:443` 只按 `appid` 过滤。因此现有 Windows 匹配与显示名无因果关系；英文名不影响补丁匹配。
- Decky 来源发现才会用标题匹配（`apps/decky/backend/discovery.py`、`sources.py`），且完全在 Python 后端；Rust 核心无来源发现代码，Windows 打包的是 Rust CLI，故 Windows 当前无刮削能力。显示名与来源发现因此可解耦。

### 本机 Steam 实测（Windows）

- Steam 根：`d:/program files (x86)/steam`（`HKCU\Software\Valve\Steam\SteamPath`）。
- 初始语言调研记录：`HKCU\Software\Valve\Steam\Language = schinese`；设计优先尝试 config 键，再以注册表作 Windows 回退。本次只读核对发现 config 当前无 `Language` 键，不能将“优先读取 config”写成本机已从 config 取得语言。
- `appcache/appinfo.vdf` 存在（2,410,670 字节），为**新版 V29**：magic `0x07564429`、universe `1`、其后 int64 key 表偏移 `2,371,981`。
- key 表：偏移处 `int32 count = 4316`，随后 4316 个 NUL 结尾 UTF-8 字符串。实测索引：`4=name`、`5=type`、`449=schinese`、`587=japanese`、`753=name_localized`。
- 名称路径实测：`appinfo.common.name = "My Life as a Cult Leader"`，`appinfo.common.name_localized.schinese = "妃神会秘史"`，`appinfo.common.name_localized.japanese = "色々教団生活"`。即本机结构为 `common.name_localized.<语言>` 直接为字符串（key 索引 753 指向的对象内，449/587 两个键为字符串值）。
- 初始条目框架尝试：脚本按 `entry_size - 64` 步进时错位；该失败记录保留。本次审阅已定位为固定头长度误算，修正公式与只读证据见下方“审阅修正”，P0 仍须固化完整解码 fixture。

### appinfo.vdf V29 格式（库调研 + 实测）

- 头部：`magic[4]` + `uint32 universe`；V29 其后 `int64`（小端）key 表偏移；V28 `0x07564428` 无该字段、字符串键内联；V27 `0x07564427` 与未知 magic 本轮不解析。
- 条目头为 **68 字节**（初稿误写约 72 字节，本次已纠正）：`appid(u32), entry_size(u32), info_state(u32), last_updated(u32), access_token(u64), sha_hash[20], change_number(u32), vdf_sha_hash[20]`。payload 长度为 `entry_size - 60`，下一条偏移为 `offset + 8 + entry_size`。
- binary VDF 标签：`0x00` 对象开始（带 key_index）、`0x01` UTF-8 字符串（NUL 结尾）、`0x02` int32、`0x03` float32、`0x04` pointer(int32)、`0x05` UTF-16 字符串、`0x06` color(int32)、`0x07` uint64、`0x0A` int64、`0x08` 对象结束（单字节）。
- 参考实现：Protontricks `src/protontricks/steam.py` 与 `src/protontricks/_vdf/__init__.py`（格式说明）。Protontricks 为 GPL-3.0，本仓库 MIT，实现须为 clean-room，仅参考格式、不复制代码。
- 稳定性：Steam 私有缓存，magic/条目/标签/路径可能变动；文件可达数十至上百 MB；Steam 运行中可能原子替换。实现需版本分支、有界读取、失败回退。

## 2026-10-07 审阅修正（施工前）

### 文档变更

- 修改范围仅为本功能的 `design.md` / `progress.md`；功能索引已有双链接和“方案（待实现）”状态，本次状态不变。
- 统一名称回退为目标语言名→`common.name`→manifest；明确 `localized_name` 是 appinfo 展示覆盖值，默认名也可填入，但 `name_language` 仅目标语言命中时非空。未取得语言仍可使用默认名。
- config 缺失/缺键/空值/读取失败时尝试 Windows 注册表；限定到注册表 `SteamPath` 对应的安装根，补齐 config 缺失与回退成功的验收分支。
- 修正 68 字节条目头与长度公式；保留初始错误步进的历史记录，区分容器核对与待实施的完整解析/fixture 验收。
- 明确 Steam 安装根与库根的关联、外置库继承、重叠库先根优先及根内名称映射；补齐对应验收。
- 明确 Windows 搜索修改共享 `filterInstalls`；Decky 修改调用方三处 fallback 参数及未完成操作记录，保留 appStore 优先。
- 将“拒绝链接”改为新增 config/appinfo 读取的约束，保留安装根别名规范化；补资源限额、路径安全与重试验收。

### 已执行的只读核对

- 代码：核心 `scan()`（`steam.rs:81-151`）先收集/去重库，现状未保留库所属安装根；`read_keyvalues`（`steam.rs:295-306`）用 metadata 与普通 open，未拒绝链接。
- 前端：共享 `filterInstalls`（`packages/ui/src/index.ts:175-183`）当前只搜索 manifest 名和 AppID；Decky `index.tsx:144/173/260` 均传 `item.game.name` 作为 fallback，“未完成的操作”记录另有 `game.name`（审阅时行号 424），找不到安装对象时显示 instance_id。
- 本机 config：只读检查 63,997 字节的 `config/config.vdf`，未发现 `Language` 键。注册表值沿用初始调研记录，本次审阅未重新查询注册表或切换 Steam 语言。
- 本机 V29 容器：读取同一份 2,410,670 字节样本，从偏移 16 按 `offset + 8 + entry_size` 遍历 899 条，最终偏移 2,371,977 的 u32 为零，随后 key 表位于 2,371,981。首条 AppID 5，entry_size 85，payload 长度 25，下一条偏移 109；AppID 2303800 的条目偏移 2,014,833，entry_size 2485，payload 长度 2425。
- 外部格式核对：[Protontricks steam.py](https://raw.githubusercontent.com/Matoking/protontricks/master/src/protontricks/steam.py) 的 V28/V29 字段布局与长度公式相符；[binary VDF 定义](https://raw.githubusercontent.com/Matoking/protontricks/master/src/protontricks/_vdf/__init__.py) 用作格式参考。本轮未复验完整名称树解码。

### 文档检查与未验证项

- 文档检查通过：两份文档共 8 个本地相对链接均可解析，代码块闭合，设计/施工记录及功能索引均保留“方案（待实现）”；P0–P4 状态未误标完成。名称/语言验收矩阵已与字段契约核对。
- 功能仍为待实现；P0–P4 未执行。未修改功能代码，未运行 Rust/Python/前端测试、构建、部署或客户端实机验收。

## 前期前置清单（保留历史）

- P0 用最小 fixture 固化已核对的 V29 条目框架，补 V28、完整 payload 解码与名称路径验证；记录采样来源/版本/校验值。
- 按设计实现并验证 config 缺键、语言缺失、注册表根匹配/不匹配与多账户场景；本次仅确认本机 config 缺键，尚未实现语言解析。
- 确认 Linux/Steam Deck 路径（`~/.local/share/Steam`、Flatpak/Snap 变体），本 feature 两平台共用核心但本轮以本机 Windows 样本为准。
- 明确 `GameInstall` 新字段对 Decky Python 消费方、Rust 测试与 schema 导出的影响（应为向后兼容的可选字段）。
- 来源发现/刮削核心化为独立 feature，本 feature 只提供本地化名字段，不越界实现。

## 施工前未执行项（保留历史）

- 未修改任何功能代码；未运行 Rust/Python/前端测试、构建、部署或实机验证；未提交 Git。

## 2026-10-07 功能施工与验证

### 实际实现

- [appinfo.rs](../../../crates/core/src/appinfo.rs)：新增独立 V28/V29 流式容器解析，只读取目标 payload；key 表、字符串、节点、深度与文件/条目大小有界。完整解码后才返回名称；坏目标条目省略，坏容器全根回退。默认名填入 localized_name 时 name_language 为 null。
- [最小 fixture 与来源](../../../crates/core/tests/fixtures/localized-game-names/README.md)：V29 为真实公开名称子树重构，V28 为合成编码，分别为 239/203 字节，记录来源版本和 SHA256。新快照为 2,416,036 字节、900 条，和前期 899 条快照区分。完整 2425 字节目标 payload 已独立核对三个名称。
- [steam_metadata.rs](../../../crates/core/src/steam_metadata.rs)：config 优先，缺键/异常时采用根限定的 HKCU 回退；语言码 trim + ASCII 小写。Unix 用目录句柄和 openat/O_NOFOLLOW，Windows 持有不共享写入/删除的目录与文件句柄并拒绝 reparse point。单句柄读取、大小/mtime 前后核对，失败最多重新打开一次。
- [steam.rs](../../../crates/core/src/steam.rs)：GameInstall 新增两个 Option 字段，manifest 名不变；保存库所属 Steam 根，先出现的根拥有重叠库；按根汇总目标 AppID 并填充，不混用其他根缓存。安装实例、排序和目录匹配规则保留。Unix 新增 libc 依赖，Cargo.lock 只补核心依赖关系。
- [共享 UI](../../../packages/ui/src/index.ts) 与 [Windows](../../../apps/windows/src/main.tsx)：可选字段与 gameDisplayName，列表/详情展示覆盖值，filterInstalls 同时搜索 manifest 名、名称覆盖和 AppID。
- [Decky 名称入口](../../../apps/decky/src/steam.ts) 与 [调用方](../../../apps/decky/src/index.tsx)：原 gamePresentation 的 appStore 优先逻辑保留；调用方传核心展示值作为 fallback。matchesGameQuery 与 transactionGameName 统一搜索/未完成操作记录，找不到安装对象时仍显示 instance_id。
- [扫描集成测试](../../../crates/core/tests/localized_game_names.rs)、目录匹配回归和 [前端名称测试](../../../apps/decky/tests/localized-name.test.ts) 覆盖多根/外置库、缺语言、旧 JSON、链接回退、双名称搜索与 appStore 异常。保留施工前其他功能的未提交修改，未提交 Git。

### 自动化与只读实测结果

| 检查 | 结果与范围 |
| --- | --- |
| Windows：cargo test --offline -j 1 -p steam-patch-core -p steam-patch-cli | 91 项通过（核心 84 + CLI 7）；含 parser 16 项、安全读取 4 项、名称集成 4 项、匹配回归 |
| Linux：同命令，经已有 WSL Ubuntu，CARGO_TARGET_DIR=target/linux | 94 项通过（核心 87 + CLI 7）；含 Unix 最终 symlink/FIFO 拒绝及根别名/父目录链接测试 |
| Windows/Linux：cargo clippy --offline -j 1 -p steam-patch-core -p steam-patch-cli --all-targets -- -D warnings | 均通过；新增嵌套模式与测试 clone 建议已修正并复验 |
| 本功能 Rust 文件 rustfmt --check（edition 2021、skip_children=true） | 通过；仅格式化本功能涉及文件 |
| cargo fmt -p steam-patch-core -p steam-patch-cli -- --check | 未全量通过：既有 crates/core/tests/install.rs 约 900 行存在格式差异；该文件本轮未修改 |
| npm.cmd run typecheck | Windows/Decky 均通过 |
| npm.cmd run test -w apps/decky | 54/54 通过；新增 3 项名称行为测试 |
| npm.cmd run build:windows / npm.cmd run build:decky | 均通过 |
| git diff --check | 通过 |
| 独立只读代码审阅 | 未发现可证实、可触发的安全/正确性/兼容问题；构建测试由主 Agent 执行，非 reviewer 声称 |
| Windows 真实 Steam CLI scan（协议 v2、用户本机 HKCU） | 33 个安装结果、0 诊断，约 55 ms；AppID 2303800 保留英文 name，localized_name=妃神会秘史，name_language=schinese，availability=installed |

本轮本机注册表重新只读核对：SteamPath 为 d:/program files (x86)/steam、Language 为 schinese，config 仍缺语言键，因此真实 CLI 证明注册表回退与实际缓存解析共同生效。Node 沙箱子进程的 HKCU 与本机账户隔离，首次扫描只得到 common.name/null；该结果不作为界面语言实测，改用本机执行账户后得到上述中文字段。未切换 Steam 语言，未写 Steam 或游戏文件。

首轮并发 Rust 链接出现 link.exe 异常，串行复跑后全部通过；未安装或修复工具链。默认文件工具通道长时间无输出/拒绝写入，后改用已批准的本机执行通道，写入限定于本功能文件并核对当前内容。

### 设计调整与剩余验收

- tag 5（UTF-16）缺少可验证的 appinfo 布局样本，本轮明确识别后丢弃整条名称覆盖；没有猜测长度前缀或 NUL 终止形式。已同步设计和 fixture 说明，不能标为 UTF-16 已支持。
- 资源边界验证采用同一解析路径注入限额来检查 file/payload/key count/key bytes 等于/超过；另有实际生产常量的 1 MiB 字符串、64 层、262,144 节点边界测试。未将小 fixture 描述成 256 MiB 真实缓存性能验收。
- Windows 已验证目录 junction 拒绝、目录替换/文件写入被持有句柄阻止；本机普通文件 symlink 创建能力不可用，最终普通文件 symlink 用例待验证。Linux 最终 symlink/FIFO 测试已执行；并发路径替换压力测试未执行。
- 截至实现阶段，Windows 列表/详情/搜索、Steam 切换语言、Decky appStore/fallback 页面待验收；后续 Windows 当前简体中文界面已验收，见下文。未执行 Steam Deck 部署、重载或设备操作。
- 实现与自动化阶段完成，P4 保留上述人工/实机验收和全量 fmt 既有差异，不标为全部验收完成。

## 2026-10-07 Windows 便携部署与界面验收

### 部署与证据

- 沿用本机既有 `.artifacts/windows-portable/` 部署方式；未发现已安装的 NSIS 客户端。备份旧便携目录、ZIP、NSIS、依赖清单及用户状态后，构建 release CLI、Tauri 与 NSIS，更新原便携目录并启动生产客户端。
- 命令：`cargo build --offline --locked -j 1 --release -p steam-patch-cli`；`CARGO_BUILD_JOBS=1 npm.cmd run bundle -w apps/windows -- -- --offline --locked`（含 Windows typecheck/Vite）；更新 SBOM 后运行 `py -3 scripts/package-artifacts.py --platform windows`。均通过。最初 Tauri 附加 Cargo 参数多带了分隔符，已纠正；未将该失败标为通过。
- 首次生产界面暴露既有 refresh 并发读取事务状态的 os error 32 阻塞；修复后重新构建并部署，详见独立[设计](../windows-scan-state-lock/design.md)与[施工记录](../windows-scan-state-lock/progress.md)。保留第一次旧部署备份。
- 最终便携 ZIP SHA-256：`df797d03f6c604aa3c07f827e1546d70a47f67f3a31b45bfd404c29c8b1aec96`。客户端和 CLI 部署哈希与 target/release 一致，全部部署文件复核通过；NSIS 仅生成，未安装或验收安装器流程。
- 证据目录：`.artifacts/localized-game-names-windows-20261007/`；含 backup、deployment-manifest.json、packages.json、ui-evidence.json、localized-detail.png、deployed-cli-names.json 和 windows-source-gap.json。截图来自真实生产窗口，未用模拟数据或注入前端结果。

### 真实界面验收结果

| 项目 | 结果 |
| --- | --- |
| 初次启动及重启 | 33 个安装实例正常加载，目录 revision 4，签名已验证，无文件占用错误 |
| 中文列表与详情 | AppID 2303800 均显示“妃神会秘史”，保留原安装目录及 build 10821988 |
| 中文名 / manifest 英文名 / AppID 搜索 | 三种均命中同一实例；英文大小写变化、首尾空格兼容 |
| 默认名 | Dota 2（AppID 570，name_language=null）列表、搜索及详情正常 |
| 分类与搜索 | 无目录条目的目标在“有补丁”分类下为 0，返回“全部”后为 1；无匹配搜索为 0；清空恢复 33 |
| 连续重新扫描 | 3 次完成，结果和名称保持，无 os error 32 |
| Steam 元数据 | config/config.vdf 与 appcache/appinfo.vdf 前后 SHA 一致，语言保持 schinese |
| 用户状态 | 事务身份、状态及内容保留；既有 migrate_state 仅刷新一条事务的 updated_at，因此 journal.sqlite 二进制哈希变化；trust.sqlite 哈希一致 |

### 边界与来源差异

- 本轮通过的是当前简体中文环境的 Windows 名称功能。未实际切换 Steam 界面语言；名称/语言边界沿用自动化记录。Decky 当前改动仍待部署验收。
- 自动探测另一个既有 Steam 根时产生 1 条库不可访问诊断，未影响 33 个有效实例；不与显式单根 CLI 的 0 诊断混写。
- 用户指出妃神会秘史 Windows 无源而 Decky 正常。用客户端相同资源/状态请求核实：name 与 localized_name 正确，catalog_trust=signed、revision=4，但 `match_status=no_catalog_match`、patches=0，目录中 AppID 2303800 条目为 0，且无 discovery 字段。Decky 的动态查源位于其 Python 宿主；Windows 当前只读静态目录。名称功能验收通过不代表两端来源能力对齐，该独立缺口尚未修复。
- 未安装游戏补丁、未操作 Steam Deck、未提交 Git。部署包含当前工作区已有 Windows 改动；没有将本轮名称验收扩大为所有安装流程验收。
