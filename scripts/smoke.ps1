$ErrorActionPreference = 'Stop'
& "$PSScriptRoot/connection-check.ps1" smoke
exit $LASTEXITCODE
