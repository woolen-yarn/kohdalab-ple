[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateScript({ Test-Path -LiteralPath $_ -PathType Leaf })]
    [string]$ConfigPath,
    [Parameter(Mandatory = $true)]
    [ValidateScript({ Test-Path -LiteralPath $_ -PathType Leaf })]
    [string]$AppPath,
    [switch]$SdkImports,
    [string]$ConexSdkDir
)

$ErrorActionPreference = "Stop"
# This wrapper performs validation only: it neither opens an instrument nor
# starts a measurement. Vendor assembly type loading is opt-in via -SdkImports.
$ConfigPath = (Resolve-Path -LiteralPath $ConfigPath).Path
$AppPath = (Resolve-Path -LiteralPath $AppPath).Path
$config = Get-Content -LiteralPath $ConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json
$python = $config.spectrometer.python
if (-not ($python -is [string]) -or [string]::IsNullOrWhiteSpace($python)) {
    throw "config.spectrometer.python must be explicit; no Python default is assumed"
}
$checker = Join-Path $PSScriptRoot "../bridge/sdk_check.py"
if (-not (Test-Path -LiteralPath $checker -PathType Leaf)) { throw "missing SDK checker: $checker" }

$checkerArgs = @($checker, "--config", $ConfigPath)
if ($SdkImports) { $checkerArgs += "--sdk-imports" }
if (-not [string]::IsNullOrWhiteSpace($ConexSdkDir)) { $checkerArgs += @("--conex-sdk-dir", $ConexSdkDir) }
$sdkText = & $python @checkerArgs
$sdkExit = $LASTEXITCODE
try { $sdkReport = $sdkText | ConvertFrom-Json } catch { throw "SDK checker did not return JSON: $sdkText" }

# The app owns authoritative fixed/position-calibration profile validation.
# --check-config is non-motion and rejects invalid scan plans.
$appText = & $AppPath --check-config --config $ConfigPath
$appExit = $LASTEXITCODE
$appReport = $null
if ($appExit -eq 0) {
    try { $appReport = $appText | ConvertFrom-Json } catch { throw "app did not return JSON: $appText" }
}

$report = [pscustomobject]@{
    check_kind = "installation/configuration only"
    installation_config_pass = (($sdkExit -eq 0) -and ($appExit -eq 0) -and ($sdkReport.installation_config_pass -eq $true) -and ($appReport.config_valid -eq $true))
    sdk_check = $sdkReport
    application_check_config_exit_code = $appExit
    application_check_config = $appReport
    hardware_not_tested = @(
        "No LightField Automation instance, CONEX COM port, VISA resource, TCP endpoint, or device discovery was opened.",
        "Physical qualification still requires LightField acquisition/Stop/export, CONEX readback/move/Stop, power calibration, and shutter fail-closed checks."
    )
}
$report | ConvertTo-Json -Depth 8
if (-not $report.installation_config_pass) {
    throw ("Preflight failed: SDK checker exit " + $sdkExit + "; app --check-config exit " + $appExit)
}
