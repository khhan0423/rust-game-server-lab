#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/lib.sh
mode=$1
shift
connections=${1:-2000}
ramp=${2:-500}
hold=${3:-20}
pid= bot_pid=
mkdir -p target/tmp
temp=$(mktemp -d "target/tmp/$mode.XXXXXXXX")
cleanup() {
    if [[ -n $bot_pid ]]; then kill "$bot_pid" 2>/dev/null || true; wait "$bot_pid" 2>/dev/null || true; fi
    if [[ -n $pid ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
    rm -rf "$temp"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
# The last stdout line is always "RESULT <mode> PASS" or "RESULT <mode> FAIL: <reason>", so a watcher can key on it.
fail() { printf '%s: %s\n' "$mode" "$1" >&2; printf 'RESULT %s FAIL: %s\n' "$mode" "$1"; exit 1; }
build_unless_prebuilt --bin gateway --bin bot >"$temp/build" 2>&1 || fail 'building Gateway and bot failed'
export RUN_PREBUILT=1
config=${GATEWAY_CONFIG:-dev/config/gateway.$mode.toml}
if [[ $mode == stress ]]; then export BOT_CONFIG=${BOT_CONFIG:-dev/config/bot.stress.toml}; fi
"$(resolve_binary gateway)" --config "$config" >"$temp/ready" 2>"$temp/errors" &
pid=$!
deadline=$((SECONDS + 20))
while ! grep -q '^ready ' "$temp/ready"; do
    if ! kill -0 "$pid" 2>/dev/null || (( SECONDS >= deadline )); then fail 'Gateway did not announce readiness'; fi
    sleep 0.1
done
read -r -a fields < "$temp/ready"
admin= client=
for field in "${fields[@]}"; do case $field in admin=*) admin=${field#admin=} ;; client=*) client=${field#client=} ;; esac; done
[[ -n $admin && -n $client ]] || fail 'Gateway did not announce readiness'
metrics() { curl --fail --silent --max-time 2 "http://$admin/metrics"; }
current() {
    awk '$1 == "net_connections_current{listener=\"client\"}" && $2 ~ /^[0-9]+$/ { print $2; found=1 } END { if (!found) exit 1 }' <<< "$1"
}
sample() { printf '%s\n' "$1" | awk '/^(net_|gateway_auth_|runtime_)/'; }
if [[ $mode == smoke ]]; then
    bash scripts/run-bot.sh connect-auth --gateway "$client" --connections 50 --expect-established 50 --expect-close-reason auth_failed || fail 'auth scenario failed'
    bash scripts/run-bot.sh connect-hold --gateway "$client" --connections 20 --hold-secs 3 --expect-established 20 --expect-close-reason auth_timeout || fail 'hold scenario failed'
else
    bash scripts/run-bot.sh connect-hold --gateway "$client" --connections "$connections" --ramp-per-sec "$ramp" --hold-secs "$hold" \
        --expect-established "$connections" >"$temp/bot" &
    bot_pid=$!
    peak=-1
    while kill -0 "$bot_pid" 2>/dev/null; do
        body=$(metrics) || fail 'peak metrics request failed'
        active=$(current "$body") || fail 'current connections metric is absent'
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
    active=$(current "$body") || fail 'current connections metric is absent'
    [[ $active == 0 ]] && break
    (( SECONDS < deadline )) || fail 'Gateway connections did not return to zero'
    sleep 0.1
done
[[ $mode == smoke ]] || { printf 'end metrics\n'; sample "$body"; }
printf 'RESULT %s PASS\n' "$mode"
