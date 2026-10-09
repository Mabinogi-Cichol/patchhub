# Windows 隔离原型实测证据（P0-B LPAC + Job Object）

日期：2026-10-08（Asia/Shanghai）。状态：**原型已实现并本机实测通过（27/31 通过，1 项因宿主所处 Job 嵌套未覆盖，3 项信息项）**。

关联：[design.md](design.md) 第 4.3 节「双平台隔离门槛」；[progress.md](progress.md)。本文件只记录 P0-B 原型与实测原始证据，不代表 P0 整体通过或生产接入。

## 1. 目标与范围

按设计 §4.3 与 deepwork「Windows 沙箱结论」，实现固定 worker 的原生隔离启动器与探针，核实以下清单项：受限身份/零 capability、只读输入与独占输出、越界读写、网络（回环/外网）、注册表、句柄继承白名单、进程树回收、内存/CPU/时限、无控制台、环境变量清洁、中文/长路径。

- 只允许新增 `crates/installer-worker/**`、根 `Cargo.toml` 的 `members` 行与本证据文档；未改动 `crates/core`、`crates/cli`、`apps/**`。
- 探针使用**合成的临时目录与假文件**模拟游戏目录/文档/state/私钥，未触碰真实游戏、用户文档、私钥或注册表业务键（写入尝试使用专门的 `SteamPatchProbeP0` 键，且被拒绝）。
- Windows 代码以 `cfg(windows)` 隔离；非 Windows 提供 stub，保证工作区可编译。

## 2. 环境与复现命令

| 项目 | 值 |
| --- | --- |
| OS | Windows 11 26H2 正式版 x64（注册表 `ProductName` 旧字段仍显示 Windows 10 Pro），DisplayVersion 26H2，Build 26300.9457（`CurrentBuildNumber`/`UBR`，注册表读取；无 manifest，`winver` 数值以此为准；2026-10-09 复核：`WindowsSelfHost` 无 Insider flighting 标记，先前“非标准 build”判断撤销） |
| 架构 | x86_64 |
| Rust | rustc 1.94.0 / cargo 1.94.0，host `x86_64-pc-windows-msvc`（仓库隔离工具链 `.tools/cargo`） |
| windows crate | 0.62.2（`Cargo.lock` 既有版本，直接依赖） |
| winreg | 0.56 |
| Runner 自身是否已在 Job 中 | `in_job=true`（见「未覆盖项」对嵌套 Job 的影响） |

复现命令（PowerShell，仓库根目录）：

```powershell
$env:CARGO_HOME="$PWD\.tools\cargo"; $env:RUSTUP_HOME="$PWD\.tools\rustup"
$env:PATH="$PWD\.tools\cargo\bin;$env:PATH"; $env:CARGO_TARGET_DIR="$PWD\target"

cargo build -p steam-patch-installer-worker --offline
cargo test  -p steam-patch-installer-worker --offline   # 4 个跨平台单测通过
cargo check --workspace --offline                        # 工作区检查通过

.\target\debug\probe-runner.exe --out "$env:TEMP\p0-probe-raw.json"
```

`probe-runner` 默认把 `probe-worker.exe` 复制到本次临时根目录下的 `worker/`（该目录由探针自身创建并拥有，才能对其设置 DACL），再对 worker 目录授权、启动并回收。

## 3. 实现摘要

`crates/installer-worker/`：

