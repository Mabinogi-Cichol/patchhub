# 运行时、RPC 与真实界面验证

## 1. 建立 loopback 隧道并选实际 target

沿用 deployment.md 的已核实 SSH 参数。用工具管理的会话保留隧道生命周期；若 Windows 用 `Start-Process`，设置 `-WindowStyle Hidden`。不要为验证公开设备调试端口。

```powershell
& ssh @deckSsh -N -o ExitOnForwardFailure=yes -L 127.0.0.1:18080:127.0.0.1:8080 $deckTarget
# 在另一个受管理的命令会话运行；下列路径相对于项目根
node .agents/skills/decky-deploy-verify/scripts/cef.mjs --list
```

默认 endpoint 为 `http://127.0.0.1:18080`；端口占用可改 `--endpoint` 并对应修改隧道。列表取自设备 CEF `/json/list`，选择实际返回的精确 title，不能假定固定 target ID。

- `SharedJSContext`：Decky/Steam 导航与已有 RPC 连接。
- `QuickAccess_*`：真实 QAM 侧栏的 DOM 与截图，名称每次重新读取。
- 实际 Steam 主窗口 target：全屏路由页，按本轮列表识别；不使用本地 HTML 预览冒充。

```powershell
node .agents/skills/decky-deploy-verify/scripts/cef.mjs --target SharedJSContext --expression-file .artifacts/<本轮>/probe.js --output .artifacts/<本轮>/runtime.json
node .agents/skills/decky-deploy-verify/scripts/cef.mjs --target '<列表中的精确 QAM title>' --screenshot .artifacts/<本轮>/qam.png --output .artifacts/<本轮>/qam-cdp.json
```

辅助脚本默认只列 target。表达式文件可以产生副作用，先审查是否只读或已获用户授权；不把历史验收脚本原样运行到新设备。工具不会自动调用 Loader、安装或修改游戏。

## 2. 后端与前端分别重载

在 SharedJSContext 先只读探测 `typeof DeckyPluginLoader`、插件列表及实际连接对象，不输出其 token/凭证。安装接口/内部连接版本可能改变，读取运行中对象或当前 Loader 实现，不猜 API。

- Python/CLI 更新：使用当前 Loader 的 `loader/reload_plugin` 或正式安装流程完成后端重启，检查新进程/导入日志。前端导入成功不足以证明新后端已运行。
- 前端更新：读取本轮 `plugin.json.name`、`package.json.version`，使用实际 `DeckyPluginLoader.importPlugin` 签名。历史有 `importPlugin(name, version, false, timeout)` 绕过排队的用法；先核对签名，设置有限超时，等待导入完成。不能将 Promise 提交当作完成。
- 检查插件实例严格为 1，`hasPlugin(name)` 为真（若当前 API 支持），服务 active。出现旧版标识/`old_frontend` 时核对 HTTP bundle 和运行中实际渲染行为。
- 只有正常导入仍无法卸载旧前端时，且无活动任务，才在部署授权范围内考虑 `SteamClient.Browser.RestartJSContext()`；该操作影响整个 Steam JS 上下文，先告知影响并保存基线。重启后旧 CDP 连接会断开，重新列 target，复核实例和新行为。不要直接重启整机。

示例只读 JS（运行前按实际身份调整）：

```javascript
(() => {
  const loader = window.DeckyPluginLoader;
  if (!loader || !Array.isArray(loader.plugins)) throw new Error('Loader unavailable');
  const name = 'Steam 补丁管理';
  return {
    instances: loader.plugins.filter(p => p.name === name).length,
    present: typeof loader.hasPlugin === 'function' ? loader.hasPlugin(name) : null
  };
})()
```

正式目录 JS 的 SHA-256、设备实际 HTTP 响应字节的 SHA-256、本轮构建 SHA-256 应一致。HTTP 资源 URL 从当前 Loader 或运行中页面读取，不能凭猜测访问固定路径；检查响应状态和字节，避免对错误页/压缩传输包装算哈希。运行中代码可能仍旧：还需检查本轮变更对应的函数/文案/界面行为。新文件已写入或 HTTP 字节一致均不能单独证明 React 已重新挂载。

