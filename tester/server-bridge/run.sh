#!/bin/sh
# ソルバPC上で常駐させ、試合が始まるたびに自動でソルバを実行する。
# 公式API(/setting・/agent・/)だけを使うので、練習用の対戦サーバー(tester/server)と
# 本番の競技サーバーのどちらにも同じ手順で参加できる。
#
#   cargo build --release -p solver -p server-bridge
#   TEAM=0 ./tester/server-bridge/run.sh     # 練習
#   ./tester/server-bridge/run.sh            # 本番(下の「本番の設定」を書き換えてから)

# ===== 本番の設定: 本番ではこの3行を書き換える =====
SERVER="${SERVER:-http://192.168.0.81:11451}"  # サーバーのURL。本番は当日に連絡される競技サーバー
TOKEN="${TOKEN:-token-p${TEAM:-0}}"           # チームのトークン。本番は配布されたトークン。練習は TEAM から token-p<TEAM>
SELECT_MS="${SELECT_MS:-}"                     # 車種選択の制限時間(ms)。空ならマップの大きさから本選の時間(60/90/120秒)
# ===================================================

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
SOLVER="${SOLVER:-$ROOT/target/release/solver}"
BRIDGE="${BRIDGE:-$ROOT/target/release/server-bridge}"
RECORD_DIR="${RECORD_DIR:-$ROOT/.server/submissions}"
BRIDGE_LOG_DIR="${BRIDGE_LOG_DIR:-$ROOT/.server/bridge-logs}"
SUBMIT_AT_MS="${SUBMIT_AT_MS-55000}"  # 空ならbridgeの従来動作。複数回は5000,15000,...,55000。
STREAM_RESPONSES="${STREAM_RESPONSES:-0}"  # ストリーム対応ソルバだけ1にする。

for f in "$SOLVER" "$BRIDGE"; do
  if [ ! -x "$f" ]; then
    echo "[run.sh] solver_exited result=not_started reason=executable_missing path=$f" >&2
    exit 1
  fi
done

set -- --server "$SERVER" --token "$TOKEN" --solver "$SOLVER" --record-dir "$RECORD_DIR"
if [ -n "$SELECT_MS" ]; then
  set -- "$@" --select-ms "$SELECT_MS"
fi

if [ -n "$SUBMIT_AT_MS" ]; then
  set -- "$@" --submit-at-ms "$SUBMIT_AT_MS"
fi
if [ "$STREAM_RESPONSES" = 1 ]; then
  set -- "$@" --stream-responses
fi

# 1試合ずつ再起動。詳細はファイルへ保存し、端末には運用上の進行だけを表示する。
exec python3 -u "$ROOT/tester/server-bridge/console.py" \
  --log-dir "$BRIDGE_LOG_DIR" --repeat-bridge -- "$BRIDGE" "$@"
