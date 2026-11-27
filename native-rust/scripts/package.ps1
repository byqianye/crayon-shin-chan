[CmdletBinding()]
param(
    [string]$OutputDirectory = '',
    [string]$Target = '',
    [string]$MakeNSIS = $env:MAKENSIS_PATH,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$taskRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$taskManifest = Join-Path $taskRoot 'native-rust/Cargo.toml'
$taskVersion = (Get-Content -LiteralPath (Join-Path $taskRoot 'package.json') -Raw | ConvertFrom-Json).version
$taskCargoVersion = [regex]::Match((Get-Content -LiteralPath $taskManifest -Raw), '(?m)^version = "([^"]+)"').Groups[1].Value
if ($taskVersion -ne $taskCargoVersion) { throw 'package.json and Cargo.toml versions differ' }
if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot 'build.ps1') -Profile release -Target $Target
}
$taskMetadataRaw = & cargo metadata --locked --format-version 1 --manifest-path $taskManifest
if ($LASTEXITCODE -ne 0) { throw 'Cannot read Cargo metadata' }
$taskMetadata = $taskMetadataRaw | ConvertFrom-Json
$taskTargetDirectory = $taskMetadata.target_directory
if (-not $Target) { $Target = $env:CARGO_BUILD_TARGET }
if ($Target) { $taskTargetDirectory = Join-Path $taskTargetDirectory $Target }
if ($Target -and $Target -notmatch '^x86_64-pc-windows-') { throw 'Only Windows x64 packages are supported' }
$taskExecutable = Join-Path $taskTargetDirectory 'release/crayon-rust.exe'
if (-not (Test-Path -LiteralPath $taskExecutable)) { throw "Missing executable: $taskExecutable" }
$taskFileVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo($taskExecutable).ProductVersion
if ($taskFileVersion -ne $taskVersion) { throw "Executable version mismatch: $taskFileVersion / $taskVersion" }

