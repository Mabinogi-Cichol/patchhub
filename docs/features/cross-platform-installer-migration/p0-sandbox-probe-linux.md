# Linux 隔离原型实测证据（P0-D bwrap）

日期：2026-10-08（Asia/Shanghai）。状态：**原型已实现并在本机 WSL2 Ubuntu 与真实 SteamOS Deck 实测通过；含 M3 输出限额 fail-closed 修订与 M1 root 宿主降权补测。非 root 回归 23/24 通过（1 信息项）、root 降权场景 26/27 通过（1 信息项），两场景均 0 失败；SteamOS Deck 对照（M1/M3 前版本，22 项 21 pass/0 fail/1 info）见 §10。**

> **范围声明：本文件包含 WSL2 对照与 SteamOS / Steam Deck 实机对照证据（Deck 节见 §10，为 M1/M3 修订前版本；M1/M3 新检查为 WSL2-only，Deck 复跑列 P3）。** WSL2 与 SteamOS 的内核、bwrap 版本、用户命名空间/cgroup 策略不同，双端结论以各自实测为准。本文件只记录 P0-D 原型与实测原始证据，不代表 P0 整体通过或生产接入。

关联：[design.md](design.md) 第 4.3 节「双平台隔离门槛」；[p0-contracts.md](p0-contracts.md) §6.1/§6.2；[p0-sandbox-probe-windows.md](p0-sandbox-probe-windows.md)（Windows LPAC 对照）；[progress.md](progress.md)。

## 1. 目标与范围

为 `crates/installer-worker` 增加 Linux 平台实现（`bwrap` 启动器 + 探针），把现行 Python 的固定路径沙箱（`apps/decky/backend/exe_worker.py:15-30`、`scripts/sf95_worker.py:128-155`）通用化到任意 worker 二进制，并核实以下清单项：bwrap 可用性、worker 非 root、只读输入写拒绝、输出独占可写 + 宿主回读、越界读/写拒绝（宿主 home/文档/state 不可见）、外网 + 回环连接失败、环境清洁（clearenv 后仅白名单）、超时/父退出进程组回收（无残留）、AS 限额生效（小值验证）、中文/长路径。

本文件在初审（条件通过）后的修订范围内补充：

- **M3（输出限额 fail-closed）**：stdout/stderr 超限不再静默截断，而是返回 `exe_worker_response_limit` 并终止沙箱（对齐 Python `_bounded`/`exe_worker_response_limit`）；新增 stdout/stderr 两个超限探针。
- **M1（root 宿主降权最小实现 + 补测）**：启动器在 `euid==0` 时于 `pre_exec` 中 `setgid→setuid` 降权；新增 `--run-as` 目标用户、root 场景探针（worker 非 root、`/root` 与 `/etc/shadow` 不可读）与非 root 回归。
- **P1 评审 R 项登记**（见 §8）。

边界（含 M1/M3 修订轮）：代码改动限于 `crates/installer-worker/**` 与本文件；未改动 `crates/core`、`crates/cli`、`apps/**`、`Cargo.toml`、`Cargo.lock`。修订轮同步了 `design.md §4.3`、`p0-contracts.md §10` 与 `progress.md` 的登记/记录；Deck 实机见 §10（为修订前版本）。未提交 git。

- 探针使用**合成的临时目录与假文件**模拟宿主 home/文档/state/私钥，未触碰真实用户数据。root 场景只读取 `/root` 与 `/etc/shadow` 的「不可见」断言（未绑定即 ENOENT），未修改任何真实 root 文件。
- Linux 代码以 `cfg(target_os = "linux")` 隔离；非 Linux 仍走 stub，保证工作区可编译。
- **未新增任何外部依赖**（`libc` 绑定改为 `extern "C"`，避免改动 `Cargo.lock`）。

## 2. 环境与复现命令

| 项目 | 值 |
| --- | --- |
| 发行版 | Ubuntu 22.04.3 LTS (Jammy)，运行于 WSL2 |
| 内核 | `Linux LEGION-REN7000P 6.18.40.1-microsoft-standard-WSL2 #1 SMP PREEMPT_DYNAMIC Fri Jul 31 22:12:15 UTC 2026 x86_64` |
| 架构 | x86_64 |
| bubblewrap | `0.6.1-1ubuntu0.3`（`/usr/bin/bwrap`） |
| Rust | rustc 1.94.0 / cargo 1.94.0，host `x86_64-unknown-linux-gnu`（仓库隔离工具链 `.tools/linux-cargo`） |
| 正常用户 | `uid=1000(wqj7894)`（非 root） |
| root 场景目标用户 | `uid=1000:gid=1000`（WSL 普通用户，由 `--run-as 1000:1000` 显式传入） |
| 用户命名空间 | 启用（内核 6.18 无 `kernel.unprivileged_userns_clone` 限制项，`--unshare-all` 直接成功） |
| merged-`/usr` | 是：`/lib -> usr/lib`、`/lib64 -> usr/lib64`（`readlink -f` 确认） |

复现命令（WSL2 bash，仓库根目录）——非 root 回归：

```bash
export CARGO_HOME="$PWD/.tools/linux-cargo"
export RUSTUP_HOME="$PWD/.tools/linux-rustup"
export CARGO_TARGET_DIR="$PWD/target/linux"
export PATH="$PWD/.tools/linux-cargo/bin:$PATH"

cargo build -p steam-patch-installer-worker --offline   # 构建
cargo test  -p steam-patch-installer-worker --offline   # 4 个跨平台单测通过

./target/linux/debug/probe-runner --out /tmp/p0d-probe-raw.json
```

root 宿主降权补测（WSL 以 root 运行探针，显式降权到 uid/gid 1000）：

```powershell
wsl.exe -d Ubuntu -u root -- bash -lc "cd /mnt/e/ProjectFiles/Git/steam-hentai-patch-searcher && ./target/linux/debug/probe-runner --run-as 1000:1000 --out /tmp/p0d-probe-root.json"
```

Windows PowerShell 等价构建调用（与本机 Linux 工具链分离）：

