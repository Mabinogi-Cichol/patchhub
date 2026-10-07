# 本地化游戏名（核心产出 + Windows 展示对接）

施工记录：[progress.md](progress.md)。状态：**已实现并自动化验证；Windows 便携部署及当前简体中文界面验收通过，Decky 实机待验收**。日期：2026-10-07（Asia/Shanghai）。

## 1. 目标与范围

让游戏显示名对齐 Steam 的界面语言：Windows 客户端不再显示 appmanifest 里的英文 `name`（例如 AppID 2303800 显示为 "My Life as a Cult Leader"），而是显示 Steam 当前语言下的本地化名（`schinese` 时为"妃神会秘史"）。

按仓库既有架构原则（[总体计划](../../development-plan.md)："共享 Rust 核心负责 Steam 库解析、匹配"；[客户端指南](../../client-guide.md)："两端调用同一个 Rust CLI"），本地化名由**共享 Rust 核心**在扫描时产出，两个客户端共用一份口径。

范围：

- 核心层在扫描阶段读取本机 Steam 文件产出本地化名，并随扫描结果返回。
- Windows 展示名与搜索对齐该本地化名。
- Decky 仍以 Steam 客户端的 `appStore` 覆盖为主，仅在取不到时回退到核心字段。

非范围（明确不做，避免范围蔓延）：

- **来源发现 / 刮削（标题匹配）**：不在本 feature。它属于应迁移到共享核心的独立工程，见"边界"。
- **联网命名**：不使用未公开的 Store `appdetails` 端点作为名称来源（列为后续可选）。
- **EXE 的语言 / 变体 / DLC 展示对齐**：另议。
- **改变现有匹配逻辑**：核心目录匹配继续只按 AppID（`crates/core/src/catalog.rs:407`），本 feature 不改变匹配结果。

需要先纠正一个前提：现有 Windows 补丁匹配完全按 AppID，英文显示名**不会**导致 Windows 匹配不到补丁；"因英文名导致无法刮削"只适用于 Decky 的 Python 来源发现，而 Windows 目前没有该能力。本 feature 解决的是显示口径，并为将来核心化来源匹配提供同一份本地化名字段。

## 2. 产品决策

### 2.1 名称来源优先级

按优先级取第一个可用值：

1. Steam 界面语言对应的本地化名。
2. `appinfo.vdf` 的 `common.name`（通常是默认/英文名）。
3. appmanifest 的 `name`（现有行为）。

名称先 trim，空值不算可用。appmanifest 的 `name` 保持原值；`localized_name` 是可选的 appinfo 展示覆盖值，可承载目标语言名或默认名。字段契约如下：

| 采用的来源 | `localized_name` | `name_language` | 前端展示 |
| --- | --- | --- | --- |
| 目标语言名 | 对应的非空名称 | 实际匹配的语言码 | `localized_name` |
| `common.name` | 对应的非空默认名 | `null`（不推断默认名的语言） | `localized_name` |
| manifest | `null` | `null` | 原有 `name` |

未取得界面语言时仍可采用 `common.name`，但不查找其他语言或假定它为英语。后续消费者不能仅凭 `localized_name` 非空认定已命中目标语言，应同时检查 `name_language`。

### 2.2 语言来源

- 主：`<steam-root>/config/config.vdf` 的 `InstallConfigStore/Software/Valve/Steam/Language`。
- 备（Windows）：注册表 `HKCU\Software\Valve\Steam\Language`（本机实测存在，值为 `schinese`）。
- config 文件缺失、读取/解析失败或该键为空时，才尝试 Windows 注册表。仅当当前 Steam 根的规范路径与注册表 `SteamPath` 对应的规范路径一致时使用；路径无法确认一致则跳过，避免多个 Steam 安装混用语言。
- 语言码 trim 后转为 ASCII 小写；空值不可用。未知非空语言码按原码查找，不猜测其他语言。
- config 与适用注册表均取不到语言时，`name_language == null`；名称仍按 2.1 回退到 `common.name`，再到 manifest。本机审阅时 `config.vdf` 未包含 `Language` 键，注册表回退是必须覆盖的分支。

### 2.3 纯本地、离线优先

名称解析只读本机 Steam 文件，不联网、不引入未公开端点。理由：核心已在本机读取 Steam 元数据；离线可用；避免对非官方 Store 端点的依赖与频率限制风险。

### 2.4 失败即回退，不影响扫描

