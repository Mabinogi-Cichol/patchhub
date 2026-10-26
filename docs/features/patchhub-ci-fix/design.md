# PatchHub 合并后 CI 修复

[施工记录](progress.md)

## 目标与范围

修复提交 `c0501b3bc5c1e6136a7df05a76f756ef1fba3a4e` 的 [push CI #70](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40121256739) 暴露的两处失败。分支为 `feature/patchhub-ci-fix`，从当前 main 创建。

## 问题与方案

- Decky：品牌改名删除了生产模块的 `PLUGIN_NAMES`，测试 FakeLoader 的探测与设置清理仍引用它，导致 Linux 安装测试 14 个错误。夹具改用当前唯一身份 `NAME`，不恢复旧品牌兼容层。
- Windows：更新脚本 Safe-Path 对文件调用 GetFullPath，却与未规范化的 Root 比较。Windows PowerShell 展开 8.3 短路径后，两者字符串不再匹配，合法更新被误判为越界。本地短事务路径复现与线上相同的 `Path escapes target`；普通长路径测试原本通过。线上日志没有输出具体失败路径，不能据此确认 runner 使用的短路径字符串。此逻辑在该合并前已存在，并非本次改名新增。
- 在 Safe-Path 入口先用相同 API 规范化 Root，保留托管路径、越界和重解析点检查。新增真实短路径事务回归，验证更新、用户文件保留与事务清理；卷不提供短名时明确跳过。

## 实施与验收

1. 读取具体 job 日志，分别复现 Linux 夹具错误与 Windows 短路径误报。
2. 修复夹具中的三处引用和 Safe-Path 的目录规范化。
3. Windows 更新全套、WSL 安装全套及桥接测试通过，检查差异并完成过度设计审阅。
4. 提交和推送后另行核验完整 GitHub CI；本轮本地测试不代表被失败步骤阻断的打包、构建和附件上传已在线通过。

不调整 workflow、模块选择、重试、超时或既有业务限制，不发布版本、不操作设备。
