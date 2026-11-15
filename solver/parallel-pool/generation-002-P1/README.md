# 並列pool 第2世代 P1候補

固定された単一スレッド共通基準から構築した、制御1＋探索14スレッドの候補です。旧並列候補や相手の実装・設計を参照していません。

```sh
PATH=/home/jikky/.cargo/bin:$PATH cargo build --release -p solver -j 2
./target/release/solver 206002
```

JSON Linesの入出力仕様は基準版と共通です。第1引数を探索seedとし、既定は206002です。探索条件・配備先は[探索と実行設定](docs/parallel.md)を参照してください。`solver/docs/`の既存資料は共通基準の説明として保持しています。
