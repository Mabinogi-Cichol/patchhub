---
name: steam-patch-git-commit
description: 用户明确要求提交本项目 Git 改动时，核对变更与已有验证、按范围暂存并创建本地提交，处理仓库所有权和 index.lock 权限问题。不自动推送或改写历史。
---

# 本项目 Git 提交

适用于本技能所在的 `steam-hentai-patch-searcher` 仓库。项目根目录为本技能目录向上三级；不将此项目的信任配置套用到其他仓库。明确的“提交 GIT”请求授权必要的检查、暂存和本地提交；用户提供了提交说明时沿用其意图，否则根据实际内容拟定说明。

## 提交前

1. 阅读根目录 [AGENTS.md](../../../AGENTS.md)，按其适用范围处理提交：实际插件开发遵守功能/文档拆分要求，独立技能维护不需要 feature 文档。
2. 核对 `status --short`、工作区 diff 和已有暂存区，确定用户本次要提交的范围。保留不相关改动；不使用 `git add .` 或 `git add -A` 混入其他任务。
3. 沿用本次 session 已完成且对应文件未新增修改的验证，不重复运行。新增修改、失败或未解决问题只补相关检查；环境缺失或未执行项如实说明。提交前运行工作区 `diff --check`。

## 项目 Git 前缀

本 Windows checkout 历史上存在 ownership 检查。使用仅对当前命令有效的 `safe.directory`，不要修改全局信任列表。仓库移动时按已核实的新根目录调整。

```powershell
# 在已确认的项目根目录执行；不要在其他仓库照抄此路径
$steamPatchGit = @('-c', 'safe.directory=D:/Project Files/git/steam-hentai-patch-searcher')
& git @steamPatchGit status --short
& git @steamPatchGit diff --stat
& git @steamPatchGit diff --cached --stat
& git @steamPatchGit diff --check
```

每次 Git 调用检查退出码；失败后不继续暂存或提交。

## 提交说明语言

- 保留 Conventional Commits 标签，例如 `feat`、`fix`、`test`、`docs`、`refactor`、`build`、`chore`；可选 scope 沿用项目模块名。
- 标签及可选 scope 后，冒号后的提交描述必须使用中文；多行正文也以中文说明改动和验证。技术名称、标识符、文件名和路径可以保留原文。
- 本轮及后续提交均遵守此规则。用户提供提交说明时保留其意图，按此规则组织；不为统一语言而改写已有提交历史。

示例：

```text
feat(sources): 支持手动选择候选补丁
fix(kagura): 修复DLC候选漏查和镜像重复显示
test(sources): 补充候选保留与分页查询回归测试
docs(skills): 规定提交描述使用中文
```

## 暂存、复核与提交

1. 只暂存明确的文件；同一文件跨任务时按改动块暂存。若已有他人或其他任务的暂存内容，先判断是否属于用户授权范围，不擅自 reset 或包含进本次提交。
2. 根据当前适用的仓库规则划分提交。先完成当前提交的暂存，再检查 `diff --cached --check`、完整 staged diff 和文件清单，确认说明与内容一致，且未包含私钥、token、真实库材料或忽略目录的验收产物。
3. 创建本地提交。提交成功后读取 commit ID/标题并再次检查 `status --short`，区分工作区干净与仍保留其他改动。
4. 返回实际提交 ID、摘要、验证情况和剩余改动。除非用户明确要求，不推送、打 tag、amend 或改写历史。

多行提交说明以文件传入 `git commit -F`，避免把文本拼接为 shell 命令。

## 失败边界

- `detected dubious ownership`：确认 Git 操作指向本项目后，使用上述单命令 `safe.directory`。
- `.git/index.lock: Permission denied` 或已有锁：立即停止写操作，先核查执行权限或是否仍有 Git 进程持锁。不要自动删除锁、递归改 ACL、接管 `.git` 所有权或绕过活跃进程。问题解决后重新检查状态与暂存区，不能假定先前暂存成功。
- 受限工具执行失败：若本次提交已经明确授权，可通过现有权限机制请求执行必要 Git 操作；不因工具失败扩大文件范围或绕过审批拒绝。
- Git hook/提交失败：保留暂存内容、记录实际错误，在授权范围内修复或报告阻碍。不要关闭 hook、伪造成功或自动更换作者身份。

本文件是从个人记忆中的同名技能迁入的项目维护版本；后续修改和引用以此版本为准，不依赖记忆目录存在。
