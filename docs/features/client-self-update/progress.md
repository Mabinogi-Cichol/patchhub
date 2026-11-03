# Windows 与 Decky 客户端自更新施工记录

- 设计：[design.md](design.md)。
- 当前状态：**Decky 单行更新候选已部署，原生页面、RPC 与真实 Steam 客户端重启验证通过；完整 UI 跨版本更新及实体手柄待验收。双端自更新已实现；本轮 Decky 更新阻断、进度与完成提示修复经自动化、用户真实更新和原生界面验证通过，已随 v0.5.1 发布。Windows 完整安装版矩阵、正式版本间更新及实体手柄验收仍待完成。**
- 修复归档分支：`fix/decky-self-update`，基于 `main@37f50fd`；原实现分支 `feature/client-self-update`（基于 `main@81a2965`）的历史记录保留。

## 2026-11-03：提交与本地合并安排

- 用户要求先提交当前改动并合并本地 main，再创建新会话实现 debug 本地更新源与一键测试脚本。
- 自更新代码及必要测试提交为 `063c69f`；功能设计/施工记录与部署技能分别提交，不包含忽略目录中的设备数据、安装包或截图。
- 沿用本轮已完成且输入未变的定向验证与设备记录，不重跑完整 CI 或无关构建；提交前核对差异、暂存范围和文档链接。后台普通检查间隔按实际 `INTERVAL = 24 * 60 * 60` 更正文档，启动检查绕过该间隔的行为不变。
- 本轮仅本地提交并将 main 快进至当前分支，不推送、不打 Tag、不发布。完整 UI 跨版本自更新和实体手柄仍待验收；后续 debug 自更新源尚未实现，交由用户指定的新会话推进。

## 2026-11-03：启动检查与原生 tips 通知

- 用户追加要求启动插件时检查更新并使用原生 tips。沿用当前功能分支和更新入口，不新增配置或通知服务。
- 原后台循环检查可能受24 小时间隔限制，前端启动只领取旧通知且每分钟才再次领取。现前端初始化立即调用 `check_app_update`，等待本次检查完成后才调用 `claim_app_update_notice`，发现新 Tag 使用现有 `toaster.toast`，点击进入更新二级页。
- 后端继续复用在途检查和 GitHub 限流 `retry_at`，不重复发起并行检查；同一版本通知沿用持久化去重。没有新版不提示，后台检查失败记日志，不打断插件主界面。通知读取改为串行检查结束后安排下次读取，卸载后不领取/显示迟到通知，清理计时器。
- 本地 typecheck、8 项更新组件/交互回归通过；包含启动检查→通知顺序、原生 toast 点击跳转、无新版静默及卸载中断。前端构建通过。设备增量替换 `dist/index.js` 与 map，未创建备份，文件及 HTTP bundle 哈希一致：`9b8e1b102ced422a3ffa63f78442d25266f139c1b02ef65844c978d95a73ab9d`。

- 通过 Loader 正常重新导入前端后，唯一实例为 0.5.3、服务 active；`last_attempt_at` 从 `1793717531.075253` 推进至 `1793717728.664066`，间隔约 198 秒，证明启动检查未受24 小时间隔跳过。真实状态 `up_to_date / v0.5.3`，没有多余通知。
- 设备证据 `.artifacts/decky-update-startup-20261103/`：`manifest.json`、`predeploy.json`、`reloaded.json`、`verified.json`、`check-completed.json`。远端暂存已清理，SSH 隧道已关闭。未在真实设备伪造新 Tag；有新版原生 tips 与点击跳转由本轮组件回归覆盖，真实更高版本提示与实体手柄仍待验收。

## 2026-11-03：候选设备部署与 Steam 客户端重启验证

