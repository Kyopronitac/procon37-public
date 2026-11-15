# ソルバ

公開通信仕様と共通判定エンジンを基に開発しているRustソルバです。内部では単一スレッドで動作し、入出力はルートの `main.rs`、探索は `solver/planner.rs` に実装しています。

- [探索方法](docs/algorithm.md)
- [実行引数・設定](docs/settings.md)
- [検証結果](docs/validation.md)
- [PR #22のレビュー修正・検証](docs/pr-022-review.md)
- [世代別の評価・登録記録](docs/solver-pool-experiment/records/README.md)
- [対戦用通信仕様](../tester/game/docs/protocol.md)

`cargo build --release -p solver` でビルドし、`target/release/solver 0` で探索seed0の既定動作を実行します。この世代の開発ビルド・試験は指定されたVOIDLINUX-1・2・3のSSH環境で実施しました。2026年10月07日のユーザー指示により実装の作成・変更を終了し、開始・予約済みのテストをすべて完了して結果を回収したうえで停止しました。

第8世代の登録版は、第7世代を継続改善したv117です。従来のsolver-pool方式によるゼロからの新規開発ではありません。既定フル30件の最良記録は平均1.433333位・1位24件で、全30件1位の目標は未達です。改善開始時との比較とソース識別は[第8世代の登録記録](docs/solver-pool-experiment/records/generation-008.md)、実装の保存版は[solver08](docs/solver-pool-experiment/solver08/README.md)を参照してください。

2026年10月07日の追加指示により、PR #22のレビューで確認した不具合を修正し、SSHでの修正前後フル30件比較を完了しました。現行の `solver/planner.rs` はその修正版で、登録済みv117の保存版・評価とは区別します。世代育成と平均1.00位のgoalは再開していません。
