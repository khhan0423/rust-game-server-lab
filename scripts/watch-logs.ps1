# Print new WARN and ERROR lines from the operations files present at startup.
$ErrorActionPreference = 'Stop'
$files = @(Get-Item -Path (Join-Path $PSScriptRoot '../logs/*-*/ops.????-??-??.jsonl') -ErrorAction SilentlyContinue)
if ($files.Count -eq 0) {
    [Console]::Error.WriteLine('no log files under logs/ yet')
    exit 1
}
$streams = @{}
try {
    foreach ($file in $files) {
        $stream = [IO.File]::Open($file.FullName, 'Open', 'Read', 'ReadWrite, Delete')
        $stream.Position = $stream.Length
        $streams[$file.FullName] = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8)
    }
    while ($true) {
        foreach ($reader in $streams.Values) {
            while ($null -ne ($line = $reader.ReadLine())) {
                if ($line -cmatch '^\{"ts":"[^"]+","level":"(WARN|ERROR)"') { Write-Output $line }
            }
        }
        Start-Sleep -Milliseconds 200
    }
} finally {
    foreach ($reader in $streams.Values) { $reader.Dispose() }
}
