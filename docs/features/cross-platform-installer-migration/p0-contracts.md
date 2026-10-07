# P0 契约冻结设计（P0-Gate 1 通过，定稿）

日期：2026-10-08（Asia/Shanghai）。状态：**P0 契约 v0.5——P0-Gate 1 终审通过（attempt 3/3，2026-10-08），定稿冻结；Windows 支持版本已确认：仅 Win11 x64（2026-10-08）**。已回填：lib-2 LZMA 依赖调研（§8）、P0-B Windows 探针实测（§6.2/§8，证据见 [p0-sandbox-probe-windows.md](p0-sandbox-probe-windows.md)）。

关联：[design.md](design.md)、[progress.md](progress.md)。取代 [shared-source-discovery/p0-p1-rust-design.md](../shared-source-discovery/p0-p1-rust-design.md) §5.1 的来源编码/ID 契约。开发阶段接受旧缓存 breaking change，不做旧格式兼容/回退。

> 决策格式：【冻结值 / 理由 / 被否替代 / 迁移动作 / 验证方式】。
> 标注约定：**待实测**（需设备/样本证据）、**待调研**（选型/规范未定，由 lib-2 等回填）。

---

## 0. 全局版本与命名基线

| 维度 | 冻结值 | 说明 |
| --- | --- | --- |
| 协议版本 | `protocol_version = 3`（由现行 2 升级） | 核心与两端宿主同版本发布，旧版本明确拒绝；不设兼容路由（设计 §6）。 |
| 来源编码版本 | `SOURCES_ENCODING_VERSION = 1` | 新增常量；取代 `python_json` 的字节契约。 |
| 安装选择/内容编码版本 | `INSTALLER_SELECTION_VERSION = 1` / `INSTALLER_CONTENT_VERSION = 1` | 与 adapter/policy/report 版本解耦。 |
| 状态 schema | `installer/rust-v1/`，`STORE_SCHEMA_VERSION = 1` | 与 `sources/rust-v1/`（`STORE_SCHEMA_VERSION = 1`）命名空间平行、语义独立。 |
| adapter / policy / report 版本 | `adapter_version`、`policy_schema_version`、`report_schema_version` 各自独立字段 | 见 §2；任何一项变化只失效依赖它的绑定，不并入来源内容更新判断。 |

**统一理由**：设计 §5.1 要求“版本化并失效受影响 cache/discovery/index”，因此版本号必须是显式常量而非隐式哈希变化。**被否替代**：不沿用 `PARSER_VERSION`/`QUERY_VERSION` 兼任编码版本（会把来源解析版本与编码格式混为一谈）。

---

## 1. 来源新确定式编码（取代 `canonical.rs::python_json`）

### 1.1 编码原语

**【冻结值】**
- 新增 `crates/core/src/sources/canonical_v2.rs`（或就地重写 `canonical.rs`），提供唯一编码入口 `canonical_bytes<T: serde::Serialize>(&T) -> Vec<u8>`（字段集合由泛型 `T` 的显式 `Serialize` 字段决定）。
- 序列化格式改用 **确定性 JSON**：对象键按 **UTF-8 字节序（等价 Unicode 码点序）升序**；分隔符无空格（`,` / `:`）；字符串按 **原样 UTF-8**（`ensure_ascii = false` 恒成立，不再有 `sort_keys`/`ensure_ascii` 参数），**仅转义 JSON 必需字符（引号、反斜杠、控制字符）**；整数十进制；**禁止浮点**（出现即 `invalid_canonical_value`）。
- 字段集合以 **结构体 `Serialize` 的显式字段**为准（`SourceCandidate`/`SourceRule`/query 结构），不依赖 `serde_json::Value` 的运行时键集。
- 字符串值 **不做 Unicode 规范化**（保留原始码点）；匹配用的 NFKC/casefold 仍只发生在 `validate::normal`/`discovery_normal`，二者互不混用。
- URL 规范化保留 `canonical_url(u) = urlunsplit(scheme_lower, host_lower, path, "", "")`（丢 query/fragment、host 小写、端口**原样保留**——现行行为，P1-1 实现确认；若引入端口折叠须作为编码行为变更单独 bump 版本）。**path 的百分号编码归一化（大小写、未编码保留字符）待调研**；冻结前以“原样 path”为准并在 fixture 注明。**该 URL path 规范化行为变化同样须 bump 编码版本**。

**【理由】** 现行 `canonical.rs` 的 `python_json`/`ensure_ascii`/`sort_keys` 存在目的只是逐字节复刻 Python；既然设计明确“不再要求与 Python 字节/ID 相等”，确定性 JSON 是更小、更可证的契约。**被否替代**：(a) 继续维护 `python_json`（维护成本高、无收益）；(b) 直接 `serde_json::to_string`（键序依赖 Map 实现，跨版本不稳，且保留空格）。

**【迁移动作】**
- 删除 `canonical.rs::python_json`、`candidate_id`/`fingerprint`/`query_key` 的 Python 语义；调用点改造：`identity.rs`（candidate/identity 建哈希）、`session.rs:693-748`、`validate.rs:531`。
- `dedupe_json`（`identity.rs`）改用 `canonical_bytes` 去重键。
- 黄金 fixture `crates/core/tests/fixtures/sources/identity-golden.json` 重写为“新 Rust 编码黄金”，删除全部 Python payload/byte 断言（`canonical.rs` 内对应测试）。
- CLI 测试内联的 `query_key`（`runtime.rs`）同步。

