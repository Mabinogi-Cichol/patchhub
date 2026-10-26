# PatchHub 合并后 CI 修复

[施工记录](progress.md)

## 目标与范围

修复提交 `c0501b3bc5c1e6136a7df05a76f756ef1fba3a4e` 的 [push CI #70](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40121256739) 暴露的两处失败。分支为 `feature/patchhub-ci-fix`，从当前 main 创建。

## 问题与方案

- Decky：品牌改名删除了生产模块的 `PLUGIN_NAMES`，测试 FakeLoader 的探测与设置清理仍引用它，导致 Linux 安装测试 14 个错误。夹具改用当前唯一身份 `NAME`，不恢复旧品牌兼容层。
- Windows：更新脚本 Safe-Path 对文件调用 GetFullPath，却与未规范化的 Root 比较。Windows PowerShell 展开 8.3 短路径后，两者字符串不再匹配，合法更新被误判为越界。本地短事务路径复现与线上相同的 `Path escapes target`；普通长路径测试原本通过。线上日志没有输出具体失败路径，不能据此确认 runner 使用的短路径字符串。此逻辑在该合并前已存在，并非本次改名新增。
- 在 Safe-Path 入口先用相同 API 规范化 Root，保留托管路径、越界和重解析点检查。新增真实短路径事务回归，验证更新、用户文件保留与事务清理；卷不提供短名时明确跳过。

### 后续 push 暴露的补充问题

- `fb232c9` 的 [push run](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40346092622) 中，Decky 安装测试通过，后续 bootstrap 测试仍依赖 README 中已经删除的旧短链接，导致 8 个错误。测试统一使用当前完整 ENTRY_URL，删除旧短链接夹具、50 字符限制及已与主入口重复的 fallback 用例。
- Windows 仍有两项失败：短事务路径更新误报越界、恢复事务误报目标不一致。将整个 TEMP 改为短路径后，本地精确复现同样两项失败。初次修复只处理 Safe-Path；当短名祖先下的 stage/backup 尚不存在时，GetFullPath 对不同存在状态的路径展开结果可能不一致。应在追加这些路径前先规范化已经存在的 Work，并对恢复标记、journal 与清理标记中的路径使用相同规范化方式比较。
- 覆盖普通 TEMP、短 TEMP 全套 Windows 更新测试，并增加短路径持久化记录的恢复回归。保留路径边界检查和非托管文件保护，不新增兼容层或重试。
- changes 与 ci 均保留且在线实际执行。此次 changes 选择 Windows/Decky、跳过 Core；ci 正常汇总模块失败。无须修改 job 定义。

## 实施与验收

1. 读取具体 job 日志，分别复现 Linux 夹具错误与 Windows 短路径误报。
2. 修复夹具中的三处引用和 Safe-Path 的目录规范化。
3. Windows 更新全套、WSL 安装全套及桥接测试通过，检查差异并完成过度设计审阅。
4. 提交和推送后另行核验完整 GitHub CI；本轮本地测试不代表被失败步骤阻断的打包、构建和附件上传已在线通过。

不调整 workflow、模块选择、重试、超时或既有业务限制，不发布版本、不操作设备。