if (-not $MakeNSIS) { $MakeNSIS = (Get-Command makensis -ErrorAction SilentlyContinue).Source }
if (-not $MakeNSIS) {
    $taskCache = Join-Path $env:LOCALAPPDATA 'electron-builder/Cache'
    $MakeNSIS = Get-ChildItem -LiteralPath $taskCache -Filter makensis.exe -Recurse -ErrorAction SilentlyContinue |
        Sort-Object FullName | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $MakeNSIS -or -not (Test-Path -LiteralPath $MakeNSIS)) { throw 'Install NSIS or set MAKENSIS_PATH' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $taskRoot 'release/rust' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$taskStage = Join-Path ([IO.Path]::GetTempPath()) ('crayon-rust-package-' + [guid]::NewGuid().ToString('N'))
$taskPayload = Join-Path $taskStage 'payload'
New-Item -ItemType Directory -Path $taskPayload | Out-Null
$taskEncoding = New-Object Text.UTF8Encoding($false)
try {
    Copy-Item -LiteralPath $taskExecutable -Destination $taskPayload
    $taskLoader = Join-Path (Split-Path $taskExecutable -Parent) 'WebView2Loader.dll'
    if (Test-Path -LiteralPath $taskLoader) { Copy-Item -LiteralPath $taskLoader -Destination $taskPayload }
    Copy-Item -LiteralPath (Join-Path $taskRoot 'build/icon.ico') -Destination $taskPayload
    Copy-Item -LiteralPath (Join-Path $taskRoot 'LICENSE'), (Join-Path $taskRoot 'README.md') -Destination $taskPayload
    $taskSpider = Join-Path $taskPayload 'spider-host'
    New-Item -ItemType Directory -Path $taskSpider | Out-Null
    foreach ($taskPart in @('out', 'lib', 'runtime', 'dex-tools', 'unidbg-libs', 'unidbg-out')) {
        $taskSource = Join-Path $taskRoot "spider-host/$taskPart"
        if (-not (Test-Path -LiteralPath $taskSource -PathType Container)) { throw "Missing Spider runtime: $taskPart" }
        Copy-Item -LiteralPath $taskSource -Destination $taskSpider -Recurse
    }
    foreach ($taskRequired in @('out/host/SpiderHost.class', 'runtime/bin/java.exe', 'lib/android-4.1.1.4.jar', 'lib/json-20090211.jar')) {
        if (-not (Test-Path -LiteralPath (Join-Path $taskSpider $taskRequired) -PathType Leaf)) { throw "Missing Spider file: $taskRequired" }
    }
    if (-not (Get-ChildItem -LiteralPath (Join-Path $taskSpider 'dex-tools') -Recurse -Filter d2j-dex2jar.bat)) { throw 'Missing dex2jar launcher' }
    if (-not (Test-Path -LiteralPath (Join-Path $taskSpider 'runtime/legal'))) { throw 'Missing Java legal notices' }

    $taskLicenses = Join-Path $taskPayload 'licenses'
    New-Item -ItemType Directory -Path $taskLicenses | Out-Null
    $taskNotices = New-Object 'Collections.Generic.List[string]'
    $taskNotices.Add('Third-party components distributed with Crayon Shin Chan (Rust).')
    $taskNotices.Add('Java runtime notices: spider-host/runtime/legal. Jar notices: licenses/java.')
    $taskNotices.Add('Microsoft Edge WebView2 Runtime is a separately installed system requirement.')
    foreach ($taskPackage in ($taskMetadata.packages | Sort-Object name, version)) {
        if ($taskPackage.name -eq 'crayon-rust') { continue }
        $taskNotices.Add("Rust: $($taskPackage.name) $($taskPackage.version) | $($taskPackage.license) | $($taskPackage.repository)")
        $taskPackageRoot = Split-Path -Parent $taskPackage.manifest_path
        $taskLicenseFiles = @(Get-ChildItem -LiteralPath $taskPackageRoot -File | Where-Object Name -Match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)')
        if ($taskPackage.license_file) {
            $taskDeclared = Join-Path $taskPackageRoot $taskPackage.license_file
            if (Test-Path -LiteralPath $taskDeclared -PathType Leaf) { $taskLicenseFiles += Get-Item -LiteralPath $taskDeclared }
        }
        if ($taskLicenseFiles.Count) {
            $taskDestination = Join-Path $taskLicenses "rust/$($taskPackage.name)-$($taskPackage.version)"
            New-Item -ItemType Directory -Path $taskDestination -Force | Out-Null
            $taskLicenseFiles | Sort-Object FullName -Unique | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $taskDestination }
        }
    }
    $taskDependencyRootsRaw = & node -e @'
const { createRequire } = require('node:module');
const { dirname, join } = require('node:path');
const fromRoot = createRequire(join(process.argv[1], 'package.json'));
const fromMpegts = createRequire(fromRoot.resolve('mpegts.js/package.json'));
process.stdout.write(JSON.stringify(Object.fromEntries(
    ['es6-promise', 'events'].map((name) => [name, dirname(fromMpegts.resolve(`${name}/package.json`))])
)));
'@ $taskRoot
    if ($LASTEXITCODE -ne 0) { throw 'Cannot locate mpegts.js dependency licenses' }
    $taskDependencyRoots = $taskDependencyRootsRaw | ConvertFrom-Json
    foreach ($taskName in @('artplayer', 'hls.js', 'mpegts.js', 'shaka-player', 'es6-promise', 'events')) {
        $taskPackageRoot = if ($taskName -in @('es6-promise', 'events')) { $taskDependencyRoots.$taskName } else { Join-Path $taskRoot "node_modules/$taskName" }
        $taskPackage = Get-Content -LiteralPath (Join-Path $taskPackageRoot 'package.json') -Raw | ConvertFrom-Json
        $taskNotices.Add("JavaScript: $taskName $($taskPackage.version) | $($taskPackage.license)")
        $taskDestination = Join-Path $taskLicenses $taskName
        New-Item -ItemType Directory -Path $taskDestination | Out-Null
        $taskLicenseFiles = @(Get-ChildItem -LiteralPath $taskPackageRoot -File | Where-Object Name -Match '^(LICENSE|LICENCE|NOTICE)' | Select-Object -ExpandProperty Name)
        switch ($taskName) {
            'mpegts.js' { $taskLicenseFiles += @('LICENSE', 'dist/mpegts.js.LICENSE.txt') }
            'shaka-player' { $taskLicenseFiles += @('LICENSE', 'third_party/cml-cmcd/LICENSE', 'third_party/cml-cmcd/NOTICE', 'third_party/language-mapping-list/LICENSE') }
            'es6-promise' { $taskLicenseFiles += 'LICENSE' }
            'events' { $taskLicenseFiles += 'LICENSE' }
        }
        foreach ($taskRelative in ($taskLicenseFiles | Sort-Object -Unique)) {
            $taskSource = Join-Path $taskPackageRoot $taskRelative
            if (-not (Test-Path -LiteralPath $taskSource -PathType Leaf)) { throw "Missing JavaScript license: $taskName/$taskRelative" }
            $taskLicensePath = Join-Path $taskDestination $taskRelative
            New-Item -ItemType Directory -Path (Split-Path -Parent $taskLicensePath) -Force | Out-Null
            Copy-Item -LiteralPath $taskSource -Destination $taskLicensePath
        }
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $taskJars = Get-ChildItem -LiteralPath $taskSpider -Filter *.jar -File -Recurse
    foreach ($taskJar in $taskJars) {
        $taskNotices.Add("Java: $($taskJar.Name)")
        $taskArchive = [IO.Compression.ZipFile]::OpenRead($taskJar.FullName)
        try {
            foreach ($taskEntry in $taskArchive.Entries) {
                if ($taskEntry.Name -match '^(LICENSE|LICENCE|NOTICE|COPYING)([._-].*)?$') {
                    $taskDestination = Join-Path $taskLicenses "java/$($taskJar.BaseName)"
                    New-Item -ItemType Directory -Path $taskDestination -Force | Out-Null
                    $taskLicensePath = Join-Path $taskDestination ($taskEntry.FullName -replace '[/\\:]', '_')
                    [IO.Compression.ZipFileExtensions]::ExtractToFile($taskEntry, $taskLicensePath, $true)
                }
            }
        } finally { $taskArchive.Dispose() }
    }
    $taskNSISLicense = & $MakeNSIS /LICENSE
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read NSIS license' }
    [IO.File]::WriteAllLines((Join-Path $taskLicenses 'NSIS.txt'), @($taskNSISLicense), $taskEncoding)
    [IO.File]::WriteAllLines((Join-Path $taskPayload 'THIRD-PARTY-NOTICES.txt'), $taskNotices, $taskEncoding)
    $taskVersionInfo = @{ version = $taskVersion; runtime = 'rust-webview2'; architecture = 'x64'; executable = 'crayon-rust.exe'; spider = $true }
    [IO.File]::WriteAllText((Join-Path $taskPayload 'version.json'), ($taskVersionInfo | ConvertTo-Json), $taskEncoding)
    $taskManifestLines = Get-ChildItem -LiteralPath $taskPayload -Recurse -File | Sort-Object FullName | ForEach-Object {
        $taskRelative = $_.FullName.Substring($taskPayload.Length + 1).Replace('\', '/')
        (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + $taskRelative
    }
    [IO.File]::WriteAllLines((Join-Path $taskPayload 'FILES.sha256'), $taskManifestLines, $taskEncoding)
    if (Test-Path -LiteralPath (Join-Path $taskPayload 'data')) { throw 'User data must never enter the payload' }

    $taskInstaller = Join-Path $OutputDirectory "CrayonShinChan-$taskVersion-rust-x64.exe"
    & $MakeNSIS /INPUTCHARSET UTF8 /WX /V2 "/DVERSION=$taskVersion" "/DPAYLOAD=$taskPayload" "/DOUTPUT=$taskInstaller" (Join-Path $PSScriptRoot 'installer.nsi')
    if ($LASTEXITCODE -ne 0) { throw "NSIS failed: $LASTEXITCODE" }
    $taskZip = Join-Path $OutputDirectory "CrayonShinChan-$taskVersion-rust-x64.zip"
    if (Test-Path -LiteralPath $taskZip) { Remove-Item -LiteralPath $taskZip }
    [IO.Compression.ZipFile]::CreateFromDirectory($taskPayload, $taskZip, [IO.Compression.CompressionLevel]::Optimal, $false)
    & (Join-Path $PSScriptRoot 'verify-package.ps1') -Path $taskZip -Version $taskVersion
    $taskChecksums = @($taskInstaller, $taskZip) | ForEach-Object {
        (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + [IO.Path]::GetFileName($_)
    }
    [IO.File]::WriteAllLines((Join-Path $OutputDirectory 'SHA256SUMS-rust.txt'), $taskChecksums, $taskEncoding)
    Write-Output "[PASS] Rust installer: $taskInstaller"
    Write-Output "[PASS] Rust portable: $taskZip"
    Write-Output '[PASS] Spider runtime, licenses and SHA256 manifests included; no user data'
} finally {
    $taskResolvedStage = [IO.Path]::GetFullPath($taskStage)
    $taskTempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if (-not $taskResolvedStage.StartsWith($taskTempRoot, [StringComparison]::OrdinalIgnoreCase) -or
        (Split-Path -Leaf $taskResolvedStage) -notlike 'crayon-rust-package-*') { throw 'Refusing unsafe staging cleanup' }
    if (Test-Path -LiteralPath $taskResolvedStage) { Remove-Item -LiteralPath $taskResolvedStage -Recurse -Force }
}
