# Quality gate (guidelines section 0). Tests need the development PostgreSQL.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
if (-not (Get-Command protoc -CommandType Application -ErrorAction SilentlyContinue)) {
    [Console]::Error.WriteLine('protoc not found on PATH; install the Protocol Buffers compiler before running the gate.')
    exit 1
}
. "$PSScriptRoot/lib.ps1"
Import-ServiceEnv @{}
$env:SQLX_OFFLINE = "true"
# The vendored htmx file must match its recorded hash.
$expected = ((Get-Content crates/ops/assets/htmx.sha256 | Where-Object { $_ -notmatch '^#' -and $_.Trim() }) -split '\s+')[0]
$actual = (Get-FileHash -Algorithm SHA256 -LiteralPath crates/ops/assets/htmx.min.js).Hash.ToLowerInvariant()
if ($actual -ne $expected) { [Console]::Error.WriteLine('crates/ops/assets/htmx.min.js does not match htmx.sha256'); exit 1 }
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
# Process tests need the publisher, gateway, and launcher executables.
cargo build --bin publisher --bin gateway --bin launcher
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo test --workspace
exit $LASTEXITCODE
