# 分散ソルバの継続開発

2026年10月8日開始。VOIDLINUXの3台が1チームとして探索し、VOIDLINUX-1が探索・結果集約・提出を担当する。ユーザーの停止指示まで改善と評価を継続する。

最新の16×16試合用配置は **C013（500ms共有・途中提出）**。[選定結果と当日の起動方法](resume-20261009/frequency16-20261010/result.md)、[本番配置ツール](../../../tester/server-bridge/competition/README.md)を参照する。D0の正式12件採用記録と従来配置は変更していない。[D0の配置・運用](../../../tester/server/docs/solver-pc.md#分散ソルバd0を3台で本番に使う)と[保存先の詳細](deployment.md)も保持する。

## 開始版の選定

完了・保存済みの正式12件の結果を確認し、第3世代P0 v3を共通開始版に選定した。最新の正式評価 `1791406043071407716-280358` は全72判定AC、順位合計20、平均順位1.6667で単独首位。種類数222、日別種類数1277、玉数2998。過去の評価は相手の構成が変化しているため、今回の4チーム構成では初回正式評価を新しく行う。

[選定・ソース照合記録](start-selection.json) に開始コミット、全ソースファイルのハッシュ、評価済み実行ファイルのハッシュ、比較した正式結果を保存する。開始版のソースは `solver/parallel-pool/generation-003-P0` のまま保存し、分散実装は `solver/distributed` に独立して置く。

## 固定構成

| team | 役割 | 配置 | 使用論理CPU |
| --- | --- | --- | --- |
| 0 | 分散ソルバ | VOIDLINUX-1・2・3 | 32・16・32、計80 |
| 1 | 共通開始版の並列比較相手 | NOMLAB-UBUNTU | 16 |
| 2 | 単一スレッド第6世代 | サーバーPC | CPU 0 |
| 3 | 単一スレッド第8世代 | サーバーPC | CPU 2 |

NOMLABは指定IPの実機を調べた結果Ryzen 9 5900HS、8物理コア・16論理スレッドだった。ユーザーの明示的な回答により、この実機の16論理スレッドをすべて使う。VOIDLINUXは指定どおり32・16・32論理スレッド。各機NUMAは1ノード。CPU governor・EPPをperformanceに揃えた（VOIDLINUX-1だけgovernorを変更、残りは既存設定を確認）。[実機記録](runtime-inventory.json)、[性能設定](cpu-optimization.json) を参照。

単一スレッド2体は既存の保存版ビルド実行ファイルと識別用ハッシュを照合した。実装は閲覧していない。[固定相手の識別と再ビルド方法](fixed-opponents.json) を参照。このPCのCPU 0と2は異なる物理コア。対戦窓口のLANアドレスは実測 `192.168.0.81:11451`。実行領域は各SSHユーザーのホーム直下 `procon37-distributed` に分離する。

## 評価・採用

短縮・正式とも4チーム、12件、seed 0〜11、形式自動（各4件）、練習場有効、全48判定ACを必須とする。短縮は車種選択と各日10秒を初期値とし、件数を減らさない。正式は制限時間や探索予算の短縮を残さない。両方ともラベルを付け、`--output-dir` を指定せず既定 `.server/runs` に保存する。

`solver/distributed/tools/run_evaluation.py` は評価を分離起動し、`.server/distributed-state.json` と `.server/distributed/<段階>-process.json` にPID・コマンド・ログ・設定を残す。`summarize.py` は件数、seed、形式、時間条件、全員AC、保存された4チームの順位と3得点合計を検証する。

初回正式評価の実装・配置・環境再現・評価記録をcommit・通常pushした後、改善を開始する。改善は採用版と同条件の短縮12件で繰り返し確認し、明確な改善が見えた候補を正式評価する。正式採用ごとに評価した版を変更せず、日本語Conventional Commitsで今回の変更だけcommit・pushする。push成功まで採用手続きは未完了。

評価開始前と終了後にソース・実行ファイル・設定・配置・bridge・公式サーバーのハッシュを新しく取得して照合する。比較条件を変更した場合は採用済み分散版も同条件で再評価する。順位が同じ場合も種類数・日別種類数・玉数、seed別の勝・同点・敗を確認する。

## 初回正式評価

[分散初期版D0の正式評価](initial-evaluation.md)は全48判定AC。平均順位は分散1.6667、固定並列2.0833、第8世代2.5000、第6世代3.2500だった。評価済み実装を初期版D0としてcommit `5d6c740962a1f3e20e77e13df243078df1d29ad2` に保存し、通常push成功。[採用記録](initial-promotion.json) と再開状態に手続き結果を保存した。

## GPUと再開

改善担当subagentは、ユーザー指定により **GPT-6 Astra / ultra**（`gpt-6-astra`、`reasoning_effort="ultra"`、履歴なし起動）を使う。初期版を構築した開発者の設定は履歴として保持し、この指定は初回正式評価・初期commit/push完了後の改善から適用する。

VOIDLINUX-1はRTX 5070 Ti Mobile、2はRTX 5060 Mobileを搭載するが、現環境は古いnouveauでNVIDIA計算用ドライバが未導入。3はAMD内蔵GPU、予約VRAM2GiB。初期版はCPU分散を成立させ、GPUランタイムの準備と処理移行は効果測定・同条件対戦評価を伴う改善として続ける。VOIDLINUX-1はLANのみでDNSと外部向けdefault routeがなく、必要なパッケージは外部接続可能な2・3経由でも転送できる。NVIDIA環境構築は[Void公式手順](https://docs.voidlinux.org/config/graphical-session/graphics-drivers/nvidia.html)のnonfreeリポジトリ・DKMS方式を確認済みで、初回評価後の保守段階で進める。

進行状態は `.server/distributed-state.json`。再開時はPID、ログ、保存されたrun、各機のプロセスとハッシュを実状態と照合し、同時に二つの評価を起動しない。停止指示時は進行中の処理、採用版、評価、commit・push、残るリモートプロセスと再開方法を報告する。
