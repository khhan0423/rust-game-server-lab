#!/usr/bin/env bash
# Print WARN and ERROR lines from every server's operations log as they are written, one line each.
# Meant to be left running during a test so that a person or a watcher sees problems the moment they happen.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p logs
shopt -s nullglob globstar
files=(logs/**/ops.*.jsonl)
if (( ${#files[@]} == 0 )); then printf 'no log files under logs/ yet\n' >&2; exit 1; fi
tail -n 0 -F "${files[@]}" 2>/dev/null | grep --line-buffered -E '"level":"(WARN|ERROR)"'