**【验证方式】** `cargo test -p steam-patch-core` 新增 `canonical_deterministic_*`：同输入跨进程/跨次运行字节一致；中文/Unicode/键顺序/URL 规范化 fixture；Windows 与 Deck 产物字节一致（同一 Rust 二进制 + 相同 fixture 哈希比对）。**待实测**：Windows/SteamOS 双端产物哈希在真实构建中一致（P1 gate）。

### 1.2 candidate / source_record / fingerprint / query_key / rule 指纹

**【冻结值】**

| 名称 | 形式 | 输入字段（编码后取 SHA-256） |
| --- | --- | --- |
| `candidate_id` | `candidate-<hex[:24]>` | 有序数组 `[source_id, canonical_url(source_page), variant, languages, version_identity]`（保持现行语义，仅换编码原语） |
| `source_record_id` | `canonical_url(source_page)` 原值 | 不哈希（保持现行字符串 ID） |
| `fingerprint` | `<hex[64]>` | 候选结构体去掉 `fingerprint`/`observed_at`/`cache_status` 后整体编码 |
| `query_key` | `<hex[64]>` | `{appid, name, name_hints, preferred_language, legacy_match, source_context, parser_version, query_version}`（`session.rs:693-741` 字段集不变）+ 新增 `encoding_version` |
| `rule_fingerprint` | `<hex[64]>` | 规范化后 `SourceRule` 整体编码（`session.rs:744-748`） |

- `query_key` **必须纳入 `encoding_version`**；`encoding_version` 须持久化进 discovery/index 记录并由 `discovery_fresh`/`shared_index` 比对（§1.3），否则 confirmed discovery 与 discovery-index 不因 query_key 变化而失效。
- `fingerprint` 需从候选去掉 `fingerprint`/`observed_at`/`cache_status` 三个字段，泛型入口下需定义影子结构体（或经 `serde_json::Value` 中间态）；P1 实现选择其一并在 fixture 固化。
- 禁用浮点：`observed_at` 只在 fingerprint 外，不参与。

**【理由】** 字段集合已经在 `canonical.rs`/`session.rs`/`identity.rs` 中被 golden 固化，语义正确，仅“编码字节”需要替换，风险最小。**被否替代**：重设字段集（会额外引入来源语义回归，超出本期范围）。

**【迁移动作】** `identity.rs`、`session.rs`、`validate.rs` 的调用点按 §1.1 改造；`valid_candidate_id`（`candidate-` + 24 位十六进制）不变。

**【验证方式】** 单元：确定性 + 中文/emoji/组合字符；同一候选字段顺序不同、JSON 解析顺序不同 → ID 相同。集成：`query_key` 变化使 `needs_refresh` 重新查询。

### 1.3 旧缓存失效与 ID 形式

**【冻结值】** 编码/查询契约变化视为破坏性升级：**不写映射、不读旧 ID**。旧 `discovery/`、`discovery-index/`、`cache/`（受 `query_key`/`parser_version` 影响者；编码契约变化经 `encoding_version` 失效）整体视为 stale，首次查询时重写；`settings.json`（用户规则/订阅/revision）与 `status.json` **保留**。

**【理由】** 设计 §5.1/§7.1 明确“不维护旧 ID 映射或旧缓存导入”，且失效来源 cache 不能扩大为设置重置。**被否替代**：版本号只在 `schema.json` 上（无法只失效 discovery 而保留 cache 语义）。

**【迁移动作】** 在 `sources/mod.rs` 增加 `SOURCES_ENCODING_VERSION`：`query_key` 纳入该值，并在 `DiscoverySuccess`/`DiscoveryAttempt`/`DiscoveryIndex` 记录中**持久化 `encoding_version` 字段**（反序列化时缺失按 `0` 处理）；`registry.rs` 的 `discovery_fresh`（confirmed discovery 闸门）与 `shared_index`（discovery-index 闸门）比对该字段，不符即失效（旧记录无字段 = 0 ≠ 当前 1 → 自动失效重查），使编码版本变化同时失效 confirmed discovery 与 discovery-index；`store.rs` 不做数据迁移。旧缓存一律**不得产生安装许可**。`import_python_state`（`store.rs:395`）随 P1 删除。

**【验证方式】** 注入旧 `query_key`/旧编码版本的已 **confirmed** discovery 与 discovery-index → 均不被命中且重新查询（不依赖“仅靠 key 不匹配自然过期”）；`settings.json` revision/规则/订阅前后一致。

---

## 2. 安装选择 ID / 内容 ID / canonical 编码与版本化

> `INSTALLER_SELECTION_VERSION` / `INSTALLER_CONTENT_VERSION` 随选择/内容记录与授权绑定持久化；**不进入 ID 哈希**（分析器升级不误判内容更新）。

### 2.1 选择 ID（candidate 级，不可 apply）

**【冻结值】** `selection_id = "exe-selection-" + hex(sha256(canonical(candidate_binding)))[:24] + "-" + appid`。
`candidate_binding` 字段集（冻结，来自 `exe_profiles.py`）：`source_id, source_record_id, game_appid, languages, variant, version_identity, source_page, requires_dlc, excludes_dlc, mirrors`。编码改用 §1 的确定性编码（不再用 `exe_profiles.py` 的 `ensure_ascii=True, sort_keys=True` 紧凑 JSON）。
- **选择 ID 只代表候选，不授予 apply 权限**（设计 §5.1）。

