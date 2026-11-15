# 本番運用と配置手順

## 当日の操作

現在の選択リリースは `C013-6cdcd06d40e4-dual-submit-20261011` です。2026年10月11日に3台とも更新しました。60秒の行動回答には55秒と57秒の提出機会を設け、直前の送信内容と同じ回答は省略します。bridgeの26件のテスト（55秒・57秒の送信と同一回答の省略、59.5秒の送信抑止を含む）、起動処理の15件のテスト、3台の認証・worker起動停止を確認しています。

2026年10月10日の配置はDHCP対応版です。3台の有線接続をDHCPに設定し、PC間は `voidlinux-1.local`・`voidlinux-2.local`・`voidlinux-3.local` で接続します。競技サーバーは `http://172.28.0.10:8080`、トークンはVOID1のGit管理外 `.server/competition.token`（0600）に設定済みです。

VOID1の `/home/voidlinux-1/procon37` で、試合の種類に合わせて起動します。

```sh
./tester/server-bridge/distributed/run.sh practice  # 予行演習：車種選択30秒
./tester/server-bridge/distributed/run.sh match     # 16×16・敗者復活戦：車種選択60秒
./tester/server-bridge/distributed/run.sh match24   # 24×24・準決勝：車種選択90秒
./tester/server-bridge/distributed/run.sh match32   # 32×32・決勝：車種選択120秒
./tester/server-bridge/distributed/run.sh test      # 接続テスト：車種選択20秒
```

引数なしは予行演習です。同時に複数起動せず、切替前にCtrl+Cまたは `run.sh stop` で停止します。行動回答はサーバーが返す各日の締切に従います。サーバーが車種選択の厳密な終了時刻を返す場合も、その時刻を優先します。詳細は [DHCP移行記録](dhcp-20261010.md) を参照してください。

`match24`・`match32` は2026年10月11日のコード更新で追加しました。使用には入口とリリース内の起動処理の両方の更新が必要です。10月10日のリリースは90秒・120秒を拒否するため、更新した `competition/distributed.py` を含む新リリースを3台に配置し、選択先を切り替えてから使用します。既存リリースのファイルや検証用ハッシュを直接書き換えません。評価済みソルバとbridgeのバイナリは同じものを使います。

2026年10月11日、VOID3台に `C013-6cdcd06d40e4-match-modes-20261011` を配置し、選択先とVOID1の入口を更新済みです。3台で同一manifestの照合、PC間認証・worker起動停止、VOID1での15件のテスト、両モードの `status` を確認しました。本番サーバーへの接続・回答提出は行っていません。旧リリースを保持し、過去の記録70ファイル・ログ20ファイルは新保存先へコピーしてSHA-256一致を確認しています。更新完了時は3台とも停止状態です。

モード名はマップを生成・変更する指定ではありません。マップ、エージェント数、日数、各日のステップ数は問題データに従います。

設定を変更する場合、接続先は入口の `SERVER` または環境変数、トークンはGit管理外の `.server/competition.token` を使います。別ファイルは `TOKEN_FILE` 環境変数で指定できます。`TOKEN` 環境変数も利用できますが、明示的な `TOKEN_FILE` がある場合はファイルを優先します。

```sh
cd ~/procon37
./tester/server-bridge/distributed/run.sh
```

VOIDLINUX-2と3はこの入口からSSHで起動します。配置時に使用したSSHパスワードはVOIDLINUX-1の選択リリース内 `private/ssh-password` へ0600権限で保存し、起動時に自動で使用します。環境変数 `PROCON37_SSH_PASSWORD` があればそちらを優先します。パスワードはソース・配布パッケージ・評価記録へ保存しません。

`Ctrl+C` はこの入口が所有する3台のプロセスを停止します。別の端末から状況を確認したり停止したりできます。

```sh
./tester/server-bridge/distributed/run.sh status
./tester/server-bridge/distributed/run.sh stop
```

実行中の評価や別配置のbridge／workerを検出すると、起動を拒否します。他の処理を勝手に停止しません。1台または2台だけ残っている部分起動状態も再利用せず拒否します。この場合は `run.sh stop` で所有する残存処理を停止してから起動します。

編集するのは上記の安定した入口だけです。`current/run.sh` や `releases/` 配下は識別用ハッシュで固定されているため、編集しません。入口に設定した値は環境変数でリリースへ渡り、ソルバ実装と提出設定の同一性を保ちます。

## 構成と保存

| PC | スレッド | 役割 | worker offset |
| --- | ---: | --- | ---: |
| VOIDLINUX-1 | 32 | 探索・集約・提出 | 0 |
| VOIDLINUX-2 | 16 | 探索 | 31 |
| VOIDLINUX-3 | 32 | 探索 | 46 |

CPU割り当てはそれぞれ0–31、0–15、0–31です。CPU探索とPC間解共有を有効にし、`cross-elite=on` とwire protocol 4を使います。評価用ポート39370と分離してworkerはTCP 39371で待ち受けます。

