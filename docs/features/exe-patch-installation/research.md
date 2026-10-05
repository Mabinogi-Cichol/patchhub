# EXE 补丁安装调研

设计：[design.md](design.md)。实际状态：[progress.md](progress.md)。

调研日期：2026-10-05（Asia/Shanghai）。结论：**两份官方本体样本可以静态提取；正式安装仍需安装语义、游戏版本和设备验证。首期应支持 Setup Factory，而非假设 EXE 是 ZIP/7z 自解压。**

## 1. 方法与范围

读取当前工作区的发现、安装、类型、打包代码，核验官方产品和说明页，下载官方 EXE 文件头及两份完整本体样本，静态读 PE manifest/overlay，审查开源提取器源码后做隔离提取与摘要校验。

研究材料在忽略目录 `.artifacts/exe-patch-research/`；没有执行任何补丁 EXE、提取出的程序/DLL/安装脚本，没有访问或修改实际游戏目录，没有部署 Steam Deck。未把官方游戏文件加入仓库或插件。

## 2. 官方样本

来源：[中文本体发布页](https://kagurafan.com/2020/06/14/%E5%A4%8F%E6%97%A5%E7%8B%82%E6%83%B3%E6%9B%B2%EF%BC%9A%E4%B9%A1%E9%97%B4%E7%9A%84%E9%9A%BE%E5%BF%98%E5%9B%9E%E5%BF%86/)、[英文本体发布页](https://www.kaguragames.com/product/summer-memories-patch/)。两个页面均提供两个 EXE 直链及其他下载入口；本轮仅从页面提供的 `kaguraserver.com` 下载，不验证各镜像字节是否一致。

| 项目 | 中文本体 | 英文本体 |
| --- | --- | --- |
| 官方补丁版本 | v1.03 | v3 |
| 完整文件 | [Summer-Memories-CN-Patch-v1.03.exe](https://kaguraserver.com/KaguraGames/CN/patch/Summer%20Memories/Summer-Memories-CN-Patch-v1.03.exe) | [Summer-Memories-EN-Patch-v3.exe](https://kaguraserver.com/KaguraGames/EN/patch/Summer%20Memories/Summer-Memories-EN-Patch-v3.exe) |
| 实际字节 | 126,137,104 | 128,050,144 |
| SHA-256 | `21400ce2540b91a75cc56c3f90bc2f3f276b9ae3bd1e3fbb522675b0c6332130` | `4912d4c92d7fbcd3ee21173eba7298af36dbcef1c84d0d61c93629d419a880e8` |
| manifest 主/次版本 | Setup Factory 9.5 | Setup Factory 9.5 |
| PE overlay 起点 | 251,904 | 251,904 |
| 提取记录总数（含运行时/资源） | 2,507 | 2,897 |
| `%AppFolder%` 下游戏文件 | 2,502 | 2,892 |
| 游戏文件展开字节 | 152,368,950 | 167,968,393 |
| 全部提取记录展开字节 | 155,802,004 | 171,620,860 |

响应头为 `application/x-msdownload`，支持 Range；两份前 1 MiB 请求得到 HTTP 206，完整下载成功且实际大小与响应一致。品牌字符串含 Setup Factory、`irsetup.exe` 和 Indigo Rose；仅凭这些字符串不足以确认版本，故进一步核验 manifest 和 SF8/9 overlay 签名。

两份样本游戏目标均为 `%AppFolder%\www\...`；中文目录包括 audio、data、img、js 和 index.html。五个非游戏记录为 `irsetup.exe`、`lua5.1.dll`、`irsetup.dat`、`IRIMG1.BMP`、`IRIMG2.BMP`。这些文件属于安装运行时/元数据/界面，不应复制进游戏。游戏 JS 是正常载荷，本轮未运行它们。

## 3. 提取实验与局限

使用 [sfextract 原作者仓库](https://github.com/CybercentreCanada/sfextract)源码 ZIP，Git commit（ZIP comment）为 `21f5c5595a70e9284fff83476c0e307d5b92f573`，源码 ZIP SHA-256 为 `8e418394125b199b29a1d43ca3e90a226602fabf2441ec53c82ed75817bded61`。

实验不是原样运行 `sfextract` CLI：开发机缺少 `pefile`，研究脚本自读 PE section offset 和 manifest，并直接复用 `SetupFactory8Extractor` overlay parser；使用隔离输出目录、写路径 audit、禁用子进程/动态库执行、512 MiB 单文件与 1 GiB 总解压限制、128 MiB LZMA 字典限制，拒绝未支持的压缩方式。纯 Python `pefile` 源码也已下载供阅读，但没有用于本次运行或验证完整依赖组合。

首次实验仅允许 LZMA，因样本实际使用 LZMA2 而主动失败，留有部分运行时研究输出。补充受限 raw LZMA2 后，在新目录对两份样本成功完成静态提取；逐记录复核声明大小、CRC（存在时）和 SHA-256。观察仅证明该 parser 能读出这两份样本并通过上述字节检查，不证明 parser 覆盖完整安装语义或生产安全。

本机文件：`probe.py`、`cn/en-prefix.headers`、`cn/en-full.headers`、完整样本；`*-static-v2/summary.json` 保存逐成员长度和摘要，中文目录中的 `installer-script-strings.json` 与 `event-string-windows.json` 保存筛选的安装器元数据线索。上述材料均留在忽略目录，仅本文的概要与摘要进入 Git。

## 4. 安装语义证据

[中文官方安装说明](https://kagurafan.com/%E5%A4%8F%E6%97%A5%E7%8B%82%E6%83%B3%E6%9B%B2%EF%BC%9A%E4%B9%A1%E9%97%B4%E7%9A%84%E9%9A%BE%E5%BF%98%E5%9B%9E%E5%BF%86-%E8%A1%A5%E4%B8%81%E5%AE%89%E8%A3%85%E8%AF%B4%E6%98%8E/)与[英文官方说明](https://www.kaguragames.com/patch-instructions/)均要求同意条款，并指向 Steam 游戏目录。中文发布页明确本体补丁不能用于 GOG，购买对应 DLC 时应使用另一补丁；不能把同名游戏、同一个父 AppID 的本体/DLC 合成一个安装 release。

中文 `irsetup.dat` 的局部静态字符串和事件窗口显示：安装位置探测会检查 `Game.exe`、读取 Windows 注册表及搜索 Steam 目录；目录选择校验还读取 `Script.vdf` CRC 并要求特定值，失败提示需要 Steam 版本。这说明安装器具有发行渠道/文件条件，单纯取出 `%AppFolder%` 文件会跳过原有检查。

本轮没有完整反序列化所有事件、文件条件或安装类型；搜索不到某个 API 字符串不能证明不存在相应动作。没有将 CRC 值与当前游戏文件、语言和 build 对照，没有验证中文/英文载荷的实际生效、存档或恢复。可将已经审核的位置探测替换为可信 Steam 扫描，但必须先证明其他动作不影响结果，并转写必需前置条件。

## 5. 第一方工具资料

| 技术 | 资料与确认能力 | 对本功能的结论 |
| --- | --- | --- |
| Setup Factory | Indigo Rose [单文件和压缩配置](https://indigorose.com/webhelp/suf80/Program_Reference/Build_Settings/Setup_File.htm)、[脚本与插件](https://indigorose.com/webhelp/suf80/Users_Guide/Chapter_4_Actions_Scripts_Plugins.htm)、[文件条件](https://indigorose.com/webhelp/suf80/Program_Reference/Files/Properties/Single/Conditions.htm) | EXE 可携带压缩载荷，也可含 Lua、条件和分卷；不能等同于普通压缩包。这些文档说明产品能力，不证明样本使用全部能力 |
| sfextract | [README](https://github.com/CybercentreCanada/sfextract)、[SF8/9 parser](https://github.com/CybercentreCanada/sfextract/blob/21f5c5595a70e9284fff83476c0e307d5b92f573/src/sfextract/setupfactory8.py)、[依赖声明](https://github.com/CybercentreCanada/sfextract/blob/21f5c5595a70e9284fff83476c0e307d5b92f573/pyproject.toml) | Linux 可运行、MIT、声明支持 SF7/8/9；当前样本与其 SF9.5 分支匹配。正式集成须改造路径、大小、CRC 和条件处理 |
| 7-Zip | [格式列表](https://www.7-zip.org/)、[Linux CLI 下载](https://www.7-zip.org/download.html) | 支持 7z SFX、NSIS 解包等；官方未列 Setup Factory 或 Inno Setup，不能承诺解决本次样本；本轮未运行 7-Zip |
| innoextract | [原作者说明](https://github.com/dscharrer/innoextract)、[CLI/路径和未知格式限制](https://github.com/dscharrer/innoextract/blob/master/doc/innoextract.1.in) | 可无 Wine 提取 Inno；格式与本次样本不同，首期不用。提取不执行脚本/条件，需独立审核 |
| NSIS | [官方解包限制](https://nsis.sourceforge.io/Can_I_decompile_an_existing_installer%3F)、[脚本指令](https://nsis.sourceforge.io/Docs/Chapter4.html) | 提取文件不能完整还原安装脚本，作为以后独立适配范围 |
| UniExtract2 / unshield | [UniExtract2](https://github.com/Bioruebe/UniExtract2)、[unshield](https://github.com/twogood/unshield) | 前者为 Windows/AutoIt 应用；后者处理 InstallShield CAB，不能混同为 Linux Setup Factory 提取器 |

上游 `sfextract` 安全审查发现：直接按包内目标路径写文件、解码使用 `errors=ignore`、读取并一次性解压数据、缺少全局资源预算；`ParseScript` 跳过文件条件、安装类型等字段，对普通游戏载荷未独立复核所有 CRC/实际长度。源码的 `match/case` 还使实际 Python 最低版本高于 metadata 所列 3.8。以上是源码观察，未进行攻击样本验证。

## 6. 当前仓库约束

- [发现模块](../../../apps/decky/backend/discovery.py)固定产生 `discover_only`，[`_installable_release`](../../../apps/decky/main.py)拒绝 candidate ID；已有 [Kagura 功能](../kagura-hgamer-fallback/progress.md)仅完成发现/引导。EXE 功能需要新的受审核 release 关联，不应移除这个守卫。
- [publisher.py](../../../apps/decky/backend/publisher.py)通过本地临时签名 catalog 调用 [CoreBridge](../../../apps/decky/backend/bridge.py)，实际游戏写入/恢复在 [Rust](../../../crates/core/src/install.rs)。现有文件事务是可复用基础，不需要 Python 再写一套备份/覆盖器。
- [安装设计](../../install-design.md)限制 ZIP 4,096 条目、8 GiB 归档、单文件 4 GiB、总展开 16 GiB，并包含路径、链接、摘要和事务校验。本轮两份游戏载荷均小于条目上限，但不能因此保证所有 Kagura 包都满足；超限样本应明确拒绝。
- 当前 publisher ZIP 的本地配方生成会绑定当前 build/平台；新 EXE 流不能把这个行为当成“当前 build 已验证”的依据，必须明确 profile 的平台和版本验收证据。
- [缓存功能](../cache-cleanup/design.md)默认不保留备份；首批 EXE 受控安装需显式保留，才有实际恢复闭环。下载缓存清理不得清理有效 plan、转换材料或所需备份。

## 7. 验证结论与剩余工作

已完成：官方页面和 EXE 链接核验、两个完整样本下载/大小/摘要、SF9.5 manifest/overlay 识别、受限静态提取及逐成员字节复核、局部事件线索调查、当前代码链路与工具源码调查。

待完成：完整安装元数据语义、当前游戏条件/语言/DLC/build 核验、生产提取器路径与资源测试、完整 `sfextract/pefile` 组合、Linux 打包与系统 Python、真实 Steam Deck 安装/运行/生效/卸载恢复及实体手柄验收。本轮没有实现功能，也未运行项目功能测试或构建。
