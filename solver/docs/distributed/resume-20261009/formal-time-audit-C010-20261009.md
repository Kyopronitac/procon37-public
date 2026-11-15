# C010 の正式時間設定の追補監査

C010-on-short-02 の開始時に4台の実 launcher を読み取り、保存 manifest の SHA と全て一致した。3台は `--cross-elite on`、future weight 0／reserve weight 15、既定の32／16／32スレッドであり、時間・探索予算を10秒に制限する引数や環境変数はない。VOID側は `env -i`、NOMは最小環境で起動する固定 team runner から実行される。固定単一2体も環境を消去して CPU 0／2 で起動する。

[`manage.launch`](../../../distributed-lab/manage.py) を再確認した。新しい stage で `--short-ms` を省略すると `timeLimitMs: null` の run-config を新規作成し、server に `--time-limit-ms` を渡さない。親環境の `PROCON37_*`／`SELECT_MS`／`TIME_LIMIT_MS` は除かれ、team runner も PATH と LANG 以外を引き継がない。今回の短縮評価の10000msは server側の当該runだけに明示されており、次の正式runへ残らない。

正式時は新規stage・必須label・`taskset -c 1,3,5,7` を維持し、`--short-ms`／`--output-dir` を指定しない。実際のcommand、run-config、問題のdaySecondsと公式車種選択時間、12件・全48AC・前後SHA一致は正式runで別途確認する。C010の500ms共有間隔、32KiB制限や通常の締切余裕は探索方式の一部であり、短縮評価用の時間capではない。

これは設定の読取監査であり、正式評価・採用の記録ではない。[元監査](formal-time-audit-20261009.md) と [今回の実launcher・SHA付き記録](formal-time-audit-C010-20261009.json) を参照。
