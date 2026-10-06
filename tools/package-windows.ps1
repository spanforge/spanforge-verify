param([string]$Toolchain = '1.98.1')
$ErrorActionPreference = 'Stop'
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    & cargo "+$Toolchain" fmt --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed' }
    & cargo "+$Toolchain" clippy --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
    & cargo "+$Toolchain" test --locked
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
    & cargo "+$Toolchain" build --release --locked --bin spanforge-verify --bin cliverifyr
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    & python tools/package-release.py --binary target/release/spanforge-verify.exe --target x86_64-pc-windows-msvc --toolchain $Toolchain
    if ($LASTEXITCODE -ne 0) { throw 'Packaging failed' }
} finally { Pop-Location }