**【理由】** 与现行 `exe_generic.py` 语义一致，只是把 Python 哈希换成新 Rust 编码。**被否替代**：把选择 ID 与内容 ID 合并（会丢失“候选可变、内容不可变”的分离，重装判断退化）。

**【迁移动作】** Rust 新增 `installer/model.rs::selection_id(candidate)`；`exe_profiles.py` 的 `candidate_binding`/`selection_id` 随 P5 删除。

**【验证方式】** 稳定输入 → 稳定 ID；mirrors/languages/dlc 任一变化 → ID 变化。

### 2.2 内容 ID（内容级，可重装判断）

**【冻结值】** `content_id = "exe-content-" + hex(sha256(canonical(identity)))[:24] + "-" + appid`。
`identity` 字段集（来自 `exe_generic.py:195`）：`appid, source_id, source_record_id, language, variant, exe_sha256, metadata_sha256, payload`。`payload` = 有序 `[{path,size,sha256}]`。
- **终态 release_id 显式返回**：EXE 动态路径的最终 `release_id = content_id`；签名 ZIP 路径的 `release_id = catalog.release_id`（`patchmoe-<appid>` / `source-<id>-<appid>`）。UI 只持有选择 ID，终态由核心回填。

**【理由】** 设计 §5.1“绑定来源、AppID、语言/变体、EXE SHA、元数据及完整载荷/选择摘要；终态显式返回最终 release_id”。**被否替代**：内容 ID 纳入 `source_page`/`mirrors`（会因镜像/页面文案变化误判内容更新，与现行实现不符）。

**【迁移动作】** `installer/model.rs::content_id(profile)`；`exe_generic.py` 删除；授权收据（`exe-policy.json` schema 2）迁为 Rust 授权类型——**授权字段集由 P1 冻结（以 [design.md](design.md) §5.2 完整绑定为准），P0 不枚举**。

**【验证方式】** 同 EXE 换 URL → content_id 不变（内容更新判据独立）；载荷字节变化 → content_id 变化。

### 2.3 adapter / policy / report 版本独立

**【冻结值】** 三个独立整型，进入授权绑定但 **不进入 content_id**：`adapter_version`（SF 解析布局）、`policy_schema_version`、`report_schema_version`。

**【理由】** §5.1“adapter/policy/report 版本独立绑定分析，不混入内容更新判断”。**被否替代**：并入 content_id（分析器升级会被误判为“内容更新”，导致无法同内容重装）。

**【迁移动作】** 授权类型新增三字段；`resolve`/`validate_apply` 重核它们而非 content_id。

**【验证方式】** 仅升 `report_schema_version` → content_id 不变、旧授权失效、可重新分析后重装。

### 2.4 ZIP 同 ID 的 URL/字节变化约束

**【冻结值】** 签名 ZIP 的 `release_id` 由签名 catalog 决定；其 **内容身份**沿用并冻结 `install::recipe_identity`：`(appid, release_id, language, artifact_sha256, [(source,destination,installed_sha256)])`。同一 `release_id` 但下载字节/URL 变化时：下载流式校验 `artifact_sha256` 必须匹配，不匹配即 `download_hash_mismatch`，**不得以版本字符串或镜像替代内容身份**（设计 §4.1/§5.1）。

**【理由】** 现行 Rust 事务已把内容身份固化为 `recipe_identity`。**被否替代**：以 URL/ETag 作为身份（可被同 URL 不同字节绕过）。

**【迁移动作】** `download.rs` 扩展动态首次 SHA 建立内容身份、ETag/Last-Modified 复核、镜像各自独立分析；`recipe_identity` 保留并纳入新授权。

**【验证方式】** 同 URL 不同字节 / 同 release_id 不同 artifact_sha → 阻断（fixture）。

---

## 3. 新状态 schema `installer/rust-v1/`

### 3.1 目录/文件布局

**【冻结值】** 在 `state_dir/` 下：

```text
installer/rust-v1/
├── schema.json                 # 完成标记（最后写）
├── journal.lock                # 跨进程实例锁
├── journal.sqlite              # 事务日志（DELETE + synchronous=FULL）
├── requests.sqlite             # 新请求幂等账本（替代旧顶层 install_requests.sqlite 的读取）
├── jobs/<uuid>/{<i>.stage,<i>.backup,revoked,exe-policy.json,owner.json}
├── content/<content_id>/       # 归一新 ZIP / 载荷缓存（可清）
├── receipts/<instance_hash>/   # 成功收据（受保护）
├── downloads/                  # 动态下载缓存 <sha256>.zip / .partial（可清）
└── reports/<content_id>/       # 分析报告（可清/留档）
```

与 `state_dir` 顶层关系：`installer/rust-v1/` 与 `sources/rust-v1/`、`trust.sqlite`、`install_requests.sqlite` 平级且互不嵌套；`publisher/`、旧 `jobs/`、旧 `downloads/`、旧 `install_requests.sqlite` 只保留为不读取的旧目录。

- `requests.sqlite` 为新请求**幂等账本**，承接 §3.3/请求幂等语义，**替代旧顶层 `install_requests.sqlite` 的读取**（旧文件只保留、不读取）。
- `instance_hash` **冻结为 `sha256(instance_id)` 前 16 hex**；成功收据必须绑定 `content_id` + transaction id；**完整收据 schema 由 P1 冻结（以 design §5.2 完整绑定为准）**。