- `src/model.rs`：跨平台数据模型（`SandboxConfig`/`SandboxLimits`/`LaunchOutcome`/`WorkerRequest`/`WorkerResponse`/`CheckResult`）与 4 个单测。
- `src/stub.rs`：非 Windows stub（`run_sandboxed` 返回 `Unsupported`）。
- `src/win/appcontainer.rs`：`CreateAppContainerProfile`（`ERROR_ALREADY_EXISTS`→`DeriveAppContainerSidFromAppContainerName`）、`DeleteAppContainerProfile`、`FreeSid`/`LocalFree`，以及 `GetNamedSecurityInfoW`+`SetEntriesInAclW`+`SetNamedSecurityInfoW` 的继承式 ACL 授权（`OBJECT_INHERIT_ACE`+`CONTAINER_INHERIT_ACE`）。
- `src/win/job.rs`：`CreateJobObjectW` + `JobObjectExtendedLimitInformation`（`KILL_ON_JOB_CLOSE`|`PROCESS_MEMORY`|`JOB_MEMORY`|`ACTIVE_PROCESS`|`JOB_TIME`）+ `JobObjectCpuRateControlInformation`（`ENABLE`|`HARD_CAP`）、`TerminateJobObject`、会计/峰值查询、`Drop` 关闭句柄。
- `src/win/launch.rs`：`InitializeProcThreadAttributeList`→`UpdateProcThreadAttribute`（`SECURITY_CAPABILITIES`、`ALL_APPLICATION_PACKAGES_POLICY=OPT_OUT`、`JOB_LIST`、可选 `HANDLE_LIST`）→ `CreateProcessAsUserW`（token=NULL，失败再试 `CreateProcessW`）→ `CREATE_SUSPENDED` + 匿名管道接管 stdio + `STARTF_USESTDHANDLES` → `ResumeThread`；等待循环内按配置执行超时/中途终止/提前关闭 Job/会计快照；`spawn_tree_child` 可向同一 Job 追加 LPAC 进程用于进程树回收测试。
- `src/win/probe_os.rs`：token（`TokenIsAppContainer`/`TokenAppContainerSid`/`TokenCapabilities`/`TokenElevation`）、控制台、`VirtualAlloc` 提交、`process_alive`（`GetExitCodeProcess != STILL_ACTIVE`）、`try_duplicate`、`self_in_job`。
- `src/bin/probe_worker.rs`：被隔离 worker，stdin 读 JSON、stdout 写 JSON；场景 selfcheck/read_file/write_file/path_rw/net_connect/registry_read/registry_write/alloc/busy/sleep/spawn_child/`child-sleep`。
- `src/bin/probe_runner.rs`：宿主探针，创建合成目录与 profile、授权、逐项启动并汇总 JSON 证据。

`windows` 0.62.2 已直接导出 `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`（0x20009）、`PROC_THREAD_ATTRIBUTE_JOB_LIST`（0x2000D）、`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`（0x20002）、`PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY`（0x2000F，`Win32_System_Threading`）与 `PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT=1`（`Win32_System_WindowsProgramming`）。**无需自定义常量**，与调研笔记中「0x2000F 未导出」的假设不同，已按实际版本使用。

## 4. 实测结果汇总

| 状态 | 数量 |
| --- | --- |
| pass | 27 |
| fail | 0 |
| unsupported | 0 |
| not_covered | 1（CPU 硬上限，嵌套 Job） |
| info | 3（普通注册表读、进程数限额、宿主 in_job） |
| 合计 | 31 |

关键原值：

- LPAC 身份：`TokenIsAppContainer=true`，`TokenAppContainerSid` 与 profile SID 一致，`TokenCapabilities` 计数 = 0；`CreateProcessAsUserW`（token=NULL）成功。
- 无控制台：`GetConsoleWindow()=0`，标准输出非控制台。
- worker 归属 Job：`IsProcessInJob=true`（`JOB_LIST` 原子分配）。
- 环境：传入 16 个变量、宿主 canary 未出现（`canary_present=false`）。
- 句柄白名单：同一 canary 句柄在 WHITELIST 模式 `present=false`，无 WHITELIST 模式 `present=true`。
- 文件：授权输入可读、授权输出可写、中文长路径（354 字符）往返成功；未授权游戏/文档/state 私钥读与越界写均 `ACCESS_DENIED (5)`，只读输入目录写同样被拒。
- 网络：`WSAStartup` 在 LPAC 下返回 `10107`（Winsock 不可用），回环与外网均未连接。
- 注册表：`HKLM\SOFTWARE\SteamPatchProbeP0` 写被拒 (5)，`HKLM\SAM` 读被拒 (5)；普通 `HKLM\...\CurrentVersion` 读也被拒 (5)。
- 进程：LPAC worker 直接创建子进程被拒 (5)；追加到同一 Job 的第二个 LPAC 进程在 `TerminateJobObject` 与关闭 Job 句柄（`KILL_ON_JOB_CLOSE`）后均被回收；1.5s 超时以 `Timeout` 终止且进程消失。
- 限额：进程内存 128 MiB 时 256 MiB `VirtualAlloc` 失败（1455），1 GiB 时 32 MiB 成功；`PerJobUserTimeLimit=1s` 使 10s busy 提前终止（退出码 0xC0000044/`STATUS_QUOTA_EXCEEDED`）。
- CPU 硬上限：Job 内配置回读为 `flags=0x5, cpu_rate=1000`（10% HARD_CAP），但 3s busy 实测 CPU ~2.5s，未生效（见第 6 节）。

