# 保存ソルバのビルドと実行

[概要](README.md) / [ソース対応表](source-manifest.json)

リポジトリのルートでRust/Cargo 1.94以降とCリンカを使用する。保存用の [Cargo.toml](Cargo.toml) は独立ワークスペースで、共通の `tester` クレートを参照する。保存ソルバはいずれも内部を単一スレッドとして扱う。

```sh
cargo build --release --offline --manifest-path solver/docs/solver-pool-experiment/Cargo.toml --target-dir target --bins
```

生成される実行ファイルは `target/release/pool-solver-01` から `pool-solver-08`。各実行ファイルは既存ソルバと同じJSON Lines通信を行い、引数 `0` が探索seedとなる。保存ソースの再ビルドは通常の `target/release/solver` や `game-sample-*` を上書きしない。

```sh
target/release/pool-solver-05 0
```

上記は標準入力から要求を受けるコマンドであり、単独では対戦を開始しない。対戦を再実行する場合は、テスタの設定で対象と相手の各実行ファイルを指定し、[poolの正式評価設定](records/generation-005-config.json)と同じ件数・seed・形式・制限時間を使用する。正式評価の保存済み成績はその時点のCPU負荷や実行時間に依存するため、再実行で同一の得点を保証するものではない。

全8世代とも `solver01.rs`〜`solver08.rs` の単一ファイル記録を直接ビルドします。元の分割ファイルは各 `solver0x/` に保存し、出典との一致は [source-manifest.json](source-manifest.json) のSHA-256で確認します。第1世代のテスト用 `include_str!` の相対パスだけは保存先に合わせて更新しています。

第6〜8世代は元の `main.rs` と `solver/planner.rs` を保持し、`planner` モジュールを同名のインラインモジュールへ展開してRustfmtで整形しています。保存元の評価記録は当時の分割ソースによるものです。今回の保存形式の統一で追加ビルド・追加テストは行っていません。[第6世代](solver06/README.md) / [第7世代](solver07/README.md) / [第8世代](solver08/README.md)。

[第8世代の対戦設定](records/generation-008-config.json)はリポジトリのルートを作業ディレクトリとして使います。実行ファイル名は `pool-solver-08` のままです。この設定の相対パスは現在のチェックアウト向けで、評価当時の正確な絶対パス・相手識別は [登録JSON](records/generation-008.json) に保持しています。

```sh
cargo test --offline --manifest-path solver/docs/solver-pool-experiment/Cargo.toml --target-dir target --bins
```
