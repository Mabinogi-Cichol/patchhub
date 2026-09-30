# Prefer this repository's isolated toolchain; fall back to an installed cargo.
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$taskCargo = Join-Path $taskRoot '.tools/cargo/bin/cargo.exe'
if (Test-Path -LiteralPath $taskCargo) {
    $env:CARGO_HOME = Join-Path $taskRoot '.tools/cargo'
    $env:RUSTUP_HOME = Join-Path $taskRoot '.tools/rustup'
    $env:PATH = (Split-Path -Parent $taskCargo) + [IO.Path]::PathSeparator + $env:PATH
} else {
    $taskCargo = (Get-Command cargo -ErrorAction Stop).Source
}
Push-Location $taskRoot
try {
    & $taskCargo @args
    $taskExitCode = $LASTEXITCODE
} finally {
    Pop-Location
}
exit $taskExitCode