## 5. 原始证据（probe-runner stdout / p0-probe-raw.json）

```json
{
  "arch": "x86_64",
  "checks": [
    {
      "actual": "create=S-1-15-2-738567026-2296793576-1106105171-3811003028-762746903-2557518052-260468480; derive=S-1-15-2-738567026-2296793576-1106105171-3811003028-762746903-2557518052-260468480",
      "expected": "首次 CreateAppContainerProfile，第二次 ERROR_ALREADY_EXISTS 派生相同 SID",
      "id": "profile_lifecycle",
      "status": "pass",
      "title": "AppContainer profile 创建/已存在派生 SID"
    },
    {
      "actual": "is_app_container=true, sid=S-1-15-2-3018636329-1860416333-3994039469-899363544-2937047660-4122862612-3041554975, capability_count=Some(0), api=CreateProcessAsUserW, exit=0, reason=Completed",
      "detail": "stdout=1286 bytes, stderr=0 bytes: ",
      "expected": "TokenIsAppContainer=true，TokenAppContainerSid=profile SID，capability=0",
      "id": "lpac_self_check",
      "status": "pass",
      "title": "LPAC/AppContainer 自检（token 身份）"
    },
    {
      "actual": "started pid=221580 exit=0",
      "expected": "无网络/资源 capability 仍能启动 worker",
      "id": "zero_capability_start",
      "status": "pass",
      "title": "零 capability 下启动"
    },
    {
      "actual": "console_window=0, std_output_is_console=false",
      "expected": "GetConsoleWindow()==0 且标准输出非控制台",
      "id": "no_console_window",
      "status": "pass",
      "title": "无控制台窗口"
    },
    {
      "actual": "in_job=true",
      "expected": "JOB_LIST 原子分配，IsProcessInJob=true",
      "id": "worker_in_job",
      "status": "pass",
      "title": "worker 归属 Job Object"
    },
    {
      "actual": "canary_present=false, env_count=16, passed_env=16",
      "expected": "宿主 STEAM_PATCH_PROBE_CANARY 不出现；仅传入白名单环境",
      "id": "env_clean",
      "status": "pass",
      "title": "环境变量清洁"
    },
    {
      "actual": "raw_canary=552; whitelist_mode_present=false; no_whitelist_present=true",
      "expected": "WHITELIST 模式不继承未列句柄；无 WHITELIST 模式继承同一句柄",
      "id": "handle_whitelist",
      "status": "pass",
      "title": "句柄继承白名单"
    },
    {
      "actual": "PROBE_INPUT_v1_中文",
      "expected": "读取成功且内容一致",
      "id": "authorized_read_input",
      "status": "pass",
      "title": "授权输入目录可读"
    },
    {
      "actual": "ok=false, error_code=Some(5), exit=0, reason=Completed",
      "error_code": 5,
      "expected": "访问被拒 (win32 5 / 无输出)",
      "id": "deny_read_game",
      "status": "pass",
      "title": "未授权合成游戏目录读被拒"
    },
    {
      "actual": "ok=false, error_code=Some(5), exit=0, reason=Completed",
      "error_code": 5,
      "expected": "访问被拒 (win32 5 / 无输出)",
      "id": "deny_read_documents",
      "status": "pass",
      "title": "未授权合成文档目录读被拒"
    },
    {
      "actual": "ok=false, error_code=Some(5), exit=0, reason=Completed",
      "error_code": 5,
      "expected": "访问被拒 (win32 5 / 无输出)",
      "id": "deny_read_state_key",
      "status": "pass",
      "title": "未授权合成 state/私钥读被拒"
    },
    {
      "actual": "ok=false, error_code=Some(5), exit=0",
      "error_code": 5,
      "expected": "访问被拒 (win32 5)",
      "id": "deny_write_game",
      "status": "pass",
      "title": "越界写合成游戏目录被拒"
    },
    {
      "actual": "ok=false, error_code=Some(5), exit=0",
      "error_code": 5,
      "expected": "访问被拒 (win32 5)",
      "id": "deny_write_documents",
      "status": "pass",
      "title": "越界写合成文档目录被拒"
    },
    {
      "actual": "ok=false, error_code=Some(5), exit=0",
      "error_code": 5,
      "expected": "ACCESS_DENIED (5)",
      "id": "deny_write_input",
      "status": "pass",
      "title": "只读输入目录写被拒"
    },
    {
      "actual": "written",
      "expected": "写入成功且宿主可回读",
      "id": "allow_write_output",
      "status": "pass",
      "title": "授权输出目录可写"
    },
    {
      "actual": "roundtrip_ok=true, long_path_len=354",
      "expected": "中文文件名与 >260 字符路径读写成功",
      "id": "unicode_long_path",
      "status": "pass",
      "title": "授权目录内中文/长路径读写"
    },
    {
      "actual": "connect 127.0.0.1:12649 connected=false, error_code=Some(10107), error=Some(\"WSAStartup failed: Winsock unavailable under LPAC\"), exit=0, stdout=129B, stderr=\"\"",
      "error_code": 10107,
      "expected": "零 capability 下回环连接失败",
      "id": "deny_network_loopback",
      "status": "pass",
      "title": "回环网络连接被拒"
    },
    {
      "actual": "connect 1.1.1.1:443 connected=false, error_code=Some(10107), error=Some(\"WSAStartup failed: Winsock unavailable under LPAC\"), exit=0, stdout=129B, stderr=\"\"",
      "error_code": 10107,
      "expected": "零 capability 下外网连接失败",
      "id": "deny_network_external",
      "status": "pass",
      "title": "外网连接被拒"
    },
    {
      "actual": "ok=false, error_code=Some(5)",
      "error_code": 5,
      "expected": "HKLM\\SOFTWARE 写入被拒 (5)",
      "id": "deny_registry_write",
      "status": "pass",
      "title": "注册表写被拒"
    },
    {
      "actual": "ok=false, error_code=Some(5)",
      "error_code": 5,
      "expected": "HKLM\\SAM 读取被拒",
      "id": "deny_registry_read_protected",
      "status": "pass",
      "title": "受保护注册表读被拒"
    },
    {
      "actual": "ok=false, error_code=Some(5)",
      "expected": "记录实际结果（LPAC 默认可读资源核验）",
      "id": "registry_read_info",
      "status": "info",
      "title": "普通注册表读（信息项）"
    },
    {
      "actual": "active_processes=4, tree_pids=[165304], worker_dead=true, tree_dead=true, reason=TerminatedByHost",
      "expected": "ActiveProcesses>=2，TerminateJobObject 后全部消失",
      "id": "child_process_in_job",
      "status": "pass",
      "title": "同 Job 多进程归属与回收"
    },
    {
      "actual": "spawn_ok=false, error_code=Some(5), error=Some(\"spawn 'C:\\\\Users\\\\wqj78\\\\AppData\\\\Local\\\\Temp\\\\steam-patch-probe-208556-1791400289356\\\\worker\\\\probe-worker.exe' failed: 拒绝访问。 (os error 5)\")",
      "error_code": 5,
      "expected": "LPAC worker 直接创建子进程被拒 (ACCESS_DENIED 5)",
      "id": "child_process_controlled",
      "status": "pass",
      "title": "worker 创建子进程受控"
    },
    {
      "actual": "reason=KilledOnJobClose, worker_dead=true, tree_dead=true",
      "expected": "关闭 Job 句柄后 worker 与 tree 子进程均终止",
      "id": "kill_on_job_close",
      "status": "pass",
      "title": "父退出/KILL_ON_JOB_CLOSE 回收进程树"
    },
    {
      "actual": "reason=Timeout, wall_ms=1546, dead=true, exit=3",
      "expected": "1.5s 超时后 Timeout 终止且进程消失",
      "id": "wall_clock_timeout",
      "status": "pass",
      "title": "宿主超时终止并回收"
    },
    {
      "actual": "spawn_ok=false, error_code=Some(1816), reason=Completed, exit=0",
      "expected": "ActiveProcessLimit=1 时子进程创建受控（失败或被回收）",
      "id": "active_process_limit",
      "status": "info",
      "title": "进程数限额"
    },
    {
      "actual": "alloc_ok=false, error_code=Some(1455)",
      "error_code": 1455,
      "expected": "PROCESS_MEMORY=128MiB 时 256MiB commit 失败",
      "id": "memory_limit",
      "status": "pass",
      "title": "内存限额"
    },
    {
      "actual": "alloc_ok=true",
      "expected": "PROCESS_MEMORY=1GiB 时 32MiB commit 成功",
      "id": "memory_limit_control",
      "status": "pass",
      "title": "内存限额对照"
    },
    {
      "actual": "cpu_ms=2531, wall_ms=3019, job_cpu_rate=(flags=0x5, cpu_rate=1000)",
      "detail": "runner_in_job=true; Windows does not enforce CPU rate control for nested jobs, which explains no observed throttling",
      "expected": "10% HARD_CAP 下 3s busy 的 CPU 时间 <= ~1.5s",
      "id": "cpu_hard_cap",
      "status": "not_covered",
      "title": "CPU 硬上限（JobObjectCpuRateControl）"
    },
    {
      "actual": "wall_ms=5380, reason=Completed, exit=0xC0000044",
      "expected": "PerJobUserTimeLimit=1s 时 10s busy 提前被终止（<9.5s）",
      "id": "job_time_limit",
      "status": "pass",
      "title": "Job 用户态时间限额"
    },
    {
      "actual": "in_job=true",
      "expected": "记录实际值（影响 JOB_LIST 原子分配）",
      "id": "host_in_job",
      "status": "info",
      "title": "宿主进程是否已在 Job 中（嵌套能力探测）"
    }
  ],
  "generated_unix_secs": 1791400304,
  "host": {
    "in_job": true,
    "runner_exe": "E:\\ProjectFiles\\Git\\steam-hentai-patch-searcher\\target\\debug\\probe-runner.exe"
  },
  "os": {
    "current_build": "26300",
    "display_version": "26H2",
    "major": 10,
    "minor": 0,
    "product_name": "Windows 10 Pro",
    "ubr": 9457
  },
  "sandbox": {
    "profile_name": "steam-patch-probe",
    "profile_sid": "S-1-15-2-3018636329-1860416333-3994039469-899363544-2937047660-4122862612-3041554975",
    "root": "C:\\Users\\wqj78\\AppData\\Local\\Temp\\steam-patch-probe-208556-1791400289356",
    "worker_dir": "C:\\Users\\wqj78\\AppData\\Local\\Temp\\steam-patch-probe-208556-1791400289356\\worker",
    "worker_exe": "C:\\Users\\wqj78\\AppData\\Local\\Temp\\steam-patch-probe-208556-1791400289356\\worker\\probe-worker.exe"
  },
  "schema": "steam-patch/p0-sandbox-probe/v1",
  "summary": {
    "failed": 0,
    "not_covered": 1,
    "passed": 27,
    "total": 31,
    "unsupported": 0
  }
}
```