## 3. 复用 Steam 已有连接执行真实 RPC

Decky Loader 的 `/ws` 可能与 Steam 前端单连接冲突。若直连立即关闭，不反复争抢或误报服务宕机；改用 SharedJSContext 已有前端连接。

历史项目使用内部 `window.__DECKY_SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED_deckyLoaderAPIInit.connect(2, pluginName)` 获取插件 API，并 `api.call('scan', false)`。它是版本相关内部入口；使用前核对当前对象与签名，优先现有插件 API，不把它视为公共稳定接口，也不要创建新的网络 `/ws`。

最低验证：

- `list_tasks` 可返回且不存在非终态任务；前置检查须在重载前完成。
- `scan(false)` 返回真实库结构，记录实际游戏/实例计数、诊断与与本功能相关字段；不要求与历史的 34/38 等计数相等。
- `list_transactions` 能读取真实记录；不为“清洁验收”删除旧数据库或 receipt。
- 修改了新 RPC 时实际调用并检查预期字段，如 `exe_installation_reviews(instance_id)`；按本轮扫描取 instance_id，不能硬编码历史样本或要求 profiles 永远为零。

`scan(false)` 不强制刷新来源，但可能更新派生缓存；按真实状态描述。来源刷新、开关、订阅或缓存清理验证需要相应授权，并保存/恢复本次测试改变的选择；不能把安装按钮点击当作普通 smoke test。实际准备/下载/安装游戏补丁另外按该功能范围执行。

## 4. 真实画面与交互

通过本轮实际 Steam API（历史 `OpenQuickAccessMenu()`、`DFL.Navigation.Navigate`）打开 QAM/全屏，选择真实插件。定位真实可见元素，先保存基线，再操作本次相关页面。

- QAM：默认筛选、搜索、可见计数、中文名称/图标、按钮换行、全局设置/重新扫描入口；验证关闭再打开后的筛选状态。独立搜索检查先切到“全部”并清除前一查询，避免旧筛选导致误报；测试结束恢复用户选择。
- 全屏：概览/补丁管理/操作记录/订阅源导航、路由参数、选中态和内容一致、返回路径。
- 顶部和底部：分别截图，记录 CSS viewport、scrollHeight/clientHeight、滚动位置、横向溢出及按钮可见性。实际 QAM 视口可能与全屏不同，不使用固定分辨率断言。
- 进度/取消改动：真实任务与合成任务分别标注。合成任务只验证渲染，不能证明真实下载速度、取消恢复或游戏补丁生效。
- 安装权限和来源引导：按当前候选实际 capability/profile 验证按钮；未审核来源不能出现正式安装入口，不沿用旧“零 profile”条件。

`Page.captureScreenshot` 必须在实际承载页面的 target 使用；检查图片本身，而不只看文件存在。若 helper 执行表达式返回 `{error: ...}` 而未抛异常，调用者仍需按失败处理；CDP 成功并不等于业务断言通过。

DevTools 导航、DOM 点击、脚本滚动是实机运行证据；只有用户/设备真实手柄操作才可标记实体手柄焦点、A/B 返回、滚动和操作验收。无法完成的项逐项写待验证。

完成后用实际 API 关闭本次菜单/弹窗，恢复本次改变的筛选和源选择，关闭本次 SSH 隧道；不要终止其他用户会话。

## 5. 结果归档

每轮 JSON/施工记录至少包括设备/时间、代码版本或工作区范围、清单/ZIP SHA、部署方式、备份位置、服务与插件实例、HTTP bundle SHA、RPC 结果摘要、截图及 viewport、明确待验收项、清理状态。认证 token、私钥和真实库原始路径仅在确有必要时留于忽略目录，不进入技能、日志摘要或 Git。

历史依据（仅说明流程来源）：[UX 施工记录](../../../../docs/features/decky-ux/progress.md)、[Kagura 验收](../../../../docs/features/kagura-hgamer-fallback/acceptance.md)、[EXE 施工记录](../../../../docs/features/exe-patch-installation/progress.md)。忽略目录 `.artifacts` 的旧脚本可供核对，但本技能不依赖它们存在。
