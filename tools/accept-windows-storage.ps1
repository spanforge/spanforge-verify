param([string]$Runner='target/release/spanforge-verify.exe')
$ErrorActionPreference='Stop'
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    $runnerPath=(Resolve-Path -LiteralPath $Runner).Path
    $evidenceDirectory=Join-Path $PWD ('target/storage-evidence/' + [guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $evidenceDirectory -Force | Out-Null
    $deniedDirectory=Join-Path $evidenceDirectory 'denied-work-root'
    New-Item -ItemType Directory -Path $deniedDirectory | Out-Null
    $suitePath=Join-Path $evidenceDirectory 'suite.toml'
    $reportPath=Join-Path $evidenceDirectory 'run.json'
    $suiteText="schema_version=1`nsuite_id='denied-storage'`nprogram='$runnerPath'`n[[cases]]`nid='first'`nargs=['--version']`nexpect={exit_code=0}`n"
    [System.IO.File]::WriteAllText($suitePath,$suiteText,[System.Text.UTF8Encoding]::new($false))
    $originalAcl=Get-Acl -LiteralPath $deniedDirectory
    $restrictedAcl=Get-Acl -LiteralPath $deniedDirectory
    $identity=[System.Security.Principal.WindowsIdentity]::GetCurrent().User
    $denyRule=[System.Security.AccessControl.FileSystemAccessRule]::new($identity,[System.Security.AccessControl.FileSystemRights]::Write,[System.Security.AccessControl.AccessControlType]::Deny)
    $restrictedAcl.AddAccessRule($denyRule)
    $previousWorkRoot=$env:SPANFORGE_VERIFY_WORK_ROOT
    try {
        # Only this freshly-created directory changes; retain WRITE_DAC for restoration.
        Set-Acl -LiteralPath $deniedDirectory -AclObject $restrictedAcl
        $env:SPANFORGE_VERIFY_WORK_ROOT=$deniedDirectory
        & $runnerPath run --file $suitePath --json $reportPath
        $runnerExit=$LASTEXITCODE
        $result=Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
        if ($runnerExit -ne 3 -or $result.status -ne 'INFRA_ERROR') {throw 'Denied-storage result was not infrastructure failure'}
        [ordered]@{passed=$true;scenario='denied-work-root';runner_exit_code=$runnerExit;runner_sha256=(Get-FileHash -LiteralPath $runnerPath -Algorithm SHA256).Hash.ToLowerInvariant()} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidenceDirectory 'manifest.json') -Encoding UTF8
    } finally {
        $env:SPANFORGE_VERIFY_WORK_ROOT=$previousWorkRoot
        Set-Acl -LiteralPath $deniedDirectory -AclObject $originalAcl
    }
    Write-Host "Windows denied-storage evidence: $evidenceDirectory"
} finally {Pop-Location}
