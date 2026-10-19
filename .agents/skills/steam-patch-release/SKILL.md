---
name: steam-patch-release
description: 发布本项目版本、维护 GitHub Release 标题或说明，并核验发行附件；Release 标题严格使用版本 Tag，不添加项目名或其他后缀。仅修改发布规范或技能时不触发发布。
---

# 本项目版本发布

适用于本技能所在的 `steam-hentai-patch` 仓库。项目根目录为技能目录向上三级，先阅读 [AGENTS.md](../../../AGENTS.md)。使用当前会话已经明确授权的发布范围；本技能本身不授权提交、推送、部署或修改游戏。

## Release 标题

- **标题与版本 Tag 完全一致**，如 `v0.1.0` 或 `v0.2.0-beta.1`。
- 不添加项目名、分隔符、空格说明、“首发”等前后缀，不使用装饰性 Markdown 或 HTML。
- 项目介绍、首发说明、变更及已知限制写入 Release 正文。
- 创建和编辑时都遵守此规则。仅要求修正现有标题时，只更新标题，保留 Tag、附件、正文和发行状态，不重建安装包或移动 Tag。

仅改标题的示例（仓库与 Tag 必须先核实）：

```powershell
gh release edit v0.1.0 --repo OWNER/REPO --title v0.1.0
gh release view v0.1.0 --repo OWNER/REPO --json name,tagName,url
```

读回并确认 `name == tagName`，不能只根据命令退出码宣称标题正确。

## 发布新版本

1. 核实实际仓库、`git remote -v`、分支、工作区、目标提交和已有 Tag/Release。用户要求“最新”时先 fetch 并确认发布提交；保留不属于本次发布的未提交内容。明确发布范围后继续，不重复索取已给出的授权。
2. 核对 Cargo workspace、Windows/Decky package.json、Tauri 配置及共享 UI 的版本字段。Tag 不会自动更新程序内版本。沿用对应且未过期的验证证据，只有新改动或未解决问题才补检查。
3. 本仓库 [CI 工作流](../../../.github/workflows/ci.yml) 在推送 `v*` Tag 后运行双平台检查和打包，只有 core、windows、decky 全部成功才发布。以 Tag 对应运行作为发行证据，不能用旧安装包代替最新源码构建。
4. Git 改动需要提交时使用 [项目提交技能](../steam-patch-git-commit/SKILL.md)，按仓库规则拆分。只有用户授权发布或推送时才推送；已发布 Tag 保持不变，新代码版本使用新 Tag。
5. Release 标题只用 Tag；正文使用中文，说明功能、下载文件、安装方式及已知限制。多行正文存入临时文件，通过 `--notes-file` 传入。版本 Tag 含 `-` 时，现有工作流将其标记为预发行版。
6. 仅修改文档、技能或标题时，不触发额外设备部署、截图或游戏补丁安装。需要设备工作且已授权时使用 [Decky 部署验证技能](../decky-deploy-verify/SKILL.md)。

## 附件与在线核验

- 预期包括 Windows NSIS、Windows portable ZIP、Decky ZIP、两端 checksums JSON、两端 CycloneDX SBOM 及 `SHA256SUMS.txt`；实际清单以该版本工作流为准。
- GitHub 上传会规范化文件名，例如把 NSIS 名称中的空格改为点号；在暂存附件时先统一公开名称，再生成校验清单。核对清单中的名称与实际公开下载名一致，不能只核对哈希值。
- 发布后从 Release 下载并验证附件 SHA-256、ZIP 内容白名单与 CRC、程序平台/架构、Decky 执行权限与 runtime 哈希、SBOM 锁文件摘要及资源/契约来源。Windows runner 的 CRLF 检出与 Git LF blob 应按实际规则区分。
- 读回 Release 的标题、Tag、公开/预发行状态、附件数量与下载链接。只改标题时无需重新下载并校验未变化的所有附件。
- 证据及下载文件保存在忽略目录 `.artifacts/`；报告实际发布 URL、Tag 对应提交和未验证项。发布记录见 [发行检查](../../../docs/release-checklist.md)，不能把历史实机验证当作本次安装包的部署证明。