```powershell
wsl.exe -d Ubuntu -- bash -lc "cd /mnt/e/ProjectFiles/Git/steam-hentai-patch-searcher && CARGO_HOME=`$PWD/.tools/linux-cargo RUSTUP_HOME=`$PWD/.tools/linux-rustup CARGO_TARGET_DIR=`$PWD/target/linux PATH=`$PWD/.tools/linux-cargo/bin:`$PATH cargo build -p steam-patch-installer-worker --offline"
```

`probe-runner` 把 `probe-worker` 复制到本次临时根目录下的 `worker/`，通过 `--ro-bind <worker_dir> /tool` 把 worker 只读暴露为 `/tool/probe-worker`，再以沙箱路径执行。

## 3. 实现摘要

`crates/installer-worker/`：

- `src/model.rs`：`SandboxLimits` 4 个 Linux-only rlimit 字段（`address_space_bytes`/`cpu_seconds`/`core_bytes`/`file_size_bytes`）；`SandboxConfig` 新增 `run_as: Option<(u32, u32)>`（Linux root 降权目标，Windows 忽略）。跨平台模型与 `run_sandboxed` API 契约保持不变。
- `src/lib.rs`：`#[cfg(target_os = "linux")] pub mod linux;`；stub 收敛为 `cfg(not(any(windows, target_os = "linux")))`；按平台重导出 `run_sandboxed`。
- `src/linux/sys.rs`：最小 `extern "C"` 绑定（`setrlimit`/`getrlimit`/`setsid`/`killpg`/`geteuid`/`setgroups`/`setgid`/`setuid` + `RLimit` 与 `RLIMIT_*` 常量）。
- `src/linux/launch.rs`：
  - `build_bwrap_command`（argv 模板）与 `run_sandboxed`。
  - **M3 fail-closed 输出限额**：`read_bounded` 返回 `Result<Vec<u8>, ()>`，观察字节数 `> limit` 即记录「哪个流」并停止读取（不再静默截断）；等待循环检测到溢出后 `terminate_group` 回收沙箱，`run_sandboxed` 返回 `stage=output-limit`、消息含 `exe_worker_response_limit` 的错误。stdout 上限 `MAX_STDOUT_BYTES = 16 MiB`（契约 §6.1），stderr 上限 `MAX_STDERR_BYTES = 64 KiB`（对齐 Python `_bounded(..., 65536)`）。阈值内的返回字节与旧实现一致。
  - **M1 root 降权**：仅当启动器 `euid==0` 时解析目标 `(uid,gid)`（优先级：`cfg.run_as` → 环境 `SUDO_UID`/`SUDO_GID` → 不降权）；`pre_exec` 顺序为 **rlimits → `setgroups(0, NULL)` → `setgid` → `setuid` → `setsid`**。rlimits 先于降权（只会下调限额，特权态执行不会因 `rlim_max` 降低而失败）。
- `src/linux/probe_os.rs`：`euid`、`bwrap_available`/`bwrap_version`、`rlimits`、`process_alive`、`find_processes`、`uname_info`。
- `src/bin/probe_worker.rs`：`linux_impl` 场景新增 `big_output`（写 `mb` MiB 到 stdout，默认 17）与 `big_stderr`（写 `mb` MiB 到 stderr），用原始 fd 写入绕过 libstd 缓冲，用于验证 fail-closed。
- `src/bin/probe_runner.rs`：`linux_impl` 新增 `--run-as <uid:gid>`、root 降权检查（`root_drop_applied`、`deny_root_home`、`deny_etc_shadow`）、输出限额检查（`output_limit_stdout`/`output_limit_stderr`，含残留检查）；root 场景下对合成目录放宽权限（输入/工具 0755、输出 0777、fixture 0644）以适配降权后的 uid。

生成的 bwrap argv（通用化模板）：

```text
/usr/bin/bwrap --unshare-all --die-with-parent --new-session --clearenv
  --setenv PATH /usr/bin --setenv HOME /tmp --cap-drop ALL
  --ro-bind /usr /usr
  [--symlink usr/lib /lib] [--symlink usr/lib64 /lib64]      # 符号链接系统目录
  [--ro-bind <dir> <dir>]                                     # 非符号链接时
  --proc /proc --dev /dev --tmpfs /tmp
  --dir /input --ro-bind <input> /input                       # 只读输入
  --ro-bind <tool_dir> /tool                                  # 只读 worker/工具
  --bind <output> /output                                     # 独占可写输出
  --chdir /tmp
  -- <worker> [args...]
```

## 4. 实测结果汇总（非 root 回归）

| 状态 | 数量 |
| --- | --- |
| pass | 23 |
| fail | 0 |
| unsupported | 0 |
| not_covered | 0 |
| info | 1（运行探针宿主 uid） |
| 合计 | 24 |

关键原值：

- bwrap：`bubblewrap 0.6.1`，`--unshare-all` 在非 root 用户命名空间下成功；worker uid=1000。
- 环境：`--clearenv` 后仅 `PATH=/usr/bin`、`HOME=/tmp`、`PWD=/tmp`，`env_count=3`，宿主 canary 未出现。
- 文件：`/input` 可读；`/input` 写 `EROFS(30)`；`/output` 写成功且宿主回读一致；宿主 home/文档/state 路径 `ENOENT(2)`；越界写 `ENOENT(2)`；中文 + 282 字符长路径往返成功。
- 网络：回环 `ECONNREFUSED(111)`；外网 `ENETUNREACH(101)`。
- 进程：1.5s 超时 `Timeout` 且无残留；fork 子进程后超时回收无残留。
- 限额：`RLIMIT_AS=256 MiB` 下 1024 MiB 分配失败、默认 3 GiB 下 64 MiB 成功；`RLIMIT_CPU=1 s` 使 10s busy 在 `wall_ms≈1005` 终止。
- **M3**：`output_limit_stdout` 返回 `[output-limit] exe_worker_response_limit: stdout exceeded 16777216 bytes`，`output_limit_stderr` 返回 `... stderr exceeded 65536 bytes`，两者 `residue=[]`。

## 5. 原始证据（非 root 回归 probe-runner stdout / raw JSON）

