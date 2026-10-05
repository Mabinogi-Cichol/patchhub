# P0 审计工具与设备适用条件

设计：[design.md](design.md)。状态及历史：[progress.md](progress.md)。日期：2026-10-05（Asia/Shanghai）。

**结论：P0 部分完成；现有两份本体样本继续保持来源引导。当前 Deck 安装了 DLC，不符合本期本体补丁范围；完整安装事件与序列化字段也尚未审核。按设计停止条件，不进入这些样本的自动安装。**

## 实现与复现

- [sf95_audit.py](../../../scripts/sf95_audit.py)：仓库维护的独立只读审计 parser，Python ≥3.10，仅依赖标准库 `lzma`、`mmap`、XML 和哈希模块。不调用上游 CLI，不加载 PE/DLL，不运行 Lua；审计 CLI 不写出游戏成员或安装配方。后续生产 worker/RPC 已按合成样本实现，状态以 progress 为准。
- 文件表布局参考固定 `sfextract` commit `21f5c5595a70e9284fff83476c0e307d5b92f573`，保留 [MIT 许可](../../../scripts/licenses/sfextract-MIT.txt)。未复制 `blast.py`、`pefile` 或其他解码后端。本期维护方式决定为仓库内专用 parser；不是完整 `sfextract + pefile` 依赖组合的验证。
- 通过 PE section/resource directory 读取 manifest，校验 SF9.5 及 overlay。证书区单独检查范围、格式和对齐零字节，并记录摘要；不声称完成 Authenticode 签名验证。
- 写前解析完整文件表、严格 UTF-8 和 `%AppFolder%` 目标，校验大小写/Unicode、设备名、逃逸及文件目录冲突。随后流式解码 LZMA2，检查字典预算、结束状态、尾部、实际长度、所有游戏成员 CRC 与 SHA-256。运行时只按存储字节记录，不当作游戏载荷。
- 文件条件、安装类型、包、快捷方式和属性等字段均进入报告；未知字段保存偏移、原始 base64 和摘要。文件表外区域只记录边界与摘要，**事件脚本没有完整反序列化/审核**。空文件条件不能证明不存在安装事件、全局条件或特殊替换策略。
- 成功报告固定包含 `capability=discover_only`、`installable=false`、`semantics_reviewed=false` 和阻止原因；报告成功不创建 release，不授予安装权限。后续 EXE 工程由独立受审 registry 授权，候选身份仍不能直接调用安装入口。

v2 边界修正：原参考布局在 CRC 后固定跳过 8 字节，跨越下一序列化对象，不能把它标为上个文件字段。当前记录摘要止于已知 CRC，非末条间隔单列未知 framing；末条后全部 bytes 保留为未知尾区。中文/英文尾区分别始于 `1383485` / `1590969`，大小 `68441` / `80370`。旧 v1 文件载荷校验未改变，但记录/表边界语义证据须使用 v2；详见 progress 的修正记录。

开发机/系统 Python 用法（官方样本仅在忽略目录中，仓库和插件不包含样本）：

```sh
python -m unittest discover -s scripts -p test_sf95_audit.py -v
python -I scripts/sf95_audit.py .artifacts/exe-patch-research/Summer-Memories-CN-Patch-v1.03.exe > .artifacts/exe-patch-research/cn-audit.json
python -I scripts/sf95_audit.py .artifacts/exe-patch-research/Summer-Memories-EN-Patch-v3.exe > .artifacts/exe-patch-research/en-audit.json
```

Linux worker 设置地址空间 1 GiB、CPU 180 秒、输出文件 16 MiB、core dump 0；流式解码另有墙钟预算 180 秒。报告限制 16 MiB。可用 `--metrics` 将当前 worker 自身的时间/RSS 写到 stderr。开发机 Windows 无 Linux 资源隔离，不能用这次开发机检查替代生产 sandbox。

## 自动化与样本证据

[12 项自制合成测试](../../../scripts/test_sf95_audit.py)在 Windows 与 Deck 的系统 Python `-I` 沙箱中通过。涵盖 PE/resource 边界、SF10/未支持版本、证书 framing、XML 实体、中文路径、严格解码、目录逃逸、设备名、Unicode/大小写碰撞、文件目录冲突、超额计数/展开/字典、截断、坏 CRC、错误声明长度、压缩尾部、超时及 CLI 输出重定向。样本在测试代码中生成，不包含官方安装器或游戏文件。已将测试加入 Windows/Linux CI；本轮没有触发远端 Actions。

