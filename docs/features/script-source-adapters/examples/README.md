# Lua 来源示例

[来源使用指南](../../../source-adapters.md) · [设计](../design.md) · [施工记录](../progress.md) · [API 合同](../p0-contract.md)

## 发行商列表与直接下载

- [source.json](publisher/source.json)：作者规则，`adapter=lua`、`role=publisher`。ID、名称、域名均须改为自己的来源；API 版本在编辑器填写 1，不写进作者规则。
- [adapter.lua](publisher/adapter.lua)：读取 `pages` 的每个页面，选择 `tr[data-appid]`，用 `.title` 取标题、首个 `a[href]` 取补丁地址，返回带 AppID 的 `entry` 条目。空数组使用 `ctx.array()`，非 200 响应或缺少必要字段明确失败。

对应的示例网页结构：

```html
<table>
  <tr data-appid="123">
    <td class="title">Game</td>
    <td><a href="https://downloads.vendor.example/game.zip">Patch</a></td>
  </tr>
</table>
```

将两份文件的内容分别粘贴到 Windows/Decky 的脚本导入编辑器。示例查看/复制不覆盖现有草稿，也不自动创建记录。保存成功后，需要用实际站点数据执行来源更新再匹配游戏。该样例演示直接下载，不实现站点分页或详情页；有分页/详情的站点应按 API 合同实现其实际逻辑，不能直接照搬并声称完整。

核心离线测试在同一执行器中验证本例，两端测试核对示例内容一致。测试输入是合成 HTML，没有访问这些占位域名；复杂录制原型见[测试夹具](../../../../crates/core/tests/fixtures/script-sources/kagura-en-prototype.lua)，它也不等于 P6 完整内置迁移。

## 自选产品详情页（可运行的真实网络示例）

[规则](product-pages/source.json)与[源码](product-pages/adapter.lua)是独立作者脚本：来源范围严格为规则 `pages` 明确列出的产品详情页，不声称完整覆盖该发行商目录。示例用公开 Kagura 海外产品页演示 `collect_index/resolve_page`、正文镜像与公共 Steam ID 提取；可添加其他符合相同 DOM 的产品 URL。网站变化会明确报结构错误。

P7 已通过真实 Release 核心执行导入、更新和重启读取，发布 1 条完整索引。该证据仅覆盖所配置页面，不代表匹配到某个真实 AppID、下载或安装通过。前面的表格示例仍是占位域名配合合成 HTML 的离线说明，不能冒充真实网站验证。验证详情见[施工记录](../progress.md)。