```json
{
  "arch": "x86_64",
  "checks": [
    {
      "actual": "path=/usr/bin/bwrap, exists=true, version=Some(\"bubblewrap 0.6.1\")",
      "expected": "/usr/bin/bwrap 存在且可执行、--version 成功",
      "id": "bwrap_available",
      "status": "pass",
      "title": "bwrap 可用性"
    },
    {
      "actual": "ok=true, exit=0, reason=Completed, stdout=193B, stderr=\"\"",
      "expected": "worker 在沙箱内成功启动并返回 JSON",
      "id": "bwrap_worker_start",
      "status": "pass",
      "title": "bwrap 沙箱内 worker 启动"
    },
    {
      "actual": "uid=1000",
      "expected": "沙箱内 geteuid() != 0",
      "id": "worker_not_root",
      "status": "pass",
      "title": "worker 非 root"
    },
    {
      "actual": "canary_present=false, env_count=3, PATH=true, HOME=true",
      "expected": "canary 不出现；仅 PATH=/usr/bin 与 HOME=/tmp",
      "id": "env_clean",
      "status": "pass",
      "title": "环境变量清洁"
    },
    {
      "actual": "PROBE_INPUT_v1_中文",
      "expected": "读取成功且内容一致",
      "id": "authorized_read_input",
      "status": "pass",
      "title": "授权输入目录可读"
    },
    {
      "actual": "ok=false, error_code=Some(30), error=Some(\"write '/input/evil.txt' failed: Read-only file system (os error 30)\"), exit=0",
      "error_code": 30,
      "expected": "EROFS / 写失败",
      "id": "deny_write_input",
      "status": "pass",
      "title": "只读输入目录写被拒"
    },
    {
      "actual": "written",
      "expected": "写入成功且宿主读回一致",
      "id": "allow_write_output",
      "status": "pass",
      "title": "授权输出目录可写且宿主可回读"
    },
    {
      "actual": "ok=false, error_code=Some(2), error=Some(\"read '/tmp/steam-patch-probe-2150-1791402432444/synthetic_home/secret.txt' failed: No such file or directory (os error 2)\"), exit=0",
      "error_code": 2,
      "expected": "ENOENT / EACCES（宿主文件不可见）",
      "id": "deny_read_home",
      "status": "pass",
      "title": "未授权宿主 home 目录读被拒/不可见"
    },
    {
      "actual": "ok=false, error_code=Some(2), error=Some(\"read '/tmp/steam-patch-probe-2150-1791402432444/synthetic_documents/secret.doc' failed: No such file or directory (os error 2)\"), exit=0",
      "error_code": 2,
      "expected": "ENOENT / EACCES（宿主文件不可见）",
      "id": "deny_read_documents",
      "status": "pass",
      "title": "未授权宿主文档目录读被拒/不可见"
    },
    {
      "actual": "ok=false, error_code=Some(2), error=Some(\"read '/tmp/steam-patch-probe-2150-1791402432444/synthetic_state/keys/private.key' failed: No such file or directory (os error 2)\"), exit=0",
      "error_code": 2,
      "expected": "ENOENT / EACCES（宿主文件不可见）",
      "id": "deny_read_state_key",
      "status": "pass",
      "title": "未授权宿主 state/私钥读被拒/不可见"
    },
    {
      "actual": "ok=false, error_code=Some(2), error=Some(\"write '/tmp/steam-patch-probe-2150-1791402432444/synthetic_home/evil.txt' failed: No such file or directory (os error 2)\"), exit=0",
      "error_code": 2,
      "expected": "ENOENT / EACCES",
      "id": "deny_write_home",
      "status": "pass",
      "title": "越界写宿主 home 目录被拒"
    },
    {
      "actual": "roundtrip_ok=true, long_path_len=282",
      "expected": "中文文件名与 >260 字符路径读写成功",
      "id": "unicode_long_path",
      "status": "pass",
      "title": "授权目录内中文/长路径读写"
    },
    {
      "actual": "connect 127.0.0.1:36667 connected=false, error_code=Some(111), exit=0, stderr=\"\"",
      "error_code": 111,
      "expected": "--unshare-net 下回环连接失败",
      "id": "deny_network_loopback",
      "status": "pass",
      "title": "回环网络连接被拒"
    },
    {
      "actual": "connect 1.1.1.1:443 connected=false, error_code=Some(101), exit=0, stderr=\"\"",
      "error_code": 101,
      "expected": "--unshare-net 下外网连接失败",
      "id": "deny_network_external",
      "status": "pass",
      "title": "外网连接被拒"
    },
    {
      "actual": "reason=Timeout, wall_ms=1527, dead=true, exit=4294967295",
      "expected": "1.5s 超时后 Timeout 终止且进程消失",
      "id": "wall_clock_timeout",
      "status": "pass",
      "title": "宿主超时终止并回收"
    },
    {
      "actual": "residue_pids=[]",
      "expected": "宿主 pgrep 不到 worker/子进程",
      "id": "timeout_residue",
      "status": "pass",
      "title": "超时后无 worker 残留进程"
    },
    {
      "actual": "child_pid=Some(3), reason=Timeout, wall_ms=2533, residue_pids=[]",
      "expected": "子进程 PID 上报；超时终止后无任何 worker/子进程残留",
      "id": "fork_and_reclaim",
      "status": "pass",
      "title": "worker fork 子进程后整体回收"
    },
    {
      "actual": "as=Some(268435456), cpu=Some(42), core=Some(0)",
      "expected": "RLIMIT_AS=256MiB / CPU=42s / CORE=0 在 worker 内可见",
      "id": "rlimit_reported",
      "status": "pass",
      "title": "子进程 rlimit 生效（机制验证）"
    },
    {
      "actual": "alloc_ok=false, error=Some(\"allocate 1024 MiB failed under RLIMIT_AS: memory allocation failed because the memory allocator returned an error\")",
      "expected": "AS=256MiB 时 1024MiB 分配失败",
      "id": "rlimit_as_enforced",
      "status": "pass",
      "title": "RLIMIT_AS 限额生效"
    },
    {
      "actual": "alloc_ok=true",
      "expected": "默认 3GiB 时 64MiB 分配成功",
      "id": "rlimit_as_control",
      "status": "pass",
      "title": "RLIMIT_AS 对照"
    },
    {
      "actual": "wall_ms=1005, reason=Completed, response=false",
      "expected": "CPU=1s 时 10s busy 提前终止（<9.5s）",
      "id": "rlimit_cpu",
      "status": "pass",
      "title": "RLIMIT_CPU 限额生效"
    },
    {
      "actual": "err=[output-limit] exe_worker_response_limit: stdout exceeded 16777216 bytes (pid=2278, exit=0, sandbox terminated); limit_error=true, stream_match=true, residue=[]",
      "expected": "返回 exe_worker_response_limit 且沙箱被回收（不静默截断）",
      "id": "output_limit_stdout",
      "status": "pass",
      "title": "stdout 超限 fail-closed"
    },
    {
      "actual": "err=[output-limit] exe_worker_response_limit: stderr exceeded 65536 bytes (pid=2284, exit=0, sandbox terminated); limit_error=true, stream_match=true, residue=[]",
      "expected": "返回 exe_worker_response_limit 且沙箱被回收（不静默截断）",
      "id": "output_limit_stderr",
      "status": "pass",
      "title": "stderr 超限 fail-closed"
    },
    {
      "actual": "runner_uid=1000, run_as=None",
      "expected": "记录实际 euid；root 宿主应通过 run_as 降权",
      "id": "runner_uid",
      "status": "info",
      "title": "运行探针的宿主用户（信息项）"
    }
  ],
  "generated_unix_secs": 1791402438,
  "host": {
    "bwrap_version": "bubblewrap 0.6.1",
    "runner_exe": "/mnt/e/ProjectFiles/Git/steam-hentai-patch-searcher/target/linux/debug/probe-runner",
    "runner_uid": 1000
  },
  "os": {
    "machine": "x86_64",
    "release": "6.18.40.1-microsoft-standard-WSL2",
    "sysname": "Linux",
    "version": "#1 SMP PREEMPT_DYNAMIC Fri Jul 31 22:12:15 UTC 2026"
  },
  "sandbox": {
    "bwrap_path": "/usr/bin/bwrap",
    "root": "/tmp/steam-patch-probe-2150-1791402432444",
    "run_as": null,
    "worker_dir": "/tmp/steam-patch-probe-2150-1791402432444/worker",
    "worker_exe": "/tmp/steam-patch-probe-2150-1791402432444/worker/probe-worker"
  },
  "schema": "steam-patch/p0-sandbox-probe/v1",
  "summary": {
    "failed": 0,
    "not_covered": 0,
    "passed": 23,
    "total": 24,
    "unsupported": 0
  }
}
```

