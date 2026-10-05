# 重新查找后 Kagura 来源消失修复计划

- 所属功能：[KaguraGames 国内 / 海外内置源与 HGamer 兜底索引](design.md)
- 施工记录：[progress.md](progress.md)
- 状态：代码与自动化回归已实现并验证；Steam Deck 实机复验待执行。

## 1. 用户报告

在《Venus Blood Frontier》的补丁管理页点击“重新查找 / 查看更多来源”后，原本显示为已审核官方 EXE 的 **Kagura Games** 来源消失。该游戏的官方候选本身不含 AppID，身份依赖 HGamer 兜底索引回填。

## 2. 根因

候选身份回填只在单次查询内进行；一次刷新若未能重新取得身份（兜底索引失败、分片未完成或超时），已审核候选会降级为仅标题线索，从而不再匹配已审核 profile，投影出的官方 EXE 来源随之消失。问题分布在两个接缝：

1. `SourceRegistry.cached_candidates`（[sources.py](../../../apps/decky/backend/sources.py)）：部分刷新会把 `attempt` 候选与 `success` 候选按 `candidate_id` 合并，且 `attempt` 覆盖 `success`。一次 `partial` 刷新中未回填身份的候选会**遮蔽**上次完整结果里已回填身份的同一候选。
2. `SourceRegistry._discovery_sync`（同上）：强制刷新会丢弃上次成功记录，改由本轮兜底索引重新推导身份。若本轮兜底失败（force 下官方与兜底同时刷新），即使是 `complete` 的官方结果也不会重新获得身份。

`candidate_id` 由 `source_id + 规范化发布页 + variant + languages + 版本身份` 推导，因此对“同一候选记录”保留身份是安全的；版本或语言变化会生成不同 `candidate_id`，不会错误继承。

## 3. 修复

1. `cached_candidates`：`partial` 的 `attempt` 候选仅作为**新增线索**补充；同一 `candidate_id` 冲突时以 `success`（最近一次完整/已验证结果）为准，避免身份、格式或镜像证据被不完整刷新遮蔽。
2. `_discovery_sync`：刷新结束后，对本次仍为 `game_appid is None` 的官方候选，按 `candidate_id` 从 `previous.success` 中回填已验证的 `game_appid` / `parent_appid` / `match_basis` / `identity_evidence`；仅限同一候选记录，且要求旧记录属于本 AppID 且匹配依据已核实。

两处都只保存/回填元数据，不扩大下载与安装信任；实际能力仍由受审 `exe_profiles.json`、域名白名单、build 与文件基线约束。

## 4. 回归测试

- `test_partial_refresh_does_not_shadow_verified_candidate_and_keeps_new_leads`：`success` 为已回填身份的候选、`attempt` 为同一 `candidate_id` 的未回填候选时，投影应保留已回填候选，同时保留仅出现在 `attempt` 的新线索。
- `test_refresh_without_fallback_identity_keeps_previously_verified_candidate`：兜底失败、官方 `complete` 返回同一候选记录时，应沿用上次已验证身份；不同 `candidate_id` 的新版本不得继承旧身份。

## 5. 验证

- 开发机（Windows，Python 3.11.9）：`apps/decky/backend` 全量 203 项通过（3 项按设计跳过），其中新增 2 项回归。
- 离线复现：构造 VBF（AppID 1189440）的 `success=已回填 official_title` + `attempt=partial 未回填` 记录，修复前 `cached_candidates` 返回未回填候选、`ExeProfiles.reviews` 不匹配 profile；修复后返回已回填候选并命中 `venusblood-frontier-cn-v5`。
- 未执行：Steam Deck 真实网络下连续点击“重新查找”的实机复验；后端与真实站点交互下的完整事务未重跑。

## 6. 待验证与边界

- 在真实 Deck 上对《Venus Blood Frontier》连续重新查找，确认 Kagura 官方 EXE 来源持续可见。
- 部署更新后的插件包并刷新前端 bundle；本轮未部署、未提交 Git。
- 若官方页面确实下架或更换版本，`candidate_id` 变化后不会保留旧身份，来源按新记录如实消失。
