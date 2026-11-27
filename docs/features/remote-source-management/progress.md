# Remote 施工记录

状态为**已实现，本地定向验证通过，跨设备实机待验收**。见[设计方案](design.md)。

2026-11-25，在 main 基线 `0bbd9d0` 上核对 Windows 来源页、Decky 来源服务及插件入口，对照 Tabler、react-admin 与 Pico 官方资料。方案审阅覆盖启用、导入、刷新、更新取消及停用路径。建议共用一份静态网页，沿用两端的来源服务和任务注册表，避免额外的常驻进程、身份账户系统和任务队列。

2026-11-25，确认 PatchHub Remote 前端采用 Tabler 开源版，首版页面为「全局来源」。

本次仅完成设计与文档检查。HTTP 契约、网页制作、两端接入与打包、并发变更及真实局域网访问均待实现和验证。

2026-11-25，核对 Windows SettingsPage 与 Decky 全局设置侧栏、路由，补齐两端 Remote 入口位置、连接信息展示及离页后返回的验收路径。客户端入口待实现。

## 2026-11-25 实现与验证

Windows 主窗口设置、Decky 全局设置均已在「缓存管理」和「关于」之间接入 Remote。监听由宿主持有，选定本机 IPv4 后由系统分配端口，每次启用生成新密钥。两端只提供固定资源与来源 API，使用 Bearer 密钥认证，校验 Host 及写请求 Origin。

共享网页位于 `apps/remote-web/`。两端使用相同的 Tabler 1.4.0 CSS、MIT 许可证、HTML、JavaScript 和样式。Windows 通过 Flutter 路径依赖嵌入资源，Decky 构建时复制到插件的 `remote/`。原生 TypeScript 入口使用生成的来源类型，`node apps/remote-web/build.cjs --check` 校验随包 JavaScript 与源码一致。

网页支持按角色分组、来源启停、改名、设置间隔、索引更新、取消和结果查询。Lua 文件在访问设备的浏览器中读入同一个源码框，粘贴与文件导入均调用既有 core 接口。内置脚本只读，自定义脚本支持编辑和删除。两端保留本机来源写入、更新与取消逻辑。保存失败保留源码；保存成功后列表刷新失败可单独重读。

### 本地自动化

| 范围 | 执行与结果 |
| --- | --- |
| Windows | `flutter analyze --no-pub` 通过。对 remote_native、remote_page、remote_server、app_lifecycle、app_update_panel、settings_page、source_model、script_source_native、script_source_editor 九个测试文件运行 `flutter test --no-pub`，50 项通过，1 项已有的空内置来源专用构建用例按条件跳过。 |
| Windows 实际核心与资源 | 以当前分支重建的 debug CLI、临时状态目录、Flutter 宿主和回环 HTTP 验证脚本读写、无效脚本保留旧内容、本机与远端同时变更及新客户端重读。验证设置离页仍监听、取消、关闭释放端口、旧密钥失效。在 `flutter build bundle --no-pub --target-platform=windows-x64` 产物中逐一核对 5 个资源的 SHA-256。 |
| Decky 服务与路由 | WSL 使用 Python 3.12、aiohttp 3.14.5。对 `apps/decky/backend` 分别运行 unittest discovery，`test_remote_server.py` 11 项、`test_source_service.py` 20 项、`test_source_tasks.py` 14 项通过。覆盖真实 HTTP、绑定失败、Host、Origin、密钥更新、任务查询与取消、保存后刷新失败和端口释放。路由 4 项测试及 TypeScript 类型检查通过。 |
| Decky 实际核心 | WSL 在隔离状态目录运行 RemoteServer、Plugin 来源入口和重新构建的 Linux release CLI。经 HTTP 导入自描述 Lua、读取并编辑、跟踪首次索引更新，和本机并发修改不同来源，再从新建的 SourceService 核对持久化字段并删除。 |
| 网页 | `npm run test:contracts` 通过，覆盖 JS 生成物一致性、鉴权头、失败后保留输入、保存后刷新及任务取消回执。桌面 Chrome 在 390 × 844 视口加载离线资源，使用模拟来源接口完成登录并展开 Lua 编辑器，无横向溢出。截图位于本地 `.artifacts/remote-mobile.png`。 |
| 打包 | WSL 独立源码目录执行 `npm ci`、协议检查、类型检查、`npm run package:plugin -w apps/decky`。逐文件比对生产暂存目录的五个网页资源和后端模块。版本同步 12 项、发行打包 12 项、SBOM 1 项、CI 变更选择 9 项测试通过。 |

