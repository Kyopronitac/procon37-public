# 第6世代ソルバの設定

## 起動

```sh
cargo build --release -p solver
target/release/solver 0
```

第1引数は探索用乱数のseedで、省略時は0。問題生成seedとは独立している。

標準入力でJSON Lines v1の`select`と`plan`を受け取り、標準出力へ同じ`requestId`の回答を1行ずつ返す。診断ログはstderrに出す。

## 探索時間

各要求の`timeLimitMs`から余裕を引いた範囲で探索する。固定反復数や環境変数による時間短縮設定は持たない。テスタで短縮試験を行う場合は`--time-limit-ms`を使用する。正式評価は既定時間で行う。

内部の探索は単一スレッドである。テスタの同時ケース数とは別の設定として扱う。

## 検証

提出前に、その日の受信交通状態を使用して共通エンジンで計画の合法性を確認する。実装の説明は[algorithm.md](algorithm.md)、実行結果は[validation.md](validation.md)を参照する。
