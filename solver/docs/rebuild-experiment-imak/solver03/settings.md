# 実行設定

再構築2（`c42d3ec`）の資料。記録ソース: [solver03.rs](../solver03.rs)。
[比較対象のアルゴリズム](../algorithms.md#solver03)。

本文中の旧ファイル名は、単一ファイル内の同名モジュールを示す。当時のアルゴリズム・設定・検証記録を保存し、参照先と実行コマンドを現在の配置へ更新した。
検証の数値・実行ファイルのハッシュ・「現在」などの記述は原コミット時点の記録であり、今回の単一ファイル化後に再計測した性能値ではない。

実行例を使う前に `cargo build --release --manifest-path solver/docs/rebuild-experiment-imak/Cargo.toml --target-dir target --bin solver-03` で対象版をビルドする。

```sh
target/release/solver-03 0
```

第1引数は乱数seed。既定は0。
探索は単一スレッドで実行する。複数ケースはテスタの `--jobs` で並列実行できる。

通常は各要求の `timeLimitMs` から、制限時間の2.5%（最低15ミリ秒、最大1000ミリ秒）を差し引いて探索する。
本番では日別計画に約59秒、車種選択には1回戦59秒・準決勝89秒・決勝119秒を使う。
経路の前計算と候補の最終検証も同じ回答期限内で実行する。

`SOLVER_BUDGET_MS` を設定すると、車種選択・日別計画の探索時間を短縮できる。
通知された制限時間を超える値は使用しない。
フルテストではこの環境変数を設定せず、テスタの `--time-limit-ms` も指定しない。

`SOLVER_HORIZON=0` は比較実験用に先読みを無効化する。既定は最大2日先までの先読みを有効にする。

```sh
# 短縮テストの例。通常の比較履歴と分ける。
procon37_test_dir="$(mktemp -d /tmp/procon37-test.XXXXXX)"
cargo run --release -p tester -- run 3 --time-limit-ms 1500 --label '新規実装・通信確認・短縮3件' --output-dir "$procon37_test_dir" --no-build --solver target/release/solver-03 -- 0
```

同じ乱数seedでも、探索回数はCPU負荷により変動する。
stderrには車種、各日の種類数・玉数、探索回数、先読み評価値または推定残燃料を出力する。
