# solver03：第3世代

[一覧](../README.md) / [保存ソース](../solver03.rs) / [元の対戦サンプル](../../../../tester/game/samples/src/bin/game-sample-4.rs) / [世代記録](../records/generation-003.md)

第3世代で独立に設計したソルバ。翌日の試走、予測計画の引き継ぎ、複数の待合地点と区間経路、保守的な交通予測を組み合わせる。ソースは現在の対戦相手slot4にあり、コミット `11d1e19` に保存されている。専用の合流モジュールは `tester/game/samples/src/pool-generation-003/rendezvous.rs`。

正式評価run `1790563781811011480-62511` は30件すべて5体AC。順位合計は対象39、相手63・100・106・130で単独首位。対象得点合計は種類542・日別種類3592・玉数7492。候補1・2は正式条件を満たさず、候補3で昇格した。

この説明は当時の開発記録と保存した実装に基づく。

単一ファイル版は [solver03.rs](../solver03.rs)。元の入口は [entry.rs](entry.rs)、依存モジュールもこのディレクトリに無改変で保存した。

詳細資料：[アルゴリズム](algorithm.md) / [実行設定](settings.md) / [検証記録](validation.md)。
