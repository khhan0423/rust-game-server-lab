param([string]$Config = 'dev/config/gateway.toml', [string]$ReadyFile, [string]$ErrorFile)
$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    if ($env:RUN_PREBUILT -ne '1') {
        $previous = $env:SQLX_OFFLINE
        try { $env:SQLX_OFFLINE = 'true'; cargo build --bin gateway }
        finally { $env:SQLX_OFFLINE = $previous }
        if ($LASTEXITCODE -ne 0) { throw 'building gateway failed' }
    }
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    $binary = Join-Path $target 'debug/gateway'
    if (Test-Path -LiteralPath "$binary.exe") { $binary = "$binary.exe" }
    if ($ReadyFile) {
        return Start-Process -FilePath (Resolve-Path $binary) -ArgumentList @('--config', ('"' + $Config + '"')) `
            -RedirectStandardOutput $ReadyFile -RedirectStandardError $ErrorFile -PassThru -NoNewWindow
    }
    & $binary --config $Config @args
    $code = $LASTEXITCODE
} finally { Pop-Location }
exit $code
