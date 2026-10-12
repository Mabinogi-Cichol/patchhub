# P3 独立安全复核

日期：2026-10-12。关联：[设计](design.md) · [施工记录](progress.md) · [验证](p3-validation.md)。

使用multi-agent，第一轮Sol/high specialist分别实现缓存和锁、Luna/medium worker适配Decky；主Agent统一安装/CLI/Windows与运行验证；第二轮第四个Sol/high reviewer只读复核，后续仅原Agent定向修复/复核，共4个子Agent。

## 发现与闭环

| 问题 | 实际修正 | 验证 |
| --- | --- | --- |
| CLI policy仍硬编码Disabled | 改params.cleanup_policy | 真实CLI AfterCommit/重放/手动清理 |
| task证明后drop owner再按名称重开 | 保留owner和原task句柄至删除结束 | 终态/owner/lease/recovery fixture及定向静态复核 |
| Unix cache.lock换inode导致旧shared/新exclusive分裂 | 对既有installer根目录再flock，identity重核独立于lease | 替换锁后exclusive Busy、跨进程冲突、静态复核 |
| 无journal终态fixture因缺created_at_ms提前跳过 | 补完整可信schema | 最终cache回归 |
| 既有任务自行收尾可跟随被替换的tasks父目录 | 主Agent扩展为启动时捕获的目录句柄，安全助手收尾 | task正常终态、类型取消保留、替换tasks祖先外部材料不删除 |

Reviewer对前三处修正与tasks前置/严格字段/诊断截断完成定向静态复核，返回findings[]；任务自行收尾最后加固及Windows句柄提升由Reviewer再次定向静态复核通过，返回findings[]；运行证据由主Agent执行。Reviewer没有运行cargo/连接设备/提交。

## 运行修正

首轮清理前拒绝fixture在安装初始化前创建reports普通文件，实际命中安装初始化错误；改为commit后cleanup阶段注入，验证清理失败不改commit。新增CLI子进程测试补Read trait。Clippy要求map_or改is_none_or。Decky全量mock补policy参数；一次并行CPU负载下续租timing失败，定向10项与全量71项重跑通过，无业务代码修改。

所有未执行的实机/最终产物范围见[验证](p3-validation.md)；不以静态复核替代运行证据。

Windows任务句柄持有期间暴露原子lease发布installer_state_busy，定向定位为目录写共享不足；最终目录允许READ/WRITE共享、禁止DELETE共享，文件保持禁止写共享。任务工作期只读pin，收尾在固定parent下重开DELETE并先核对volume/file index身份；拒绝链接/替换后才删除。定向7项安装回归通过，完整Windows487/Linux494回归及Clippy已通过。

P3 阶段完成时结论：P3阶段可放行，独立只读复核findings[]，主Agent最终自动化通过。未连接设备、部署或提交，P4–P6与最终产物/实机仍待实施。

后续提交记录（2026-10-12）：P3/P4 代码已本地提交 `19ffc8a`，文档独立提交；P4 最终矩阵见 [P4 验证](p4-validation.md)，P5/P6 与实机待实施。
