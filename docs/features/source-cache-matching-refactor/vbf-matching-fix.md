# VenusBlood FRONTIER 匹配与重新匹配异常修复

- 设计：[design.md](design.md)；施工：[progress.md](progress.md)。
- 设备证据：2026-10-19 Loader 的 rematch_game 抛出 instance_required；前端同时传 instance_id/appid，后端要求二选一。
- Kagura CN revision=1/complete=true 已含 Venus Blood Frontier 文章及 VenusBlood Frontier International 分类；1189440 旧记录保留 HGamer 的 AppID 身份和精确 upstream_page，普通源候选为空。
- 修改：前端按安装实例发起重匹配，appid 参数传 null；Rust 使用当前游戏的可信 HGamer 候选的精确 upstream_page，在已启用普通源的已发布索引内查找并解析文章，记录身份依据。已有可信 fallback 可复用；本次新补查命中后同一任务完成索引内文章解析。禁止上游链接触发全站更新、禁止仅靠相似标题确认身份。
- 验证：回归覆盖前端参数互斥、旧 fallback 与新 fallback、错误 AppID/未入索引上游、无标题误认；运行相关 Rust/Decky 测试与构建。设备只读复现、隔离状态与真实网站 CLI 对比已完成；修复正式部署与页面按钮验证待执行。
- 过度设计计划自审：复用现有候选、索引、详情缓存和任务流程，不增加配置、重试、状态或硬限制。

实现与验证已完成，详见施工记录 2026-10-19 的 VenusBlood 专项段落。正式插件尚未替换。