缺失目标语言名或未取得语言时，若目标条目完整且 `common.name` 可用，则采用默认名覆盖；只有默认名也不可用时才回退 manifest。appinfo 文件读取失败、版本不支持、文件超限、容器/key 表损坏时，本根的名称覆盖全部置空；单个目标条目解码失败、超限或截断时，只清空该条目的覆盖，不保留从不完整条目读出的部分名称。条目边界损坏、无法安全定位下一条时，视为容器损坏并清空本根覆盖。

以上情况都保持扫描成功，不减少已有安装结果，不改变匹配结果。默认静默回退；原因仅在显式开启的开发诊断中记录，不新增常规扫描诊断噪声。

### 2.5 Decky 保持 Steam 客户端优先

Decky 的 `apps/decky/src/steam.ts` 已通过 `appStore.GetAppOverviewByAppID(...).display_name` 取得 Steam 覆盖语言后的名称，保持不变；仅当它取不到时，回退到核心的本地化名字段。这样两端口径一致，且不弱化 Decky 现有行为。

## 3. 技术方案

### 3.1 数据模型与协议

`crates/core/src/steam.rs` 的 `GameInstall` 增加两个可选字段：

```text
localized_name: Option<String>   // appinfo 展示覆盖值：目标语言名或 common.name
name_language:  Option<String>   // 仅目标语言名命中时填写；默认名/manifest 回退为 null
```

- 可选字段，序列化后旧消费方忽略未知字段；`scan` 结果向后兼容，**不提升协议版本**。
- `GameInstall` 的构造点（`steam.rs:269`、`crates/core/tests/catalog.rs:9`）需同步补齐。

### 3.2 appinfo.vdf 解析（新增核心模块）

新增 `crates/core/src/appinfo.rs`，职责单一：给定规范 Steam 根、目标 AppID 集合和可选语言，返回各 AppID 的名称覆盖与语言码，遵循 2.1/2.4。格式要点（依据库调研 + 本机样本实测）：

- 文件头：`magic(u32)` + `universe(u32)`。
  - V29：`0x07564429`，其后 8 字节小端 int64 为 key 表绝对偏移。
  - V28：`0x07564428`，无 key 表偏移，字符串键内联。
  - V27 `0x07564427` 及未知 magic：本轮不解析，直接回退（可后续补）。
- 条目（V28/V29）：固定头为 **68 字节**：`appid(u32), entry_size(u32), info_state(u32), last_updated(u32), access_token(u64), sha_hash[20], change_number(u32), vdf_sha_hash[20]`；`entry_size` 不包含最前面的 appid 与 size 两个 u32。条目起始偏移为 `offset` 时，`payload_start = offset + 68`、`payload_len = entry_size - 60`、`next_offset = offset + 8 + entry_size`。
  - V29 条目从文件偏移 16 开始，结束于 key 表偏移前 4 字节的零 AppID 终止标记；V28 从偏移 8 开始，以文件末尾的零 AppID 标记结束。
  - 所有偏移/长度运算使用 checked arithmetic；拒绝 `entry_size < 60`、越界 key 表偏移、跨越条目区的 payload，以及缺失终止标记。不得读取终止标记后的伪条目。
  - 本机 V29 样本按此框架遍历 899 条后精确到达偏移 2,371,977 的终止标记，key 表偏移为 2,371,981；P0 仍须建立可提交 fixture 并验证完整 payload 解码，V28 也需单独 fixture，不能把容器遍历视为完整解析器验收。
- key 表（V29）：`int32 count` + count 个 NUL 结尾 UTF-8 字符串，索引从 0 开始；binary VDF 记录用 `int32 key_index` 引用。
- binary VDF 键：V29 用 int32 key_index，V28 用内联 NUL 结尾键名。标签：`0x00` 对象开始（带键）、`0x01` UTF-8 字符串（NUL 结尾）、`0x02` int32、`0x03` float32、`0x04` pointer(int32)、`0x05` UTF-16 字符串（本轮识别后丢弃整条名称覆盖，布局待真实 fixture 确认）、`0x06` color(int32)、`0x07` uint64、`0x0A` int64、`0x08` 对象结束（单字节）。
- 名称路径：以**解码后的树**为准。
  - 本机 V29 实测：`appinfo.common.name`（英文名）与 `appinfo.common.name_localized.<语言>`（字符串；实测 key 表 `753=name_localized`、`449=schinese`、`587=japanese`，`common.name_localized.schinese == "妃神会秘史"`）。
  - 兼容变体：优先取 `common.name_localized.<语言>`；其不存在或为空白时，再尝试 `common.localization.<语言>`（对象取 `name` 字段，字符串直接采用）。此变体尚无本机实测证据，P0/P1 用独立 fixture 验证。