**【理由】** 设计 §7.1/§7.2 要求统一 schema；沿用 `install.rs::Store` 的 `jobs/<uuid>/<i>.<kind>` 便于复用恢复算法。**被否替代**：沿用 `publisher/<release_id>/<instance_hash>/`（设计明确不复用）。

**【迁移动作】**
- `install.rs::Store` 新增可配置 root（`installer/rust-v1`），或抽 `StorePaths`；`journal.sqlite` 与 `jobs/` 迁移到新根。
- 新增 `installer/store.rs` 管理 `schema.json`/`content`/`receipts`/`reports`。
- 删除 `publisher.py` 的 `state_dir/publisher/` 写入（随 P5）。

**【验证方式】** 新目录结构快照测试；旧目录存在不影响新引擎启动。

### 3.2 与 journal.sqlite 的关系

**【冻结值】** `journal.sqlite` 仍是唯一事务真相源；`jobs/<uuid>/` 中 `.stage/.backup/revoked` 是受保护恢复材料；`content/`、`downloads/`、`reports/` 为可清缓存。`exe-policy.json`（授权收据）受保护。

**【理由】** 现行 `apply`/`restore` 依赖 `.stage/.backup`；`revoked` 是持久撤销标记。**被否替代**：把授权收据放可清目录（会绕过恢复与重装判断）。

**【迁移动作】** `cleanup_terminal` 只清 `.stage/.backup`；缓存清理接口只允许 `content/`/`downloads/`/`reports/`（设计 §6 清理表）。

**【验证方式】** 清缓存后 `journal.sqlite`、`receipts/`、`.stage/.backup`、`revoked` 仍在；active lease 不被清。

### 3.3 原子初始化与中断幂等

**【冻结值】** 首次初始化：在 `installer/rust-v1/.init-<uuid>/` 建 staging → 写 `journal.sqlite`/`jobs`/`schema.tmp` → `sync_all` 文件与目录 → 原子 `rename` 发布 → **最后**写 `schema.json` 完成标记。`schema.json` 存在即跳过；中途崩溃则以临时目录/缺失标记推断未完成并重试，**幂等**。

**【理由】** 复用 `store.rs::write_atomic`/`publish_tree` 与 `import_python_state` 的“标记最后写”模式。**被否替代**：直接 `create_dir_all`（无法区分半初始化）。

**【迁移动作】** 抽出 `installer/store.rs::init_atomic()`；Windows 复用 `store.rs::write_atomic` 的 rename 重试与 link 检查。

**【验证方式】** 注入 staging 残留/半写 → 重开成功且不重复；`schema.json` 存在时不动数据。

---

## 4. 锁顺序与互斥

### 4.1 冻结的总获取顺序

**【冻结值】** 任意写路径按序获取，**禁止逆序**：

1. `installer/rust-v1/journal.lock`（`fs2` try_lock 独占）——**跨进程实例锁**，整个写事务期间持有。
2. `<install_path>/.steam-patch-manager.lock`（每游戏锁）。
3. 任务租约（§5）为 RPC/内存层，不是文件锁，最后获取、最先释放。

- `sources/rust-v1/settings.lock` 为**独立短锁，不参与上述 1/2/3 排序**，**绝不在持有 `journal.lock`/游戏锁时获取**（网络/解析期间不得持有 `journal.lock`，获取 `settings.lock` 时也不得持有 1/2）。
- `validate_apply` 与任务启动**各做一次原子只读比对** settings revision（同进程可用内存 revision，跨进程以磁盘原子读为准；`SourceStore::with_short_lock` 不取 `journal.lock`）。**跨进程可见性边界**：来源设置的跨进程改动，仅在下次重核/任务启动读取最新 revision 时可见，重核矩阵不得假设同事务内即时可见。

- **网络/解析/来源任务不持 journal 写锁**（设计 §6.1）。
- `scan`/`list_transactions` **只读协调**：走只读快照（`rusqlite` `SQLITE_OPEN_READ_ONLY` + 不取 `journal.lock` 独占），确保与恢复/提交并发时读到一致的事务状态；真实安装/恢复 busy 仍上报。

**【理由】** 现行 `Store::open` 对含 `list_transactions` 的**所有**调用都取 `journal.lock` 独占，会让只读查询与写互斥并误报 busy；且旧 engine-switch 的双引擎已删。**被否替代**：(a) 只靠 UI busy 标志（不能证互斥）；(b) 每路由各自 journal.lock（不同任务仍会同游戏并发写）。

**【迁移动作】** 拆 `Store::open_rw`（取 1）与 `Store::open_ro`（只读）；`list_transactions`/`recover` 的读取路径改只读、`recover` 的写路径仍取 1。冻结同 state 排队、不同 state 独立的现行语义（设计 §6.1）。

**【验证方式】** 并发用例：两写者互斥、写者 + 只读者不误报、恢复中取消等待可取消、不同 `state_dir` 独立。

### 4.2 两代引擎切换规则

**【冻结值】** 切换期旧引擎**停止发起新任务**；`applying/rolling_back/needs_recovery` 由旧引擎恢复完毕后，新引擎才建立 `installer/rust-v1/`。**同一实例同一时刻只有一个写引擎**；不允许双写 state 或游戏文件。跨进程实例锁冻结为 `journal.lock`（**待调研**：是否需要独立 `engine.lock` + generation 标记以在旧 journal 收尾前显式阻断新引擎启动；倾向不新增文件，用旧 journal 存在性 + `schema.json` 缺失判定）。

