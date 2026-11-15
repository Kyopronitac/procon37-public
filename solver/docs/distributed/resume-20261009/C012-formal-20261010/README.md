# C012正式12件：開始・監視引継ぎ

**終了更新：02:29:33 JSTに12件終了。固定第8世代の車種提出遅延による4WAで全48AC条件不成立。C012自身は12AC、採用は保留。** [結果と参考比較](result.md)を参照。以下の起動情報と成功時の手順は開始時点の記録として残す。成功用集計は実行せず、失敗記録として確定した。

ユーザーはC012の短縮2回目を不要とし、C012の正式評価を実行して**保存済みD0正式結果**と比較するよう指定した。以前の停止記録・比較計画にある「次は短縮2回目」「新しいD0正式が必要」は今回の実行指示には適用しない。追加の評価や候補変更は開始しない。

## 実行状況

- 実際のチェックアウト：`/home/imak/work/procon37`。再開時も配置を確認する。
- stage：`C012-formal-01`。run ID：`1791561513817407923-45859`。
- 開始：**2026-10-10 00:58:33 JST**。終了見込み：**同日02:30〜02:40 JST**。
- 12ケース・72日×60秒、選択時間合計18分、開始待ち合計1分＝公称91分。公称終了02:29:33に起動・通信・保存時間を見込んだ推定で、終了保証ではない。[各ケースの日程](schedule.json)。
- server PID `45859`、startTicks `1349208`、boot ID `61fafedf-4b69-4c18-81a9-6af42f0f3422`。
- 原本：`.server/runs/1791561513817407923-45859/run.json`。
- 起動・前後照合・ログ：`.server/distributed/evaluations/C012-formal-01/`。
- 開始参加観測：`.server/distributed/observations/C012-formal-01/{mark,start}.json`。
- 通常の独立セッションで起動済み。全4チームの実CPU稼働を開始観測で確認した。完了・全48AC・改善はまだ未判定。

`timeLimitMs: null`、`--time-limit-ms`および`--output-dir`なし。seed0の実際のproblemは全日60秒、公式サーバーの`-kind-deadline 60000ms`を確認した。seed0〜11、各形式4件、練習場マップ有効。各試合の実時間条件も終了時に確認する。

## 固定識別と配置

C012は`--cross-elite on`。source34集約SHA `10acfa3bde81f1dddf1d80ed4ff2fbe0288395fc4eb6dd0923a410607ed8a541`、binary SHA `574dd99a8ff0d20035acccca3f4e207ad2d7701308bc10374694137aa0f33016`。新規ビルド・探索設定変更なし。切替記録は`.server/distributed/switches/20261010T005813-C012-isolated-cooperation-on/switch.json`。評価中はソース・バイナリ・登録設定・配置を変更しない。

| 機体 | 配置とCPU | 開始時の活動探索スレッド |
| --- | --- | ---: |
| VOID1 | `~/procon37-distributed-resume/20261009-D0`、CPU0-31、集約・提出も担当 | 31 |
| VOID2 | 同名ディレクトリ、CPU0-15 | 15 |
| VOID3 | 同名ディレクトリ、CPU0-31 | 31 |
| NOMLAB | `~/procon37-distributed`、CPU0-15、固定D0並列 | 15 |
| ローカル第6/8世代 | `.server/distributed/bin`、CPU0/2 | 各1 |
| ローカルserver・clock | CPU1,3,5,7 | 対象外 |

NOMLABは実測5900HS・8C16Tのままで、全16論理CPUを割り当てる。VOIDは制御込み32/16/32論理CPU。時計の復元は[clock-restored.json](clock-restored.json)、ローカルclock PID45307の識別は[clock-process.json](clock-process.json)。4台のchronyはruntime source `192.168.0.81:32123`を使用。時計stepは開始前に完了し、試合中には行わない。永続設定SHAは既存バックアップと一致した。接続・設定の開始前確認は[readiness-inventory.json](readiness-inventory.json)。

