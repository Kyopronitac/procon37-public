# セットアップと更新

- テスタの入口 → [../README.md](../README.md)
- CUI → [usage.md](usage.md)
- GUI操作 → [gui.md](gui.md)

## 必要な環境

| 用途 | 必要なもの |
| --- | --- |
| ビルド・CUI | Rust / Cargo 1.94以降 |
| GUI | 上記＋ブラウザ |
| Linux / WSLのコマンド・アプリ登録 | Bash、curl 7.76以降、util-linuxの `flock` / `setsid` |
| ブラウザの自動起動 | Linuxは `xdg-open` または `gio`。WSLは導入済みなら `wslview` を優先 |

初回ビルドにはCargo依存の取得が必要。取得後は `--offline` でもビルドできる。Python / Node.jsは通常利用には不要。

## Cargoから起動する

Linux / WSL / macOSで、リポジトリのルートから実行する。

```sh
cargo run --release -p tester -- serve
```

表示されたURLをブラウザで開く。既定は `http://127.0.0.1:8080/`。ポート・保存先は変更できる。

```sh
cargo run --release -p tester -- serve --port 8090 --output-dir .tester/runs
```

サーバーはlocalhost限定。指定したビルド・ソルバコマンドをサーバーの権限で実行するため、外部公開には使用しない。

## Linuxのコマンド・アプリ登録

リポジトリのルートで、一度だけセットアップする。管理者権限は不要。

```sh
bash tester/solo/desktop/setup.sh
solo
```

以後は任意のディレクトリで `solo` を実行するか、アプリ一覧で「solo」を検索する。

| 操作 | コマンド |
| --- | --- |
| 起動・既存サーバーを再利用 | `solo` |
| 再ビルドして再起動 | `solo --restart` |
| 停止 | `solo --stop` |
| ブラウザを開かずURLを表示 | `solo --no-open` |
| ポートを指定 | `solo --port 8090` |
| 空きポートを選ぶ | `solo --port 0` |

- サーバーはバックグラウンドで動き、ターミナルやブラウザを閉じても継続する。OSログイン時の自動起動は行わない。
- 同じ作業ディレクトリ・履歴保存先のサーバーを再利用し、重複起動を避ける。
- 既定の8080が他のアプリで使用中なら空きポートへ切り替える。明示したポートが使用中ならエラー。
- 試合実行中は停止・再起動できない。完了するか、GUIで試合を中断してから操作する。

## 変更をGUI・CUIへ反映する

| 変更対象 | 反映方法 |
| --- | --- |
| ソルバ | 次の実行でビルド。`--no-build` を使う場合は自分でビルド |
| テスタ・生成器・HTML/CSS/JS（登録済みGUI） | `solo --restart` 後、ブラウザを再読み込み |
| テスタ・生成器・HTML/CSS/JS（CargoからGUI起動） | サーバーを終了し、`cargo run --release -p tester -- serve` で起動し直す |
| テスタ・生成器（CUI） | `cargo run --release -p tester -- run`。実行前にCargoが更新をビルド |

起動済みサーバーは、ディスク上の実行ファイルをビルドし直しても以前の処理を使い続ける。新しい生成器の確認には、再起動後に新しく問題を生成する。保存済み履歴は生成当時の問題を表示する。

## 登録先を変更・解除する

| 登録物 | 既定の保存先 |
| --- | --- |
| コマンド | `~/.local/bin/solo` |
| アプリ登録 | `${XDG_DATA_HOME:-~/.local/share}/applications/solo.desktop` |
| アイコン | 同じデータディレクトリの `icons/hicolor/scalable/apps/solo.svg` |

セットアップの再実行で登録を更新できる。リポジトリを移動した場合は移動先で再実行する。複数チェックアウトを登録した場合、最後に登録したものを開く。

| オプション | 用途 |
| --- | --- |
| `--bin-dir /absolute/path` | コマンドの登録先を変更 |
| `--data-dir /absolute/path` | アプリ・アイコンの登録先を変更 |
| `--no-build` | ビルド済みの場合にセットアップ時のビルドを省略 |

別用途の同名ファイルは上書きしない。解除時も、独自の登録先を使っていれば同じオプションを指定する。

```sh
solo --stop
bash tester/solo/desktop/setup.sh --uninstall
```

登録解除ではコマンド・アプリ登録・アイコンを削除する。実行履歴は残る。

## 困ったとき

| 症状 | 確認・対処 |
| --- | --- |
| `solo` が見つからない | セットアップが表示したPATH設定を適用。`~/.local/bin` がPATHに必要 |
| ブラウザが開かない | `solo --no-open` のURLを手動で開く |
| 変更した生成器・画面が出ない | サーバーを再起動し、ブラウザを再読み込み。新しい問題を生成 |
| 起動・ビルドが失敗する | `.tester/launcher/build.log`、`server.log`、`browser.log` を確認 |

アプリ一覧からの起動は絶対パスを使うため、ターミナルのPATH設定には依存しない。
