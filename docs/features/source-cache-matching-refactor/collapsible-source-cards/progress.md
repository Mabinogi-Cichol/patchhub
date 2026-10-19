# Decky 可折叠订阅源卡片施工记录

- 设计：[design.md](design.md)。
- 父 feature：[设计](../design.md) / [施工记录](../progress.md)。
- 当前状态：**C1–C3 已实现并通过自动化；C4 已部署、真实页面/键盘/RPC 验证通过，实体手柄与真实网络任务待验收**。未将部分设备证据标为完整验收。
- 分支：沿用父功能分支 `feature/source-cache-matching-refactor`。

## 阶段状态

| 阶段 | 状态 | 证据/下一步 |
| --- | --- | --- |
| C0 设计 | 已完成（2026-10-19） | 设计 §1–§8；本文件与父功能、总索引关联 |
| C1 卡片结构 | 已实现 | 普通源/HGamer 独立容器、深色标题、边框/间距及来源内操作 |
| C2 折叠与会话 | 已实现，自动化/真实 DOM/键盘通过 | 默认收起、多卡独立、路由保留、焦点、aria；实体手柄待验收 |
| C3 任务与本地验证 | 已完成（2026-10-19） | 前端 67 tests passed、typecheck/build/package 通过；实现精简自审 |
| C4 Steam Deck 验收 | 已部署/部分实机通过，待完整验收 | 正式文件/HTTP/RPC/真实页面/键盘通过；实体手柄、真实任务未执行 |

## 2026-10-19：建立子 feature 完整计划

- 用户先要求优化来源之间的分界，后补充每个卡片可折叠，并明确先编写计划，作为 `source-cache-matching-refactor` 的子 feature。
- 已核对当前 `SourcesPage` / `SourceCard`：多个源以 Fragment 连续渲染，更新间隔与操作在 SourceCard 外；补查来源亦无独立容器。这是本次分组改造的实际依据。
- 已核对 `createManagerSession` 与页面级轮询：规划按 source ID 在现有前端 session 记录展开状态，避免路由卸载丢失，也避免折叠影响后台任务。
- 明确默认全部收起、可同时展开多个、同一前端会话保留、前端重载重置；整条标题支持 A/Enter/Space，B 保留页面返回；标题持续显示状态/任务摘要。
- 设计覆盖范围、产品契约、技术落点、C0–C4 实施拆分、自动化与实机矩阵、风险及过度设计自审。复用父分支和父功能目录，不将子项拆为脱离父功能的独立顶层功能。
- 保留此前父 feature 部署记录及未提交文档改动。此次未修改功能代码、未编译、未测试、未部署、未操作设备、未提交、未合并。

## 后续验收记录要求

实施后按阶段补充修改文件、实际命令与结果；记录源码版本/工作区范围、部署清单与备份、页面截图、viewport/overflow、路由与焦点行为、状态恢复、任务测试的真实/合成口径。

完整验收矩阵见设计 §6。所有项目当前均待验证；不得引用父 feature 之前的部署成功，代替本子 feature 的实现或实机证据。实体手柄未操作时明确保留待验收。


## 2026-10-19：C1–C3 实现与本地验证

- 用户授权“开始实施”；沿用父分支与此前文档改动，未创建新分支、未提交或合并。
- `apps/decky/src/index.tsx`：SourceCard 统一普通源/HGamer 容器与标题，移入所属来源的开关/间隔/更新/取消操作；标题展示名称/启用状态/箭头/任务或失败摘要，删除重复名称与只读启用状态行；无嵌套按钮，正文按展开状态渲染。
- `session.ts`：expandedSources + 同步 toggleSourceExpanded，复用现有快照通知；跨页面卸载保留，同一源以 ID 定位，新 session 默认为空。展开/收起不调用业务 API、不改变任务。
- `source-tasks.ts`：sourceCardSummary 复用现有任务/失败语义，活动任务优先，取消显示取消中；HGamer 使用“补查失败”。来源任务失败结束后重读一次来源摘要；读取失败保留原任务错误并附读状态错误。
- 新增 6 项行为测试（session 4、source-cards 2），覆盖独立展开/跨订阅/快照变化、任务不中断、前端重置、失败后摘要刷新和双重错误、摘要优先级与 HGamer 文案。
- 验证：`npm test -w apps/decky` **67 passed / 0 failed**；`npm run typecheck -w apps/decky`、`npm run build:decky`、`npm run package:plugin -w apps/decky`、`scripts/package-artifacts.py --platform decky` 通过。纯前端改动，未重跑 Rust/Python 全量测试。
- 依据真实截图修正 Steam Button 默认亮白标题背景，改为深色背景、亮色文字、独立焦点描边；随后重新执行 typecheck/build/package。移除无用 Fragment 导入并同步最终产物。
- 按 overengineering-review 完成静态实现自审；执行路径直接，未增加后端、持久化、配置或通用组件框架。该自审不替代实体手柄及真实任务验收。


## 2026-10-19：C4 编译部署与实际页面验证

