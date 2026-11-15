# C008 の CPU 共有実測：構築は減ったが成績改善は未確認

対象は C008-short-01、4チーム・12件・10秒、全48 AC。平均順位1.6667、種類数/日別種類数/玉数は222/1273/2965です。D0短縮基準1に対して明確な改善とは判断していません。以下は同じcollectorで取得したC007→C008の実対戦ログ比較で、同一Graph入力の固定仕事量測定ではありません。

## 確定範囲と集計対象

| ホスト | C008 byte範囲（終端を含まない） | SHA-256 | planner行 |
| --- | --- | --- | ---: |
| VOID2 | [2544119,3863228) | `4c75e8a4b560287bd96edeca45b9cef16f55537c766dfc445bd44c110e9e7420` | 2700 |
| VOID3 | [5222894,7880612) | `4085e2ad3037377544fc6c7310ae66e60a97c6e8c96a99f37faca5de8fb15b35` | 5580 |

両区間ともcompletionのSHAと一致し、84要求で全worker完了、parse欠落・未帰属は0件です。Graph/iterationsのquantilesは全84要求を含みます。

**共有counterは両機とも `before_request_1` → `before_request_84` の差で、完了した要求1〜83だけです。要求84は含みません。** `search_pool_drop` の終了総計はありません。最初のcounterは0、最後の共有Graph要求数2685/5549は、この範囲のplanner行数と一致しています。全runの総計へ外挿していません。

## Graph取得と探索回数

| ホスト・段階 | Graph ms 中央値 C007→C008 | p90 C007→C008 | iterations 中央値変化 |
| --- | ---: | ---: | ---: |
| VOID2・plan.primary | 129→55.5 | 375→132 | +9.43% |
| VOID2・plan.forecast | 66→75 | 337→363.1 | +0.78% |
| VOID2・select.primary | 143→59.5 | 414.1→152.4 | +10.97% |
| VOID2・select.forecast | 116→135 | 339.1→372.2 | −0.12% |
| VOID3・plan.primary | 126→47 | 318→117 | +4.17% |
| VOID3・plan.forecast | 76→83 | 280→312 | +1.54% |
| VOID3・select.primary | 143→49 | 351→131 | +8.12% |
| VOID3・select.forecast | 121→144 | 282.9→327.9 | +3.57% |

primaryのGraph取得は短くなりましたが、forecastでは中央値もp90も増えています。C008のGraph取得時間は共有待機を含みます。iterationsは探索軌跡や1回の仕事量に依存するため、上表を等仕事量の速度向上とは扱いません。

形式別のGraph中央値は VOID2の1回戦/準決勝/決勝で62→53、126→62、354→171.5 ms、VOID3で70→54、125→51、292→134 msです。

## 共有counter：要求1〜83

| 指標 | VOID2 | VOID3 |
| --- | ---: | ---: |
| GraphCache requests | 2685 | 5549 |
| CPU builds / completed | 1404 / 1404 | 2718 / 2718 |
| `1-builds/requests` | 47.71% | 51.02% |
| private bypass | 921 (34.30%) | 2226 (40.12%) |
| wait timeout / build failure | 0 / 0 | 0 / 0 |
| cache owner builds | 483 | 492 |
| 待機が発生したcall | 1276 | 2826 |
| build wall合計 | 199.041秒 | 385.554秒 |
| wait wall合計 | 89.008秒 | 171.072秒 |
| 完了buildあたり平均 | 141.767 ms | 141.852 ms |
| 待機callあたり平均 | 69.755 ms | 60.535 ms |

`1-builds/requests` は「取得ごとに1回構築する場合」に対する構築回数の削減で、C007の実測CPU時間との対応比較ではありません。build/wait時間はthread間で足したwall時間で、プロセスwall timeやCPU時間ではありません。

C008のbypass分岐は「4 entryがすべてBuilding」と「待機timeout」の2種類です。timeoutが0なので、観測したprivate bypassはすべて前者です。待機上限を延ばすだけではこの分岐を改善できません。primaryが同じ状態で始まることとforecastの取得時間の増加から、異なる予測Graphの同時構築が残ると考えられます。ただしcounterは要求単位で、bypassを内部primary/forecastへ直接帰属するログはないため、この段階別原因は推論です。

`timeouts`というJSONキー名を単純文字列検索すると、値が0でもtimeoutイベントとして数えてしまいます。ここでは構造化counter値だけを使っています。

## メモリ

| 指標 | VOID2 | VOID3 |
| --- | ---: | ---: |
| 最後のlive table数 | 1 | 1 |
| 最後のlive/retained table概算 | 78.0 MiB | 78.4 MiB |
| counterの生存表概算peak（要求83まで） | 1173.7 MiB | 2446.5 MiB |
| process VmHWM C007→C008（全12件） | 1412.2→1516.2 MiB | 2964.0→2988.3 MiB |
| VmHWM比 | +7.37% | +0.82% |

processのHWMは専用workerの全12件に対する値で、counterの83要求という範囲と区別します。共有後にHWMが減ったとは言えません。allocatorの保持や私有memo、構築中の表もRSSへ寄与します。

`peakTableBytesEstimate` は完成して登録した表と訪問列の会計で、構築途中の表、private memo、allocator管理領域などは含みません。また、request境界で観測したretained表の最大概算は約142.8/143.3 MiBで、128 MiB目標を超えています。訪問列の増加は次回getのtrimで処理するsoft limitであり、実プロセスメモリの厳密な上限ではないという設計どおりです。

## 次の判断に使えること

primaryの重複構築削減は実測できましたが、現時点の成績改善は確認できません。GPU Owner限定の案は、すでに取得が約47〜60 msへ短くなったprimaryと、private CPU構築が残る予測側を分けて評価する必要があります。CPU/GPUの構築時間短縮だけを採用根拠にしません。

C010のPC間elite共有は、これらの演算最適化と混ぜずD0を基準として協調の効果を測る方針です。

詳細は [VOID2 JSON](C008-short-01-v2.json)、[VOID3 JSON](C008-short-01-v3.json)。`metrics.sharedGraph`に始点/終点context、covered jobs、最終未包含job、counter差分、形式/要求phase別の集約、会計定義があります。rawは `.server/distributed/graph-metrics/C008-short-01/`。リモートは確定byte範囲の読み取りのみで、負荷試験・source変更はありません。