提出機会は5秒から10秒間隔で、終盤には締切5秒前と3秒前の2回を設けます。60秒の行動回答は全モードで5・15・25・35・45・55・57秒です。タイプ選択の終盤は `match` が55・57秒、`match24` が85・87秒、`match32` が115・117秒です。接続テスト20秒は15・17秒、予行演習30秒は25・27秒になります。直前に送信した回答と同じ場合は、どちらの機会も省略します。短いローカル試験では5秒の余裕をフェーズ長の半分、3秒の余裕を3分の1まで縮めます。明示的に早い時刻だけを指定したスケジュールには終盤の機会を追加しません。

送信開始・通信失敗時の再送・途中復帰時の提出は締切の約2秒前までに打ち切ります（行動回答では既存の0.1秒の余裕も含めて2.1秒前）。8秒未満のローカル試験では送信の余裕をフェーズ長の4分の1に縮めます。通信待ちの後にも送信可能時刻を再確認し、遅延による締切直前の送信を防ぎます。サーバーが示す締切を優先し、公式の制限時間そのものは変更しません。

直前に実際に送信した回答と同一なら、受理・却下・採否不明のいずれでも次の提出機会での再送を省きます。1回のHTTP提出処理の中で行う通信失敗時の再試行は維持します。

| 保存先（選択リリース内） | 内容 |
| --- | --- |
| `manifest.json` | 実行ファイル・起動実装・設定のSHA-256 |
| `selection.json` | 候補ID・評価証拠・実行ファイルのSHA-256 |
| `logs/` | 起動単位の完全ログ（0600） |
| `records/` | 試合ごとの問題全体・マップ・日別道路状態・HTTP通信 |
| `runner.json` | PID・起動時刻・boot ID・UID・session・cwd |
| `private/` | ノード間トークン・当日の競技トークン・VOID1のSSHパスワード（0600） |

通常の入口は `~/procon37/tester/server-bridge/distributed/run.sh` です。リポジトリ内 `.server/distributed-production/selected-release` がリリースを直接指します。既存の `~/procon37-competition/run.sh` と `current` も互換用に保持しますが、当日の設定・起動には通常の入口を使います。リリースは `~/procon37-competition/releases/<候補>-<SHA先頭>/` に保持します。リポジトリ本体の移動に依存せず、評価済み実行ファイルを維持する構成です。リポジトリの `.server/submissions`・`.server/bridge-logs` はリリースの `records/`・`logs/` へのリンクです。

停止時はPIDだけで判断せず、起動時刻・boot ID・所有者・session・作業ディレクトリを照合します。pidfdで対象プロセスを保持してから終了させ、PID再利用で別プロセスを停止しない構成です。SSH切断・親プロセス消失でも既存の停止経路を利用します。`runtime_base.py` はD0の運用コントローラーのプロセス管理を維持し、設定検査・名前解決だけをDHCPに対応させています。

## 評価担当者によるパッケージ作成

`config.example.json` を作業用設定にコピーし、選定版のソルバ実行ファイル・SHA-256・一意な配置先を設定します。bridgeには指定した評価済み実行ファイルを使います。採用候補の識別情報を含む評価前の固定記録または評価記録を `--evidence` に渡します。両実行ファイルのSHA-256が証拠に含まれなければパッケージ化を拒否します。

```sh
python3 tester/server-bridge/competition/distributed.py package \
  --config /path/to/selected-config.json \
  --archive /path/to/selected-release.tar.gz \
  --candidate C014 \
  --evidence /path/to/evaluated-preflight.json
```

この処理はコンパイルを行いません。評価したバイナリをコピーし、照合します。取得元のソースやバイナリを改変しません。

VOIDLINUX-1へ `competition/` の運用ファイル、設定、パッケージを別の準備用ディレクトリに転送し、VOIDLINUX-1で配置します。既に存在するリリースを上書きしません。

```sh
python3 /path/to/bootstrap/distributed.py deploy \
  --config /path/to/selected-config.json --archive /path/to/selected-release.tar.gz
python3 /path/to/bootstrap/distributed.py probe --config /path/to/selected-config.json
python3 /path/to/bootstrap/distributed.py activate --config /path/to/selected-config.json
```

`deploy`・`probe`・`activate` は問題サーバーのURL・競技トークンを要求せず、試合に接続しません。`probe` はVOID2/3の待機workerへwire 4の認証だけを行い、VOID1のソルバを空の標準入力で起動して終了させます。Solve要求やHTTP通信は送らず、作成したプロセスは終了します。実施は評価終了後に行います。3台への配置とハッシュ・実行権限・CPU設定・トークン権限を確認し、起動は操作者が明示的に行います。既に別のリリースが `current` として選択されている場合は、誤変更を防ぐため選択更新を拒否します。

## ローカル検証

```sh
taskset -c 1 python3 -m unittest discover -s tester/server-bridge/competition/tests -v
```

実際のソルバやSSHを使わず、スレッドと提出引数、評価証拠の照合、ファイル改変検出、未設定時の起動拒否、wire 4認証、入口編集後もハッシュ照合が維持されることを確認します。
