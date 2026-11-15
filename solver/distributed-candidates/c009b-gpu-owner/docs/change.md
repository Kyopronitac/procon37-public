# C009bの変更範囲と検証予定

親C009のsource aggregateは`9f8e182d3573dfc3514502c75ba8a4f11bb9c39be8f2dc2e4294ca675d4b533a`。47ファイルをコピーし、`solver/gpu_graph.rs`と`solver/gpu_graph_tests.rs`だけ変更した。

変更はcgroup tasksへの書込みを一回にまとめること、短いwriteをエラーとして再試行しないこと、open/writeエラーの文脈追加。GPU計算・検証・CPU builder・探索・共有cacheに変更はない。

成功writeが一回か、短いwriteを再試行しないか、errno22時に操作文脈が残り再試行しないかをmock Writerの3テストで確認するコードを用意した。現時点では未実行。

初回のerrno22が断片化書込みによるものかは推定。次の評価間にCPU3/-j1で単体検証し、新しいbinary/source SHAを保存する。その後、別probe `solver/distributed-gpu-lab/c009b-owner-probe`をビルドし、v2専用の新規deployment/groupで有限preflightを1回だけ実行する。成功時も初期化・初回要求・転送・検証・CPU組立・終了を含む全費用からGPUの有用性を判断する。失敗時は再試行せず原因と残存を報告する。

C009の失敗記録は`solver/distributed-gpu-lab/c009-owner-probe/docs/v2-idle-20261009-r1.md`に保存している。C009bの結果として流用しない。
