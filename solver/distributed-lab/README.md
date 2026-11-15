# 分散ソルバの再開用評価ハーネス

採用D0の47ファイルを変更せず、現在のリポジトリ配置から4チーム評価を再開する。`manage.py`は自身の場所からルートを決定し、配置の識別・起動した評価の記録・結果確定を担当する。SSHは読み取り専用。遠隔への配置、固定相手の上書き、チームプロセスの起動・停止は行わない。

実機への準備は同じディレクトリの`prepare.py`が担当する。2026-10-09の復元ではVOIDLINUX 3台の新規`~/procon37-distributed-resume/20261009-D0`へD0・評価時bridge・launcher・内部認証ファイルを配置し、SHA一致を確認した。配置記録は`.server/distributed/deployments/20261009-D0/deployment.json`。配置は起動を伴わない。NOMLABへ再接続し、固定相手の照合と4チームの専用プロセス起動を終えてから、このハーネスの`snapshot`と`run`を使う。ローカル固定2体の再ビルド記録は`solver/docs/distributed/resume-20261009/local-restoration.json`、launcherは`.server/distributed/launchers`に保存する。

再開時点の採用版はD0、実装commit `5d6c740962a1f3e20e77e13df243078df1d29ad2`。正式12件は全48AC、平均順位1.6667、得点合計222・1276・2989。初期採用と通常pushは完了している。過去のserver PC・bridge・配置と現在が異なる場合は、候補比較前に同じ新条件でD0を評価し直す。

## 設定と保存先

設定JSONを作り、次のように登録する。最初の登録は遠隔PCがofflineでも可能。設定にパスワード・チームトークン・内部認証文字列は入れない。

```sh
python3 solver/distributed-lab/manage.py configure .server/distributed/resume-config.json
python3 solver/distributed-lab/manage.py status
```

設定項目:

| 項目 | 内容 |
| --- | --- |
| `version` | D0などの対象版名 |
| `serverUrl` | 実測したこのPCの対戦窓口 `http://IP:11451` |
| `server` | リポジトリ相対のserver実行ファイル。既定`target/release/server` |
| `localFiles` | 対象ソルバ・ハーネス等の相対ファイル名→SHA256。候補の全ソースと配置ファイルを列挙する |
| `fixedLocalFiles` | 固定するローカルファイル名→SHA256。server、公式`tester/server/procon-server`、bridge、第6・第8世代バイナリ、それぞれのlauncherを必ず含める |
| `localProcesses` | ローカル2チームの常駐bridge/runnerを確認する条件の配列 |
| `nodes` | `v1`・`v2`・`v3`・`nomlab`の4項目。下記の形式 |

各nodeは以下の形で実機に合わせる。`directory`は実際に確認した`home`配下の絶対パス。`files`はその配置先内の相対パス→期待SHA256で、solver・bridge・launcher・起動スクリプト・設定を含める。秘密ファイルは内容を記録せず、必要な場合は識別用SHAだけを登録する。

```json
{
  "user": "voidlinux-2",
  "address": "192.168.0.22",
  "home": "/home/voidlinux-2",
  "directory": "/home/voidlinux-2/procon37-distributed-resume/20261009-D0",
  "cpus": "0-15",
  "files": {
    "bin/distributed-solver": "実ファイルの64桁SHA256",
    "launcher": "実ファイルの64桁SHA256"
  },
  "processes": [{
    "matchArg": "/home/voidlinux-2/procon37-distributed-resume/20261009-D0/bin/distributed-solver",
    "cpus": "0-15",
    "threads": 16
  }],
  "fixed": false
}
```

`matchArg`は`/proc/PID/cmdline`の要素との完全一致で確認する。`cpus`は`Cpus_allowed_list`の表記、任意の`threads`は待機時に確認するスレッド数。v1・NOMLAB・ローカル2体は試合待ち中にも存在する専用runnerを指定する。v2・v3は常駐workerを指定する。NOMLABだけ`fixed: true`を必須とする。

