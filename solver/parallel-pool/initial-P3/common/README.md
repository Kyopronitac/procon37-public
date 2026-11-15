# 共通テスタ基盤

`procon37-common` は [単独テスタ](../tester/solo/README.md) と [5人対戦テスタ](../tester/game/README.md) が共有するRustクレート、ブラウザ画面、デスクトップ起動処理をまとめる。[対戦サーバー](../tester/server/README.md) も問題生成・判定・比較と画面部品・デスクトップ起動処理を共有する。

- [構成・互換性・検証](docs/design.md)
- [通信の基本仕様](../tester/solo/docs/protocol.md)
- [対戦での通信・判定](../tester/game/docs/protocol.md)

`tester::model`、`tester::engine`、`tester::runner` などの公開パスは再公開で維持する。既存ソルバの探索実装や単独テスタの保存形式は変更しない。
