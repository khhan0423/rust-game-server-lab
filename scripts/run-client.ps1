$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    $config = if ($env:CLIENT_CONFIG) { $env:CLIENT_CONFIG } else { 'dev/config/client.toml' }
    & cargo run --bin client -- --config $config @args
    $code = $LASTEXITCODE
} finally { Pop-Location }
exit $code
