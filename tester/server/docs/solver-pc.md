# ソルバPCのセットアップ

[対戦サーバー](../README.md)（`server`）の対戦に、ソルバを動かすPC（ソルバPC）を参加させるための手順。
ソルバPCは参加するチームごとに用意する。サーバーPCの準備は [サーバーPCのセットアップ](server-pc.md) を参照する。

**VOID1・VOID2・VOID3で1チームとして動かす場合は、[分散ソルバD0を3台で本番に使う](#分散ソルバd0を3台で本番に使う)へ進む。** VOID1から配置・起動・停止をまとめて操作する。以下の1〜6は単一PC構成の手順。

ソルバPCは公式API（`/setting`・`/agent`・`/`）だけを使う。
練習用の対戦サーバーと本番の競技サーバーには、同じ `run.sh` で参加でき、違うのは接続先とトークンだけ（[本番で使う](#本番で使う)）。

```
[ソルバPC] チームごとに1台                        [サーバーPC]（練習）／ 競技サーバー（本番）
 run.sh（常駐）
   └ server-bridge  ──────── HTTP (LAN) ────────▶  /setting・/agent・/
        └ solver
```

ソルバPCから開始するのは、サーバーへのHTTP通信だけ。
サーバー側からソルバPCへ接続することはないので、単一PC構成ではソルバPC側でポートを開ける必要はない。
分散構成では、VOID1からVOID2/3への内部通信にTCP 39370を使う。

## 1. 必要なもの（ソルバPC）

| 項目 | 内容 |
| --- | --- |
| OS | Linux / WSL / macOS |
| Rust | 1.88以上（`rustc --version` で確認）。[rustup](https://rustup.rs/) で導入する |
| 時刻同期 | OSの自動時刻合わせを有効にする（[4. 時刻を合わせる](#4-時刻を合わせる)） |
| ネットワーク | サーバーと同じLANにつなぐ（本番は有線LANのみ）。初回ビルド時のみ、依存crateの取得にインターネット接続が必要 |

Rustが未導入の場合（`curl` が無いPCでは1行目を `wget -qO- https://sh.rustup.rs | sh` にする）：

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
rustc --version   # 1.88 以上であること
```

## 2. ソルバとserver-bridgeをビルドする

リポジトリのルートで、対戦させたいブランチのソルバをビルドする。

```sh
cargo build --release -p solver -p server-bridge
```

次の2つが生成される。`run.sh` は既定でこの2つを使う。

| ファイル | 内容 |
| --- | --- |
| `target/release/solver` | ソルバ本体 |
| `target/release/server-bridge` | 公式APIとソルバの入出力の変換器 |

## 3. サーバーへつながるか確認する

サーバーPCの担当者からチーム番号を受け取り、サーバーへ接続できるか確認する（`curl` が無ければ `wget -qO- --content-on-error` でもよい）。

```sh
curl http://192.168.0.81:11451/setting?token=token-p0
# {"code":"AccessTimeError","message":"match has ended"}  ← 試合が無いときの応答。何か返れば接続できている
```

試合中なら問題のJSONが返る。つながらない場合は [トラブルシューティング](#トラブルシューティング) を参照する。

## 4. 時刻を合わせる

`server-bridge` は、サーバーが返す各日の締切時刻（UNIX時刻）と**ソルバPCの時計**の差から、ソルバに渡す制限時間を計算する。
ソルバPCの時計が遅れていると、ソルバが締切を過ぎてから回答して、その日が全車待機（TLE）になる。
時計が進んでいると、探索時間が本来より短くなる。本番の競技サーバーはNICTのNTPに同期している（問い合わせと回答その2 A57）。

| OS | 確認方法 |
| --- | --- |
| Linux | `timedatectl` で `System clock synchronized: yes` |
| macOS | システム設定 → 一般 → 日付と時刻 →「時刻と日付を自動的に設定」をオン |
| WSL2 | スリープ復帰後に時計がずれることがある。`date` をWindows側の時刻と比べ、ずれていれば `sudo hwclock -s` か `wsl --shutdown` で再起動する |

サーバーPCとソルバPCで、`date +%s` の差が1秒未満であればよい。

## 5. run.shを起動する

リポジトリのルートで、サーバーPCの担当者から受け取ったチーム番号（0, 1, 2, …）を `TEAM` に指定して起動する。

```sh
TEAM=0 ./tester/server-bridge/run.sh
```

他の設定は既定値のままでよい。必要なときだけ環境変数で変更する。

| 環境変数 | 内容 | 既定値 |
| --- | --- | --- |
| `TEAM` | 練習用の対戦サーバーでのチーム番号 | `0` |
| `TOKEN` | チームのトークン | `token-p<TEAM>` |
| `SERVER` | サーバーのURL | `http://192.168.0.81:11451` |
| `SELECT_MS` | 車種選択の制限時間（ms） | マップの大きさから本選の時間（下表） |
| `SOLVER` | ソルバ実行ファイル | リポジトリの `target/release/solver` |
| `BRIDGE` | server-bridge 実行ファイル | リポジトリの `target/release/server-bridge` |
| `RECORD_DIR` | 試合データの保存先（単一PC版・三台分散版で共通） | リポジトリの `.server/submissions` |

`run.sh` は起動したまま待機し、試合が始まる（`/setting` で問題が取得できるようになる）たびに自動でソルバを実行する。
試合ごとにソルバを起動し直すので、12件などの連続した試合でも起動したままでよい。止めるときは Ctrl+C。
試合開始より前に起動しておく。車種選択の時間は、問題を最初に取得した時刻から数えるため。

実行ログは英語で出力する。試合ごとに次のようなログが出る（数値は例）。

```
[bridge] connection_check server=http://192.168.0.81:11451 path=/setting
[bridge] server_connection result=reachable http_status=403 request_ms=2
[bridge] problem_wait http_status=403 state=not_ready
[bridge] problem_received width=16 height=16 agents=4 days=5 players=2 spots=14 brands=7 fuel_limit=100 busy_threshold=3 jammed_threshold=6
[bridge] schedule day_steps=[32, 32, 32, 32, 32] day_seconds=[10, 10, 10, 10, 10] selection_remaining_ms=60000 starts_at=0 late_join=false match_id=none
[bridge] solve_start phase=select request_id=0 limit_ms=59500
[bridge] solve_end phase=select solver_ms=59000 limit_ms=59500 result=ok fallback=false failure_kind=none
[bridge] vehicle_selection kinds=[0,1,0,1] fallback=false
[bridge] selection_submission result=accepted http_status=200 solver_ms=59000 submit_ms=2 attempts=1 fallback=false
[bridge] day_state day=1 ends_at=1800000000 remaining_ms=9000 previous_accepted=true
[bridge] solve_start phase=plan day=1 request_id=1 steps=32 limit_ms=8500
[bridge] solve_end phase=plan day=1 solver_ms=8000 limit_ms=8500 result=ok failure_kind=none
[bridge] submission_attempt path=/ attempt=1 remaining_ms=900
[bridge] submission_response path=/ http_status=200 revision=1 attempts=1 submit_ms=2
[bridge] plan_submission day=1 solver_ms=8000 http_status=200 revision=1 result=accepted http_accepted=true submit_ms=2 attempts=1
...
[bridge] match_complete elapsed_ms=110000
[run.sh] Waiting for the next match.
```

bridgeとソルバの実行ログは、`run.sh` を起動した端末にそのまま表示される。実行ログをファイルにも保存したい場合は `TEAM=0 ./tester/server-bridge/run.sh >run.log 2>&1` のように明示的に出力先を指定する。問題・観測状態・提出通信は下記のとおり自動保存する。

| ログ・フィールド | 意味 |
| --- | --- |
| `connection_check` / `server_connection` | 通常の `/setting` 要求で接続確認する。追加の接続テスト用HTTP要求は送らない。`reachable` は応答が返ったことを示し、認証成功や問題公開を保証しない |
| `authentication_failed` / `problem_wait` | トークン拒否、または問題の公開待ち。待機中はHTTP状態が変わったときだけ表示する |
| `problem_received` / `schedule` | 盤面サイズ、車両数、日数、プレイヤー数、補給地点数・種類数、燃料上限、混雑の閾値、日別ステップ数・秒数、選択締切、途中参加、試合ID |
| `solve_start` / `solve_end` | 車種選択・日別探索の開始と終了、制限時間、失敗種別。`solver_ms` はソルバの応答を待った実時間 |
| `vehicle_selection` / `fallback` | 提出する車種一覧。選択に失敗した場合は全車種0で代替し、`fallback=true` と表示する |
| `submission_attempt` / `submission_response` | 提出の試行、締切までの残り時間、HTTP応答、`revision`、試行回数。`submit_ms` は間隔調整・再送を含む提出処理の実時間 |
| `selection_submission` / `plan_submission` の `result` | `accepted` は受理、`rejected` は却下、`unknown` は応答消失や締切をまたいだ受理などで採否不明、`not_submitted` は未提出。HTTP 200だけでは日別計画の受理と判定しない |
| `connection_lost` / `connection_restored` / `submission_retry` | 通信断・上流サーバーの502〜504、復旧までの `outage_ms`、提出の再送 |
| `submission_confirmed` / `safety_submission` / `catch_up` / `warning` | 採否不明の提出を翌日の状態から確認した結果、安全のための待機計画、飛ばした日への追従、状態の不一致 |
| `match_complete` の `elapsed_ms` | 問題取得から試合終了確認までの実時間。探索・通信・日別待機を含む |

すべての時間フィールドの単位はms（`day_seconds` とUNIX秒の `starts_at` / `ends_at` を除く）。ソルバのJSON回答は通信に使い、端末へ混ぜない。

### 試合データの保存

単一PC版・三台分散版とも、問題を取得した時点で `.server/submissions/<取得時刻のUNIXミリ秒>-<PID>-<連番>/` を作る。同じマップを続けて対戦した場合や途中で再起動した場合も、以前の記録を上書きしない。分散版ではVOID1だけが保存し、保存先は配置ディレクトリから独立している。`.server/` はGit管理外である。

| ファイル | 内容 |
| --- | --- |
| `metadata.json` | 形式バージョン、取得時刻、接続先、試合ID（取得できる場合）、車種選択締切、途中参加の有無 |
| `problem.json` | `/setting` で取得した問題全体。マップ・補給地点・車両初期位置・日程・燃料上限などと、サーバーが追加したフィールドも保存 |
| `map.json` | 問題の `map`。`width`・`height`・地形配列 `cells` |
| `observations.jsonl` | 各日の処理に使った実観測。1行1JSONで `usedAtMs` と `state` を保存し、`state` に `day`・`endsAt`・`agents`・`others`・道路状態 `traffics` を含む |
| `http.jsonl` | 問題取得後のHTTP要求・応答・通信失敗。取得時の `/setting` 応答、車種・行動計画の提出内容、再送、HTTP状態、`revision` などを時刻付きで保存 |

`http.jsonl` の `event=request` は通信を試みる直前の記録。記録処理後に締切を過ぎた場合は `event=not_sent`、応答を受け取れなければ `event=transport_error` が続く。終了確認や別試合の検出に使う応答も含むため、各日の道路状態を調べるときは `observations.jsonl` を使う。通信断で取得できなかった日の推定状態は、実観測として保存しない。

取得したデータは順次書き込み、終了処理まで待たずに保存する。保存先の作成や書き込みに失敗すると、英語の `recording_failed ... submissions_continue=true` を表示して保存を中断し、提出は継続する。保存場所は `recording_started directory=...` に表示する。チームトークンを保存するフィールドは設けない。

保存先を変更する場合は、絶対パスで指定する。

```sh
RECORD_DIR=/path/to/match-data ./tester/server-bridge/run.sh
RECORD_DIR=/path/to/match-data ./tester/server-bridge/distributed/run.sh
```

bridgeを直接実行する場合は `--record-dir /path/to/match-data` を使う。分散版の保存先は起動時設定 `runtime.json` に保持され、変更時は試合間に停止・再起動する。

長時間の対戦では、SSHの切断で `run.sh` が止まらないように `tmux` や `screen` の中で起動するとよい。

### 制限時間と通信

| 対象 | 制限時間 |
| --- | --- |
| 車種選択 | 問題を最初に取得した時刻から、16×16は60秒・24×24は90秒・32×32は120秒（[本選マップに関するお知らせ](../../../official/本選マップに関するお知らせ.pdf)）。`SELECT_MS` で上書きできる |
| 各日 | サーバーが返す締切 `endsAt` まで |

どちらも、締切までの残り時間から通信の余裕として500msを引いてソルバへ渡す。
リクエストは200ms以上の間隔を空けて送る（公式の目安「秒間5リクエストほど」、問い合わせと回答その2 A42）。

通信できなくなっても（ケーブルが抜けた場合など）試合は諦めず、再接続を待つ。提出はその日の締切まで再送する。
その日のうちに復旧すれば、ソルバはそのまま動き続け、途中の日の記憶（先読みした計画など）も失わない。

**既知の制限**：通信断が日をまたぎ、状態を1日分も受け取れなかった日があると、ソルバは手元の予測と照合できずに終了し、その試合の残りの日は回答できない（全車待機になる）。次の試合からは通常どおり参加する。
練習用の対戦サーバーで制限時間を短縮しているときは、サーバーが車種選択の残り時間を知らせるので、`SELECT_MS` の指定は不要。

## 6. ソルバを更新したとき

ブランチを切り替えたり最新にしたりした後、ビルドし直す。

```sh
cargo build --release -p solver -p server-bridge
```

`run.sh` は試合ごとに実行ファイルを読み込み直すので、試合の合間であれば再起動しなくても次の試合から新しいソルバが使われる。
試合中にビルドすると、その試合のソルバは古いまま最後まで動く。確実に切り替えたいときは、試合が無いときにビルドする。

## 本番で使う

本番も同じ `run.sh` を使う。当日に連絡される接続先と、配布されたトークンを、`run.sh` 冒頭の「本番の設定」に書き込む。

```sh
# ===== 本番の設定: 本番ではこの3行を書き換える =====
SERVER="${SERVER:-http://192.168.0.81:11451}"  # ← http://192.168.0.81:11451 を競技サーバーのURLに
TOKEN="${TOKEN:-token-p${TEAM:-0}}"           # ← token-p${TEAM:-0} を配布されたトークンに
SELECT_MS="${SELECT_MS:-}"                     # ← 予行演習などで車種選択の時間が告知されたら、その値(ms)に
```

たとえば接続先が `http://10.0.0.1:8080`、トークンが `abc123` なら、次のようになる。

```sh
SERVER="${SERVER:-http://10.0.0.1:8080}"
TOKEN="${TOKEN:-abc123}"
SELECT_MS="${SELECT_MS:-}"
```

書き換えたら、環境変数を付けずに起動する。

```sh
./tester/server-bridge/run.sh
```

起動すると `http://10.0.0.1:8080 の試合を待っています` と表示される。接続先を確かめてから試合開始を待つ。
書き換えた `run.sh` はコミットしない。大会後は `git checkout -- tester/server-bridge/run.sh` で元に戻す。

| 項目 | 練習（対戦サーバー） | 本番（競技サーバー） |
| --- | --- | --- |
| 接続先 | サーバーPCの対戦窓口（既定値） | `run.sh` の `SERVER` を書き換える |
| トークン | `TEAM` から `token-p<TEAM>` | `run.sh` の `TOKEN` を書き換える |
| 起動 | `TEAM=0 ./tester/server-bridge/run.sh` | `./tester/server-bridge/run.sh` |
| 使うAPI | `/setting`・`/agent`・`/` | 同じ |
| 試合の検知 | `/setting` が問題を返し始めたら開始 | 同じ（本選マップのお知らせ「試合開始と同時に問題データが取得可能」） |
| 車種選択の時間 | マップの大きさから本選の時間。短縮時はサーバーが残り時間を知らせる | マップの大きさから本選の時間 |
| 通信が途切れたとき | 再接続を待って続行（日をまたぐとソルバが止まる。[既知の制限](#制限時間と通信)） | 同じ |

注意すること：

- **予行演習**では制限時間が本選より短くなることがある（本選実施要項 3.2）。車種選択の時間が告知されたら `SELECT_MS` を書き換える。そのままだと本選の時間で探索し、締切に間に合わない。
- 本番のネットワークは有線LANのみで、Wi-Fiは使えない（募集要項）。
- 試合開始より前に `run.sh` を起動しておく。
- トークンが誤っていると `トークンが認められません (HTTP 401)` と表示して `run.sh` が終了する。

## トラブルシューティング

| 症状 | 原因と対処 |
| --- | --- |
| 接続確認がタイムアウトする | 別ネットワーク（ゲスト用Wi-Fiなど）につながっている。またはサーバーPC側の設定（[サーバーPCのセットアップ](server-pc.md)） |
| 接続確認が `Connection refused` | サーバーPCで対戦サーバーが起動していない、またはURLのポート番号が違う |
| `waiting_for_problem` / `problem_wait` のまま進まない | 試合がまだ始まっていない（正常）。サーバーPCで対戦が開始されるのを待つ |
| `connection_lost retrying=true` | サーバー停止、ケーブル切断、または上流サーバーの問題。復旧すると `connection_restored` を表示して自動で続行する |
| `Executable not found` | ビルドしていない。[2.](#2-ソルバとserver-bridgeをビルドする) のコマンドを実行する |
| `authentication_failed http_status=401` | `TEAM` がチーム数以上、または `TOKEN` の誤り。チーム数が2なら `TEAM` は0か1 |
| `Failed to start the solver` | ソルバの実行ファイル・実行権限・作業ディレクトリを確認する |
| `selection_submission result=rejected http_status=403` | 車種選択の締切に間に合わなかった。試合開始後に `run.sh` を起動した、または `SELECT_MS` が実際より長い |
| `submission_end reason=deadline`（`result` などのフィールドを挟む） | 締切までに提出の応答を受け取れなかった。通信・残り時間と `result` を確認する。次の日からは再接続後に回答を続ける |
| 毎日 `solve_end result=failed`（`phase` などのフィールドを挟む）や TLE | ソルバの失敗種別を確認する。時計の遅れ（[4.](#4-時刻を合わせる)）やPCの性能不足で回答が間に合っていない場合もある |
| `cargo build` で `requires rustc 1.88` などのエラー | Rustが古い。`rustup update` を実行する |
| 同じチームで2つの `run.sh` を起動した | 両方が提出し、後の提出が採用される。ソルバは予測と違う状態を受け取って止まることがある。1チームにつき1つだけ起動する |

## 配置済みC013を3台で本番に使う

**当日のDHCP対応とトークン設定は完了済み。** VOID1から `./tester/server-bridge/distributed/run.sh practice`（予行演習30秒）、`match`（本試合60秒）、`test`（接続テスト20秒）で起動する。引数なしは予行演習。切替前に停止する。PC間は `voidlinux-1.local`〜`voidlinux-3.local` で接続する。[最新の当日操作](../../server-bridge/competition/docs/operation.md#当日の操作)を参照する。

2026年10月10日の16×16試合用にはC013を選定した。VOID1の `/home/voidlinux-1/procon37` にリポジトリ全体を配置し、評価済みの実行ファイルへ接続してある。`tester/server-bridge/distributed/run.sh` 冒頭の `SERVER`・`TOKEN` を当日指定の値へ編集する。

```sh
cd /home/voidlinux-1/procon37
./tester/server-bridge/distributed/run.sh
```

VOID2/3もこの入口から起動する。`status`・`stop` を付ければ状態確認・停止ができる。コンパイル・配置は完了済みなので、下記のD0用手順を重ねて実行する必要はない。問題・マップは `.server/submissions/`、完全ログは `.server/bridge-logs/` のリンクから参照でき、実体は選択した不変リリース内に保存される。[C013の操作と選択設定](../../server-bridge/distributed/README.md#配置済みc013を使う2026年10月10日) を参照する。

## 分散ソルバD0を3台で本番に使う

以下は従来D0配置の手順。現在のC013配置は上記の入口を使う。

採用済みCPU分散版 **D0** を使う手順。操作はすべて **VOID1の `voidlinux-1` ユーザー**で、リポジトリのルートから行う。VOID2/3にはリポジトリを置かなくてよい。三台が設定済みの有線LANでスイッチングハブにつながり、VOID1にリポジトリ・Rust環境・依存crateのCargoキャッシュがあれば、Wi-Fi・インターネット接続なしでコンパイルと配置ができる。当日指定の競技サーバーURLとチームトークンは、分散用 `run.sh` 冒頭に設定する。

[compile.sh・deploy.sh・run.sh](../../server-bridge/distributed/README.md) に本番操作をまとめている。コンパイル、三台へのコピー、共有認証、CPU割当て、起動順、試合間の待機、停止はスクリプトが扱う。既存のRust・Linux・Python 3.10以上・OpenSSH・`ip`・`tar` を利用する。依存crateは事前に取得し、当日はオフラインでコンパイルする。`tmux` の操作は不要。

### コンパイル → deploy.sh → run.sh

まず三台を起動し、LANケーブルとスイッチングハブを接続する。VOID1の時計も[時刻を合わせる](#4-時刻を合わせる)の手順で確認する。有線接続できることと時刻が合っていることは別なので、会場で外部NTPに接続できない場合も事前の同期を確認する。

VOID1でリポジトリに移動し、コンパイル、配置、起動の順に操作する。`sudo` は付けない。コンパイルには既存Rust環境とCargoキャッシュを使う。**配置前には必ず `compile.sh` を実行する。**

```sh
cd /path/to/procon37
./tester/server-bridge/distributed/compile.sh
./tester/server-bridge/distributed/deploy.sh
```

`deploy.sh` はコンパイルで登録した実行ファイルを三台へ配布する。URL・チームトークンの入力は求めない。初期設定はSHAが未登録なので、コンパイルせずに配置しようとすると停止する。旧設定の `bundled:` 指定も停止するため、`compile.sh` で生成したファイルへ更新する。

配置後、[分散用run.sh](../../server-bridge/distributed/run.sh) 冒頭の「本番の設定」を当日指定の値に書き換える。単一PC用と同じ変数名を使う。

```sh
SERVER="${SERVER:-http://10.0.0.1:8080}"  # 当日指定の競技サーバーURL
TOKEN="${TOKEN:-abc123}"                 # 配布されたチームトークン
SELECT_MS="${SELECT_MS:-}"               # 通常は空。指定があればミリ秒数
```

既定値は練習用の `http://192.168.0.81:11451` / `token-p0`。環境変数での指定もできる。

```sh
SERVER=http://10.0.0.1:8080 TOKEN=abc123 ./tester/server-bridge/distributed/run.sh
```

VOID2/3のSSHパスワードは、VOID1のリポジトリ内 `.server/distributed-production/ssh-password` に設定する。Git管理外の1行ファイルで、今回のVOID1配置時に設定済みなので入力は不要。SSH鍵が使える場合はOpenSSHが先に鍵認証を試す。環境変数 `PROCON37_SSH_PASSWORD` での指定もできる。

`deploy.sh` はVOID1に配布パッケージを作り、VOID2/3へSSH/SCPで転送する。実行ファイル・設定・転送後のSHA-256、CPU番号、ライブラリ、トークンファイルを照合し、三台の配置完了を表示する。**配置だけでは試合に参加しない。**

配置完了後、試合開始前に `run.sh` を起動する。

```sh
./tester/server-bridge/distributed/run.sh
```

`run.sh` は三台のログを表示しながら待機する。状態確認は別端末で `./tester/server-bridge/distributed/run.sh status` を実行する。

起動順はVOID2 → VOID3 → VOID1。両workerへの有線TCP接続・共有認証・スレッド数・探索IDを確認してから、VOID1のbridgeを起動する。起動の途中で失敗した場合は、その操作で新しく起動した処理を停止する。三台とも起動済みなら、`start` をもう一度実行しても追加起動しない。

`status` は各機の `running`、`pid`、`identity_mismatch`、`orphans` を表示する。三台とも `running: true`、`identity_mismatch: false`、`orphans: []` が正常な起動状態。これはプロセスの確認であり、試合の提出受理・全探索の完了は下のログで確認する。

VOID1は1試合ごとにbridgeとcoordinatorを起動し、終了後は次の試合を待つ。VOID2/3はworkerとして常駐し、その標準出力・標準エラーもSSH経由でVOID1の端末へ流れる。`Ctrl+C` で今回起動した三台の処理を停止する。端末終了やSSH切断でも停止処理を行う。別端末から停止する場合は次のコマンドを使う。

```sh
./tester/server-bridge/distributed/run.sh stop
./tester/server-bridge/distributed/run.sh status
```

停止順はVOID1 → VOID2 → VOID3。記録した起動時刻・ユーザー・作業ディレクトリ・専用セッションを照合し、その処理と子プロセスを停止する。応答しない処理は待機後に強制停止し、終了を確認する。全機 `running: false`、`orphans: []` を確認する。LANが切れている機械は操作に失敗するので、接続を戻して `stop` を再実行する。停止コマンドは、開発の自動監視を再開しない。

### 固定構成と設定ファイル

| 機械 | 有線IP / SSHユーザー | 役割 | 使用CPU | 内部スレッド / 探索ID |
| --- | --- | --- | --- | --- |
| VOID1 | `192.168.0.11` / `voidlinux-1` | coordinator・bridge・公式提出 | `0-31` | 32 / 0〜30 |
| VOID2 | `192.168.0.22` / `voidlinux-2` | worker | `0-15` | 16 / 31〜45 |
| VOID3 | `192.168.0.33` / `voidlinux-3` | worker | `0-31` | 32 / 46〜76 |

内部合計80は制御3＋探索77。bridgeとOSの処理はこの数に含まない。**公式サーバーへ提出するのはVOID1のbridge 1個だけ。** VOID2/3では `run.sh` を起動しない。VOID1でも別の `run.sh` や旧評価用bridgeと同時に動かさない。

```text
競技サーバー ← HTTP → VOID1: bridge ← JSONL → coordinator
                                            ├ TCP 39370 → VOID2: worker
                                            └ TCP 39370 → VOID3: worker
```

初回操作でVOID1の `.server/distributed-production/config.json` を作る。[設定例](../../server-bridge/distributed-config.example.json)を元にし、次の項目だけ必要に応じて変更する。

| 項目 | 意味・既定値 |
| --- | --- |
| `server_url` | 互換用の保存項目。通常は変更せず、接続先は `run.sh` の `SERVER` で指定 |
| `remote_directory` | 各ユーザーのホーム下の配置先。`procon37-production/D0` |
| `nodes` | 上表のIP・SSHユーザー・CPU番号。SSH先も有線IPを使う |
| `worker_port` | 三台の内部TCP通信。`39370`。VOID2/3への接続を許可する |
| `select_ms` | 互換用の保存項目。通常は変更せず、選択時間は `run.sh` の `SELECT_MS` で指定 |
| `solver_binary` / `bridge_binary` | `compile.sh` が登録するコンパイル済み実行ファイルのパス |
| `solver_sha256` / `bridge_sha256` | 初期値は `null`。両方のコンパイル成功後に自動登録される識別値 |

スクリプトは車種選択・各日の探索時間を10秒に短縮せず、本番のbridgeから渡された時間で実行する。`SELECT_MS` や開発用の探索環境変数が端末に残っていても引き継がない。探索設定は評価済みの `FUTURE_WEIGHT=0` / `RESERVE_WEIGHT=15` を固定する。

**URL・トークン・選択時間を変更するときは、試合の合間に `run.sh stop` → `run.sh` の設定変更 → `run.sh` の順に行う。再コンパイル・再配置は不要。** 起動時の設定はVOID1の `runtime.json` と `private/competition.token` に保存し、配布版の設定やworkerを変更しない。起動中に異なる接続設定で `run.sh` を実行すると、停止してから変更するよう案内する。

IP・CPU・内部ポート・実行ファイルを変更するときは、古い設定のまま `run.sh stop` してから配布設定を変更する。`config.json` の `remote_directory` を `procon37-production/D0-next` など未使用の名前にし、`compile.sh` → `deploy.sh` → `run.sh` する。配置済みディレクトリは上書きしない。複数設定を保存する場合は、全操作で同じ `--config /path/to/config.json` を指定する。

### ログで提出と三台の参加を確認する

VOID1で起動した `run.sh` の端末に三台の実行ログが表示される。問題・マップ・日別観測・提出の通信記録は、VOID1の `.server/submissions/` に自動保存する（[保存形式](#試合データの保存)）。

| 出力元 | 端末に表示されるログ | 確認内容 |
| --- | --- | --- |
| 全機 | 起動・停止メッセージ | 起動エラー、認証エラー、常駐処理の終了 |
| VOID1 bridge | `[bridge] server_connection` / `problem_received` / `plan_submission` | 接続状態、問題の主要パラメータ、各日の `http_status` / `revision` / `result` / `solver_ms` / `submit_ms` |
| VOID1 coordinator | `dist mode=coordinator` / `dist request=...` | `local_completed`、`remote_results`、`selected_worker` |
| VOID2/3 worker | `dist worker` | 各要求の `started/completed`、接続・切断、認証 |

保存したい場合は、起動時に出力先を明示する。

```sh
./tester/server-bridge/distributed/run.sh >run.log 2>&1
```

同じ要求について、VOID1が `local_completed=31/31`・`remote_results=2/2`、VOID2が `completed=15/15`、VOID3が `completed=31/31` になっていることを確認する。`selected_worker` は0〜76が探索結果、`18446744073709551615` はfallbackを示す。正常なHTTP受理だけでは、両workerが探索に参加した証拠にはならない。車種選択と各日、終了直前を含めて端末の出力を確認する。

本番前には、同じ三台・同じ配置・有線接続で練習サーバーに参加し、三台の探索参加と提出受理を確認する。D0の採用版は正式12件・全48ACを確認済みだが、今回再コンパイルした版の三台対戦は未実施。2026-10-09には記録機能追加版のreleaseビルド、保存内容と保存失敗時の提出継続を含む模擬HTTP検証6ケース、運用テスト29件、三台への新配置 `procon37-production/D0-records-20261009` の配布・照合を確認した。新配置は未起動で、開発と自動監視はユーザー指示で一時停止したまま。

### 困ったとき

| 表示・症状 | 対処 |
| --- | --- |
| VOID1のユーザー・LANアドレスの検査で止まる | `voidlinux-1` で実行し、`ip -4 addr` と設定の `.11` を確認。`sudo` は使わない |
| SSHが接続できない | VOID2/3の電源・有線IP・設定済みsshd・認証を確認。Wi-Fiへ切り替える必要はない |
| `deploy` が既存ディレクトリで止まる | 既に配置済みなら `start` / `status`。更新・途中失敗のやり直しなら未使用の `remote_directory` へ配置する |
| SHA-256不一致・配置ファイル変更 | 実行せず、元ファイルと設定を確認して別ディレクトリへ再配置 |
| `worker` 認証・Ready照合が失敗 | 同じ `deploy` で三台を揃えたか、TCP 39370とCPU設定を確認。失敗時に残った処理は `stop` で確認 |
| `running: false` | `run.sh` を実行した端末の出力を確認する。トークン拒否ならVOID1は待機を終了する。`run.sh` のTOKENを確認し、試合間に停止・再起動 |
| `identity_mismatch: true` | 保存PIDが別のプロセスに再利用された。`runner.json` と該当PIDを確認する。スクリプトは別プロセスへ信号を送らない |
| `orphans` が空でない | 親終了後に残った同じ専用セッションの処理。`stop` して再確認する |
| 両workerが稼働しているのに `remote_results` が不足 | 端末上のworkerとcoordinatorの要求番号・完了数を照合し、有線接続・負荷を確認 |
| VOID1のbridge/coordinatorを試合途中で再起動した | それまでの提出履歴を復元できず、その試合で回答を続けられない場合がある。通常の更新・再起動は試合間に行う |

workerやSSH接続が終了した場合、`run.sh` は端末に原因を表示し、今回起動した残りの処理も停止する。接続と端末の出力を確認し、試合間に `run.sh stop` で残った処理を停止してから、`run.sh` を再実行する。接続できず停止を確認できなかったPCは表示されるので、復旧後に `stop` を再実行する。

### 配置前にコンパイルする

配置前にはVOID1の既存Rust環境でコンパイルし、三台で別々にビルドせず、VOID1で作った同一ファイルを配る。採用D0ソースは `solver/distributed`。未採用の `distributed-candidates/generation-*` を本番用として選ばない。

```sh
./tester/server-bridge/distributed/compile.sh
```

内部では以下の二つを `--release --locked --offline -j 2` でビルドする。CPU 4〜11を使用する。既存の実行ファイルがあっても両方のCargoビルドを呼ぶ。Cargo自身の差分ビルドは利用する。

- `solver/distributed/Cargo.toml` → `solver/distributed/target/release/distributed-solver`
- `server-bridge` → `target/release/server-bridge`

両方が成功した後、ファイルのパスとSHA-256を `.server/distributed-production/config.json` に自動登録する。SHAの手入力は不要。次の `deploy.sh` はこのビルド結果を配る。ビルド失敗時は配布元の設定を変更しない。

`--offline` は依存crateがVOID1のCargoキャッシュにある場合に使える。Rustが入っていることだけでは依存の取得済みまでは保証しないので、両方のビルドを持込み前に確認する。キャッシュが足りなければ事前に依存を取得する。コンパイル失敗時に同梱バイナリを代わりに配置する処理はない。

配置済み版を更新する場合は、古い設定のまま `run.sh stop` → コンパイル → 未使用の `remote_directory` に変更 → `deploy.sh` → `run.sh` の順に行う。再ビルドするとRust・依存・ビルド環境によってハッシュが変わり得る。旧正式評価と同じ実行ファイルとは扱わず、新しいファイルで練習参加を確認する。採用commit・同梱ファイルのSHA・詳細な保存先は[配布資料](../../../solver/docs/distributed/deployment.md)を参照する。
