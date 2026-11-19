请为 PatchHub 生成一份符合规范的单文件自描述 Lua 5.4 订阅源脚本。请严格按以下四步执行：

1. **先拉取权威规范**：在开始编写前，先读取 GitHub 仓库的规范文档并严格遵循其中全部契约与红线：
   - 优先读取 Raw：`https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/docs/custom-subscription-sources.md`
   - 备用网页：`https://github.com/Mabinogi-Cichol/patchhub/blob/main/docs/custom-subscription-sources.md`
   - 若当前环境无法联网或拉取失败，请立即停止并请用户提供该文档内容或本地路径后再继续。

2. **强制首轮提问收集参数**：优先调用你的交互式提问工具（如 `ask_question`、`AskUserQuestion` 等；若无提问工具则首轮直接发问并等待用户回复），向用户确认以下信息：
   - **订阅源类型**：索引源（`publisher`，站内有完整补丁列表/目录/RSS/API）还是补查源（`fallback_indexer`，按游戏名站内搜索）？
   - **订阅源入口网址**：完整的 `https://...` URL。
   - **可选补充**：期望的显示名称，或特殊访问/筛选说明（如分页结构、年龄确认、语言限制等）。

3. **实地探查网页与编写**：使用工具实际抓取并检查目标网页结构、分页逻辑、Steam AppID 证据（禁止捏造 AppID）与下载链接域名，严禁凭猜测编造 CSS 选择器或主机名；按规范编写包含静态 `meta` 表的完整单文件 Lua 脚本。

4. **最终输出格式**：仅输出一段可直接整段复制粘贴导入 PatchHub 的完整 Lua 代码块（使用 ` ```lua ` 包裹），并在代码块外简洁列出脚本声明的 `page_hosts` 与 `download_hosts` 供用户核对，不要附带任何额外的 JSON 规则或手工表单字段。