- 标签支持边界：本轮支持 0/1/2/3/4/6/7/10/8；tag 5 尚无可验证的 appinfo 样本，不猜测长度或终止形式，遇到时按单条解码失败回退。该限制不等于已支持 UTF-16；后续有真实 fixture 后再扩展。
- 效率：先扫描条目头匹配 `appid`，只解码命中的 payload；不整文件反序列化（文件可达数十至上百 MB）。
- 健壮性：未知标签、越界 key 索引、截断按 2.4 回退。单句柄读取；读取或一致性检查失败后最多重新打开重试一次，重试前丢弃本次部分结果，并重新执行路径安全检查。
- 资源限额：appinfo 文件最大 256 MiB、单条 payload 最大 16 MiB、key 表最多 65,536 项且字符串总字节最多 8 MiB、单个字符串最多 1 MiB、对象嵌套最多 64 层、单条解码节点最多 262,144 个；config 最大 4 MiB。检查长度/计数后才分配，超限按 2.4 回退。P0 如需调整阈值，须用样本证据同步修改设计与边界测试。

许可：库调研参考的 Protontricks 为 GPL-3.0；本仓库为 MIT。只把其格式说明作为**格式参考**，实现需为根据格式文档的 clean-room Rust 代码，不复制其源码。

### 3.3 集成到扫描

当前 `scan()` 先从每个 Steam 安装根读取 libraryfolders，随后将主库与附加库去重，交给 `scan_library`。安装根与库根不是同义词：config/appinfo 属于 Steam 安装根，不应从外置库根查找。新增集成应保留这一关联：

1. 按现有 roots 顺序规范化 Steam 安装根，为每个唯一安装根建立扫描上下文；收集其主库及 libraryfolders 中的附加库。
2. 按现有规范路径去重库，并保留 `library → owning_steam_root`。多个 Steam 根引用同一库时，沿用现有“先出现者优先”的扫描顺序，由第一个根提供该库的名称元数据；不得重复产出安装结果或混合多个根的名称/语言。
3. 扫描各库、解析 manifest，保留原有 `name`，并按所属 Steam 根汇总已安装的目标 AppID 集合。
4. 每个根先读一次 config 获取语言；缺文件、缺键、空值或解析失败时，按 2.2 尝试适用的 Windows 注册表回退。语言仍不可用时继续查默认名，不提前跳过 appinfo。
5. 每个根打开一次 `appcache/appinfo.vdf`（失败重试例外），只解码本根目标 AppID 的 payload，建立根内 `appid → (localized_name, name_language)` 映射，再填充所属安装结果。其他根的缓存不得充当兜底。
6. 显式输入只有游戏库、没有 config/appinfo 的路径时，扫描仍可返回安装结果，两个新增字段为 null；不凭库位置推断它属于另一个 Steam 安装。

路径安全是本 feature **新增**的元数据读取约束，不能表述成 scanner 已有“拒绝链接”行为。现有 Steam 根/库路径会 canonicalize，`read_keyvalues` 使用普通 metadata/open，未拒绝链接：

- 保留现有安装根/库根规范化行为，允许 Steam 常见根别名（如 Linux 的 `~/.steam/steam` 符号链接）；规范化后的安装根是名称读取的锚点。
- 本 feature 新读的 `config/config.vdf` 与 `appcache/appinfo.vdf`，须拒绝锚点以下父目录及最终文件的 symlink/Windows reparse point，且只接受普通文件。
- 检查和读取须结合平台无跟随打开或等效的句柄校验，不能只在打开前做一次 metadata 检查。若无法确认读取满足约束，则跳过该来源并回退；config 被拒绝时仍可尝试适用注册表，appinfo 被拒绝时清空名称覆盖。
- 不扩展到重写既有 manifest/libraryfolders 扫描安全策略；新增读取只读、有界，不写 Steam 文件。

### 3.4 Windows 与 Decky 展示对接