`configure`は進行中・結果未確定の評価があると変更を拒否する。また、登録済みの固定相手と基盤、ローカル参加プロセス条件は再登録で変更できない。固定条件そのものを変える必要がある場合は別の実験として記録し、D0の基準を取り直す。採用D0のソースは正式preflightの47 SHAとの一致を毎回確認する。改善版はD0を直接変更せず別ディレクトリに保存し、`localFiles`にその全ソース・ビルド設定を追加する。

Git無視対象の保存先:

```text
.server/distributed-state.json             最新の構成・進行状態
.server/distributed/lab-config.json        現在の登録設定
.server/distributed/lab.lock               同時操作防止
.server/distributed/evaluations/<stage>/
  manifest.json                           凍結した対象版・配置・期待SHA
  before.json / after.json                 新規取得した前後SHAとプロセス
  run-config.json / process.json           評価条件・コマンド・PID・起動識別
  server.log                              serverの出力
  run.json / results.json                  確定した履歴の複製・集計
  failure.json                            明示的に失敗扱いにした記録
.server/runs/<run-id>/                     server本来の履歴・リプレイ
```

## 照合・評価・確定

SSH鍵が使える場合は追加設定不要。パスワード認証を使う場合は親シェルに`PROCON37_SSH_PASSWORD`を設定する。この場合だけPythonの`pexpect`が必要。パスワードをファイル・コマンド引数・評価の環境へ書き出さない。

```sh
python3 solver/distributed-lab/manage.py snapshot \
  --save .server/distributed/readiness.json

python3 solver/distributed-lab/manage.py run resume-D0-short-01 \
  --short-ms 10000 --label '分散D0・現配置基準1回目・短縮12件・10秒'

# 正式評価は--short-msを付けない
python3 solver/distributed-lab/manage.py run candidate-006-formal \
  --label '分散候補006・正式採用確認・正式12件'

python3 solver/distributed-lab/manage.py summarize resume-D0-short-01 \
  --run-id '実際のrun-id'

# 同じ時間条件で確定済みの基準と比較
python3 solver/distributed-lab/manage.py summarize candidate-006-short-01 \
  --run-id '実際のrun-id' --before-stage resume-D0-short-01
```

`run`は全4台とローカル2チームのファイルSHA・必須プロセス・CPU affinityを照合してから、分離したserverプロセスを起動する。offline、SHA不一致、二重評価、既存stageの上書きは起動前に拒否する。常に12件・4チーム・seed 0〜11・形式自動・練習場有効。`--output-dir`は使用しない。短縮の`--short-ms`はserverへ一度だけ渡し全員に共通適用する。正式では時間の上書きを渡さない。

起動するserverの環境から`PROCON37_*`・`SELECT_MS`・`TIME_LIMIT_MS`を除く。チーム側の起動環境は配置担当が明示して固定すること。初期D0の探索係数は`PROCON37_G007_FUTURE_WEIGHT=0`と`PROCON37_G007_RESERVE_WEIGHT=15`。正式評価時には全チームの探索予算・時間短縮変数が残っていないことを配置時にも確認する。

`summarize`はserverのPID・起動時刻・boot IDを照合して終了を確認し、開始後に作られたrunだけを受け付ける。全48AC、12seed、各形式4件、時間条件とラベルを検証し、前後のソース・実行ファイル・設定・配置hash一致を確認してから結果を確定する。得点合計と平均順位を保存し、基準指定時はseed別の得点辞書順で勝・同点・敗を計算する。比較時に固定相手・基盤・配置・時間条件が変わっていれば拒否する。評価結果の確定は採用を意味せず、正式評価に基づく採否とcommit・通常pushは別途記録する。

serverが失敗して正常集計できない場合、関連プロセスを確認・停止した後に以下で失敗記録を残す。実行中PIDまたは同じリポジトリのserver runが残る場合は解除しない。プロセスを停止する機能は含めない。

```sh
python3 solver/distributed-lab/manage.py release-failed candidate-006-short-01 \
  --reason '失敗原因と停止確認の内容'
```

## 同じ配置での候補切替

