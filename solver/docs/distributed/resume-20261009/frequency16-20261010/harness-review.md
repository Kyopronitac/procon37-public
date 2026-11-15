# 有限評価ハーネスの確認

作成担当者とは別にroot担当が `solver/distributed-lab/frequency_matrix.py` を読み、以下を確認した。既存C013評価を再実行しない有限の実行である。

- C014とC015を各seed 0・3・6・9で1回ずつ評価する。
- 今回の起動planはtimeLimitMs=nullで、serverコマンドへ時間短縮を渡さない。
- 4チーム・形式自動・練習場マップ有効をrun.jsonから再確認する。保存先の上書きはしない。
- 生成した期待problemと試合で使用したproblemを完全一致で照合し、全員ACを要求する。
- 切替対象は分散3台のソルバ本体。bridge、launcher、固定3相手、CPU配置、環境条件を維持する。
- 候補のソース全体・manifest・validation・バイナリを登録し、開始前と終了後のsnapshotで照合する。ハーネス自身のSHAも固定する。
- 失敗時に再試行せず、評価プロセスと登録チームを停止して失敗を保存する。成功時は次の操作を担当者が行うため登録チームが残る。
- 内部deadlineは06:08 JST。評価完了後に全チームと試験時計を停止・確認してから、本番用probeとactivateへ進む。

ハーネスSHA256: `aa46f22ca4314411a7614d3a11e17f52f7c4ebdd6c5eb25d4fff7d0e5eed8d53`。
実機評価の結果確定と全32ACの確認は、完了後のresults.jsonが根拠となる。
