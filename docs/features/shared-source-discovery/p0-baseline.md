# P0 基线与契约冻结记录

设计：[design.md](design.md)。P0/P1 实现规范：[p0-p1-rust-design.md](p0-p1-rust-design.md)。施工记录：[progress.md](progress.md)。

状态：**P0 已完成（实机证据 + 离线契约）**。日期：2026-10-07（Asia/Shanghai）。

## 1. 范围

冻结默认规则/旧 schema 语义、跨语言 ID golden、限制清单，并核对 AppID 2303800（妃神会秘史 / My Life as a Cult Leader）在真实 Decky 设备上的来源发现结果与身份证据。原始设备文件与页面正文保存在忽略目录 `.artifacts/`，**不入库**；本文件只记录脱敏后的可核对事实。

## 2. 设备与身份核对

| 项目 | 本轮实际值 |
| --- | --- |
| Devkit 配对设备 | `steamdeck`，`deck@192.168.31.186`，密钥路径已确认（不记录内容） |
| SSH 身份 | `uid=1000(deck)`，`plugin_loader` **active** |
| 插件目录 | `/home/deck/homebrew/plugins/steam-patch-manager` `root:root 755` |
| 插件显示名 / 版本 | `Steam 补丁管理` / `steam-patch-decky@0.1.0` |
| 状态目录 | `/home/deck/.local/share/steam-patch-manager` |
| 采集时间 | 2026-10-07（设备发现 `observed_at` 约 `2026-10-07T05:43Z`） |

**主机密钥变化**：系统旧记录 `SHA256:b77rklEC2scaeGfv0qeSbl7JCWPizDbzZ6APlCDF9So`，远端当前 `SHA256:1eQWToCx6mKr5lz/YPXgwit390H7XTkWcpR56L0mJx8`。经用户确认即本轮目标设备（SteamOS 重建 host key），采用本次配对指纹写入本轮独立 `known_hosts`，系统旧 `known_hosts` 保留未改。

## 3. 2303800 真实发现结果（冻结）

`overrides.json`：`enabled={patchmoe:true, hgamer:true}`，`added=[]`，`games={3239270:{patchmoe}, 1227890:{kagura-cn}, 1788100:{patchmoe}}`（2303800 无逐游戏覆盖，走默认订阅）。

`status/`：`patchmoe=source_refreshed`；`kagura-cn`/`kagura-en`=`source_refreshed`（`entry_count 0`）；`hgamer=source_refresh_failed`。

`discovery/<source>/2303800.json`（schema 1，`parser_version=2`，`query_version=1`，`success.status=complete`，`attempt.status=complete` with 0 candidates）：

| source_id | candidate_id | match_basis | variant | languages | version_identity | identity_evidence |
| --- | --- | --- | --- | --- | --- | --- |
| `kagura-cn` | `candidate-21d97c05e6cf1324b0cea5c7` | `verified_alias` | `base` | `["zh-CN"]` | `4:My Life as a Cult Leader CN Patch v4.exe` | `verified_alias` @ hgamer page, appid 2303800 |
| `kagura-en` | `candidate-d04331eb1335da62bedec536` | `verified_alias` | `base` | `["en"]` | `3:My Life as a Cult Leader EN Patch v3.exe` | `verified_alias` @ hgamer page, appid 2303800 |
| `hgamer` | `candidate-625cfade8b8786da9e58842c` | `appid` | `base` | `["zh-CN"]` | `4` | `steam_appid` 2303800 @ hgamer page |

来源页：

- kagura-cn：`https://kagurafan.com/2023/03/24/%e5%a6%83%e7%a5%9e%e4%bc%9a%e7%a7%98%e5%8f%b2/`
- kagura-en：`https://www.kaguragames.com/product/my-life-as-a-cult-leader-patch/`
- hgamer：`https://hgamer.tw/zh-TW/patches/my-life-as-a-cult-leader-patch`

**关键点**：三源均给出 `game_appid=2303800` 的候选；kagura 双站以 `verified_alias`（引用 hgamer 身份页）匹配，hgamer 以 `steam_appid` 匹配。全部 `capability=discover_only`，不产生安装权限。

## 4. 跨语言 ID 复现（实机 golden）

用上述真实 identity `[source_id, canonical_url(page), variant, languages, version_identity]` 反算 `candidate_id`，三条与设备值**逐字节一致**。已并入
`crates/core/tests/fixtures/sources/identity-golden.json`（`candidate_id` 用例，`origin=decky-2303800-real`），由
`crates/core/tests/fixtures/sources/gen_identity_golden.py` 复现，并由 Rust
`sources::canonical::tests::canonical_matches_python_golden` 断言。

## 5. 页面证据（脱敏，正文不入库）

抓取到 `.artifacts/<run>/pages/`，仅记录大小与 SHA-256：

| 来源 | bytes | sha256 |
| --- | --- | --- |
| kagura-cn | 141675 | `c7953533bd83d996523b89451f72b31ccabcbd76eda13672bb898cf6703b39fa` |
| kagura-en | 369381 | `ea65731cc4bd2304ce30e2f29232f2b2c1b38c621583b110eaabb4e88f52c3fc` |
| hgamer | 175788 | `e95447b3644abbd249e8cf58154002b43855de1af9fbdebafd4ed505c745a5d9` |

抓取时间晚于设备发现，网页可能已变化；**该哈希只作当轮证据，不作为解析契约**。适配器离线契约由
`crates/core/src/sources/adapters/*` 的最小内联 HTML fixture 固化；真实页面在线语义对齐仍列入 P2/P4 待办。

## 6. 结论与遗留

- 已满足 P0 门槛：2303800 的真实 `source_id`/身份已核对；模型/常量/ID/TTL/限制契约与跨语言 golden（含实机候选）已冻结并通过 Rust 测试；Windows「无源」根因（仅静态签名目录、无动态查源）在两轮施工记录中可复现。
- 未执行/明确遗留：
  - 整份设备状态库、完整网页正文、凭证与补丁资产**不入库**；本文件与 `.artifacts/` 为唯一证据来源。
  - 页面级在线语义对齐与真实补丁下载/安装验收属 P4/P5，未在本轮声称完成。
  - `patchmoe`/`kagura` 真实请求方法（POST/表单/cookie）与设备逐字一致性未验证（当前 `RequestContext` 仅 GET）。
