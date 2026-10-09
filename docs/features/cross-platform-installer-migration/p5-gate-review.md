# P5 独立复核

设计：[design.md](design.md)。施工：[progress.md](progress.md)。实施：[p5-decky.md](p5-decky.md)。日期：2026-10-10。

复核方式：后端与前端两个实现 Agent 分工；它们在完成代码及部分检查后遇到额度限制，由主 Agent 接续。独立 reviewer 使用 gpt-6.1-sol / high（原强模型路线额度受限后的替代），主体只读复核；实机反馈问题由 reviewer 限定修改前端并验证，不操作设备。

## 闭合问题

| 问题 | 修复与证据 |
|---|---|
| 旧 committed 内容在新收据缺失时可能被当作原版覆盖 | 切换前停旧准入，发布当前 schema 阻断记录；ZIP、EXE、签名入口统一拒绝，CLI 三路由回归通过 |
| 终态 note/error 跨游戏残留，包括异步晚到反馈 | 反馈分别保存实例归属；任务/恢复/单游戏订阅/preview 绑定原实例，全局消息保持全局；新增3回归，最终前端63项通过 |
| Steam 移库改变 instance_id 可绕过旧实例保护 | 同时绑定可信 AppID，移库路径回归覆盖，界面显示已有补丁需检查 |
| 错误空状态目录被切换工具判安全 | RPC 与磁盘事务数量必须一致，错误目录主入口回归通过 |
| 清理偏好被移到新 host-v3 导致丢失 | 偏好继续读取状态根 settings.json；实机 auto_cleanup=true 保留 |
| 动态来源归属丢失及 EXE 条件隐藏 | 核心投影 source_id/candidate_id；界面按候选展示语言、版本与 DLC 条件，实机页面核对 |
| 每个来源更新均完整扫描游戏库 | 批量完成时一次核心投影，逐事件只维护发现数据 |
| 实机 signed ZIP 首帧空阶段导致 invalid_core_response | CLI ProgressCursor 仅在协议输出边界把内部空默认映射 connecting；operation::run→archive→Cursor 真实回归，CLI 41 项通过，实机复验进入下载并安全取消 |

最后复核结论：已报告问题闭合，未发现剩余代码门禁阻断。复核结论不代替实机安装、屏幕与实体手柄验收；各项以实施记录为准。

## EXE 实装支持边界复核

《夏日狂想曲+》CN DLC 2.02.1 实际样本下载成功，静态复核未执行安装器。样本大小、EXE/metadata SHA 与旧人工 DLC profile 一致；Python 通用图与 Rust 在 CSetupFileData 尾字节同样拒绝（metadata offset 45133，expected 0 / actual 1，5874 条记录均为 1）。未发现 Rust 移植偏差；这是未支持记录字段，错误码 exe_project_options_unsupported 太宽泛。

支持此字段前必须取得通用序列化或静态字段消费证据，明确对复制、条件、卸载、注册、重启/执行/快捷方式等语义的影响，然后实现命名字段、合法值/非法值矩阵以及全图/CRC/属性/路径/事件与旧样本回归。不能只接受 0|1 或按样本/AppID/profile 摘要绕过；本轮生产代码保持严格拒绝。