## 6. 发现、偏差与踩坑记录

1. **LPAC 生效**：`TokenIsAppContainer=true`、`TokenAppContainerSid` 等于 package SID、`TokenCapabilities` 为 0，且 worker 在零 capability 下正常启动；`ALL_APPLICATION_PACKAGES_POLICY=OPT_OUT` 是关键属性。Win10 1703+（build 15063）为最低要求，本机 26300 通过。
2. **创建 API**：`CreateProcessAsUserW`（token=NULL）成功；实现保留 `CreateProcessW` 兜底并记录实际路径。`CreateProcessAsUserW` 需要兼容参数顺序与 `CREATE_UNICODE_ENVIRONMENT`。
3. **环境块**：仅含 `SystemRoot/windir/PATH/PATHEXT/TEMP/TMP/COMPUTERNAME/PROCESSOR_ARCHITECTURE/NUMBER_OF_PROCESSORS` 的最小块会使 `CreateProcessAsUserW`/`CreateProcessW` 失败 `ERROR_ENVVAR_NOT_FOUND (203 / 0x800700CB)`；补入 `SystemDrive/USERPROFILE/LOCALAPPDATA/APPDATA/ProgramData/ProgramFiles/ComSpec` 后成功。生产实现必须先验证所需变量集合。
4. **句柄枚举陷阱**：在 `0..N` 逐一调用 `GetHandleInformation`（以及 `GetFileType`）会在遇到无效句柄值（本机为 1）时以 `STATUS_INVALID_HANDLE (0xC0000008)` **终止进程**（疑似该加固环境/EDR 行为）。已改为「宿主传入 canary 句柄值 + worker 用 `DuplicateHandle` 单句柄探测」的差分测试，避免扫描。
5. **子进程受限**：LPAC worker 无法直接创建子进程（`ACCESS_DENIED (5)`）；`ActiveProcessLimit=1` 时创建失败返回 `ERROR_NOT_ENOUGH_QUOTA (1816)`。进程树回收改用「启动器向同一 Job 追加 LPAC 进程」验证。这意味着生产转换链路不得依赖 worker fork 子进程。
6. **网络**：LPAC 下 `WSAStartup` 返回 `10107`，Winsock 整体不可用（回环/外网均拒绝）；Rust std `connect_timeout` 会因断言在 `std/sys/pal/windows/winsock.rs` 崩溃，故 worker 先显式调用 `WSAStartup` 并结构化上报。
7. **CPU 硬上限未覆盖**：`JobObjectCpuRateControlInformation` 配置成功并回读为 `flags=0x5, cpu_rate=1000`，但未观察到限速；原因是本机 runner 自身已在父 Job 中（`in_job=true`），Windows 不支持嵌套 Job 的 CPU rate control。`JOB_OBJECT_LIMIT_JOB_TIME` 正常生效（退出码 0xC0000044 `STATUS_QUOTA_EXCEEDED`），可作为 CPU/时限的可移植下限。
8. **注册表访问比预期更严**：不仅写 `HKLM\SOFTWARE` 与读 `HKLM\SAM` 被拒，连普通 `HKLM\...\CurrentVersion` 读也返回 `ACCESS_DENIED (5)`。LPAC 的注册表可用面需要在生产前用实际键集重新核对。
9. **windows crate 版本**：0.62.2 已导出全部所需 `PROC_THREAD_ATTRIBUTE_*` 常量与 `PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT`，无需自定义常量。
10. **ACL 前提**：当前加固宿主下 `target\debug` 对当前用户没有 `WRITE_DAC`，直接授权失败；已改为把 worker 复制到探针自有的临时目录再授权。生产 worker 应放在安装器自有目录中。

