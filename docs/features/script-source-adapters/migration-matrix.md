# 内置来源迁移行为矩阵

[设计](design.md) · [施工记录](progress.md) · [API 合同](p0-contract.md)

状态：**P6 已完成五类适配器的生产切换和旧 Rust 网站模块退役。** 旧行为基线为 `66ac372`，迁移前 40 项断言通过、1 项网络忽略；相同输入的 9 组新旧结果/请求对照已冻结为[固定期望](../../../crates/core/tests/fixtures/builtin-migration/README.md)。最终回归调用生产分派，不再编译旧解析器。

| 来源 | 保持的执行路径 | 当前资源 / 验证入口 | P6 证据与边界 |
| --- | --- | --- | --- |
| `patchmoe` / Shiravune | 首页条件 GET、304；三语言表单 POST、lang 赋值、Cookie/最终 URL；多表格、COMING SOON；仅同归档合并双语别名，优先简中；ZIP 域名/格式 | [patchmoe.lua](../../../crates/core/resources/source-scripts/patchmoe.lua)、[迁移回归](../../../crates/core/tests/builtin_script_regression.rs)、[来源更新](../../../crates/core/tests/source_update.rs) | 离线基线、条件/失败/取消断言；真实首页完成 165 条完整索引，无诊断 |
| `kagura-cn` | article/分类、多入口/分页、重复页；分类内文章事实/链接；详情 Steam ID、正文、镜像、日期；公共本体/DLC/上游关联与共享缓存 | [kagura.lua](../../../crates/core/resources/source-scripts/kagura.lua)、[匹配](../../../crates/core/tests/source_matching.rs)、[验收](../../../crates/core/tests/source_acceptance.rs) | 同输入索引/详情/分类对照，离线分页/DLC/更新/匹配；真实首页选取一个详情返回 4 镜像。不是全站实时目录验收 |
| `kagura-en` | patch/产品分类多根列表；product 卡片排除 related/widget；next 分页；标题、tab-patch、Steam ID（有时）、正文/镜像及变体 | [kagura.lua](../../../crates/core/resources/source-scripts/kagura.lua)、[录制夹具](../../../crates/core/tests/fixtures/script-sources/README.md)、[匹配](../../../crates/core/tests/source_matching.rs) | 录制页面固定期望、合成多根/分页与变体；真实详情返回 4 镜像。未抓取全站目录；该真实页没有 Steam ID |
| `hgamer` | 核心 query terms；真实结果数、games/patches/posts 分组；game/patch body、同列 Steam widget；别名、ZIP/EXE、镜像和 upstream；完整空/部分/失败；多补查聚合 | [hgamer.lua](../../../crates/core/resources/source-scripts/hgamer.lua)、[补查](../../../crates/core/tests/script_source_fallback.rs)、[网络](../../../crates/core/tests/hgamer_live.rs) | 两组同输入候选/请求对照与错误回归；真实 App1250180 完整空候选/1身份，App1227890 本体+DLC 两候选/1身份，无诊断 |
| `table_html` / `table` | 原 columns/table_id/language 配置、相对 ZIP、可选日期/语言/appid、完整/部分/304；旧 JSON 示例保持 | [table.lua](../../../crates/core/resources/source-scripts/table.lua)、[common.lua](../../../crates/core/resources/source-scripts/common.lua)、[迁移回归](../../../crates/core/tests/builtin_script_regression.rs) | 生产分派同输入对照及更新/安装回归；补验第 22 列与字面 Unicode/冒号 ID，不设旧 20 列/64 字节 ASCII 限制。通用配置没有单一真实网站可验收 |

## 资源与修订

完整可读取源码为 `common.lua + 站点代码`，Kagura 另附模式选择行。由 [builtin.rs](../../../crates/core/src/sources/builtin.rs) 编译内嵌；以下 SHA-256 对应完整原始 UTF-8/LF 文本，公共文本变化会更新每个受影响来源的解析身份。

| 来源 | SHA-256 |
| --- | --- |
| `patchmoe` | `22a99f67a1c0893eed24a892cf4e1872cf33094852619c279430a6aea1d7d262` |
| `kagura-cn` | `a56a58db73530a6109df921336d39450746f9f60c03223251ecaa0a3d28e9994` |
| `kagura-en` | `aca5d2b41d10e99307092e25606db6dca2072b012284e99bd9658e027a7b64b3` |
| `hgamer` | `be0602dec767bd5bc23721741b4c665c14b29c0b6542a3c7811695a73e1b5ac2` |
| `table_html` | `fd673383c22f38fcc7b6b3e6bfb002410d89d859da13a7b85be8d5b60a77b0e8` |

## 退役与保留

`adapters/patchmoe.rs`、`kagura.rs`、`hgamer.rs`、`table.rs` 及只为旧站点实现服务的 `dom.rs` 已删除。`adapters/mod.rs` 只有源码定位与统一 Lua 调用；身份、格式、候选、HTTP、URL、DOM、缓存及安装仍由公共核心处理。没有 Lua 失败后的 Rust 网站兜底，没有站点专用宿主解析回调。旧对照测试改为固定 JSON，未保留测试版旧引擎。

四个 ID、名称覆盖、启用、订阅和间隔保持。内置脚本无需导入，可读取并显式保存为同 ID 用户覆盖；用户 `adapter=lua` 不被后续默认规则覆盖。旧无指纹索引及历史文件保留，但不授权迁移后解析器或旧选择，需更新/重新匹配。详见[覆盖与旧缓存回归](../../../crates/core/tests/builtin_scripts.rs)。

P7 仍负责发行包/许可证与干净环境资源核验、全量实时目录、自定义样例网络链路及约定范围设备验收。本阶段网络仅只读元数据，未下载补丁或操作真实游戏目录。
