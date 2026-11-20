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
if (-not (Test-Path (Join-Path $Destination "dex-tools-v2.4\lib"))) {
  Write-Host "[unzip] $Destination"
  Expand-Archive -Path $zip -DestinationPath $Destination -Force
}
Write-Host "[ok] dex-tools ready at $Destination"
