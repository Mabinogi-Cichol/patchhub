# P6 独立复核阶段记录

日期：2026-10-13。关联：[施工记录](progress.md)、[阶段验证](p6-validation.md)。状态：**最终只读gate通过，findings=[]，D1–D10已关闭，P6按用户调整范围完成。**

用户明确授权独立复核。使用 1 个只读子 Agent `/root/p6_review`，GPT-6 Sol/high、fork none；主 Agent 负责实现、验证和整合，未并行启动其他子 Agent。

复核覆盖 D1–D10 的定义、生产调用和消费者、v4 与持久化数据边界、共享 URL helper、核心服务和打包路径。源码核对支持当前核心/CLI职责收敛及 D1–D9 的阶段实现；持久化路径中的历史版本号不能误判为 wire v3 生产回退。

| 发现 | 处理 | 验证边界 |
| --- | --- | --- |
| P2：共享 URL 规则允许尾点域名，双端私网保护规则不一致 | 共享 helper 拒绝尾点 host，新增共享与 Decky TS 回归 | 相关契约/TS 测试通过，审阅后续核对修复 |
| P3：core 顶层说明仍声称只读、没有写入 | 更新文档为当前核心服务职责 | 审阅后续核对修复 |
| P3：installer 模块说明与正式入口、缓存所有权不一致 | 更新为 `service::install` 三路线及 `CacheService` 唯一删除入口 | 主 Agent 核对；暂停前没有启动新一轮独立复核 |

随后 Windows 打包补齐许可证并要求 SBOM，已有最终工具测试与产物核验；该最终修改尚未纳入全范围独立终轮 gate。Linux 最终产物、Deck 正式 v4 部署、真实 UI/游戏/手柄及取消回滚证据未齐全，因此不能宣称整个 P6 独立验收通过，也不关闭 D10 或其他债项的最终 gate。

## WSL恢复后的新增证据复核

沿用用户授权，1个新只读子Agent `/root/p6_final_review`，GPT-6.1 Sol/high、fork none，主Agent继续集成和实机验证；累计两个独立reviewer，无Astra。恢复轮不运行构建/测试/网络或操作设备，不编辑文件。

独立核对当前Linux日志575passed/6ignored、Python78无skip、fmt/Clippy/check/release完成；release/count四场景响应及指纹、snapshot哈希一致。cargo.absolute正确保留rustup代理argv0。DeckZIP24成员、verification/deploy-manifest与plugin-output源字节及CLI/worker release哈希一致，ELF/mode/许可证/SBOM覆盖。NSIS8应用文件一致，主exe与portable只差三个NSS/UNK字节，补齐之前内容gate缺口。Windows覆盖committed、取消rolled_back及71文件前后相等独立核对通过；用户游戏效果确认按声明保留。

正式Deck加载/RPC证据：插件实例1，真实扫描27游戏，signed目录rev4，含两个目标AppID；不将该读取证据等同游戏实装。源码与上述新增证据findings=[]。

| 债项 | 恢复后判断 |
| --- | --- |
| D1–D8 | 源码收敛判断成立；Windows/Linux自动化与双端匹配包齐备 |
| D9 | 无旧协议执行降级；最终Deck包allowlist/同源/runtime匹配已补齐 |
| D10 | 产物、NSIS内容、正式Deck加载/RPC通过；剩余Deck游戏/控制器和最终文档待收口 |

Windows真实UI与直链EXE按用户明确豁免记“跳过”，不要求补跑，不写作已验证。Deck实际安装/覆盖/写入取消和最终游戏效果仍待追加证据，不能提前宣布整个P6完成。

## 最终实机发现与修复复核

P2：publisher在计划生成前的download_cancelled未纳入core标准取消列表，真实前台重连安全取消被宿主标成failed。已修复core公共边界并新增公共安装API回归；旧实现Linux RED、新实现双端GREEN，不新增宿主错误码分支。GPT-6.1 Sol/high只读复核修复及回归，findings=[]。随后重建双端最终包并正常重载Deck，新版真实下载取消返回cancelled，真实EXE写入取消rolled_back且83游戏文件不变。

最终Windows604/Linux576/Python78、双端包/NSIS内部内容、同源计数基准、Linux同包冒烟与正式Deck24文件/HTTP/RPC、ZIP和EXE覆盖/取消、恢复/清理及本轮用户游戏/实体手柄确认已补齐；远端本轮暂存已清理，本地备份重新核验。最终只读gate正在核对这些证据与更新后的文档，尚不提前将其记为通过。

## 最终 gate 结论

GPT-6.1 Sol/high只读终轮复核通过，findings=[]。独立核验最终Windows604/Linux576/Python78、双端四场景计数与当前生产源、Deck24/Windows便携11文件及NSIS内部8文件、最终三包SHA与部署清单、真实下载取消与EXE写入回滚、83游戏文件一致、恢复/清理及14事务不变、正式RPC/HTTP/页面、本轮用户游戏/手柄确认，以及远端清理和本地保留六项备份/证据。最终文档/链接与git diff --check通过。

D1–D10与P6按用户调整范围正式关闭。Windows真实UI/直链EXE仍记跳过，NSIS未执行实际安装，Deck已托管实例使用覆盖验证；没有将这些边界改成通过，也未新增提交/推送/发布。此前各节是当时复核的历史记录，最终结论以本节为准。
