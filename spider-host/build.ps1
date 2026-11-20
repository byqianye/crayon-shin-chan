param(
  [string]$Jdk = $env:JAVA_HOME,
  [string]$Destination = (Join-Path $PSScriptRoot "out")
)

$ErrorActionPreference = "Stop"
if (-not $Jdk) {
  throw "请设置 JAVA_HOME，或传入 -Jdk <JDK 目录>"
}

$javac = Join-Path $Jdk "bin\javac.exe"
if (-not (Test-Path $javac)) {
  throw "找不到 javac：$javac"
}

$lib = Join-Path $PSScriptRoot "lib"
$required = @(
  "okhttp-3.14.9.jar",
  "okio-1.17.5.jar",
  "gson-2.10.1.jar",
  "json-20090211.jar",
  "android-4.1.1.4.jar"
)
if ($required | Where-Object { -not (Test-Path (Join-Path $lib $_)) }) {
  & (Join-Path $PSScriptRoot "fetch-deps.ps1")
}

New-Item -ItemType Directory -Force -Path $Destination | Out-Null
$sources = Get-ChildItem -Path (Join-Path $PSScriptRoot "src\main\java") -Recurse -Filter *.java |
  Where-Object { $_.Name -ne "FishGuardBridge.java" } |
  ForEach-Object { $_.FullName }

& $javac -encoding UTF-8 -source 17 -target 17 -cp (Join-Path $lib "*") -d $Destination $sources
if ($LASTEXITCODE -ne 0) {
  throw "javac 失败，退出码 $LASTEXITCODE"
}
Write-Host "[ok] compiled to $Destination"