| 样本 | 游戏文件 / 展开字节 | 文件条件 / 类型 / 包 / 快捷方式 | 与旧实验清单对照 |
| --- | --- | --- | --- |
| 中文 v1.03 | 2,502 / 152,368,950 | 条件为空；类型 `All`；包与快捷方式为空 | 全部游戏成员大小和 SHA-256 一致，CRC 逐成员通过 |
| 英文 v3 | 2,892 / 167,968,393 | 条件为空；类型 `All`；包与快捷方式为空 | 全部游戏成员大小和 SHA-256 一致，CRC 逐成员通过 |

原 EXE 摘要与[研究记录](research.md)一致。审计记录含五个运行时/元数据资源，全部展开量分别 155,802,004 / 171,620,860 字节。文件表之前均有 44,642 字节未解析；中文/英文表后分别有 68,433 / 80,362 字节未解析。文件表内部还有未审核的字段；这些是真实的 P0 门槛，不将字符串窗口推断算作完整语义审核。

## Steam Deck 隔离与资源实测

- 已连接现有 Devkit；系统 Python `3.13.5`，标准库 `lzma`、`resource` 可用，`/usr/bin/bwrap` 可用。
- 测试目录：`/home/deck/steam-patch-validation/exe-audit-20261005-092e/`，独立于游戏与插件。上传本轮脚本、自制测试、两份样本供静态审计；未执行 EXE、DLL、Lua 或提取出的游戏代码。
- bubblewrap 使用 `--unshare-all --die-with-parent --new-session --clearenv --cap-drop ALL`，只读挂载 `/usr`、`/lib`、`/lib64` 与受控输入目录，单独 `/proc`、`/dev` 和临时 `/tmp`。沙箱内 `/home`、外置库 `/run/media` 不存在；只有 loopback，外网 TCP 探针被拒绝。
- 取消探针启动一个 Python worker 及受控 sleep 子进程；终止宿主进程组后，追踪到的全部四个进程均退出或成为待回收 zombie，没有仍在运行的后代。这里验证的是该设备 bubblewrap 能力，生产 TaskManager/取消/租约接入仍待 P2/P3。

| 样本 | worker 审计耗时 | worker 自身峰值 RSS | 外层沙箱总耗时 |
| --- | --- | --- | --- |
| 中文 v1.03 | 3.302 秒 | 169,200 KiB（约 165 MiB） | 3.475 秒 |
| 英文 v3 | 2.493 秒 | 173,780 KiB（约 170 MiB） | 2.658 秒 |

均低于设计拟定预算，本轮保持原预算，不扩大限制。这是只读审计实测，不是全量 EXE 下载、转换、Rust staging 与安装耗时。最终中文/英文报告 SHA-256 分别为 `116e2bee3b0f60cb9156d25a7d6b1da5a0394a636fd526dc6c5e38989c534a05` / `f8c76ce6b24ec3907679580edf891f7e114778303ec9a6de7483f23423012559`。

本轮证据位于本地 `.artifacts/exe-patch-research/`：`cn-audit.json`、`en-audit.json`、`deck_probe.py`、`deck-probe-result-final.jsonl`、`deck-game-baseline-after.json`；设备目录保存 `cn-audit-final.json` / `en-audit-final.json`。原始样本不加入 Git。首次 Linux 报告重定向因 `RLIMIT_FSIZE=0` 失败，修复为受限 16 MiB 并补回归检查后通过。取消探针首次按进程组枚举忽略了 bwrap 新会话后代，改为父子关系枚举后通过；不得采用首次失败结果作为通过证据。

## 当前游戏条件：明确拒绝本体补丁

只读 Steam manifest 显示：AppID `1227890`，内置库，build `6270017`，已安装/挂载语言 `schinese`；已安装 depots `1390151`、`1390152`，二者归属 DLC AppID `1390150`，安装脚本来自 DLC depot。未调查账户是否另有未安装 DLC 权益，本轮也不推断“没有购买其他 DLC”。

| 文件 | 大小 | SHA-256 | CRC32 |
| --- | --- | --- | --- |
| `Game.exe` | 1,806,456 | `16aef53d35ad834bfebc1c2d765476f898867d32384957dc2362ed926ee49a81` | `1410843218` |
| `Script.vdf` | 508 | `77225a3c852d8a632c478f98556ef00863a4241e9381f36d23f1a38368e78fa9` | `2002373744` |

