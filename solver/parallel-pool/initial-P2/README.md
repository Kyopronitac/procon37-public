# 並列 pool 初期 P2

このディレクトリは固定基準から構築した P2 の独立ビルド版です。方針・起動設定・検証は [P2 の記録](solver/docs/parallel-P2.md) を参照してください。

---

# 並列pool共通基準

育成開始時の現行単一スレッド版。育成中は変更しない。

`cargo build --release -p solver` で独立ビルドする。
