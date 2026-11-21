param([string]$Mode, [int]$Connections = 2000, [int]$Ramp = 500, [int]$Hold = 20)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$oldPrebuilt = $env:RUN_PREBUILT
$oldOffline = $env:SQLX_OFFLINE
$gateway = $null
$bot = $null
$code = 0
function Read-Metrics {
    (Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 'http://127.0.0.1:18490/metrics').Content
}
function Current([string]$Metrics) {
    if ($Metrics -notmatch '(?m)^gateway_connections_current ([0-9]+)\r?$') { throw 'current connections metric is absent' }
    [int]$Matches[1]
}
function Show-Metrics([string]$Title, [string]$Metrics) {
    Write-Output $Title
    $Metrics -split '\r?\n' | Where-Object { $_ -match '^(gateway_|runtime_)' }
}
Push-Location (Join-Path $PSScriptRoot '..')
$temp = Join-Path (Get-Location).Path "target/tmp/$Mode.$([Guid]::NewGuid())"
try {
    [void][IO.Directory]::CreateDirectory($temp)
    $env:SQLX_OFFLINE = 'true'
    & cmd.exe /d /c "cargo build --bin gateway --bin bot > `"$temp/build`" 2>&1"
    if ($LASTEXITCODE -ne 0) { throw 'building Gateway and bot failed' }
    $env:RUN_PREBUILT = '1'
    $config = 'dev/config/gateway.stress.toml'
    if ($Mode -eq 'smoke') {
        $config = Join-Path $temp 'gateway.toml'
        (Get-Content dev/config/gateway.toml -Raw).Replace('auth_timeout_ms = 10000', 'auth_timeout_ms = 1000') |
            Set-Content -LiteralPath $config -Encoding ASCII
    }
    $ready = Join-Path $temp 'ready'
    $gateway = & "$PSScriptRoot/run-gateway.ps1" -Config $config -ReadyFile $ready -ErrorFile (Join-Path $temp 'gateway-errors')
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (!(Test-Path $ready) -or !(Select-String -LiteralPath $ready -Pattern '^ready admin=127\.0\.0\.1:18490 client=127\.0\.0\.1:18400$' -Quiet)) {
        if ($gateway.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw 'Gateway did not announce readiness' }
        Start-Sleep -Milliseconds 100
    }
    if ($Mode -eq 'smoke') {
        & "$PSScriptRoot/run-bot.ps1" connect-auth --connections 50 --expect-established 50 --expect-close-reason auth_failed
        if ($LASTEXITCODE -ne 0) { throw 'auth scenario failed' }
        & "$PSScriptRoot/run-bot.ps1" connect-hold --connections 20 --hold-secs 3 --expect-established 20 --expect-close-reason auth_timeout
        if ($LASTEXITCODE -ne 0) { throw 'hold scenario failed' }
    } else {
        $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
        $binary = Join-Path $target 'debug/bot.exe'
        $output = Join-Path $temp 'bot'
        $bot = Start-Process -FilePath (Resolve-Path $binary) -ArgumentList @('--config', 'dev/config/bot.stress.toml', `
            'connect-hold', '--connections', $Connections, `
            '--ramp-per-sec', $Ramp, '--hold-secs', $Hold, '--expect-established', $Connections) `
            -RedirectStandardOutput $output -RedirectStandardError (Join-Path $temp 'bot-errors') -PassThru -NoNewWindow
        [void]$bot.Handle
        $peak = -1
        $peakBody = ''
        while (!$bot.HasExited) {
            $body = Read-Metrics
            $active = Current $body
            if ($active -gt $peak) { $peak = $active; $peakBody = $body }
            Start-Sleep -Milliseconds 200
        }
        $bot.WaitForExit()
        Get-Content -LiteralPath $output
        if ($bot.ExitCode -ne 0) { throw "stress scenario failed (bot exit: $($bot.ExitCode))" }
        Show-Metrics 'peak metrics' $peakBody
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $body = Read-Metrics
        if ((Current $body) -eq 0) { break }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Gateway connections did not return to zero' }
        Start-Sleep -Milliseconds 100
    } while ($true)
    if ($Mode -eq 'stress') { Show-Metrics 'end metrics' $body }
} catch {
    [Console]::Error.WriteLine("${Mode}: $($_.Exception.Message)")
    $code = 1
} finally {
    foreach ($process in @($bot, $gateway)) {
        if ($null -ne $process) {
            if (!$process.HasExited) { $process.Kill() }
            $process.WaitForExit()
            $process.Dispose()
        }
    }
    if (Test-Path $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
    $env:RUN_PREBUILT = $oldPrebuilt
    $env:SQLX_OFFLINE = $oldOffline
    Pop-Location
}
# The last stdout line is always "RESULT <mode> PASS" or "RESULT <mode> FAIL", so a watcher can key on it.
if ($code -eq 0) { [Console]::Out.WriteLine("RESULT $mode PASS") } else { [Console]::Out.WriteLine("RESULT $mode FAIL") }
exit $code