两者均为普通文件，测试后摘要再次只读复核一致。沙箱没有挂载游戏，因此审计 worker 无法写入游戏目录；未作全游戏目录的前后摘要审计，不将关键文件复核扩称为全量目录审计。

中文本体样本的局部目录确认脚本要求 `Game.exe` 存在，并要求 `Script.vdf` CRC32 为 `3876755782`；该片段是静态线索，不是完整事件审核。设备实际 CRC 不满足，且 `Script.vdf` 的 registry 名明确为 `Summer Memories DLC`。按原设计的本体/DLC 条件，这个实例不得使用本轮本体样本，不修改检查、不将当前 build 记为已验证，也不为了测试卸载/禁用用户 DLC。

## 后续静态 runtime 核对（2026-10-05）

独立审查 Agent 继续在研究副本上工作；工具、解包副本、反汇编和探针只保存在忽略的 `p0-semantic-20261005/` 前缀。没有运行安装器或解包后的 runtime，也没有把研究工具加入插件。

`CSetupFileData` 的工厂、构造器、vtable 与 `Serialize(CArchive&)` 读写分支已交叉定位。下表的属性名称来自 runtime 属性导出，数值来自两样本逐条探针；不是按常量或公开文档猜测 byte schema：

| 对象成员 | 已定位属性 | 中文 2502 条 / 英文 2892 条 |
| --- | --- | --- |
| `+0x6C` | Overwrite | 均为 2；覆盖决策分支跳过版本/日期比较而设置 copy-eligible，上游文件/系统/包条件仍须单独审核 |
| `+0x6D` / `+0x6E` | CreateBackup / FailSafeCopy | 均为 0 |
| `+0xA4` / `+0xA5` | RegisterAsOCX / RegisterAsTLB | 均为 0 |
| `+0xA6` | SuppressInUseNotice | 均为 0 |
| `+0xA7` / `+0xA8` | CompressFile / 使用原始属性标志 | 与已审计清单逐条对齐，提供独立偏移核对 |
| `+0xAC` / `+0xB0` | 新文件属性位 / InstallOrder | 分别为 0 / 1000 |
| `+0xB4` / `+0xB5` / `+0xB6` | DisableCRCCheck / NeverRemove / SharedSystemFile | 均为 0 |

本项目 Rust 强制保留备份是产品恢复策略；官方安装器的 CreateBackup=0 不会取消本项目备份要求。上述 copy-eligible 分支也不能独立证明最终写入行为。

对象边界进一步得到 serializer 证据：CRC 后首字节为当前文件对象 `+0x11C`；非末条随后才是旧类 tag、DWORD 版本 1、下一对象首字段。末条该字节后的 4 bytes 仍未知，然后才进入屏幕类标记。嵌套 `+0xB8` 对象以 count×2 bytes 序列化，两样本每条 count=16，内容为 `0x8000` 与 15 个 `0xffff`；其业务语义尚未确认，不能猜成已审核系统条件。生产 v2 parser 仍采用保守未知 framing，未因该调查修改已交付包。

审查报告按类/表标记列出剩余区域：文件表前全局 include；屏幕/安装设置；会话集合 count；Package 对象；密码/卸载区；图像/语言区。该阶段账本中的 CN / EN `U` 区间合计为 46,587 / 58,516 bytes，另有逐文件的 opaque 属性；这是字节分类快照，后续字段映射不自动更新账本，不能把计数等同于语义审核剩余量。字节账本完整不等于语义完整。完整范围和证据见研究目录 `report.md`，最新工程与真实验收状态见 [验收矩阵](acceptance.md)。正式 registry 保持为空。

Package01 的 prefix 后续由独立探针核对：CN `1413362/1413366/1413370`、EN `1623783/1623787/1623791` 的三个 DWORD 对应 runtime `Install`、`Enabled`、`AdditionalDiskSpace` 属性，均为 0；各成员文件的 packages 列表为空，局部 Lua 中启用该包的行被注释。其余包字段和全局动作仍待审核，不能据此宣称项目语义完整。

安装器自身卸载数据创建也得到独立序列和消费分支证据：`CPasswordData` 后的 `CUninstallData` version=1；CN `1413535`、EN `1624000` 的 CreateUninstall DWORD 均为 0，随后三个长度框定字符串为 `uninstall.xml`、`%AppFolder%\\Uninstall`、`%AppFolder%\\uninstall.exe`。runtime 的 CreateUninstall 阶段对该成员为 0 时跳过创建，两个官方样本的这些路径属于未启用的默认配置。双样本 prefix JSON 保存在研究目录，已核对字段偏移/版本/值/顺序；未据此省略其他项目级审核，也不影响本项目 Rust 的强制备份与恢复策略。

