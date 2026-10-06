param([ValidateRange(1,1000)][int]$Repeat=100,[string]$Toolchain='1.98.1')
$ErrorActionPreference='Stop'
if (-not ('SpanForgeConsolePower' -as [type])) {Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public static class SpanForgeConsolePower { [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags); }'}
$previousPowerState=[SpanForgeConsolePower]::SetThreadExecutionState([uint32]2147483649)
if ($previousPowerState -eq 0) {throw 'Could not inhibit automatic sleep for console testing'}
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    & cargo "+$Toolchain" build --locked --bin spanforge-verify --bin spanforge-verify-fixture --bin console-gate
    if ($LASTEXITCODE -ne 0) {throw 'Console gate build failed'}
    $consoleEvidence = Join-Path $PWD ('target/console-gate-' + [guid]::NewGuid().ToString() + '.json')
    $probe=Start-Process -FilePath (Join-Path $PWD 'target/debug/console-gate.exe') -ArgumentList @('--runner','./target/debug/spanforge-verify.exe','--fixture','./target/debug/spanforge-verify-fixture.exe','--repeat',"$Repeat",'--evidence',('"' + $consoleEvidence + '"')) -WindowStyle Hidden -Wait -PassThru
    if ($probe.ExitCode -ne 0) {throw "Console gate failed; inspect $consoleEvidence and its error.txt sidecar"}
    Write-Host "Console cancellation evidence: $consoleEvidence"
} finally {Pop-Location;[SpanForgeConsolePower]::SetThreadExecutionState($previousPowerState) | Out-Null}
