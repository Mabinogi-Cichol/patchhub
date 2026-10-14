# 2026-10-15 Steam Deck 部署验收

关联：[设计](design.md) · [施工记录](progress.md) · [默认值限制处理](policy-inventory.md)。

状态：**新版插件部署、运行时/RPC、设备隔离 worker 分析转换及真实页面验收通过；实体手柄未验证；实际游戏安装/生效不在本轮授权范围。** 本次用户授权部署至 Steam Deck 并验收；用户随后明确选择“只部署插件并验收运行与界面”，本轮按该范围完成，未写入游戏。

## 部署与恢复材料

- 使用当前 Devkit 选中 steamdeck、实际配对密钥与既有可信独立 known_hosts，StrictHostKeyChecking=yes；没有更改全局 SSH 信任。设备时间为 2026-10-15 01:17–01:33（Asia/Shanghai）。Loader v3.2.10。
- 部署前真实 list_tasks：23 条任务全终态；唯一插件实例。当前目录 root:root 755，采用已核对同版本正式 Loader 本地 ZIP 安装，UPDATE=2，唯一确认请求 `1791998486.8079994`；正常安装流程停止旧后端并加载新后端，未手工结束插件进程或修改目录所有权。
- 新 ZIP 7,228,118 字节，SHA-256 `f2a7ce9248f0358c211783f7ba5de0b357169d4684e954b7b642e1f717b0a3a2`；上传后逐成员核对。正式 24 文件全部大小/SHA 匹配，CLI/worker 为 Linux ELF 且可执行。服务 active，实际运行插件实例为 1。
- 本轮首次完整旧插件、全部外部状态、Loader 设置与可安装 rollback.zip 已取回本机并核对 SHA，保存在忽略目录 `.artifacts/default-options-deck-20261015/`。回滚 ZIP SHA `7cc51b205182d6fdb702b8c43c44c33f7c7d3e6ccbc85e4d3e4ed4a56360f6d7`；旧插件 tar SHA `63fb168c285516389836427d1e4bfed64370f6bda9d5508455fb05753f2cab70`；完整旧状态 tar SHA `56b41b377f62c8f485b94ee02b8407a17603a040397a6e7297cfbffface81548`。完整摘要见 backup-summary.json。

## 运行时与修复行为

- 通过 Steam SharedJSContext 现有认证连接调用真实 scan(false)、list_tasks 和 list_transactions；27 实例、23 原终态任务、14 事务。两条 library_unavailable 指向当前未挂载的历史库，未将它们伪装为扫描全部库成功。旧夏日失败记录保留为历史，未改写为完成。
- 实际 HTTP 前端为 112,599 字节，SHA `9e162cd6caabadab0b92c05592c7796b130dab48c683fcff4ebb5fd437463bac`，与候选包及正式目录一致。前端代码本次未变，正常安装重新挂载；后端新行为由以下设备生产 worker 证据确认。
- 从正式目录复制已核对的 sf-worker 至唯一验证目录，以 uid 1000、实际生产 bwrap 启动器分析和转换设备原有夏日 DLC 缓存；未执行原始 EXE/Lua，worker 不访问游戏目录。
- 正式图 decoder=sf9-graph-v2、policy=sf9-behavior-v2。分析 3,156 ms，转换 3,578 ms；5,874 个成员逐一核对路径、大小、ZIP CRC 与解压 SHA，产物 SHA `7b20d4bc35fb66e6dabe746b7ceea48fadebf8491a7a396ba8e7119434c75bfe` 与本机两平台一致。必要条件为 Game.exe 存在且 Script.vdf CRC32=2002373744；旧 exe_project_options_unsupported 未再出现。
- 所有设备 JSON 证据已取回并核对 SHA，device-analysis.json / device-convert.json / device-worker-verification.json / deployed-verification.json / evidence-summary.json。

## 界面、状态与清理

- 实际 QAM：有补丁筛选、6 行列表、夏日“可手动选择补丁”、底部设置及重新扫描可见；真实顶部/底部截图 qam.png、qam-bottom.png。CSS viewport 854×454，滚动容器 440/1219，无横向溢出。
- 实际全屏补丁页：导航、补丁选择、下载文件、详情与底部手动选择/安装区域可见；summer-fresh-top.png / summer-fresh-bottom.png 已逐张查看。CSS viewport 854×534，内容 1646 高、滚动至 1112，无横向溢出。真实 DLC 选项显示夏日狂想曲+、2.02.1、DLC 1390150、对应下载文件与手动选择入口，DOM 证据 page-layout.json。
- 第一次菜单入口使用旧 SteamClient.UI API失败、第一次本机中文解码失败以及导航与异步 RPC 同表达式造成 Promise was collected，均修正后重新核对；这些失败尝试不计作通过。确认弹窗残留已正常关闭；下拉选择后的模糊截图不计作通过，重新进入页面后清晰顶部/底部画面验证通过。没有通过改 CSS 掩盖遮罩，也没有重启 Steam/设备。
- Loader 的 pluginOrder/frozenPlugins/hiddenPlugins/disabled_plugins 与基线一致。正式替换后全部外部状态 SHA 不变；UI 自然扫描后变化仅为 sources/rust-v1 的派生发现/状态缓存，任务、事务/SQLite、订阅设置与下载缓存保持原值。
- 恢复原用户路由，关闭本轮菜单；已核实唯一绝对验证目录后删除本轮暂存，共 1,148,547,124 字节，确认不存在；关闭唯一匹配本轮的 SSH 隧道，本地首次备份与证据保留。
- 未操作实体手柄，未启动游戏，未安装/覆盖游戏补丁。因此不宣称游戏内生效或真实安装事务已通过。本次仅设备部署、实际分析转换、RPC 与界面证据；用户已明确排除本轮实际游戏安装，按确认范围收尾。