- 用户授权部署当前候选；采用 `decky-deploy-verify` 技能。Devkit 当前所选设备经既有可信 SSH 主机密钥校验，部署前插件 0.5.2、Loader 3.3.1，补丁历史任务 45 条、来源任务 0，均无活动任务。
- 实际 Loader 为 3.3.1，核对官方 [browser.py](https://github.com/SteamDeckHomebrew/decky-loader/blob/v3.3.1/backend/decky_loader/browser.py) 与 [utilities.py](https://github.com/SteamDeckHomebrew/decky-loader/blob/v3.3.1/backend/decky_loader/utilities.py) 的本地 ZIP、安装请求/确认事件及参数契约后，将 3.3.1 纳入现有适配集合。新增针对性桥接测试后 12 项通过；未放开未知 Loader 版本。
- 使用仓库 `.tools/linux-cargo` / `.tools/linux-rustup` 在 WSL 构建当前 0.5.3 Linux Core/worker（`cargo build --locked --release -p patchhub-cli -p patchhub-installer-worker --bin patchhub-core --bin patchhub-worker`），重新生成当前锁文件 SBOM，沿用此前已通过前端验证；完成生产装包。
- 实机发现 SharedJSContext 与 Steam 主页面的 `performance.timeOrigin` 不同。修正为由按钮所在页面把标记传入 start/restart RPC，宿主原样转交独立安装器，避免用后台上下文标记判断主页面是否已重启。修正后前端 5 项、typecheck、构建、Linux 安装器 35 项及更新宿主 11 项通过；增加前端/宿主参数传递断言并复测。
- 最终候选 ZIP：7,910,854 字节，SHA-256 `6b5354ce9a60fb06b0febeb11b65a72a209fb5d19c0f7a1f78d172852d4251ba`。设备整包与逐成员验证通过，经真实 Loader 确认事件完成安装；正式 27 文件哈希/大小一致，Core/worker 均可执行。HTTP bundle 与本地最终构建哈希一致：`66fdb3e4da876dc8e54961f67ffc4d84a415dcd0218f2a4a1a20495724ded951`。
- 新版后端及前端均 0.5.3、实例 1、Loader active。真实 `scan()` 返回 27 个游戏实例，任务 45 条、事务 25 条，无活动补丁/来源任务；本仓库当前 scan RPC 无参数，最初尝试 `scan(false)` 的签名错误已按实际接口修正，不计为功能失败。Loader 安装移除的本插件排序已仅恢复 PatchHub 原位置，其他插件设置保留。
- 通过独立状态文件运行生产 `restart_client` 函数完成真实客户端重启验证，未伪造或覆盖产品更新事务结果。Steam PID 3239 → 53238，系统 boot ID 保持不变；新客户端自动加载唯一 0.5.3 实例、真实 RPC 与最终 bundle 再次通过。此项证明客户端重启及新版加载，不等同于完整 UI 跨版本自更新链验收。
- 原生画面：Steam 实际 CSS viewport 854×534，CDP 截图文件 1281×801；更新区域仅“当前版本 0.5.3 + 检查更新”一行，无横向溢出，按钮滚动进入视口后 240×44 CSS px。真实 DOM 点击检查更新，原位置呈现“检查中…”后恢复按钮，通知显示已是最新正式版。QAM 实际打开 PatchHub 面板，无重复更新区域。均已查看截图；实体手柄未操作。
- 用户明确调整调试部署策略：默认不创建、不保留旧版备份，已固化至部署技能及关联参考。首次部署曾按旧规则生成备份，现已按本轮指示删除本地旧版 ZIP、归档及对应清单；设备本轮暂存目录与其中旧版备份全部清理，不删除其他历史目录。候选清单、验证 JSON 和截图继续保留。
- 设备证据：`.artifacts/decky-update-ui-restart-20261103/summary.json`、`manifest.json`、`final-device.txt`、`post-restart-rpc.json`、`restart-smoke.json`、`settings-checked.png`、`qam-plugin.png`。验证后关闭本轮菜单并恢复原 Steam 网络设置页，关闭本轮 SSH 隧道。
- 尚待验收：从实际旧版本的更新按钮完成完整下载→百分比→安装→自动客户端重启链；实体手柄焦点/返回。当前候选与公开最新发行均为 0.5.3，未伪造高版本候选或改写产品更新记录。未提交、合并、推送、Tag 或 Release。

## 2026-11-03：Decky 单行更新与客户端重启施工

- 分支 `fix/decky-update-ui-restart`，基线 `main@f5488d8`；保留此前计划文档改动，未提交、合并、推送或部署。范围见[专项计划](decky-ui-restart-plan.md)。本轮代码已实现，设备验收待完成。
- 前端更新区域改为一行当前运行版本与唯一控件。检查/更新/恢复/重启重试共用该位置；点击后立即显示百分比，保持同一个原生 DialogButton 以保留焦点，不再自动弹进度窗口或展示多行详情。清理旧 Modal、重复进度、取消/稍后/跳过等界面专属逻辑；后台检测与通知机制保留。
- 安装器发布整体进度：真实 ZIP 字节占 0–80%，校验完成 85%、备份完成 90%、安装完成 95%、运行验证后进入清理 99%，全部收尾后 100%。可信总量优先取冻结 Release 附件大小，无总量不模拟增长；清单/API 下载不影响 ZIP 进度。前端约 1 秒串行读取，临时断连保留最后进度。
- 当前 Decky UI 依赖的 `User.ts` 明确提供客户端重启签名 `StartRestart(force: boolean): void`。独立执行者在正常 GUI 安装、运行验证及门禁收尾后，先持久化 installed/completed 结果，再通过既有 CDP 调用 `SteamClient.User.StartRestart(true)`；未调用 reboot、shutdown 或替代为 Loader/WebHelper 重启。历史结果查询不发起重启；CLI、检查模式、失败回滚和恢复清理不进入该调用。
- 增加显式重启重试 RPC；只接受安装成功但重启失败结果，不重新下载包。重启进程退出、启动失败或客户端 API 错误保留已安装版本；调用造成的 CEF 关闭/断开仅表示请求未获回执，不将安装改写为失败。无定时重试、倒计时或循环重启。
- 手工完整路径与过度设计复核见[复核记录](review.md)。SharedJSContext `performance.timeOrigin` 与插件渲染上下文的一致性、实际客户端进程重建、boot ID 不变、新组件加载均待实机确认。

### 本轮定向验证

| 平台/范围 | 命令 | 结果 |
| --- | --- | --- |
| Windows / 前端测试 | `npm test -w apps/decky` | 107 项通过；最后新增失败状态文案后更新相关 5 项再次通过 |
| Windows / 类型 | `npm run typecheck -w apps/decky` | 通过 |
| Windows / 构建 | `npm run build -w apps/decky` | 最终源码构建通过，版本同步无变更；产物 `apps/decky/dist/index.js` |
| WSL Ubuntu / 安装器 | `python3 -m unittest discover -s scripts -p test_install_decky.py` | 最终 35 项通过；包括实际安装/恢复夹具、整体进度、重启前持久化、预期断连与重启接口失败 |
| WSL Ubuntu / 更新宿主 | `python3 -m unittest discover -s apps/decky/backend -p "test_app_update*.py"` | 11 项通过；含独立执行者跨宿主退出/目录移除存活和重启重试 |
| Windows / Loader 桥接 | `node scripts/test-install-decky-bridge.mjs` | 11 项通过 |
| 文档与差异 | `git diff --check`、新增文档相对链接核对 | 通过 |
| Windows / Python 补充尝试 | `python -m unittest discover -s scripts -p test_install_decky.py` | 当时 34 项中 25 项依平台跳过；后续 Windows 宿主回归未完成，已停止该会话，不计为通过；改在目标 Linux 环境完成上列回归 |
| 实机与发行 | 未执行 | 未部署、未重启设备、未打包发布；原生 1280×800、实体手柄、从按钮完整更新与客户端重启、新页面加载、其他插件恢复均待验收；未运行发布完整矩阵 |

## 2026-11-03：Decky 单行更新与客户端重启计划（待实现）

- 用户要求二级页面只保留当前版本与状态按钮，点击更新后按钮原位变为实时百分比；完成后强制重启 Steam 客户端以加载新版插件。
- 在干净的 `main@f5488d8` 上创建 `fix/decky-update-ui-restart`，复用本功能目录，新增[专项计划](decky-ui-restart-plan.md)，同步设计与索引。未修改运行代码，未提交、部署或重启设备。
- 已静态核对 `app-update.tsx`、`app-update-view.tsx` 和安装器成功收尾路径，确认现状有重复进度与自动弹窗，独立安装器具备跨插件卸载的执行边界。
- 官方 Loader 3.3.0 `updater.py` 的更新重启调用重启 Loader 服务，`utilities.py` 另提供 WebHelper 重启；尚未核实满足完整 Steam 客户端重启的具体接口与实机效果，列为 P0。来源链接见专项计划。
- 计划级过度设计审阅完成；定向测试、构建、设备进程及 boot ID、实体手柄、新版页面加载全部待实施验证。本轮仅检查文档差异与相对链接，不重复执行代码构建。

## 2026-11-01：v0.5.1 已合并发布

- 修复和配套文档已分项提交并合并 main，Tag v0.5.1 固定指向 `f66d0dd`；[Tag CI](https://github.com/Mabinogi-Cichol/patchhub/actions/runs/41296277719) 全部通过，[正式 Release](https://github.com/Mabinogi-Cichol/patchhub/releases/tag/v0.5.1) 已公开。
- 三个安装包及 SHA256SUMS.txt 已下载核验，大小/摘要、ZIP 白名单/CRC/Tag 来源、平台架构、包内版本及 SBOM 通过；Windows 正式便携包隔离启动及正常关闭通过。详见[发行记录](../../release-checklist.md#v051-发布记录2026-11-01)。
- 未部署正式 v0.5.1 包或重跑设备更新；此前源码候选与用户测试记录保持原边界，实体手柄和 Windows 安装版完整矩阵仍待验收。

## 2026-11-01：v0.5.1 合并与发布准备

- 用户要求将本轮修复合并至 main 并发布 v0.5.1。当前分支为 `fix/decky-self-update`，远端 main 与本地 main 均为 `37f50fd`，远端尚无 v0.5.1 Tag。
- 提交范围仅为本轮 Decky 自更新、进度界面、完成提示及必要测试/文档；代码、功能文档、版本同步和发行文档分别提交。沿用前述 148 项完整回归、最后提示修改的 5 项针对性回归、构建、真实 0.4.0 → 0.5.0 更新和 0.5.0 提示候选实机结果。
- 代码提交 `52f28b7`。应用版本源修改为 0.5.1，使用项目同步脚本生成全部派生版本；版本同步回归 12 项及 `--check` 通过，仅工作区项目版本字段改变，第三方依赖未变。发行说明见 [v0.5.1](../../releases/v0.5.1.md)。正式 v0.5.1 发行附件尚未构建/核验，既有测试候选不替代正式附件验收。

## 2026-11-01：用户真实更新通过与完成提示修正

- 用户确认 0.4.0 测试候选已经完成更新至 v0.5.0，但当前页面需要退出重开。设备 `result.json` 为 `completed / v0.5.0`，正式包/Loader 实例均 0.5.0、Loader active；日志确认 17:50:17 开始安装、17:50:22 导入新版。因此这次完整真实自更新通过，当前页面是否重新挂载须独立判定。
- 实机调查：已有 Loader 接口会卸载插件实例并重新导入前端，同时重新注册路由；这不保证 Steam 已打开页面使用新组件。通过临时路由渲染标记及同路径版本查询参数导航检查，未取得可靠的新组件挂载证据；探针随后恢复原路由、退出测试页并删除，不将导航 URL 变化等同于新页面加载。证据 `.artifacts/decky-self-update-fix/reload-*.json`。
- 按用户允许的替代方案修改提示：按钮改为“更新插件”；安装阶段不再宣称完成后界面自动重载；设置页和弹窗完成文案统一为“更新已完成。请退出当前插件页面，再从 Decky 重新打开 PatchHub，以加载新版界面。” 保留真实后台安装/验证流程，没有新增重启、重试或 Steam 内部缓存操作。
- 针对本次前端修改运行更新组件/交互回归 **5 项通过**；类型检查、前端构建、生产插件装包和 ZIP 验证通过。既有 148 项完整回归及用户实际更新结果沿用，不重复运行无关测试。
- 保持 0.5.0 制作并经 Loader 部署提示修正候选，沿用用户此前不备份的要求；部署前补丁历史任务 43 条且无活动任务，来源任务 0。包 `.artifacts/decky-self-update-fix/v050-completion-candidate/patchhub-decky.zip` 为 8,218,215 字节，SHA-256 `f38696dcae8f73e6c67e3ab3956abdea8e903ec287b7d84837092ef58205a0b3`，上传哈希一致。
- 部署验证通过：30 个正式文件逐项大小/哈希及二进制执行位一致，Loader active、实际前/后端均 0.5.0、插件实例 1、Core RPC 正常。HTTP bundle 与本轮产物一致，SHA-256 `c84786a3b3e84259d7592ddbcc376bdb5d9866ff434aa6f210bc3e85cb5ddeca`。保留用户这次真实 completed 结果，未重新启动自更新。
- 真实 Steam 界面重开后显示完整新版完成提示，检查无横向溢出，截图已查看：`v050-completion-candidate/completed-final.png`；DOM 证据 `completion-visible.json`。已恢复原 Loader 设置、释放安装保护，仅清理本轮远端暂存目录，并关闭 SSH 隧道。未提交/合并/发布；当前提示方案需要用户退出重开，不能声称自动页面刷新已实现。

## 2026-11-01：部署 0.4.0 自更新测试候选

- 用户要求将修复继续标识为 v0.4.0 部署到已确认 Steam Deck，供用户自行测试更新到 v0.5.0；随后明确要求**不备份**。按当前指示直接经 Loader 安装，未创建本轮插件备份或回滚包。沿用已确认的 Devkit 配对/SSH 指纹。
- 设备与前置检查：Loader 3.3.0 / active，原插件标识 0.4.0；43 个补丁历史任务均终态，来源任务 0，更新无需恢复。Loader 设置原样记录并仅合并恢复 PatchHub 成员与位置。
- 构建过程中的实际问题：首次误用正式 v0.4.0 底包，无法读取设备已经由后续开发版写入的来源状态，启动返回 `io_error`；改用当前代码后又确认本地缓存 Linux Core 旧于当前生成协议，启动返回 `capability_mismatch`。这些候选均未作为完成结果。没有删除、回退或手改业务状态；安装保护持续保留，阻止交叉业务写入。
- 最终候选：从当前源码重新编译 Linux Core/worker，使用当前生产 `CoreBridge.preflight` 在隔离状态中验证协议 v5、24 个方法通过；重新构建前端并按生产 allowlist 装包。测试包的 `package.json` 版本及应用 SBOM 版本设为 0.4.0，来源为当前开发代码，**不是正式 v0.4.0 原样发行包**；仓库 `Cargo.toml` 与客户端源元数据仍保持 0.5.0。完整 Cargo/npm 依赖元数据、许可和锁文件清单核验通过。
- 最终包 `.artifacts/decky-self-update-fix/current-v040-candidate/patchhub-decky-v0.4.0-update-fix.zip`：8,218,065 字节，SHA-256 `ccdcfb9bf300ab9dfa7b954b680e47524b4b292b3ade2be4862b578198a0ece8`。设备上传哈希一致，通过已有 Steam/Decky 连接关联安装确认事件并完成真实 Loader 安装，不直接替换 root 插件目录或结束插件进程。
- 正式目录 30 个文件逐项大小/SHA-256 通过，Linux 二进制可执行，Python 语法通过；实际 HTTP bundle 与本轮文件一致，SHA-256 `fb73ba89a3917c6a12f48d9cfeeab9e6f6d9c41eac2ac2b20e675e82c718d51e`。真实前端/后端版本均为 0.4.0、插件实例 1、Core RPC 正常、Loader active。
- 真实 RPC：`scan()` 返回 27 个安装实例，`list_tasks` 43 条、活动任务 0，能力协议 v5 / 方法 24；`get_app_update_status` 为 current=0.4.0、tag=v0.5.0、available、无需恢复。保留此前 `pending_plugin_write` 失败记录，下一次用户启动更新时由真实新任务覆盖，未伪造 completed。
- 原生界面：重新打开全局设置，检查当前 0.4.0 与可用 v0.5.0，更新按钮可用且无横向溢出；已滚动到“更新并重载插件”按钮，**未点击、未启动实际自更新**。查看真实截图 `settings-final.png` / `update-button.png`，原始 RPC/DOM 证据见同目录 JSON；DOM 自动探测不能读取跨上下文 React 函数源码，因此不将该探测当作进度弹窗运行证据。
- 已恢复原 Loader 排序/显示设置，释放本轮写入保护；仅清理经绝对路径核对的本轮远端暂存目录，关闭 SSH 隧道。本地候选和证据保留。未提交、未合并或发布；完整 0.4.0 → 0.5.0 自更新、进度弹窗运行及实体手柄验收留给用户本次测试。

## 2026-11-01：Decky 自更新修复（已实现及本地验证，实机待验收）

- 修复分支 `fix/decky-self-update`；[专项方案](decky-fix-plan.md)。工作区起始干净。
- 实机只读确认：当前插件 0.4.0，候选 v0.5.0；失败结果为 `pending_plugin_write` 且已恢复安装前状态。Steam 现有连接中存在多个 `claim_app_update_notice` 在途调用，安装器遗漏该允许方法，误阻止更新。原始证据 `.artifacts/decky-self-update-fix/baseline.json`。
- 已实现：安装器允许在途 `claim_app_update_notice`，保留真实业务写入/Loader 安装互斥；独立执行者报告下载、校验、备份、安装、验证、恢复和清理阶段。下载统计只对应 ZIP，不再被 API/校验清单请求覆盖。
- 已实现：原生更新进度弹窗与设置页内进度条；已知总量显示真实百分比/字节，未知总量和其他阶段显示不定进度。收起不取消，下载取消独立操作；恢复启动立即标记 applying，禁止误用取消下载。前端重载后读取外部结果，活动更新自动重开进度；临时读取异常不标记安装失败。轮询顺序执行，组件卸载清理计时器和弹窗。
- 验证通过：`npm test -w apps/decky` **95 项**；`node --test scripts/test-install-decky-bridge.mjs` **11 项**；WSL Ubuntu `python3 -m unittest discover -s scripts -p test_install_decky.py` **32 项**；`python3 -m unittest discover -s apps/decky/backend -p 'test_app_update*.py'` **10 项**，均无跳过。合计 **148 项**。`npm run typecheck -w apps/decky`、`npm run build:decky`、`git diff --check` 通过。
- 新回归覆盖：保留在途提醒调用时成功安装/释放保护，同时仍拒绝真实写入；真实安装/失败恢复操作驱动阶段；已知/未知下载量；收起与取消分离；替换阶段不可取消；连接错误保留活动状态；启动自动弹窗、重载续显、顺序轮询及清理；恢复启动不可当下载取消。
- 平台说明：Windows 安装器回归按既有规则跳过 POSIX 项，另一次沙箱内宿主回归未结束后中止；已用上述 WSL 完整无跳过回归完成替代验证，未以跳过项宣称通过。
- 过度设计复核见[实现复核](review.md)：未新增硬限制、自动重试或恢复框架；未发现需继续修改项。
- **未提交、未合并、未发布、未部署。** 本轮设备操作仅只读取证；设备仍为 0.4.0。真实 GitHub 完整更新链、原生 Steam 界面、实体手柄焦点/返回/滚动均待验收。旧版启动的是自己携带的安装器，发布新包本身不能替换它；设备首次修复需要通过现有独立安装/部署入口安装修复版。

## 2026-10-28：Git 提交归档

- 按用户“提交 GIT 合并 main”要求，将本轮自更新修复与追加的 Windows 单实例分别提交，文档单独归档。
- 自更新代码：`ebdc4ed`（补齐下载进度并提前检查更新目录占用）；Windows 单实例代码及必要测试/CI：`4fa4e43`（阻止重复实例并唤起已有窗口）。
- 本次提交沿用本 session 已通过的 14 项 Flutter、12 项真实更新助手、6 项原生单实例、9 项 CI 选择回归，以及 analyze/原生构建/隔离更新链记录；代码没有新增修改，不重复运行。检查工作区及暂存差异和空白，排除 `.artifacts` 本地程序、日志和夹具材料。
- 合并安排：文档提交后将本地 `main` 快进至 `fix/windows-self-update`；不推送远程、不创建 Tag 或 Release。正式 GitHub 跨版本更新与系统安装版验收仍待完成。

## 2026-10-28：追加 Windows 单实例

- 用户追加要求禁止重复实例窗口；沿用 `fix/windows-self-update` 和本功能目录，未提交或合并。
- 在原生 `wWinMain` 最早阶段获取会话级 PatchHub 互斥量，先于 Flutter 引擎/用户数据/后台任务初始化；重复进程通过主窗口专属属性定位窗口，还原最小化状态并激活，然后直接退出。安装版与便携版在同一会话也互斥，不依赖 EXE 路径或通用 Flutter 窗口类名。
- 互斥量持有至 Flutter 引擎及窗口析构完成；正常退出后释放，异常退出由内核回收。`--update-probe` 不绕过互斥量；助手等待旧进程退出后才启动新版，原生真实更新链确认可以顺利接管。
- 新增真实 C++/Win32 进程窗口回归，直接编译生产 `single_instance.cpp`：重复启动/最小化还原、8 进程并发仅一个主实例、主窗口初始化中重复启动、另一安装目录互斥、正常退出后探针启动及探针不可绕过、崩溃遗弃互斥量重新获取，共 6 项通过。夹具使用独立互斥量标识，不操作用户窗口。测试已接入 Windows CI，新增测试路径的作业选择回归 9 项通过；CI 本身尚未在线执行。
- 原生完整构建通过：MSVC 14.44 / CMake / Ninja，输出 `.artifacts/windows-self-update-fix/native-build/runner/`，在隔离目录装配本地候选 `.artifacts/windows-self-update-fix/candidate/`；未覆盖用户现有 Release 文件。旧 CMake 3.18 的运行库自动发现有警告，候选复用本地既有发行目录的 MSVC runtime 后启动通过，不代表已生成 NSIS 或完成正式打包验收。
- 真实 Flutter 候选隔离验证通过：重复进程 exit 0、首实例仍运行且窗口从最小化还原；正常关闭旧进程 exit 0 后，实际 PowerShell 助手替换元数据 0.3.0 → 0.4.0 夹具，启动新版原生进程完成 Core/首帧/commit/ack 握手，助手 exit 0、结果 completed，额外用户文件保留。证据 `.artifacts/windows-self-update-fix/native-single-instance-result.json`。这是脚本授权助手的本地夹具链，不是 UI 点击或真实 GitHub 跨正式版本发行验收。新版随后收到正常关窗请求、窗口消失，未单独获取新版退出码。
- 冒烟夹具首轮在 WindowManager 初次展示完成前执行最小化，状态被初始化覆盖而等待失败；调整为等待更新初始化记录后再次验证通过，生产代码未增加延时。CI 选择首轮沙盒因临时目录权限失败，原生权限重跑通过。
- 单实例设计/执行路径的过度设计复核通过：仅进程生命周期互斥量、窗口属性与现有 Win32 显示/激活 API；没有新配置、持久锁文件、轮询、等待阈值或更新豁免。原下载 UI/Dart 服务与助手逻辑本次未变，沿用上节 14/12 项验证及 analyze 记录。
- 未执行：真实 GitHub 跨版本升级、正式 Windows 安装包/系统安装版完整矩阵；未发布。运行中的旧构建不具备新互斥逻辑，需正常退出后启动新候选才生效。

## 2026-10-27：Windows 下载进度与多实例更新失败修复

- 修复分支：`fix/windows-self-update`，基于 `main@2bf0dda`；专项分析与验收项见[修复计划](fix-plan.md)。
- 现场只读证据：更新结果文件仅记录 `Application still running in target directory`，目标是本地 Flutter Release 目录；同目录有两个仍持有响应窗口的 PatchHub 实例（PID 1216100、1253460），均由 Explorer 启动。未关闭或终止用户窗口，也未改动该目录中的发行文件或用户更新记录。
- 原因：助手等待发起者退出后才检查其他进程，另一实例会令安装在替换前失败，导致窗口已关闭且没有新版启动。`HandshakeException` 是另一网络操作的错误，旧界面缺少清晰分隔，不能据此认定安装失败由 TLS 引起。
- 已实现：下载进度条/百分比/可读大小、未知长度不定进度、下载附件与校验阶段提示；清除上次进度；历史安装结果与当前操作错误分隔及重新检查入口；助手准备阶段拒绝同目录其他客户端并报告进程/PID，保留退出后的全部进程检查。
- 2026-10-28 最终验证：`flutter test --no-pub test/app_update_test.dart test/app_update_panel_test.dart test/app_lifecycle_test.dart` 14 项通过；`python -m unittest discover -s scripts -p test_windows_update.py -v` 12 项通过（包含强化后的双实例真实进程回归，两个客户端均保持存活，助手必须在 15 秒测试期限内拒绝准备；生产未增加超时）；`flutter analyze --no-pub` 无问题。助手原有的替换、坏哈希、失败恢复、中断恢复、无关文件保留及卸载回归均通过。
- 网络实测：相同 Dart `HttpClient`、请求头及默认 TLS 验证访问官方 latest API 和 v0.4.0 校验清单，均 HTTP 200（API 返回 v0.4.0，清单 278 字节）；当前没有重现握手异常，不能由此断言间歇性网络故障已消除。诊断脚本与结果摘要在 `.artifacts/windows-self-update-fix/`。
- 验证环境：首轮沙盒 Flutter 因全局 telemetry 文件拒绝写入而失败，改用原生权限完成检查；真实助手测试也使用原生权限以读取测试进程。未将权限问题记为代码测试失败。
- 过度设计复核：沿准备、退出、占用拒绝、替换及恢复路径核对，准备阶段检查对应现场两个同目录窗口的实际文件占用；复用现有检查函数，无新常驻服务、配置、固定延时、强杀或自动网络重试。复核范围与未验证项见[实现复核](review.md)。
- 未执行：真实 GitHub 跨正式版本更新、用户当前应用更新、系统安装版完整验收；尚未构建、提交或发布本次修复包。TLS 握手异常的网络原因尚未复现，当前只改善可辨识性与手动重试入口，未绕过证书校验。

## 阶段状态

| 阶段 | 状态 | 证据 / 后续条件 |
| --- | --- | --- |
| 调研与设计 | 已完成 | 核对现有双端宿主、Core v5、CI、NSIS、Decky 终端安装器与上游 Loader 3.2.9 源码；形成客户端层方案 |
| P0 安装可行性原型 | 本地 Release 夹具完整链通过；正式发布链待验收 | Windows 真实 UI 下载/确认、退出及新版握手通过；Decky 实机下载、Loader 替换/重载、验证及恢复通过 |
| P1 检测与提示 | 已实现 / 自动化通过 | 宿主 GitHub 检测、SemVer、每日调度、手动检查、跳过版本、通知与附件冻结校验；无 Core 协议改动 |
| P2 Windows 自更新 | 已实现 / 隔离完整更新链通过；安装版系统级验收待执行 | 统一 ZIP 助手、事务恢复、动态卸载清单、安装版版本维护；本地 HTTPS 响应夹具 + 真实 UI/原生程序通过 |
| P3 Decky 自更新 | 已实现 / 实机夹具完整链通过；正式跨版本链待验收 | Loader 3.3.0 完整自更新执行链、失败恢复、GitHub 检测和页面通过；测试后恢复正常 0.3.0 |
| P4 发布与验收 | CI 改造已实现 / 正式发布验收待执行 | 草稿上传/下载校验/公开步骤已改造；未创建或修改公开 Release |

## 2026-10-24：调研与设计

- 用户范围：双端自动检测 GitHub 新 Release 并提示，调研 Core/客户端分层并提供设计；本轮不实施。
- 工作树开始时干净，按仓库规则从 main 创建本功能分支，新增设计/施工记录并更新功能索引。未提交或合并。
- 当前事实以仓库重新核对为准：Flutter 是 Windows 当前客户端，原有记忆中的迁移阶段不作为本轮完成状态依据。
- 只读查询 `https://api.github.com/repos/Mabinogi-Cichol/steam-hentai-patch-searcher/releases/latest`，返回规范仓库 `Mabinogi-Cichol/steam-hentai-patch` 的 `v0.3.0`，发布时间 `2026-10-23T18:23:52Z`（北京时间 2026-10-24 02:23:52），draft/prerelease 均为 false。
- 核对附件：Windows ZIP 19,199,825 字节、NSIS 14,551,300 字节、Decky ZIP 7,003,317 字节，另有 `SHA256SUMS.txt`、两端 checksums/SBOM 和终端安装脚本。未下载或安装这些附件；仅核对 API 元数据。
- 阅读官方 GitHub Releases/限流文档和 Loader v3.2.9 utilities/browser/plugin-loader 源码；本地核对 `@decky/api`，确认不能假设插件 callable 可直接调用安装管理路由。
- 关键结论：自更新归客户端宿主，Core/协议不变；Windows 独立助手接管退出后更新，Decky 复用安装器引擎并由独立进程跨卸载执行；完整包一次更新配套 GUI/插件、Core、worker。
- 设计级过度设计审阅已完成：移除共享 Rust 更新协议/SDK、预发布配置、任意硬限制和重试框架的预设；保留由实际文件占用与卸载路径支持的门禁和恢复。实机可行性仍为待验证项，不等同通过实现审查。

## 本轮验证

### 2026-10-24：移除自动检查开关

- 移除自动检查开关，保留手动检查、“稍后”和“跳过此版本”；经用户澄清，不采用“始终启用”的表述，用户通过“跳过此版本”控制该版本的更新提醒。
- 同步移除设计中的 `auto_check` 偏好字段和关闭自动检查验收项；其余偏好仅用于跳过版本等既定行为。

### 2026-10-24：Windows 自更新统一使用 ZIP

- 按用户确认，安装版和便携版共用 ZIP 下载、校验、退出协调、文件替换与恢复助手；取消原设计的 NSIS 自更新路径。NSIS 继续用于首次安装、手动安装和卸载。
- 安装版仅额外维护本产品注册表版本值和当前受管理文件清单，保留安装路径、产品名、卸载器及快捷方式；便携版不创建安装注册信息。
- 核对已有 NSIS 卸载列表为构建时编入，设计要求首次接入版本的卸载器改为读取已提交文件清单，确保 ZIP 多次更新后仍能正确卸载。v0.3.0 安装版需手动安装包含该契约的新版本，再使用自更新。
- 同步修订架构图、附件选择、Windows 安装流程、实施阶段、验收矩阵和设计级复杂度审阅；功能仍待实现，未运行程序更新或部署。

- 文档结构与相对链接检查通过：两份功能文档及功能索引的本地 Markdown 链接目标均存在；`git diff --check` 通过（另核对新增文档空白）；仅针对文档。
- 未运行 Rust/Flutter/Decky 功能测试或构建：没有功能代码改动。
- 未执行 UI 自更新、真实 Windows 替换、Decky 连接/部署/重载、失败恢复、断电或正式发布验收。

## 后续工作

1. 使用下一正式新版本验证从提示/确认、GitHub 下载到双端独立执行者完成跨版本更新的整条链路；没有创建测试 Release 或伪造线上新版。
2. 完成 Windows 系统安装版注册表、快捷方式、自定义位置及连续多次更新后的真实卸载矩阵；现有隔离程序文件/清单/恢复验证不能替代这些项目。
3. 完成 Steam Deck 实体手柄操作、正式新旧版本升级与断电恢复矩阵，按真实证据更新记录。CI 发布流程已修改但未实际发布验证。

## 2026-10-24：实施与阶段验证

- 用户授权“开始实施”；Windows 安装版/便携版统一 ZIP，保留“跳过此版本”，没有自动检测开关。Core 和 v5 协议未修改。
- Windows：Dart 宿主更新服务和界面、外部 PowerShell 更新助手、ZIP 路径/哈希/清单验证、退出协调、文件/安装版版本信息恢复、新版首帧与 Core 探针握手、待确认启动不执行正常业务初始化、故障恢复入口；NSIS 改为读取当前清单卸载，并与更新共享目录锁。
- Decky：Python 宿主更新服务、React 更新区域和通知去重、独立会话安装进程、下载进度/取消、任务与调度维护门禁、重载期间等待提交、恢复入口；复用并随插件打包受信任的当前安装器。
- 构建：生成 Windows 版本与文件清单，核对 Cargo/Flutter/Decky/Tag；更新生产 backend allowlist 和 CI 输入选择；Release 改为草稿完整上传、下载 SHA-256 核验后公开，拒绝自动替换公开附件。
- 自动化：Flutter 全量 213 项通过，`flutter analyze --no-pub` 通过；Windows 真实 PowerShell 更新助手 6 项通过（成功、坏哈希、失败回滚、无关文件冲突、模拟中断恢复、更新后卸载）；Decky 后端 107 项通过（2 项跳过），前端 68 项通过，桥接 14 项通过；Ubuntu WSL 的安装器 34 项全部通过；打包 8 项、CI 选择 9 项通过。后续代码变更只补跑受影响检查。
- Windows 构建环境：标准 `flutter build windows` 仍受本机 VS 自动发现问题阻断。沿用仓库已有 MSVC 14.44 / CMake / Ninja 命令构建普通 `lib/main.dart` 候选，未修改系统工具链；候选 NSIS 已编译。证据在 `.artifacts/client-self-update/`。
- Windows 真实原生隔离验证：`.artifacts/client-self-update/native-smoke-result.json` 记录 helper exit 0、版本 0.3.0、额外文件保留、事务目录已清理；实际 Flutter 程序完成 Core capabilities 与首帧/提交握手，随后响应正常。正常关闭请求成功，约 701ms 退出；该次 Get-Process 附着未返回退出码，因此不宣称已确认 exit 0。使用独立 APPDATA，不修改现有安装或游戏。
- 自动审批拒绝批量移除既有 CEF 响应大小限制和操作超时，理由为无界等待及可用性风险。本轮保留这些既有边界，不绕过拒绝；这是与设计中全面精简旧防御路径的差异，后续需单独基于实证审议。下载 ZIP 的任意大小/数量限制已在获准的修改中删除。
- 用户打开并连接 Devkit 后，按部署技能确认当前配对与已有可信主机记录；设备 Loader 为 v3.3.0（原方案只核对过 3.2.9），旧插件 `Steam 补丁管理` v0.2.0，无非终态补丁或来源任务。已核对官方 v3.3.0 utilities/browser/plugin 源码及设备只读桥接 probe，扩展并验证管理适配。
- 设备第一次候选安装失败：本地旧 Linux Core 缓存与当前生成协议不一致，新后端报 `unsupported_protocol`。真实 Loader 安装事务自动恢复旧版 v0.2.0、运行验证通过并清理缓存事务，未将该次部署记为成功。随后从当前锁定源码重建 Linux Core/worker，当前候选实际 v5 preflight 通过，再进行候选安装验证。
- 旧插件完整备份：`.artifacts/client-self-update/old-plugin.tar.gz`，SHA-256 `0051fa516d26d2a1f5379ddea86d54a4d9a7144299c135231ac4a2a397957d12`；旧版任务/设置/数据库基线与设备证据保存在同目录，未提交个人数据。

### Steam Deck 后续验证结果

- 重建后的 Linux Core/worker 与当前生成契约通过实际 `CoreBridge.preflight`；经 Loader 正常安装候选 0.3.0 并完成运行验证，缓存安装事务已清理。第一次失败时的旧版恢复属于真实设备证据，而非模拟测试。
- 实机发现 Loader 冻结 Python 默认 CA 解析失败：真实 GitHub 检查报证书链错误。已改为系统 `/etc/ssl/cert.pem` CA，并用系统 `python3` 启动独立执行者，清除冻结环境的 `LD_LIBRARY_PATH` 影响；保持 TLS 验证。修订候选已重新部署。
- 此阶段 Decky ZIP：7,031,919 字节，SHA-256 `ef76ccca18d104d863a10b945e9633953d8b08974ed2a6e435b0e50e3b073da5`。设备后端版本 0.3.0、唯一有效前端实例 0.3.0、真实 Core capabilities 协议 5；HTTP `dist/index.js` SHA-256 `20f241d02de1e03240e1a8b60d87eabddb888f9781178c3f576f1e583e3f5a00` 与本次本地构建一致。后续完整链验证的最终包见下节。
- 实际 `check_app_update` 成功读取公开 latest `v0.3.0`，状态 `up_to_date`；无 GitHub Token。真实游戏模式全局设置中点击“检查更新”，显示“当前已是最新正式版。”，页面无横向溢出。截图 `.artifacts/client-self-update/deck-update-checked.png`，原始 RPC/DOM 记录见 `deck-verify-final.json` / `deck-update-checked.json`。实体手柄未操作。
- 独立进程测试使用实际 `AppUpdate.start`，安装行为替换为隔离无安装脚本：宿主 SIGTERM、原插件目录删除后 worker 继续写出完成哨兵。在 WSL 和实际 SteamOS 均通过。该结果验证进程生命周期，不等同于前端确认后跨正式版本自更新已验收。
- 部署后 `journal.sqlite` 与 `trust.sqlite` SHA-256 均与部署前完全一致，Loader 其他插件顺序/成员保留，本插件身份按当前命名由“Steam 补丁管理”变为“Steam 黄油补丁助手”。没有安装或恢复游戏补丁。
- Windows 复核追加第 7 项真实 PowerShell 回归：失败的准备事务不能删除其他事务的恢复标记；中文路径的 JSON 统一 UTF-8。固定空维护锁载体保留在程序目录，卸载时删除，不进入发行 ZIP；下载、暂存与备份仍按事务清理。
- 实现复核见 [review.md](review.md)。本阶段尚未提交、合并或发布；最终 Windows/Decky 发行验收不因部分机制通过而自动标记完成。

### 本地 Release 夹具完整链与收尾

- 未发布测试 Release。Windows 使用测试进程的 `HttpOverrides`，Decky 使用测试启动环境的 `sitecustomize`，仅将固定官方 URL 的响应替换为本地 Release JSON、校验清单与完整 ZIP；这些注入不进入发行包。网络检测另以真实公开 latest 验证。
- Windows：真实 Flutter 更新组件点击下载、完成 SHA-256 校验、点击“重启并更新”、正常关闭宿主，实际 PowerShell 助手等待父进程退出后替换隔离旧目录 0.2.9，启动真实原生 0.3.0 程序并完成 Core/首帧/提交握手；无关用户文件保留、事务目录清理。最新源码构建后再次通过，结果 `.artifacts/client-self-update/windows-full-chain-result.json`；验证后的原生窗口通过正常关窗退出。
- 完整链发现并修复 Windows 临时目录 API 使用错误、下载阶段提示区域不可见、重复初始化与有效混合分隔符路径比较问题；未增加生产测试入口或固定等待后强制覆盖。新增 native chain 测试默认跳过，只有显式提供隔离夹具才启动外部真实更新器。
- Decky：生产 `AppUpdate.start` 启动真实外部安装器；启动宿主退出后进程继续下载、验证，并经实际 Loader 把 0.3.0 替换为测试 ZIP 的 0.3.1，运行验证完成，结构化结果 `completed`，事务目录清理。证据 `.artifacts/client-self-update/deck-full-chain-result.json`。这是执行链实机验证，未通过真实手柄或前端按钮开始该次更新。
- 完整链发现 `release_package` 丢弃校验结果中的 `name`，Loader 无法使用该结果；改为保留完整校验结果并补充契约断言，WSL 34 项安装器测试通过。状态轮询同时补充终态重读，防止进程退出检查覆盖刚写出的成功结果；新增竞争回归通过。
- SSH 测试会话退出曾终止所属会话进程；保留 SSH 验证会话后执行正常。真实 Loader 宿主不在 SSH 测试会话内；设备 POSIX 独立进程测试另已通过。没有据此增加生产进程保活框架。
- 测试后经 Loader 恢复正常 0.3.0。最终候选 ZIP 7,032,016 字节，SHA-256 `67cb6026030377a5a4e3dd5fe327ec5d038ab07e61ab4ce3527fa1a1ff931de7`；后端/唯一前端实例 0.3.0、协议 5、HTTP bundle 哈希与上节一致；真实检测为 `up_to_date`。
- 清理前核对首次完整备份哈希与本地副本一致，确认无恢复目录、无执行进程，删除本轮远端暂存及测试 `v0.3.1` 的终态结果；正常更新状态恢复为 `execution: null` / `needs_recovery: false`。Loader active、非终态补丁任务 0，持久数据库哈希仍与原始基线一致。保留本地备份和全部证据。
- 最新增量检查：Flutter analyze 无问题，自更新 3 项通过（native chain 默认 1 项跳过，显式夹具执行通过）；Windows 真实 PowerShell 7 项通过；Decky 更新服务 8 项通过；WSL 安装器 34 项、打包 8 项通过。先前未受影响的全量测试结果沿用。
- 最终真实扫描返回 27 个安装实例，证据 `deck-scan-result.json`。单次 CEF 等待窗口不足以覆盖扫描，改为异步记录后读取完成结果，没有终止 Core 正常工作或修改生产超时；测试界面已恢复 Steam 游戏库。

### Decky 更新入口与排版修订

- 按用户反馈移除 QAM 侧边栏中的更新区域；保留全局设置“缓存、备份与更新”页，以及既有通知跳转入口。
- 原更新组件与 `GlobalCache` 并列，位于 `.patch-page` 之外，导致边距和滚动范围与缓存/备份不一致；现在嵌入同一个页面容器。版本与发布日期使用标准 `Field` 行，按钮使用与现有设置相同的 `layout="below"` / `bottomSeparator="none"`，提示采用可换行段落。
- `npm run typecheck` / `npm run build`、`git diff --check` 通过；这是可逆的界面调整，未改更新引擎或增加配置、状态分支，沿用原有引擎测试记录。
- 经 Loader 部署实机候选：ZIP 7,032,365 字节，SHA-256 `99eb78dce54d548b7dd6d0c0cbe957a0da22461a4455093e0ad8b0cf19590f65`。修改前完整备份 `.artifacts/client-self-update/layout-before.tar.gz`，SHA-256 `45c520367158b2d087f894b651debd645ef0298639d5ac17c39f65c2648f52d8`。
- 真实全局设置点击检查更新通过，截图 `layout-update-checked.png`；按钮处于统一页面容器内，无横向溢出。QAM 已打开的旧组件需返回 Decky 列表再进入；重新进入后的真实 DOM 仅含全局设置/重新扫描，不含应用更新/检查更新，记录 `layout-qam-final.json`。
- 运行中的唯一插件/前端实例 0.3.0、Core 协议 5，非终态补丁及来源任务均为 0；HTTP bundle SHA-256 `01b605e8110944629aae1dbcfd331cc1836a47e996cc806aba158248122ce4e5` 与本地本次构建相同。实体手柄未操作。
- 验证后恢复 Steam 游戏库并关闭菜单；核对本地备份后清理本轮远端界面验证暂存，保留本地备份、截图和 RPC 证据。

### 2026-10-24 Windows 更新提示与布局修订

- 手动检查结果移入可滚动弹窗；底部固定版本/检查行保持原高度。自动检查发现 available 且未跳过/未暂缓时才新增底部提醒。检测服务记录本次实际检查是否手动触发，不改变查询/下载/安装引擎。
- 自动提醒直接查看既有结果；ready/downloading 时重新打开详情不再次检测，避免丢失下载完成状态。下载、取消、跳过、Release 页面和重启更新沿用现有入口。
- 新增 5 项更新面板回归：手动最新/失败/包未就绪/新版本均不改变主内容矩形；自动提醒、查看不重复检查、下载后关闭重开仍可重启更新通过。Flutter analyze 无问题；本轮完整 Flutter 回归 219 项通过、1 项既有原生更新链夹具测试跳过。没有重新执行已验证且未修改的更新替换引擎或 Decky 验收。
- 原生 Windows 布局验收因本机 Visual Studio 2026 缺少 Flutter 要求的 C++ 组件未能启动，截图与新包待验证。详细界面修复与证据见[Windows 施工记录](../windows-ux-redesign/progress.md)。

- 2026-10-24 用户要求构建后已补齐 VS 工作负载/CMake 并成功生成含本轮 Windows UI 修复的 Release；隔离 APPDATA 启动和正常退出通过，原生更新弹窗几何断言通过。产物/30 张原生布局截图及验证边界见[Windows 本轮施工记录](../windows-ux-redesign/progress.md)。本次仅本地候选，不是正式跨版本发行验收。

## 2026-10-25：提交归档

- 按用户要求将自更新功能提交到 `feature/client-self-update`，代码提交 `d3d2e68`，文档独立提交；未推送或合并。
- 本次只收录双端自更新、必要打包/CI/安装卸载改动及对应测试和文档，不收录品牌素材或其他 Windows 页面布局改动。
- 沿用本轮已有验证记录，提交前检查差异和空白；正式 GitHub 跨版本发布、Windows 安装版完整矩阵与实体手柄仍待验收。
