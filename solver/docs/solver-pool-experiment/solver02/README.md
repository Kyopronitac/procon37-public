# solver02：第2世代

[一覧](../README.md) / [保存ソース](../solver02.rs) / [元の対戦サンプル](../../../../tester/game/samples/src/bin/game-sample-1.rs) / [世代記録](../records/generation-002.md)

第2世代で独立に設計したソルバ。移動合流探索を中心に、巡回順、補給地点、合流順、燃料余力を同時に最適化する。ソースは現在の対戦相手slot1にあり、コミット `11d1e19` に保存されている。専用の合流モジュールは `tester/game/samples/src/pool-generation-002/rendezvous.rs`。

正式評価run `1790551749037999892-54402` は30件すべて5体AC。順位合計は対象45、相手120・66・91・107で単独首位。対象得点合計は種類542・日別種類3587・玉数7395。

この説明は当時の開発記録と保存した実装に基づく。

単一ファイル版は [solver02.rs](../solver02.rs)。元の入口は [entry.rs](entry.rs)、依存モジュールもこのディレクトリに無改変で保存した。

詳細資料：[アルゴリズム](algorithm.md) / [実行設定](settings.md) / [検証記録](validation.md)。
