# 再構築実験の記録ソースと再実行方法

[実験結果まとめ](README.md) / [実装とアルゴリズム](algorithms.md)

各実装の記録は、通信処理・探索処理・単体テストを含む**1つの `.rs` ファイル**にまとめる。
開発用の `main.rs` と `solver/` 内の分割ソースはそのまま維持する。
この資料と `solver/docs/rebuild-experiment-imak/` はコミット時点の記録であり、開発用ソースの代わりに編集・同期する対象ではない。
元の `mod foo;` を同じ名前の `mod foo { ... }` へ展開し、外部のソルバ用Rustファイルを読み込まない構成にした。
元の探索式、近傍操作、乱数、時間配分、テストを維持し、Rustfmtで整形している。
共通依存の `tester`（問題型・シミュレータ・テスト用生成器）と `serde_json` は引き続きCargoで参照する。
「単一ファイル」はソルバ自身の実装を指し、標準ライブラリだけで `rustc` 単独ビルドできる形式ではない。
再構築3の単体テストはリポジトリ内の公式サンプルJSONも利用する。

## 記録用ソースのビルド・実行

コマンドはリポジトリのルートで実行する。Rust / Cargo 1.94以降を使用する。

```sh
# 記録用の全5版をビルドする
cargo build --release --manifest-path solver/docs/rebuild-experiment-imak/Cargo.toml --target-dir target --bins

# 再構築1だけをビルドする
cargo build --release --manifest-path solver/docs/rebuild-experiment-imak/Cargo.toml --target-dir target --bin solver-02

# 通常の開発用ソルバは、従来どおり分割ソースからビルドする
cargo build --release -p solver

# 各版へJSON Linesを標準入力で渡す。引数0は探索用乱数seed
cargo run --release --manifest-path solver/docs/rebuild-experiment-imak/Cargo.toml --target-dir target --bin solver-02 -- 0
cargo run --release -p solver -- 0
```

記録用の [Cargo.toml](Cargo.toml) は独立したワークスペースである。
ルートのCargo設定・通常のビルド・テスタの既定ソルバ `target/release/solver` は変更していない。
上記の `--target-dir target` はビルド成果物だけをルートの `target/` に集める指定である。
再構築4の記録も `solver-05` としてビルドするため、開発用の `solver` を上書きしない。
同じ問題を別の実装へ渡すには、ビルド後にテスタの `--solver` を指定する。

```sh
# 短縮した通信・合法性の確認。フルテストとは別の保存先を使う
procon37_test_dir="$(mktemp -d /tmp/procon37-solvers.XXXXXX)"
cargo build --release --manifest-path solver/docs/rebuild-experiment-imak/Cargo.toml --target-dir target --bins
cargo build --release -p tester
target/release/tester run 3 --jobs 3 --time-limit-ms 2000 \
  --no-build --solver target/release/solver-03 \
  --label '再構築2・単一ファイル構成・短縮3件' --output-dir "$procon37_test_dir" -- 0

# 本番条件の比較用。対象の版名とラベルを変更して実行する
target/release/tester run 60 --jobs 24 --no-build \
  --solver target/release/solver-03 --label '再構築2・本番条件フル60件' -- 0

# 既存の単体テストを全版または1版について実行する
cargo test --release --manifest-path solver/docs/rebuild-experiment-imak/Cargo.toml --target-dir target --bins
cargo test --release --manifest-path solver/docs/rebuild-experiment-imak/Cargo.toml --target-dir target --bin solver-03
cargo fmt --manifest-path solver/docs/rebuild-experiment-imak/Cargo.toml -- --check
```

全版とも内部は単一スレッド。問題生成seedと探索seedは別である。
本番比較では探索時間や先読みを変える環境変数を外し、同一問題・同一条件で比較する。
基準実装の既定探索予算は55秒、再構築1以降は要求の制限時間を主に使うなど、実装間で時間配分も異なる。
実時間で停止するため、同じseedでもCPU負荷などにより結果は変わる。

通常の開発は既存の分割ソースで行う。記録用 `.rs` は原コミット時点の内容を保つ。
記録を読む・ビルドするためにGitの過去コミットへ切り替えたり、ソースを再生成したりする必要はない。
検証結果は[単一ファイル化の検証](consolidation.md)を参照する。