**【理由】** 设计 §7.1/§6.1。**被否替代**：双写/双读过渡（用户明确不做兼容）。

**【迁移动作】** 切换前收尾由旧 CLI 执行；`install.rs::migrate_state` 与 `main.rs:1010` setup 调用在 P1 决定去留（倾向删除，随新 schema）。

**【验证方式】** 存在未完成旧事务 → 新引擎阻断；收尾后新 schema 正常。

---

## 5. 取消 / 租约 / 关闭

### 5.1 租约 TTL 与持久撤销

**【冻结值】**
- **前台租约 TTL = 15 秒**（Decky 现行 `tasks.py`），宿主转发续租（`renew`），核心掌握真实授权/取消。
- **`PLAN_TTL ≤ 900 秒`**，不放宽；授权收据 `expires_at` 取自 core plan。
- **持久撤销**优先于停止获取/worker：先写 `jobs/<uuid>/revoked` 并 `sync_all`，再取消网络/worker；进入写入后按 journal 恢复，**完成恢复才确认取消**。
- **提交成功优先于迟到取消**。

**【理由】** 设计 §6“取消/退出/EOF/租约过期先持久撤销，再停止获取/worker；完成恢复才确认取消”“提交成功优先”。**被否替代**：仅内存标志（崩溃/新进程不生效）。

**【迁移动作】** 租约 deadline 落 `jobs/<uuid>/lease.json` 或 Job 文档；续租更新；过期即 `revoke`。保留 `PLAN_TTL_SECONDS`。

**【验证方式】** 逐阶段取消（download/analyze/convert/plan/apply）；撤销写失败/`discard` 失败的 `needs_recovery`；迟到取消不覆盖 committed。

### 5.2 EOF / 父退出 / needs_recovery

**【冻结值】** stdin EOF 或父进程退出 → 取消当前任务并回收 worker 进程组（Decky `exe_worker.py` 的 SIGTERM→1s→SIGKILL；Windows Job `KILL_ON_JOB_CLOSE`）。恢复失败置 `needs_recovery`，**不伪装 cancelled/completed**。

**【理由】** 设计 §6。**被否替代**：失败降级为 `failed`（丢失恢复材料语义）。

**【迁移动作】** CLI `serve_task` 保持单任务 + 取消；worker 启动器双平台回收。

**【验证方式】** 父退出/信号/超时下进程树无残留；`needs_recovery` 上报。

---

## 6. worker schema / 等价内存限额

### 6.1 协议（analyze / convert）

**【冻结值】** 固定 action、描述符 JSON over stdin，响应 JSON over stdout（现行 `sf95_worker.py` 语义迁 Rust，字段不变）：

| action | 请求描述符键 | 响应/产物 |
| --- | --- | --- |
| `analyze` | `{action, exe_sha256, exe_size}` | `{schema_version:1, action, exe_sha256, report, installable:false}`，`report.capability == discover_only` |
| `analyze_behavior` | `{action, exe_sha256, exe_size}` | 同上，`report.capability == behavior_supported`，`report.behavior.coverage.complete == true`，`behavior_sha256 == digest(behavior)` |
| `convert_behavior` | `{action, exe_sha256, exe_size, metadata_sha256, payload:[{path,size,sha256}], behavior_sha256}` | 写 `/output/normalized.zip`，返回 `{extractor, exe_sha256, metadata_sha256, artifact_sha256, artifact_size, files}` |

- 描述符/响应 **≤16 MiB**；worker 无任意 command/脚本/路径参数（设计 §6）。
- 转换前先完成全部路径/CRC/manifest 校验，**再**首次写入独占输出目录；写 `*.partial` 后原子 `rename`。

**【理由】** 现行协议已在 `sf95_worker.py`、`exe_worker.py` 固化且被测试覆盖。**被否替代**：重设 action（无收益、破坏 fixture）。

**【迁移动作】** `crates/installer-worker/` 实现同 schema；`crates/installer-format/` 提供 `audit`/`lower`/`Decoder`。

**【验证方式】** 与 Python 黄金报告语义/产物摘要一致（§9）；未知动作 fail-closed。

### 6.2 限额映射

**【冻结值】**

| 项 | 冻结值 | 现行依据 |
| --- | --- | --- |
| 包/单文件/总展开 | ≤2 GiB / ≤2 GiB / ≤2 GiB | `sf95_audit.py` |
| publisher ZIP | 包 ≤8 GiB、展开 ≤16 GiB、单文件 ≤4 GiB、8,192 条目、压缩比 200 | 沿用 design §8 |
| 元数据/内部报告 | ≤16 MiB | `sf95_audit.py`，`exe_worker.py` |
| 条目 | 8192 | `sf95_audit.py` |
| LZMA2 dict | ≤128 MiB | `sf95_audit.py` |
| Lua | ≤100,000 token / 96 层 / ≤1 MiB | `sf9_lua.py` |
| 条件 IR | ≤8,192 节点 | `exe_generic.py` |
| 地址空间 | 3 GiB | `RLIMIT_AS`，`sf95_worker.py` |
| CPU | 300 s | `RLIMIT_CPU`，`sf95_worker.py` |
| 超时 | analyze 180 s / analyze_behavior 240 s / convert 480 s | `exe_worker.py` |
| 确定性 ZIP | `ZIP_STORED`；`date_time=(1980,1,1,0,0,0)`；`create_system=3`；`external_attr=0o100644<<16` | `sf95_worker.py` |

