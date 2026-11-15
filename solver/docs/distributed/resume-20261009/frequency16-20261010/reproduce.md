# 評価版と配布版の再現

## ソースと実行ファイル

比較した3案は `solver/distributed-candidates/c013-scheduled-submission`、`c014-share100ms`、`c015-share2000ms` に独立したCargo workspaceとして保存する。各 `source-manifest.json` にビルド対象34ファイルのSHA-256と親との差分を記録した。評価中と配置時にソルバ実装を編集しない。実際に使った実行ファイルは `validation.json` のSHAと評価開始・終了の配置記録で識別する。

配布パッケージは評価済みソルバとbridgeの実行ファイルを直接取り込む。Gitにはソースと検証記録を保存し、ビルド生成物や秘密情報を含めない。評価したバイナリと配布archiveはこのPCの `.server/competition/` とVOID3台の選定releaseに保持する。実際の保存先・SHAは最終deployment記録を参照する。回収や再配置ではSHAを照合し、再ビルドしたファイルを同一バイナリと仮定しない。

新しいビルドを検証する場合、各候補で空の専用target directoryを使用する。今回C015の最初の試行で共用target由来の古いバイナリを検出したため、その試行を無効化して独立targetで再ビルド・再検証した。無効な記録も理由付きで保持している。C015の有効SHAは `13c261e0512a224172ab49b1db4f733960c96ce522b45d6aba829728769b1972`。

## 評価と配布の違い

評価は4チームで、分散がVOID3台、固定並列がNOMLAB、単一第6・第8世代がこのPC。VOIDは32・16・32論理スレッド、NOMLABの実機は16論理スレッド、単一2体はCPU 0と2に固定した。問題serverはCPU 1・3・5・7を使う。CPU governorはperformance、時計は試験専用runtime chrony sourceで評価前に合わせ、途中で時刻をstepしない。

本番配布では選定した分散3台だけを使い、VOID1で `~/procon37-competition/run.sh` を実行する。編集可能な入口のSERVER/TOKEN以外は配布manifestで固定する。workerポートは評価用39370から専用39371へ分ける。探索実装・実行ファイル・CPU数・worker offset・共有設定・提出時刻は評価時と同じ。起動管理wrapperの配置・認証・停止は独立した11件のローカル検証と実機probeで確認する。専用wrapperそのものを4seed対戦で再評価したという意味ではない。

本番サーバーのURLとチームトークンは当日ユーザーが入口を編集する。deploy/probe/activateにこの情報は不要で、配置確認で本番問題サーバーへHTTPを送らない。

## 保存された履歴

新規8試合の原本は既定 `.server/runs/` に保存する。Gitへ保存する比較記録にはrun.jsonの複製と、problem/config/replayの識別用ハッシュを含める。基準の4seedはユーザー指定の既存記録で、seed0だけC012、seed3・6・9がC013。完全に均一な3案比較ではないため、C013と同じ提出方針の3seed集計も併記する。
