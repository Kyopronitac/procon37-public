# VOID1からの分散ソルバ本番操作

## 配置済みC013を使う（2026年10月10日）

2026年10月11日の現在の選択先は `C013-6cdcd06d40e4-dual-submit-20261011` です。60秒の行動回答の終盤に55秒・57秒の提出機会を設けたbridgeを3台に反映しています。同じ回答の追加提出は省略します。

3台のDHCP移行と本番サーバー・トークン設定は完了しています。VOID1で `./tester/server-bridge/distributed/run.sh <モード>` を使います。引数なしは予行演習です。切替前に停止してください。[当日の操作](../competition/docs/operation.md#当日の操作) を参照してください。

| モード | 用途 | タイプ選択時間 |
| --- | --- | ---: |
| `test` | 接続テスト | 20秒 |
| `practice` | 予行演習（既定） | 30秒 |
| `match` | 16×16の本試合・敗者復活戦 | 60秒 |
| `match24` | 24×24の準決勝 | 90秒 |
| `match32` | 32×32の決勝 | 120秒 |

`match24`・`match32` は2026年10月11日に追加したコードです。10月10日の配置に入口だけをコピーしても、旧リリース内の起動処理が90秒・120秒を拒否します。利用前に、更新した `competition/distributed.py` を含む新しいリリースを3台に配置してください。ソルバとbridgeの実行ファイルは変更しません。モードはタイプ選択時間を指定し、マップや日数などは受信した問題を使います。

VOID3台には2026年10月11日に新リリース `C013-6cdcd06d40e4-match-modes-20261011` を反映済みです。VOID1の通常入口で両モードを使用できます。更新後のPC間認証・worker起動停止と、全台の停止状態を確認しています。

VOID1にはリポジトリ全体を `/home/voidlinux-1/procon37` に配置する。
本番URLは `tester/server-bridge/distributed/run.sh` の既定値に設定済みで、トークンはGit管理外の `.server/competition.token` から読み込む。

```sh
cd /home/voidlinux-1/procon37
./tester/server-bridge/distributed/run.sh
./tester/server-bridge/distributed/run.sh status
./tester/server-bridge/distributed/run.sh stop
```

Git管理外の `.server/distributed-production/selected-release` が選択済みの不変リリースを指すシンボリックリンクになっている。
この配置では評価済みC013とbridgeを使い、VOID2/3も自動で起動する。コンパイルと再配置は済んでいるため、以下のD0用 `compile.sh`・`deploy.sh` は実行しない。
URL・トークンが空なら問題サーバーへ接続せず停止する。選択先の破損や追加引数（`--config` 等）がある場合も停止し、D0へ切り替えない。

提出機会は5秒から10秒間隔に加え、各フェーズの締切3秒前にも設けます。タイプ選択の終盤は `match` が55・57秒、`match24` が85・87秒、`match32` が115・117秒です。60秒の行動回答はどのモードでも5・15・25・35・45・55・57秒になります。`test` の20秒は15・17秒、`practice` の30秒は25・27秒です。通信失敗時の再送も締切の約2秒前までに打ち切ります。同じフェーズの直前の実送信内容と同一なら次の提出を省略します。
DHCP対応リリースでは車種選択時間を試合モードから指定します。行動回答は公式の締切時刻を使い、`RECORD_DIR`・`BRIDGE_LOG_DIR` は使用しません。
問題・マップ・日別道路状態・HTTP通信は選択リリースの `records/`、完全ログは `logs/` に保存する。
今回のリポジトリ内 `.server/submissions` と `.server/bridge-logs` は、それぞれへのリンクである。
実行ファイル・認証・CPU配置は [本番運用](../competition/docs/operation.md)、実機の確認結果は [リポジトリ配置記録](../../../solver/docs/distributed/resume-20261009/frequency16-20261010/repository-deployment.md) を参照する。

## D0を新しくコンパイル・配置する

以下は選択リリースのリンクがない従来D0配置の手順。

リポジトリルートで、コンパイル → 配置 → 起動の順に操作する。

```sh
./tester/server-bridge/distributed/compile.sh
./tester/server-bridge/distributed/deploy.sh
./tester/server-bridge/distributed/run.sh
```

`compile.sh` はVOID1の既存RustとCargoキャッシュを使い、D0・bridgeをオフラインビルドする。既存の実行ファイルがあっても両方のCargoビルドを呼び、成功したファイルのパスとSHAを配布設定へ登録する。配置前に必ず実行する。依存crateは事前にCargoキャッシュへ取得しておく。

`deploy.sh` はコンパイルで登録した実行ファイルをSHAで照合し、三台へ配置する。未コンパイルの初期設定や旧設定の `bundled:` 指定では、`compile.sh` の実行を案内して停止する。同梱バイナリへの切り替えは行わない。URL・チームトークン・選択時間は `run.sh` 冒頭の `SERVER`・`TOKEN`・`SELECT_MS` に設定する。環境変数での指定もでき、URL・トークンの変更で再配置する必要はない。SSHパスワードはGit管理外の `.server/distributed-production/ssh-password` に設定する。今回のVOID1配置時に設定済みで、入力は不要。

`run.sh` は三台を起動し、VOID1のbridge・coordinatorとVOID2/3のworkerの実行ログを端末へ流し続ける。試合データはVOID1のリポジトリ内 `.server/submissions/` に保存する。`RECORD_DIR` で保存先を指定でき、配置先を更新しても同じ保存先を使う。`Ctrl+C` で今回起動した三台の処理を停止する。端末終了やSSH切断でも停止処理を行う。別端末から状態確認・停止もできる。

出力は英語。起動時の配置検査とworkerの有線TCP認証に加え、bridgeが問題サーバーへの接続状態、問題の主要パラメータ、提出の採否、探索・提出・試合全体の実時間を表示する。項目の意味は [ログの説明](../../server/docs/solver-pc.md#5-runshを起動する) を参照する。

```sh
./tester/server-bridge/distributed/run.sh status
./tester/server-bridge/distributed/run.sh stop
```

ログを保存したい場合は、実行時に明示的に指定する。

```sh
./tester/server-bridge/distributed/run.sh >run.log 2>&1
```

通常の操作はこの三つのスクリプトを使う。詳しい設定・提出確認・更新・障害復旧は [ソルバPCのセットアップ](../../server/docs/solver-pc.md#分散ソルバd0を3台で本番に使う)を参照する。単一PC用の親ディレクトリの `run.sh` は、この三台分散用の `distributed/run.sh` と区別する。
