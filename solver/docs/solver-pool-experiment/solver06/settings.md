# 第6世代ソルバの設定

[一覧](../algorithms.md#solver06) / [単一ファイル](../solver06.rs) / [設定](settings.md) / [検証](validation.md)

## 起動

```sh
cargo build --release --offline --manifest-path solver/docs/solver-pool-experiment/Cargo.toml --target-dir target --bin pool-solver-06
target/release/pool-solver-06 0
```

第1引数は探索用乱数のseedで、省略時は0。問題生成seedとは独立している。

標準入力でJSON Lines v1の`select`と`plan`を受け取り、標準出力へ同じ`requestId`の回答を1行ずつ返す。診断ログはstderrに出す。

## 探索時間

各要求の`timeLimitMs`から余裕を引いた範囲で探索する。固定反復数や環境変数による時間短縮設定は持たない。テスタで短縮試験を行う場合は`--time-limit-ms`を使用する。正式評価は既定時間で行う。

内部の探索は単一スレッドである。テスタの同時ケース数とは別の設定として扱う。

## 検証

提出前に、その日の受信交通状態を使用して共通エンジンで計画の合法性を確認する。実装の説明は[algorithm.md](algorithm.md)、実行結果は[validation.md](validation.md)を参照する。

## 保存版の起動と対戦設定

保存用のビルド対象は `pool-solver-06`、入口は [単一ファイル版](../solver06.rs) です。通信・引数・探索設定は元の分割ソースと同じです。[当時の正式評価設定](../records/generation-006-config.json)には実行時のパスと対戦相手を保存しています。現在の起動手順は [再現手順](../reproduction.md) を参照してください。保存形式の整理では追加ビルド・追加テストを行っていません。
