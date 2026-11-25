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

1. 核实实际仓库、`git remote -v`、分支、工作区、目标提交和已有 Tag/Release。保留不属于本次发布的未提交内容。明确发布范围后继续，不重复索取已给出的授权。
2. 使用 `python scripts/project_version.py --bump <patch|minor|major>`（或 `--set <版本>`）一步更新根 `Cargo.toml` 的 `workspace.package.version`、同步派生清单及锁文件并完成一致性校验。同时在打 Tag 前于 `docs/releases/v<主版本>.md` 编写符合“发布描述”格式的中文 Release 正文并一并提交，CI 在 `changes` 阶段会校验该文件存在且非空。
3. 本仓库 [CI 工作流](../../../.github/workflows/ci.yml) 在推送 `v*` Tag 后自动运行双平台检查、打包、`sha256sum --check` 校验并创建 GitHub Release。发布前不强制运行全套本地 CI 对齐。
4. Git 改动需要提交时使用 [项目提交技能](../steam-patch-git-commit/SKILL.md)，按仓库规则拆分。只有用户授权发布或推送时才推送；已发布 Tag 保持不变，新代码版本使用新 Tag。
5. 推送 `main` 与 `v<主版本>` Tag 后，确认对应线上 CI 工作流已触发即可完成发布操作；**无需阻塞等待线上 CI 完成，也无需下载核验 Release 附件**（若线上 CI 失败，GitHub 会邮件通知仓库所有者）。
6. Release 标题只用 Tag；正文遵循上述“发布描述”格式，由 CI 直接读取 `docs/releases/v<主版本>.md` 通过 `--notes-file` 写入 Release。事后仅要求修正已有 Release 标题或正文时，将多行正文存入临时文件并通过 `gh release edit --title / --notes-file` 更新。版本 Tag 含 `-` 时，现有工作流将其标记为预发行版。
7. 仅修改文档、技能或标题时，不触发额外设备部署、截图或游戏补丁安装。需要设备工作且已授权时使用 [Decky 部署验证技能](../decky-deploy-verify/SKILL.md)。
