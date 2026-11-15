#!/usr/bin/env bash
set -Eeuo pipefail

repo=${procon37_repo:?}
app=${procon37_app:?}
command_name=solo
legacy_name=procon37
app_dir=tester/solo
if [[ $app == game ]]; then command_name=game; legacy_name=procon37-game; app_dir=tester/game; fi
if [[ $app == server ]]; then command_name=server; legacy_name=procon37-server; app_dir=tester/server; fi
generic_name="ヘキサうどん テスタ"
comment="実行履歴・比較・リプレイをブラウザで開く"
keywords="procon37;ヘキサうどん;テスタ;ビジュアライザ;"
if [[ $app == server ]]; then
  generic_name="ヘキサうどん 対戦サーバー"
  comment="対戦の実行と結果の閲覧をブラウザで行う"
  keywords="procon37;ヘキサうどん;対戦;サーバー;"
fi
bin_dir="$HOME/.local/bin"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
action=install
build=true
while (($#)); do
  case $1 in
    --uninstall) action=uninstall ;;
    --no-build) build=false ;;
    --bin-dir|--data-dir)
      option=$1; shift
      [[ -n ${1:-} ]] || { printf '%sにディレクトリを指定してください。\n' "$option" >&2; exit 1; }
      if [[ $option == --bin-dir ]]; then bin_dir=$1; else data_dir=$1; fi ;;
    --help|-h)
      printf 'bash %s/desktop/setup.sh [--uninstall] [--no-build] [--bin-dir DIR] [--data-dir DIR]\n' "$app_dir"
      printf 'ユーザーのコマンドとLinuxのアプリ一覧に%sを登録します。sudoは不要です。\n' "$command_name"
      exit 0 ;;
    *) printf '不明な引数: %s\n' "$1" >&2; exit 1 ;;
  esac
  shift
done
[[ $(uname -s) == Linux ]] || { printf 'このセットアップはLinux/WSL用です。\n' >&2; exit 1; }
[[ $bin_dir == /* && $data_dir == /* ]] || { printf '登録先は絶対パスで指定してください。\n' >&2; exit 1; }
[[ $repo$bin_dir$data_dir != *$'\n'* && $repo$bin_dir$data_dir != *$'\r'* ]] || { printf '改行を含むパスには対応していません。\n' >&2; exit 1; }
launcher="$bin_dir/$command_name"
desktop="$data_dir/applications/$command_name.desktop"
# Generic names such as "server" exist in system icon themes, which take precedence
# over hicolor. A prefixed icon name keeps the launcher from showing a theme icon.
icon_name="hexaudon-$command_name"
icon="$data_dir/icons/hicolor/scalable/apps/$icon_name.svg"
old_icon="$data_dir/icons/hicolor/scalable/apps/$command_name.svg"
marker="$legacy_name-managed-launcher-v1"
# Refuse to overwrite/remove an unrelated command or desktop entry.
for file in "$launcher" "$desktop" "$icon"; do
  if [[ -e $file || -L $file ]] && ! grep -Fq -- "$marker" "$file"; then
    printf '別のファイルが存在するため変更しません: %s\n' "$file" >&2; exit 1
  fi
done
# Icons installed under the command name by earlier versions.
if [[ -f $old_icon && ! -L $old_icon ]] && grep -Fq -- "$marker" "$old_icon"; then rm -f -- "$old_icon"; fi
if [[ $action == uninstall ]]; then
  rm -f -- "$launcher" "$desktop" "$icon"
  printf '%sのコマンドとアプリ登録を解除しました。実行履歴は残しています。\n' "$command_name"
else
  for program in bash curl flock setsid; do command -v "$program" >/dev/null || { printf '%sが必要です。\n' "$program" >&2; exit 1; }; done
  export PATH="$PATH:$HOME/.cargo/bin"
  if $build; then cargo build --locked --release --manifest-path "$repo/Cargo.toml" --target-dir "$repo/target" -p "$app"; fi
  mkdir -p -- "$bin_dir" "$(dirname -- "$desktop")" "$(dirname -- "$icon")"
  # %q quotes repository paths as Bash code, including spaces and apostrophes.
  { printf '#!/usr/bin/env bash\n# %s\nexec /bin/bash ' "$marker"; printf '%q' "$repo/$app_dir/desktop/launch.sh"; printf ' "$@"\n'; } >"$launcher"
  chmod 755 "$launcher"
  # Desktop Entry applies string escaping before Exec quoting. See the spec:
  # https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html
  exec_path=${launcher//\\/\\\\}
  exec_path=${exec_path//\"/\\\"}
  exec_path=${exec_path//\$/\\\$}
  exec_path=${exec_path//\`/\\\`}
  exec_path=${exec_path//\\/\\\\}
  exec_path=${exec_path//%/%%}
  cat >"$desktop" <<EOF
[Desktop Entry]
Type=Application
Name=$command_name
GenericName=$generic_name
Comment=$comment
Exec=/bin/bash "$exec_path"
Icon=$icon_name
Terminal=false
StartupNotify=false
Categories=Development;
Keywords=$keywords
X-Procon37-Managed=$marker
EOF
  cp -- "$repo/$app_dir/desktop/$command_name.svg" "$icon"
  chmod 644 "$desktop" "$icon"
  if command -v desktop-file-validate >/dev/null; then desktop-file-validate "$desktop"; fi
  printf '登録しました。ターミナルで %s、またはアプリ一覧で %s を検索してください。\n' "$command_name" "$command_name"
  case :$PATH: in
    *:"$bin_dir":*) ;;
    *) printf 'このターミナルで使うには、次を実行してください:\n  export PATH='; printf '%q' "$bin_dir"; printf ':"$PATH"\n' ;;
  esac
fi
# Retire old names only when their managed launcher points at this checkout.
# Old checkouts used $app/desktop; moved checkouts already use $app_dir/desktop.
legacy_launcher="$bin_dir/$legacy_name"
printf -v legacy_exec 'exec /bin/bash %q "$@"' "$repo/$app_dir/desktop/launch.sh"
printf -v old_legacy_exec 'exec /bin/bash %q "$@"' "$repo/$app/desktop/launch.sh"
if [[ -f $legacy_launcher && ! -L $legacy_launcher ]] && grep -Fq -- "$marker" "$legacy_launcher" &&
   { grep -Fxq -- "$legacy_exec" "$legacy_launcher" || grep -Fxq -- "$old_legacy_exec" "$legacy_launcher"; }; then
  for file in "$legacy_launcher" "$data_dir/applications/$legacy_name.desktop" "$data_dir/icons/hicolor/scalable/apps/$legacy_name.svg"; do
    if [[ -f $file && ! -L $file ]] && grep -Fq -- "$marker" "$file"; then rm -f -- "$file"; fi
  done
fi
if command -v update-desktop-database >/dev/null && [[ -d $data_dir/applications ]]; then update-desktop-database "$data_dir/applications" >/dev/null 2>&1 || true; fi
if command -v gtk-update-icon-cache >/dev/null && [[ -d $data_dir/icons/hicolor ]]; then gtk-update-icon-cache --force --ignore-theme-index "$data_dir/icons/hicolor" >/dev/null 2>&1 || true; fi
