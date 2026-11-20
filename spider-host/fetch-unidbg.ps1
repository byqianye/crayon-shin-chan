param(
  [string]$Destination = (Join-Path $PSScriptRoot "unidbg-libs")
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $Destination | Out-Null

$deps = @(
  "com/github/zhkl0228/unidbg-android/0.9.8/unidbg-android-0.9.8.jar",
  "com/github/zhkl0228/unidbg-api/0.9.8/unidbg-api-0.9.8.jar",
  "com/github/zhkl0228/unidbg-unicorn2/0.9.8/unidbg-unicorn2-0.9.8.jar",
  "com/github/zhkl0228/unicorn/1.0.14/unicorn-1.0.14.jar",
  "com/github/zhkl0228/capstone/3.1.8/capstone-3.1.8.jar",
  "com/github/zhkl0228/keystone/0.9.7/keystone-0.9.7.jar",
  "net/dongliu/apk-parser/2.6.10/apk-parser-2.6.10.jar",
  "commons-codec/commons-codec/1.15/commons-codec-1.15.jar",
  "org/apache/commons/commons-collections4/4.4/commons-collections4-4.4.jar",
  "commons-io/commons-io/2.11.0/commons-io-2.11.0.jar",
  "commons-logging/commons-logging/1.2/commons-logging-1.2.jar",
  "com/alibaba/fastjson/1.2.83/fastjson-1.2.83.jar",
  "com/github/zhkl0228/demumble/1.0.4/demumble-1.0.4.jar",
  "net/java/dev/jna/jna/5.10.0/jna-5.10.0.jar",
  "org/scijava/native-lib-loader/2.3.5/native-lib-loader-2.3.5.jar",
  "log4j/log4j/1.2.17/log4j-1.2.17.jar",
  "org/slf4j/slf4j-api/1.7.36/slf4j-api-1.7.36.jar",
  "org/slf4j/slf4j-simple/1.7.36/slf4j-simple-1.7.36.jar"
)

foreach ($rel in $deps) {
  $name = $rel.Split('/')[-1]
  $path = Join-Path $Destination $name
  if (Test-Path $path) {
    Write-Host "[skip] $name"
    continue
  }
  Write-Host "[get ] $name"
  Invoke-WebRequest -Uri "https://repo1.maven.org/maven2/$rel" -OutFile $path -UseBasicParsing
}
