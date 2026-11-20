param(
  [string]$Jdk = $env:JAVA_HOME,
  [string]$Destination = (Join-Path $PSScriptRoot "unidbg-out")
)

$ErrorActionPreference = "Stop"
if (-not $Jdk) {
  throw "请设置 JAVA_HOME，或传入 -Jdk <JDK 目录>"
}
$javac = Join-Path $Jdk "bin\javac.exe"
if (-not (Test-Path $javac)) {
  throw "找不到 javac：$javac"
}
$lib = Join-Path $PSScriptRoot "unidbg-libs"
if (-not (Test-Path (Join-Path $lib "unidbg-android-0.9.8.jar"))) {
  throw "缺少 unidbg-libs，请先运行 spider-host/fetch-unidbg.ps1"
}
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
& $javac -encoding UTF-8 -source 17 -target 17 -cp (Join-Path $lib "*") -d $Destination (Join-Path $PSScriptRoot "src\main\java\host\FishGuardBridge.java")
if ($LASTEXITCODE -ne 0) {
  throw "javac 失败，退出码 $LASTEXITCODE"
}
Write-Host "[ok] compiled unidbg bridge to $Destination"
