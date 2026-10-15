# 插件品牌素材

正式采用的 LOGO 与横幅，使用内置 imagegen 工具生成。采用黄油色系与扁平图形，不含爱心元素。横幅已用于[项目首页](../../../README.md)，客户端图标尚未替换。

以成人游戏的中文俗称“黄油”为视觉线索，采用奶油黄、深可可色与柔软的边角轮廓；拼图缺口表达“补丁”，对勾表达“安装完成”。按反馈去掉爱心、高光、厚边和立体阴影，降低拟物感。

配色参考：黄油黄 `#FFD66B`、奶油白 `#FFF1BF`、深可可 `#39242B`。生成图片的实际像素颜色可能有轻微变化。

## 正式素材

| 文件 | 尺寸 | 用途 |
| --- | --- | --- |
| [logo.png](logo.png) | 1254 × 1254 | 透明背景 LOGO 原图 |
| [banner.png](banner.png) | 2172 × 724（3:1） | README、项目介绍横幅 |

![正式横幅](banner.png)

<img src="logo.png" alt="正式 LOGO" width="256">

## 检查记录

- 已目视核对正式素材的标志、配色和横幅文字，没有爱心元素。
- 已读取图片尺寸；LOGO 外部角点透明，横幅背景不透明。
- 尚未验证缩小到 Decky 菜单图标尺寸的显示效果；客户端 ICO 与各尺寸图标未导出。

## 实际生成提示词

请求尺寸是生成意图，最终尺寸以文件表为准。

### LOGO

以此前拟物风格的黄油色 LOGO 草稿为编辑目标；草稿已移除，仅保留正式素材。

```text
Use case: style-transfer
Asset type: revised finished logo for Steam 补丁管理.
Input image 1 is the EDIT TARGET. Keep the recognisable rounded-square tile, left circular puzzle notch, large central checkmark and just a gentle softened uneven butter corner. CHANGE THE RENDERING STYLE to crisp FLAT graphic design.
Specific user request: current skeuomorphism is too strong. Remove ALL glossy highlights, cream bevel rings, dark extruded sides, glow, reflections, shadows, shiny material, lighting gradients, 3D depth and food texture.
Render the silhouette in ONE SOLID butter-yellow #FFD66B fill. Render the checkmark and the interior of the left notch in ONE SOLID deep cocoa #39242B fill. Clean sharp antialiased contours. Prefer solid fills only; no outline strokes, no gradient, no double border. Same soft friendly proportions with generous negative space; the bottom right may have a very subtle butter-soft bulge but no droplets.
This is a minimal flat app brand mark, like carefully drawn vector artwork, not a 3D food tile. Single centered logo filling roughly 72 percent of a square canvas with equal transparent margins. Deliver actual transparent PNG.
No heart, no love symbols, no faces, no text, no extra objects, no presentation board, no frame, no watermark. Preserve the warm butter reference through COLOR and SHAPE only.
```

### 横幅

以正式的 `logo.png` 为品牌参考图。

```text
Use case: logo-brand
Asset type: polished FLAT wide project banner for Steam 补丁管理, a patch management utility for Windows and Steam Deck.
Input image 1: use this yellow puzzle tile and dark cocoa checkmark as the exact brand identity reference. Match the rounded tile, left puzzle notch, checkmark silhouette and subtly softened bottom right corner. This is a new banner, not an edit to the logo file.
Style direction from the user: butter-inspired yellow identity, NO HEART ELEMENTS, substantially LESS SKEUOMORPHISM. Use crisp flat editorial graphic design, solid color fields, restrained warm character. Absolutely no 3D depth, highlights, bevels, glowing outlines, extruded letters, shadows, gloss, realistic food texture or lighting effects.
Canvas: opaque horizontal 3:1, intended 2400x800. Background a uniform solid dark cocoa #39242B. A few optional enormous almost-invisible flat puzzle contours at the extreme edges in a slightly lighter cocoa, no glow. Keep generous negative space.
Composition: flat buttery yellow #FFD66B logo on the left occupying about one quarter of canvas width. On its right, align three lines of carefully set clean modern sans-serif typography. Large bold warm cream title, a smaller butter-yellow action line, and a much smaller muted cream platform line. Comfortable margins. Title must fit on one line. Typography flat, neither inflated nor melted, with elegant spacing.
Text EXACTLY:
"Steam 补丁管理"
"查找 · 下载 · 安装"
"Windows · Steam Deck"
Constraints: all typography is flat, no fake dimensionality. No hearts, no faces, no characters, no food props, no Steam official symbol, no additional text, no badges, no watermark, no mockup presentation. Deliver one finished warm minimal banner that clearly belongs with the referenced flat logo.
```
