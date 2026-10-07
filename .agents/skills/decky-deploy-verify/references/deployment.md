# 连接、构建、备份与部署

以下命令在仓库根目录的 PowerShell 执行。设备值来自本轮用户提供的信息及当前 Devkit 配对；指纹记录须完成下面的设备核实，不能把历史私网地址当作当前设备身份。

## 1. Devkit SSH 和只读基线

### 1.1 先核对当前 Devkit

用户说明 Devkit 已正常连接时，先定位正在运行的 Devkit，读取其当前所选设备和配对密钥路径，不先反复测试历史地址。当前已验证的 SteamOS Devkit Client 提供 loopback API：

```powershell
$devkitDevice = Invoke-RestMethod 'http://127.0.0.1:32010/selected_devkit' -TimeoutSec 5
$devkitKey = Invoke-RestMethod 'http://127.0.0.1:32010/ssh_key_path' -TimeoutSec 5
$devkitDevice | Select-Object name, address, login
$devkitKey | Select-Object key_path
```

先核对本轮进程实际监听端口和已安装客户端实现；以上路由、字段及端口是已验证线索，不保证未来版本不变。只输出设备名、地址、登录用户和密钥路径，不输出整个 settings 或凭证。接口没有选中设备/不可用时，再查当前客户端配置或向用户补充询问，不把空结果当成旧地址确认。

接口的地址/名称用于识别当前配对，不是主机公钥认证。不同局域网可以重复使用同一私网地址，当前执行电脑也可能与用户查看 Devkit 的电脑不同；有歧义时核对执行主机和用户所指设备。

没有可读当前配对时，Devkit 常见私钥位置为 `$env:LOCALAPPDATA/steamos-devkit/steamos-devkit/devkit_rsa`；先检查路径存在，仅传给 SSH，勿输出密钥。历史设备为 `deck@192.168.31.186`，仅作查找线索，不能默认部署到该地址。

### 1.2 处理旧主机指纹冲突

Devkit 正常连接和命令行 SSH 报 host-key 变化可以同时发生。已验证的官方客户端版本使用 Paramiko AutoAddPolicy；部分外部 SSH 命令还使用 StrictHostKeyChecking=no。因此 Devkit 正常连接不表示命令行 known_hosts 已更新，也不能单凭它把扫描到的公钥当作可信身份。

遇到冲突时：

1. 核对当前 Devkit 所选设备、地址、登录用户及实际密钥路径，区分新网络/不同设备与旧记录；读取当前官方客户端实现确认连接方式。
2. 获取目标的公开 SSH 主机密钥并计算指纹，记录与旧可信记录的差异。ssh-keyscan 只收集公钥，不完成身份认证。若 Windows 自带 keyscan 报 unsupported KEX，可使用本机已有且兼容的 Git OpenSSH keyscan；不因此关闭正式连接的主机校验或安装新工具。
3. 当前用户已确认该 Devkit 所选设备就是本轮目标、且确认采用本次配对/指纹时，沿用这个确认，不重复要求用户到 Deck 敲命令，也不每次连接再询问。信息仍有歧义时，只请求必要的设备确认；当前配对无法复核时才让用户提供 Deck 本机指纹。不能自动采纳一个未确认的新密钥。
4. 将确认过的公钥写入本轮 `.artifacts/<本轮>/known_hosts`，后续 SSH/SCP 都指定该独立文件并保持 StrictHostKeyChecking=yes。保留系统旧 known_hosts，不为单次验收删除旧条目或全局覆盖信任。连接后继续核对 id、插件身份和路径，不能跳过任务/备份/哈希门槛。

设备确认和本轮指纹记录属于连接准备，不能提前写插件或游戏。公钥变化未核实时保留现场；不要反复重试相同旧记录并把每次失败当成新证据。

### 1.3 SSH 与只读基线