- **Windows 等价限额（P0-B 已实测，证据见 [p0-sandbox-probe-windows.md](p0-sandbox-probe-windows.md)）**：
  - `JOB_OBJECT_LIMIT_PROCESS_MEMORY` 生效（128 MiB 限额下 256 MiB `VirtualAlloc` 失败 `ERROR_COMMITMENT_LIMIT(1455)`；1 GiB 对照通过）→ 正式值取 3 GiB。**语义差异**：该限制约束 *commit*，与 Linux `RLIMIT_AS`（地址空间）不等价；worker 以 mmap 读入包时文件映射在 Windows 不计 commit，故应同时设置 `JOB_MEMORY` 纵深。
  - **wall-clock 超时（analyze 180 s / analyze_behavior 240 s / convert 480 s）是硬时限主手段**；`JOB_OBJECT_LIMIT_JOB_TIME`（`PerJobUserTimeLimit`）为 **CPU 兜底**，粒度粗（实测 1 s 限额下 10 s busy 约 5.4 s（`wall_ms=5380`）才终止 `STATUS_QUOTA_EXCEEDED(0xC0000044)`）→ 300 s 映射为 PerJobUserTimeLimit；**不得宣称精确 CPU 计费**。
  - `JobObjectCpuRateControlInformation`（rate hard cap）在宿主已处于父 Job 的嵌套场景**不生效**（实测未限速）→ 仅作为非嵌套加固项，**不得作为唯一 CPU 证明**；实现须探测宿主 in_job 并如实上报降级。
  - 探针另证：LPAC worker **不能创建子进程**（`ACCESS_DENIED(5)`）→ worker 必须单进程结构，多阶段由核心多次单进程调用；环境块必须包含 `SystemDrive/USERPROFILE/LOCALAPPDATA` 等变量否则 `ERROR_ENVVAR_NOT_FOUND(203)`；**生产 worker 二进制须置于 installer 自有、可授 DACL 的目录（app_data 类），不得依赖 `Program Files` 写权限**（`target\debug` 无 `WRITE_DAC`）；LPAC 注册表可读面比预期更严（普通 HKLM 读也被拒），生产前需按实际键集核对。

**【理由】** 设计 §8“沿用现有限额，不因 Rust 移植自动放宽”；§4.3 要求 Windows 建立 Job/限制后才 ResumeThread。**被否替代**：Windows 用无限制子进程（不满足隔离门槛）。

**【迁移动作】** `crates/installer-worker/` 双平台启动器：Linux `bwrap` argv 固定；Windows LPAC（AppContainer + `PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY=0x2000F` + Job Object + `CREATE_SUSPENDED`→`ResumeThread`）。

**【验证方式】** 超限/超时/超输出/内存上限用例；进程树回收；无窗口。Linux 与 Windows 分别实测（P0-B / P0-D）。

---

## 7. 规则 schema（仅接受 schema 2）

**【冻结值】**
- `RulesConfig.schema_version` **必须 == 2**；`1` 或其它值 → `invalid_sources_config`（删除 `mod.rs` 的 `1|2` 双接受与相关测试）。
- **必填字段**：`id`、`name`、`adapter`、`enabled`、`pages`、`page_hosts`。
- **正常可选字段（带合法默认值，保留）**：`role`（默认 `publisher`）、`discovery_only`（默认 `false`）、`priority`（默认 `100`，范围 0–1000）、`download_hosts`（`discovery_only=true` 时默认 `[]`；否则必填 1–8）。
- **条件字段**：`columns`（`adapter=="table_html"` 时必填）、`table_id`（可选）、`language`、`url`（可选）。
- **兼容残留（拒绝/删除）**：`root_layout` 作为可被用户指定的权威字段——**publisher 规则必须携带 `root_layout == "game_root"`；`discovery_only` 规则必须缺失**（`validate.rs` 现行强制）。P1 如删除该字段，须提供等价的默认/派生校验，**不得放宽现有拒绝语义**；P1 用真实 `settings.json.added` 抽样确认兼容性（**不作为冻结前提**）。

**【理由】** 设计 §7.3 要求仅接受当前 schema 并区分正常可选与兼容残留；现行 `validate_rule` 已把语义固化，只是文档未冻结。**被否替代**：彻底删除 `role`/`discovery_only`/`priority` 默认值（会破坏现有 schema 2 用户规则的“正常可选”语义）。

**【迁移动作】** `mod.rs` 收紧为 `schema_version == 2`；删除 schema1 测试；新增“必填缺失/默认值/越界/拒绝 schema 1”用例。

**【验证方式】** 现有 `DEFAULT_RULES_JSON` 与用户 `settings.json` 通过；schema 1 明确拒绝；旧规则不产生授权。

---

## 8. 依赖 / 许可清单（P1/P2）

**【冻结值/要求清单】**