## 6. root 宿主降权补测（M1）

### 6.1 环境与命令

在 WSL2 中以 **root** 运行探针（模拟 Decky Loader 的 root 宿主），显式把目标用户设为 WSL 普通用户 `1000:1000`：

```powershell
wsl.exe -d Ubuntu -u root -- bash -lc "cd /mnt/e/ProjectFiles/Git/steam-hentai-patch-searcher && ./target/linux/debug/probe-runner --run-as 1000:1000 --out /tmp/p0d-probe-root.json"
```

启动器逻辑：`euid==0` 且 `run_as=Some((1000,1000))` → `pre_exec` 中 `setgroups(0,NULL) → setgid(1000) → setuid(1000)`，再 `exec bwrap`，因此 bwrap 的 `--unshare-user` 映射到 uid 1000，沙箱内 worker 为 uid 1000。合成目录在 root 下创建后按目标 uid 放宽：输入/工具目录 0755、fixture 0644、输出目录 0777、worker 可执行 0755。

### 6.2 逐项结果

| 状态 | 数量 |
| --- | --- |
| pass | 26 |
| fail | 0 |
| unsupported | 0 |
| not_covered | 0 |
| info | 1（root 宿主 euid） |
| 合计 | 27 |

新增/关键项：

- `root_drop_applied`：`worker_uid=1000, target=1000:1000` → pass。
- `worker_not_root`：`uid=1000` → pass。
- `deny_root_home`：读 `/root` → `ENOENT(2)`（未绑定即不可见）→ pass。
- `deny_etc_shadow`：读 `/etc/shadow` → `ENOENT(2)` → pass。
- 其余 23 项（bwrap、环境、文件、网络、进程、rlimit、输出限额）与 §4 非 root 回归一致，全部 pass。
- `runner_uid` info：`runner_uid=0, run_as=Some((1000, 1000))`。

### 6.3 负向对照（证明降权确实生效）

以 root 运行但**不传** `--run-as`（`run_as=None`，无 `SUDO_UID/SUDO_GID` 时启动器不降权），沙箱内 worker 为 `uid=0`，`worker_not_root` 失败：

- 命令：`wsl.exe -d Ubuntu -u root -- ./target/linux/debug/probe-runner --out /tmp/p0d-probe-root-nodrop.json`
- 结果：`worker_not_root | fail | uid=0`；summary `{"failed":1,"passed":22,"total":24}`。

即未降权时 worker 为 root，降权场景下 worker=1000，两者对照证明 `pre_exec` 降权路径有效（负向对照的 1 项失败是预期，属对照而非回归）。

### 6.4 原始证据（root 降权 probe-runner stdout / raw JSON）

