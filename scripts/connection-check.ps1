param([string]$Mode, [int]$Connections = 2000, [int]$Ramp = 500, [int]$Hold = 20)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
. "$PSScriptRoot/lib.ps1"
$oldPrebuilt = $env:RUN_PREBUILT
$oldBotConfig = $env:BOT_CONFIG
$gateway = $null
$bot = $null
$code = 0
function Read-Metrics {
    (Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 "http://$admin/metrics").Content
}
function Current([string]$Metrics) {
    if ($Metrics -notmatch '(?m)^net_connections_current\{listener="client"\} ([0-9]+)\r?$') { throw 'current connections metric is absent' }
    [int]$Matches[1]
}
function Show-Metrics([string]$Title, [string]$Metrics) {
    Write-Output $Title
    $Metrics -split '\r?\n' | Where-Object { $_ -match '^(net_|gateway_auth_|runtime_)' }
}
Push-Location (Join-Path $PSScriptRoot '..')
$temp = Join-Path (Get-Location).Path "target/tmp/$Mode.$([Guid]::NewGuid())"
try {
    [void][IO.Directory]::CreateDirectory($temp)
    try { Build-UnlessPrebuilt --bin gateway --bin bot } catch { throw 'building Gateway and bot failed' }
    $env:RUN_PREBUILT = '1'
    $config = if ($env:GATEWAY_CONFIG) { $env:GATEWAY_CONFIG } else { "dev/config/gateway.$Mode.toml" }
    $ready = Join-Path $temp 'ready'
    $gateway = Start-ServiceBinary gateway $config @() $ready (Join-Path $temp 'gateway-errors')
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (!(Test-Path $ready) -or !(Select-String -LiteralPath $ready -Pattern '^ready ' -Quiet)) {
        if ($gateway.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw 'Gateway did not announce readiness' }
        Start-Sleep -Milliseconds 100
    }
    $fields = @{}
    foreach ($field in ((Get-Content -LiteralPath $ready -TotalCount 1) -split ' ' | Select-Object -Skip 1)) {
        $name, $address = $field -split '=', 2; $fields[$name] = $address
    }
    $admin = $fields['admin']; $clientAddr = $fields['client']
    if (!$admin -or !$clientAddr) { throw 'Gateway did not announce readiness' }
    if ($Mode -eq 'smoke') {
        & "$PSScriptRoot/run-bot.ps1" connect-auth --gateway $clientAddr --connections 50 --expect-established 50 --expect-close-reason auth_failed
        if ($LASTEXITCODE -ne 0) { throw 'auth scenario failed' }
        & "$PSScriptRoot/run-bot.ps1" connect-hold --gateway $clientAddr --connections 20 --hold-secs 3 --expect-established 20 --expect-close-reason auth_timeout
        if ($LASTEXITCODE -ne 0) { throw 'hold scenario failed' }
    } else {
        if (!$env:BOT_CONFIG) { $env:BOT_CONFIG = 'dev/config/bot.stress.toml' }
        $output = Join-Path $temp 'bot'
        $bot = Start-ServiceBinary bot (Get-ServiceConfig bot) @('connect-hold', '--gateway', $clientAddr, '--connections', $Connections, '--ramp-per-sec', $Ramp, '--hold-secs', $Hold, '--expect-established', $Connections) $output (Join-Path $temp 'bot-errors')
        [void]$bot.Handle
        $peak = -1
        $peakBody = ''
        while (!$bot.HasExited) {
            try { $body = Read-Metrics } catch { throw 'peak metrics request failed' }
            $active = Current $body
            if ($active -gt $peak) { $peak = $active; $peakBody = $body }
            Start-Sleep -Milliseconds 200
        }
        $bot.WaitForExit()
        Get-Content -LiteralPath $output
        if ($bot.ExitCode -ne 0) { throw 'stress scenario failed' }
        Show-Metrics 'peak metrics' $peakBody
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        try { $body = Read-Metrics } catch { throw 'final metrics request failed' }
        if ((Current $body) -eq 0) { break }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Gateway connections did not return to zero' }
        Start-Sleep -Milliseconds 100
    } while ($true)
    if ($Mode -eq 'stress') { Show-Metrics 'end metrics' $body }
} catch {
    $reason = $_.Exception.Message
    [Console]::Error.WriteLine("${Mode}: $reason")
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
    $env:BOT_CONFIG = $oldBotConfig
    Pop-Location
}
# The last stdout line is always "RESULT <mode> PASS" or "RESULT <mode> FAIL: <reason>", so a watcher can key on it.
if ($code -eq 0) { [Console]::Out.WriteLine("RESULT $mode PASS") } else { [Console]::Out.WriteLine("RESULT $mode FAIL: $reason") }
exit $code
