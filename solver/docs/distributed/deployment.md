# 分散ソルバD0の配布版と運用資料

当日の操作は **[ソルバPCのセットアップ](../../../tester/server/docs/solver-pc.md#分散ソルバd0を3台で本番に使う)** に集約した。VOID1から [compile.sh・deploy.sh・run.sh](../../../tester/server-bridge/distributed/README.md) を使う。状態確認・停止は `run.sh status / stop`。本書は配布する版の識別と保存先を記録する。

## 配布する版

採用済みCPU分散D0の実装commitは `5d6c740962a1f3e20e77e13df243078df1d29ad2`、採用記録commitは `bf0807116eb882bf8cafc54ac01a535dcc26b6b4`。初回正式run `1791414715585964511-304414` は12件・全48判定AC、平均順位1.6667。[採用記録](initial-promotion.json)・[正式結果](initial-evaluation.md)を参照する。

[同梱アーカイブ](../../../tester/server-bridge/distributed-assets/README.md)は正式評価で使った実行ファイルのコピーを識別・照合用に保存したもの。初回の配置作業では再コンパイルしていないが、現在の本番手順はVOID1で `compile.sh` → `deploy.sh` の順に実行し、生成した実行ファイルを配布する。以下のSHAは評価時の記録であり、コンパイル後の識別値は配布設定に登録する。

| ファイル | SHA-256 |
| --- | --- |
| `distributed-solver` | `e80474cc5f65a30b18d8143eec656e3abe093c927d75080aed2bd9dce72a5de0` |
| `server-bridge` | `24cb1008dbb537f6941f08d2e9c31042ff5587a9215748dd9b8e8b15f933c15e` |
| `D0-binaries.tar.gz` | `6556afdbc07aedde2c31e0598de19f078bbce178395108c0c6c2d05b42217d7a` |

`solver/distributed` の47ソースファイルの識別hashは `b1834367734437c57537e9c58b1c6ecdc0f2e3d3a83295cfb6f0c1fd30cdd644`。初回正式評価の[事前照合](initial-formal-preflight.json)にファイル別SHAを保存している。未採用の001〜005候補は本番の配布元にしない。

## 構成と実行条件

| 機械 | 有線IP | 内部スレッド | 探索ID | CPU |
| --- | --- | ---: | --- | --- |
| VOID1 | `192.168.0.11` | 32 | 0〜30 | 0〜31 |
| VOID2 | `192.168.0.22` | 16 | 31〜45 | 0〜15 |
| VOID3 | `192.168.0.33` | 32 | 46〜76 | 0〜31 |

制御3＋探索77＝内部80。VOID1のbridgeだけが公式HTTP APIへ提出する。内部通信はTCP 39370、共有認証はwire version 1。GPU/OpenCLは使わない。CPU affinityは起動時に設定し、子プロセスにも継承する。探索環境は `PROCON37_G007_FUTURE_WEIGHT=0` と `PROCON37_G007_RESERVE_WEIGHT=15` を固定し、端末に残る開発用変数は引き継がない。

同梱バイナリはx86-64 Linux/glibc用。solverはGLIBC_2.34、bridgeはGLIBC_2.39までのシンボルを必要とする。VOID1にはglibc 2.39以上、VOID2/3には2.34以上が必要。全機で `/lib64/ld-linux-x86-64.so.2`、`libgcc_s.so.1`、`libm.so.6` を使用する。既存VOID環境を対象にしており、別CPU・musl環境へそのまま配布するものではない。

常駐の制御スクリプトはLinuxのPID記録とpidfdを使うため、既存Python 3.10以上とpidfd対応kernelを必要とする。現在の三台はこの条件を満たす。配置前にはVOID1のRust環境を使ってコンパイルする。依存crateのCargoキャッシュを事前に用意しておけば、会場でのWi-Fi・外部ダウンロード・tmuxは不要。

## 保存先

2026-10-09の記録機能追加版ではVOID1でD0・bridgeを再コンパイルし、三台へ `$HOME/procon37-production/D0-records-20261009/` として配置した。VOID1の `.server/distributed-production/config.json` はこの配置先を選択済み。前日の `$HOME/procon37-production/D0-terminal-20261008/` と今回の新配置は未起動・残存プロセスなしである。初期の `$HOME/procon37-production/D0/` はユーザー許可により停止済み。以下は既定名 `D0` で配置した場合の構成を示す。

VOID1のリポジトリ側:

```text
.server/distributed-production/
  config.json                配布設定（IP・CPU・配布元SHA）
  operator.lock              操作の二重実行防止
  D0-<時刻>.tar.gz           配布パッケージ（設定・スクリプト・実行ファイル）
```

三台それぞれの実行領域（既定）:

```text
$HOME/procon37-production/D0/
  bin/distributed-solver
  bin/server-bridge
  distributed.py
  node-config.json           三台共通の配布設定
  manifest.json              配布ファイル4個のSHA-256
  package.tar.gz
  private/cluster.token      三台共通の内部認証（自動生成）
  private/competition.token  run.shの公式チームトークン（VOID1だけ）
  runtime.json               run.shのURL・選択時間・試合データ保存先（VOID1だけ）
  runner.json                起動PID・時刻・boot ID・ユーザー・専用セッション
  .control.lock
```

公式チームトークン・URL・選択時間・試合データ保存先はrun.shから起動時にVOID1へ設定する。deploy時は不要で、変更時は停止・再起動だけでよい。自動生成の内部認証はdeploy時に三台へ保存する。VOID2/3の共通SSHパスワードは、VOID1リポジトリ内のGit管理外 `.server/distributed-production/ssh-password` から読む。今回の配置時に設定済みで、配布パッケージにもGit履歴にも含めない。三台の実行ログはrun.shを起動した端末へ出力する。実行ログをファイルにも保存したい場合は明示的に標準出力・標準エラーをリダイレクトする。問題・マップ・観測状態・提出のHTTP通信は、VOID1リポジトリの `.server/submissions/` に試合ごとのJSON/JSONLとして自動保存する。`RECORD_DIR` で変更でき、保存形式は[ソルバPCのセットアップ](../../../tester/server/docs/solver-pc.md#試合データの保存)を参照する。ログの伏字処理は行わない。通信エラーのURLなどにチームトークンが含まれる場合がある。

## 起動・停止の動き

`deploy` は全配置先が未使用か確認し、三台へ同じファイルを配る。転送したアーカイブのSHAと配置後の各ファイルSHAを検証する。起動は行わない。失敗した途中領域も自動削除せず残すので、原因を確認し、未使用の配置先名でやり直す。

`start` は三台の配置・CPU・ライブラリと保存PIDを確認する。VOID2/3を起動してwireのAuthenticate/Readyのみで認証・thread数・offsetを検査し、その後VOID1を起動する。この確認は探索を実行しない。起動後は端末にログを流しながら待機する。既に三台が稼働していればworkerへ追加接続せず終了する。途中失敗では今回起動した処理だけを停止する。

VOID1のbridgeは1試合につき1プロセス。試合終了後は次の試合を待ち、HTTP 401による終了コード2なら待機を終了する。その他のbridge異常終了は1秒後に次のbridgeを起動する。Ctrl+C・端末終了・SSH切断やworkerの異常終了では、今回起動した処理を停止する。停止できなかったPCは端末に表示する。途中再起動で以前の提出履歴を復元できる保証はないので、通常の切替は試合間に行う。

`status` は保存PIDと実PIDの起動時刻・session・boot・ユーザー・作業ディレクトリを照合する。PIDが生存しているだけでは提出成功とは扱わない。`stop` はVOID1から停止し、同じ専用sessionの子や残った処理も照合して停止する。別の起動時刻のPIDや別配置の処理へ信号を送らない。

## 検証範囲

配置スクリプトのローカル試験は、コンパイル済みファイルの識別、未コンパイル・同梱バイナリ指定の拒否、ビルド失敗時の設定維持、設定・転送前検査、改変拒否、worker認証フレーム、二重起動、起動失敗時の停止、ダミープロセスの実行中ログ表示と端末終了時の停止、PID再利用、子・親終了後の残処理・応答しないダミープロセスの停止を対象とする。三台を介した配置・本番HTTP・実ソルバの稼働はこの検証に含めていない。

```sh
python3 -m unittest discover -s tester/server-bridge/tests -p 'test_distributed.py' -v
```

2026-10-08にはVOID1で `compile.sh` によるオフラインreleaseビルド、生成したbridgeの模擬HTTPログ検証5ケース、運用テスト28件、三台への `deploy.sh` による配布・SHA照合、旧配置の停止と新配置の未起動状態を確認した。

2026-10-09には保存機能を追加し、Rustテスト18件、運用テスト29件、release版の模擬HTTP検証6ケース、三台への新配置・SHA照合を確認した。問題全体の追加フィールド、マップ、道路状態、提出内容・応答の保存に加え、提出前にデータを読み出せること、同一時刻の試合を上書きしないこと、保存先が使えない場合も提出を続けることを検証した。

D0自体の正式評価と、今回再コンパイルしたファイルの検証は区別する。新配置による三台対戦は未実施で、開発と自動監視は一時停止のまま。本番前の練習接続確認は `solver-pc.md` の手順で行う。
