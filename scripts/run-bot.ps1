$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    if ($env:RUN_PREBUILT -ne '1') {
        $previous = $env:SQLX_OFFLINE
        try { $env:SQLX_OFFLINE = 'true'; cargo build --bin bot }
        finally { $env:SQLX_OFFLINE = $previous }
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    $binary = Join-Path $target 'debug/bot'
    if (Test-Path -LiteralPath "$binary.exe") { $binary = "$binary.exe" }
    $config = if ($env:BOT_CONFIG) { $env:BOT_CONFIG } else { 'dev/config/bot.toml' }
    & $binary --config $config @args
    $code = $LASTEXITCODE
} finally { Pop-Location }
exit $code
