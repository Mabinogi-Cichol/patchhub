[CmdletBinding()]
param(
    [string]$Flutter = '',
    [switch]$SkipRust
)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$taskApp = Join-Path $taskRoot 'apps/windows-flutter'
if (-not $Flutter) {
    $taskLocal = Join-Path $taskRoot '.tools/flutter/bin/flutter.bat'
    $Flutter = if (Test-Path -LiteralPath $taskLocal) { $taskLocal } else {
        (Get-Command flutter -ErrorAction Stop).Source
    }
}
function Invoke-Checked {
    param([string]$Executable, [string[]]$Arguments)
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Executable failed with exit code $LASTEXITCODE" }
}
$taskExpected = (Get-Content (Join-Path $taskApp '.flutter-version') -Raw).Trim()
# A fresh SDK bootstraps its tool before machine output is usable as JSON.
Invoke-Checked $Flutter @('--version')
$taskVersionJson = & $Flutter --version --machine
if ($LASTEXITCODE -ne 0) { throw 'Unable to read Flutter version metadata' }
$taskVersion = ($taskVersionJson | ConvertFrom-Json).frameworkVersion
if ($taskVersion -ne $taskExpected) {
    throw "Flutter $taskExpected required; found $taskVersion"
}
if (-not $SkipRust) {
    $taskCargo = Join-Path $taskRoot '.tools/cargo/bin/cargo.exe'
    if (Test-Path -LiteralPath $taskCargo) {
        $env:CARGO_HOME = Join-Path $taskRoot '.tools/cargo'
        $env:RUSTUP_HOME = Join-Path $taskRoot '.tools/rustup'
    } else { $taskCargo = (Get-Command cargo -ErrorAction Stop).Source }
    Push-Location $taskRoot
    try {
        Invoke-Checked $taskCargo @('build','--release','--locked','-p','patchhub-cli',
            '-p','patchhub-installer-worker','--bin','patchhub-core','--bin','patchhub-worker')
    } finally { Pop-Location }
}
Copy-Item -LiteralPath (Join-Path $taskRoot 'packages/contracts/generated/v5.schema.json') -Destination (Join-Path $taskApp 'assets/protocol-v5.schema.json')
Push-Location $taskApp
try {
    Invoke-Checked $Flutter @('pub','get','--enforce-lockfile')
    Invoke-Checked $Flutter @('analyze')
    Invoke-Checked $Flutter @('test')
    # Flutter 3.47.7 can reuse a font subset after app.dill changes because the
    # Windows asset target does not list app.dill as an input. Rebuild assets so
    # newly referenced icons are included in the release font.
    $taskFlutterCache = Join-Path $taskApp '.dart_tool/flutter_build'
    if (Test-Path -LiteralPath $taskFlutterCache) {
        Get-ChildItem -LiteralPath $taskFlutterCache -Filter 'release_bundle_windows-x64_assets.stamp' -File -Recurse |
            Remove-Item -Force
    }
    Invoke-Checked $Flutter @('build','windows','--release')
    $taskDeps = & $Flutter pub deps --json
    if ($LASTEXITCODE -ne 0) { throw 'Unable to collect Flutter dependencies' }
} finally { Pop-Location }
$taskOutput = Join-Path $taskApp 'build/windows/x64/runner/Release'
foreach ($taskBinary in @('patchhub-core.exe','patchhub-worker.exe')) {
    Copy-Item -LiteralPath (Join-Path $taskRoot "target/release/$taskBinary") -Destination $taskOutput
}
$taskResources = Join-Path $taskOutput 'resources'
New-Item -ItemType Directory -Path $taskResources -Force | Out-Null
foreach ($taskResource in @('catalog.json','signed-catalog.json','trust-keys.json')) {
    Copy-Item -LiteralPath (Join-Path $taskRoot "catalog/$taskResource") -Destination $taskResources
}
Copy-Item -LiteralPath (Join-Path $taskRoot 'LICENSE') -Destination $taskResources
New-Item -ItemType Directory -Path (Join-Path $taskResources 'licenses') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $taskRoot 'scripts/licenses/sfextract-MIT.txt') -Destination (Join-Path $taskResources 'licenses')
Write-Output "Flutter Windows candidate: $taskOutput"

$taskEvidence = Join-Path $taskRoot '.artifacts/windows-flutter'
New-Item -ItemType Directory -Path $taskEvidence -Force | Out-Null
$taskVersionJson | Set-Content -Encoding utf8 (Join-Path $taskEvidence 'flutter-version.json')
$taskDeps | Set-Content -Encoding utf8 (Join-Path $taskEvidence 'pub-deps.json')
$taskFlutterRoot = Split-Path -Parent (Split-Path -Parent (Resolve-Path $Flutter).Path)
Copy-Item -LiteralPath (Join-Path $taskFlutterRoot 'LICENSE') -Destination (Join-Path $taskResources 'licenses/Flutter-LICENSE')
Copy-Item -LiteralPath (Join-Path $taskFlutterRoot 'bin/cache/dart-sdk/LICENSE') -Destination (Join-Path $taskResources 'licenses/Dart-LICENSE')
& python (Join-Path $PSScriptRoot 'update_manifest.py') $taskOutput
if ($LASTEXITCODE -ne 0) { throw 'Update manifest generation failed' }