- Devkit 当前所选 `steamdeck`，使用当前配对密钥、已信任主机记录和独立 known_hosts。安装前/最终安装及来源任务均空闲。Loader v3.2.10，插件目录 root 所有，采用 Loader 本地 ZIP + SHA-256 正常重装流程。
- 本项仅修改 Decky 前端；完整包沿用本 session 已构建并核验的 Linux CLI/worker，未将 Windows 产物部署为 Linux。最终包 23 文件，7,184,784 字节，SHA-256 `54227ce93a65d867dd638277616f27e7f58ec89f2739380c0e7387be14dc38a0`。
- 正式文件 23/23 大小/哈希和 CLI 执行位通过；实际认证 HTTP bundle 与构建/正式文件 SHA-256 一致：`8b6f1ff47a9fae0578c80bc8494d928014c0742d3da198b5bbddee57479341cd`。
- 首次旧插件/完整状态/设置/回滚 ZIP 保留本地，7 个备份文件与远端 SHA-256 全部一致。最终原有外部状态文件哈希无改变、无缺失；未写入展开偏好到后端。Loader 重装改变的本插件排序已按原成员关系恢复。
- 真实 RPC：28 个游戏实例、4 个来源、23 条历史事务，单实例投影可读；最终插件实例为 1、安装/来源活动任务均空，服务 active。扫描保留两个既存 `library_unavailable` 挂载诊断，本项未改设备挂载。
- 部署中 Loader 文件监测触发重复后端重载，首个页面扫描请求未正常返回；正常前端 importPlugin 尝试也出现 30s 导入超时。依技能确认无活动任务并告知影响后，调用 Steam JS 上下文重启；重新枚举 targets、运行 RPC 与真实交互，页面恢复。该事件及导入中断期间的空页面/route_state_lost 取证不算通过；最终验收以重启后的成功结果为准。未手工结束后端或重启设备。
- 最终 UI 证据位于忽略目录 `.artifacts/collapsible-source-cards-deck-20261019/`：`cards-after-context.json`（四卡默认收起及 aria）、`interactions.json`（双卡独立/折叠/缓存页往返保留/DOM 焦点/HGamer 控件）、`keyboard.json`（真实 CEF Enter 展开、Space 收起且焦点保持）、`layout.json` / `long-url.json`（实际 CSS viewport 854×534，无横向溢出）。
- 已查看 `collapsed.png`、`expanded.png`、`hgamer.png` 与长网址布局数据；`long-url.png` 保存 Kagura 海外两入口换行；`qam.png` / `qam.json` 记录真实 QAM 7 款有补丁游戏、viewport 854×454、无横向溢出。新的样式限定卡片类，未扩展到单游戏来源页。
- 本轮只读检查/折叠导航/键盘输入，未更改源开关或间隔，未启动真实来源更新、重匹配或补丁安装。标题 HGamer“最近补查失败”来自设备已保存的尝试状态，不是本轮网络测试失败。
- 最终回到管理订阅源页并收起本次打开的卡片，关闭菜单；首次备份与证据已本地归档，远端本轮唯一暂存目录和本轮隧道已清理，见 `cleanup.json`。

## 当前验收矩阵

| 项目 | 状态 | 证据/限制 |
| --- | --- | --- |
| 首次/前端新会话默认收起 | 通过（真实设备） | 上下文重启后四卡均 false，正文不挂载 |
| 多卡独立展开/收起 | 通过（真实 DOM 操作） | interactions.json，收起一张不影响另一张 |
| 来源控件归属/HGamer 差异 | 通过（静态 + 真实 DOM） | 普通源正文含原控件；HGamer 无间隔/更新按钮 |
| 来源页与缓存页往返 | 通过（真实导航） | 同 session 保留双卡展开选择；进入游戏页返回由相同 session 契约覆盖，未单独实测 |
| 列表/启停变化/新 ID/新 session | 通过（自动化） | session 行为测试；未实机改源设置 |
| 折叠不触发业务操作/不中断任务 | 通过（自动化 + 静态） | 任务保持 running，未 cancel/restart；真实网络任务折叠待验证 |
| 活动任务/失败/取消摘要 | 通过（自动化）；真实失败状态可见 | source-cards 测试，HGamer 已保存失败实际显示；真实活动进度/取消待验证 |
| aria/无嵌套按钮/正文移除 | 通过（真实 DOM） | 标题原生 BUTTON，aria-expanded/controls 正确 |
| Enter/Space、DOM 焦点保持 | 通过（真实 CEF 输入） | keyboard.json，不等价实体手柄 |
| 手柄 A、方向导航、B 返回 | 待验收 | 未进行实体手柄操作 |
| 长 URL、分界、底部、QAM | 通过（真实页面/截图/布局数据） | 长 URL 无溢出；自定义超长源名称未实机添加 |
| 后端单独重载保留/真实任务生命周期 | 待验证 | 前端纯内存契约已测试，未做独立后端重载及真实网络任务实验 |

结论：实现、自动化、部署与真实页面/键盘冒烟完成；完整实机验收尚需实体手柄和真实任务验证。保持功能分支，不提交、不合并 main。