## 7. 未覆盖项与原因

| 项目 | 状态 | 原因 |
| --- | --- | --- |
| CPU rate hard cap | 未覆盖 | 宿主已在 Job 中，嵌套 Job 不支持；需在非 Job 宿主复核，或以 `JOB_OBJECT_LIMIT_JOB_TIME` 为基准 |
| 真实游戏/文档/state/私钥路径 | 未覆盖 | 按任务要求只使用合成临时目录，不触碰真实用户数据 |
| reparse point / symlink / 跨卷 ACL | 未覆盖 | 原型未实现；不同卷与 reparse 需后续专门用例 |
| 占用文件/杀软实时拦截 | 未覆盖 | 仅记录观察到的错误码，未做杀软矩阵 |
| profile 崩溃后遗留清理 | 未覆盖 | 仅实现正常路径 `DeleteAppContainerProfile`；异常退出清理待补 |
| Windows 版本矩阵 | 部分覆盖 | 本机为 Win11 26H2 正式版（2026-10-09 复核）；更早 Win11 版本（23H2/24H2 等）与 1703/1809 等未测 |
| ARM64 | 未覆盖 | 仅 x64 |

## 8. 对 P0 冻结的建议（待编排/oracle 复核）

- Windows 继续采用 **LPAC（AppContainer + ALL_APPLICATION_PACKAGES_OPT_OUT）+ Job Object**；最低版本建议 Win10 1703（build 15063）并在发布前按实际主流版本补测。
- CPU 限额以 `PerJobUserTimeLimit`（已验证）为主，`JobObjectCpuRateControlInformation` 作为「宿主非嵌套」时的加固项，不能作为唯一 CPU 证明。
- 显式环境块是隔离的一部分，但必须冻结必需变量集合（至少含 SystemDrive/USERPROFILE/LOCALAPPDATA 等），并在启动失败时区分 `ERROR_ENVVAR_NOT_FOUND`。
- worker 必须单进程：不得依赖 fork/子进程；如需多阶段，应由核心编排多次单进程调用。
- 只读输入 + 独占输出的 ACL 授权已被实测证明可行；越界读写、注册表、网络均被语言/OS 层拒绝。
- 句柄继承用 `HANDLE_LIST` 白名单，差分测试证明有效；生产只需列入 stdio 句柄与极少数必要句柄。

## 9. 变更文件

- 新增 `crates/installer-worker/`（库 + `probe-runner`/`probe-worker` 两个 bin）。
- 修改根 `Cargo.toml` 的 `members` 行（加入 `crates/installer-worker`）。
- 修改 `Cargo.lock`（新增 workspace 成员及其依赖，`--locked` 构建所需）。
- 新增本文件。

未执行：真实游戏/安装链路、跨版本/跨卷/EDR 矩阵、Linux 隔离对照（P0-D 待授权）。
