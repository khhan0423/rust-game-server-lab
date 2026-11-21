$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/lib.ps1"
Push-Location (Join-Path $PSScriptRoot '..')
try {
    Build-UnlessPrebuilt --bin launcher --bin publisher --bin server --bin gateway --bin ops
    & (Resolve-Binary launcher) @args
    $code = $LASTEXITCODE
} finally {
    Pop-Location
}
exit $code
