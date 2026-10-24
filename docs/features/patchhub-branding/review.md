# PatchHub 实现与过度设计复核

[设计](design.md) · [施工记录](progress.md)

2026-10-25，基线 `75cc867`，对象为当前全面改名差异。使用 overengineering-review 技能进行静态执行路径复核，并结合本轮自动化与本地原生证据。结论：未发现需新增设计机制的事项；实机和正式发布边界仍见施工记录。

## 核对路径

- Windows CMake 主程序名、Runner 元数据、Flutter package/import、窗口标题、桌面方法通道和 CorePaths 相互一致。GUI patchhub.exe 与 CLI patchhub-core.exe 分离，避免覆盖。
- Dart AppUpdate 与 windows-update.ps1、NSIS、update_manifest.py 使用同一维护锁、更新标记、程序名及受管理文件契约；更新助手只接受 PatchHub 注册身份，不发现旧产品注册项。
- Decky plugin.json/name 与前端 name/title、Python 宿主、终端安装器 BRIDGE 固定身份一致；FOLDER、ZIP 顶层和 package 校验使用 patchhub/@patchhub/decky。运行时仍核对新 CLI/worker 的大小、哈希和执行权限。
- 删除仅服务于跨品牌切换的旧名集合、元数据名称兜底、设置名称替换、跨名卸载与专属测试。保留同一身份的安装前任务检查、更新写门禁、真实失败恢复和其他插件设置保护；这些由运行中替换场景支持。
- Cargo crate/bin、Rust imports、Cargo.lock，npm workspace/scoped imports 与 package-lock，Flutter package 与 pubspec 保持一致；生成契约检查、两端构建和真实更新握手提供执行证据。
- Logo 直接使用已有 Rollup 资产处理，不添加品牌配置层或图片依赖；CI selector 覆盖共享 Logo 构建输入，普通文档仍不触发重构建。

## 有意保留项与未验收边界

- 历史发布/施工证据、第三方 Steam 名称、项目技能调用 ID 及本机现有 checkout 路径不是漏改的产品身份，不进行全盘替换。
- 新安装使用新数据目录，未复制或删除用户旧数据；本地候选使用隔离 APPDATA，不替换用户现有安装。
- 远端仓库名称已验证；源码推送、首个 PatchHub Release、公开下载安装、Windows 注册安装与 Decky 实机未执行，不能由本地构建推导为通过。
