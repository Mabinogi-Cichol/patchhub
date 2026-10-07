# 通用 EXE 自动安装调研

日期：2026-10-07（Asia/Shanghai）。状态：**调研及实施验收完成。完整对象图/受限 AST 行为分析、四款本地安装矩阵和最终 Decky 验收结果见施工记录与验收审计。**

关联：[设计与修改方案](design.md) · [施工记录](progress.md) · [原 EXE 设计](../exe-patch-installation/design.md) · [一站式安装设计](../simplified-install-flow/design.md)。

以下第 1–5 节保留初始方案阶段的调查事实；第 6 节为格式验证阶段记录，第 7 节指向完整实施与验收结果。

## 1. 问题与现状

用户要求按安装器格式和行为支持 EXE 自动安装，无需逐游戏、逐版本手动适配。现状不是格式通用安装：来源可以发现许多 EXE，但安装权限只来自三条人工审核 profile。

| 代码位置 | 本轮确认的事实 | 对泛化的影响 |
| --- | --- | --- |
| [exe_profiles.json](../../../apps/decky/backend/exe_profiles.json) | VBF 中文 v5、夏日中文本体 v1.03、夏日中文 DLC v2.02.1，共三条 | 白名单外游戏没有安装 release |
| [exe_profiles.py](../../../apps/decky/backend/exe_profiles.py) `validate_profile` / `_match` / `resolve` / `reviews` | 精确绑定来源记录、AppID、语言、变体、版本；要求固定 EXE/元数据/逐文件摘要、`verified_buildids`、人工 `review` | 即使同一格式、同一复制行为，也必须补新 profile；官网更新字节同样失效 |
| [exe_facts.py](../../../apps/decky/backend/exe_facts.py) `game_facts` | 核验 Steam manifest、实例根、语言、已安装 DLC、build 和 profile 基线 | 实例安全检查可复用；build 白名单和预置 SHA 基线须与运行时快照分离 |
| [exe_installation.py](../../../apps/decky/backend/exe_installation.py) | 投影 `compatibility=verified`；准备前需要 resolve；下载必须匹配预置 SHA/大小；apply 再 resolve | 需把人工 profile 授权改为后端行为报告与内容身份授权 |
| [exe_worker.py](../../../apps/decky/backend/exe_worker.py) / [sf95_worker.py](../../../scripts/sf95_worker.py) | Linux 非特权 bwrap；后端输入固定清单与 `semantics_approved=true`；worker 对照清单生成 ZIP | 隔离与转换可复用；worker 当前不能自行证明未知包语义 |
| [sf95_audit.py](../../../scripts/sf95_audit.py) | 严格 PE/overlay/SF9.5 与 LZMA2 检查；文件表、CRC、摘要、路径和预算校验 | 已有格式基础，不能据此宣称所有 SF9.5 包可安装 |
| 同上 `file_table` / `audit` | 保留大量 `unreviewed_fields` 和跨记录 framing；表前后区域未完整解码；返回 `unparsed_installer_events` 等阻断项 | 完整元数据对象与行为覆盖是核心缺口；单独移除 profile 守卫会跳过未知条件与动作 |
| [main.py](../../../apps/decky/main.py) / [tasks.py](../../../apps/decky/backend/tasks.py) | 安装只接受 release；已有后台安装任务、取消、进度与事务分发 | 增加只读 EXE 分析选择入口，不能让 candidate 直接成为 apply 授权 |
| [index.tsx](../../../apps/decky/src/index.tsx) / [共享类型](../../../packages/ui/src/index.ts) | 未审核候选显示来源引导；通用错误不能解释具体行为缺口 | 增加检查中、条件已核验、具体不支持原因；不能沿用“已审核”冒充新模式的实测验证 |

本轮未发现新的二进制实验材料；本地 `.artifacts/exe-patch-research/` 不存在。旧施工记录保留三个样本的历史实验结论，但本轮没有重新执行或把它们扩展到新游戏。

## 2. 官方资料核验

资料访问日期为 2026-10-07。官网说明反映产品能力；不能替代具体二进制验证。