```json
{
  "arch": "x86_64",
  "checks": [
    {
      "actual": "path=/usr/bin/bwrap, exists=true, version=Some(\"bubblewrap 0.6.1\")",
      "expected": "/usr/bin/bwrap 存在且可执行、--version 成功",
      "id": "bwrap_available",
      "status": "pass",
      "title": "bwrap 可用性"
    },
    {
      "actual": "ok=true, exit=0, reason=Completed, stdout=193B, stderr=\"\"",
      "expected": "worker 在沙箱内成功启动并返回 JSON",
      "id": "bwrap_worker_start",
      "status": "pass",
      "title": "bwrap 沙箱内 worker 启动"
    },
    {
      "actual": "uid=1000",
      "expected": "沙箱内 geteuid() != 0",
      "id": "worker_not_root",
      "status": "pass",
      "title": "worker 非 root"
    },
    {
      "actual": "worker_uid=1000, target=1000:1000",
      "expected": "启动器 euid==0 时降权到 1000:1000",
      "id": "root_drop_applied",
      "status": "pass",
      "title": "root 宿主降权到目标用户"
    },
    {
      "actual": "canary_present=false, env_count=3, PATH=true, HOME=true",
      "expected": "canary 不出现；仅 PATH=/usr/bin 与 HOME=/tmp",
      "id": "env_clean",
      "status": "pass",
      "title": "环境变量清洁"
    },
    {
      "actual": "PROBE_INPUT_v1_中文",
      "expected": "读取成功且内容一致",
      "id": "authorized_read_input",
      "status": "pass",
      "title": "授权输入目录可读"
    },
    {
      "actual": "ok=false, error_code=Some(30), error=Some(\"write '/input/evil.txt' failed: Read-only file system (os error 30)\"), exit=0",
      "error_code": 30,
      "expected": "EROFS / 写失败",
      "id": "deny_write_input",
      "status": "pass",
      "title": "只读输入目录写被拒"
    },
    {
      "actual": "written",
      "expected": "写入成功且宿主读回一致",
      "id": "allow_write_output",
      "status": "pass",
      "title": "授权输出目录可写且宿主可回读"
    },
    {
      "actual": "ok=false, error_code=Some(2), error=Some(\"read '/tmp/steam-patch-probe-2312-1791402440570/synthetic_home/secret.txt' failed: No such file or directory (os error 2)\"), exit=0",
      "error_code": 2,
      "expected": "ENOENT / EACCES（宿主文件不可见）",
      "id": "deny_read_home",
      "status": "pass",
      "title": "未授权宿主 home 目录读被拒/不可见"
    },
    {
      "actual": "ok=false, error_code=Some(2), error=Some(\"read '/tmp/steam-patch-probe-2312-1791402440570/synthetic_documents/secret.doc' failed: No such file or directory (os error 2)\"), exit=0",
      "error_code": 2,
      "expected": "ENOENT / EACCES（宿主文件不可见）",
      "id": "deny_read_documents",
      "status": "pass",
      "title": "未授权宿主文档目录读被拒/不可见"
    },
    {
      "actual": "ok=false, error_code=Some(2), error=Some(\"read '/tmp/steam-patch-probe-2312-1791402440570/synthetic_state/keys/private.key' failed: No such file or directory (os error 2)\"), exit=0",
      "error_code": 2,
      "expected": "ENOENT / EACCES（宿主文件不可见）",
      "id": "deny_read_state_key",
      "status": "pass",
      "title": "未授权宿主 state/私钥读被拒/不可见"
    },
    {
      "actual": "ok=false, error_code=Some(2), error=Some(\"write '/tmp/steam-patch-probe-2312-1791402440570/synthetic_home/evil.txt' failed: No such file or directory (os error 2)\"), exit=0",
      "error_code": 2,
      "expected": "ENOENT / EACCES",
      "id": "deny_write_home",
      "status": "pass",
      "title": "越界写宿主 home 目录被拒"
    },
    {
      "actual": "roundtrip_ok=true, long_path_len=282",
      "expected": "中文文件名与 >260 字符路径读写成功",
      "id": "unicode_long_path",
      "status": "pass",
      "title": "授权目录内中文/长路径读写"
    },
    {
      "actual": "connect 127.0.0.1:43141 connected=false, error_code=Some(111), exit=0, stderr=\"\"",
      "error_code": 111,
      "expected": "--unshare-net 下回环连接失败",
      "id": "deny_network_loopback",
      "status": "pass",
      "title": "回环网络连接被拒"
    },
    {
      "actual": "connect 1.1.1.1:443 connected=false, error_code=Some(101), exit=0, stderr=\"\"",
      "error_code": 101,
      "expected": "--unshare-net 下外网连接失败",
      "id": "deny_network_external",
      "status": "pass",
      "title": "外网连接被拒"
    },
    {
      "actual": "reason=Timeout, wall_ms=1527, dead=true, exit=4294967295",
      "expected": "1.5s 超时后 Timeout 终止且进程消失",
      "id": "wall_clock_timeout",
      "status": "pass",
      "title": "宿主超时终止并回收"
    },
    {
      "actual": "residue_pids=[]",
      "expected": "宿主 pgrep 不到 worker/子进程",
      "id": "timeout_residue",
      "status": "pass",
      "title": "超时后无 worker 残留进程"
    },
    {
      "actual": "child_pid=Some(3), reason=Timeout, wall_ms=2532, residue_pids=[]",
      "expected": "子进程 PID 上报；超时终止后无任何 worker/子进程残留",
      "id": "fork_and_reclaim",
      "status": "pass",
      "title": "worker fork 子进程后整体回收"
    },
    {
      "actual": "as=Some(268435456), cpu=Some(42), core=Some(0)",
      "expected": "RLIMIT_AS=256MiB / CPU=42s / CORE=0 在 worker 内可见",
      "id": "rlimit_reported",
      "status": "pass",
      "title": "子进程 rlimit 生效（机制验证）"
    },
    {
      "actual": "alloc_ok=false, error=Some(\"allocate 1024 MiB failed under RLIMIT_AS: memory allocation failed because the memory allocator returned an error\")",
      "expected": "AS=256MiB 时 1024MiB 分配失败",
      "id": "rlimit_as_enforced",
      "status": "pass",
      "title": "RLIMIT_AS 限额生效"
    },
    {
      "actual": "alloc_ok=true",
      "expected": "默认 3GiB 时 64MiB 分配成功",
      "id": "rlimit_as_control",
      "status": "pass",
      "title": "RLIMIT_AS 对照"
    },
    {
      "actual": "wall_ms=1004, reason=Completed, response=false",
      "expected": "CPU=1s 时 10s busy 提前终止（<9.5s）",
      "id": "rlimit_cpu",
      "status": "pass",
      "title": "RLIMIT_CPU 限额生效"
    },
    {
      "actual": "err=[output-limit] exe_worker_response_limit: stdout exceeded 16777216 bytes (pid=2440, exit=0, sandbox terminated); limit_error=true, stream_match=true, residue=[]",
      "expected": "返回 exe_worker_response_limit 且沙箱被回收（不静默截断）",
      "id": "output_limit_stdout",
      "status": "pass",
      "title": "stdout 超限 fail-closed"
    },
    {
      "actual": "err=[output-limit] exe_worker_response_limit: stderr exceeded 65536 bytes (pid=2446, exit=0, sandbox terminated); limit_error=true, stream_match=true, residue=[]",
      "expected": "返回 exe_worker_response_limit 且沙箱被回收（不静默截断）",
      "id": "output_limit_stderr",
      "status": "pass",
      "title": "stderr 超限 fail-closed"
    },
    {
      "actual": "ok=false, error_code=Some(2), error=Some(\"read '/root' failed: No such file or directory (os error 2)\"), exit=0",
      "error_code": 2,
      "expected": "ENOENT/EACCES（root 文件不可见）",
      "id": "deny_root_home",
      "status": "pass",
      "title": "root 降权后 /root 不可读"
    },
    {
      "actual": "ok=false, error_code=Some(2), error=Some(\"read '/etc/shadow' failed: No such file or directory (os error 2)\"), exit=0",
      "error_code": 2,
      "expected": "ENOENT/EACCES（root 文件不可见）",
      "id": "deny_etc_shadow",
      "status": "pass",
      "title": "root 降权后 /etc/shadow 不可读"
    },
    {
      "actual": "runner_uid=0, run_as=Some((1000, 1000))",
      "expected": "记录实际 euid；root 宿主应通过 run_as 降权",
      "id": "runner_uid",
      "status": "info",
      "title": "运行探针的宿主用户（信息项）"
    }
  ],
  "generated_unix_secs": 1791402446,
  "host": {
    "bwrap_version": "bubblewrap 0.6.1",
    "runner_exe": "/mnt/e/ProjectFiles/Git/steam-hentai-patch-searcher/target/linux/debug/probe-runner",
    "runner_uid": 0
  },
  "os": {
    "machine": "x86_64",
    "release": "6.18.40.1-microsoft-standard-WSL2",
    "sysname": "Linux",
    "version": "#1 SMP PREEMPT_DYNAMIC Fri Jul 31 22:12:15 UTC 2026"
  },
  "sandbox": {
    "bwrap_path": "/usr/bin/bwrap",
    "root": "/tmp/steam-patch-probe-2312-1791402440570",
    "run_as": "1000:1000",
    "worker_dir": "/tmp/steam-patch-probe-2312-1791402440570/worker",
    "worker_exe": "/tmp/steam-patch-probe-2312-1791402440570/worker/probe-worker"
  },
  "schema": "steam-patch/p0-sandbox-probe/v1",
  "summary": {
    "failed": 0,
    "not_covered": 0,
    "passed": 26,
    "total": 27,
    "unsupported": 0
  }
}
```

