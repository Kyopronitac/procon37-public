# C008 CPU 共有の設計と計測手順

2026-10-09。未採用候補の設計記録です。local correctness と localhost network 検証は完了し、以下の性能確認は実行前の計画です。

## 寿命と同一性

`SearchPool` と `PoolSolver` がそれぞれ 1 個の `Arc<GraphCache>` を所有し、作成した `Solver` に注入します。分散では同一機内だけで共有し、PC 間通信を増やしません。select による Solver 再生成と snapshot restore 後も pool の共有 handle を使用します。pool 破棄後は最後の active Arc がなくなるまで表を保持し、その後解放します。新しい service thread は作りません。

`PreparedGraph::new` は C007 の Graph 前処理をそのまま移し、`GraphTables::build` が C007 の Dijkstra、mode 重複判定、potential、mobility、target 向け燃料表を作ります。`Graph::from_tables` が私有可変 cache を初期化します。非 target 向け燃料表は従来同様に必要時に作り、共有表へ書き戻しません。

hash は検索の絞り込みだけに使い、key 全要素の比較を省略しません。店舗・車の同一セル位置による positions の重複と順序は保存します。extra の順序も保存し、従来と同じ前処理による重複排除だけを行います。交通は観測の順で duration に適用してから比較するので、同一セルの重複指定でも最後の指定が反映されます。

## 同期と失敗回収

全体 mutex の内側は最大 4 entry の比較・登録・退去に限定します。CPU 構築、entry 待機、大きい表の最後の drop は mutex の外です。1 つの key について owner が構築し、他の worker は entry の condition variable で待ちます。

owner の guard が Ready を公開するか、unwind 時に Failed を公開して全 waiter を起こします。Failed の除去は entry の Arc 同一性で確認し、新しい owner の entry を誤って消しません。既存 SearchPool の catch_unwind は保持しています。構築以外の探索 panic により cache を Building のまま残すことはありません。

待機期限は get 呼出しの開始時に固定します。失敗・再試行しても同じ期限を使うため、owner が繰り返し脱落した場合に 2 秒待ちを累積しません。局所期限の残量、構築実測、余裕から待機上限を決め、期限到達後は元の owner を置き換えず私有 CPU 構築へ進みます。4 entry がすべて Building の場合も私有構築に切り替えます。

構築見積りは `cell 数 × target 数 × 6 mode` に対する実測 ns の pool 内最大値です。燃料 Dijkstra や mode mask の費用も測定時間に含みます。CPU 負荷と形状に依存するため厳密な予測ではありません。初回は 1 µs/work、以降は実測上限、再構築用に 50% と 20 ms の余裕を設けます。さらに待機自体を残量の 1/4 以下に抑えます。構築が元の局所予算を超える場合まで解決したものではありません。

cache の 128 MiB は GraphTables 本体・主要 heap 配列・遅延訪問列の概算です。entry/key、allocator 管理領域、探索 memo 等を含む RSS とは異なります。128 MiB を超えた表でも、使用中の Arc を無効化せず cache 参照だけを退去します。訪問列の増加は次回 get の trim で反映します。

## ログと意味

stderr の `shared_graph context=... {JSON}` は累積値です。分散 SearchPool は次の要求開始前と Drop 時、並列 PoolSolver は collect 完了時に出力します。分散の要求境界では前要求の遅い worker がまだ処理中の場合があるので、request ごとの正確な集計とは扱いません。最終集計は全 worker 終了後の値を優先します。

| 指標 | 意味 |
| --- | --- |
| requests | GraphCache 呼出し数 |
| hits | 既存 entry と一致した acquire 数。Building・Failed を含み、失敗後の再試行で requests と一致しない場合があります |
| builds / completedBuilds / failures | 実際に CPU 構築を開始・完了・unwind した数。私有 fallback も含みます |
| waits / timeouts / bypasses | Building に遭遇した数、待機限界到達数、私有構築を選んだ数 |
| buildMs / waitMs | 全 worker の構築/待機時間の合計。wall time ではありません |
| retainedEntries / retainedTableBytesEstimate | cache 自身が保持する表の数と概算 bytes |
| liveTables / liveBaseBytesEstimate / liveCellBytes | active Arc と私有 fallback を含む生存表の会計 |
| peakTableBytesEstimate | 表の生存 bytes の観測ピーク。RSS の代替ではありません |
| materializedPaths / materializedBytes | 一度生成された訪問列の累積数・bytes |
| measuredMaxBuildNsPerWork / budgetLimitedRequests | 構築見積り用の実測上限、局所予算によって 2 秒より短く制限した要求数 |

## 次の評価

1. local network 統合テストは完了。CPU 1 の localhost のみで、16 thread の状態同期、切断・復帰、遠隔候補採用、前日不受理を確認した。validation.json に記録。
2. 有限 microbench は C007 と C008 の同一要求を 15/31 worker に渡し、同一入力・異なる観測・rollout の 3 条件を比較する。Graph 構築数、wall time、合計 build/wait 時間、graph_ms、探索 iteration、RSS、終了時残存 worker を記録する。全要求同一の人工例だけで性能向上を主張しない。
3. 実際の 4 チーム短縮12件では C007 または現在の採用版を同条件で反復し、平均順位・3 得点指標・全 AC・締切余裕・共有 hit と構築削減率を照合する。`1 - builds/requests` は失敗がない通常時の構築削減率として使い、raw hits だけで共有効果を判断しない。
4. 明確な短縮改善が得られた場合だけ正式12件へ進む。正式評価の実装と設定を固定し、採用時に commit・通常 push を行う。

共有の効果が少ない場合は key ごとの位置/交通の違いを調査します。target tree 単位の cache や GPU producer は別候補とし、C008 へ未評価の変更を重ねません。