| 能力 | 依赖要求 | 状态 |
| --- | --- | --- |
| LZMA2 raw 解码 | 需支持 `FORMAT_RAW` + `FILTER_LZMA2` + 显式 `dict_size`（≤128 MiB），流式/64 KiB 分块，`eof`/trailing 校验，与 `sf95_audit.py` Decoder 逐字节一致 | **已调研（lib-2）：选 `lzma-rust2` 0.21**（纯 Rust、Apache-2.0、无 C/无 build.rs、dict 外部化匹配 sf9；`default-features=false, features=["std"]`）。备选 `liblzma` 0.4.8（需 C 工具链、SBOM 补记 0BSD）；不用 `xz2`（2022 停更） |
| 确定性 ZIP 写 | `zip` 6（已用）需支持 `ZIP_STORED` + 手动 `ZipInfo` 头字段 | 已具备 |
| PE/overlay/framing | 自研（现行 `sf95_audit.py` 无第三方 PE 依赖），**零新增依赖** | 已定 |
| Windows 隔离 | `windows` crate 0.62 + LPAC（`SECURITY_CAPABILITIES`+Job Object） | 已定（lib-1） |
| 哈希/序列化 | `sha2`/`serde`/`serde_json`（已有） | 已定 |
| Unicode | 新增编码不做规范化；匹配继续用 `unicode-normalization`/`caseless`/`unicode-general-category`（已有） | 已定 |

- **SBOM/许可**：`lzma-rust2` 为 Apache-2.0 单组件，`build-sbom.py` 经 `cargo metadata` 自动收录，无需人工补录；保留 `licenses/sfextract-MIT.txt` 与 commit 记录。备选 liblzma 路线需手工登记 vendored C 的 0BSD 与版本。静态链接对便携包体积的影响待实测（P1 基线记录）。
- **LZMA2 选型要点（lib-2）**：dict_size 必须由调用方外部传入（`Lzma2Reader::new_mem_limit(reader, dict_size, mem_limit_kb, None)`），sf9 流内不含 dict 尺寸，此点排除 `lzma-rs`；prop→dict 解析与拒绝顺序（先 `prop<=40` 再 `≤128 MiB`，`prop=40` 的 4 GiB 不得进入分配）需在 Rust 侧复刻；输出上限由调用方封顶（`expected+1`），不依赖库；`Ok(0)` = 结束标记，截断/杂散字节（trailing）分类待实测；内存以 `mem_limit_kb` + 外层 3 GiB 限额双重约束；建议保持 `optimization` 关闭（纯 safe Rust）。
- **便携体积/静态链接**：Windows 便携包仍不含 Python/WSL；Rust worker 与解析依赖静态链接，体积上限在 P1 基线中登记（**待实测**）。
- **Windows 支持版本冻结（用户已确认 2026-10-08）**：LPAC 需 Win10 1703+（build 15063）。**冻结值：仅承诺 Windows 11 x64**（LPAC + Job Object），Win10 1703–10 不承诺。P0-B 实测环境为**非标准 build**（`CurrentBuildNumber=26300` / `ProductName "Windows 10 Pro"`），**不足以宣称 Win11 零售机验证**；标准零售机版本矩阵列为 **P4 发布条件**（见证据文档 §7）。

**【理由】** 设计 §4.3/§7.2/§8。**被否替代**：运行时从 PATH 找解压工具或联网下载运行时（设计禁止）。

**【迁移动作】** 选型落定后写入 `Cargo.toml` 与 SBOM；`crates/installer-format` 不依赖宿主/网络/游戏目录（设计 §4）。

**【验证方式】** 解码器对黄金 LZMA2 fixture 与 Python 产物哈希一致；SBOM 含新依赖；便携包体积记录。

---

## 9. 黄金 fixture 清单与存放

**【冻结值】** P0 冻结以下 fixture（文本/JSON 入库；**二进制样本不入库**，测试内合成）：

| # | fixture | 内容 | 存放/形式 |
| --- | --- | --- | --- |
| F1 | 来源编码黄金 | 中文/Unicode/键顺序/URL 规范化、`candidate_id`/`fingerprint`/`query_key`/`rule_fingerprint` 期望值（新 Rust 生成，非 Python 字节） | `crates/core/tests/fixtures/sources/encoding-golden.json`（重写现 `identity-golden.json`） |
| F2 | ZIP 映射/载荷 | XP3 包装映射、重复/大小写/Unicode/目录冲突、extra 字段、压缩比、原文件摘要 | `crates/core/tests/fixtures/zip/*.json` + 合成 ZIP |
| F3 | SF IR/行为黄金 | `sf9-behavior-v1` IR、`audit` 报告 schema v1、coverage/terms/condition、worker 协议信封 fixture（`schema_version`/`action`/`installable`/`capability`） | `crates/installer-format/tests/fixtures/sf/*.json`（对 `scripts/test_sf9_behavior.py`、`test_sf95_*` 的脱敏结果） |
| F4 | 授权绑定 | `exe-policy` schema 2 各绑定字段、`validate_apply` 重核矩阵 | `crates/core/tests/fixtures/installer/authorization.json` |
| F5 | 恢复/事务 | journal 状态机、`revoked`、`needs_recovery`、同内容重装 | 复用 `crates/core/tests/install.rs` 合成 + fixture 断言 |
| F6 | 选择/内容 ID | `candidate_binding`/`identity` 字段变化 → ID 变化矩阵 | `crates/core/tests/fixtures/installer/ids.json` |

- **合成策略**：所有 EXE/ZIP/XP3/ACF 由测试在临时目录生成（沿用 `test_publisher.py`/`test_exe_*.py` 的合成方式）；无提交的二进制样本。
- 三条旧 profile 对应包只作 **F3 通用行为/载荷回归**，不迁移 profile 注册表。**当前仓库不含这些旧 profile 包的二进制样本**；P2 前由用户提供，否则降级为合成 fixture，该部分保持**待验证**。

**【理由】** 设计 §8/§9“冻结最小脱敏 fixture/黄金报告”；§9.1 明确“新 ID 验证 Rust 确定性和跨平台一致，不要求与 Python 旧 ID 相等”。**被否替代**：提交真实样本（体积/许可/隐私风险）。

