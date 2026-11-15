# 実対戦ログの Graph 取得時間と探索回数

`collect_graph_metrics.py` は、正式に保存した completion の byte 範囲だけを SSH で読み出し、同じ記録の SHA-256 に一致することを確認してから集計します。raw log は `.server/distributed/graph-metrics/` に置き、このディレクトリには quantiles・件数・範囲と意味を保存します。リモートは読み取りだけで、local/remote の collector を CPU 1 に限定しています。

## D0 短縮基準 1 回目

対象 run は `1791503276339409187-91328`、12 件 seed 0〜11、4 チーム、10 秒、全48 AC です。VOID2 は探索15 thread＋主thread、VOID3 は探索31 thread＋主thread。いずれも84要求すべてで期待した探索worker全員が完了しています。

| ホスト | 捕捉範囲 (byte、終端を含まない) | SHA-256 | planner 行数 |
| --- | --- | --- | ---: |
| VOID2 | [0, 1272113) | `ade1fbe2324dc95e9588b3082063b7916f99dd9b10e45d7973cd559ddde1f335` | 2700 |
| VOID3 | [0, 2611949) | `07e5279129632a8dec5fe05e7f78f21bd2fdd31d4a7e56c9295ca35f98336d6e` | 5580 |

回収時には同じファイルへ C007 が追記していましたが、上記の D0 範囲だけを読みました。inode も保存時と一致しています。集計不能行と要求へ帰属できない planner 行は両機とも 0 件です。

| ホスト・形式 | 標本数 | Graph ms 中央値 | p90 | iterations 中央値 |
| --- | ---: | ---: | ---: | ---: |
| VOID2・1回戦 | 1260 | 95 | 103 | 1392079 |
| VOID2・準決勝 | 750 | 186 | 218 | 2134563.5 |
| VOID2・決勝 | 690 | 450.5 | 636.1 | 1277068.5 |
| VOID3・1回戦 | 2604 | 126 | 139 | 1492804.5 |
| VOID3・準決勝 | 1550 | 261 | 306 | 2289439 |
| VOID3・決勝 | 1426 | 638 | 888 | 1328318.5 |

1 標本は各 worker の `plan_inner` 完了時の `g007` 行です。車種選択、実際の日、rollout の予測日を含みます。形式別の日数や1回戦の複数予測scenarioにより件数は異なり、試合ごとの等重み平均ではありません。詳細 JSON では format、phase/stage、seed に分けています。

VOID3 の `select.primary` は全形式 p90 が1519 ms、決勝だけでは中央値840 ms・p90 1917.1 msです。決勝の `plan.primary` は中央値638 ms・p90 882 ms。実対戦の最初の Graph 取得に相応の時間があるため、同一入力の重複構築を減らす C008 と、その後の GPU producer を調べる根拠になります。改善する得点幅はまだ分かりません。

要求ごとの `sum(graph_ms)/(開始worker数×要求elapsed_ms)` の中央値は、VOID2 の1回戦/準決勝/決勝で3.2%/4.3%/10.4%、VOID3 で4.2%/6.1%/14.9%です。これは scheduling を含む取得 wall time の記述比率で、CPU利用率や確実に回収できる wall time の割合ではありません。

## C007 短縮 1 回目との比較

C007 の確定範囲は VOID2 `[1272113, 2544119)`、SHA `2e6c7f5ebdc4a578ca207967c2f4bade3a188466ddddb9126d27b43e9b69b2fc`、VOID3 `[2611949, 5222894)`、SHA `fc3a6a78c49f61e372b648a945e4ed0085511f4ad4da5c2914ca61defe51e17d`。いずれも byte 範囲の SHA と inode を確認済みです。各84要求全員完了、D0と同じ標本数、未帰属/parse欠落0件でした。

