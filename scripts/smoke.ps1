$ErrorActionPreference = 'Stop'
& "$PSScriptRoot/connection-check.ps1" smoke @args
exit $LASTEXITCODE
