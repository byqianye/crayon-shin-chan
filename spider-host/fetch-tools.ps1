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
  # Windows PowerShell 5.1 的 Expand-Archive -Force 在目标目录已存在时会抛
  # MethodCountCouldNotFindBest（CI 上必现），所以先解到临时目录再整体搬过去。
  $staging = Join-Path $Destination ".unzip-staging"
  if (Test-Path $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $staging | Out-Null
  Write-Host "[unzip] $zip -> $staging"
  Expand-Archive -Path $zip -DestinationPath $staging -Force
  $extracted = Join-Path $staging "dex-tools-v2.4"
  if (-not (Test-Path (Join-Path $extracted "lib"))) {
    throw "解压后没有找到 dex-tools-v2.4/lib：$extracted"
  }
  if (Test-Path $target) { Remove-Item -LiteralPath $target -Recurse -Force }
  Move-Item -LiteralPath $extracted -Destination $target
  Remove-Item -LiteralPath $staging -Recurse -Force
}
Write-Host "[ok] dex-tools ready at $Destination"
