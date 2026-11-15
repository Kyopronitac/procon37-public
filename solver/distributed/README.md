# 分散ソルバ

`solver/parallel-pool/generation-003-P0` を開始版とする独立Rust workspace。
VOIDLINUX 3台を1チームとして動かす分散モードと、NOMLABの16論理スレッドを使う比較モードを提供する。

```sh
cargo build --release --manifest-path solver/distributed/Cargo.toml
```

実行ファイルは `solver/distributed/target/release/distributed-solver`。
JSON Linesの外部入出力とserver-bridgeは従来のままで、coordinatorだけをteam0のbridgeへ接続する。

| ホスト | モード | 内部総スレッド | 探索ID |
| --- | --- | ---: | --- |
| VOIDLINUX-1 | coordinator | 32（制御1＋探索31） | 0〜30 |
| VOIDLINUX-2 | worker | 16（制御1＋探索15） | 31〜45 |
| VOIDLINUX-3 | worker | 32（制御1＋探索31） | 46〜76 |
| NOMLAB | parallel（比較相手） | 16（制御1＋探索15） | 0〜14 |

起動例、通信仕様、状態同期、開始版との差分、検証結果は [実装説明](../docs/distributed/implementation.md) を参照する。
`tools/` 配下の実機配置・評価管理ツールは親担当が管理する。
