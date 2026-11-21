#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mode=$1
shift
if [[ $(uname -s) == MINGW* || $(uname -s) == MSYS* ]]; then
    exec powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$(cygpath -aw "scripts/$mode.ps1")" "$@"
fi
connections=${1:-2000}
ramp=${2:-500}
hold=${3:-20}
pid= bot_pid=
temp=$(mktemp -d)
cleanup() {
    if [[ -n $bot_pid ]]; then kill "$bot_pid" 2>/dev/null || true; wait "$bot_pid" 2>/dev/null || true; fi
    if [[ -n $pid ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
    rm -rf "$temp"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
fail() { printf '%s: %s\n' "$mode" "$1" >&2; exit 1; }
export SQLX_OFFLINE=true
cargo build --bin gateway --bin bot >"$temp/build" 2>&1 || fail 'building Gateway and bot failed'
export RUN_PREBUILT=1
config=dev/config/gateway.stress.toml
if [[ $mode == smoke ]]; then
    config=$temp/gateway.toml
    sed 's/auth_timeout_ms = 10000/auth_timeout_ms = 1000/' dev/config/gateway.toml >"$config"
fi
GATEWAY_CONFIG=$config bash scripts/run-gateway.sh >"$temp/ready" 2>"$temp/errors" &
pid=$!
deadline=$((SECONDS + 20))
while ! grep -q '^ready admin=127\.0\.0\.1:18490 client=127\.0\.0\.1:18400$' "$temp/ready"; do
    if ! kill -0 "$pid" 2>/dev/null || (( SECONDS >= deadline )); then fail 'Gateway did not announce readiness'; fi
    sleep 0.1
done
metrics() { curl --fail --silent --max-time 2 http://127.0.0.1:18490/metrics; }
current() { awk '$1 == "gateway_connections_current" { print $2 }' <<< "$1"; }
sample() { printf '%s\n' "$1" | awk '/^(gateway_|runtime_)/'; }
if [[ $mode == smoke ]]; then
    bash scripts/run-bot.sh connect-auth --connections 50 --expect-established 50 --expect-close-reason auth_failed || fail 'auth scenario failed'
    bash scripts/run-bot.sh connect-hold --connections 20 --hold-secs 3 --expect-established 20 --expect-close-reason auth_timeout || fail 'hold scenario failed'
else
    bash scripts/run-bot.sh connect-hold --connections "$connections" --ramp-per-sec "$ramp" --hold-secs "$hold" \
        --expect-established "$connections" >"$temp/bot" &
    bot_pid=$!
    peak=-1
    while kill -0 "$bot_pid" 2>/dev/null; do
        body=$(metrics) || fail 'peak metrics request failed'
        active=$(current "$body")
        if (( active > peak )); then peak=$active; peak_body=$body; fi
        sleep 0.2
    done
    status=0
    wait "$bot_pid" || status=$?
    bot_pid=
    cat "$temp/bot"
    (( status == 0 )) || fail 'stress scenario failed'
    printf 'peak metrics\n'
    sample "${peak_body:-}"
fi
deadline=$((SECONDS + 5))
while :; do
    body=$(metrics) || fail 'final metrics request failed'
    [[ $(current "$body") == 0 ]] && break
    (( SECONDS < deadline )) || fail 'Gateway connections did not return to zero'
    sleep 0.1
done
[[ $mode == smoke ]] || { printf 'end metrics\n'; sample "$body"; }
