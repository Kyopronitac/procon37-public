# C009 owner構築の独立測定器

凍結したC009のCPU builderとGPU owner adapterをそのまま読み込む、有限の補助プログラム。C009や評価中のソルバを変更せず、VOIDLINUX-2で3形式・各2組の比較を行うために準備した。

CPU 3・`-j1`のreleaseビルドとCPU-only検査が完了。6組で3,208,980経路のscalar一致、正常終了、子PID残存なしを確認した。実GPUの測定結果はまだない。実機操作は対戦の区切りで行う。

詳細は [実行計画](docs/procedure.md)。`src/owner_impl.rs` が実際のbuilderを呼び、`tools/run_v2.py` が今回専用cpusetの作成・監視・子孫回収を行い、`tools/summarize.py` が費用と償却条件を集計する。実戦成績の改善を判定するテストではない。
