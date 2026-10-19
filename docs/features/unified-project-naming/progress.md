# 项目统一命名施工记录

[设计文档](design.md)

## 状态

已实现，当前平台自动化检查通过。Linux/POSIX 及实机升级待验证。

## 2026-10-20

- 从 main 创建 feature/unified-project-naming。
- 统一英文名 Steam Hentai Patch、中文名 Steam 黄油补丁助手及简介。
- 更新当前仓库链接、Windows 与 Decky 标题、发布元数据及终端安装器。保留持久化目录和已发布附件名。
- 补齐旧 Decky 名称的升级与恢复匹配。
- 更新当前技能的项目名称及运行验证示例；部署参考中仅暂存名称改动块，保留原有两处部署规范修改。
- GitHub 仓库已改名为 [Mabinogi-Cichol/steam-hentai-patch](https://github.com/Mabinogi-Cichol/steam-hentai-patch)，简介已更新。SSH publickey 认证失败，origin 切换为现有已认证的 HTTPS 地址；fetch 成功，main 与 origin/main 初始一致。
- 检查：npm run typecheck、npm run build:windows、npm run build:decky 全部通过。桥接测试 14/14 通过，包括旧名发现、任务读取、先卸载旧身份再确认安装、设置迁移和恢复原身份。
- Python 安装器/引导测试：43 项，10 项通过、33 项因当前 Windows 缺少 POSIX/SteamOS Bash 条件跳过（包含新增完整旧名升级与失败恢复用例）；不将跳过项计为验证通过。
- 横幅使用内置 imagegen 同步更新中英文名称，目视确认文字及现有图案；提示词记录在 [品牌素材说明](../../images/branding/README.md)。
- overengineering-review 静态复核：仅增加已发布旧身份所需的定向兼容；未增加硬限制、通用迁移框架或重试策略。沿实际安装/恢复路径核对了名称传递及设置恢复。
- 未部署设备，未验证真实 Decky 改名升级、离线 ZIP 升级或 Windows 安装包升级。离线 ZIP 的旧名升级步骤已在 README 明确。
- 代码提交：`2c9c2fa chore(branding): 统一项目名称并兼容旧名插件升级`。文档与横幅单独提交；交付流程为验证后合并 main 并推送至新仓库。
