#!/usr/bin/env bash
set -Eeuo pipefail

repo=${procon37_repo:?}
app=${procon37_app:?}
command_name=solo
launcher_id=procon37
storage=.tester
app_label=テスタ
if [[ $app == game ]]; then command_name=game; launcher_id=procon37-game; storage=.game; fi
if [[ $app == server ]]; then command_name=server; launcher_id=procon37-server; storage=.server; app_label=対戦サーバー; fi
export PATH="${PATH}:${HOME}/.cargo/bin"
export CARGO_TARGET_DIR="$repo/target"
cache="$repo/$storage/launcher"
open_browser=true
action=start
preferred=8080
if [[ $app == game ]]; then preferred=8081; fi
if [[ $app == server ]]; then preferred=8082; fi
explicit_port=false
fail() {
  printf '%s: %s\n' "$command_name" "$*" >&2
  if command -v notify-send >/dev/null && [[ -n ${DISPLAY:-}${WAYLAND_DISPLAY:-} ]]; then
    notify-send --app-name="$command_name" 'GUIを起動できませんでした' "$*" >/dev/null 2>&1 || true
  fi
  exit 1
}
while (($#)); do
  case $1 in
    --no-open) open_browser=false ;;
    --restart) action=restart ;;
    --stop) action=stop ;;
    --port)
      shift
      [[ ${1:-} =~ ^[0-9]{1,5}$ ]] || fail '--portには0〜65535を指定してください。'
      preferred=$((10#$1))
      ((preferred <= 65535)) || fail '--portには0〜65535を指定してください。'
      explicit_port=true ;;
    --help|-h)
      cat <<EOF
$command_name: $app_label のGUIをブラウザで開く
  $command_name                起動済みなら再利用、未起動ならビルドして起動
  $command_name --restart      再ビルドして再起動（実行中は不可）
  $command_name --stop         サーバー停止（実行中は不可）
  $command_name --no-open      ブラウザを開かずURLだけ表示
  $command_name --port PORT    ポートを指定（0は空きポート）
EOF
      exit 0 ;;
    *) fail "不明な引数: $1（--helpで使い方を表示）" ;;
  esac
  shift
done
for program in curl flock setsid; do command -v "$program" >/dev/null || fail "$program が必要です。"; done
cd -- "$repo"
mkdir -p -- "$cache" "$repo/$storage/runs"
runs=$(cd -- "$repo/$storage/runs" && pwd -P)
# Keep the server identity stable so a renamed launcher can reuse/stop it.
identity=$(printf '%s-launcher-v1\n%s\n%s' "$launcher_id" "$repo" "$runs")
exec 9>"$cache/launch.lock"
flock -w 120 9 || fail '別の起動処理が終わりませんでした。少し待って再実行してください。'
request() { curl --noproxy '*' --silent --show-error --fail-with-body --connect-timeout 1 --max-time 3 "$@"; }
ours() {
  local response
  response=$(request "http://127.0.0.1:$1/api/launcher" 2>/dev/null) || return 1
  [[ $response == "$identity" ]]
}
port=''
if [[ -f "$cache/port" ]] && { [[ $action == restart ]] || ! $explicit_port || ((preferred == 0)); }; then
  read -r saved_port <"$cache/port" || true
  if [[ ${saved_port:-} =~ ^[0-9]{1,5}$ ]] && ours "$saved_port"; then port=$saved_port; fi
fi
if [[ -z $port ]] && ((preferred > 0)) && ours "$preferred"; then port=$preferred; fi

if [[ $action != stop && ( -z $port || $action == restart ) ]]; then
  command -v cargo >/dev/null || fail 'Rust/Cargoが見つかりません。Rust 1.94以降をインストールしてください。'
  printf '%s: %sを準備しています…\n' "$command_name" "$app_label" >&2
  if ! cargo build --locked --release --manifest-path "$repo/Cargo.toml" -p "$app" >"$cache/build.log" 2>&1; then
    tail -n 30 "$cache/build.log" >&2
    fail "ビルドに失敗しました。ログ: $cache/build.log"
  fi
fi
if [[ $action == stop || $action == restart ]] && [[ -n $port ]]; then
  if ! result=$(request -X POST "http://127.0.0.1:$port/api/shutdown" 2>&1); then fail "$result"; fi
  for ((i=0;i<50;i++)); do
    if ! ours "$port"; then break; fi
    sleep 0.1
  done
  ours "$port" && fail 'サーバーの終了待ちがタイムアウトしました。'
  port=''
  rm -f -- "$cache/port"
fi
if [[ $action == stop ]]; then printf '%s: サーバーは停止しています。\n' "$command_name"; exit 0; fi

if [[ -z $port ]]; then
  # Bind errors can also race another application. An automatic launch retries
  # with port 0; it never assumes an unrelated listener is this tester.
  for attempt in 1 2; do
    bind_port=$preferred
    if ((attempt == 2)); then
      $explicit_port && fail "ポート $preferred で起動できません。ログ: $cache/server.log"
      bind_port=0
    fi
    nohup setsid "$repo/target/release/$app" serve --port "$bind_port" --output-dir "$runs" \
      </dev/null >"$cache/server.log" 2>&1 9>&- &
    server_pid=$!
    for ((i=0;i<100;i++)); do
      detected=$(sed -n 's@^ビジュアライザ: http://127\.0\.0\.1:\([0-9]*\)/$@\1@p' "$cache/server.log" | head -n 1)
      if [[ -n $detected ]] && ours "$detected"; then port=$detected; break; fi
      kill -0 "$server_pid" 2>/dev/null || break
      sleep 0.1
    done
    [[ -n $port ]] && break
    # Do not leave an unresponsive child behind, or start a duplicate of it.
    if kill -0 "$server_pid" 2>/dev/null; then fail "サーバーの起動待ちがタイムアウトしました。ログ: $cache/server.log"; fi
  done
  [[ -n $port ]] || fail "サーバーを起動できません。ログ: $cache/server.log"
fi
printf '%s\n' "$port" >"$cache/port"
flock -u 9
exec 9>&-
url="http://127.0.0.1:$port/"
printf '%s\n' "$url"
if $open_browser; then
  if command -v wslview >/dev/null && [[ -n ${WSL_DISTRO_NAME:-} ]]; then opener=(wslview)
  elif command -v xdg-open >/dev/null; then opener=(xdg-open)
  elif command -v gio >/dev/null; then opener=(gio open)
  else fail "ブラウザを開くコマンドがありません。手動で $url を開いてください。"; fi
  # Detach the opener as well: some desktop handlers stay alive with the browser.
  nohup "${opener[@]}" "$url" </dev/null >>"$cache/browser.log" 2>&1 &
fi