`summarize`で評価を確定し、終了時のclock/governorと各チームの追記ログを保存した後に、`switch.py`を使う。分散3台の専用プロセスだけを停止し、候補の全ソースと検証済みバイナリのSHAを照合してから、同じディレクトリへ配置する。固定NOMLAB・ローカル2体・launcher・CPU割当・時計同期は維持する。

```sh
python3 solver/distributed-lab/switch.py c007-compact --validate-only
python3 solver/distributed-lab/switch.py c007-compact
# 採用D0へ戻す場合
python3 solver/distributed-lab/switch.py D0
```

C010は `switch.py c010-cross-elite --cross-elite on`（共有無効の診断は `off`）と明示する。3台を停止した後、同じバイナリと同じ共有設定のlauncherを配置する。版名にon/offを付け、実launcherのSHAと生成内容を切替記録へ保存する。元の配置記録から毎回生成するため、D0や他候補へ戻す際に共有引数を残さない。CPU・ディレクトリが初期配置から変わっている場合や、初期launcherのSHA不一致は拒否する。

C012の独立受信担当方式も `switch.py c012-isolated-cooperation --cross-elite on` と明示する。C010と同様に3台同じ設定を必須とし、それ以外の候補へ共有引数を付けることは拒否する。実際の配置は候補のビルド・検証記録と全ソース/バイナリ識別が揃ってから行う。

切替記録は`.server/distributed/switches/<日時>-<版>/`。評価中と未確定の評価がある間は拒否する。転送途中の失敗を自動rollbackで隠さず、専用プロセスを停止して記録する。失敗時は`switch.json`と実配置を確認して復旧し、整合するsnapshotが取れるまで次の評価を始めない。

問題serverのCPUは親プロセスのaffinityを継承するため、再開評価では`taskset -c 1,3,5,7 python3 solver/distributed-lab/manage.py run ...`で起動する。ローカル固定2体はCPU 0・2。SMT siblingは実機で確認する。最初の復元記録では4・6、2026年10月9日夜のRyzen 7 5800Hでは8・10であり、どちらもserverの集合から外れる。再起動やPC構成・電力設定の変化は`environmentIdentity`で区別し、同じ新環境で採用D0も再評価する。

## 観測と反復比較

`observe.py STAGE mark`で起動前のログ境界を保存し、`manage.py run`で開始する。実試合中に一度だけ`observe.py STAGE start`で探索スレッドのCPU時間増分を観測する。終了後は`summarize`と`observe.py STAGE end`で前後照合・ログ追記・電力設定・時計を保存してから、`record_result.py STAGE`で履歴識別と全workerの完了を記録する。進行中に重いビルドやGPU測定を追加しない。

VOID2/3の終了時観測はmark以降の全追記を取得する。VOID1のsolver stderrはbridgeが試合ごとに上書きするため、終了時には最後の試合だけが残る。開始時の追記ログはサイズ上限があり、各ホストで観測日数が異なることもある。通信カウンタには必ず対象ホスト・日数・範囲を併記し、全試合の成績改善回数と解釈しない。

複数回の反復は次の形で集計する。対応する順番で基準・候補を渡す。

```sh
taskset -c 3 python3 solver/distributed-lab/compare_repeats.py \
  --baseline H2-D0-short-01 H2-D0-short-02 \
  --candidate H2-C012-on-short-01 H2-C012-on-short-02 \
  --save solver/docs/distributed/resume-20261009/restart-C012-02/paired-short-02.json
```

この集計は全48AC・確定済み結果だけを受け付け、環境・固定相手・時間・実際の問題ファイルの一致、同一候補内の実装/設定固定を検査する。全体と形式別順位、3得点、各組の勝同敗、seedごとの反復平均得点の勝同敗を保存する。現在の実装を変更せず保存結果を読む補助ツールであり、統計的有意差や採用を自動判定しない。

## ローカル検証

```sh
python3 -m unittest discover -s solver/distributed-lab -p 'test_*.py' -v
```

テストは保存済み正式runの集計、条件違反の拒否、前後hash/boot ID照合、進行中の設定変更拒否を対象にする。実機のSSH・分散通信・4チーム対戦はこの単体試験に含まれない。