```powershell
$deckAddress = '<本轮确认的 IP 或主机名>'
$deckLogin = '<本轮确认的登录用户，例如 deck>'
$deckTarget = $deckLogin + '@' + $deckAddress
$deckKey = '<当前 Devkit API 返回或已核实的密钥路径>'
$deckKnownHosts = Join-Path (Get-Location) '.artifacts/<本轮>/known_hosts'
$deckSsh = @('-i', $deckKey, '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=8',
  '-o', 'StrictHostKeyChecking=yes', '-o', "UserKnownHostsFile=$deckKnownHosts")
Test-Path -LiteralPath $deckKey
& ssh @deckSsh $deckTarget 'id'
& ssh @deckSsh $deckTarget 'systemctl is-active plugin_loader'
& ssh @deckSsh $deckTarget 'stat -c "%U:%G %a %n" /home/deck/homebrew/plugins/steam-patch-manager'
```

先确认 known-hosts 来自已有可信记录或本轮已核实的配对/指纹；冲突按 1.2 节处理。连接失败先检查设备唤醒、网络及 SSH 服务，不自动更改防火墙或系统配置。

核实下列项目，路径随实际设备调整：

| 项目 | 本仓库通常值 / 检查方式 |
| --- | --- |
| 插件目录 | `/home/deck/homebrew/plugins/steam-patch-manager`；确认真实路径、非符号链接和插件身份 |
| 显示身份 | 从本地与远端 `plugin.json` 读取 `name`，目前为 `Steam 补丁管理`；版本读 `package.json` |
| 状态目录 | `main.py` 使用 `DECKY_USER_HOME/.local/share/steam-patch-manager`；独立于插件目录 |
| Loader | 本轮读取其版本、服务状态与日志，历史 3.2.9 的安装参数不能当作永久协议 |
| CEF / HTTP | 设备 loopback 常见 `8080` / `1337`，只读探测是否可用，不直接暴露到局域网 |
| 活动任务 | Steam 现有连接调用 `list_tasks`；当前终态为 completed/failed/cancelled，其他状态禁止部署 |
| 状态基线 | 活动任务、事务摘要、Loader 的 pluginOrder/frozenPlugins/hiddenPlugins/disabled_plugins；必要时状态文件哈希 |

无可用 CEF 时继续构建/打包/备份等独立工作，将运行时与界面验证记为待验证；不要凭 SSH 文件检查宣称实机通过。

## 2. 依改动选择构建与检查

```powershell
# 前端改动：从根目录通过 workspace 正确进入 apps/decky
npm test -w apps/decky
npm run typecheck -w apps/decky
npm run build:decky
git -c safe.directory='D:/Project Files/git/steam-hentai-patch-searcher' diff --check
```

依赖不存在时按 lockfile 执行 `npm ci`；已有验证且代码没变时沿用记录。直接调用 Rollup 必须设工作目录 `apps/decky`（`node ../../node_modules/rollup/dist/bin/rollup -c`），根目录执行会找不到 `plugin.json`。

Python 变动运行相关 unittest；CLI 桥接测试需指定实际可运行的 CLI，缺二进制的检查记作未执行。Rust 变动执行对应核心测试和 Linux 构建；Windows `.exe` 不能部署为 Linux `bin/steam-patch`。无 Linux 编译环境时可在独立设备验证目录构建，不把工具链安装进 SteamOS 系统分区。

完整插件以当前 [package-plugin.mjs](../../../../apps/decky/scripts/package-plugin.mjs) 为准：该脚本读取 `target/linux/release/steam-patch`。即使 Cargo 编译成功，也要把**对应新构建**的 Linux CLI 放到这个精确路径，核对来源/哈希，再执行：

```powershell
npm run package:plugin -w apps/decky
# 使用本机实际 Python 可执行路径
python scripts/package-artifacts.py --platform decky
```

`package:plugin` 生成 `apps/decky/plugin-output`；ZIP 打包参数以 `scripts/package-artifacts.py --help` 为准。不要假定已有 `plugin-output` 是本轮新包。完整清单包括 `main.py`、`plugin.json`、`package.json`、`dist`、`backend`（含默认 sources/profiles、vendor、`exe_worker` 和许可证）及 `bin` 的 CLI/catalog/signature/trust 文件；排除测试、fixtures、`__pycache__`、私钥和真实游戏文件。

