# 分散ソルバ候補 C008：CPU 経路表の worker 間共有

2026年10月9日に C007 から作成した独立 Rust workspace です。**未採用・対戦未評価**です。CPU 共有の効果を先に切り分ける候補で、GPU は使用しません。D0、C006、C007、固定比較相手、評価ハーネスは変更していません。

## 実装

C007 の compact 経路表を不変の `Arc<GraphTables>` に分離しました。同じ pool の worker が同じ入力を要求すると、1 worker が作った表を共有します。訪問列は thread safe な `OnceLock<Box<[usize]>>` へ初回だけ復元します。route memo、supplier cache、非 target 向け燃料距離の遅延 cache は worker 私有のままです。既存の `selection_graph` も worker 私有です。

key は `PreparedGraph` 全要素の一致で確認します。map、最終 duration/cost、店舗対応、順序・重複を含む positions、cell node、target 数、店舗位置を保持し、hash が衝突しても異なる表を共有しません。day、fuel、乱数状態など経路表を変えない探索入力は key に含めません。

構築中に全体の map mutex は保持しません。entry ごとの Building/Ready/Failed と通知を使い、owner の panic 時には RAII guard が Failed を通知します。異なる入力は並行に構築できます。上限 4 entry、目標 128 MiB の LRU は保持 cache の soft limit です。使用中の Arc と私有 fallback は直ちに破棄できず、実メモリの厳密な上限ではありません。

共有待機は呼出側の局所期限に従い、`min(2 秒, 残量の 1/4, 残量 − CPU 再構築見積り − 20 ms)` に制限します。再構築見積りは pool 内の構築実測の最大 ns/(cell×target×mode) に 50% の余裕を加えます。初回は 1 µs/(cell×target×mode) を使います。owner が失敗した場合は再試行し、待機限界なら私有 CPU 構築へ進みます。呼出し時点で残り時間が短い場合は追加待機をせず、従来どおり構築します。構築自体は中断できず、この見積りで締切を厳密に保証するものではありません。

探索の 91% と 110 ms 控除、当日 67%・車種選択 62% の rollout 配分、評価関数、乱数操作、通信・提出設定は保持しています。変更した既存ソースは `solver/planner.rs`、`pool.rs`、テストの `solver/compact_tests.rs` です。共有処理と追加テストは `solver/graph_cache.rs`、`solver/graph_cache_tests.rs` にあります。

## 検証と識別

21 件の unit test と localhost network 統合 2 件が成功しました。継承した microbench 1 件は ignore しています。CPU 1・-j 1 で build/test を実行し、全 5 子プロセスの停止と PID 消失を確認しました。read-only review の重大指摘はありません。

[検証記録](validation.json)、[全ソースの SHA-256](source-manifest.json)、[共有と性能測定の設計](docs/shared-graph.md) を参照してください。C007 の D0 eager 参照器による 1,608,270 経路の完全同値テストを保持しています。並行共有・hash 衝突・owner panic・timeout・局所期限・退去・可変 cache 分離を追加検査します。

本候補 source aggregate は `ff0e146e8ebf18f9934708eb85a274a3ede3f237e22c8430922dccf544da0a60`、production binary SHA-256 は `651d075b36969555b360f6b21f77fcac6a2612dae5c6f0e1d7ef4758a7545491`。

親 C007 source aggregate は `8db728384c9a6cc3727c8def4a0e4f6720a5dd54b370ab33675e4789415e731f`。採用中の D0 初期実装 commit は `5d6c740962a1f3e20e77e13df243078df1d29ad2`。コピー元は `solver/distributed-candidates/c007-compact` で、生成物 `target/` はコピーしていません。

ローカル correctness・localhost 統合検証を終えた段階です。共有版の性能向上、server 短縮12件・正式12件での成績は未確認です。C007 の microbench 成績を C008 の成績として扱いません。

## ビルドと再現

実際のリポジトリルートで実行します。固定相手の CPU 0/2/4/6 を避け、現在の作業は CPU 1 のみです。

```sh
taskset -c 1 cargo build --release --offline --manifest-path solver/distributed-candidates/c008-shared/Cargo.toml -j 1
taskset -c 1 cargo test --release --offline --manifest-path solver/distributed-candidates/c008-shared/Cargo.toml --bin distributed-solver -j 1 -- --test-threads=1 --skip wire::tests::fragmented_frames_and_oversized_frames --nocapture
```

上記は TCP を使う unit test を除外します。TCP unit と network 統合 2 件は、実行担当の許可後に localhost 接続を許可する権限で別途成功しました。再現コマンドは validation.json と docs/network-runner.json にあります。リモート配置・実機負荷・性能計測は実行担当との調整後に行います。通常 unit test は C007 から引き継いだ性能 microbench を ignore します。
