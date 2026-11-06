[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$SourceDirectory,
      [Parameter(Mandatory = $true)][string]$Destination)
$ErrorActionPreference = "Stop"
$source = (Resolve-Path -LiteralPath $SourceDirectory).Path
$destinationPath = [System.IO.Path]::GetFullPath($Destination)
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$expected = "7e8d1c953905ad92a49a07cd75b916c38a3a518d"
Push-Location $source
try {
    $revision = (& git rev-parse HEAD | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $revision -ne $expected) { throw "Unexpected kohdalab-shutter source revision" }
    if (-not [string]::IsNullOrWhiteSpace((& git status --porcelain | Out-String))) { throw "Shutter companion source must be clean" }
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "Shutter companion build failed" }
    if (Test-Path -LiteralPath $destinationPath) { throw "Companion destination already exists" }
    New-Item -ItemType Directory -Path $destinationPath | Out-Null
    Copy-Item target/release/kohdalab-shutter-gui.exe,target/release/kohdalab-shutter.exe,LICENSE -Destination $destinationPath
    git archive --format=zip --output (Join-Path $destinationPath "source.zip") HEAD
    if ($LASTEXITCODE -ne 0) { throw "Shutter source archive failed" }
    python (Join-Path $projectRoot "scripts/collect_licenses.py") --target x86_64-pc-windows-msvc --supplemental-root (Join-Path $projectRoot "third_party/rust-upstream") --output (Join-Path $destinationPath "third_party/rust")
    if ($LASTEXITCODE -ne 0) { throw "Shutter dependency licenses failed" }
    & (Join-Path $projectRoot "scripts/inspect-windows-binary.ps1") -Directory $destinationPath -ExecutableName kohdalab-shutter-gui.exe -OutputPrefix shutter-gui -SkipVersionQuery
    & (Join-Path $projectRoot "scripts/inspect-windows-binary.ps1") -Directory $destinationPath -ExecutableName kohdalab-shutter.exe -OutputPrefix shutter-cli -SkipVersionQuery
    $help = & (Join-Path $destinationPath "kohdalab-shutter.exe") --help | Out-String
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($help)) { throw "Shutter CLI help check failed" }
    $version = python -c "import pathlib,tomllib; print(tomllib.loads(pathlib.Path('Cargo.toml').read_text())['package']['version'])"
    if ($LASTEXITCODE -ne 0) { throw "Shutter manifest version read failed" }
    $gui = Get-Content -LiteralPath (Join-Path $destinationPath "shutter-gui-metadata.json") -Raw | ConvertFrom-Json
    $cli = Get-Content -LiteralPath (Join-Path $destinationPath "shutter-cli-metadata.json") -Raw | ConvertFrom-Json
    [ordered]@{ repository = "https://github.com/woolen-yarn/kohdalab-shutter"; commit = $revision; version = $version; protocol = 2; gui_sha256 = $gui.executable_sha256; cli_sha256 = $cli.executable_sha256; physical_hardware_tested = $false } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destinationPath "companion-origin.json") -Encoding UTF8
    Copy-Item (Join-Path $projectRoot "packaging/shutter-companion/README.md") -Destination $destinationPath
    Copy-Item (Join-Path $projectRoot "packaging/shutter-companion/Start-Shutter.cmd") -Destination $destinationPath
} finally { Pop-Location }
