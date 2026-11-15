# 保存ソースの単一ファイル化と照合

[概要](README.md) / [ソース対応表](source-manifest.json) / [再現手順](reproduction.md)

## 第6〜8世代の保存形式の統一（2026年10月07日）

第1〜5世代と同じ形式に揃えるため、`solver06.rs`・`solver07.rs`・`solver08.rs` を追加しました。元の分割ソースは変更せず保持し、入口の `#[path = "solver/planner.rs"] mod planner;` だけを、同じ名前の `mod planner { ... }` へ展開してRustfmtで整形しています。探索ロジック・引数・通信・診断識別子の編集は行っていません。

各世代の `README.md`・`algorithm.md`・`settings.md`・`validation.md` も揃えました。第6・7世代のアルゴリズムと設定は昇格コミットの設計資料を基に、保存先と評価状況を更新したものです。第8世代の生成方法の注意とbefore / afterは保持しています。Cargoの全8ビルド対象はそれぞれの単一ファイル版を入口にします。

元の分割ソース・以前の単一ファイル版・評価原記録のハッシュを確認し、元ソースを同じ手順で展開・整形した結果と追加ファイルのバイト一致を照合しました。Rustfmtによる構文解析とCargo設定・資料参照の静的確認を行い、追加ビルド・追加テストは実施していません。第1〜5世代の下記のビルド・テスト結果は以前の保存作業の記録です。第6〜8世代の追加ファイルのビルド・対戦結果ではありません。[形式統一の照合記録](records/format-unification-006-008-20261007.json) / [ソース対応表](source-manifest.json)。

## 第6・7世代の追加保存（2026年10月07日）

評価・昇格の原記録だけが保存されていた第6・7世代について、実装保存版を追加しました。昇格コミットの対象ソルバの `main.rs` と `solver/planner.rs` を `solver06/`、`solver07/` にバイト同一で複製しています。第6世代の出典は `a9216446bd0ae6837689a0847bb3f467c2d1d428`、第7世代の出典は `da1c7888e251b84689059b61569ec8040809b177`（正式昇格版v37）です。

正式評価の識別情報と一致し、元のモジュール構成を維持しています。第7世代はJSONの `formal_candidate_files_sha256` を照合根拠とし、別候補の値が残る `candidate_source_sha256` は採用していません。保存用Cargo設定に `pool-solver-06` と `pool-solver-07` を追加し、入口・モジュール参照・依存先を静的に確認しました。元の評価記録と既存保存ソースは保持し、追加ビルド・追加テストは行っていません。[照合記録](records/source-registration-006-007-20261007.json) / [ソース対応表](source-manifest.json)。

## 第8世代の追加保存（2026年10月07日）

第7世代を継続改善したv117を第8世代として登録し、評価済みの `main.rs` と `solver/planner.rs` を `solver08/` 配下へバイト同一で複製しました。初回登録では単一ファイル化やソルバの編集は行わず、元のモジュール構成を維持しました。後の形式統一で追加した単一ファイル版も、元の分割ソースと併せて保持しています。SHA-256と評価時の実行IDを [source-manifest.json](source-manifest.json) に追記しました。

保存用のCargo設定へ `pool-solver-08` の入口を追加し、モジュール・依存先・設定の参照を静的に確認しました。開発終了指示に従い追加ビルド・追加テストは行わず、保存元の既定フル30件とソース照合を検証根拠としています。以下の第1〜5世代の保存手順・ソースはそのまま保持しています。

第1〜4世代はコミット `11d1e19` の対戦サンプルから、入口と依存するRustモジュールを無改変で複製した。第5世代も対象ソルバの3ファイルを無改変で保存した。各版の `solver0x.rs` は外部モジュール指定を同名のインラインモジュールへ展開し、Rustfmtで整形した単一ファイルである。第1世代のテスト用 `include_str!` は、同じ公式サンプルJSONを読むよう相対パスのみ更新した。

| 記録 | 元のRustファイル数 | 単一ファイル |
| --- | ---: | --- |
| 第1世代 | 2 | [solver01.rs](solver01.rs) |
| 第2世代 | 2 | [solver02.rs](solver02.rs) |
| 第3世代 | 2 | [solver03.rs](solver03.rs) |
| 第4世代 | 11 | [solver04.rs](solver04.rs) |
| 第5世代 | 3 | [solver05.rs](solver05.rs) |
| 第6世代 | 2 | [solver06.rs](solver06.rs) |
| 第7世代 | 2 | [solver07.rs](solver07.rs) |
| 第8世代 | 2 | [solver08.rs](solver08.rs) |

[source-manifest.json](source-manifest.json) に元ファイル、保存した分割ファイル、単一ファイル版のSHA-256と変換内容を記録した。元の分割ファイルは記録された出典のSHA-256と一致する。

```sh
cargo check --offline --manifest-path solver/docs/solver-pool-experiment/Cargo.toml --target-dir target --bins
cargo test --offline --manifest-path solver/docs/solver-pool-experiment/Cargo.toml --target-dir target --bins
```

以前の保存作業で第1〜5世代の `cargo check` は成功。既定の単体テストは第1世代6件、第2世代3件、第3世代3件、第4世代18件、合計30件成功した。第4世代の2件は原実装で `#[ignore]` が指定されており、今回も実行していない。第5世代に単体テストはない。

この検査は保存構成でのコンパイルと既存テストの確認である。本番条件での新しい対戦性能は測定していない。正式評価の成績は各世代の [検証記録](algorithms.md) と `records/` にある。
