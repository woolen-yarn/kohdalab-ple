$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
cargo test --workspace --locked
if ($LASTEXITCODE -ne 0) { throw "Rust tests failed" }
cargo build --release --locked --bin kohdalab-ple
if ($LASTEXITCODE -ne 0) { throw "Build failed" }
$dist = Join-Path (Get-Location) "dist"
$dest = Join-Path $dist "kohdalab-ple-windows"
$stage = Join-Path $dist (".kohdalab-ple-windows-staging-" + $PID)
$backup = Join-Path $dist (".kohdalab-ple-windows-previous-" + $PID)
if (Test-Path -LiteralPath $stage) { throw "unexpected existing staging directory: $stage" }

try {
    New-Item -ItemType Directory -Path $stage | Out-Null
    Copy-Item "target/release/kohdalab-ple.exe" $stage
    Copy-Item "bridge" "$stage/bridge" -Recurse
    New-Item -ItemType Directory -Path "$stage/scripts" | Out-Null
    Copy-Item -Path @("scripts/preflight.ps1", "scripts/verify_run.py", "scripts/verify_gui_api.py", "scripts/collect_licenses.py") -Destination "$stage/scripts/"
    Copy-Item "bindings" "$stage/bindings" -Recurse
    Copy-Item "examples" "$stage/examples" -Recurse
    Copy-Item "docs" "$stage/docs" -Recurse
    Copy-Item "README.md", "LICENSE" -Destination $stage
    Copy-Item "third_party" "$stage/third_party" -Recurse
    $rustTarget = (rustc -vV | Select-String '^host: ').ToString().Substring(6)
    python scripts/collect_licenses.py --target $rustTarget --output "$stage/third_party/rust"
    if ($LASTEXITCODE -ne 0) { throw "License collection failed" }

    if (Test-Path -LiteralPath $dest) {
        Move-Item -LiteralPath $dest -Destination $backup
    }
    try {
        Move-Item -LiteralPath $stage -Destination $dest
    } catch {
        if (Test-Path -LiteralPath $backup) { Move-Item -LiteralPath $backup -Destination $dest }
        throw
    }
    if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Recurse -Force }
    Write-Output "Built $dest. Install Python and vendor software separately; see README."
} finally {
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
