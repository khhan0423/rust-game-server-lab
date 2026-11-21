$ErrorActionPreference = 'Stop'
$service = $args[0]
$defaultCommand = $args[1]
$serviceArgs = @($args | Select-Object -Skip 2)
$oldEnv = @{}
Push-Location (Join-Path $PSScriptRoot '..')
try {
    foreach ($line in Get-Content -LiteralPath .env) {
        if ($line -match '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
            $name = $Matches[1]
            if (!$oldEnv.ContainsKey($name)) {
                $oldEnv[$name] = [Environment]::GetEnvironmentVariable($name)
            }
            [Environment]::SetEnvironmentVariable($name, $Matches[2].Trim().Trim('"', "'"))
        }
    }
    if ($service -eq 'ops') {
        foreach ($name in 'DATABASE_URL', 'LOGIN_DB_URL', 'GAME_DB_URL', 'POSTGRES_PASSWORD', 'PUBLISHER_GAME_API_KEY') {
            if (!$oldEnv.ContainsKey($name)) {
                $oldEnv[$name] = [Environment]::GetEnvironmentVariable($name)
            }
            [Environment]::SetEnvironmentVariable($name, $null)
        }
    }
    if ($serviceArgs.Count -eq 0 -and $defaultCommand) {
        $serviceArgs = @($defaultCommand)
    }
    $oldEnv['SQLX_OFFLINE'] = [Environment]::GetEnvironmentVariable('SQLX_OFFLINE')
    $env:SQLX_OFFLINE = 'true'
    & cargo build --bin $service
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    $binary = Join-Path $target "debug/$service"
    if (Test-Path -LiteralPath "$binary.exe") { $binary = "$binary.exe" }
    & $binary --config "dev/config/$service.toml" @serviceArgs
    $code = $LASTEXITCODE
} finally {
    foreach ($name in $oldEnv.Keys) {
        [Environment]::SetEnvironmentVariable($name, $oldEnv[$name])
    }
    Pop-Location
}
exit $code