- `packages/ui/src/index.ts`：`Install.game` 增加 `localized_name?: string | null`、`name_language?: string | null`；新增 `gameDisplayName(game)` 返回 `localized_name?.trim() || name`。
- `packages/ui/src/index.ts` 的 `filterInstalls` 才是 Windows 实际搜索入口：在现有 manifest 名与 AppID 基础上加入 `localized_name`，保留 trim、大小写处理和现有分类过滤。
- `apps/windows/src/main.tsx`：列表、详情标题与详情顶部等面向用户的游戏名改用 `gameDisplayName(game)`；搜索仍调用共享 `filterInstalls`。
- `apps/decky/src/steam.ts`：保留 `gamePresentation(appid, fallbackName)` 及 appStore 优先逻辑，不在函数内访问它未持有的 game 对象。
- `apps/decky/src/index.tsx`：将 `GameButton`、`QuickAccess` 搜索和详情页三处 `gamePresentation` 调用的第二个参数改为 `gameDisplayName(item.game)`（审阅时行号 144、173、260）。搜索继续包含 manifest 名、最终展示名与 AppID。
- Decky “未完成的操作”记录（审阅时 `index.tsx:424`）当前通过 instance_id 查找安装对象并展示 `game.name`；找到对象时也使用 `gamePresentation(appid, gameDisplayName(game)).name`，找不到时保留原有 `tx.instance_id` 回退。仅调整展示，不改变后端 discovery 的标题输入或匹配逻辑。

### 3.5 参考实现与 fixture

