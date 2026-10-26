# PatchHub 合并后 CI 修复施工记录

[设计](design.md)

## 状态

已实现并通过针对性自动化验证；按用户要求进行本地提交与 main 合并，实际提交 ID 以 Git 历史为准；未推送，完整在线 CI 待验证。

## 2026-10-26

### 线上证据

- [push run #70](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40121256739)，head 为 `c0501b3bc5c1e6136a7df05a76f756ef1fba3a4e`，结论 failure。
- changes、core (ubuntu-latest)、core (windows-latest) 成功。
- [Decky job](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40121256739/job/120953101884)：test_install_decky.py 共 30 项、14 个错误，均引用不存在的 PLUGIN_NAMES。
- [Windows job](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/40121256739/job/120953101927)：test_windows_update.py 共 7 项、5 项失败，报 Path escapes target；其后构建/打包未执行。
- 汇总 ci 因模块失败而失败，release 跳过；不属于新增独立故障。

### 实际修改与验证

- 从当前 main `f92e7dc` 建立 feature/patchhub-ci-fix，保留已有 README 提交。
- scripts/test_install_decky.py：FakeLoader 三处旧名称集合引用改为当前 NAME 的成员判断/过滤。
- scripts/windows-update.ps1：Safe-Path 对 Root 先执行 GetFullPath，保证前缀比较两端一致。
- scripts/test_windows_update.py：允许传入短事务路径，新增 GetShortPathNameW 真实短路径更新回归。
- 修复前：原生 Windows 普通路径 7 项通过；新增短路径用例失败并报 Path escapes target。WSL 安装 30 项中 14 个错误，与线上相符。
- 修复后：原生 Windows 更新 8 项全部通过（短路径用例实际执行，无跳过）；WSL Linux 安装 30 项全部通过；Windows Node 桥接 9 项全部通过。
- WSL 未安装 Node，桥接测试改在本机现有 Node 执行，通过；不将其记录为 WSL 验证。
- 按 overengineering-review 沿 Safe-Path 调用与 FakeLoader 设置变更路径复核：改动仅统一路径表示和清理夹具旧身份引用，未新增兼容层、重试或硬限制，未发现需修改项。
- git diff --check 通过。

## 待办

- 用户已要求本地提交并合并 main；Windows 更新修复、Decky 测试夹具修复分别提交，文档独立提交，沿用本 session 验证，不重复运行。
- [ ] 推送后检查新 head 的完整在线 CI（本轮未要求推送）。
- [ ] 完整在线构建、打包及附件上传待验证；本轮未部署或发布。
