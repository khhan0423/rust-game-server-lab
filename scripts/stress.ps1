param([int]$Connections = 2000, [int]$Ramp = 500, [int]$Hold = 20)
$ErrorActionPreference = 'Stop'
& "$PSScriptRoot/connection-check.ps1" stress $Connections $Ramp $Hold
exit $LASTEXITCODE
