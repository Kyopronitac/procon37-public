# 実行設定

再構築4（`6fd85e7`）の資料。記録ソース: [solver05.rs](../solver05.rs)。
[比較対象のアルゴリズム](../algorithms.md#solver05)。

本文中の旧ファイル名は、単一ファイル内の同名モジュールを示す。当時のアルゴリズム・設定・検証記録を保存し、参照先と実行コマンドを現在の配置へ更新した。
検証の数値・実行ファイルのハッシュ・「現在」などの記述は原コミット時点の記録であり、今回の単一ファイル化後に再計測した性能値ではない。

実行例を使う前に `cargo build --release --manifest-path solver/docs/rebuild-experiment-imak/Cargo.toml --target-dir target --bin solver-05` で対象版をビルドする。

ソルバの第1引数は探索乱数のseed。既定値は0。

探索期限は各要求の `timeLimitMs` に基づく。
全体の1%（最低15 ms、最大700 ms）を最終検証とJSON出力のために確保し、残りを探索に使う。
車種選択も日別回答も固定55秒などの上限を設けない。

探索時間や反復回数を上書きする独自の環境変数は使わない。
短縮確認ではテスタの `--time-limit-ms` を指定する。
本番性能の比較では指定せず、seed 0〜59、24ケース並列、探索seed 0で実行する。
ソルバ内部は単一スレッド。

```sh
procon37_test_dir="$(mktemp -d /tmp/procon37-record05.XXXXXX)"
cargo run --release -p tester -- run 12 --jobs 12 --time-limit-ms 1500 --label '探索変更・短縮確認12件' --output-dir "$procon37_test_dir" --no-build --solver target/release/solver-05 -- 0
```
