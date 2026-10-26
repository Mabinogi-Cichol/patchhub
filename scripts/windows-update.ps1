[CmdletBinding()]
param(
    [ValidateSet('Apply','Recover','Uninstall')][string]$Mode = 'Apply',
    [Parameter(Mandatory=$true)][string]$Target,
    [string]$Work = '',
    [int]$ParentId = 0
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$taskTarget = [IO.Path]::GetFullPath($Target).TrimEnd('\','/')
# Resolve existing short-name ancestors before appending stage/backup paths.
if ($Work) { $Work = [IO.Path]::GetFullPath($Work).TrimEnd('\','/') }
$taskMarker = Join-Path $taskTarget '.patchhub-update.json'
$taskLockPath = Join-Path $taskTarget '.patchhub-maintenance.lock'
$taskLock = $null
$taskChild = $null
$taskJournal = $null

function Write-Json($Path, $Value) {
    $taskTemp = "$Path.tmp"
    [IO.File]::WriteAllText($taskTemp, ($Value | ConvertTo-Json -Depth 20), [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $taskTemp -Destination $Path -Force
}
function Safe-Path($Root, [string]$Relative) {
    $Root = [IO.Path]::GetFullPath($Root).TrimEnd('\','/')
    if (-not $Relative -or $Relative.Contains('\') -or $Relative.Contains(':') -or
        $Relative.StartsWith('/') -or ($Relative.Split('/') | Where-Object { $_ -in @('','..','.') }) -or
        $Relative -match '[<>"|?*]' -or $Relative.Split('/')[0] -in @('install.ini','uninstall.exe','.patchhub-update.json','.patchhub-maintenance.lock')) {
        throw "Invalid managed path: $Relative"
    }
    $taskPath = [IO.Path]::GetFullPath((Join-Path $Root $Relative))
    if (-not $taskPath.StartsWith($Root.TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Path escapes target' }
    $taskCursor = $taskPath
    while ($taskCursor -and $taskCursor.Length -ge $Root.Length) {
        if ((Test-Path -LiteralPath $taskCursor) -and ((Get-Item -LiteralPath $taskCursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Linked update path: $taskCursor" }
        $taskCursor = Split-Path -Parent $taskCursor
    }
    return $taskPath
}
function Read-Manifest($Root) {
    $taskManifest = Get-Content -LiteralPath (Join-Path $Root 'update-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($taskManifest.schema -ne 1 -or $taskManifest.version -notmatch '^\d+\.\d+\.\d+([+-].+)?$') { throw 'Invalid update manifest' }
    $taskSeen = @{}
    foreach ($taskName in $taskManifest.files) {
        $null = Safe-Path $Root $taskName
        if ($taskSeen.ContainsKey($taskName)) { throw "Duplicate managed path: $taskName" }
        $taskSeen[$taskName] = $true
    }
    foreach ($taskRequired in @('patchhub.exe','patchhub-core.exe','patchhub-worker.exe','windows-update.ps1','update-manifest.json')) {
        if (-not $taskSeen.ContainsKey($taskRequired)) { throw "Missing managed file: $taskRequired" }
    }
    return $taskManifest
}
function Assert-Idle {
    foreach ($taskProcess in Get-Process -Name patchhub,patchhub-core,patchhub-worker -ErrorAction SilentlyContinue) {
        if ($taskProcess.Path -and [IO.Path]::GetDirectoryName($taskProcess.Path).Equals($taskTarget, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Application still running in target directory'
        }
    }
}
function Restore-Update {
    Assert-Idle
    foreach ($taskEntry in $taskJournal.entries) {
        $taskPath = Safe-Path $taskTarget $taskEntry.path
        $taskBackup = Safe-Path (Join-Path $Work 'backup') $taskEntry.path
        if ($taskEntry.existed) {
            New-Item -ItemType Directory -Path (Split-Path -Parent $taskPath) -Force | Out-Null
            Copy-Item -LiteralPath $taskBackup -Destination $taskPath -Force
        } elseif (Test-Path -LiteralPath $taskPath) { Remove-Item -LiteralPath $taskPath -Force }
    }
    if ($taskJournal.registry) {
        Set-ItemProperty -LiteralPath $taskJournal.registry.path -Name DisplayVersion -Value $taskJournal.registry.version
    }
    $taskJournal.phase = 'restored'
    Write-Json (Join-Path $Work 'journal.json') $taskJournal
}
function Clear-Transaction {
    # Work is bound to the persisted transaction; never recursively remove Target.
    $taskResolvedWork = [IO.Path]::GetFullPath($Work)
    if ($taskResolvedWork.Equals($taskTarget, [StringComparison]::OrdinalIgnoreCase) -or
        $taskTarget.StartsWith($taskResolvedWork.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe transaction cleanup path' }
    foreach ($taskSub in @('stage','backup')) {
        $taskSubPath = Join-Path $taskResolvedWork $taskSub
        if (Test-Path -LiteralPath $taskSubPath) { Remove-Item -LiteralPath $taskSubPath -Recurse -Force }
    }
    foreach ($taskName in @('package.zip','ready.json','commit.json')) {
        $taskFile = Join-Path $taskResolvedWork $taskName
        if (Test-Path -LiteralPath $taskFile) { Remove-Item -LiteralPath $taskFile -Force }
    }
    if (Test-Path -LiteralPath $taskMarker) {
        $taskSavedMarker = Get-Content -LiteralPath $taskMarker -Raw -Encoding UTF8 | ConvertFrom-Json
        if ([IO.Path]::GetFullPath($taskSavedMarker.work).TrimEnd('\','/') -eq $Work -and
            [IO.Path]::GetFullPath($taskSavedMarker.target).TrimEnd('\','/') -eq $taskTarget) { Remove-Item -LiteralPath $taskMarker -Force }
    }
}
function Finish-Transaction($Result) {
    Write-Json (Join-Path (Split-Path -Parent $Work) 'last-update-result.json') (@{work=$Work; target=$taskTarget} + $Result)
    Clear-Transaction
    foreach ($taskName in @('request.json','prepared.json','apply.json','abort.json','probe.json','ack.json','journal.json','result.json','windows-update.ps1')) {
        $taskFile = Join-Path $Work $taskName
        if (Test-Path -LiteralPath $taskFile) { Remove-Item -LiteralPath $taskFile -Force }
    }
    if (-not (Get-ChildItem -LiteralPath $Work -Force | Select-Object -First 1)) { Remove-Item -LiteralPath $Work }
}

try {
    $taskLock = [IO.File]::Open($taskLockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    if ($Mode -eq 'Uninstall') {
        if (Test-Path -LiteralPath $taskMarker) { throw 'Recover pending update before uninstalling' }
        Assert-Idle
        $taskManifest = Read-Manifest $taskTarget
        foreach ($taskName in $taskManifest.files) {
            $taskPath = Safe-Path $taskTarget $taskName
            if (Test-Path -LiteralPath $taskPath) { Remove-Item -LiteralPath $taskPath -Force }
        }
        # Remove only empty parents of managed files; extra user files survive.
        $taskDirectories = @($taskManifest.files | ForEach-Object {
            $taskDirectory = Split-Path -Parent (Safe-Path $taskTarget $_)
            while ($taskDirectory -and $taskDirectory.Length -gt $taskTarget.Length) {
                $taskDirectory
                $taskDirectory = Split-Path -Parent $taskDirectory
            }
        } | Sort-Object -Unique | Sort-Object Length -Descending)
        foreach ($taskDirectory in $taskDirectories) {
            if ((Test-Path -LiteralPath $taskDirectory) -and -not (Get-ChildItem -LiteralPath $taskDirectory -Force | Select-Object -First 1)) { Remove-Item -LiteralPath $taskDirectory }
        }
        exit 0
    }
    if ($Mode -eq 'Recover') {
        if ($ParentId) {
            $taskParent = Get-Process -Id $ParentId -ErrorAction SilentlyContinue
            if ($taskParent) { $taskParent.WaitForExit() }
        }
        $taskSaved = Get-Content -LiteralPath $taskMarker -Raw -Encoding UTF8 | ConvertFrom-Json
        if ([IO.Path]::GetFullPath($taskSaved.target).TrimEnd('\','/') -ne $taskTarget) { throw 'Transaction target mismatch' }
        $Work = [IO.Path]::GetFullPath($taskSaved.work).TrimEnd('\','/')
        $taskJournal = Get-Content -LiteralPath (Join-Path $Work 'journal.json') -Raw -Encoding UTF8 | ConvertFrom-Json
        if ([IO.Path]::GetFullPath($taskJournal.target).TrimEnd('\','/') -ne $taskTarget) { throw 'Journal target mismatch' }
        if ($taskJournal.phase -notin @('committed','restored')) { Restore-Update }
        Finish-Transaction @{state=$taskJournal.phase; message='Update recovery completed'}
        if ($ParentId) { Start-Process -FilePath (Join-Path $taskTarget 'patchhub.exe') -WindowStyle Hidden }
        exit 0
    }
    $taskRequest = Get-Content -LiteralPath (Join-Path $Work 'request.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([IO.Path]::GetFullPath($taskRequest.target).TrimEnd('\','/') -ne $taskTarget) { throw 'Request target mismatch' }
    $taskPackage = Join-Path $Work 'package.zip'
    $taskHashStream = [IO.File]::OpenRead($taskPackage)
    $taskHasher = [Security.Cryptography.SHA256]::Create()
    try { $taskHash = [BitConverter]::ToString($taskHasher.ComputeHash($taskHashStream)).Replace('-','').ToLowerInvariant() }
    finally { $taskHashStream.Dispose(); $taskHasher.Dispose() }
    if ($taskHash -ne $taskRequest.sha256) { throw 'ZIP SHA-256 mismatch' }
    $taskStage = Join-Path $Work 'stage'
    if (Test-Path -LiteralPath $taskStage) { throw 'Transaction stage already exists' }
    New-Item -ItemType Directory -Path $taskStage | Out-Null
    $taskArchive = [IO.Compression.ZipFile]::OpenRead($taskPackage)
    try {
        $taskSeen = @{}
        foreach ($taskEntry in $taskArchive.Entries) {
            $taskName = $taskEntry.FullName
            if ($taskName.EndsWith('/')) { $null = Safe-Path $taskStage $taskName.TrimEnd('/'); continue }
            $taskPath = Safe-Path $taskStage $taskName
            if ($taskSeen.ContainsKey($taskName) -or (($taskEntry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000) { throw 'Duplicate or linked ZIP entry' }
            $taskSeen[$taskName] = $true
            New-Item -ItemType Directory -Path (Split-Path -Parent $taskPath) -Force | Out-Null
            [IO.Compression.ZipFileExtensions]::ExtractToFile($taskEntry, $taskPath)
        }
    } finally { $taskArchive.Dispose() }
    $taskOld = Read-Manifest $taskTarget
    $taskNew = Read-Manifest $taskStage
    if ($taskNew.version -ne $taskRequest.version) { throw 'Package version does not match frozen Release' }
    if (@($taskNew.files).Count -ne $taskSeen.Count) { throw 'Unmanaged ZIP entries' }
    foreach ($taskName in $taskNew.files) {
        if (-not $taskSeen.ContainsKey($taskName)) { throw "Missing ZIP entry: $taskName" }
    }
    Write-Json (Join-Path $Work 'prepared.json') @{state='prepared'}
    # GUI explicitly permits application only after all core work has stopped.
    while (-not (Test-Path -LiteralPath (Join-Path $Work 'apply.json'))) {
        if (Test-Path -LiteralPath (Join-Path $Work 'abort.json')) { throw 'Update cancelled before application' }
        if ($ParentId -and -not (Get-Process -Id $ParentId -ErrorAction SilentlyContinue)) { throw 'Application exited before authorizing update' }
        Start-Sleep -Milliseconds 100
    }
    if ($ParentId) {
        $taskParent = Get-Process -Id $ParentId -ErrorAction SilentlyContinue
        if ($taskParent) {
            while (-not $taskParent.WaitForExit(100)) {
                if (Test-Path -LiteralPath (Join-Path $Work 'abort.json')) { throw 'Application cancelled update before exiting' }
            }
        }
    }
    if (Test-Path -LiteralPath (Join-Path $Work 'abort.json')) { throw 'Application cancelled update' }
    Assert-Idle
    $taskEntries = @()
    foreach ($taskName in @(@($taskOld.files) + @($taskNew.files) | Sort-Object -Unique)) {
        $taskPath = Safe-Path $taskTarget $taskName
        $taskExists = Test-Path -LiteralPath $taskPath
        if ($taskExists -and $taskName -notin $taskOld.files) { throw "New file conflicts with unmanaged user file: $taskName" }
        $taskEntries += @{path=$taskName; existed=$taskExists}
        if ($taskExists) {
            $taskBackup = Safe-Path (Join-Path $Work 'backup') $taskName
            New-Item -ItemType Directory -Path (Split-Path -Parent $taskBackup) -Force | Out-Null
            Copy-Item -LiteralPath $taskPath -Destination $taskBackup
        }
    }
    $taskRegistry = $null
    $taskIni = Join-Path $taskTarget 'install.ini'
    if (Test-Path -LiteralPath $taskIni) {
        $taskProduct = [regex]::Match((Get-Content -LiteralPath $taskIni -Raw -Encoding UTF8), '(?m)^ProductName=(.+)\r?$').Groups[1].Value.Trim()
        if ($taskProduct -ne 'PatchHub') { throw 'Invalid installed product identity' }
        $taskKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$taskProduct"
        $taskValues = Get-ItemProperty -LiteralPath $taskKey
        if ([IO.Path]::GetFullPath($taskValues.InstallLocation).TrimEnd('\') -ne $taskTarget) { throw 'Installed location mismatch' }
        $taskRegistry = @{path=$taskKey; version=$taskValues.DisplayVersion}
    }
    $taskJournal = @{target=$taskTarget; phase='applying'; entries=$taskEntries; registry=$taskRegistry}
    Write-Json (Join-Path $Work 'journal.json') $taskJournal
    Write-Json $taskMarker @{target=$taskTarget; work=$Work}
    foreach ($taskName in $taskOld.files) {
        if ($taskName -notin $taskNew.files) { Remove-Item -LiteralPath (Safe-Path $taskTarget $taskName) -Force -ErrorAction Stop }
    }
    foreach ($taskName in $taskNew.files) {
        if ($taskName -eq 'update-manifest.json') { continue }
        $taskPath = Safe-Path $taskTarget $taskName
        New-Item -ItemType Directory -Path (Split-Path -Parent $taskPath) -Force | Out-Null
        Copy-Item -LiteralPath (Safe-Path $taskStage $taskName) -Destination $taskPath -Force
    }
    $taskProbe = Join-Path $Work 'probe.json'
    Write-Json $taskProbe @{target=$taskTarget; version=$taskNew.version; work=$Work}
    $taskChild = Start-Process -FilePath (Join-Path $taskTarget 'patchhub.exe') -ArgumentList @('--update-probe', ('"' + $taskProbe + '"')) -PassThru -WindowStyle Hidden
    while (-not (Test-Path -LiteralPath (Join-Path $Work 'ready.json'))) {
        if ($taskChild.HasExited) { throw "Updated client initialization failed: $($taskChild.ExitCode)" }
        Start-Sleep -Milliseconds 100
    }
    $taskReady = Get-Content -LiteralPath (Join-Path $Work 'ready.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($taskReady.version -ne $taskNew.version -or $taskReady.pid -ne $taskChild.Id) { throw 'Update probe identity mismatch' }
    Copy-Item -LiteralPath (Join-Path $taskStage 'update-manifest.json') -Destination (Join-Path $taskTarget 'update-manifest.json') -Force
    if ($taskRegistry) { Set-ItemProperty -LiteralPath $taskRegistry.path -Name DisplayVersion -Value $taskNew.version }
    $taskJournal.phase = 'committed'
    Write-Json (Join-Path $Work 'journal.json') $taskJournal
    Write-Json (Join-Path $Work 'commit.json') @{state='committed'}
    # Retain commit until the new process acknowledges it, so cleanup cannot race it.
    while (-not (Test-Path -LiteralPath (Join-Path $Work 'ack.json'))) {
        if ($taskChild.HasExited) { throw 'Updated client exited before commit acknowledgement' }
        Start-Sleep -Milliseconds 100
    }
    Finish-Transaction @{state='completed'; version=$taskNew.version}
} catch {
    $taskFailure = $_.ToString()
    if ($taskJournal -and $taskJournal.phase -eq 'applying') {
        # A live probe owns executable files. It sees abort and exits normally.
        if ($taskChild -and -not $taskChild.HasExited) {
            Write-Json (Join-Path $Work 'abort.json') @{error=$taskFailure}
            $taskChild.WaitForExit()
        }
        try { Restore-Update } catch { $taskFailure += "; Recovery incomplete: $_" }
    }
    if ($Work -and (Test-Path -LiteralPath $Work)) {
        if (-not $taskJournal -or $taskJournal.phase -in @('restored','committed')) {
            Finish-Transaction @{state='failed'; error=$taskFailure}
            if ($ParentId -and $taskJournal -and $taskJournal.phase -eq 'restored') { Start-Process -FilePath (Join-Path $taskTarget 'patchhub.exe') -WindowStyle Hidden }
        } else { Write-Json (Join-Path $Work 'result.json') @{state='failed'; error=$taskFailure} }
    }
    [Console]::Error.WriteLine($taskFailure)
    exit 1
} finally {
    if ($taskLock) {
        $taskLock.Dispose()
        # Leave the empty lock inode in place. Deleting after release can race a
        # second installer that has just opened it; uninstall removes it last.
        if ($Mode -eq 'Uninstall') { Remove-Item -LiteralPath $taskLockPath -Force -ErrorAction SilentlyContinue }
    }
}
