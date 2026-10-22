[CmdletBinding()]
param([string]$Makensis = '', [string]$Version = '0.2.1')
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$taskBundle = Join-Path $taskRoot 'apps/windows-flutter/build/windows/x64/runner/Release'
if (-not $Makensis) {
    $taskCached = Join-Path $env:LOCALAPPDATA 'tauri/NSIS/makensis.exe'
    $Makensis = if (Test-Path -LiteralPath $taskCached) { $taskCached } else { (Get-Command makensis -ErrorAction Stop).Source }
}
$taskArtifacts = Join-Path $taskRoot '.artifacts'
New-Item -ItemType Directory -Path $taskArtifacts -Force | Out-Null
foreach ($taskName in @('dependencies.cdx.json','client-guide.md','privacy.md','LICENSE')) {
    $taskSource = switch ($taskName) {
        'dependencies.cdx.json' { Join-Path $taskArtifacts $taskName }
        'LICENSE' { Join-Path $taskRoot $taskName }
        default { Join-Path $taskRoot "docs/$taskName" }
    }
    Copy-Item -LiteralPath $taskSource -Destination $taskBundle
}
New-Item -ItemType Directory -Path (Join-Path $taskBundle 'licenses') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $taskRoot 'scripts/licenses/sfextract-MIT.txt') -Destination (Join-Path $taskBundle 'licenses')
$taskInclude = Join-Path $taskArtifacts 'flutter-uninstall-files.nsh'
$taskDeletes = @(Get-ChildItem -LiteralPath $taskBundle -Recurse -File | ForEach-Object {
    'Delete "$INSTDIR\' + $_.FullName.Substring($taskBundle.Length + 1) + '"'
})
$taskDeletes += @(Get-ChildItem -LiteralPath $taskBundle -Recurse -Directory | Sort-Object { $_.FullName.Length } -Descending | ForEach-Object {
    'RMDir "$INSTDIR\' + $_.FullName.Substring($taskBundle.Length + 1) + '"'
})
$taskDeletes | Set-Content -Encoding utf8 $taskInclude
$taskInstaller = Join-Path $taskArtifacts "Steam.Hentai.Patch_${Version}_x64-setup.exe"
& $Makensis /V2 "/DBUNDLE=$taskBundle" "/DVERSION=$Version" "/DOUTPUT=$taskInstaller" "/DUNINSTALL_FILES=$taskInclude" (Join-Path $PSScriptRoot 'windows-installer.nsi')
if ($LASTEXITCODE -ne 0) { throw "NSIS failed: $LASTEXITCODE" }
Get-FileHash -LiteralPath $taskInstaller -Algorithm SHA256