## Automationの状態

**監視未設定。** 現在利用可能なツールの名前と説明を調べたが、Codexの`automation_update`や同等の作成・更新ツールは公開されていなかった。PagesやSitesの別製品のスケジュールを代用していない。以前のD0開始記録にあるAutomation識別子は今回の有効性を確認できず、再利用・動作中と扱わない。

[テスタスキル](../../../../../.agents/skills/procon37-tester/SKILL.md)と[実行手順](../../../../../.agents/skills/procon37-tester/docs/run.md)の「Automationが利用できない、または設定に失敗した場合は、監視が設定できていないことと実行状況・保存先を明示してターンを終了する」に従う。別方式の常駐監視は作らない。したがって、この記録だけでは自動通知・完了集計は起動しない。

Codex側で現在のチャットに紐づくheartbeatを設定できるようになった場合は、次を監視指示とする。目安10分間隔。同じrunに既存監視があれば更新し、重複作成しない。

> このチェックアウトのAGENTS/READMEと本記録を読み、C012-formal-01だけを監視する。run.json、server.log、process.jsonのPID・startTicks・boot IDを軽く確認する。進行中で異常がなければ通知・長い分析をせず終了する。正常終了は12件・全48AC・前後識別一致を確認してから下記の集計・記録・保存済みD0比較を行う。失敗・中断は正常完了とせず、原因を報告し同じバッチを自動再実行しない。重要な変化、完了、失敗、ユーザー対応が必要な場合だけ通知する。完了または失敗時は今回のheartbeatを無効にする。新しい候補・評価は開始しない。テスト中の固定設定と保護対象を守る。

## 完了後の集計

以下は**未実行の完了処理**。SSH操作には既存の許可とスキルを適用し、必要な秘密値は既存の認証方法で渡す。評価プロセスが終了してから実行する。停止・失敗を成功と扱わない。

```sh
taskset -c 1,3,5,7 python3 solver/distributed-lab/manage.py summarize C012-formal-01 --run-id 1791561513817407923-45859
taskset -c 1,3,5,7 python3 solver/distributed-lab/observe.py C012-formal-01 end
taskset -c 3 python3 solver/distributed-lab/record_result.py C012-formal-01
taskset -c 3 python3 solver/distributed-lab/compare_saved_formal.py C012-formal-01
```

既存の`--before-stage`や`compare_repeats.py`の同条件照合は緩めない。今回だけ、ユーザー指定の保存済み正式記録を明示した別の履歴比較を行う。比較スクリプトは完了検証後だけ実行でき、平均順位、3得点指標、形式別成績、各seedの辞書順得点比較による勝・同点・敗を記録する。

D0基準runは`1791414715585964511-304414`、[保存run](../../initial-formal-run.json)・[保存集計](../../initial-formal-results.json)。全48AC、平均順位1.6666667、種類222／日別1276／玉2989。旧原本problem.jsonは当地にないため、問題そのもののハッシュ一致は証明できない。指定seed・形式・マップ設定・公式時間は一致するが、復元後のローカルserver・単一2体の実行ファイルSHAは旧正式記録と異なる。比較結果にはこの識別差を添え、完全に同一環境の因果比較と断定しない。

採用版はD0のまま。採用commit `5d6c740962a1f3e20e77e13df243078df1d29ad2`はpush済み。今回のC012は未採用、今回の追加commit/pushはまだない。採否は全件結果と比較の限界を確認して判断する。採用する場合は実際に正式評価した版をそのままcommit・通常pushし、成功を記録する。

集計後、次の評価を始めず待つ場合は専用teamを`lifecycle.py stop`で識別照合して停止し、runtime chrony sourceと専用clockも[pause-cleanup-20261010.json](../pause-cleanup-20261010.json)に記録された安全な識別手順で片付ける。時計・チームはテスト終了後も自動では停止しない。残存プロセスと再開方法を報告する。
