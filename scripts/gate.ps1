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
# The vendored htmx file must match its recorded hash.
$expected = ((Get-Content crates/ops/assets/htmx.sha256 | Where-Object { $_ -notmatch '^#' -and $_.Trim() }) -split '\s+')[0]
$actual = [System.BitConverter]::ToString([System.Security.Cryptography.SHA256]::Create().ComputeHash([System.IO.File]::ReadAllBytes((Resolve-Path crates/ops/assets/htmx.min.js)))).Replace('-', '').ToLower()
if ($actual -ne $expected) { [Console]::Error.WriteLine('crates/ops/assets/htmx.min.js does not match htmx.sha256'); exit 1 }
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
# The operations integration test drives both binaries; Cargo exposes only its own package's binary.
cargo build --bin publisher
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo test --workspace
exit $LASTEXITCODE
