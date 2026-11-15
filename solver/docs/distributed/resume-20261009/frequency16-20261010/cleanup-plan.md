# 評価終了後の停止・時計復元手順

準備日時: 2026-10-10 05:11 JST。この手順の準備では対戦プロセスや時計設定を変更していない。実行担当の明示指示後に一度だけ実行する。

## 実行ゲート

`cleanup_finished.py --execute` は次を確認してから停止に進む。

- `frequency16-matrix-20261010-01/results.json` が `verified`・`all32AC: true`。
- C014・C015の各 seed 0・3・6・9、合計8件・32人がAC。
- 状態ファイルも同じstageのverifiedで、matrixの親プロセスと問題serverが終了済み。
- lifecycle用設定と評価用設定の6登録チームの配置が一致。
- 時計プロセスのPID 75930、start ticks 2767228、boot ID `61fafedf-4b69-4c18-81a9-6af42f0f3422`、UID 1000、完全な起動引数が一致。

実ホストの `/proc` とLANへアクセスできる実行環境を使う。通常の隔離環境でPIDの不在を確認しても、実ホストでの停止確認にはならない。

## 順序

1. 既存 `solver/distributed-lab/lifecycle.py stop` で全6登録チームを停止する。登録された専用プロセスの照合・停止処理を再利用する。
2. 各SSH先4台とローカルの `/proc` を調べ、登録ファイルの完全パスに一致するargvまたは実行ファイルの残留を確認する。停止できていなければ失敗として記録し、広いパターンによる一括killは行わない。
3. 4台で既存記録と恒久chrony設定のSHA-256を照合する。runtime source `192.168.0.81` だけを `chronyc delete` し、他のsource一覧を変えていないこと、恒久設定ファイルのSHA-256が同じことを再確認する。サービス再起動・設定書換え・時刻のstepは行わない。
4. 時計プロセスを再照合し、pidfdを取得してからもう一度照合する。同じプロセスだけにSIGTERMを送り、終了を確認する。
5. `cleanup.json`・`clock-cleanup.json` を新規保存する。既存 `clock-restored.json` は今回開始前の同期復元記録として保持し、上書きしない。

```sh
# 認証情報は既存の許可済み環境変数 PROCON37_SSH_PASSWORD を使う。
taskset -c 1,3,5,7 python3 \
  solver/docs/distributed/resume-20261009/frequency16-20261010/cleanup_finished.py --execute
```

成功時のみ `cleanup.json` に `productionProbePermitted: true` を記録する。その後で本番配置の `probe` へ進む。試合・ソルバ・評価をこのスクリプトから再起動することはない。

## 失敗時

既存記録があれば自動再実行を拒否する。失敗箇所と実ホストの状態を確認し、必要な限定操作を別途記録する。スクリプトは他の配置のプロセス、恒久chrony設定、採用版、評価結果を変更しない。

準備時にPython構文検査、CLIヘルプ、完全識別情報を変更した場合の拒否を確認した。VOID1の `chronyc -c -n sources` を読み取り、CSVの3列目がsourceアドレスであることを実機出力で確認した。終了処理そのものはまだ実行していない。
