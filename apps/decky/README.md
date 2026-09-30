# Decky P0 桥接

当前只有 Python → Rust CLI 的只读 JSON 通信模块，尚不是可安装的 Decky 插件。没有 plugin.json、游戏模式 UI 或实际 Steam Deck 验证。

`backend/bridge.py` 由宿主配置核心二进制与目录路径；UI 只能调用 scan 和带 ID 的 preview，不能传入 shell、游戏文件路径或下载 URL。核心进程运行 `serve`，请求带协议版本与关联 ID，并限制大小和超时。卸载插件时的任务生命周期将在 Decky 主后端中接入。

先在仓库根目录执行 `cargo build --workspace`，再运行：

```text
python -m unittest discover -s apps/decky/backend -p test_bridge.py -v
```

桥接集成测试调用真实编译核心，scan 会只读扫描本机 Steam 元数据，测试输出不列出游戏名称或路径。Windows 上通过此测试只证明通信，不证明 SteamOS 平台兼容。
