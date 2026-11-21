$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/lib.ps1"
$service = $args[0]
$serviceArgs = @($args | Select-Object -Skip 1)
$oldEnv = @{}
Push-Location (Join-Path $PSScriptRoot '..')
try {
    if ($service -in 'server', 'publisher', 'ops') { Import-ServiceEnv $oldEnv }
    if ($service -eq 'ops') { Remove-OpsSecrets $oldEnv }
    Build-UnlessPrebuilt --bin $service
    & (Resolve-Binary $service) --config (Get-ServiceConfig $service) @serviceArgs
    $code = $LASTEXITCODE
} finally {
    foreach ($name in $oldEnv.Keys) {
        [Environment]::SetEnvironmentVariable($name, $oldEnv[$name])
    }
    Pop-Location
}
exit $code