## 7. 发现、偏差与踩坑记录

1. **bwrap 可用且用户命名空间启用**：WSL2 Ubuntu 22.04（内核 6.18）默认允许非 root 用户命名空间，`--unshare-all`（含 user/pid/net/ipc/uts/cgroup）成功。**SteamOS 的 userns 策略不同**，必须在 Deck 上复核。
2. **merged-`/usr` 下的 `/lib`、`/lib64`**：启动器对符号链接目录改用 `--symlink <target> <dest>`（指向已只读绑定的 `/usr`），真实目录回退 `--ro-bind`，缺失的跳过；read 面等价、不放大。宿主 home/文档/state 仍不可见。
3. **最小 bind 集**：仅 `/usr`（及 `/lib`、`/lib64` 解析结果）、`/proc`、`/dev`、私有 `/tmp`（tmpfs）、`/input`（只读）、`/tool`（只读）、`/output`（读写）。未绑定宿主 `/home`、`/root`、`/etc`；worker 以绝对沙箱路径执行。
4. **环境清洁**：`--clearenv` + `PATH`/`HOME`，沙箱内为 `PATH`、`HOME`、`PWD`（bwrap 设置），宿主 canary 不出现。
5. **网络**：`--unshare-all` 含 `--unshare-net`，回环 `ECONNREFUSED(111)`、外网 `ENETUNREACH(101)`。
6. **进程回收**：`--new-session` 使沙箱进程独立会话/进程组；回收依赖 bwrap `--die-with-parent`（PDEATHSIG），`killpg(SIGTERM)`→1s→`killpg(SIGKILL)` 回收监控进程。超时/输出超限后宿主 `/proc` 无 `probe-worker` 残留。
7. **Linux 允许 worker fork**：与 Windows LPAC「子进程创建被拒」不同；实测 fork 子进程随 **PID 命名空间销毁**回收（无残留）。生产 worker 仍应单进程（契约 §6.2），跨平台措辞统一列 P1（§8）。
8. **rlimits 在 `pre_exec` 生效**：`setrlimit` 于 fork 后、`exec` 前应用并继承；`min(hard, desired)` 尊重硬限。
9. **M3 fail-closed 输出限额**：`read_bounded` 在超过 `limit` 时返回 Err 并记录流别；等待循环 `terminate_group` 后返回 `stage=output-limit` 且消息含 `exe_worker_response_limit`，不再静默截断。stdout 16 MiB / stderr 64 KiB，阈值内字节不变。**正常路径回归为 0 失败。**
10. **M1 root 降权**：仅 `euid==0` 时降权；`pre_exec` 顺序 `rlimits → setgroups(0,NULL) → setgid → setuid → setsid`。目标优先级 `cfg.run_as` → `SUDO_UID/SUDO_GID` → 不降权。root 场景 worker=1000、`/root` 与 `/etc/shadow` 不可读；负向对照（root 不降权）worker=0，证明降权路径有效。
11. **stdout/stderr 有界捕获的收尾**：reader 线程读到溢出即停止读取并置标志；等待循环终止沙箱，两个 reader 在线程 join 前随管道关闭结束。
12. **`pre_exec` 安全**：闭包内仅做系统调用与 `std::io::Error` 构造；异步信号安全性的正式复核列为 P1（§8）。

## 8. 未覆盖项与 P1 评审 R 项

**未覆盖项与原因：**

| 项目 | 状态 | 原因 |
| --- | --- | --- |
| SteamOS / Steam Deck 实机（M1/M3 修订后版本） | **部分未覆盖** | Deck 已实测（M1/M3 前 22 项，见 §10）；M1/M3 新增检查（root 降权/output-limit）为 WSL2-only，Deck 复跑列 P3 |
| Deck 内核差异（cgroup v2 委派、seccomp） | **未覆盖** | bwrap/userns 已在 Deck 实测通过（§10）；cgroup/seccomp 未覆盖 |
| `RLIMIT_CPU` 正式值 300 s、wall-clock 180/240/480 s | **未按正式值实测** | 探针用小值验证机制；正式值交互留待 P1 |
| 真实 SF9 worker 与黄金产物 | **未覆盖** | 本原型只跑探针 worker；P1 用真实 worker 对照 Python 产物 |
| 多沙箱并发 / 跨用户 | **未覆盖** | 仅单次顺序启动 |
| ARM64 | **未覆盖** | 仅 x86_64；`RLimit` 结构按 64 位 Linux 假设 |
| 无 core dump 的端到端证明 | **未覆盖** | 仅回读 `RLIMIT_CORE=0` |

**P1 评审 R 项（登记，不阻塞 P0-D）：**

