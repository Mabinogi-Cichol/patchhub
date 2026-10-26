# 分平台 BAT 构建入口

施工记录：[progress.md](progress.md)。

## 目标与范围

保留现有根目录 `build.bat`，新增可双击的 Windows 与 Decky 独立构建入口。本次按用户要求直接在当前分支修改。

## 方案

- [build-windows.bat](../../../build-windows.bat) 调用现有 [Flutter 构建脚本](../../../scripts/build-windows-flutter.ps1) 的 `-Preview` 模式，增量构建 Rust 核心与 Flutter Release 应用、复制运行必需的资源后自动启动。跳过 analyze/test、依赖清单采集、更新清单、安装包、ZIP、SBOM 与发行校验。完整发行继续使用 [build.bat](../../../build.bat)。保留 Rust 增量构建以确保核心改动生效，不默认复用旧二进制。
- [build-decky.bat](../../../build-decky.bat) 在 Windows 执行 npm 安装、类型检查、前端打包和 Python 发行打包；在默认 WSL 发行版通过现有 [build-linux.sh](../../../scripts/build-linux.sh) 测试、构建 Linux 核心与 worker，并导出锁定的 Cargo metadata。
- WSL 使用现有 `.tools/linux-cargo`、`.tools/linux-rustup` 工具链与 `target/linux` 输出目录；需预先安装 WSL 发行版、Linux Rust 工具链及原生编译依赖。Windows 需 Node.js/npm、Python 3.11+；Windows 入口依赖沿用原脚本。
- Decky 复用现有插件打包、SBOM 与归档校验流程，产物为 `.artifacts/patchhub-decky.zip`、`checksums-decky.json`、`dependencies.cdx.json` 和 `install-decky.py`。既有打包器也生成 `SHA256SUMS.txt`。
- 两个入口默认结束时暂停，支持 `--no-pause`、`--help`，错误退出码向调用方传播。不添加重试、超时或设备部署。
- Windows 预览输出至 `apps/windows-flutter/build/windows/x64/runner/Release/`；两个平台的完整发行沿用 `.artifacts` 公共产物目录，按顺序构建；SBOM、metadata 和总校验表由最近一次发行构建生成。

## 验收

检查入口帮助、未知参数退出码、Windows 预览构建与必需运行文件，以及 Decky 调用顺序、失败停止和路径处理。完整发行构建需单独记录实际结果，未执行不得视为通过。
