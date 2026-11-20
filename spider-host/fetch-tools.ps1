param(
  [string]$Destination = (Join-Path $PSScriptRoot "dex-tools")
)

$ErrorActionPreference = "Stop"
$zip = Join-Path $env:TEMP "dex-tools-v2.4.zip"
if (-not (Test-Path $zip)) {
  Write-Host "[get ] dex-tools-v2.4.zip"
  Invoke-WebRequest `
    -Uri "https://github.com/pxb1988/dex2jar/releases/download/v2.4/dex-tools-v2.4.zip" `
    -OutFile $zip `
    -UseBasicParsing
}

New-Item -ItemType Directory -Force -Path $Destination | Out-Null
$target = Join-Path $Destination "dex-tools-v2.4"
if (-not (Test-Path (Join-Path $target "lib"))) {
  # 不用 Expand-Archive：它在 -Force 时会走 ExtractToDirectory 的 3 参数重载，
  # GitHub runner 的 .NET 上没有这个重载，必抛 MethodCountCouldNotFindBest。
  # ZipFile 的 2 参数重载从 .NET 4.5 起就有，行为稳定。
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $staging = Join-Path $Destination ".unzip-staging"
  if (Test-Path $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $staging | Out-Null
  Write-Host "[unzip] $zip -> $staging"
  try {
    [System.IO.Compression.ZipFile]::ExtractToDirectory($zip, $staging)
    $extracted = Join-Path $staging "dex-tools-v2.4"
    if (-not (Test-Path (Join-Path $extracted "lib"))) {
      throw "解压后没有找到 dex-tools-v2.4/lib：$extracted"
    }
    if (Test-Path $target) { Remove-Item -LiteralPath $target -Recurse -Force }
    Move-Item -LiteralPath $extracted -Destination $target
  } finally {
    if (Test-Path $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
  }
}
Write-Host "[ok] dex-tools ready at $Destination"
