[CmdletBinding()]
param([string]$Makensis = '', [string]$Version = '')
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$taskBundle = Join-Path $taskRoot 'apps/windows-flutter/build/windows/x64/runner/Release'
$taskPython = Get-Command python,py -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $taskPython) { throw 'Python 3.11 or newer is required (python or py)' }
[string[]]$taskPythonArgs = if ($taskPython.Name -eq 'py.exe') { @('-3') } else { @() }
$taskVersion = & $taskPython.Source @taskPythonArgs (Join-Path $PSScriptRoot 'project_version.py')
if ($LASTEXITCODE -ne 0) { throw 'Unable to read application version' }
if ($Version -and $Version -ne $taskVersion) { throw 'NSIS version differs from Cargo.toml' }
$Version = $taskVersion
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

foreach ($taskLicense in @('Lua-5.4-LICENSE.txt','mlua-MIT.txt')) {
    Copy-Item -LiteralPath (Join-Path $taskRoot "scripts/licenses/$taskLicense") -Destination (Join-Path $taskBundle 'licenses')
}
& $taskPython.Source @taskPythonArgs (Join-Path $PSScriptRoot 'update_manifest.py') $taskBundle
if ($LASTEXITCODE -ne 0) { throw 'Update manifest generation failed' }
if ((Get-Content -LiteralPath (Join-Path $taskBundle 'app-version.json') -Raw | ConvertFrom-Json).version -ne $Version) { throw 'NSIS version differs from application' }
$taskInstaller = Join-Path $taskArtifacts 'patchhub-windows-x64-setup.exe'
& $Makensis /V2 /INPUTCHARSET UTF8 "/DBUNDLE=$taskBundle" "/DVERSION=$Version" "/DOUTPUT=$taskInstaller" (Join-Path $PSScriptRoot 'windows-installer.nsi')
if ($LASTEXITCODE -ne 0) { throw "NSIS failed: $LASTEXITCODE" }
Get-FileHash -LiteralPath $taskInstaller -Algorithm SHA256
