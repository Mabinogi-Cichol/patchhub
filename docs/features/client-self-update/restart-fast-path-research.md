# Decky 自更新“重启加载 Steam 客户端”环节调研

- 日期：2026-11-04。
- 状态：已采纳方案 A 并实现；本文件保留调研事实与候选比较，实施与验证见[施工记录](progress.md)。
- 触因：用户反馈 Decky 自更新完成后“重启加载 Steam 客户端”的逻辑拖沓，要求参考 [chenx-dust/DeckyClash](https://github.com/chenx-dust/DeckyClash) 的自更新重启实现。
- 关联：[设计](design.md) / [施工记录](progress.md) / [单行交互与客户端重启计划](decky-ui-restart-plan.md)。

## 1. 结论摘要

1. DeckyClash 的“重启”不是 Steam 客户端重启，而是 `systemctl restart plugin_loader.service`。它以插件进程内直接替换文件，最后重启 Decky Loader 收尾，没有独立 worker、没有包哈希/回滚事务、没有重启结果确认。
2. Decky Loader 启动时若发现 Steam 页面已注入过 Decky（`deckyHasLoaded`），会主动 `killall -s SIGTERM steamwebhelper`，由 Steam 重建 WebHelper，从而得到干净的页面并重新注入 Decky 前端。也就是说，DeckyClash 的重启链最终落到 **Steam UI（WebHelper）重载**，并非 Steam 进程重启。
3. 本项目当前链路在验证与清理之后调用 `SteamClient.User.StartRestart(true)`，这是 **整个 Steam 客户端重启**。它是当前更新耗时的主要来源（十秒级，具体分段耗时待埋点实测），也是更新完成后 QAM 与 Steam 界面整体消失的原因。
4. 三个层级的“重载”并非等价：Steam 客户端重启（本项）≫ WebHelper 重载/`RestartJSContext`（Decky 自身恢复用路径）＞ 单插件 `loader/import_plugin`（安装时已经发生）。
5. 选型结论：**已采纳方案 A**（`utilities/restart_webhelper`），保留事务化安装与验证，只替换末端动作；`StartRestart` 不再作为兜底路径，B（`RestartJSContext`）保留为实测对比项，未实现。

## 2. 调研方法与证据来源

| 对象 | 版本/提交 | 用途 |
| --- | --- | --- |
| DeckyClash | `main@0f8cc1d`（2026-10-24） | 参考实现 |
| decky-loader | `v3.3.1@ffd103b`（2026-10-27），与设备实际 Loader 一致 | 判定“重启/重载”的真实语义 |
| 本项目 | `main@e6ccfb7`（v0.5.4） | 现行实现 |
| 实机证据 | `.artifacts/debug-update-20261104-000029-c2e3a9d8/summary.json`、`progress.json` | 现行整链已实测事实（PID 72231 → 74654、boot ID 不变、新版实例加载） |

参考仓库均以只读方式 clone 到临时目录后静态阅读，未修改、未运行；本仓库仅新增文档。

## 3. DeckyClash 的自更新与重启链

实现全部在插件进程内完成（[upgrade.py](https://github.com/chenx-dust/DeckyClash/blob/main/py_modules/upgrade.py)）：

| 步骤 | 实现 | 说明 |
| --- | --- | --- |
| 版本发现 | `_VERSION_URL_MAP` + `version.txt` | 直接读 `releases/latest/download/version.txt`，不经 GitHub API，无速率限制问题 |
| 下载 | `utils.download_with_progress`（aiohttp） | 进程内下载，按 128 KiB 分块，`decky.emit('dl_plugin_progress', percent)` 推送进度 |
| 替换文件 | `recursive_chmod` → 备份 `bin/` → `shutil.rmtree(plugin_dir)` → 解压到临时目录 → `copytree` → 回填 `bin/` → `chown` | 直接写插件目录，无 Loader 安装 RPC、无包哈希校验、无事务日志 |
| 重启 | `await restart_plugin_loader()`：`asyncio.create_subprocess_shell("systemctl restart plugin_loader.service")` | 重启整个 Loader 服务；进程自身被 systemd 杀掉，调用方拿不到稳定回执 |
| 失败语义 | `_upgrade_tasks` 去重 + 异常原样抛出 | 已执行的 `rmtree` 不回滚；文件半替换后 Loader 仍按旧模块继续跑，直到下次重启才暴露 |

前端侧（[UpgradeItem.tsx](https://github.com/chenx-dust/DeckyClash/blob/main/src/components/UpgradeItem.tsx)）只订阅 `dl_*_progress` 事件显示百分比，`await backend.upgrade(...)` 的返回在 Loader 重启过程中通常不会到达；更新完成提示的 toast 因此不是可靠路径，靠重新连接后的版本查询恢复。

“重启后新版生效”的真正机制来自 Loader 而不是 Steam 客户端（decky-loader `v3.3.1`）：

- `backend/decky_loader/main.py:246-252`：`inject_javascript(tab, first=True)` 时若 `deckyHasLoaded` 已存在，则 `await restart_webhelper()` 并等待 1 秒后返回，等待重连后再注入。
- `backend/decky_loader/localplatform/localplatformlinux.py:232-236`：`restart_webhelper()` 即 `killall -s SIGTERM steamwebhelper`。
- `backend/decky_loader/main.py:185-252`：`loader_reinjector` 循环在 CEF 断开时调用 `handle_crash()`（`:225-226`），随后重连新标签页并在无 `deckyHasLoaded` 时注入 Decky 前端（`:214`）。
- `frontend/src/plugin-loader.tsx:480`：插件前端以 `import('http://127.0.0.1:1337/plugins/<name>/dist/index.js?t=' + Date.now())` 载入，天然绕过缓存，重注入即加载新 bundle。

因此 DeckyClash 的“重启很快”不是因为它更聪明地重启了 Steam，而是因为它只做了一次 WebHelper 重载；Steam 进程、游戏会话、下载任务都没有被打断。

## 4. 本项目当前链

现行链（[scripts/install-decky.py](../../../scripts/install-decky.py)、[apps/decky/backend/app_update.py](../../../apps/decky/backend/app_update.py)、[apps/decky/src/app-update.tsx](../../../apps/decky/src/app-update.tsx)）：

| 步骤 | 代码点 | 备注 |
| --- | --- | --- |
| 点击更新 | `app_update.py:165 start()` | 写 `request.json`，`Popen` 独立 worker，宿主立即返回快照 |
| 取消业务 | `main.py:296 _begin_app_update()`、`app_update.py`、`install-decky.py:621 prepare_app_update()` | 与下载并行；CEF 每 0.5 s 轮询 `status` |
| 下载/校验 | `install-decky.py:175 release_package()` | 字节进度写入状态文件 |
| Loader 安装 | `install-decky.py:662 mutate()`、`:643 settle()` | 桥接 job + 0.5 s 轮询；Loader 内部 `_install` 另有 `sleep(1)`（`browser.py:290`） |
| 设置恢复 | `install-decky.py:702 restore_settings()` | 第二次 Loader mutate |
| 运行验证 | `install-decky.py:678 verify()` | 校验版本/实例/RPC/HTTP bundle 哈希，0.5 s 重试；此时新版前端已由 Loader 的 `loader/import_plugin` 导入当前页面 |
| 收尾 | `install-decky.py:713 finish()` → `loader.call('finish_app_update')`（`:948`） | 释放 guard、清理事务目录 |
| 重启 | `install-decky.py:52 restart_client()` → `SteamClient.User.StartRestart(true)`（`:60`） | **整个 Steam 客户端重启** |
| 重启重试 | `install-decky.py:897`、`main.py:287` | 只重发重启请求，不重新下载安装 |
| 前端感知 | `app-update.tsx:16` 每秒轮询；`:84` 用 `performance.timeOrigin` 判定“同一页面仍在等重启” | 新页面上下文不同即回到“检查更新” |

已实测事实（2026-11-04 整链）：安装与验证通过、`restart: requested`、Steam PID 72231 → 74654、boot ID 不变、新版实例唯一且页面上下文更换。即当前实现的正确性没问题，问题只在“终点动作太重”。

需要并列记录的事实：`loader.verify()` 通过本身已证明新版前端模块被导入了当前页面（否则前端实例版本不等于新版本）。而 [progress.md 2026-11-01 调查](progress.md) 记录的是“导入不等于已打开视图换新”，用户当时确实需要退出重开页面。两者并不矛盾，但说明“到底需不需要一次页面级重载”本身是待实测项，而不是既定前提；见第 7 节。

## 5. 差异与耗时结构

| 维度 | DeckyClash | 本项目 |
| --- | --- | --- |
| 终点动作 | Loader 服务重启 → WebHelper 重载 | Steam 客户端整体重启 |
| 影响面 | Steam UI 重载；所有插件后端重启；游戏会话保留 | Steam 进程重启；UI 全部消失后重登；游戏/下载可能被打断 |
| 可确认性 | 无（进程被杀，fire-and-forget） | 有（先持久化 `restart: requested`，新页面按上下文判定；失败可单独重试） |
| 事务/回滚 | 无 | 有（journal、备份、回滚、恢复续做） |
| 进度通道 | `decky.emit` 推送 | 独立进程状态文件 + 1 s 轮询 |
| 耗时主项 | 下载 + 解压 + Loader/WebHelper 重启 | 下载 + 校验 + Loader 安装 + 验证 + **Steam 客户端重启** |

耗时结构判断：前段（下载 8 MB 级、哈希、Loader 安装、验证）为秒级；末端 `StartRestart` 是唯一以十秒计的阶段，用户感知的“拖沓”与 QAM 长时间消失基本由它贡献。具体分段耗时本项目尚无埋点数据，列为待实测项。

## 6. 候选方案

| 方案 | 动作 | 预期体验 | 影响面与代价 | 风险/未知 |
| --- | --- | --- | --- | --- |
| A（参考 DeckyClash 的落点） | 走 Loader 路由 `utilities/restart_webhelper` | Steam UI 重载，秒级；页面与插件前端重注入 | 不动插件后端；所有插件前端内存态丢失一次（等同 UI 重载） | 该路由调用会立刻打断自身 WebSocket，只能拿到“已请求”；需确认重注入稳定 |
| B（更贴近 Steam 语义） | CDP 执行 `SteamClient.Browser.RestartJSContext()` | 同上，可能更快，不杀 WebHelper | 同上 | 需实测 Decky 是否在 `Inspector.detached` 后自动重注入；Steam 侧异常时无回执 |
| C（DeckyClash 原样） | `systemctl restart plugin_loader.service` | 秒级；顺带触发 WebHelper 重载 | 我们自己的后端与全部插件后端一起重启 | 与设计“本插件重载不应重启整个 Loader、不影响其他插件”的边界冲突；能否以非特权进程执行该重启需实测；失败回执更弱 |
| D（保留现状 + 提前） | 仍在验证通过后立刻 `StartRestart`，把清理放到请求之后 | 只省下收尾的秒级开销 | 无结构变化 | 治标；客户端重启的数十秒仍在 |

推荐：**A 为主、B 作为备选实现、`StartRestart` 降级为回退**。理由是 A/B 都保持了“页面真的换新”这一当初引入重启的目的（见 [progress.md 2026-11-01 调查](progress.md)：Loader 卸载并重导前端不保证已打开页面使用新组件），同时把成本从“客户端重启”降到“UI 重载”；而 C 与我们既有的“只重载本插件、不影响其他插件后端”的边界相冲突。

不建议引入的部分：DeckyClash 的无校验直写、无回滚、fire-and-forget 语义，以及“用 Loader 重启代替结果确认”。我们的安装事务和验证链应原样保留，替换范围只限末端重启动作与其触发时机。

## 7. 待实测项（决定最终选型）

1. **是否还需要页面级重载**：先做零重载对照——更新完成后不做任何重载，只关闭并重新打开 PatchHub 页面，观察设置页与 QAM 面板是否已是新组件（`loader.verify()` 已证明新模块被导入）。若确认必现旧界面，再进入 A/B 实测；判定标准是新页面出现、PatchHub 设置页与 QAM 面板显示新版本与新组件（不能只看 RPC 版本号）。
2. **重注入可靠性**：重载后 Decky 前端是否在可接受时间内恢复（`plugin_loader` 日志出现重新注入、`deckyHasLoaded` 重新建立）；连续两次重载是否落入 `handle_crash` 的 60 s 内连击计数。
3. **副作用边界**：游戏运行中重载是否影响游戏；其他插件的前后端状态是否只丢前端内存态；正在下载的 Steam 内容是否中断（客户端重启会中断，UI 重载不应中断）。
4. **回执与重试语义**：A/B 调用会断开 WebSocket，需确认“已请求但未确认”与现有 `restart: requested` 记录方式一致，且失败后“重试重载”按钮仍可用；不得把断连写回安装失败。
5. **分段耗时**：给 worker 增加每阶段单调时钟记录（仅用于本次测量，是否保留另行决定），量化下载、Loader 安装、验证、重载各自耗时，避免凭感觉优化。
6. **取消/恢复路径不变**：失败回滚、`--recover`、`restart_only` 重试在更换终点动作后行为不变。

## 8. 实际改动范围（方案 A）

- `scripts/install-decky.py`：`restart_client()` → `reload_client_ui()`，先持久化 `completed` + `restart: requested`，再经桥接调用 `reload_ui`；`restart_only` 重试路径复用同一函数；CEF/WS 断开仍视为“已受理未确认”，Loader 明确报错才记为失败。
- `scripts/install-decky.py` 桥接：新增 `reload_ui` op，仅调用 `utilities/restart_webhelper`；该路由在受支持的 Loader 3.2.9 / 3.3.0 / 3.3.1 均存在，因此没有版本分支，也没有 `StartRestart` 兜底。
- `apps/decky/src/app-update-view.tsx`、`app-update.tsx`：`restarting` 文案改为“正在重新加载界面”，失败入口改为“重新加载界面”，错误说明改为“更新已安装，界面重新加载失败”；`timeOrigin` 判定逻辑未改。
- 测试：`scripts/test_install_decky.py` 改为断言 `reload_ui` 调用、先持久化与失败不改写安装结果；`scripts/test-install-decky-bridge.mjs` 增加委派与拒绝两条夹具；`apps/decky/tests/app-update.test.ts` 同步文案断言。
- 状态字段 `restart` / `restart_context` 与 RPC 名 `restart_app_update_client` 保持不变，原因见[设计](design.md#63-更新完成后的界面重载2026-11-04)。

## 9. 未决问题

- 方案 B（`RestartJSContext`）在 SteamOS 客户端上的实际表现与 Decky 重注入时序尚无证据，未实现；只有在实机证明 A 的重载不可靠时才考虑。
- 是否需要在重载前显式提示“界面将短暂重新加载”，取决于实测中断时长；不预设固定倒计时。

## 10. 实机验证结果（2026-11-04）

设备侧已验证机制：经 Loader 路由 `utilities/restart_webhelper` 触发一次重载后，CEF target 全部重建、Decky 重新注入、插件实例恢复为 1、HTTP bundle 保持为新构建哈希，设置页重新打开即渲染新按钮布局；而在此之前，Loader 安装完成但未重载时，已打开页面仍显示旧按钮（实机测量按钮宽 240 px 未更新）。这与第 7 节第 1 项的零重载对照结论一致：**新模块已导入不等于已打开视图换新，页面级重载仍然必要**，A 的重载强度足够。完整自更新整链（下载 → 安装 → 末端重载）仍待单独验收，详见[施工记录](progress.md)。