| R 项 | 说明 | 建议 |
| --- | --- | --- |
| Linux 树级资源限额与 Windows Job 不对称 | Windows 用 Job Object 约束**整棵进程树**的内存/进程数；Linux 侧目前只有每进程 `RLIMIT_AS`/`RLIMIT_CPU`，未对「树」聚合限额（bwrap `--unshare-cgroup` 且未委派 cgroup v2） | P1 评估 cgroup v2 委派（memory.max/pids.max）与/或 `RLIMIT_NPROC`，与 Windows Job 语义对齐并记录差异 |
| `RLIMIT_FSIZE` 默认值 | 当前 `file_size_bytes=None`（不设置）；Python 侧设为 `MAX_TOTAL + MAX_METADATA` | P1 冻结 `RLIMIT_FSIZE` 默认值（2 GiB + 16 MiB 量级）并对齐 worker 写出上限 |
| 固定 `/input/archive.exe` 命名 | 契约/Python 固定输入名为 `/input/archive.exe`；Linux 启动器当前把输入目录绑定到 `/input`（或文件绑定到 `/input/<name>`） | P1 统一为固定沙箱输入名，worker 与核心契约一致 |
| 路径校验（no_links / 不相交） | Python `worker_command` 校验 `no_links`（无符号链接）且 input/output/tool 互不相交；Rust 启动器尚未实现 | P1 在启动前加入路径校验，防止符号链接/重叠路径逃逸 |
| 单进程不变量的跨平台措辞 | Windows LPAC 直接拒绝子进程；Linux 允许 fork（靠 PID 命名空间销毁遏制） | P1 统一契约措辞为「worker 不得依赖多进程」，并明确两平台遏制机制差异 |
| `pre_exec` 安全复核 | `pre_exec` 闭包处于 fork 后、exec 前的异步信号受限上下文 | P1 审计闭包，避免分配/加锁；仅保留直接系统调用 |
| host 进程不可见断言 | 当前仅靠 PID 命名空间间接保证，未显式断言沙箱内看不到宿主进程 | P1 增加探针项（沙箱内 `/proc` 仅含命名空间进程）作为显式证据 |

## 9. 对 P0 冻结的建议（待编排/oracle 复核）

- Linux 采用 **bwrap（`--unshare-all --die-with-parent --new-session --clearenv --cap-drop ALL`）+ 最小只读 bind 集 + 独占输出**；`/lib`/`/lib64` 自适应符号链接/真实目录。
- wall-clock 超时与 fail-closed 输出限额为主手段；进程回收依赖 `--die-with-parent` + PID 命名空间；Linux 允许 fork，生产 worker 仍应单进程。
- rlimits：`RLIMIT_AS=3 GiB`、`RLIMIT_CPU=300 s`、`RLIMIT_CORE=0` 作为默认。
- **root 宿主降权（M1）已完成最小实现并在 WSL root 场景实测**；Decky Loader root 宿主应在生产接入前复用同一 `run_as`/`SUDO_UID` 路径。
- SteamOS/Deck 已实机复核 userns/bwrap 版本（§10：bwrap 0.12.0、`--unshare-all` 成功）；cgroup/seccomp 与 M1/M3 的 Deck 复跑需在生产接入前完成。
- 上述 R 项（尤其树级限额不对称、路径校验、固定输入名）应在 P1 开工前冻结。

## 10. SteamOS / Deck 实测（2026-10-08）

> 本节为 M1/M3 修订前版本（探针 22 项）在真实 SteamOS Deck 上的对照结果；M1/M3 新增检查为 WSL2-only，Deck 复跑列 P3。

在真实 SteamOS Steam Deck 上运行同一 `probe-runner` + `probe-worker`，与 §2 的 WSL2 Ubuntu 22.04 结果对照。设备通过 Devkit loopback API 配对（`steamdeck@192.168.31.186`，deck 用户 uid=1000 非 root），SSH 使用本轮独立 known_hosts（`StrictHostKeyChecking=yes`、`BatchMode=yes`、`ConnectTimeout=8`）。远端仅使用唯一验证目录，证据取回并核对 SHA-256 后已删除。

### 10.1 环境对照

| 项目 | WSL2 Ubuntu 22.04（§2） | SteamOS Deck（本节） |
| --- | --- | --- |
| 发行版 | Ubuntu 22.04.3 LTS (Jammy) | SteamOS（holo），VERSION_CODENAME=holo |
| 内核 | 6.18.40.1-microsoft-standard-WSL2 | 6.18.50-valve2-1-neptune-618-gc7289a96b14d |
| 架构 | x86_64 | x86_64 |
| bubblewrap | 0.6.1-1ubuntu0.3（`/usr/bin/bwrap`） | **0.12.0**（`/usr/bin/bwrap`） |
| glibc | （未记录，测于 `ldd --version`） | 2.41 |
| 用户 | uid=1000(wqj7894) 非 root | uid=1000(deck) 非 root |
| 文件系统书写 | WSL2 ext4 | SteamOS 只读根 + btrfs `/home`（可用 56G） |

### 10.2 复现命令（Deck）

本地构建产物上传到远端唯一目录后以 deck 用户运行：

```bash
# 远端验证目录
RD=/home/deck/steam-patch-validation/p0d-20261008-033904
cd "$RD"
./probe-runner --worker "$RD/probe-worker" --root "$RD/probe-root" --out "$RD/probe.json" \
  > probe.stdout.txt 2> probe.stderr.txt
```

上传前本地 `probe-runner` SHA-256 `c567b651…8d4d213`、`probe-worker` SHA-256 `f9eea109…859c186e`；远端 `sha256sum` 与本地一致，`chmod +x` 后以 deck 用户运行。`--root` 指向 `/home/deck` 下的真实路径（而非 `/tmp`），使「宿主真实 home 下文件在沙箱内不可见」的证据更强。

### 10.3 逐项结果对照（各 22 项）