独立审阅发现三项问题，均已修正：Decky 来源修改成功后刷新失败导致误报、网页将正常取消显示为失败、Windows 保留已被宿主替换的旧任务结果。停用 Remote 会清除该次会话的任务句柄，已交付宿主的任务继续按原生命周期运行。

### 待实机验收

- 用另一台物理设备分别连接 Windows 和 Steam Deck 展示的 IP、端口，验证来源管理和设备重启后的持久化；检查 Windows 专用网络防火墙提示。
- 在 Steam Deck 游戏模式验证实际 Loader 的 aiohttp 导入、手柄焦点、收起面板后继续访问，以及卸载插件后的端口释放。
- 在真实手机上选择 Lua 文件并提交。浏览器窄屏模拟和 WSL 测试结果单独记录。线上 CI 尚未运行。

### 2026-11-25 默认网卡与六位访问密钥

原先 Windows 直接选枚举结果的第一项，复现时该项是带有高优先级默认路由的代理隧道。现在使用 Windows IP Helper 返回的已连接物理以太网和 Wi-Fi 接口索引，对本机 IPv4 候选进行排序。Decky 根据 Linux 网络接口的设备关联优先显示物理网卡。两端均保留手动选择虚拟网卡的能力。

每次启用生成六位随机数字，按字符串保留前导零，同一宿主进程内连续两次开启的密钥不同。浏览器使用数字键盘提示。定向验证通过 Flutter 静态分析及三个 Remote 测试文件（11 项）、WSL Decky Remote HTTP 测试（11 项）和浏览器前导零登录检查。
Windows Release 预览构建成功，五份网页资源与源码目录逐一核对 SHA-256。另用隔离单实例名称的 Windows 原生测试窗口，在物理 WLAN 与 Mihomo 隧道同时启用的环境中验证首选 WLAN，且仍能手动选择隧道。

### Remote 快捷按钮

Windows 与 Decky 的 Remote 设置页在监听期间显示「打开网页」和「复制密钥」。打开网页调用宿主的系统浏览器入口并传入完整监听 URL。复制将当前密钥按字符串写入剪贴板，保留前导零。

Flutter 静态分析与两份设置测试通过（13 项）。Decky 类型检查与前端测试通过（115 项）。两端自动化均检查浏览器调用收到显示的 URL、剪贴板调用收到原始密钥。Steam Deck 游戏模式下的浏览器与剪贴板实机验证待完成。

### 2026-11-26 默认端口与连接卡片

两端优先绑定 TCP `47625`。首选端口被占用时由系统分配可用端口，地址和打开网页按钮均使用实际监听端口。Windows 回归测试覆盖 Winsock 在独占端口冲突时返回的访问权限错误。连接卡片放大并加粗六位密钥，底部使用等宽并排的实底按钮。

Flutter 静态分析及 Remote 定向测试 12 项通过，包含固定端口、占用回退、端口释放后恢复固定端口，以及密钥字重和按钮对齐。WSL Decky HTTP 测试 12 项、Decky 设置页相关测试 18 项及类型检查通过。Steam Deck 游戏模式的实机布局及焦点仍待验收。

### 2026-11-28 来源卡片与脚本导入

索引源卡片显示各自的更新进度、取消入口和结果。两端的全部更新回执包含来源与任务的对应关系。导入、查看和编辑 Lua 脚本复用右侧面板，导入时可以一键复制 AI 提示词。

`npm run test:contracts` 和 WSL Decky Remote HTTP 测试（12 项）通过。Flutter 定向静态分析和 Windows Remote 的 9 项测试通过，默认端口测试的通配地址被本机 TCP 47625 监听占用。Chrome 在 390 × 844 窄屏、模拟接口和普通 HTTP 下验证卡片内更新结果、侧滑布局、复制事件、Esc 关闭及焦点返回。真实手机和 Steam Deck 的访问和复制待验收。
