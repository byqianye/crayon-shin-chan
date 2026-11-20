param(
  [string]$Jdk = $env:JAVA_HOME,
  [string]$Destination = (Join-Path $PSScriptRoot "runtime")
)

$ErrorActionPreference = "Stop"
if (-not $Jdk) {
  throw "请设置 JAVA_HOME，或传入 -Jdk <JDK 目录>"
}

$jlink = Join-Path $Jdk "bin\jlink.exe"
if (-not (Test-Path $jlink)) {
  throw "找不到 jlink：$jlink"
}
if (Test-Path (Join-Path $Destination "bin\java.exe")) {
  Write-Host "[skip] runtime 已存在：$Destination"
  exit 0
}

New-Item -ItemType Directory -Force -Path (Split-Path $Destination -Parent) | Out-Null
& $jlink `
  --add-modules java.base,java.logging,java.xml,java.net.http,java.naming,java.security.jgss,java.sql,java.management,jdk.crypto.ec,jdk.unsupported,jdk.zipfs,java.desktop `
  --strip-debug `
  --no-header-files `
  --no-man-pages `
  --compress=2 `
  --output $Destination
if ($LASTEXITCODE -ne 0) {
  throw "jlink 失败，退出码 $LASTEXITCODE"
}
Write-Host "[ok] runtime -> $Destination"
