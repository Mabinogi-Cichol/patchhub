---
name: steam-patch-release
description: 发布本项目版本、维护 GitHub Release 标题或说明，并核验发行附件；Release 标题严格使用版本 Tag，不添加项目名或其他后缀。仅修改发布规范或技能时不触发发布。
---

# 本项目版本发布

适用于本技能所在的 `patchhub` 仓库。项目根目录为技能目录向上三级，先阅读 [AGENTS.md](../../../AGENTS.md)。使用当前会话已经明确授权的发布范围；本技能本身不授权提交、推送、部署或修改游戏。

## Release 标题

- **标题与版本 Tag 完全一致**，如 `v0.1.0` 或 `v0.2.0-beta.1`。
- 不添加项目名、分隔符、空格说明、“首发”等前后缀，不使用装饰性 Markdown 或 HTML。
- Release 正文按下述“发布描述”格式列出新增与优化、问题修复。
- 创建和编辑时都遵守此规则。仅要求修正现有标题时，只更新标题，保留 Tag、附件、正文和发行状态，不重建安装包或移动 Tag。

仅改标题的示例（仓库与 Tag 必须先核实）：

```powershell
gh release edit v0.1.0 --repo OWNER/REPO --title v0.1.0
gh release view v0.1.0 --repo OWNER/REPO --json name,tagName,url
```

读回并确认 `name == tagName`，不能只根据命令退出码宣称标题正确。

## 发布描述

- 正文使用中文，按“新增与优化”和“问题修复”分类，以简短列表说明本版本的实际变化；某类没有变更时省略整个分类，不写“无”。
- 每项用一句话说明用户可感知的变化，避免堆叠技术细节、提交记录和验证过程。
- 不附加项目介绍、下载文件清单、安装教程、校验信息或单独的已知限制章节。影响使用的变化直接写入对应条目。
- 发布核验仍按本技能执行，核验结果写入发布记录及任务汇报。

```markdown
## 新增与优化

- 新增……
- 优化……

## 问题修复

- 修复……
```

## 发布新版本

1. 核实实际仓库、`git remote -v`、分支、工作区、目标提交和已有 Tag/Release。用户要求“最新”时先 fetch 并确认发布提交；保留不属于本次发布的未提交内容。明确发布范围后继续，不重复索取已给出的授权。
2. 应用版本只人工修改根 `Cargo.toml` 的 `workspace.package.version`；执行 `python scripts/project_version.py --sync`，检查并提交自动生成的 Flutter / Decky / 共享 UI 清单及 Cargo/npm 项目锁字段，再运行 `--check`。Windows EXE（默认额外构建号 `+1`）、更新 JSON、NSIS、Decky 包内 package.json 与 SBOM 应用版本须按格式映射对应同一主版本。CI 在编译前核对派生字段及 `v<主版本>` Tag；Tag 不会自动改写程序内版本。沿用对应且未过期的验证证据，只有新改动或未解决问题才补检查。
3. 本仓库 [CI 工作流](../../../.github/workflows/ci.yml) 在推送 `v*` Tag 后运行双平台检查和打包，只有 core、windows、decky 全部成功才发布。以 Tag 对应运行作为发行证据，不能用旧安装包代替最新源码构建。
4. Git 改动需要提交时使用 [项目提交技能](../steam-patch-git-commit/SKILL.md)，按仓库规则拆分。只有用户授权发布或推送时才推送；已发布 Tag 保持不变，新代码版本使用新 Tag。
5. Release 标题只用 Tag；正文遵循上述“发布描述”格式。多行正文存入临时文件，通过 `--notes-file` 传入；工作流自动生成的正文也要替换为此格式。版本 Tag 含 `-` 时，现有工作流将其标记为预发行版。
6. 仅修改文档、技能或标题时，不触发额外设备部署、截图或游戏补丁安装。需要设备工作且已授权时使用 [Decky 部署验证技能](../decky-deploy-verify/SKILL.md)。

## 附件与在线核验

- 公开附件仅包括 Windows NSIS、Windows portable ZIP、Decky ZIP 和 `SHA256SUMS.txt`，清单覆盖这三个安装包。不单独上传 checksums JSON、CycloneDX SBOM 或 `install-decky.py`；包内 SBOM 与仓库安装脚本继续保留，在线安装入口从仓库读取脚本。
- GitHub 上传会规范化文件名，例如把 NSIS 名称中的空格改为点号；在暂存附件时先统一公开名称，再生成校验清单。核对清单中的名称与实际公开下载名一致，不能只核对哈希值。
- 发布后从 Release 下载并验证附件 SHA-256、ZIP 内容白名单与 CRC、程序平台/架构、Decky 执行权限与 runtime 哈希、SBOM 锁文件摘要及资源/契约来源。Windows runner 的 CRLF 检出与 Git LF blob 应按实际规则区分。
- 读回 Release 的标题、Tag、公开/预发行状态、附件数量与下载链接。只改标题时无需重新下载并校验未变化的所有附件。
- 证据及下载文件保存在忽略目录 `.artifacts/`；报告实际发布 URL、Tag 对应提交和未验证项。发布记录见 [发行检查](../../../docs/release-checklist.md)，不能把历史实机验证当作本次安装包的部署证明。
