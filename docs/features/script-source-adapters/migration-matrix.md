# 内置来源迁移行为矩阵

[设计](design.md) · [施工记录](progress.md) · [P0 合同](p0-contract.md)

状态：**P0 建立行为清单，P6 生产迁移全部待实施。** 基线 `cb06188`。表中 Rust 路径是迁移前的依据，不能在最终产品中作为脚本失败兜底保留。

| 来源 | 必须保持的真实执行路径 | 既有实现 / 验证入口 | P0 / P6 状态 |
| --- | --- | --- | --- |
| `patchmoe` / Shiravune | 首页条件请求；`form_en/form_zh_cn/form_zh_tw` 的 POST 与 lang 赋值；Cookie/最终 URL；多表格的 title/release/patch 列；COMING SOON；相同归档 URL 才合并双语别名，优先简中；ZIP 域名/格式；304 无缓存明确失败 | [patchmoe.rs](../../../crates/core/src/sources/adapters/patchmoe.rs)、[source_update.rs](../../../crates/core/tests/source_update.rs)、[patchmoe-index.html](../../../crates/core/tests/fixtures/sources/patchmoe-index.html) | P0 验证公共表单/Cookie/重定向能力；站点完整迁移及真实网络待 P6 |
| `kagura-cn` | WordPress article/分类目录；分页/重复页；分类内完整文章和后续文章；详情 Steam ID、正文、镜像与发布日期；共享详情缓存；已有本体/DLC/上游身份关联 | [kagura.rs](../../../crates/core/src/sources/adapters/kagura.rs)、[source_matching.rs](../../../crates/core/tests/source_matching.rs)、[source_update.rs](../../../crates/core/tests/source_update.rs)、[source_acceptance.rs](../../../crates/core/tests/source_acceptance.rs) | P6 全部待迁移；公共身份算法留在 Rust，不能把 CN HTML 解析藏入专用宿主回调 |
| `kagura-en` | `/patch/` 与产品分类多入口；`.product.type-product` 卡片、排除 related/widget；`a.next/rel=next` 分页；标题、`#tab-patch`、Steam ID（存在时）、正文/镜像、格式与变体 | [kagura.rs](../../../crates/core/src/sources/adapters/kagura.rs)、[script-sources](../../../crates/core/tests/fixtures/script-sources/README.md)、[source_matching.rs](../../../crates/core/tests/source_matching.rs) | P0 真实列表/详情夹具与 Rust 相等，分页确实请求下一页；全站生产迁移、变体与真实完整链路仍待 P6 |
| `hgamer` | 公共 query_terms 驱动搜索；按实际结果数判断结构完整性；games/patches/posts 分组；`data-game-body/data-patch-body`；Steam widget/同列链接；别名事实、ZIP/EXE 声明、镜像及 upstream_page；完整无结果与部分/失败；多补查源聚合 | [hgamer.rs](../../../crates/core/src/sources/adapters/hgamer.rs)、[adapters/mod.rs](../../../crates/core/src/sources/adapters/mod.rs)、[matching.rs](../../../crates/core/src/sources/matching.rs)、[source_matching.rs](../../../crates/core/tests/source_matching.rs) | P4/P6 待实施；脚本输出事实，候选身份/可信度仍由 Rust 生成 |
| `table_html` | 保持既有规则 `columns/table_id/language`；相对 ZIP 链接；可选日期/语言/appid；完整/部分/304；旧 JSON 示例可继续使用 | [table.rs](../../../crates/core/src/sources/adapters/table.rs)、[adapters/mod.rs](../../../crates/core/src/sources/adapters/mod.rs)、[source_update.rs](../../../crates/core/tests/source_update.rs) | P6 待迁移为随包通用 Lua 脚本；不能保留独立 Rust 表格解析生产路径 |

## 每源退役记录要求

P6 每切换一个来源，在施工记录追加：脚本资源与摘要、生产分派入口、离线夹具/真实网络结果、删除的 Rust 模块/分支、仍开放的验收项。需要保留的旧行为断言迁到脚本生产链路；已有身份、安装和网络公共能力继续保留。

最终共同检查：四个来源 ID、名称覆盖、启用、订阅、间隔及历史文件保留；无需用户额外导入内置脚本；同 ID 显式用户覆盖不被发行更新替换；更新/匹配/只读/ZIP/EXE/手动选择均检查当前解析身份；脚本异常不进入旧 Rust 引擎。

生产迁移前，P0 的测试用 Rust 对照只能作为迁移证据。P6 删除旧模块后，将对照改为固定期望与生产分派断言，不能为维持对照测试继续把旧网站解析器打包。
