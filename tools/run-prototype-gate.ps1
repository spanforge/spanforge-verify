param(
    [ValidateRange(1, 1000)][int]$Repeat = 100,
    [string]$EvidenceRoot = 'target/prototype-evidence',
    [string]$Toolchain = '1.98.1'
)
$ErrorActionPreference = 'Stop'
if (-not ('SpanForgeGatePower' -as [type])) {
    Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public static class SpanForgeGatePower { [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags); }'
}
$previousPowerState=[SpanForgeGatePower]::SetThreadExecutionState([uint32]2147483649)
if ($previousPowerState -eq 0) {throw 'Could not inhibit automatic sleep for lifecycle testing'}
$workspaceRoot = Split-Path -Parent $PSScriptRoot
Push-Location $workspaceRoot
try {
    $rustVersion = (& rustc "+$Toolchain" --version)
    & cargo "+$Toolchain" build --locked --bins
    if ($LASTEXITCODE -ne 0) { throw 'Prototype build failed' }
    $evidencePath = Join-Path $workspaceRoot $EvidenceRoot
    $runDirectory = Join-Path $evidencePath ([guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $runDirectory -Force | Out-Null
    $os = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $metadata = [ordered]@{
        run_id = Split-Path -Leaf $runDirectory
        started_at = [DateTime]::UtcNow.ToString('o')
        product_name = $os.ProductName
        display_version = $os.DisplayVersion
        build = $os.CurrentBuildNumber
        update_build_revision = $os.UBR
        architecture = $env:PROCESSOR_ARCHITECTURE
        rust_version = $rustVersion
        repetitions = $Repeat
        warmup_iteration = 0
        parent_job_active_process_limit = 64
        fixture_sha256 = (Get-FileHash target/debug/spanforge-verify-fixture.exe -Algorithm SHA256).Hash.ToLowerInvariant()
        prototype_sha256 = (Get-FileHash target/debug/prototype.exe -Algorithm SHA256).Hash.ToLowerInvariant()
        cargo_lock_sha256 = (Get-FileHash Cargo.lock -Algorithm SHA256).Hash.ToLowerInvariant()
        passed = $false
        automatic_sleep_inhibited = $true
    }
    $ordinary = Join-Path $runDirectory 'ordinary.json'
    & ./target/debug/prototype.exe --fixture ./target/debug/spanforge-verify-fixture.exe --repeat $Repeat --evidence $ordinary
    $ordinaryExit = $LASTEXITCODE
    $nested = Join-Path $runDirectory 'parent-job.json'
    & ./target/debug/prototype.exe --fixture ./target/debug/spanforge-verify-fixture.exe --repeat $Repeat --parent-job --evidence $nested
    $nestedExit = $LASTEXITCODE
    $metadata.passed = ($ordinaryExit -eq 0 -and $nestedExit -eq 0 -and $Repeat -ge 100)
    $metadata.ordinary_exit_code = $ordinaryExit
    $metadata.parent_job_exit_code = $nestedExit
    $metadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $runDirectory 'manifest.json') -Encoding UTF8
    Write-Host "Prototype evidence: $runDirectory"
    if ($ordinaryExit -ne 0 -or $nestedExit -ne 0) { throw 'Lifecycle prototype failed; evidence retained' }
} finally {
    Pop-Location
    [SpanForgeGatePower]::SetThreadExecutionState($previousPowerState) | Out-Null
}
