#!/bin/sh
# VOID1 operator: show progress and save complete logs. Ctrl+C stops the cluster.
# run.sh [practice|match|match24|match32|test] [start|status|stop] [--config FILE].
set -eu
# ===== 本番の設定: 当日指定のURLとトークンをここに設定する =====
SERVER="${SERVER:-http://172.28.0.10:8080}"
TOKEN="${TOKEN:-}"
MATCH_MODE="${MATCH_MODE:-practice}"  # practice=30秒 / match=60秒 / match24=90秒 / match32=120秒 / test=20秒
# ==========================================================
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
TOKEN_FILE="${TOKEN_FILE:-$ROOT/.server/competition.token}"
if [ -z "$TOKEN" ] && [ -f "$TOKEN_FILE" ]; then export TOKEN_FILE; fi
case "${1:-}" in
  practice|match|match24|match32|test) MATCH_MODE=$1; SELECT_MS=; shift ;;
esac
case "$MATCH_MODE" in
  practice) SELECT_MS="${SELECT_MS:-30000}" ;;
  match) SELECT_MS="${SELECT_MS:-60000}" ;;
  match24) SELECT_MS="${SELECT_MS:-90000}" ;;
  match32) SELECT_MS="${SELECT_MS:-120000}" ;;
  test) SELECT_MS="${SELECT_MS:-20000}" ;;
  *) echo 'MATCH_MODE must be practice, match, match24, match32, or test.' >&2; exit 1 ;;
esac
RECORD_DIR="${RECORD_DIR:-$ROOT/.server/submissions}"
cd "$ROOT"
action=start
case "${1:-}" in
  start|status|stop) action=$1; shift ;;
  ""|--config) ;;
  *) echo "Usage: $0 [practice|match|match24|match32|test] [start|status|stop] [--config FILE]" >&2; exit 1 ;;
esac
# 配置済みの評価版を選択している場合は、その不変リリースを使う。
# リンク切れや不正な選択ではD0へ切り替えず停止する。
selected="$ROOT/.server/distributed-production/selected-release"
if [ -e "$selected" ] || [ -L "$selected" ]; then
  if [ "$#" -ne 0 ]; then
    echo "Selected release accepts only start, status, or stop; --config is not supported." >&2
    exit 1
  fi
  if [ ! -L "$selected" ] || [ ! -f "$selected/manifest.json" ] || [ ! -x "$selected/run.sh" ]; then
    echo "Invalid selected release: $selected" >&2
    exit 1
  fi
  export SERVER TOKEN SELECT_MS
  exec "$selected/run.sh" "$action"
fi
# D0の従来配置では練習用の既定値を維持する。
SERVER="${SERVER:-http://192.168.0.81:11451}"
TOKEN="${TOKEN:-token-p0}"
if [ "$action" = start ]; then
  export SERVER TOKEN SELECT_MS RECORD_DIR
  BRIDGE_LOG_DIR="${BRIDGE_LOG_DIR:-$ROOT/.server/bridge-logs}"
  exec python3 -u "$ROOT/tester/server-bridge/console.py" --log-dir "$BRIDGE_LOG_DIR" -- \
    python3 -u tester/server-bridge/distributed.py "$action" "$@"
fi
exec python3 -u tester/server-bridge/distributed.py "$action" "$@"
