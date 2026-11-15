# 分散ソルバ候補 C007：compact経路表と訪問列の遅延復元

2026年10月9日に採用D0から作成した独立Rust workspaceです。**未採用・対戦未評価**です。D0、C006、固定比較相手、評価ハーネスは変更していません。

## 実装

D0は各mode・target・sourceについて全訪問セルの `Vec<usize>` をGraph構築時に作っていました。C007は各mode・targetの逆向きDijkstraが決めた `next` を `u16` の不変表として持ち、個々のPathはsource、経路長、時間、燃料、表への参照を持ちます。訪問列は初回利用時にだけ `OnceCell<Box<[usize]>>` へ復元し、以後再利用します。

Dijkstraのheap順序と厳密な距離更新条件はD0を保持しています。正の辺重みにより、あるセルが確定する時点でそのsuccessorは確定済みです。時間・燃料・経路長をsuccessorから計算し、出発セルの料金を加算します。経路mode重複判定ではfingerprintを除外条件として使い、一致した場合にnext列を厳密に比較します。hash衝突による別経路の誤統合はありません。この比較も訪問列の展開を行いません。

不変の `Arc<RouteTree>` は各workerのGraph内で同じtargetに向かうPathから共有します。worker間のGraph共有やGPU処理は今回の変更に含みません。将来の別候補で不変表を共有する余地があります。

変更した既存ソースは `solver/planner.rs` だけです。時間予算はD0の91%と110ms控除、当日67%・車種選択62%のrollout配分、通信期限を保持しています。探索・評価関数・乱数操作は変更していません。

## 検証

- 既存単体11件と追加2件、計13件が成功。
- 3形式で1,608,270経路について、独立したD0 eager参照器と訪問列・所要時間・燃料・店舗訪問順序・mode maskの同一性を確認。
- Graph構築とmode重複判定が訪問列を展開しないこと、初回展開の再利用、強制したhash衝突でも異なる経路を除外しないことを確認。
- 既存localhost通信統合2件が成功。内部16thread、状態同期、遠隔候補採用、切断・復帰、前日不受理を確認。全試験プロセスは停止済み。
- CPU1だけに固定した有限microbenchを、同一test binary内のD0 eager実装と比較。構築＋4096件ずつのmetadata/cold/hot queryの合計は、3形式各3反復の対応比較で1.54/1.92/1.98倍。cold訪問列query単体は遅くなるため、実探索での効果は対戦試験で確認する必要があります。

[microbenchの方法・全結果](../../docs/distributed/resume-20261009/c007-microbench-20261009.md) と [検証記録](validation.json) を参照してください。microbenchはserver短縮12件・正式12件の代替ではありません。

## 識別

親は採用D0、初期実装commit `5d6c740962a1f3e20e77e13df243078df1d29ad2`。コピー元は作業リポジトリ内 `solver/distributed` です。配置用 `tools/`、`runtime/`、生成物 `target/` はコピーしていません。Rustモジュール `runtime.rs` は含みます。

- ソース34ファイル集約：`8db728384c9a6cc3727c8def4a0e4f6720a5dd54b370ab33675e4789415e731f`
- production binary：`49eb466a4eb127075bbb5ce9bd90e297ca19f8cb7e61f6af320d585aef53cecc`

[source-manifest.json](source-manifest.json) に全Rustソース、Cargoファイル、問題fixtureを識別保存しています。追加テストは `solver/compact_tests.rs` と `solver/compact_bench.rs`。集約は相対パス順の `<sha256>  <relative path>\n` 列に対するSHA-256で、README・manifest・ビルド生成物を含みません。

## ビルドと再現

実際のリポジトリルートで実行します。絶対パスは固定しません。固定相手のCPU0/2/4/6を避けるため、以下はCPU1に限定しています。

```sh
taskset -c 1 cargo build --release --offline --manifest-path solver/distributed-candidates/c007-compact/Cargo.toml -j 1
taskset -c 1 cargo test --release --offline --manifest-path solver/distributed-candidates/c007-compact/Cargo.toml --bin distributed-solver -j 1 -- --test-threads=1
taskset -c 1 cargo test --release --offline --manifest-path solver/distributed-candidates/c007-compact/Cargo.toml --test network -j 1 -- --test-threads=1 --nocapture
```

通信テストはlocalhostへのTCP接続を許可する環境で実行してください。通常のunit testでは性能計測testはignoreされます。明示実行コマンドはmicrobench記録にあります。リモート配置・対戦評価・採用commit/pushは行っていません。