## 后续门槛

1. 对本体样本完整反序列化并审核事件、序列化未知字段、替换策略与条款，证明声明式方案能复现全部必要动作；无法可靠映射则维持引导。
2. 准备满足语言、本体/DLC 与 `Script.vdf` 条件的真实实例，取得可信文件/build 基线；当前 DLC 实例不能充当本体安装验收。
3. 若要覆盖当前实例，应先扩展独立 DLC 样本范围、取得官方 DLC 安装包并重新审核；本轮范围没有擅自扩展到 DLC。
4. 官方 profile 开放须满足上述门槛；P1–P4 工程已改按合成样本独立推进，最新 ZIP 转换、事务、持久备份、Decky UI 与打包事实见 [progress.md](progress.md)。本篇 P0 只读证据不代替官方安装生效/存档/恢复与实体手柄验收。

## VenusBlood FRONTIER CN v5（2026-10-05）

用户提供 AppID 1189440、build 5430305、schinese、无 DLC 的真实实例。Kagura FAN 官方中文补丁 v5 为 Setup Factory 9.5（与夏日同系列），但只含 3 个游戏根目录成员，语义比夏日简单：

| 项目 | 值 |
| --- | --- |
| EXE SHA-256 / 大小 | `502063d21d2e741533e7479955b6bbdc68dd36d55ad3cd01e86fa509d9056410` / 1,126,694,464 |
| 元数据 SHA-256 | `5a0ca3e10199a457ac8a52c83f4d61bc4662f2bbeb5a149825263b2ee558f5e3` |
| 游戏文件 | `extra.xp3` 584,428,965；`extra2.xp3` 648,789,888；`R-18 Patch readme (CN).txt` 1,580 |
| 条件 / 类型 / 包 | 全空 / `All` / 无 |

事件只按 `VBFI.exe` 定位并推进界面、取消 `Application.Exit`、通用关机重启助手；framed 文本内无 `File.Copy/Delete/Run/Move`、无 `Web.*`、无注册表写。按纯新增建模，基线为 `VBFI.exe`。该结果只授权 VBF 的审核 profile，不外推到其他游戏或安装包版本；夏日两份样本的完整项目级语义仍待受控样本或发行商工程文件。详细报告在忽略目录 `.artifacts/exe-patch-research/vbf/`，真实安装/恢复证据见 [progress.md](progress.md)。

## 夏日狂想曲官方 DLC 补丁 CN v2.02.1（2026-10-05）

夏日安装器（本体与 DLC）的类清单只有文件表、`CIREvent`、界面/控件、语言/消息、`CSessionVar`、`CSUFPackage`、`CPasswordData`、`CImageInfo`、`CGlobalIncludeLuaFile`，没有任何文件动作/运行类；非事件字符串只有项目元数据、被禁用的卸载路径和通用错误文案；framed Lua 无 `File.Copy/Delete/Run/Move`、无 `Web.*`。据此确认夏日行为＝文件表复制 + 已解码事件。

| 项目 | DLC CN v2.02.1 |
| --- | --- |
| EXE SHA-256 / 大小 | `de370e86645e273bae205517b49077a9fd682a9d73309c518f5524d96216ecad` / 279,515,928 |
| 元数据 SHA-256 | `6cc974e6b7a1c997097554acff0d0cb7cb8d1a8f677a38b5f86a4166d6699baf` |
| 成员 | 5,874 个 `www/` 文件（9 新增 / 5,865 替换），条件空、类型 `All`、未压缩 |
| 门槛条件 | `Game.exe` 存在且 `Script.vdf` CRC = 2002373744（DLC 版） |

DLC 屏幕的 `SetPackageProperties` 为注释、未启用。该结果授权夏日 DLC profile（`variant=dlc`，`requires_dlc=[1390150]`）；本体系列（CN v1.03 / EN v3，门槛 CRC 3876755782、排除 DLC）语义同源。2026-10-05 在临时移除 DLC 的实例上核验本体 CN 基线 `Script.vdf` SHA-256 `5cdc0e696aea4ea526840c1327bbb258846cbc7c703982c6e0aa833420554d6c`、CRC 3876755782，并完成本体 CN v1.03（2,502 文件）真实安装与全量恢复；英文本体 v3 待英文实例。真实安装/全量恢复证据见 [progress.md](progress.md)。
