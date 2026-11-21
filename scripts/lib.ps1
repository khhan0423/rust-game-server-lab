# Shared helpers; callers run from the workspace root.
function Import-ServiceEnv([hashtable]$Saved) {
    if (!(Test-Path -LiteralPath .env)) { return }
    foreach ($line in [IO.File]::ReadLines((Resolve-Path .env))) {
        if ($line -match '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
            $name = $Matches[1]; $value = $Matches[2]
            if ($null -ne [Environment]::GetEnvironmentVariable($name)) { continue }
            if ($value -match '^(["''])(.*)\1$') { $value = $Matches[2] }
            $Saved[$name] = $null
            [Environment]::SetEnvironmentVariable($name, $value)
        }
    }
}
function Resolve-Binary([string]$Service) {
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    $binary = Join-Path $target "debug/$Service"
    if (Test-Path -LiteralPath "$binary.exe") { $binary += '.exe' }
    (Resolve-Path -LiteralPath $binary).Path
}
function Get-ServiceConfig([string]$Service) {
    $config = [Environment]::GetEnvironmentVariable($Service.ToUpperInvariant() + '_CONFIG')
    if ($config) { $config } else { "dev/config/$Service.toml" }
}
function Build-UnlessPrebuilt {
    if ($env:RUN_PREBUILT -eq '1') { return }
    $previous = $env:SQLX_OFFLINE
    try {
        $env:SQLX_OFFLINE = 'true'
        & cargo build @args
        if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    } finally { $env:SQLX_OFFLINE = $previous }
}
function Remove-OpsSecrets([hashtable]$Saved) {
    foreach ($name in 'DATABASE_URL', 'LOGIN_DB_URL', 'GAME_DB_URL', 'POSTGRES_PASSWORD', 'PUBLISHER_GAME_API_KEY') {
        if (!$Saved.ContainsKey($name)) {
            $Saved[$name] = [Environment]::GetEnvironmentVariable($name)
        }
        [Environment]::SetEnvironmentVariable($name, $null)
    }
}
function Start-ServiceBinary($Service, $Config, $Arguments, $OutputFile, $ErrorFile) {
    $arguments = @('--config', ('"' + $Config + '"')) + $Arguments
    Start-Process -FilePath (Resolve-Binary $Service) -ArgumentList $arguments -RedirectStandardOutput $OutputFile -RedirectStandardError $ErrorFile -PassThru -NoNewWindow
}
