# 正式時間へ戻す経路の読取監査

2026-10-09。C006短縮2の終了時点のmanifest・実機launcher保存内容と、manage／team runner／serverの実装を確認した。正式テストはこの監査では開始していない。[SHA付き記録](formal-time-audit-20261009.json) に対象を保存した。

**`manage.py run` で `--short-ms` を省略すれば、10秒上書きは引き継がれない。確認対象のlauncherにも時間・探索予算のテスト用短縮設定はなかった。**

- `manage.launch` は新しいrun-configを作り、`timeLimitMs: null` を保存する。`--time-limit-ms` は `short_ms` が指定されたときだけserverへ渡す。古い短縮run-configを流用しない。
- serverの問題生成は `time_limit_ms` があるときだけ日別時間を上書きする。正式時は問題・fixture本来の `daySeconds` を使用し、車種選択は形式に応じた60／90／120秒となる。
- manageは親環境の `PROCON37_*`、`SELECT_MS`、`TIME_LIMIT_MS` を除く。team_runnerはPATHとLANGだけの環境でbridgeを起動し、期限を上書きする引数を渡さない。
- VOID3台のlauncherは `env -i`、固定単一2体も `env -i`。NOMのlauncher自体は環境を消さないが、直前のteam_runnerが最小環境を与え、launcherが追加するのは固定のfuture weight 0／reserve weight 15だけである。
- 終了時に4台のlauncherを実際に読み取り、全てC006短縮2のmanifestのSHAと一致した。NOM・固定単一2体の設定も変更していない。

C006の91%→97%という内部上限変更は候補本体であり、短縮テスト用の10秒capではない。110ms控除、通信回収の余裕、当日／rolloutの配分などは通常アルゴリズムの一部として維持している。正式評価でもテスト用の予算上書きは追加しない。

正式開始時は新規stageを使い、親プロセスの `taskset -c 1,3,5,7` を維持する。`--label` は必須、`--short-ms` と `--output-dir` は指定しない。実際に生成されたcommand／run-config／各problemの時間と前後の識別一致を確認し、12件seed 0〜11・形式各4件・arena有効・固定4チーム・全48ACを記録する。

この監査後に候補やlauncherを変えた場合、その正式対象のmanifestで再確認する。C006は短縮2で平均順位改善が再現せず、次候補の短縮評価を優先する判断となった。この文書はC006の正式評価や採用を意味しない。
