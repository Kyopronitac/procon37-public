# 分散ソルバ候補 C006：探索時間上限97%

2026年10月9日に採用済みD0から作成した、**未採用・対戦評価前**の独立Rust workspaceです。採用済みの `solver/distributed` と固定比較相手は変更していません。

## 変更

`solver/planner.rs` の `plan_inner` で、探索時間の割合上限を91%から97%へ変更しました。変更は次の2箇所だけです。

- 通常探索の `total_budget`：`min(limit - 110ms, limit × 91%)` → `min(limit - 110ms, limit × 97%)`。
- 観測車種が通常の補給車1台構成でない場合：`collector_plan` に渡す期限を同じ97%の規則に変更。

110msの固定控除、短い要求の待機処理、coordinatorの5%（250〜1500ms）の余裕、遠隔探索の追加150ms控除、結果回収期限を保持します。当日探索の67%、車種選択探索の62%、翌日rolloutへ実際の残時間を割り当てる規則、archiveの時間配分も保持します。当日探索時間が増えた分、既存の残時間計算によるrollout時間は変わり得ます。

`tools/`、配置用 `runtime/`、`target/` はコピーしていません。Rustモジュールの `runtime.rs` は含みます。通信方式・スレッド構成・探索手法・評価関数はD0と同じです。

## 根拠と評価予定

VOIDLINUX-3に保存されたD0の初回短縮12件ログでは、84要求すべて31/31探索が完了し、各要求の回答は概ね8.4〜8.6秒でした。この余裕の一部を探索へ回せるか確認する候補です。実行時間制御の変更だけで対戦成績が改善すると断定するものではありません。

評価は採用済みD0と同じ4チーム・12件・seed 0〜11・形式自動・練習場有効・各10秒で行い、全48判定AC、要求ごとの探索完了数、iterations、回答時間、平均順位、種類数・日別種類数・玉数、変更前後の勝同敗を比較します。短縮で再現する改善を確認した場合に正式12件へ進みます。対戦用の配置はまだ行っていません。独立した計測配置については下記を参照してください。

## 識別

親の採用版はD0、初期実装commitは `5d6c740962a1f3e20e77e13df243078df1d29ad2` です。コピー元は作業リポジトリ内の `solver/distributed`。全Rustソース・Cargoファイル・ビルドに含まれる問題fixtureの32ファイルを [source-manifest.json](source-manifest.json) に記録しました。README・manifest自身・ビルド生成物を識別集合に含めません。

| 識別対象 | SHA-256 |
| --- | --- |
| コピー元32ファイルの集約 | `61b56e0ae79cfae1ca86ddc62e8e38bb94a9410871508cadbd783a9b9a469d65` |
| C006の32ファイルの集約 | `4e8b4df8116889211bf4f47d1933759fff0c011053296460bd16375d0bf08a6b` |
| コピー元 `solver/planner.rs` | `11e043e33018dee34f41f295d6ae1500497b0bfc7c8c33d5b253a5621587461a` |
| C006 `solver/planner.rs` | `94936d87109ec750e27d848b0f008a6756afc2fb8afdc0d526110e4a038bac23` |

集約は相対パス順に並べた `<sha256>  <relative path>\n` のUTF-8列のSHA-256です。コピー元との差は `solver/planner.rs` のみです。

## ビルドと検証

以下は実際のリポジトリルートを作業ディレクトリとして実行します。絶対パスを固定する必要はありません。

```sh
cargo build --release --manifest-path solver/distributed-candidates/c006-budget/Cargo.toml -j 2
cargo test --release --manifest-path solver/distributed-candidates/c006-budget/Cargo.toml --bin distributed-solver -j 2 -- --test-threads=2
cargo test --release --manifest-path solver/distributed-candidates/c006-budget/Cargo.toml --test network -j 2 -- --nocapture
```

2026年10月9日、Rust/Cargo 1.94.0、release、ビルド並列数2で検証しました。既存単体テスト11件、localhost通信統合テスト2件がすべて成功しました。初回の単体テストはsandboxでlocalhostへのbindを禁止されていたため1件失敗し、TCPを利用できる許可環境で全11件を再実行しています。統合テストの全プロセスは停止を確認済みです。詳細は [validation.json](validation.json) に保存しています。

生成された実行ファイルのSHA-256は `23c0d0abb72e3d3aeff217c8a5aabfa8084a2655345d03ee7e291848fe7490d2`。ビルド生成物はGit管理対象外です。server短縮12件・正式12件の成績は未取得です。

## 独立microbench

2026年10月9日、VOIDLINUX-2の専用領域でD0/C006の初日10秒要求を3ペア比較しました。当日iterationsは6.14〜9.23%増え、全6提出が公開engineで合法、全回答10秒以内でした。翌日rollout量が減り、全探索量の増加は限定的です。これはserver短縮12件・正式12件の代替ではなく、採用判断を行っていません。[結果と再現手順](../../docs/distributed/resume-20261009/c006-microbench-20261009.md) に記録しています。全計測プロセスは停止済みです。