## 3. 备份、清单与上传

本地 `.artifacts/<feature>-deck-<timestamp>/` 保存新文件清单（相对路径、字节数、SHA-256）、首次旧文件备份及旧清单；完整更新还生成可供 Loader 安装的旧版回滚 ZIP。保留二进制可执行权限。重复部署不覆盖首次有效备份，也不将失败后的混合版本作为原始备份。

远端暂存用唯一目录 `/home/deck/steam-patch-validation/<feature>-<timestamp>/`。上传前验证清单无绝对路径、`..`、重复路径或符号链接逃逸；ZIP 顶层为 `steam-patch-manager/`，成员与清单一致。上传后核对整个 ZIP 的 SHA-256 和每个成员的 SHA-256。

PowerShell 用 `Get-FileHash -Algorithm SHA256`，设备用 `sha256sum` 或 `/usr/bin/python3 -I` 的 hashlib。通过 SCP 参数数组上传到确定的路径，不拼接未转义用户数据执行 shell。前端最小更新同时带 `dist/index.js` 和 map；完整更新必须包括新模块、默认规则及 worker，不能只拷 `main.py`。

## 4. 按权限选部署路径

### 目录可写且变更明确：增量替换

先备份全部将改文件（新增文件记为原先不存在），核实正式目标/父目录非符号链接、路径在插件目录内。将已验证新文件写到目标同目录临时文件，校验后逐文件原子替换，再核对正式 SHA-256。保留原权限；CLI 应具有执行位。多文件替换整体不具原子性，任意失败停止后续更新并恢复本轮清单。

### root 所有或完整插件更新：Loader 本地 ZIP 安装

不要因为历史目录可写就断言本轮可以直接覆盖。通过当前 Loader 支持的安装入口提交已校验的本地 ZIP，不改目录所有权，不以无密码 sudo 作为前提。优先复用 Steam 中已认证的 Loader 连接（运行时参考说明如何发现现有对象）。

历史 Loader 3.2.9 的路由链仅作为核对线索：

1. `utilities/install_plugin(file:///绝对路径.zip, pluginName, versionLabel, archiveSha256, installType)`。
2. 等待 `loader/add_plugin_install_prompt`，核对 name/version/hash/type 和唯一 request ID。
3. 经实际授权的部署流程调用 `utilities/confirm_plugin_install(requestId)`。
4. 核对正式文件清单、唯一插件实例和 CLI 执行位。

当时 `versionLabel='dev'`、`installType=1` 是该设备的开发更新参数；执行前读取当前 Loader 的实现/参数，不直接套用。确认事件不同或接口不可用时停止，不猜测 request ID，不跳过校验。不要单独新开 Loader `/ws` 与 Steam 前端争抢连接。

更新可能改变本插件排序。比对部署前后的 Loader 设置，确有变化时只合并恢复本插件的原成员关系/相邻位置；保留其他插件现状，不整体回写旧设置快照。比较外部状态基线，区分自然缓存刷新与异常改动，不强行恢复数据库。

## 5. 回滚、归档与清理

文件校验或重载失败：停止后续验证，保存异常、服务日志和当前清单。增量更新恢复本轮旧文件及权限，删除本轮新增且身份可确认的文件；Loader 更新安装已验证旧版回滚 ZIP。确认状态与活动任务安全后重载，复查旧版哈希、RPC 及服务；回滚失败或设备失联时报告待恢复路径，不无限重试，也不删除证据。

正常结束后将远端备份、回滚 ZIP、manifest、RPC JSON、日志和截图取回本地并核对哈希。再检查本轮远端暂存目录解析后的绝对路径严格属于 `/home/deck/steam-patch-validation/<本轮目录>/`，不能是验证根目录或插件/state/游戏目录；使用同一远端脚本清理该目录并复核不存在。设备只保留运行必需文件；本地回滚材料和证据继续保留。失联或取回失败时标明待清理，不删除唯一备份。
