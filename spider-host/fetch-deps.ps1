param(
  [string]$Destination = (Join-Path $PSScriptRoot "lib")
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $Destination | Out-Null

$deps = @(
  @{ Name = "okhttp-3.14.9.jar"; Url = "https://repo1.maven.org/maven2/com/squareup/okhttp3/okhttp/3.14.9/okhttp-3.14.9.jar" },
  @{ Name = "okio-1.17.5.jar"; Url = "https://repo1.maven.org/maven2/com/squareup/okio/okio/1.17.5/okio-1.17.5.jar" },
  @{ Name = "json-20090211.jar"; Url = "https://repo1.maven.org/maven2/org/json/json/20090211/json-20090211.jar" },
  @{ Name = "android-4.1.1.4.jar"; Url = "https://repo1.maven.org/maven2/com/google/android/android/4.1.1.4/android-4.1.1.4.jar" },
  @{ Name = "gson-2.10.1.jar"; Url = "https://repo1.maven.org/maven2/com/google/code/gson/gson/2.10.1/gson-2.10.1.jar" }
)

foreach ($dep in $deps) {
  $path = Join-Path $Destination $dep.Name
  if (Test-Path $path) {
    Write-Host "[skip] $($dep.Name)"
    continue
  }
  Write-Host "[get ] $($dep.Name)"
  Invoke-WebRequest -Uri $dep.Url -OutFile $path -UseBasicParsing
}
