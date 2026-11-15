# C008 CPU 共有候補の local 検証

2026-10-09。C007 から独立作成した `solver/distributed-candidates/c008-shared` は、local correctness 検証を終えた**未採用候補**です。D0/C006/C007、評価ハーネス、固定相手は変更していません。

| 識別 | 値 |
| --- | --- |
| 親 C007 source aggregate | `8db728384c9a6cc3727c8def4a0e4f6720a5dd54b370ab33675e4789415e731f` |
| C008 source aggregate (36 files) | `ff0e146e8ebf18f9934708eb85a274a3ede3f237e22c8430922dccf544da0a60` |
| C008 production binary SHA-256 | `651d075b36969555b360f6b21f77fcac6a2612dae5c6f0e1d7ef4758a7545491` |
| local build/test | CPU 1 のみ、release、offline、cargo -j 1、test-threads=1 |
| unit | 21 成功、0 失敗、1 microbench ignore |
| network | localhost 統合 2 件成功 |
| 性能 / server | 未実行 |
| remote 配置・プロセス | 起動なし |

同一 pool 内で不変 `Arc<GraphTables>` を共有し、訪問列を `OnceLock` で共有遅延生成します。可変 memo・supplier cache・非 target 燃料 cache は worker 私有です。D0/C007 と同じ経路・費用・訪問列を作り、共有 key は準備済み入力の全要素を照合します。

owner panic 時の RAII 通知、待機 timeout、局所期限と再構築費用を考慮した待機短縮、LRU と active Arc の寿命、31 worker による単一構築を確認しました。元の探索時間配分、通信・提出経路は保持しています。C007 の 1,608,270 経路の D0 eager 完全同値検証も再実行して成功しました。

ビルド、unit、network の全プロセスは終了済みです。最初に TCP 1 件を除外して 20 unit を実行し、read-only review の重大指摘なしを受けて、親の許可のもと TCP 1 件と network 統合 2 件を CPU 1・localhost のみに限定して実行しました。5 子プロセス全ての stopped 記録と PID 消失を保存しています。性能 microbench は設計のみで、C008 の改善成績はまだありません。

[候補 README](../../../distributed-candidates/c008-shared/README.md)、[検証 JSON](../../../distributed-candidates/c008-shared/validation.json)、[共有・計測設計](../../../distributed-candidates/c008-shared/docs/shared-graph.md) を参照してください。採用と commit/push は正式評価後の担当判断です。
