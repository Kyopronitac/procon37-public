# 並列 pool 初期候補 P0

固定された単一スレッド基準から作成した、制御 1 本＋探索最大 14 本の並列ソルバです。基準の探索・シミュレーションを使い、経路表を共有した独立探索と翌日試算で計画を選びます。

```sh
PATH=/home/jikky/.cargo/bin:$PATH cargo build --release -p solver
./target/release/solver
```

既定起動は引数なし（探索 seed 0）です。第 1 引数で探索 seed、第 2 引数で基準の到達性評価重み、第 3 引数でビーム内の時間重みを指定できます。第 4 引数の逐次 archive 設定は使用せず、P0 の並列翌日比較を使います。探索スレッド数はコンパイル時定数 14 で、引数や環境変数では増やせません。

- [P0 の並列設計と検証](solver/docs/p0.md)
- [共通基準の探索方法](solver/docs/algorithm.md)
- [共通基準の設定](solver/docs/settings.md)

このコピーは単独でビルドできます。SSH 配置・CPU 割り当て・正式評価は親エージェントが管理します。
