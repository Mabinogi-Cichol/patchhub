# 重新查找后 Kagura 来源消失修复计划

- 所属功能：[KaguraGames 国内 / 海外内置源与 HGamer 兜底索引](design.md)
- 施工记录：[progress.md](progress.md)
- 状态：代码、自动化回归与 Steam Deck 实机复验均已通过。设备证据见 [progress.md](progress.md) 2026-10-05 条目与本目录对应取证目录。

## 1. 用户报告

在《Venus Blood Frontier》的补丁管理页点击“重新查找 / 查看更多来源”后，原本显示为已审核官方 EXE 的 **Kagura Games** 来源消失。该游戏的官方候选本身不含 AppID，身份依赖 HGamer 兜底索引回填。

## 2. 根因

候选身份回填只在单次查询内进行；一次刷新若未能重新取得身份或候选（兜底索引失败、分片未完成、超时或官方增量索引本轮未覆盖该文章），已审核候选会降级为仅标题线索或整条消失，从而不再匹配已审核 profile，投影出的官方 EXE 来源随之消失。问题分布在三个接缝（设备实测确认）：

1. `SourceRegistry.cached_candidates`（[sources.py](../../../apps/decky/backend/sources.py)）：部分刷新会把 `attempt` 候选与 `success` 候选按 `candidate_id` 合并，且 `attempt` 覆盖 `success`。一次 `partial` 刷新中未回填身份的候选会**遮蔽**上次完整结果里已回填身份的同一候选。
2. `SourceRegistry._discovery_sync`（同上）：强制刷新会丢弃上次成功记录，改由本轮兜底索引重新推导身份。若本轮兜底失败（force 下官方与兜底同时刷新），即使是 `complete` 的官方结果也不会重新获得身份。
3. `SourceRegistry._discovery_sync`：官方适配器的受限增量索引可能在预算内返回**空的 `complete`**（未覆盖目标文章且无诊断），随后被当作权威结果覆盖成功缓存；若本轮第二遍直取官方页也失败，来源整条消失。设备实时状态即 `kagura-cn success complete []`，而兜底仍持有该游戏的身份与官方上游页。

`candidate_id` 由 `source_id + 规范化发布页 + variant + languages + 版本身份` 推导，因此对“同一候选记录”保留身份是安全的；版本或语言变化会生成不同 `candidate_id`，不会错误继承。

## 3. 修复

1. `cached_candidates`：`partial` 的 `attempt` 候选仅作为**新增线索**补充；同一 `candidate_id` 冲突时以 `success`（最近一次完整/已验证结果）为准，避免身份、格式或镜像证据被不完整刷新遮蔽。
2. `_discovery_sync`：刷新结束后，对本次仍为 `game_appid is None` 的官方候选，按 `candidate_id` 从 `previous.success` 中回填已验证的 `game_appid` / `parent_appid` / `match_basis` / `identity_evidence`；仅限同一候选记录，且要求旧记录属于本 AppID 且匹配依据已核实。
3. `_discovery_sync`：当某官方源本轮结果为空、且兜底索引（本轮或已缓存成功记录）仍指向该官方源的同一发布页时，保留 `previous.success` 中该 AppID 的已验证候选，而不把空结果当作权威删除；没有兜底页确认时仍按空结果覆盖（可负缓存）。后续出现非空清单且未包含该候选时照常替换。

三处都只保存/回填元数据，不扩大下载与安装信任；实际能力仍由受审 `exe_profiles.json`、域名白名单、build 与文件基线约束。

## 4. 回归测试

- `test_partial_refresh_does_not_shadow_verified_candidate_and_keeps_new_leads`：`success` 为已回填身份的候选、`attempt` 为同一 `candidate_id` 的未回填候选时，投影应保留已回填候选，同时保留仅出现在 `attempt` 的新线索。
- `test_refresh_without_fallback_identity_keeps_previously_verified_candidate`：兜底失败、官方 `complete` 返回同一候选记录时，应沿用上次已验证身份；不同 `candidate_id` 的新版本不得继承旧身份。
- `test_empty_official_refresh_keeps_verified_candidate_when_index_confirms_page`：官方空结果且兜底索引指向同一官方页时保留已验证候选；无兜底确认时保持为空。

## 5. 验证

- 开发机（Windows，Python 3.11.9）：`apps/decky/backend` 全量 204 项通过（3 项按设计跳过），其中新增 3 项回归。
- 离线复现：构造 VBF（AppID 1189440）的 `success=已回填 official_title` + `attempt=partial 未回填` 记录，修复前 `cached_candidates` 返回未回填候选、`ExeProfiles.reviews` 不匹配 profile；修复后返回已回填候选并命中 `venusblood-frontier-cn-v5`。
- Steam Deck（`deck@192.168.31.186`，SteamOS 6.18.50，系统 Python 3.13.5）实机复验：
  - 部署 `sources.py`（安装 SHA-256 `42f0b899…`，旧文件备份 SHA-256 `07778044…`），经 `loader/reload_plugin` 重载，后端重启。
  - 确定性 A/B：旧代码投影 `kagura-cn title/null`、`reviews` guidance（来源消失）；修复后投影 `official_title/1189440`、`reviews installable`（`venusblood-frontier-cn-v5`）。
  - 空官方结果与同候选兜底失败两种 fake-module 场景在设备上均按预期保留。
  - 真实网络 `discover_game(force=True)` 连续两次：Kagura 官方 EXE 持续 `installable`。
  - 实时插件状态恢复后由部署代码读取为 `installable`；Steam CEF 真实界面显示 `VenusBlood FRONTIER International — 已找到补丁 · 1 个来源候选`（截图 SHA-256 `90040f30…`）。
- 未执行/边界：直接 Python 连 Decky loader WebSocket 被 Steam 前端单连接重载关闭，实时状态恢复改用部署后代码对状态目录执行一次强制重查再重载，非真实前端点击；未做实体手柄操作；未验证官方 EXE 下载/安装（本修复不改变安装信任边界）。

## 6. 待验证与边界

- 若官方页面确实下架或更换版本，`candidate_id` 变化后不会保留旧身份；出现非空清单且不含该候选时也会正常替换，来源按新记录如实消失。
- 实体手柄连续操作、断网后菜单操作仍未执行；均沿用既有功能验收记录中的边界。
