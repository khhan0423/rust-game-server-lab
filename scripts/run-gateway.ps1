param([string]$Config, [string]$ReadyFile, [string]$ErrorFile)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/lib.ps1"
$previous = $env:GATEWAY_CONFIG
Push-Location (Join-Path $PSScriptRoot '..')
try {
    if ($Config) { $env:GATEWAY_CONFIG = $Config }
    if ($ReadyFile) {
        Build-UnlessPrebuilt --bin gateway
        return Start-ServiceBinary gateway (Get-ServiceConfig gateway) @() $ReadyFile $ErrorFile
    }
    & "$PSScriptRoot/run-service.ps1" gateway @args
    $code = $LASTEXITCODE
} finally { $env:GATEWAY_CONFIG = $previous; Pop-Location }
exit $code