格式细节与来源见施工记录的调研与审阅修正段。外部格式参考：[Protontricks appinfo 容器实现](https://raw.githubusercontent.com/Matoking/protontricks/master/src/protontricks/steam.py)、[binary VDF 实现](https://raw.githubusercontent.com/Matoking/protontricks/master/src/protontricks/_vdf/__init__.py)。这些为上游分支链接，内容可能变动；P0 须记录采样版本、样本来源与校验值，并以独立 fixture 固化本轮契约。

本轮实现使用 [最小 fixture 与来源说明](../../../crates/core/tests/fixtures/localized-game-names/README.md) 固化契约：V29 为后续 900 条缓存快照的真实名称子树重构，V28 为合成编码；与初次审阅的 899 条快照区分记录。完整解析器与安全文件读取分别位于 [appinfo.rs](../../../crates/core/src/appinfo.rs) 和 [steam_metadata.rs](../../../crates/core/src/steam_metadata.rs)。

## 4. 边界与风险

- **私有格式变动**：appinfo.vdf 是 Steam 私有缓存，magic/条目/标签/路径都可能变。用版本分支 + 解码树查找 + 失败回退，绝不因格式变化让扫描失败。
- **格式精确性**：68 字节条目头和长度公式已由本机 V29 容器遍历核对；完整 payload 解码及 V28/V29 fixture 已自动化验证，实机与边界限制见施工记录。
- **资源消耗**：只扫描条目头、只解命中 AppID；按 3.2 限制文件、key 表、payload、字符串、节点与嵌套深度，避免文件未超限但解码分配或递归失控。
- **并发替换**：Steam 运行中可能重写文件；单句柄、失败重试一次，不假设文件在打开后不变。
- **语言覆盖不全**：目标语言名缺失时依次回退 `common.name`、manifest；默认名语言未知，不标为目标语言。
- **不联网 / 不刮削**：本 feature 不引入网络名称来源，也不改变 AppID 匹配。
- **多根/外置库**：按 3.3 保存所属关系，重叠库使用第一个根的元数据，不跨根兜底；缺元数据的显式库仍正常扫描。
- **安全**：对新增 config/appinfo 读取落实 3.3 的链接与普通文件约束，保留安装根别名规范化；现有 Steam 元数据读取并无同等拒绝链接保证。

## 5. 实施拆分与依赖

| 阶段 | 工作 | 完成门槛 |
| --- | --- | --- |
| P0 样本与契约 | 基于 V29 实测框架抽取可提交的最小多语言 fixture，补充 V28 与 config 缺键样本；记录来源/版本/校验值，固化长度、终止标记、标签与路径 | 完整 payload 可稳定解码出默认名与目标语言名；V28/V29 分别验证，资源阈值有样本依据 |
| P1 核心解析 | 新增 `appinfo.rs`；config + 根限定注册表回退；覆盖 V28/V29、名称回退矩阵、坏边界/标签/key 表、资源限额、并发重试与新增路径约束 | 字段契约与 2.1 一致；不 panic、可回退；命中样本得到“妃神会秘史” |
| P2 扫描集成 | `GameInstall` 加字段；保留根→库关联、根内 AppID 集合及名称映射；更新构造点与测试 | 外置库可继承所属根，多根/重叠库不串数据，扫描结果数与顺序不变；协议版本不变 |
| P3 前端对接 | 共享类型/helper/`filterInstalls`；Windows 展示；Decky 三处 fallback 调用及未完成操作记录 | 默认名/manifest 搜索仍可用，本地化搜索命中；Decky appStore 优先不变，无值/空白/异常分支正确 |
| P4 验收 | Windows typecheck/build、`cargo test` / clippy / fmt；真实 Steam 数据人工核对 | 见验收标准；AppID 匹配回归无变化 |

P0 → P1 → P2 → P3 → P4。文档与代码分开提交。

## 6. 验收标准

以下为实现验收契约，当前已执行与待验收项以 [施工记录](progress.md) 为准；早期容器核对不能替代自动化测试或客户端实机验收。

### 6.1 名称与语言契约

| 输入/条件 | `localized_name` | `name_language` | 展示 |
| --- | --- | --- | --- |
| V29 样本、语言 `schinese`、目标名称可用 | `妃神会秘史` | `schinese` | 妃神会秘史 |
| `english` 且存在非空 English 本地化字段 | 该字段的名称 | `english` | 对应名称 |
| 未命中目标语言字段（含 `english`/未知语言），但完整条目有 `common.name` | 默认名 | `null` | 默认名 |
| config 缺失/缺键/空值/读取失败，且根匹配的注册表为 `schinese` | 样本中文名 | `schinese` | 妃神会秘史 |
| config 与适用注册表都无语言（含 Linux 无语言） | 完整条目的默认名；也缺失则 `null` | `null` | 默认名或 manifest 名 |
| 当前根与注册表 `SteamPath` 不一致，config 无语言 | 当前根的默认名；也缺失则 `null` | `null` | 不借用其他根的目标语言名 |
| 目标语言的所有兼容路径均缺失/为空白，默认名可用 | 默认名 | `null` | 默认名 |
| 目标字段与默认名均不可用，或 appinfo 不可读/损坏 | `null` | `null` | manifest 名 |

同一契约覆盖 V28 fixture；存在多个兼容名称路径时优先 `name_localized.<语言>`，仅其不可用时尝试 `localization.<语言>`。

### 6.2 扫描、健壮性与安全

- **容器/条目**：tag 5 必须整条回退且不能保留此前读到的名称；验证 68 字节头、`entry_size - 60`、零 AppID 终止标记；覆盖负/越界 key 表偏移、`entry_size < 60`、长度溢出/跨界、截断、未知标签、越界 key 索引、缺 key 表、V27/未知 magic、无 appinfo。按 2.4 清空对应条目或本根覆盖，扫描仍成功、无 panic。
- **资源/并发**：各项限额分别覆盖等于上限与超过上限；深层嵌套和过多节点不得导致栈溢出或不受限分配。重试最多一次，部分结果不能混入新句柄读取的结果。
- **库归属**：主库与外置库都使用所属安装根缓存；多根拥有同 AppID 时互不串语言/名称；重叠库按第一根归属且只扫描一次；显式库缺元数据时两个新增字段为 null。安装结果数、instance_id 与顺序保持不变。
- **路径**：安装根别名规范化后仍可读取正常缓存；锚点以下 config/appcache 目录或最终文件为 symlink/reparse point、非普通文件或打开期间发生路径替换时，应拒绝该来源并回退。覆盖 Windows 与 Linux 的平台实现；不能以普通路径检查代替链接测试。
- **离线/只读**：新增名称读取无网络请求、不写 Steam 文件。
- **匹配不变**：目录匹配结果与仅按 AppID 时一致（`catalog::matches` 不改）。

### 6.3 客户端与协议

- **Windows**：列表与详情中面向用户的游戏标题遵循覆盖值→manifest；共享 `filterInstalls` 可搜索 manifest 名、名称覆盖及 AppID，分类过滤不变；执行 Windows typecheck/build。
- **Decky**：appStore 有非空名称时优先显示该值；overview 缺失、名称为空白或调用异常时，列表、详情、搜索与未完成操作记录均回退核心覆盖值，再回退 manifest；操作记录找不到安装对象时仍显示 instance_id；执行 Decky typecheck/测试。
- **兼容**：旧扫描 JSON 不含新增字段时新客户端仍可展示；新 JSON 被现有 Python/其他消费方读取时不失败；更新 `GameInstall` 构造点和共享 TS 类型，核对 schema 导出路径；协议版本不变。
- **实现检查与实机**：执行适用 Rust test/clippy/fmt 与前端检查；人工核对 Windows 真实 Steam 名称及 Decky appStore/fallback 两分支，逐项记录已执行与待验证。

## 7. 关联

- 来源发现 / 刮削核心化：[来源发现与订阅迁入共享 Rust 后端](../shared-source-discovery/design.md)（迁移方案，待实现）；本 feature 为其提供本地化名字段。
- Decky 展示：[Decky UX](../decky-ux/design.md)。
- Windows 一站式安装：[补丁一站式安装与流程精简](../simplified-install-flow/design.md)。
- 总体架构：[开发计划](../../development-plan.md)、[客户端指南](../../client-guide.md)。
