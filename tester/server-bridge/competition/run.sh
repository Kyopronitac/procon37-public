#!/bin/sh
# Streaming competition release: run only from VOIDLINUX-1. Ctrl+C stops all three nodes.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
# No default endpoint: set the actual competition SERVER and TOKEN/TOKEN_FILE before start.
SERVER="${SERVER:-}"
TOKEN="${TOKEN:-}"
export SERVER TOKEN
action=${1:-start}
case "$action" in start|status|stop) ;; *) echo 'Usage: run.sh [start|status|stop]' >&2; exit 1 ;; esac
if [ "$#" -gt 1 ]; then echo 'Unexpected arguments' >&2; exit 1; fi
if [ "$action" = start ]; then
  exec python3 -u "$ROOT/progress.py" --log-dir "$ROOT/logs" -- \
    python3 -u "$ROOT/distributed.py" start --config "$ROOT/node-config.json"
fi
exec python3 -u "$ROOT/distributed.py" "$action" --config "$ROOT/node-config.json"
