# 分散初期版の実装

採用D0を本番へ配置する具体的な手順は [ソルバPCのセットアップ](../../../tester/server/docs/solver-pc.md#分散ソルバd0を3台で本番に使う) を参照する。[配布版の詳細](deployment.md)も保存している。以下は実装・通信・探索構成の説明。

## 開始版と独立ビルド

共通開始版は `solver/parallel-pool/generation-003-P0`。正式12件 `run1791406043071407716-280358` で6体全員AC、P0平均順位1.6667で単独首位だった版を用いる。元の開始版には変更を加えていない。

独立workspaceは `solver/distributed/`。開始版から探索本体 `solver/planner.rs` と必要な公開テスタ基盤（model・engine・generator、公式問題・練習場fixture）をコピーした。保護された対戦サンプル・pool保存版・他の固定対戦相手の内部実装は参照・コピーしていない。ビルド出力は専用 `.gitignore` で除外する。

開始版の識別用SHA-256:

| ファイル | SHA-256 |
| --- | --- |
| `solver/planner.rs` | `46ececb49a65b4eead87ba76f4cb4078dd320908d189a723edce0dac8f07cc91` |
| `main.rs` | `6848c38e33d57b426fe5ebb568b7392b3c3adabaf3af4b88af408da3abbdf806` |
| `common/src/engine.rs` | `e02a3335c87475295692a8d6de54110cb33466279cddd94957c57f645f698a49` |

```sh
cargo build --release --manifest-path solver/distributed/Cargo.toml
```

実行ファイルは `solver/distributed/target/release/distributed-solver`。依存はworkspace内の小さい `tester` crateとserde/serde_json等のみ。開始版の探索本体は同じまま、Gameのprivateフィールドを含む状態の直列化を可能にした。

## 配置とスレッド数

| ホスト | 制御 | 探索 | 内部合計 | 探索ID |
| --- | ---: | ---: | ---: | --- |
| VOIDLINUX-1 | 1 | 31 | 32 | 0〜30 |
| VOIDLINUX-2 | 1 | 15 | 16 | 31〜45 |
| VOIDLINUX-3 | 1 | 31 | 32 | 46〜76 |
| 分散チーム合計 | 3 | 77 | 80 | 0〜76 |

制御スレッドは各プロセスのmainのみ。VOIDLINUX-1のmainは通常探索の起動に加え、nonblocking TCP 2本の送受信、候補検証・集約、JSONL回答を処理する。通信専用スレッド、非同期runtimeの隠れたthread pool、要求ごとの探索スレッド追加はない。探索スレッドを全機で常駐させ、接続が切れても上限を維持する。server-bridgeは別プロセスとしてこの数から除外する。

通常探索の乱数seedは開始版と同じ式にグローバル探索IDを入れて分散させる。温度・ビーム重みは7通り、燃料余裕減点は7通りを循環する。同じ機内・同じ燃料減点の探索は当日のeliteを共有する。機間では1要求につき各機の最良候補を交換する。GPU処理はこのCPU初期版には含まれない。

## 起動設定

全モードで以下を固定する。

```sh
export PROCON37_G007_FUTURE_WEIGHT=0
export PROCON37_G007_RESERVE_WEIGHT=15
```

分散3台では同じ秘密文字列を含む権限600のファイルを配置し、`PROCON37_DISTRIBUTED_TOKEN_FILE` をその絶対パスへ設定する。32〜128文字の印字可能ASCIIを受け付け、実配置は64桁hexを用いる。秘密文字列そのものをCLI引数・ログ・Gitへ入れない。

```sh
# VOIDLINUX-2
./distributed-solver --mode worker --threads 16 --worker-offset 31 \
  --listen 192.168.0.22:39370

# VOIDLINUX-3
./distributed-solver --mode worker --threads 32 --worker-offset 46 \
  --listen 192.168.0.33:39370

# VOIDLINUX-1: このプロセスのstdin/stdoutだけをteam0のbridgeへ接続
./distributed-solver --mode coordinator --threads 32 --seed 0 \
  --peers '192.168.0.22:39370/16/31,192.168.0.33:39370/32/46'

# NOMLAB: 固定比較相手、ネットワーク通信なし
./distributed-solver --mode parallel --threads 16 --seed 0
```

peersの各要素は `IP:PORT/内部総スレッド数/先頭探索ID`。起動時に総計80以内、探索IDの重複なし、peer最大2台を検証する。接続後もdaemonの申告するプロトコル版・thread数・offsetと設定が一致しなければ候補を採用しない。実際のCPU affinity・接続ユーザー・秘密ファイルの配置は `tools/` の配置記録を参照する。

## NOMLAB比較版の差分

`--mode parallel --threads 16` は開始版の `PoolSolver` をそのまま使う。探索・評価・車種選択・翌日のrollout・候補選択・同期・60ms控除は変更していない。元の制御1＋探索14を制御1＋探索15へ適応するため、次の変更だけを `PoolSolver` に加えた。

1. constructorで内部総スレッド数を受け取り、探索数を `total_threads - 1` にする。
2. 14本超でも配列境界を越えないよう、燃料減点と共有eliteの組番号を7で循環する。既存の探索ID 0〜13は同じ係数・seed・組番号を保つ。追加のID14は最初の燃料減点組を使う。

共通の追加は、`Solution`・`Snapshot`・`Game` のserde derive、Snapshotをcrate内から参照できるようにする変更、分散管理moduleの登録のみ。mainはモード分岐を追加したが、parallel側のJSONL応答形式と選択後のPoolSolverライフサイクルは開始版を保つ。分散用の通信予算控除や期限打切りを比較モードへ適用していない。serde_jsonのfloat_roundtripを有効にし、分散状態の浮動小数点値が転送で変わらないようにした。

## 採用プランと状態同期

snapshotは「当日の採用プランをまだ実行していない」状態。以下を丸ごと転送する。

- Gameのproblem、agents、day、score、取得済み種類集合、交通履歴、実観測交通。
- `last`（実際に提出する採用プラン）、supplier、取得済み種類mask。
- ウォームスタート用の当日・翌日Solution、座標対応、日番号・完了flag。
- 相手の前日滞在推定と交通予測用データ。

集約後、coordinatorは最良候補のanswerとsnapshotを同じ組として採用する。次の要求ではこの採用snapshotを全機・全探索へ渡し、各探索は探索開始前に自分の非採用履歴・cacheを上書きする。待機中の非採用cacheを翌日の探索に流用する経路はない。新しい盤面と採用snapshotを毎要求に含めるので、daemon再起動後にも当日の入力だけで復元できる。

前日プランの成否は既存JSONLの `previousAccepted` を使用する。承認なら採用プランを内部Gameで実行し、却下なら待機プランを実行して履歴を進め、最後に実観測の位置・燃料・日番号・交通を適用する。この順序で取得種類と滞在履歴も実際の採用・却下結果にそろう。

候補のanswerとsnapshot内lastが一致すること、内部Gameの実行前状態が採用済み履歴から復元した状態と完全一致すること、公開engineで当日のplanが合法であることを検証する。cache座標・hub・path mode・route tokenの範囲も確認する。request IDや入力digestが合わない古い候補は捨てる。

## 通信と締切

TCP上の4byte big-endian長さ＋JSONを用いる。1frameは最大4MiB、送信待ちqueueは最大8MiB、peer数は最大2。最初の認証frameでversionと共通秘密を照合し、成功後にだけ探索要求を受け付ける。通信内容は暗号化しないため、実験LANの指定ホストIPへだけbindする。

各要求はcoordinator起動ごとのsession ID、単調増加する内部request ID、Task全体の整合性digestを持つ。digestはFNV-1aによる診断・取り違え防止用で、認証には共通秘密を用いる。daemonは同一sessionの重複・過去request、600秒超の予算、設定不一致を拒否する。未認証接続は5秒、認証済みidle接続は180秒で閉じる。

要求時間の5%（250〜1500ms、短い要求では全時間以内）を余裕として取り、探索期限をそこから手前に置く。遠隔探索にはさらに150msを控除する。最終収集は外部期限より余裕の半分だけ手前で打ち切る。接続試行は各peer最大45ms、通信poll間隔は2ms。時間は各機のmonotonic clockで計測するので時計の同期を要求しない。

各探索は同時に1要求だけ処理する。前要求が遅れて終わった場合はその結果を破棄し、空いた探索を次要求に使う。古い要求をqueueへ積み上げない。worker panicは捕捉し、次要求時にSolverを作り直す。通信断時は得られた他機・ローカル候補から選択する。全候補がない場合にも、採用済み履歴から生成した合法な待機案と対応snapshotを提出する。250ms以下のcatch-up要求では探索せず同じ状態更新だけを行う。

探索本体の一部のgraph構築等は開始版同様に途中で中断できない。集約側の締切はその完了を待たず、実行中thread数の上限を保って次要求へ進む。

## 診断と検証

stderrには各要求の外部/内部ID、入力digest、採用worker、評価値、ローカル完了数、遠隔応答数、時間、採用状態digestを出力する。遠隔応答には開始/完了探索数と各機の探索時間を含める。TCPのrx/tx byteは接続ごとの累積値。実機で3台の探索参加、通信遅延、採用機、状態一致を追跡できる。

```sh
cargo test --release --manifest-path solver/distributed/Cargo.toml \
  --bin distributed-solver -- --test-threads=2
cargo test --release --manifest-path solver/distributed/Cargo.toml \
  --test network -- --nocapture
```

localhostへのbind/connectが制限された実行sandboxでは、通信テストだけネットワーク許可のある実行環境で行う。

初期検証では以下を確認した。

- 11単体テスト: 開始版の公開engine整合性・役割変更・catch-upに加え、採用状態のJSON往復、取得種類・交通履歴・前日却下の同期、遅延結果の排除、分割TCP frame・最大長制限、認証一致。
- 実プロセス試験: coordinator＋daemon2台を起動し、認証失敗、遠隔候補の実採用、前日planの却下、片側daemon停止中の継続、再起動後の復帰を検証。全日planを公開engineで確認。`/proc/PID/task` で小構成の総thread数2/3/3を確認した。
- 比較モード試験: `/proc/PID/task` で16threadを確認し、selectとplanのJSONLおよび公開engineの受理を確認する。

最初の実プロセス試験ログは `/tmp/procon37-dist-test-300494-1791413168288/`。coordinator PID 300502、daemon PID 300496・300499、再起動daemon PID 300504。全プロセスは試験終了時に停止済みで、各 `.process.json` に記録した。この試験は1要求900msの通信・同期検証であり、serverでの指定12件評価の代替ではない。最終の検証ログと実行ファイルSHAは親への凍結連絡に記載する。