| 第一方/原作者资料 | 确认内容 | 方案影响 |
| --- | --- | --- |
| Indigo Rose [文件条件](https://www.indigorose.com/docs/suf/program_reference_files_properties_single_conditions.htm) | 文件选择可由 Lua 条件决定 | 文件表存在不等于所有文件都应复制；条件必须转写或拒绝 |
| Indigo Rose [项目与界面事件](https://www.indigorose.com/webhelp/suf10/Users_Guide/Chapter_4_Actions_Scripts_Plugins.htm) | 安装前后、错误、关闭及界面都有脚本入口 | 必须覆盖所有行为入口，不能只搜一段脚本的危险 API |
| Indigo Rose [脚本资源](https://www.indigorose.com/docs/suf/program_reference_resources_script_files.htm) | 外部 Lua 资源可在构建时纳入安装包；含默认全局函数 | 需识别全局资源与调用关系；不能把默认脚本的存在本身当作危险或安全证明 |
| [innoextract 原作者 README](https://github.com/dscharrer/innoextract#limitations) | 提取时不执行包含的脚本和检查 | 可作为后续格式提取基础，不能直接当作完整通用安装授权 |
| [NSIS 官方说明](https://nsis.sourceforge.io/Can_I_decompile_an_existing_installer%3F) | 解包工具与完整反编译有区别，指令编码跨版本变化 | NSIS 应独立完成语义适配，不能只用 7-Zip 解出文件就自动安装 |

Indigo Rose 当前检索到的部分文档为 v10；它们证明需要处理的能力类别，不是 SF9.5 二进制布局规范。历史使用的 sfextract 固定 commit 页面本轮抓取失败，未据此声称重新审核了上游实现。二进制格式结论以当前仓库解析器为依据。

## 3. 妃神会秘史：白名单外目标样本

本轮读取[官方发布页](https://kagurafan.com/2023/03/24/%E5%A6%83%E7%A5%9E%E4%BC%9A%E7%A7%98%E5%8F%B2/)与[官方安装说明](https://kagurafan.com/%E5%A6%83%E7%A5%9E%E4%BC%9A%E7%A7%98%E5%8F%B2-%E8%A1%A5%E4%B8%81%E5%AE%89%E8%A3%85%E8%AF%B4%E6%98%8E/)。页面列出中文 v4、约 1.01G、Steam/GOG 通用与两个神乐下载入口；说明要求同意条款、定位游戏目录后安装。

**已确认**：页面级来源和说明，以及它没有当前安装 profile。**未确认**：当前下载文件大小/摘要、安装器品牌版本、完整文件清单、事件动作、原文件门槛、当前 Steam AppID 绑定、语言/DLC 条件及游戏内效果。本轮通过浏览工具打开一个官方二进制入口未成功，没有下载完整包或执行安装器。

“页面安装步骤简单”不能证明包内仅复制文件；“Steam/GOG 通用”也不意味着本插件要支持任意 GOG 路径。实施前应下载官方当前包并在只读隔离环境解析，完成格式与行为报告。将本游戏设为必须调查的验收样本，不预先承诺格式支持；若受阻，必须归因具体格式/行为/身份缺口，不能新增专属 profile 作为完成泛化的证据。

## 4. 可行性结论与风险

建议先实现 **Setup Factory 9.5 的受限声明式安装行为适配器**，再扩展其他格式。复用已有解码、安全路径、离线 worker、规范化 ZIP 与 Rust 文件事务。主要工作是完整元数据解码、有限脚本分析、游戏条件归一化和动态授权，而不是换解压工具。

通用静态分析不能理解任意 Lua 程序。可行目标是确定且有边界的语法/行为子集：复制文件、只读文件存在/CRC 条件、已知位置探测、条款和界面流程；无法证明结果的动态构造、插件、外部脚本执行或任意副作用应具体阻断。规则按 AST/字段语义匹配，文件名、CRC、AppID 等是参数；不得按某游戏完整脚本文本或摘要授权。

关键风险：对象布局未知导致漏读动作；界面分支或文件属性改变实际复制集合；语法匹配过宽误放行；把现场 SHA 当作原版认证；把“未标语言/DLC”错误当作任意适用；内容更新误继承旧重装范围。修改方案分别设置语义覆盖、来源证据、条件事实、内容绑定和独立验证门槛。

下一步首要实验是对现有三份样本及用户准备的四款本地游戏（你妈真漂亮、妻子变美的理由、妃神会秘史、众人觊觎的女友）建立统一行为报告，统计每个阻断项及证据偏移。此实验尚未执行。只有跨游戏参数化规则能在不新增专属配置时处理新样本，才算方向成立。

## 5. 本地验证范围补充

用户明确这四款游戏已准备在本地，后续开发可以直接用它们验证；Steam Deck 留到最后阶段，主要测试 Decky 前端交互。本轮只记录该约束，未检查本地游戏安装、下载样本或修改游戏。游戏身份、格式与运行环境仍待实施时核实。

后端、格式解析、条件判断和文件事务以本机实验为主要证据；Linux worker 优先用本机已有 WSL2/容器验证，Windows 核心/事实检查分别验证。当前 bwrap worker 不能原生在 Windows 运行，不能以关闭隔离代替本地适配，也不能把本地开发测试入口说成已交付 Windows 客户端功能。详细矩阵与最终 Deck 范围见[设计第 7.1 节](design.md#71-本地游戏矩阵与验证环境)。


## 6. 施工阶段补充：高版本与向下兼容

本节保留施工初期的格式验证事实，完整实施与最终验收结果以[施工记录](progress.md)及[验收审计](acceptance.md)为准。

本地真实官方样本覆盖 SF9.5.2.0（妃神）、9.6.0.1（众人）、9.7.0.0（妻子）。三者共用外层签名、受限压缩与大部分文件表规则，但 9.7 文件记录多一个序列化字节，CString 还存在短/长长度和宽字符编码。因此实现统一版本分派可以兼容这些旧包；简单使用最高版本记录布局不能保证向下兼容。未实验的版本仍属于未知格式，不能隐式接受。

三个完整包已在本机实际 bwrap 中完成全量长度、CRC、SHA 校验，详见施工记录中的版本/大小/时长矩阵；这里只证明当前格式读取，不证明安装行为可安全转写。前置 include 数组已按 framing 解码；当前事件引用扫描只作诊断，无法排除其他对象或属性中的条件/动作，固定禁止授予安装权。后续完整对象图和受限 AST 按通用行为扩展，不加入游戏、build 或包 SHA 许可项。

## 7. 完整实施与最终验收

SF9.5–9.7 完整对象图、全部事件/include 和受限 Lua AST 已接入通用行为分析。三个官方 EXE 使用同一规则和空 profile 注册表完成实际安装；四款本地案例完成安装、同内容重装、失败恢复及文件/存档保护，用户确认均正常启动且补丁生效。最终 Deck 部署/RPC/真实页面、标记为合成任务的进度与取消 UI、用户实体手柄确认及只读大包 worker 验证完成，临时材料已清理。详细证据、自动化范围及未知安装器行为的支持边界见[施工记录](progress.md)和[验收审计](acceptance.md)。
