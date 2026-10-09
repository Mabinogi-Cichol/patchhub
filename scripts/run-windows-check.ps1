<#
.SYNOPSIS
  P4 Windows acceptance runner: build the bundled worker in RELEASE and run the
  core/cli tests plus the ignored production EXE orchestrator tests with the real
  generic-EXE samples on Windows.

.DESCRIPTION
  Mirrors .artifacts/p3-production/run-linux-check.sh for the Windows host:
    1. cargo build --release -p steam-patch-installer-worker --bin sf-worker
    2. set SF_WORKER_TEST_BIN to <target>\release\sf-worker.exe
    3. set P3_EVIDENCE_PATH to .artifacts\p4-windows\windows-real-sf.json
    4. cargo test -p steam-patch-core -p steam-patch-cli
    5. cargo test -p steam-patch-core --lib installer::exe_orchestrator::tests -- --ignored --nocapture
  All output is teed to .artifacts\p4-windows\windows-check.log; the first failing
  step aborts with a non-zero exit code and prints a summary line.

  Optional -Clippy runs `cargo clippy` (core/cli/worker, --all-targets -D warnings)
  before the tests.

.PREREQUISITES
  - Real SF/generic-EXE samples under .artifacts\generic-exe-local (appids
    2303800, 3606360, 3721970 as *-archive.exe); the ignored tests read them.
  - The RELEASE worker is mandatory: a debug worker cannot finish the ~1.2 GiB
    conversion within the 300s CPU sandbox limit.

.NOTES
  Usage: pwsh -NoProfile -File scripts\run-windows-check.ps1 [-Clippy]
#>
[CmdletBinding()]
param(
    [switch]$Clippy
)

$ErrorActionPreference = 'Stop'

$taskRoot = Split-Path -Parent $PSScriptRoot
$artifactDir = Join-Path $taskRoot '.artifacts\p4-windows'
$logPath = Join-Path $artifactDir 'windows-check.log'
New-Item -ItemType Directory -Path $artifactDir -Force | Out-Null
Set-Content -LiteralPath $logPath -Value "# P4 Windows check $(Get-Date -Format o)" -Encoding UTF8

# Prefer the repository's isolated toolchain; fall back to an installed cargo.
$taskCargo = Join-Path $taskRoot '.tools\cargo\bin\cargo.exe'
if (Test-Path -LiteralPath $taskCargo) {
    $env:CARGO_HOME = Join-Path $taskRoot '.tools\cargo'
    $env:RUSTUP_HOME = Join-Path $taskRoot '.tools\rustup'
    $env:PATH = (Split-Path -Parent $taskCargo) + [IO.Path]::PathSeparator + $env:PATH
} else {
    $taskCargo = (Get-Command cargo -ErrorAction Stop).Source
}

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $taskRoot 'target' }
$workerExe = Join-Path $targetDir 'release\sf-worker.exe'
$evidencePath = Join-Path $artifactDir 'windows-real-sf.json'

function Write-Summary {
    param([Parameter(Mandatory)][string]$Message)
    $Message | Tee-Object -FilePath $logPath -Append
}

function Invoke-Step {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string[]]$Command
    )
    $exe = $Command[0]
    $arguments = @($Command | Select-Object -Skip 1)
    Write-Host "== $Name =="
    "== $Name ==" | Tee-Object -FilePath $logPath -Append | Out-Null
    & $exe @arguments 2>&1 | Tee-Object -FilePath $logPath -Append
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        Write-Summary "FAILED: $Name (exit $exitCode). See $logPath"
        exit $exitCode
    }
}

Push-Location $taskRoot
try {
    Invoke-Step -Name 'build sf-worker (release)' -Command @(
        $taskCargo, 'build', '--release', '-p', 'steam-patch-installer-worker', '--bin', 'sf-worker'
    )

    if (-not (Test-Path -LiteralPath $workerExe)) {
        Write-Summary "FAILED: release worker missing at $workerExe. See $logPath"
        exit 1
    }
    $env:SF_WORKER_TEST_BIN = $workerExe
    $env:P3_EVIDENCE_PATH = $evidencePath
    "SF_WORKER_TEST_BIN=$workerExe" | Tee-Object -FilePath $logPath -Append | Out-Null
    "P3_EVIDENCE_PATH=$evidencePath" | Tee-Object -FilePath $logPath -Append | Out-Null

    if ($Clippy) {
        Invoke-Step -Name 'clippy (core/cli/worker)' -Command @(
            $taskCargo, 'clippy', '-p', 'steam-patch-core', '-p', 'steam-patch-cli',
            '-p', 'steam-patch-installer-worker', '--all-targets', '--', '-D', 'warnings'
        )
    }

    Invoke-Step -Name 'core/cli tests' -Command @(
        $taskCargo, 'test', '-p', 'steam-patch-core', '-p', 'steam-patch-cli'
    )

    Invoke-Step -Name 'exe_orchestrator production tests (ignored)' -Command @(
        $taskCargo, 'test', '-p', 'steam-patch-core', '--lib',
        'installer::exe_orchestrator::tests', '--', '--ignored', '--nocapture'
    )

    Write-Summary "OK: P4 Windows check passed. Evidence: $evidencePath; log: $logPath"
} finally {
    Pop-Location
}