| # | id | 标题 | WSL2 | Deck | 差异 |
| --- | --- | --- | --- | --- | --- |
| 1 | bwrap_available | bwrap 可用性 | pass（0.6.1） | pass（**0.12.0**） | 版本不同，可用性/调用方式一致 |
| 2 | bwrap_worker_start | 沙箱内 worker 启动 | pass（exit=0, stdout=193B） | pass（exit=0, stdout=193B） | 无 |
| 3 | worker_not_root | worker 非 root | pass（uid=1000） | pass（uid=1000） | 无 |
| 4 | env_clean | 环境变量清洁 | pass（canary=false, env_count=3） | pass（canary=false, env_count=3） | 无 |
| 5 | authorized_read_input | 授权输入目录可读 | pass | pass | 无 |
| 6 | deny_write_input | 只读输入写被拒 | pass（EROFS 30） | pass（EROFS 30） | 无 |
| 7 | allow_write_output | 授权输出可写+宿主回读 | pass | pass | 无 |
| 8 | deny_read_home | 宿主 home 读被拒 | pass（ENOENT 2） | pass（ENOENT 2） | 路径为真实 `/home/deck`，仍不可见（证据更强） |
| 9 | deny_read_documents | 宿主文档读被拒 | pass（ENOENT 2） | pass（ENOENT 2） | 同上 |
| 10 | deny_read_state_key | 宿主 state/私钥读被拒 | pass（ENOENT 2） | pass（ENOENT 2） | 同上 |
| 11 | deny_write_home | 越界写宿主 home 被拒 | pass（ENOENT 2） | pass（ENOENT 2） | 同上 |
| 12 | unicode_long_path | 中文/长路径读写 | pass（long_path_len=282） | pass（long_path_len=282） | 无 |
| 13 | deny_network_loopback | 回环连接被拒 | pass（ECONNREFUSED 111） | pass（ECONNREFUSED 111） | 随机端口不同，结果一致 |
| 14 | deny_network_external | 外网连接被拒 | pass（ENETUNREACH 101） | pass（ENETUNREACH 101） | 无 |
| 15 | wall_clock_timeout | 宿主超时终止并回收 | pass（wall_ms=1528, dead=true） | pass（wall_ms=1531, dead=true） | ~3ms |
| 16 | timeout_residue | 超时后无残留进程 | pass（residue_pids=[]） | pass（residue_pids=[]） | 无 |
| 17 | fork_and_reclaim | fork 子进程整体回收 | pass（child_pid=3, wall_ms=2533） | pass（child_pid=3, wall_ms=2538） | ~5ms |
| 18 | rlimit_reported | rlimit 生效（机制） | pass（as=256MiB/cpu=42/core=0） | pass（as=256MiB/cpu=42/core=0） | 无 |
| 19 | rlimit_as_enforced | RLIMIT_AS 限额生效 | pass | pass | 无 |
| 20 | rlimit_as_control | RLIMIT_AS 对照 | pass | pass | 无 |
| 21 | rlimit_cpu | RLIMIT_CPU 限额生效 | pass（wall_ms=1005） | pass（wall_ms=1024） | ~19ms |
| 22 | runner_uid | 运行探针宿主用户（信息项） | info（1000） | info（1000） | 无 |

汇总对照：

| 状态 | WSL2 | Deck |
| --- | --- | --- |
| pass | 21 | 21 |
| fail | 0 | 0 |
| unsupported | 0 | 0 |
| not_covered | 0 | 0 |
| info | 1 | 1 |
| 合计 | 22 | 22 |

### 10.4 原始证据

- 原始 JSON：本机工件 `.artifacts/p0d-deck-20261008-033904/probe.json`（SHA-256 `cf3f110e2ac3f11ec8584c63b9150bf757b50a29e5aa1fbbeab713e620b8a8bb`，7232 字节）；stdout 同内容字符串化，stderr 为空（0 字节）。上传/取回均逐文件核对 SHA-256 一致。
- 关键原值：bwrap `0.12.0`；`--unshare-all` 在 deck 非 root 用户命名空间下成功；越界读/写与私钥读在真实 `/home/deck/...` 路径上均返回 `ENOENT(2)`；回环 `ECONNREFUSED(111)`、外网 `ENETUNREACH(101)`；超时/回收与 rlimit 行为与 WSL2 同一错误码与量级。

### 10.5 结论与未覆盖项

- **结论**：SteamOS Deck（bwrap 0.12.0、内核 6.18.50-valve）与本机 WSL2（bwrap 0.6.1）**逐项结果一致（21 pass / 0 fail / 1 info）**，`--unshare-all` 与最小只读 bind 集在 SteamOS 上行为等价；合成 home/文档/state 路径位于真实 `/home/deck` 下时仍不可见，隔离门槛成立。未观察到需针对 bwrap 0.12 调整启动参数的现象。
- **已由本节覆盖**：SteamOS / Steam Deck 实机（remediation 前 22 项）、Deck 内核 userns 可用性（`--unshare-all` 成功）、bwrap 版本差异（0.6.1 → 0.12.0）。
- **仍未覆盖**：root 宿主降权（WSL root 场景已补测，见 §6；Deck root 形态复跑列 P3）、Deck 上 cgroup v2 委派与 seccomp 策略、真实 SF9 worker 与黄金产物、并发/跨用户、ARM64、正式超时值（wall-clock 180/240/480s、CPU 300s）、无 core dump 的端到端证明、CEF/HTTP 与插件部署验证。这些留待 P1/P3。

### 10.6 变更文件（本节）

- 本文件追加第 10 节，并同步首部范围声明与 §7 未覆盖表；另更新 `progress.md` 状态行、P0 行并追加本轮施工记录。未改动任何代码、`Cargo.toml`/`Cargo.lock`。
- 设备侧只使用验证目录 `/home/deck/steam-patch-validation/p0d-20261008-033904/`，证据取回并核对后已删除，未触碰插件/state/游戏目录。

## 11. 变更文件

- 修改 `crates/installer-worker/src/lib.rs`（平台模块 gating 与 `run_sandboxed` 重导出）。
- 修改 `crates/installer-worker/src/model.rs`（`SandboxLimits` Linux rlimit 字段、`SandboxConfig::run_as`）。
- 新增 `crates/installer-worker/src/linux/mod.rs`、`sys.rs`、`launch.rs`、`probe_os.rs`。
- 修改 `crates/installer-worker/src/bin/probe_worker.rs`（`linux_impl`，新增 `big_output`/`big_stderr`）。
- 修改 `crates/installer-worker/src/bin/probe_runner.rs`（`linux_impl`，`--run-as`、root 降权与输出限额检查）。
- 新增本文件。
- **未修改** `Cargo.toml`/`Cargo.lock`（无新增依赖）、`crates/core`、`crates/cli`、`apps/**`、既有 P0 文档。

未执行：真实游戏/安装链路、Deck 上的 M1/M3 复跑（列 P3）、并发/ARM64 矩阵、正式超时值与树级限额。