**【迁移动作】** 重写 `identity-golden.json`；保留 Python 黄金工具为开发对照但**不进生产包**（设计 §7.2）。

**【验证方式】** fixture 驱动单测；`cargo test`/`clippy`/`fmt`；两端宿主 typecheck/build；SBOM/包检查。

---

## 10. 待实测 / 待调研汇总（不静默假设）

| 项 | 归属 | 状态 |
| --- | --- | --- |
| URL path 百分号编码/默认端口规范化细节 | §1.1 | 待调研 |
| 两端产物哈希真实一致 | §1.1 | 待实测（P1 gate） |
| `engine.lock` 是否独立于 `journal.lock` | §4.2 | 待调研（倾向不新增） |
| Windows 内存/CPU 限额常量与 LPAC 组合行为 | §6.2 | 已实测（P0-B）：内存（commit 计量，与 `RLIMIT_AS` 语义不同）与 JOB_TIME 生效；CPU rate cap 嵌套限制不生效（以 wall-clock 为主、JOB_TIME 为 CPU 兜底）；单进程/环境块/ACL 前提见 §6.2 |
| 环境块精确 allowlist（Rust worker 实际所需） | §6.2 | 待实测（P1） |
| LPAC 注册表键集（确认 worker 需 0 键） | §6.2 | 待实测（P1） |
| 生产 worker 目录须为可授 DACL 的 installer 自有目录 | §6.2 | P1 实施冻结 |
| reparse/跨卷/杀软/profile 崩溃清理 | §6.2 | P4 前用例 |
| Windows 支持版本冻结（Win11 x64） | §8 | 已确认冻结（2026-10-08）；标准零售机矩阵为 P4 发布条件 |
| Windows 版本矩阵（标准 Win11 零售机） | §8 | P4 发布条件 |
| ARM64 | §8 | 不承诺（登记） |
| LZMA2 依赖选型/许可/体积 | §8 | 已调研（lib-2）：选定 `lzma-rust2` 0.21；parity/截断/内存/交叉构建/性能/SBOM 待实测（P1） |
| 用户规则是否携带 `root_layout` | §7 | P1 用真实 settings 抽样确认（不作为冻结前提） |
| `migrate_state` 去留 | §4.2 | P1 决策（倾向删除） |
| 宿主 RPC 方法白名单/事件命名空间/终态字段 | §11（design §6） | 已冻结：[p1-host-contract.md](p1-host-contract.md) v1.2（P1-0 门禁通过，2026-10-08） |
| Linux 树级资源限额（cgroup v2 / RLIMIT_NPROC；与 Windows Job 树级约束不对称） | §6.2 | 待实现/待实测（P1/P3；P0-D 评审 M2） |
| Linux launcher 输出限额 fail-closed（stdout 16 MiB / stderr 64 KiB，超限终止并报 `exe_worker_response_limit`） | §6.1 | 已修复（P0-D remediation，WSL 非 root + root 场景验证；评审 M3） |
| Linux launcher P1 事项：固定 `/input/archive.exe` 命名、路径校验（no_links/输入输出不相交）、`RLIMIT_FSIZE` 默认、单进程不变量跨平台措辞、`pre_exec` 安全复核、宿主进程不可见断言 | §6.1/§6.2 | P1 登记（P0-D 评审 R1-R6） |
| root 宿主降权（setgroups→setgid→setuid） | §4.3 | 最小实现 + WSL root 场景补测通过（P0-D）；生产 Decky 接入按 P3 复核 |

---

## 11. 显式延后到 P1 的契约

**【冻结值】** 宿主 RPC 方法白名单、事件命名空间与终态字段**延后到 P1**，以 [design.md](design.md) §6（CLI、事件与任务生命周期）接口表为准；P1 开始时**先冻结这些契约再实现**，不得在未冻结前固化为生产实现。P0 已冻结的是 worker 内部协议（§6.1）与状态 schema（§3），二者不随此延后。

**2026-10-08 更新：本项已冻结**——见 [p1-host-contract.md](p1-host-contract.md)（v1.2，P1-0 门禁初审 + 定向复审通过；含 R1-R3 折入）。P1 实现以该稿为基线。

**【理由】** design §8 P0 交付含“锁与事件”，但 §6 接口表当前为“拟定”；本契约 P0 仅冻结 worker 协议。**被否替代**：P0 直接冻结宿主事件（会锁定未经评审的 UI/宿主面）。

**【迁移动作】** P1 开工前输出宿主方法/事件契约冻结稿，追加到本文件或独立契约；同步共享 UI、宿主白名单与 Rust 类型。

**【验证方式】** P1 冻结稿经评审；共享 UI/宿主白名单与之一致。

---

**与设计文档一致性**：本文覆盖设计 §10 全部待冻结项（Windows 支持版本/隔离配置 → §8 + §6.2；Rust 解析/解压依赖与许可 → §8；worker schema/等价内存限额 → §6；来源与安装 ID/确定性编码/版本 → §1/§2；新规则必填项与合法默认值 → §7；可丢弃来源/安装缓存与受保护设置/恢复材料边界 → §3；新状态 schema → §3；锁顺序 → §4；取消/租约/关闭细节 → §5）。宿主 RPC/事件契约按 C5 显式延后到 P1（§11）；worker 协议与状态 schema 已在 P0 冻结。未冻结项已标注，未以“全部可安装”开放入口。