| ホスト・形式 | Graph ms 中央値 D0→C007 | p90 D0→C007 | iterations 中央値の変化 |
| --- | ---: | ---: | ---: |
| VOID2・1回戦 | 95→62 | 103→68 | +0.04% |
| VOID2・準決勝 | 186→126 | 218→142 | +0.51% |
| VOID2・決勝 | 450.5→354 | 636.1→488 | +4.50% |
| VOID3・1回戦 | 126→70 | 139→78 | +2.07% |
| VOID3・準決勝 | 261→125 | 306→142 | +2.22% |
| VOID3・決勝 | 638→292 | 888→414 | +14.71% |

VOID3 の select.primary 全形式 p90 は1519→351 msとなり、C007 の compact 化で初期取得時間の長いケースも減りました。ただし全形式をまとめた plan.primary の iterations 中央値は VOID2 が3531425→3499153.5、VOID3が4034763→4008321でほぼ横ばいです。局面・乱数経路・1 iteration の仕事量が違うため、形式別の iteration 増加をそのまま等仕事量の速度向上と解釈しません。

C007 の要求ごとの graph 取得 wall time 比率の中央値は、VOID2 の1回戦/準決勝/決勝で2.2%/3.0%/8.3%、VOID3で2.4%/3.0%/6.9%。共有/GPUで取り除ける割合と断定できませんが、次に同一入力を共有する C008 の hit/build/wait を確認し、その後 GPU の利益が残るかを測る判断材料になります。成績の正式な採否は別の対戦評価記録に従います。

比較の全 quantiles と意味は [VOID2](C007-short-01-v2.json)、[VOID3](C007-short-01-v3.json) に保存しています。基準は [D0 VOID2](resume-D0-short-01-v2.json)、[D0 VOID3](resume-D0-short-01-v3.json) です。

## C008 CPU共有の結果

[共有構築削減・private fallback・メモリの比較](C008-summary.md) を追加しました。primary の Graph 取得は短縮しましたが、forecast と process HWM は改善せず、対戦成績にも明確な改善はありません。共有counterは要求83まで、Graph quantilesは全84要求です。

## 指標の限界

`graph_ms` は `plan_inner` が主 Graph を取得する範囲だけを測っています。初期解構築、探索、後で作る予測 Graph、提出処理は含みません。`Rc` の既存表を再利用すれば0付近になります。共有版では待機も含むため、C008 の CPU 構築時間そのものは `shared_graph` counters と併せて確認します。

`iterations` は探索 loop 回数で、worker の重み、当日/予測日の予算、経路長や局面に依存します。1 iteration の仕事量を固定した throughput 測定ではありません。内部 score は公式得点と混同しないため集計していません。

log の g007 行には worker ID と graph key がありません。したがって per-worker の厳密な対応比較や共有 hit 率、GPU の担当時間は復元できません。C007 と D0 は採用した移動計画によって後続局面が変わるため、同じ形式でも同一 graph 入力の対比ではありません。

## 再現

リポジトリルートで、使用した版の `completion`、`manifest`、`run` を指定します。SSH password は `PROCON37_SSH_PASSWORD` から読み、出力や保存 JSON へ含めません。既存 raw の再解析は `--reuse-raw` を付ければ SSH に接続しません。

```sh
taskset -c 1 python3 solver/docs/distributed/resume-20261009/collect_graph_metrics.py \
  --completion solver/docs/distributed/resume-20261009/evaluations/resume-D0-short-01/v2-completion.json \
  --manifest solver/docs/distributed/resume-20261009/evaluations/resume-D0-short-01/manifest.json \
  --run solver/docs/distributed/resume-20261009/evaluations/resume-D0-short-01/run.json \
  --node v2 --planner solver/distributed/solver/planner.rs \
  --raw .server/distributed/graph-metrics/resume-D0-short-01/v2.log \
  --output solver/docs/distributed/resume-20261009/graph-metrics/resume-D0-short-01-v2.json \
  --reuse-raw
```

VOID3 は node とファイル名を v3 に変更します。後続 C007 はその評価の正確な区間記録を使い、`--baseline` に同じホストの D0 JSON を指定すると中央値・p90・平均の記述的な比率も保存します。既存評価記録や frozen source は変更しません。
