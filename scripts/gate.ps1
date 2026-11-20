# Quality gate (guidelines section 0). Tests need the development PostgreSQL.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
if (-not (Get-Command protoc -CommandType Application -ErrorAction SilentlyContinue)) {
    [Console]::Error.WriteLine('protoc not found on PATH; install the Protocol Buffers compiler before running the gate.')
    exit 1
}
if (Test-Path .env) {
    Get-Content .env | Where-Object { $_ -match '^\s*[^#=]+=' } | ForEach-Object {
        $name, $value = $_ -split '=', 2
        Set-Item -Path "Env:$($name.Trim())" -Value $value.Trim()
    }
}
$env:SQLX_OFFLINE = "true"
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo test --workspace
exit $LASTEXITCODE
