# run.shの表示と保存

`tester/server-bridge/run.sh` と `tester/server-bridge/distributed/run.sh` の通常起動では、端末への表示と詳細ログの保存を分けます。リポジトリの配置場所はスクリプトから求めるため、起動時の作業ディレクトリに依存しません。

## 端末への表示

表示する内容は次の8種類です。探索統計、通信の内部状態、回答の内容、保存先の説明、再起動待機の案内は詳細ログだけに残します。

- 分散ノードへのアクセス確認（分散版のみ）
- 問題サーバーへのアクセス確認・接続状態
- 問題の受信（マップの縦横・日数）
- ソルバの起動
- 現在の日
- 提出（車種選択／行動計画・日・開始からの秒数・提出回数）
- 提出状況（accepted／rejected／unknown）
- ソルバの終了

指定時刻での提出では `elapsed_sec` がその選択期間／その日の開始からの実測秒数です。従来のbridgeには実測時刻のイベントがないため、従来動作を明示した場合は要求ログからの概算を `elapsed_sec_approx` と表示します。提出状況が `unknown` の場合は、受理を確認できていません。

`distributed/run.sh status` と `stop` は運用状況の照会・停止専用なので、従来どおりそれぞれの結果を表示します。

## 詳細ログ

`BRIDGE_LOG_DIR` の既定値はリポジトリ内の `.server/bridge-logs/` です。起動ごとに `UTC日時-PID.log` を新規作成し、bridgeとソルバの標準出力・標準エラーをすべて保存します。分散版ではSSH経由のノード出力も含みます。各ファイルは所有者だけが読み書きできる権限です。

```sh
BRIDGE_LOG_DIR=/path/to/logs TEAM=0 ./tester/server-bridge/run.sh
```

このファイルには表示を省いた理由や通信・探索の詳細が残ります。診断時は該当起動のファイルを参照します。中断シグナルは起動した処理へ転送し、通常版の認証拒否（終了コード2）では再起動しません。分散版のCtrl+Cでは既存のノード停止処理が動きます。

## 問題・マップ・道路状態の保存

表示ログを減らしても、試合データの保存方法は変わりません。`RECORD_DIR` の既定値はリポジトリ内の `.server/submissions/` です。試合ごとに重複しないディレクトリを作成し、次のファイルを保存します。

| ファイル | 内容 |
| --- | --- |
| `problem.json` | `/setting` で受信した問題全体。未知の追加フィールドも保持 |
| `map.json` | 受信した問題の `map` 全体 |
| `metadata.json` | 試合ID・受信時刻・車種選択締切など |
| `observations.jsonl` | 日別状態。道路状態は各レコードの `state.traffics` |
| `http.jsonl` | 問題サーバーとの要求・応答・提出内容 |

問題とマップは車種選択の提出前、日別状態はその日の提出前にファイルへ書き込みます。保存先が書き込めない場合は詳細ログに `recording_failed` を残し、提出自体は継続します。ログだけを見て保存成功と断定せず、必要な試合のファイルを確認してください。

## 通常版の提出設定

新しいbridgeで通常版の `run.sh` を起動すると、既定では選択開始／各日の開始から55秒に提出します。`SUBMIT_AT_MS` はミリ秒のカンマ区切りです。短い制限時間ではbridgeが実際の締切に収まるよう調整します。

```sh
# 通常の一回提出（既定値）
SUBMIT_AT_MS=55000 TEAM=0 ./tester/server-bridge/run.sh

# 複数応答に対応した開発中ソルバだけに使用する
SUBMIT_AT_MS=5000,15000,25000,35000,45000,55000 STREAM_RESPONSES=1 \
  SOLVER=/path/to/stream-capable-solver ./tester/server-bridge/run.sh
```

`STREAM_RESPONSES=1` は途中解を複数返すソルバ専用です。従来動作での確認が必要なら `SUBMIT_AT_MS=` を明示します。採用済みD0の `distributed/run.sh` は既存の配置パッケージを起動するため、この通常版の環境変数では提出方式を変更しません。開発中分散版の評価設定は評価ハーネス側で固定します。

## 検証

```sh
python3 -m unittest discover -s tester/server-bridge/tests -p test_console.py -v
BRIDGE_TEST_BINARY=target/debug/server-bridge \
  python3 -m unittest discover -s tester/server-bridge/tests -p test_console.py -v
```

2番目はビルド済みbridgeを模擬HTTPサーバーへ接続し、`run.sh` 経由で問題・マップ・道路状態の完全保存と提出前の保存、詳細ログと表示の分離を検証します。実際の対戦・SSHは行いません。
