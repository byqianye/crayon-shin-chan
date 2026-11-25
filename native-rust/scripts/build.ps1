[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$Profile = 'debug',
    [string]$Target = '',
    [switch]$Run,
    [string]$DataDir = ''
)

$ErrorActionPreference = 'Stop'
$taskManifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Cargo.toml'
$taskArguments = @('build', '--locked', '--manifest-path', $taskManifest, '--bin', 'crayon-rust', '--message-format=json-render-diagnostics')
if ($Profile -eq 'release') { $taskArguments += '--release' }
if ($Target) { $taskArguments += @('--target', $Target) }

$taskMessages = & cargo @taskArguments
$taskBuildExit = $LASTEXITCODE
$taskEvents = @($taskMessages | ForEach-Object { $_ | ConvertFrom-Json })
foreach ($taskEvent in $taskEvents) {
    if ($taskEvent.reason -eq 'compiler-message' -and $taskEvent.message.rendered) {
        [Console]::Error.WriteLine($taskEvent.message.rendered)
    }
}
if ($taskBuildExit -ne 0) { throw "cargo build failed ($taskBuildExit)" }
$taskArtifacts = @($taskEvents | Where-Object {
    $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'crayon-rust' -and $_.executable
})
if ($taskArtifacts.Count -ne 1) { throw 'Expected one desktop executable from cargo' }
$taskExecutable = $taskArtifacts[0].executable

if (-not $Target) {
    if ($env:CARGO_BUILD_TARGET) {
        $Target = $env:CARGO_BUILD_TARGET
    } else {
        $taskHost = & rustc -vV
        if ($LASTEXITCODE -ne 0) { throw 'Cannot identify Rust host target' }
        $Target = ($taskHost | Select-String '^host: ').ToString().Substring(6)
    }
}

if ($Target -match '-windows-gnu') {
    $taskMetadataJson = & cargo metadata --locked --format-version 1 --manifest-path $taskManifest
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read resolved Cargo dependencies' }
    $taskMetadata = $taskMetadataJson | ConvertFrom-Json
    $taskPackages = @($taskMetadata.packages | Where-Object name -eq 'webview2-com-sys')
    if ($taskPackages.Count -ne 1) { throw 'Expected one resolved WebView2 loader package' }
    $taskArchitecture = switch -Regex ($Target) {
        '^x86_64-' { 'x64'; break }
        '^i[3-6]86-' { 'x86'; break }
        '^aarch64-' { 'arm64'; break }
        default { throw "Unsupported WebView2 architecture: $Target" }
    }
    $taskPackageRoot = Split-Path -Parent $taskPackages[0].manifest_path
    $taskLoader = Join-Path $taskPackageRoot "$taskArchitecture/WebView2Loader.dll"
    $taskDestination = Join-Path (Split-Path -Parent $taskExecutable) 'WebView2Loader.dll'
    Copy-Item -LiteralPath $taskLoader -Destination $taskDestination -Force
    if ((Get-FileHash -LiteralPath $taskLoader).Hash -ne (Get-FileHash -LiteralPath $taskDestination).Hash) {
        throw 'WebView2 loader verification failed'
    }
    Write-Output "[PASS] WebView2Loader.dll: $taskDestination"
}

Write-Output "[PASS] Desktop executable: $taskExecutable"
if ($Run) {
    $taskRunArguments = @()
    if ($DataDir) { $taskRunArguments += @('--data-dir', $DataDir) }
    & $taskExecutable @taskRunArguments
    if ($LASTEXITCODE -ne 0) { throw "Desktop exited with code $LASTEXITCODE" }
}
