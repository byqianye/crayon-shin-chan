[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$taskArchive = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $Path).Path)
try {
    $taskFiles = @($taskArchive.Entries | Where-Object { $_.Name })
    $taskNames = @{}
    foreach ($taskEntry in $taskFiles) {
        $taskName = $taskEntry.FullName.Replace('\', '/')
        if ($taskName -match '(^/|^[A-Za-z]:|(^|/)\.\.(/|$)|(^|/)data/|(^|/)app\.asar$|(^|/)electron\.exe$)') { throw "Unexpected payload entry: $taskName" }
        if ($taskNames.ContainsKey($taskName)) { throw "Duplicate payload entry: $taskName" }
        $taskNames[$taskName] = $taskEntry
    }
    foreach ($taskRequired in @('crayon-rust.exe', 'version.json', 'FILES.sha256', 'THIRD-PARTY-NOTICES.txt', 'LICENSE', 'spider-host/out/host/SpiderHost.class', 'spider-host/runtime/bin/java.exe', 'spider-host/lib/android-4.1.1.4.jar', 'spider-host/lib/json-20090211.jar')) {
        if (-not $taskNames.ContainsKey($taskRequired)) { throw "Missing payload entry: $taskRequired" }
    }
    foreach ($taskRequiredPattern in @('^spider-host/dex-tools/.+/d2j-dex2jar\.bat$', '^spider-host/unidbg-libs/.+\.jar$', '^spider-host/unidbg-out/.+\.class$', '^spider-host/runtime/legal/.+', '^licenses/.+')) {
        if (-not ($taskNames.Keys -match $taskRequiredPattern)) { throw "Incomplete payload: $taskRequiredPattern" }
    }
    $taskReader = New-Object IO.StreamReader($taskNames['version.json'].Open())
    try { $taskInfo = $taskReader.ReadToEnd() | ConvertFrom-Json } finally { $taskReader.Dispose() }
    if ($taskInfo.version -ne $Version -or $taskInfo.runtime -ne 'rust-webview2' -or $taskInfo.architecture -ne 'x64' -or $taskInfo.spider -ne $true) { throw 'Package version/runtime mismatch' }
    $taskReader = New-Object IO.StreamReader($taskNames['FILES.sha256'].Open())
    try { $taskLines = $taskReader.ReadToEnd() -split '\r?\n' | Where-Object { $_ } } finally { $taskReader.Dispose() }
    $taskHashed = @{}
    foreach ($taskLine in $taskLines) {
        $taskMatch = [regex]::Match($taskLine, '^([a-f0-9]{64})  (.+)$')
        if (-not $taskMatch.Success) { throw 'Invalid payload manifest line' }
        $taskName = $taskMatch.Groups[2].Value
        if (-not $taskNames.ContainsKey($taskName) -or $taskHashed.ContainsKey($taskName)) { throw "Missing/duplicate manifest file: $taskName" }
        $taskStream = $taskNames[$taskName].Open()
        $taskHasher = [Security.Cryptography.SHA256]::Create()
        try { $taskHash = [BitConverter]::ToString($taskHasher.ComputeHash($taskStream)).Replace('-', '').ToLowerInvariant() }
        finally { $taskStream.Dispose(); $taskHasher.Dispose() }
        if ($taskHash -ne $taskMatch.Groups[1].Value) { throw "Payload checksum mismatch: $taskName" }
        $taskHashed[$taskName] = $true
    }
    if ($taskHashed.Count -ne $taskFiles.Count - 1) { throw 'Payload contains unlisted files' }
    Write-Output "[PASS] Rust $Version ZIP: $($taskHashed.Count) hashes, complete Spider, licenses, no user data/Electron"
} finally { $taskArchive.Dispose() }
