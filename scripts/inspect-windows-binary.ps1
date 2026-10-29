[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$Directory,
      [string]$ExecutableName = "kohdalab-ple.exe",
      [string]$OutputPrefix = "windows",
      [switch]$SkipVersionQuery)
$ErrorActionPreference = "Stop"
$Directory = (Resolve-Path -LiteralPath $Directory).Path
$exe = Join-Path $Directory $ExecutableName
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio/Installer/vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere)) { throw "Visual Studio vswhere is required for binary inspection" }
$dumpbin = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find 'VC/Tools/MSVC/**/bin/Hostx64/x64/dumpbin.exe' | Select-Object -First 1
if (-not $dumpbin) { throw "Could not find dumpbin for Windows binary inspection" }
$text = & $dumpbin /dependents $exe
if ($LASTEXITCODE -ne 0) { throw "dumpbin failed" }
$imports = @($text | ForEach-Object { if ($_ -match '^\s+([^\s]+\.dll)\s*$') { $Matches[1] } } | Sort-Object -Unique)
if ($imports.Count -eq 0) { throw "No Windows DLL imports could be read" }
if ($imports | Where-Object { $_ -match '^(VCRUNTIME|MSVCP|CONCRT|UCRTBASE|api-ms-win-crt-|libgcc|libwinpthread|libstdc\+\+)' }) { throw "Unexpected external C/C++ runtime dependency: $imports" }
$imports | Set-Content -LiteralPath (Join-Path $Directory ($OutputPrefix + "-imports.txt")) -Encoding UTF8
$version = "not queried (GUI startup avoided)"
if (-not $SkipVersionQuery) {
    $version = & $exe --version | Out-String
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($version)) { throw "Packaged application did not report its version" }
}
$hash = [System.Security.Cryptography.SHA256]::Create()
$stream = [System.IO.File]::OpenRead($exe)
try { $digest = [System.BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
finally { $stream.Dispose(); $hash.Dispose() }
[ordered]@{ application_version = $version.Trim(); executable_sha256 = $digest; imports = $imports; external_vc_runtime_required = $false; hardware_tested = $false } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $Directory ($OutputPrefix + "-metadata.json")) -Encoding UTF8
