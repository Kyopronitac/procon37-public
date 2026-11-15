# 並列pool 第1世代 P0 候補

共通基準から独立に構築した、制御1＋探索14スレッドの並列ソルバです。複数の探索島が経路を共有し、当日の候補と翌日の試算を公開エンジンの得点で比較します。

- [候補の設計と検証](solver/docs/parallel-generation-001.md)
- [起動設定](solver/docs/settings.md)
- [共通基準の探索方法](solver/docs/algorithm.md)

候補ルートから `PATH=/home/jikky/.cargo/bin:$PATH cargo build --release -p solver -j 2` で独立ビルドします。起動は `target/release/solver 0` です。正式評価と配置は親が管理します。
